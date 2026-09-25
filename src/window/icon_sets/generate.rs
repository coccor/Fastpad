//! Regenerates every set's `icons.bin` and `icons.source-hash` under `assets/icons/` from its SVGs
//! (icon sets spec §5.1), with the Markdown preview's Direct2D SVG renderer: Material's as
//! premultiplied BGRA, Minimal's and Solid's as coverage masks. Ignored in the suite; run by
//! `tools/generate-file-icons.ps1`.

use super::masks::{MaskIcon, MaskSet, set_dir};
use super::material::{MaterialIcon, SIZES, source_hash, svg_dir};

/// `source` with `width` and `height` of `size` on its root, so `decode_svg` renders the viewBox
/// into a `size`-px square.
fn sized_svg(source: &str, size: u32) -> String {
    let at = source.find("<svg").expect("an <svg> root") + "<svg".len();
    let root_end = at + source[at..].find('>').expect("a closed <svg> tag");
    let root = &source[at..root_end];
    assert!(
        !root.contains(" width=") && !root.contains(" height="),
        "the root already has a size"
    );
    format!(
        "{} width=\"{size}\" height=\"{size}\"{}",
        &source[..at],
        &source[at..]
    )
}

#[test]
#[ignore = "regenerates the committed icon data; run tools/generate-file-icons.ps1"]
fn generate_file_icons() {
    use windows::Win32::Graphics::Imaging::{CLSID_WICImagingFactory, IWICImagingFactory};
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
    };
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    let wic: IWICImagingFactory =
        unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER) }.unwrap();
    let scratch = std::env::temp_dir().join(format!("fastpad-icongen-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).unwrap();
    let path = scratch.join("icon.svg");
    // `source` at `size` px: premultiplied BGRA, `size * size * 4` bytes.
    let render = |source: &str, size: u32, name: &str| {
        std::fs::write(&path, sized_svg(source, size)).unwrap();
        let image = crate::preview::svg::decode_svg(&wic, &path, 0).unwrap();
        assert_eq!((image.width, image.height), (size, size), "{name}");
        image.pixels
    };

    let mut blob = Vec::new();
    for icon in MaterialIcon::ALL {
        let source = std::fs::read_to_string(svg_dir().join(icon.file_name())).unwrap();
        for size in SIZES {
            blob.extend_from_slice(&render(&source, size, icon.file_name()));
        }
    }
    let material = svg_dir().parent().unwrap().to_path_buf();
    std::fs::write(material.join("icons.bin"), &blob).unwrap();
    std::fs::write(
        material.join("icons.source-hash"),
        format!(
            "{:016x}
",
            source_hash()
        ),
    )
    .unwrap();

    for set in MaskSet::ALL {
        let mut blob = Vec::new();
        for icon in MaskIcon::ALL {
            let file = set_dir(set).join("svg").join(icon.file_name());
            let source = std::fs::read_to_string(file).unwrap();
            for size in SIZES {
                let pixels = render(&source, size, icon.file_name());
                let (chunks, _) = pixels.as_chunks::<4>();
                blob.extend(chunks.iter().map(|pixel| pixel[3]));
            }
        }
        std::fs::write(set_dir(set).join("icons.bin"), &blob).unwrap();
        std::fs::write(
            set_dir(set).join("icons.source-hash"),
            format!(
                "{:016x}
",
                super::masks::source_hash(set)
            ),
        )
        .unwrap();
    }
    let _ = std::fs::remove_dir_all(&scratch);
}

#[test]
fn a_sized_svg_gets_its_size_on_the_root_only() {
    // Break caught: the size added to a child element, or a root left unsized (a 300×150 render).
    assert_eq!(
        sized_svg(r#"<svg viewBox="0 0 16 16"><path d="M0 0"/></svg>"#, 24),
        r#"<svg width="24" height="24" viewBox="0 0 16 16"><path d="M0 0"/></svg>"#
    );
}
