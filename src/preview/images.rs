//! Image decoding off the UI thread. A short-lived worker decodes queued files with WIC into
//! premultiplied BGRA pixels and posts `message` to `notify`; the UI thread drains results and
//! creates Direct2D bitmaps on demand, keeping the pixels so bitmaps can be rebuilt after device
//! loss. The same worker renders Mermaid diagrams (`preview::mermaid`), keyed by their source.

use crate::preview::dwrite::hresult_error;
use crate::preview::mermaid::{self, DiagramKey};
use crate::{FastPadError, Result};
use std::collections::{HashMap, VecDeque};
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use windows::Win32::Foundation::GENERIC_READ;
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_SIZE_U, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{D2D1_BITMAP_PROPERTIES, ID2D1Bitmap, ID2D1RenderTarget};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_WICPixelFormat32bppPBGRA, IWICBitmapSource, IWICImagingFactory,
    WICBitmapDitherTypeNone, WICBitmapInterpolationModeFant, WICBitmapPaletteTypeCustom,
    WICDecodeMetadataCacheOnDemand,
};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
};
use windows::core::{Interface, PCWSTR};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW;

pub const MAX_IMAGE_PIXELS: u64 = 64_000_000;

/// A 2x2 opaque red PNG.
#[cfg(test)]
pub(crate) const PNG_2X2: [u8; 125] = [
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x02, 0x08, 0x06, 0x00, 0x00, 0x00, 0x72, 0xb6, 0x0d,
    0x24, 0x00, 0x00, 0x00, 0x01, 0x73, 0x52, 0x47, 0x42, 0x00, 0xae, 0xce, 0x1c, 0xe9, 0x00, 0x00,
    0x00, 0x04, 0x67, 0x41, 0x4d, 0x41, 0x00, 0x00, 0xb1, 0x8f, 0x0b, 0xfc, 0x61, 0x05, 0x00, 0x00,
    0x00, 0x09, 0x70, 0x48, 0x59, 0x73, 0x00, 0x00, 0x0e, 0xc3, 0x00, 0x00, 0x0e, 0xc3, 0x01, 0xc7,
    0x6f, 0xa8, 0x64, 0x00, 0x00, 0x00, 0x12, 0x49, 0x44, 0x41, 0x54, 0x18, 0x57, 0x63, 0xf8, 0xcf,
    0xc0, 0xf0, 0x1f, 0x84, 0xa1, 0x0c, 0x86, 0xff, 0x00, 0x47, 0xca, 0x07, 0xf9, 0xac, 0x78, 0x42,
    0xbc, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

pub struct DecodedImage {
    /// Decoded pixel size; smaller than the natural size when the decode was scaled down.
    pub width: u32,
    pub height: u32,
    /// The file's own pixel size, which layout uses so an image's DIP size never depends on how
    /// large it happened to be decoded.
    pub natural_width: u32,
    pub natural_height: u32,
    pub pixels: Vec<u8>,
}

pub(crate) struct ComScope(bool);

impl ComScope {
    pub(crate) fn enter() -> Self {
        Self(unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok())
    }
}

impl Drop for ComScope {
    fn drop(&mut self) {
        if self.0 {
            unsafe { CoUninitialize() };
        }
    }
}

pub fn decode_image(path: &Path, max_width: u32) -> Result<DecodedImage> {
    if path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("svg"))
    {
        return crate::preview::svg::decode_svg(path, max_width);
    }
    let _com = ComScope::enter();
    let factory: IWICImagingFactory =
        unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER) }
            .map_err(hresult_error)?;
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
            .map_err(hresult_error)?;
        let frame = decoder.GetFrame(0).map_err(hresult_error)?;
        let (mut width, mut height) = (0_u32, 0_u32);
        frame
            .GetSize(&mut width, &mut height)
            .map_err(hresult_error)?;
        if width == 0 || height == 0 || u64::from(width) * u64::from(height) > MAX_IMAGE_PIXELS {
            return Err(FastPadError::Invariant(
                "image is empty or larger than 64 megapixels",
            ));
        }
        let (natural_width, natural_height) = (width, height);
        let mut source: IWICBitmapSource = frame.cast().map_err(hresult_error)?;
        if max_width > 0 && width > max_width {
            let scaled_height =
                ((u64::from(height) * u64::from(max_width)) / u64::from(width)).max(1) as u32;
            let scaler = factory.CreateBitmapScaler().map_err(hresult_error)?;
            scaler
                .Initialize(
                    &source,
                    max_width,
                    scaled_height,
                    WICBitmapInterpolationModeFant,
                )
                .map_err(hresult_error)?;
            source = scaler.cast().map_err(hresult_error)?;
            (width, height) = (max_width, scaled_height);
        }
        let converter = factory.CreateFormatConverter().map_err(hresult_error)?;
        converter
            .Initialize(
                &source,
                &GUID_WICPixelFormat32bppPBGRA,
                WICBitmapDitherTypeNone,
                None,
                0.0,
                WICBitmapPaletteTypeCustom,
            )
            .map_err(hresult_error)?;
        let mut pixels = vec![0_u8; width as usize * height as usize * 4];
        converter
            .CopyPixels(std::ptr::null(), width * 4, &mut pixels)
            .map_err(hresult_error)?;
        Ok(DecodedImage {
            width,
            height,
            natural_width,
            natural_height,
            pixels,
        })
    }
}

/// Diagrams kept beyond those on screen: each edit of a diagram's source renders a new one.
const MAX_DIAGRAMS: usize = 32;

enum EntryState {
    Pending,
    Ready(DecodedImage),
    /// Why, for diagrams; files show their alt text instead.
    Failed(String),
}

struct Entry {
    state: EntryState,
    bitmap: Option<ID2D1Bitmap>,
    /// The largest `max_width` queued so far (0 = unlimited).
    requested: u32,
    /// Pixels per natural unit wanted: 1 for files, the DPI scale for diagrams, which are vector
    /// and can be rasterized past their natural size.
    scale: f32,
    /// When a diagram was last requested or drawn, for evicting the oldest.
    used: u64,
}

/// A diagram's state, for layout.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DiagramState<'a> {
    /// Never requested, or evicted.
    Unknown,
    Pending,
    /// Natural size in DIPs.
    Ready(u32, u32),
    Failed(&'a str),
}

enum Job {
    File(PathBuf),
    Diagram(DiagramKey, f32),
}

/// Whether a decode limited to `requested` pixels already satisfies a limit of `max_width`.
fn covers(requested: u32, max_width: u32) -> bool {
    requested == 0 || (max_width != 0 && requested >= max_width)
}

impl Entry {
    /// True when a (new) decode is needed for `max_width` device pixels. A ready image is decoded
    /// again only when it was scaled below both the natural width and the new limit.
    fn wants(&self, max_width: u32) -> bool {
        match &self.state {
            EntryState::Failed(_) => false,
            EntryState::Pending => !covers(self.requested, max_width),
            EntryState::Ready(image) => {
                let natural = (image.natural_width as f32 * self.scale).round() as u32;
                let target = if max_width == 0 {
                    natural
                } else {
                    max_width.min(natural)
                };
                image.width < target && !covers(self.requested, target)
            }
        }
    }

    fn new(state: EntryState, requested: u32, scale: f32, used: u64) -> Self {
        Self {
            state,
            bitmap: None,
            requested,
            scale,
            used,
        }
    }

    fn bitmap(&mut self, target: &ID2D1RenderTarget) -> Option<ID2D1Bitmap> {
        let EntryState::Ready(image) = &self.state else {
            return None;
        };
        if self.bitmap.is_none() {
            let properties = D2D1_BITMAP_PROPERTIES {
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: 96.0,
                dpiY: 96.0,
            };
            self.bitmap = unsafe {
                target.CreateBitmap(
                    D2D_SIZE_U {
                        width: image.width,
                        height: image.height,
                    },
                    Some(image.pixels.as_ptr().cast()),
                    image.width * 4,
                    &properties,
                )
            }
            .ok();
        }
        self.bitmap.clone()
    }

    /// Takes a finished decode. A result no larger than the image already shown is dropped, and a
    /// failed re-decode keeps the smaller image that already works.
    fn finish(&mut self, result: std::result::Result<DecodedImage, String>) {
        match (result, &self.state) {
            (Ok(image), EntryState::Ready(current)) if image.width <= current.width => {}
            (Ok(image), _) => {
                self.state = EntryState::Ready(image);
                self.bitmap = None;
            }
            (Err(_), EntryState::Ready(_)) => {}
            (Err(error), _) => self.state = EntryState::Failed(error),
        }
    }
}

#[derive(Default)]
struct Shared {
    queue: VecDeque<(Job, u32)>,
    done: Vec<(Job, std::result::Result<DecodedImage, String>)>,
    worker_running: bool,
}

pub struct ImageCache {
    entries: HashMap<PathBuf, Entry>,
    diagrams: HashMap<DiagramKey, Entry>,
    /// Advances on every diagram request and draw.
    clock: u64,
    shared: Arc<Mutex<Shared>>,
    /// `HWND` as an integer so the worker closure is `Send`.
    notify: isize,
    message: u32,
}

impl ImageCache {
    pub fn new(notify: HWND, message: u32) -> Self {
        Self {
            entries: HashMap::new(),
            diagrams: HashMap::new(),
            clock: 0,
            shared: Arc::new(Mutex::new(Shared::default())),
            notify: notify as isize,
            message,
        }
    }

    /// Queues a decode of `path` at most `max_width` pixels wide (0 = natural size). Repeated
    /// requests are free unless the image must be decoded larger than before.
    pub fn request(&mut self, path: &Path, max_width: u32) {
        if let Some(entry) = self.entries.get_mut(path) {
            if !entry.wants(max_width) {
                return;
            }
            // Keep a ready image (and its size) on screen while the sharper decode runs.
            entry.requested = max_width;
        } else {
            let exists = std::fs::metadata(path).is_ok_and(|metadata| metadata.is_file());
            let state = if exists {
                EntryState::Pending
            } else {
                EntryState::Failed(String::new())
            };
            self.entries
                .insert(path.to_owned(), Entry::new(state, max_width, 1.0, 0));
            if !exists {
                return;
            }
        }
        self.queue(Job::File(path.to_owned()), max_width);
    }

    /// Queues a render of `key` at `scale` device pixels per DIP, at most `max_width` pixels wide
    /// (0 = no limit). Repeated requests are free unless the diagram must be drawn larger.
    pub fn request_diagram(&mut self, key: &DiagramKey, max_width: u32, scale: f32) {
        self.clock += 1;
        if let Some(entry) = self.diagrams.get_mut(key) {
            entry.used = self.clock;
            // A higher DPI needs more pixels even where the last render had no width limit.
            let sharper = scale > entry.scale && !matches!(entry.state, EntryState::Failed(_));
            entry.scale = entry.scale.max(scale);
            if !sharper && !entry.wants(max_width) {
                return;
            }
            entry.requested = max_width;
        } else {
            self.evict_diagrams();
            self.diagrams.insert(
                key.clone(),
                Entry::new(EntryState::Pending, max_width, scale, self.clock),
            );
        }
        let scale = self.diagrams[key].scale;
        self.queue(Job::Diagram(key.clone(), scale), max_width);
    }

    /// Drops the least recently used diagrams beyond `MAX_DIAGRAMS`. Pending ones stay: their
    /// results are on the way.
    fn evict_diagrams(&mut self) {
        while self.diagrams.len() >= MAX_DIAGRAMS {
            let oldest = self
                .diagrams
                .iter()
                .filter(|(_, entry)| !matches!(entry.state, EntryState::Pending))
                .min_by_key(|(_, entry)| entry.used)
                .map(|(key, _)| key.clone());
            let Some(oldest) = oldest else {
                return;
            };
            self.diagrams.remove(&oldest);
        }
    }

    fn queue(&mut self, job: Job, max_width: u32) {
        let spawn = {
            let mut shared = self
                .shared
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            shared.queue.push_back((job, max_width));
            !std::mem::replace(&mut shared.worker_running, true)
        };
        if spawn {
            let shared = Arc::clone(&self.shared);
            let (notify, message) = (self.notify, self.message);
            std::thread::spawn(move || {
                loop {
                    let job = {
                        let mut state = shared.lock().unwrap_or_else(|error| error.into_inner());
                        let job = state.queue.pop_front();
                        if job.is_none() {
                            state.worker_running = false;
                        }
                        job
                    };
                    let Some((job, max_width)) = job else {
                        return;
                    };
                    let result = match &job {
                        Job::File(path) => {
                            decode_image(path, max_width).map_err(|error| error.to_string())
                        }
                        Job::Diagram(key, scale) => mermaid::render(key, max_width, *scale),
                    };
                    shared
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .done
                        .push((job, result));
                    unsafe { PostMessageW(notify as HWND, message, 0, 0) };
                }
            });
        }
    }

    /// Moves finished decodes into the cache; true when anything changed.
    pub fn drain(&mut self) -> bool {
        let done = std::mem::take(
            &mut self
                .shared
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .done,
        );
        let changed = !done.is_empty();
        for (job, result) in done {
            let entry = match &job {
                Job::File(path) => self.entries.get_mut(path),
                Job::Diagram(key, _) => self.diagrams.get_mut(key),
            };
            if let Some(entry) = entry {
                entry.finish(result);
            }
        }
        changed
    }

    pub fn diagram(&self, key: &DiagramKey) -> DiagramState<'_> {
        match self.diagrams.get(key).map(|entry| &entry.state) {
            None => DiagramState::Unknown,
            Some(EntryState::Pending) => DiagramState::Pending,
            Some(EntryState::Ready(image)) => {
                DiagramState::Ready(image.natural_width, image.natural_height)
            }
            Some(EntryState::Failed(error)) => DiagramState::Failed(error),
        }
    }

    pub fn diagram_bitmap(
        &mut self,
        target: &ID2D1RenderTarget,
        key: &DiagramKey,
    ) -> Option<ID2D1Bitmap> {
        self.clock += 1;
        let entry = self.diagrams.get_mut(key)?;
        entry.used = self.clock;
        entry.bitmap(target)
    }

    /// The image's natural pixel size once decoded, whatever size it was decoded at.
    pub fn size(&self, path: &Path) -> Option<(u32, u32)> {
        match &self.entries.get(path)?.state {
            EntryState::Ready(image) => Some((image.natural_width, image.natural_height)),
            _ => None,
        }
    }

    pub fn is_failed(&self, path: &Path) -> bool {
        matches!(
            self.entries.get(path).map(|entry| &entry.state),
            Some(EntryState::Failed(_))
        )
    }

    pub fn bitmap(&mut self, target: &ID2D1RenderTarget, path: &Path) -> Option<ID2D1Bitmap> {
        self.entries.get_mut(path)?.bitmap(target)
    }

    /// Device loss: bitmaps belong to the lost device; pixels stay for rebuilding.
    pub fn release_bitmaps(&mut self) {
        for entry in self.entries.values_mut().chain(self.diagrams.values_mut()) {
            entry.bitmap = None;
        }
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.diagrams.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_png(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("fastpad-{name}-{}.png", std::process::id()));
        std::fs::write(&path, PNG_2X2).unwrap();
        path
    }

    #[test]
    fn decodes_png_to_premultiplied_bgra() {
        let path = write_png("decode");
        let image = decode_image(&path, 0).unwrap();
        assert_eq!((image.width, image.height), (2, 2));
        assert_eq!(image.pixels.len(), 16);
        assert_eq!(&image.pixels[..4], &[0, 0, 255, 255]);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn scales_down_to_the_maximum_width() {
        let path = write_png("scale");
        let image = decode_image(&path, 1).unwrap();
        assert_eq!((image.width, image.height), (1, 1));
        assert_eq!((image.natural_width, image.natural_height), (2, 2));
        let _ = std::fs::remove_file(path);
    }

    fn decoded_width(cache: &ImageCache, path: &Path) -> Option<u32> {
        match &cache.entries.get(path)?.state {
            EntryState::Ready(image) => Some(image.width),
            _ => None,
        }
    }

    fn wait_for(cache: &mut ImageCache, path: &Path, width: u32) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while decoded_width(cache, path) != Some(width) && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
            cache.drain();
        }
        assert_eq!(decoded_width(cache, path), Some(width));
    }

    #[test]
    fn missing_files_fail() {
        assert!(decode_image(Path::new(r"C:\definitely\missing.png"), 0).is_err());
    }

    #[test]
    fn the_cache_decodes_on_a_worker_and_reports_sizes() {
        let path = write_png("cache");
        let missing = PathBuf::from(r"C:\definitely\missing.png");
        let mut cache = ImageCache::new(std::ptr::null_mut(), 0);
        cache.request(&path, 1);
        cache.request(&missing, 0);
        wait_for(&mut cache, &path, 1);
        // The size is the natural size even though the decode was scaled to one pixel.
        assert_eq!(cache.size(&path), Some((2, 2)));
        assert!(cache.is_failed(&missing));
        // A narrower request reuses the decode; a wider one decodes again at the larger size.
        cache.request(&path, 1);
        assert_eq!(cache.entries[&path].requested, 1);
        cache.request(&path, 0);
        wait_for(&mut cache, &path, 2);
        assert_eq!(cache.size(&path), Some((2, 2)));
        // Fully decoded images never decode again.
        cache.request(&path, 8);
        assert_eq!(cache.entries[&path].requested, 0);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn the_cache_decodes_svg_files_and_decodes_them_larger_on_request() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("assets")
            .join("fastpad-icon.svg");
        let mut cache = ImageCache::new(std::ptr::null_mut(), 0);
        cache.request(&path, 16);
        wait_for(&mut cache, &path, 16);
        assert_eq!(cache.size(&path), Some((244, 256)));
        cache.request(&path, 64);
        wait_for(&mut cache, &path, 64);
    }

    fn diagram(source: &str) -> DiagramKey {
        let colors =
            crate::preview::colors::preview_colors(crate::platform::theme::Theme::Light, false);
        DiagramKey::new(
            source,
            &crate::preview::mermaid::DiagramStyle::new(colors, "Segoe UI", 16.0),
        )
    }

    fn wait_for_diagram(cache: &mut ImageCache, key: &DiagramKey) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while matches!(cache.diagram(key), DiagramState::Pending)
            && std::time::Instant::now() < deadline
        {
            std::thread::sleep(std::time::Duration::from_millis(10));
            cache.drain();
        }
    }

    #[test]
    fn the_cache_renders_diagrams_at_the_dpi_scale_and_reports_failures() {
        let mut cache = ImageCache::new(std::ptr::null_mut(), 0);
        let key = diagram("flowchart LR\n  A --> B\n");
        assert_eq!(cache.diagram(&key), DiagramState::Unknown);
        cache.request_diagram(&key, 0, 1.5);
        assert_eq!(cache.diagram(&key), DiagramState::Pending);
        wait_for_diagram(&mut cache, &key);
        let DiagramState::Ready(width, height) = cache.diagram(&key) else {
            panic!("{:?}", cache.diagram(&key));
        };
        assert!(width > height);
        // Natural size in DIPs, pixels at the scale.
        let pixels = |cache: &ImageCache| match &cache.diagrams[&key].state {
            EntryState::Ready(image) => image.width,
            _ => 0,
        };
        assert!(pixels(&cache).abs_diff((width as f32 * 1.5).round() as u32) <= 1);
        // The same request is free; a higher DPI renders again, sharper.
        cache.request_diagram(&key, 0, 1.5);
        assert!(cache.shared.lock().unwrap().queue.is_empty());
        cache.request_diagram(&key, 0, 2.0);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while pixels(&cache) < 2 * width - 1 && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
            cache.drain();
        }
        assert!(pixels(&cache).abs_diff(2 * width) <= 1);

        let broken = diagram("flowchart LR\n  A -->\n");
        cache.request_diagram(&broken, 0, 1.0);
        wait_for_diagram(&mut cache, &broken);
        assert!(matches!(cache.diagram(&broken), DiagramState::Failed(error) if !error.is_empty()));
        cache.clear();
        assert_eq!(cache.diagram(&key), DiagramState::Unknown);
    }

    #[test]
    fn the_cache_keeps_only_the_most_recently_used_diagrams() {
        let mut cache = ImageCache::new(std::ptr::null_mut(), 0);
        let keys = (0..MAX_DIAGRAMS + 4)
            .map(|index| diagram(&format!("flowchart LR\n  A{index} --> B\n")))
            .collect::<Vec<_>>();
        for key in &keys {
            cache.request_diagram(key, 0, 1.0);
            wait_for_diagram(&mut cache, key);
            // The first stays in use, as a diagram on screen is.
            cache.request_diagram(&keys[0], 0, 1.0);
        }
        assert!(cache.diagrams.len() <= MAX_DIAGRAMS);
        assert!(matches!(cache.diagram(&keys[0]), DiagramState::Ready(..)));
        assert_eq!(cache.diagram(&keys[1]), DiagramState::Unknown);
        assert!(matches!(
            cache.diagram(keys.last().unwrap()),
            DiagramState::Ready(..)
        ));
    }
}
