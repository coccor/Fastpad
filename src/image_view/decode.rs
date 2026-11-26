//! Full-resolution decoding for the image view (image preview spec §6.2), on one short-lived
//! worker thread per request. Raster files go through WIC: the largest ICO frame, EXIF orientation
//! applied, downscaled only past the render target's maximum bitmap size. SVG source is rasterized
//! with Direct2D at the width the view asks for.

use crate::preview::images::{ComScope, MAX_IMAGE_PIXELS};
use crate::preview::svg::{MAX_SVG_BYTES, natural_size, parse, rasterize_region};
use resvg::usvg;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use windows::Win32::Foundation::GENERIC_READ;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::StructuredStorage::{PROPVARIANT, PropVariantClear};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::Win32::System::Variant::VT_UI2;
use windows::core::{GUID, Interface, PCWSTR, w};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW;

/// WINCODEC_ERR_COMPONENTNOTFOUND: no installed decoder accepts the bytes.
const COMPONENT_NOT_FOUND: i32 = 0x8898_2F50_u32 as i32;
/// Formats whose decoder is an optional Store extension.
const OPTIONAL_CODECS: [&str; 4] = ["webp", "heic", "heif", "avif"];

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ImageError {
    TooLarge,
    NoCodec,
    Damaged,
    Missing,
    SvgTooLarge,
    /// A FastDraw diagram that cannot be drawn: `Line n: message`.
    Diagram(String),
    Read(String),
}

impl ImageError {
    pub fn message(&self) -> String {
        match self {
            Self::TooLarge => "The file is larger than 64 megapixels.".to_owned(),
            Self::NoCodec => "Windows has no decoder for this format.".to_owned(),
            Self::Damaged => "The file is damaged or not an image.".to_owned(),
            Self::Missing => "The file no longer exists.".to_owned(),
            Self::SvgTooLarge => "The SVG is larger than 8 MB.".to_owned(),
            Self::Diagram(text) | Self::Read(text) => text.clone(),
        }
    }

    /// The heading of the failed state.
    pub fn title(&self) -> &'static str {
        match self {
            Self::Diagram(_) => "FastPad can't draw this diagram",
            _ => "FastPad can't display this image",
        }
    }
}

pub struct FullImage {
    /// Decoded pixels (after orientation; smaller than natural only past `max_side`).
    pub width: u32,
    pub height: u32,
    /// Display size in image pixels, after orientation.
    pub natural_width: u32,
    pub natural_height: u32,
    pub format: &'static str,
    pub pixels: Vec<u8>,
    /// Set for a FastDraw diagram.
    pub diagram: Option<DiagramFacts>,
    /// The parsed picture of an SVG or diagram, kept to render other zooms and tiles from.
    pub tree: Option<Arc<usvg::Tree>>,
    /// The tree is drawn by Direct2D at every paint (`vector::Scene`); `pixels` is then empty.
    pub vector: bool,
}

/// Past this many pixels, a vector picture is not rasterized whole at the zoom: the whole picture
/// renders at this size (drawn enlarged while panning), and a tile of the view renders sharp. A
/// whole picture costs about 14 ns a pixel in a release build, so this keeps a zoom step near
/// 0.05 s however deep it goes.
pub(crate) const WHOLE_PIXEL_BUDGET: u64 = 4_000_000;
/// How far a tile reaches past each edge of the view, in pixels, so a short pan stays sharp.
pub(crate) const TILE_MARGIN: f32 = 256.0;

/// A rectangle of a zoomed picture in its pixels, from the picture's top-left corner.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TileArea {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl TileArea {
    pub fn contains(&self, other: &TileArea) -> bool {
        other.x >= self.x
            && other.y >= self.y
            && other.x + other.width <= self.x + self.width
            && other.y + other.height <= self.y + self.height
    }
}

/// The part of the picture at `scale` the view shows, rasterized at that scale.
pub struct Tile {
    pub scale: f32,
    pub area: TileArea,
    pub pixels: Vec<u8>,
}

/// Whether a vector picture of `natural` size zoomed to `width` pixels is larger than the whole
/// picture renders, so the view needs a tile to show it sharp.
pub(crate) fn tiled(natural: (u32, u32), width: u32, max_side: u32) -> bool {
    svg_render_size(natural, width, max_side, WHOLE_PIXEL_BUDGET).0 < width
}

/// The area of a picture `picture` pixels large, placed at `offset` in a view `client` pixels
/// large, that the view shows plus `margin` on each side; whole pixels, at most `max_side` a side.
/// `None` when no part of the picture is in view.
pub(crate) fn tile_area(
    picture: (f32, f32),
    offset: (f32, f32),
    client: (f32, f32),
    margin: f32,
    max_side: u32,
) -> Option<TileArea> {
    let axis = |length: f32, offset: f32, view: f32| {
        let start = (-offset - margin).max(0.0).floor();
        let end = (-offset + view + margin).min(length.ceil()).ceil();
        (end > start).then(|| (start as u32, ((end - start) as u32).min(max_side.max(1))))
    };
    let (x, width) = axis(picture.0, offset.0, client.0)?;
    let (y, height) = axis(picture.1, offset.1, client.1)?;
    Some(TileArea {
        x,
        y,
        width,
        height,
    })
}

/// What the view needs to know about a drawn diagram.
#[derive(Clone, Debug, PartialEq)]
pub struct DiagramFacts {
    /// The look's paper colour, `0x00RRGGBB`, painted behind the picture instead of a checkerboard.
    pub paper: u32,
    /// The error the band shows over a drawing that is still shown (a refused look file).
    pub band: Option<String>,
    /// Where the drawing's elements sit and their lines, so a click finds its line.
    pub hits: Arc<crate::preview::fastdraw_hits::Hits>,
}

/// A tile to render with a decode or render: its zoom and area.
pub type TileJob = (f32, TileArea);

pub enum Source {
    File(PathBuf),
    /// `gpu`: a tree Direct2D can draw comes back unrasterized, to be drawn at every paint.
    Svg {
        text: Arc<str>,
        width: u32,
        tile: Option<TileJob>,
        gpu: bool,
    },
    FastDraw {
        text: Arc<str>,
        file: Option<PathBuf>,
        dark: bool,
        width: u32,
        tile: Option<TileJob>,
        gpu: bool,
    },
}

/// A finished decode of the shown source.
pub struct Decoded {
    pub generation: u64,
    pub result: Result<FullImage, ImageError>,
    pub tile: Option<Tile>,
}

/// A finished render from a kept tree: the whole picture at another size, a tile, or both.
pub struct Rendered {
    /// The tree it was rendered from, as the view numbers them.
    pub tree_id: u64,
    /// `(width, height, pixels)`.
    pub whole: Option<(u32, u32, Vec<u8>)>,
    pub tile: Option<Tile>,
}

/// What a worker posts back; `lparam` is a `Box<Delivery>`, which the receiver frees.
pub enum Delivery {
    Decoded(Decoded),
    Rendered(Rendered),
}

fn post(delivery: Delivery, notify: isize, message: u32) {
    let boxed = Box::into_raw(Box::new(delivery));
    if unsafe { PostMessageW(notify as HWND, message, 0, boxed as isize) } == 0 {
        drop(unsafe { Box::from_raw(boxed) });
    }
}

/// Decodes `source` on a new thread and posts `message` to `notify` with a
/// `Box<Delivery::Decoded>` in `lparam`.
pub fn spawn(generation: u64, source: Source, max_side: u32, notify: HWND, message: u32) {
    let notify = notify as isize;
    std::thread::spawn(move || {
        let (result, tile) = match &source {
            Source::File(path) => (decode_file(path, max_side), None),
            Source::Svg {
                text,
                width,
                tile,
                gpu,
            } => split(svg_source(text, *width, *tile, max_side, *gpu)),
            Source::FastDraw {
                text,
                file,
                dark,
                width,
                tile,
                gpu,
            } => split(fastdraw_source(
                text,
                file.as_deref(),
                *dark,
                (*width, *tile, max_side),
                *gpu,
            )),
        };
        post(
            Delivery::Decoded(Decoded {
                generation,
                result,
                tile,
            }),
            notify,
            message,
        );
    });
}

fn split(
    result: Result<(FullImage, Option<Tile>), ImageError>,
) -> (Result<FullImage, ImageError>, Option<Tile>) {
    match result {
        Ok((image, tile)) => (Ok(image), tile),
        Err(error) => (Err(error), None),
    }
}

/// Renders a kept `tree` of `natural` size again on a new thread: the whole picture at `whole`
/// size and the `tile`, either or both. Posts `message` to `notify` with a
/// `Box<Delivery::Rendered>` in `lparam`.
#[allow(clippy::too_many_arguments)]
pub fn spawn_render(
    tree_id: u64,
    tree: Arc<usvg::Tree>,
    natural: (u32, u32),
    whole: Option<(u32, u32)>,
    tile: Option<TileJob>,
    notify: HWND,
    message: u32,
) {
    let notify = notify as isize;
    std::thread::spawn(move || {
        let (whole, tile) = render_parts(&tree, natural, whole, tile);
        let whole = whole.and_then(Result::ok);
        post(
            Delivery::Rendered(Rendered {
                tree_id,
                whole,
                tile,
            }),
            notify,
            message,
        );
    });
}

/// Rasterizes the whole picture at `whole` size and the `tile`, side by side when both are asked.
/// A tile that fails is left out: the whole picture still shows.
#[allow(clippy::type_complexity)]
fn render_parts(
    tree: &usvg::Tree,
    natural: (u32, u32),
    whole: Option<(u32, u32)>,
    tile: Option<TileJob>,
) -> (
    Option<Result<(u32, u32, Vec<u8>), ImageError>>,
    Option<Tile>,
) {
    let render_whole = |(width, height): (u32, u32)| {
        rasterize_region(tree, (width as f32, height as f32), (0, 0), (width, height))
            .map(|pixels| (width, height, pixels))
            .map_err(|_| ImageError::Damaged)
    };
    let render_tile = |(scale, area): TileJob| {
        let picture = (natural.0 as f32 * scale, natural.1 as f32 * scale);
        rasterize_region(tree, picture, (area.x, area.y), (area.width, area.height))
            .ok()
            .map(|pixels| Tile {
                scale,
                area,
                pixels,
            })
    };
    match (whole, tile) {
        (Some(size), Some(job)) => std::thread::scope(|scope| {
            let tile = scope.spawn(|| render_tile(job));
            let whole = render_whole(size);
            (Some(whole), tile.join().ok().flatten())
        }),
        (whole, tile) => (whole.map(render_whole), tile.and_then(render_tile)),
    }
}

pub(crate) fn classify(code: i32, extension: Option<&str>) -> ImageError {
    let optional = extension.is_some_and(|extension| {
        OPTIONAL_CODECS
            .iter()
            .any(|known| known.eq_ignore_ascii_case(extension))
    });
    match code {
        COMPONENT_NOT_FOUND if optional => ImageError::NoCodec,
        // ERROR_FILE_NOT_FOUND / ERROR_PATH_NOT_FOUND as HRESULTs.
        code if code == 0x8007_0002_u32 as i32 || code == 0x8007_0003_u32 as i32 => {
            ImageError::Missing
        }
        code if (code as u32) & 0xFFFF_0000 == 0x8007_0000 => ImageError::Read(
            windows::core::Error::from_hresult(windows::core::HRESULT(code)).message(),
        ),
        _ => ImageError::Damaged,
    }
}

/// The EXIF orientation (1–8) as a WIC transform, and whether it swaps width and height.
pub(crate) fn orientation_transform(value: u16) -> (WICBitmapTransformOptions, bool) {
    match value {
        2 => (WICBitmapTransformFlipHorizontal, false),
        3 => (WICBitmapTransformRotate180, false),
        4 => (WICBitmapTransformFlipVertical, false),
        5 => (
            WICBitmapTransformOptions(
                WICBitmapTransformRotate90.0 | WICBitmapTransformFlipHorizontal.0,
            ),
            true,
        ),
        6 => (WICBitmapTransformRotate90, true),
        7 => (
            WICBitmapTransformOptions(
                WICBitmapTransformRotate270.0 | WICBitmapTransformFlipHorizontal.0,
            ),
            true,
        ),
        8 => (WICBitmapTransformRotate270, true),
        _ => (WICBitmapTransformRotate0, false),
    }
}

/// Index of the frame with the most pixels (the first on ties).
pub(crate) fn largest_frame(sizes: &[(u32, u32)]) -> usize {
    sizes
        .iter()
        .enumerate()
        .fold((0, 0_u64), |best, (index, &(w, h))| {
            let area = u64::from(w) * u64::from(h);
            if area > best.1 { (index, area) } else { best }
        })
        .0
}

/// `(width, height)` scaled down to fit `max_side` on both axes, keeping the aspect ratio.
pub(crate) fn fit_within(width: u32, height: u32, max_side: u32) -> (u32, u32) {
    if max_side == 0 || (width <= max_side && height <= max_side) {
        return (width, height);
    }
    let scale = f64::from(max_side) / f64::from(width.max(height));
    (
        ((f64::from(width) * scale).round() as u32).max(1),
        ((f64::from(height) * scale).round() as u32).max(1),
    )
}

pub(crate) fn container_name(guid: &GUID) -> Option<&'static str> {
    [
        (GUID_ContainerFormatPng, "PNG"),
        (GUID_ContainerFormatJpeg, "JPEG"),
        (GUID_ContainerFormatGif, "GIF"),
        (GUID_ContainerFormatBmp, "BMP"),
        (GUID_ContainerFormatIco, "ICO"),
        (GUID_ContainerFormatTiff, "TIFF"),
        (GUID_ContainerFormatWebp, "WebP"),
        (GUID_ContainerFormatHeif, "HEIF"),
    ]
    .iter()
    .find(|(known, _)| known == guid)
    .map(|(_, name)| *name)
}

fn wic_factory() -> Result<IWICImagingFactory, ImageError> {
    unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER) }
        .map_err(|error| ImageError::Read(error.message()))
}

pub fn decode_file(path: &Path, max_side: u32) -> Result<FullImage, ImageError> {
    let _com = ComScope::enter();
    let extension = path.extension().map(|e| e.to_string_lossy().into_owned());
    let fail = |error: windows::core::Error| classify(error.code().0, extension.as_deref());
    if !path.exists() {
        return Err(ImageError::Missing);
    }
    let factory = wic_factory()?;
    let wide = path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<u16>>();
    unsafe {
        let decoder = factory
            .CreateDecoderFromFilename(
                PCWSTR(wide.as_ptr()),
                None,
                GENERIC_READ,
                WICDecodeMetadataCacheOnDemand,
            )
            .map_err(fail)?;
        let format = decoder
            .GetContainerFormat()
            .ok()
            .and_then(|guid| container_name(&guid))
            .unwrap_or("Image");
        let count = decoder.GetFrameCount().map_err(fail)?.max(1);
        let index = if format == "ICO" {
            let sizes = (0..count)
                .map(|index| {
                    let (mut w, mut h) = (0, 0);
                    if let Ok(frame) = decoder.GetFrame(index) {
                        let _ = frame.GetSize(&mut w, &mut h);
                    }
                    (w, h)
                })
                .collect::<Vec<_>>();
            largest_frame(&sizes) as u32
        } else {
            0
        };
        let frame = decoder.GetFrame(index).map_err(fail)?;
        let (mut width, mut height) = (0_u32, 0_u32);
        frame.GetSize(&mut width, &mut height).map_err(fail)?;
        if width == 0 || height == 0 {
            return Err(ImageError::Damaged);
        }
        if u64::from(width) * u64::from(height) > MAX_IMAGE_PIXELS {
            return Err(ImageError::TooLarge);
        }
        let mut source: IWICBitmapSource = frame.cast().map_err(fail)?;
        let (transform, swaps) = orientation_transform(read_orientation(&frame));
        if transform != WICBitmapTransformRotate0 {
            let rotator = factory.CreateBitmapFlipRotator().map_err(fail)?;
            rotator.Initialize(&source, transform).map_err(fail)?;
            source = rotator.cast().map_err(fail)?;
            if swaps {
                (width, height) = (height, width);
            }
        }
        let (natural_width, natural_height) = (width, height);
        let (scaled_width, scaled_height) = fit_within(width, height, max_side);
        if (scaled_width, scaled_height) != (width, height) {
            let scaler = factory.CreateBitmapScaler().map_err(fail)?;
            scaler
                .Initialize(
                    &source,
                    scaled_width,
                    scaled_height,
                    WICBitmapInterpolationModeFant,
                )
                .map_err(fail)?;
            source = scaler.cast().map_err(fail)?;
            (width, height) = (scaled_width, scaled_height);
        }
        let converter = factory.CreateFormatConverter().map_err(fail)?;
        converter
            .Initialize(
                &source,
                &GUID_WICPixelFormat32bppPBGRA,
                WICBitmapDitherTypeNone,
                None,
                0.0,
                WICBitmapPaletteTypeCustom,
            )
            .map_err(fail)?;
        let mut pixels = vec![0_u8; width as usize * height as usize * 4];
        converter
            .CopyPixels(std::ptr::null(), width * 4, &mut pixels)
            .map_err(fail)?;
        Ok(FullImage {
            width,
            height,
            natural_width,
            natural_height,
            format,
            pixels,
            diagram: None,
            tree: None,
            vector: false,
        })
    }
}

/// EXIF orientation from `System.Photo.Orientation`, or 1 when the frame has none.
fn read_orientation(frame: &IWICBitmapFrameDecode) -> u16 {
    unsafe {
        let Ok(reader) = frame.GetMetadataQueryReader() else {
            return 1;
        };
        let mut value = PROPVARIANT::default();
        if reader
            .GetMetadataByName(w!("System.Photo.Orientation"), &mut value)
            .is_err()
        {
            return 1;
        }
        let raw = &value.Anonymous.Anonymous;
        let orientation = if raw.vt == VT_UI2 {
            raw.Anonymous.uiVal
        } else {
            1
        };
        let _ = PropVariantClear(&mut value);
        orientation
    }
}

/// The pixel size an SVG of `natural` size renders at for `target_width` (0: natural): the
/// aspect ratio kept, within `max_side`, and shrunk to at most `budget` pixels so a deep zoom
/// renders a smaller bitmap that is drawn enlarged instead of failing.
pub(crate) fn svg_render_size(
    (natural_width, natural_height): (u32, u32),
    target_width: u32,
    max_side: u32,
    budget: u64,
) -> (u32, u32) {
    let width = if target_width == 0 {
        natural_width
    } else {
        target_width
    };
    let height = ((u64::from(natural_height) * u64::from(width)) / u64::from(natural_width.max(1)))
        .max(1) as u32;
    let (width, height) = fit_within(width, height, max_side);
    let budget = budget.min(MAX_IMAGE_PIXELS);
    let area = u64::from(width) * u64::from(height);
    if area <= budget {
        return (width, height);
    }
    let shrink = (budget as f64 / area as f64).sqrt();
    (
        ((f64::from(width) * shrink).floor() as u32).max(1),
        ((f64::from(height) * shrink).floor() as u32).max(1),
    )
}

pub fn decode_svg_source(
    text: &str,
    target_width: u32,
    tile: Option<TileJob>,
    max_side: u32,
) -> Result<(FullImage, Option<Tile>), ImageError> {
    svg_source(text, target_width, tile, max_side, false)
}

fn svg_source(
    text: &str,
    target_width: u32,
    tile: Option<TileJob>,
    max_side: u32,
    gpu: bool,
) -> Result<(FullImage, Option<Tile>), ImageError> {
    rasterize_svg(text, "SVG", (target_width, tile, max_side), gpu)
}

/// Draws a FastDraw diagram and rasterizes it like an SVG. Errors that hide the drawing fail the
/// decode; a refused look file leaves the drawing with its error for the band.
pub fn decode_fastdraw(
    text: &str,
    file: Option<&Path>,
    dark: bool,
    target_width: u32,
    tile: Option<TileJob>,
    max_side: u32,
) -> Result<(FullImage, Option<Tile>), ImageError> {
    fastdraw_source(text, file, dark, (target_width, tile, max_side), false)
}

/// The zoom a picture rasterizes at: the whole picture's width, the sharp tile, and the target's
/// largest bitmap side.
type RasterJob = (u32, Option<TileJob>, u32);

fn fastdraw_source(
    text: &str,
    file: Option<&Path>,
    dark: bool,
    job: RasterJob,
    gpu: bool,
) -> Result<(FullImage, Option<Tile>), ImageError> {
    let drawn = crate::preview::fastdraw::draw(text, file, dark);
    let band = crate::preview::fastdraw::band_text(&drawn.errors);
    if !drawn.show {
        return Err(ImageError::Diagram(band.unwrap_or_default()));
    }
    let (mut image, tile) = rasterize_svg(&drawn.svg, "FastDraw", job, gpu)?;
    image.diagram = Some(DiagramFacts {
        paper: drawn.paper,
        band,
        hits: drawn.hits,
    });
    Ok((image, tile))
}

/// Parses `text` and rasterizes it for `job`, unless `gpu` and Direct2D can draw the whole tree:
/// that comes back with no pixels, to be drawn at every paint.
fn rasterize_svg(
    text: &str,
    format: &'static str,
    (target_width, tile, max_side): RasterJob,
    gpu: bool,
) -> Result<(FullImage, Option<Tile>), ImageError> {
    if text.len() as u64 > MAX_SVG_BYTES {
        return Err(ImageError::SvgTooLarge);
    }
    let tree = parse(text).map_err(|_| ImageError::Damaged)?;
    let natural = natural_size(&tree);
    if gpu && super::vector::drawable(&tree) {
        return Ok((
            FullImage {
                width: 0,
                height: 0,
                natural_width: natural.0,
                natural_height: natural.1,
                format,
                pixels: Vec::new(),
                diagram: None,
                tree: Some(Arc::new(tree)),
                vector: true,
            },
            None,
        ));
    }
    let size = svg_render_size(natural, target_width, max_side, WHOLE_PIXEL_BUDGET);
    let (whole, tile) = render_parts(&tree, natural, Some(size), tile);
    let (width, height, pixels) = whole.expect("asked for the whole picture")?;
    Ok((
        FullImage {
            width,
            height,
            natural_width: natural.0,
            natural_height: natural.1,
            format,
            pixels,
            diagram: None,
            tree: Some(Arc::new(tree)),
            vector: false,
        },
        tile,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_deeply_zoomed_svg_renders_within_its_budget() {
        // Break caught: a valid 800×600 SVG at 1600% showing "Can't render this SVG", or a deep
        // zoom rasterizing the whole picture at the zoom (seconds per step in a debug build).
        let (width, height) = svg_render_size((800, 600), 12_800, 16_384, MAX_IMAGE_PIXELS);
        assert!(u64::from(width) * u64::from(height) <= MAX_IMAGE_PIXELS);
        assert!(width > 8_000 && (width * 3).abs_diff(height * 4) < 8);
        let (width, height) = svg_render_size((800, 600), 12_800, 16_384, WHOLE_PIXEL_BUDGET);
        assert!(u64::from(width) * u64::from(height) <= WHOLE_PIXEL_BUDGET);
        assert!(width > 2_000 && (width * 3).abs_diff(height * 4) < 8);
        assert_eq!(
            svg_render_size((40, 20), 0, 16_384, WHOLE_PIXEL_BUDGET),
            (40, 20)
        );
        assert_eq!(
            svg_render_size((40, 20), 80, 16_384, WHOLE_PIXEL_BUDGET),
            (80, 40)
        );
    }

    #[test]
    fn only_a_zoom_past_the_budget_is_tiled() {
        // Break caught: a modest zoom drawn from a blurred whole picture plus a tile, or a deep one
        // rasterized whole.
        assert!(!tiled((1044, 752), 1044, 16_384));
        assert!(!tiled((1044, 752), 2_000, 16_384));
        assert!(tiled((1044, 752), 8_352, 16_384));
        // Past the largest bitmap the target holds, even a small area is tiled.
        assert!(tiled((40_000, 10), 40_000, 16_384));
    }

    #[test]
    fn a_tile_covers_the_view_and_its_margin_within_the_picture() {
        // Break caught: a tile that leaves the view's edge blurred, reaches past the picture, or
        // asks for more than the target's largest bitmap.
        let view = (1000.0, 700.0);
        // 800% of 1044×752, scrolled to (3000.5, 2000.25).
        let picture = (8352.0, 6016.0);
        let area = tile_area(picture, (-3000.5, -2000.25), view, 256.0, 16_384).unwrap();
        assert_eq!(
            area,
            TileArea {
                x: 2744,
                y: 1744,
                width: 1513,
                height: 1213,
            }
        );
        let visible = tile_area(picture, (-3000.5, -2000.25), view, 0.0, 16_384).unwrap();
        assert!(area.contains(&visible));
        // At the top-left corner the margin is cut at the picture's edge.
        let corner = tile_area(picture, (0.0, 0.0), view, 256.0, 16_384).unwrap();
        assert_eq!(
            (corner.x, corner.y, corner.width, corner.height),
            (0, 0, 1256, 956)
        );
        // At the bottom-right too.
        let end = tile_area(
            picture,
            (1000.0 - 8352.0, 700.0 - 6016.0),
            view,
            256.0,
            16_384,
        )
        .unwrap();
        assert_eq!((end.x + end.width, end.y + end.height), (8352, 6016));
        // A picture narrower than the view (centered) is covered across.
        let narrow = tile_area((600.0, 6016.0), (200.0, -100.0), view, 256.0, 16_384).unwrap();
        assert_eq!((narrow.x, narrow.width), (0, 600));
        // A tile never asks for more than the target holds.
        let huge = tile_area(picture, (-10.0, -10.0), (20_000.0, 700.0), 256.0, 4_096).unwrap();
        assert_eq!(huge.width, 4_096);
        // Scrolled out of view: nothing to render.
        assert_eq!(
            tile_area(picture, (-9000.0, 0.0), view, 256.0, 16_384),
            None
        );
    }

    #[test]
    fn a_tile_matches_the_same_pixels_of_the_whole_picture() {
        // Break caught: a tile offset, scaled or flipped against the picture it sharpens.
        let source = r##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20">
            <rect width="20" height="10" fill="#f00"/><rect x="20" y="10" width="20" height="10" fill="#00f"/>
            <circle cx="25" cy="5" r="3" fill="#0f0"/></svg>"##;
        let (image, tile) = decode_svg_source(
            source,
            160,
            Some((
                4.0,
                TileArea {
                    x: 70,
                    y: 6,
                    width: 50,
                    height: 40,
                },
            )),
            16_384,
        )
        .unwrap();
        let tile = tile.expect("a tile");
        assert_eq!((image.width, image.height), (160, 80));
        assert_eq!(tile.pixels.len(), 50 * 40 * 4);
        for y in 0..40 {
            for x in 0..50 {
                let at = |pixels: &[u8], stride: u32, x: u32, y: u32| {
                    let i = ((y * stride + x) * 4) as usize;
                    pixels[i..i + 4].to_vec()
                };
                assert_eq!(
                    at(&tile.pixels, 50, x, y),
                    at(&image.pixels, 160, 70 + x, 6 + y),
                    "pixel {x},{y}"
                );
            }
        }
    }

    /// Times a deep zoom of `FASTPAD_TIMING_FDRAW` (a FastDraw file): run with
    /// `cargo test --release timing -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn timing_of_a_deep_zoom() {
        let Some(path) = std::env::var_os("FASTPAD_TIMING_FDRAW") else {
            return;
        };
        let path = PathBuf::from(path);
        let text = std::fs::read_to_string(&path).unwrap();
        let start = std::time::Instant::now();
        let (image, _) = decode_fastdraw(&text, Some(&path), false, 0, None, 16_384).unwrap();
        println!(
            "decode at 100%: {:?} ({}x{})",
            start.elapsed(),
            image.natural_width,
            image.natural_height
        );
        let natural = (image.natural_width, image.natural_height);
        let tree = image.tree.unwrap();
        for scale in [1.0_f32, 4.0, 8.0, 16.0] {
            let width = (natural.0 as f32 * scale).round() as u32;
            let picture = (natural.0 as f32 * scale, natural.1 as f32 * scale);
            let offset = (-(picture.0 - 1600.0) / 2.0, -(picture.1 - 1000.0) / 2.0);
            let tile = tiled(natural, width, 16_384)
                .then(|| tile_area(picture, offset, (1600.0, 1000.0), TILE_MARGIN, 16_384))
                .flatten()
                .map(|area| (scale, area));
            let whole = svg_render_size(natural, width, 16_384, WHOLE_PIXEL_BUDGET);
            let start = std::time::Instant::now();
            render_parts(&tree, natural, Some(whole), tile);
            let both = start.elapsed();
            let start = std::time::Instant::now();
            render_parts(&tree, natural, None, tile);
            println!(
                "{}%: whole {:?} + tile {:?}: {both:?}; tile alone (a pan): {:?}",
                scale * 100.0,
                whole,
                tile.map(|(_, area)| (area.width, area.height)),
                start.elapsed()
            );
        }
    }

    #[test]
    fn classify_maps_decoder_errors_to_messages() {
        // Break caught: a HEIC with no Store codec called "damaged", or a PNG that WIC can't
        // read called "no decoder".
        assert_eq!(
            classify(COMPONENT_NOT_FOUND, Some("HEIC")),
            ImageError::NoCodec
        );
        assert_eq!(
            classify(COMPONENT_NOT_FOUND, Some("png")),
            ImageError::Damaged
        );
        assert_eq!(
            classify(0x8007_0002_u32 as i32, Some("png")),
            ImageError::Missing
        );
        assert!(matches!(
            classify(0x8007_0005_u32 as i32, None),
            ImageError::Read(_)
        ));
    }

    #[test]
    fn orientation_rotates_and_swaps_for_the_quarter_turns() {
        // Break caught: a portrait phone photo shown sideways, or stretched to landscape.
        assert_eq!(orientation_transform(6), (WICBitmapTransformRotate90, true));
        assert_eq!(
            orientation_transform(8),
            (WICBitmapTransformRotate270, true)
        );
        assert_eq!(
            orientation_transform(3),
            (WICBitmapTransformRotate180, false)
        );
        assert_eq!(orientation_transform(0), (WICBitmapTransformRotate0, false));
        assert!(orientation_transform(5).1 && orientation_transform(7).1);
    }

    #[test]
    fn the_largest_icon_frame_wins_and_huge_images_fit_the_maximum_bitmap() {
        // Break caught: an .ico shown as its 16×16 frame, or a 20000-px panorama failing to create
        // a Direct2D bitmap.
        assert_eq!(largest_frame(&[(16, 16), (256, 256), (32, 32)]), 1);
        assert_eq!(largest_frame(&[]), 0);
        assert_eq!(fit_within(20_000, 5_000, 16_384), (16_384, 4_096));
        assert_eq!(fit_within(800, 600, 16_384), (800, 600));
    }

    #[test]
    fn decoding_a_png_and_an_svg_gives_premultiplied_pixels_and_their_format() {
        // Break caught: the viewer receiving a scaled or unformatted decode.
        let path = std::env::temp_dir().join(format!("fastpad-full-{}.png", std::process::id()));
        std::fs::write(&path, crate::preview::images::PNG_2X2).unwrap();
        let image = decode_file(&path, 16_384).unwrap();
        assert_eq!((image.width, image.height, image.format), (2, 2, "PNG"));
        assert_eq!(image.pixels.len(), 16);
        std::fs::remove_file(&path).unwrap();
        let (svg, tile) = decode_svg_source(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20"><rect width="40" height="20"/></svg>"#,
            80,
            None,
            16_384,
        )
        .unwrap();
        assert!(tile.is_none() && svg.tree.is_some());
        assert_eq!((svg.width, svg.height, svg.natural_width), (80, 40, 40));
        assert_eq!(
            decode_file(&std::env::temp_dir().join("fastpad-missing.png"), 0).err(),
            Some(ImageError::Missing)
        );
    }

    #[test]
    fn a_fastdraw_diagram_decodes_with_its_paper_and_no_band() {
        // Break caught: a diagram drawn without its paper colour, or a clean one with a band.
        let image = decode_fastdraw(
            "a: rect \"A\" at 0,0
",
            None,
            false,
            0,
            None,
            16_384,
        )
        .unwrap()
        .0;
        let facts = image.diagram.expect("diagram facts");
        assert_eq!(facts.band, None);
        assert_eq!(image.format, "FastDraw");
        assert!(
            image.width > 0
                && image.pixels.len() == image.width as usize * image.height as usize * 4
        );
        let dark = decode_fastdraw(
            "a: rect \"A\" at 0,0
",
            None,
            true,
            0,
            None,
            16_384,
        )
        .unwrap()
        .0;
        assert_ne!(dark.diagram.unwrap().paper, facts.paper);
    }

    #[test]
    fn a_refused_look_still_draws_with_its_error_for_the_band() {
        // Break caught: a diagram naming an absolute look path blanking, or its refusal unreported.
        let text = "look \"//server/x.look\"
a: rect \"A\" at 0,0
";
        let (image, _) = decode_fastdraw(text, None, false, 0, None, 16_384).unwrap();
        assert_eq!(
            image.diagram.unwrap().band.as_deref(),
            Some(
                "Line 1: FastPad reads look files only by a relative path; \"//server/x.look\" was not read"
            )
        );
    }

    #[test]
    fn a_broken_diagram_fails_with_its_line_and_the_diagram_title() {
        // Break caught: a broken diagram shown as "damaged image", or without its line number.
        let error = decode_fastdraw(
            "a: nosuchkind \"A\" at 0,0
",
            None,
            false,
            0,
            None,
            16_384,
        )
        .err()
        .expect("an error");
        let ImageError::Diagram(text) = &error else {
            panic!("{error:?}");
        };
        assert!(text.starts_with("Line 1: "), "{text}");
        assert_eq!(error.message(), *text);
        assert_eq!(error.title(), "FastPad can't draw this diagram");
        assert_eq!(
            ImageError::Damaged.title(),
            "FastPad can't display this image"
        );
    }
}
