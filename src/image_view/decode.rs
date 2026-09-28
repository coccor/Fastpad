//! Full-resolution decoding for the image view (image preview spec §6.2), on one short-lived
//! worker thread per request. Raster files go through WIC: the largest ICO frame, EXIF orientation
//! applied, downscaled only past the render target's maximum bitmap size. SVG source is rasterized
//! with Direct2D at the width the view asks for.

use crate::preview::images::{ComScope, MAX_IMAGE_PIXELS};
use crate::preview::svg::{MAX_SVG_BYTES, natural_size, rasterize_source};
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
            Self::Read(text) => text.clone(),
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
}

pub enum Source {
    File(PathBuf),
    Svg { text: Arc<str>, width: u32 },
}

pub struct Decoded {
    pub generation: u64,
    pub result: Result<FullImage, ImageError>,
}

/// Decodes `source` on a new thread and posts `message` to `notify` with a `Box<Decoded>` in
/// `lparam`, which the receiver frees.
pub fn spawn(generation: u64, source: Source, max_side: u32, notify: HWND, message: u32) {
    let notify = notify as isize;
    std::thread::spawn(move || {
        let result = match &source {
            Source::File(path) => decode_file(path, max_side),
            Source::Svg { text, width } => decode_svg_source(text, *width, max_side),
        };
        let boxed = Box::into_raw(Box::new(Decoded { generation, result }));
        if unsafe { PostMessageW(notify as HWND, message, 0, boxed as isize) } == 0 {
            drop(unsafe { Box::from_raw(boxed) });
        }
    });
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

pub fn decode_svg_source(
    text: &str,
    target_width: u32,
    max_side: u32,
) -> Result<FullImage, ImageError> {
    if text.len() as u64 > MAX_SVG_BYTES {
        return Err(ImageError::SvgTooLarge);
    }
    let natural = natural_size(text);
    let (natural_width, natural_height) = (natural.0.ceil() as u32, natural.1.ceil() as u32);
    if natural_width == 0 || natural_height == 0 {
        return Err(ImageError::Damaged);
    }
    let width = if target_width == 0 {
        natural_width
    } else {
        target_width
    };
    let height =
        ((u64::from(natural_height) * u64::from(width)) / u64::from(natural_width)).max(1) as u32;
    let (width, height) = fit_within(width, height, max_side);
    if u64::from(width) * u64::from(height) > MAX_IMAGE_PIXELS {
        return Err(ImageError::TooLarge);
    }
    let _com = ComScope::enter();
    let factory = wic_factory()?;
    let pixels = rasterize_source(&factory, text, natural, (width, height))
        .map_err(|_| ImageError::Damaged)?;
    Ok(FullImage {
        width,
        height,
        natural_width,
        natural_height,
        format: "SVG",
        pixels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let svg = decode_svg_source(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20"><rect width="40" height="20"/></svg>"#,
            80,
            16_384,
        )
        .unwrap();
        assert_eq!((svg.width, svg.height, svg.natural_width), (80, 40, 40));
        assert_eq!(
            decode_file(&std::env::temp_dir().join("fastpad-missing.png"), 0).err(),
            Some(ImageError::Missing)
        );
    }
}
