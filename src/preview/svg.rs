//! SVG images, rasterized on the image worker thread by resvg into the same premultiplied BGRA
//! pixels the WIC decoders produce. resvg covers static SVG 1.1 and the parts of SVG 2 that
//! exporters write: text in installed fonts, markers, masks, filters, patterns, `<style>`
//! sheets, and CSS colors. Like a browser showing an SVG as an image, it loads no external
//! resources; only `data:` images embedded in the file render.

use crate::preview::images::{DecodedImage, MAX_IMAGE_PIXELS};
use crate::{FastPadError, Result};
use resvg::usvg::{self, fontdb};
use std::path::Path;
use std::sync::{Arc, OnceLock};

pub const MAX_SVG_BYTES: u64 = 8 * 1024 * 1024;

pub fn decode_svg(path: &Path, max_width: u32) -> Result<DecodedImage> {
    if std::fs::metadata(path).map_err(FastPadError::Io)?.len() > MAX_SVG_BYTES {
        return Err(FastPadError::Invariant("SVG file is larger than 8 MB"));
    }
    let bytes = std::fs::read(path).map_err(FastPadError::Io)?;
    let tree = parse(&String::from_utf8_lossy(&bytes))?;
    let (natural_width, natural_height) = natural_size(&tree);
    if u64::from(natural_width) * u64::from(natural_height) > MAX_IMAGE_PIXELS {
        return Err(FastPadError::Invariant(
            "image is empty or larger than 64 megapixels",
        ));
    }
    let (width, height) = if max_width > 0 && natural_width > max_width {
        let scaled = (u64::from(natural_height) * u64::from(max_width)) / u64::from(natural_width);
        (max_width, scaled.max(1) as u32)
    } else {
        (natural_width, natural_height)
    };
    let pixels = rasterize(&tree, (width, height))?;
    Ok(DecodedImage {
        width,
        height,
        natural_width,
        natural_height,
        pixels,
    })
}

/// Parses SVG `source`. Runs on a worker thread: the first SVG with text loads the installed
/// fonts' tables, which takes a moment.
pub(crate) fn parse(source: &str) -> Result<usvg::Tree> {
    let mut options = usvg::Options::default();
    // Embedded `data:` images only; a file or URL reference draws nothing.
    options.image_href_resolver.resolve_string = Box::new(|_, _| None);
    if has_text(source) {
        options.fontdb = system_fonts();
    }
    usvg::Tree::from_str(source, &options).map_err(|_| FastPadError::Invariant("SVG is malformed"))
}

/// The tree's size in whole pixels, at least 1×1: `width`/`height`, completed from the `viewBox`
/// aspect ratio; otherwise the `viewBox` size; otherwise the extent of what it draws.
pub(crate) fn natural_size(tree: &usvg::Tree) -> (u32, u32) {
    let size = tree.size();
    (
        (size.width().ceil() as u32).max(1),
        (size.height().ceil() as u32).max(1),
    )
}

/// Rasterizes `tree` to `width`×`height` premultiplied BGRA pixels, scaling its natural size to
/// fit exactly.
pub(crate) fn rasterize(tree: &usvg::Tree, (width, height): (u32, u32)) -> Result<Vec<u8>> {
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height)
        .ok_or(FastPadError::Invariant("SVG raster size is empty"))?;
    let size = tree.size();
    let transform = resvg::tiny_skia::Transform::from_scale(
        width as f32 / size.width(),
        height as f32 / size.height(),
    );
    resvg::render(tree, transform, &mut pixmap.as_mut());
    // tiny-skia writes premultiplied RGBA.
    let mut pixels = pixmap.take();
    for pixel in pixels.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    Ok(pixels)
}

/// Whether `source` may contain a `<text>` element (or `<svg:text>`), so parsing needs fonts.
fn has_text(source: &str) -> bool {
    source.contains("<text") || source.contains(":text")
}

/// The installed fonts, scanned once per process and shared by every later SVG with text.
fn system_fonts() -> Arc<fontdb::Database> {
    static FONTS: OnceLock<Arc<fontdb::Database>> = OnceLock::new();
    Arc::clone(FONTS.get_or_init(|| {
        let mut fonts = fontdb::Database::new();
        fonts.load_system_fonts();
        Arc::new(fonts)
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preview::images::decode_image;
    use std::path::PathBuf;

    fn icon() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("assets")
            .join("fastpad-icon.svg")
    }

    fn scratch(name: &str, contents: &[u8]) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("fastpad-svg-{name}-{}.svg", std::process::id()));
        std::fs::write(&path, contents).unwrap();
        path
    }

    fn pixel(image: &DecodedImage, x: u32, y: u32) -> [u8; 4] {
        let offset = ((y * image.width + x) * 4) as usize;
        image.pixels[offset..offset + 4].try_into().unwrap()
    }

    /// BGRA within a small tolerance for antialiasing.
    fn near(actual: [u8; 4], expected: [u8; 4]) -> bool {
        actual
            .iter()
            .zip(expected)
            .all(|(actual, expected)| actual.abs_diff(expected) <= 8)
    }

    const COVER_ORANGE: [u8; 4] = [0x20, 0xB0, 0xFF, 0xFF];

    /// The spec §7.3 spike: Direct2D must rasterize SVG into a WIC bitmap off the UI thread.
    #[test]
    fn the_fastpad_icon_rasterizes_on_a_worker_thread() {
        let image = std::thread::spawn(|| decode_image(&icon(), 96))
            .join()
            .unwrap()
            .unwrap();
        assert_eq!((image.natural_width, image.natural_height), (244, 256));
        assert_eq!((image.width, image.height), (96, 100));
        // The cover (#FFB020) at (219, 198) of 244 px wide, below and right of the bolt.
        let cover = pixel(&image, 86, 78);
        assert!(near(cover, COVER_ORANGE), "{cover:?}");
        // Above the binder rings, left of the cover, the icon is transparent.
        assert_eq!(pixel(&image, 2, 2), [0, 0, 0, 0]);
    }

    #[test]
    fn use_elements_with_a_plain_href_render() {
        let image = decode_image(&icon(), 0).unwrap();
        // Only a binder ring drawn through `<use href="#rings">` covers (20, 128), left of the cover.
        let ring = pixel(&image, 20, 128);
        assert!(near(ring, COVER_ORANGE), "{ring:?}");
        // The notch around the ring stays transparent.
        assert_eq!(pixel(&image, 71, 128), [0, 0, 0, 0]);
    }

    #[test]
    fn malformed_huge_and_oversized_svgs_fail() {
        let malformed = scratch("malformed", b"<svg width='10' height='10'><path d=");
        assert!(decode_image(&malformed, 0).is_err());
        let huge = scratch(
            "huge",
            br#"<svg xmlns="http://www.w3.org/2000/svg" width="100000" height="100000"></svg>"#,
        );
        assert!(decode_image(&huge, 0).is_err());
        let mut padded = b"<svg xmlns='http://www.w3.org/2000/svg' width='1' height='1'>".to_vec();
        padded.resize(MAX_SVG_BYTES as usize + 1, b' ');
        let oversized = scratch("oversized", &padded);
        assert!(decode_image(&oversized, 0).is_err());
        for path in [malformed, huge, oversized] {
            let _ = std::fs::remove_file(path);
        }
    }

    #[test]
    fn natural_size_follows_dimensions_then_view_box_then_the_content() {
        let size = |root: &str| {
            natural_size(
                &parse(&format!(
                    r#"<svg xmlns="http://www.w3.org/2000/svg" {root}><rect x="5" y="5" width="20" height="10"/></svg>"#
                ))
                .unwrap(),
            )
        };
        assert_eq!(size(r#"width="64" height="32px""#), (64, 32));
        assert_eq!(size(r#"viewBox="0 0 256 128""#), (256, 128));
        assert_eq!(size(r#"width="100%" viewBox="0,0,10,20""#), (10, 20));
        assert_eq!(size(r#"width="50" viewBox="0 0 10 20""#), (50, 100));
        // With neither, the image fits what it draws.
        assert_eq!(size(""), (25, 15));
    }

    /// A 40×40 SVG with `body`, rasterized; its pixel at (10, 10) as BGRA.
    fn center_of(body: &str) -> [u8; 4] {
        let source = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="40" height="40">{body}</svg>"#
        );
        let pixels = rasterize(&parse(&source).unwrap(), (40, 40)).unwrap();
        let offset = (10 * 40 + 10) * 4;
        pixels[offset..offset + 4].try_into().unwrap()
    }

    const RED: [u8; 4] = [0, 0, 0xFF, 0xFF];

    /// Break caught: Direct2D drew these blank or black, so exported diagrams lost their labels,
    /// arrowheads, and colors.
    #[test]
    fn exporter_features_render() {
        let cases = [
            (
                "style sheet class",
                r#"<style>.a{fill:#f00}</style><rect class="a" width="40" height="40"/>"#,
            ),
            (
                "rgba()",
                r#"<rect width="40" height="40" fill="rgba(255,0,0,1)"/>"#,
            ),
            (
                "hsl()",
                r#"<rect width="40" height="40" fill="hsl(0,100%,50%)"/>"#,
            ),
            (
                "#rrggbbaa",
                r##"<rect width="40" height="40" fill="#ff0000ff"/>"##,
            ),
            (
                "pattern",
                r##"<pattern id="p" width="40" height="40" patternUnits="userSpaceOnUse"><rect width="40" height="40" fill="red"/></pattern><rect width="40" height="40" fill="url(#p)"/>"##,
            ),
            (
                "inside <a>",
                r#"<a href="https://example.com"><rect width="40" height="40" fill="red"/></a>"#,
            ),
            (
                "<switch>",
                r#"<switch><rect width="40" height="40" fill="red"/></switch>"#,
            ),
            (
                "<symbol> through <use>",
                r##"<symbol id="s"><rect width="40" height="40" fill="red"/></symbol><use href="#s"/>"##,
            ),
            (
                "marker",
                r##"<marker id="m" markerWidth="40" markerHeight="40" markerUnits="userSpaceOnUse" overflow="visible"><rect width="40" height="40" fill="red"/></marker><path d="M0 0 H1" marker-start="url(#m)"/>"##,
            ),
            (
                "data: image",
                r#"<image width="40" height="40" href="data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='4' height='4'%3E%3Crect width='4' height='4' fill='red'/%3E%3C/svg%3E"/>"#,
            ),
        ];
        for (name, body) in cases {
            let pixel = center_of(body);
            assert!(near(pixel, RED), "{name}: {pixel:?}");
        }
    }

    #[test]
    fn text_renders_in_an_installed_font() {
        let ink = |body: &str| {
            let source = format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="80" height="40">{body}</svg>"#
            );
            rasterize(&parse(&source).unwrap(), (80, 40))
                .unwrap()
                .as_chunks::<4>()
                .0
                .iter()
                .filter(|pixel| pixel[3] > 0)
                .count()
        };
        assert!(ink(r#"<text x="4" y="30" font-family="Arial" font-size="28">Hi</text>"#) > 50);
        // A `<switch>` falls back past an HTML `<foreignObject>` to its `<text>`, as draw.io writes.
        assert!(
            ink(
                r#"<switch><foreignObject requiredFeatures="http://www.w3.org/TR/SVG11/feature#Extensibility" width="80" height="40"/><text x="4" y="30" font-size="28">Hi</text></switch>"#
            ) > 50
        );
    }

    #[test]
    fn a_referenced_file_is_not_loaded() {
        let mut red = resvg::tiny_skia::Pixmap::new(4, 4).unwrap();
        red.fill(resvg::tiny_skia::Color::from_rgba8(255, 0, 0, 255));
        let image =
            std::env::temp_dir().join(format!("fastpad-svg-red-{}.png", std::process::id()));
        std::fs::write(&image, red.encode_png().unwrap()).unwrap();
        let href = image.display().to_string();
        let embedded = format!(
            "data:image/png;base64,{}",
            base64_of(&std::fs::read(&image).unwrap())
        );
        // The same PNG renders when embedded, so only the file reference is refused.
        assert!(near(
            center_of(&format!(
                r#"<image width="40" height="40" href="{embedded}"/>"#
            )),
            RED
        ));
        assert_eq!(
            center_of(&format!(r#"<image width="40" height="40" href="{href}"/>"#))[3],
            0
        );
        let _ = std::fs::remove_file(image);
    }

    fn base64_of(bytes: &[u8]) -> String {
        const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in bytes.chunks(3) {
            let n = chunk
                .iter()
                .enumerate()
                .fold(0_u32, |n, (i, b)| n | u32::from(*b) << (16 - 8 * i));
            for i in 0..4 {
                out.push(if i <= chunk.len() {
                    ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char
                } else {
                    '='
                });
            }
        }
        out
    }

    #[test]
    #[ignore = "performance measurement: cargo test --release --lib preview::svg -- --ignored --nocapture"]
    fn decoding_the_icon_at_256_px_is_recorded() {
        let mut samples = (0..21)
            .map(|_| {
                let started = std::time::Instant::now();
                decode_image(&icon(), 256).unwrap();
                started.elapsed().as_micros() as u64
            })
            .collect::<Vec<_>>();
        samples.sort_unstable();
        println!(
            "svg decode at 256 px: median {} us, max {} us",
            samples[10], samples[20]
        );
    }
}
