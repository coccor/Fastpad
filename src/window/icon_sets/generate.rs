//! Regenerates `assets/icons/material/icons.bin` and `icons.source-hash` from the SVGs (icon sets
//! spec §5.1), with the Markdown preview's Direct2D SVG renderer. Ignored in the suite; run by
//! `tools/generate-file-icons.ps1`.

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
fn generate_material_icons() {
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
    let mut blob = Vec::new();
    for icon in MaterialIcon::ALL {
        let source = std::fs::read_to_string(svg_dir().join(icon.file_name())).unwrap();
        for size in SIZES {
            std::fs::write(&path, sized_svg(&source, size)).unwrap();
            let image = crate::preview::svg::decode_svg(&wic, &path, 0).unwrap();
            assert_eq!((image.width, image.height), (size, size), "{icon:?}");
            blob.extend_from_slice(&image.pixels);
        }
    }
    let _ = std::fs::remove_dir_all(&scratch);
    let material = svg_dir().parent().unwrap().to_path_buf();
    std::fs::write(material.join("icons.bin"), &blob).unwrap();
    std::fs::write(
        material.join("icons.source-hash"),
        format!("{:016x}\n", source_hash()),
    )
    .unwrap();
}

#[test]
fn a_sized_svg_gets_its_size_on_the_root_only() {
    // Break caught: the size added to a child element, or a root left unsized (a 300×150 render).
    assert_eq!(
        sized_svg(r#"<svg viewBox="0 0 16 16"><path d="M0 0"/></svg>"#, 24),
        r#"<svg width="24" height="24" viewBox="0 0 16 16"><path d="M0 0"/></svg>"#
    );
}
