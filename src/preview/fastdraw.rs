//! FastDraw (`.fdraw`) diagrams, drawn by `fastdraw-render` under FastPad's rules: look files are
//! read only by a relative path, a renderer panic becomes an error instead of taking FastPad down,
//! and the fonts a diagram names are registered with the SVG rasterizer.

use crate::preview::images::DecodedImage;
use fastdraw_render::{Level, Source};
use std::io::Read;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// What FastPad shows when the renderer panics.
const PANIC_TEXT: &str = "FastDraw could not draw this diagram";

/// A drawn diagram.
#[derive(Debug, Clone)]
pub struct Drawn {
    pub svg: String,
    /// The look's paper colour, `0x00RRGGBB`.
    pub paper: u32,
    /// `Line n: message`, for each error in order.
    pub errors: Vec<String>,
    /// False when an error other than the look-path refusal exists: the drawing is not shown.
    pub show: bool,
    /// The diagram's own look file as read, for the share link.
    pub look: Option<fastdraw_render::LookFile>,
}

/// Whether FastPad refuses to read a look file named `look`: an absolute path, a network path, or
/// one with a drive prefix (`C:x.look` included).
pub fn refused(look: &str) -> bool {
    Path::new(look).is_absolute()
        || look.starts_with(['/', '\u{5c}'])
        || look.get(1..2) == Some(":")
}

fn refusal(look: &str) -> String {
    format!("FastPad reads look files only by a relative path; \"{look}\" was not read")
}

/// Draws `text`, which lives in `file` (the base of a relative look path), in the light or dark
/// look. Never panics.
pub fn draw(text: &str, file: Option<&Path>, dark: bool) -> Drawn {
    guarded(|| draw_unguarded(text, file, dark))
}

/// Runs `draw`, turning a panic into a hidden drawing with one error.
fn guarded(draw: impl FnOnce() -> Drawn) -> Drawn {
    catch_unwind(AssertUnwindSafe(draw)).unwrap_or_else(|_| Drawn {
        svg: String::new(),
        paper: 0x00FF_FFFF,
        errors: vec![PANIC_TEXT.to_owned()],
        show: false,
        look: None,
    })
}

fn draw_unguarded(text: &str, file: Option<&Path>, dark: bool) -> Drawn {
    let refused_look = fastdraw_render::look_path(text).filter(|look| refused(look));
    // A refused look is never handed to the renderer's file reading.
    let file = if refused_look.is_some() { None } else { file };
    let drawing = fastdraw_render::render(&Source { text, file, dark });
    crate::preview::svg::add_fastdraw_fonts();
    crate::preview::svg::add_font_files(&drawing.font_files);

    let mut show = true;
    let mut errors = Vec::new();
    for issue in drawing.issues.iter().filter(|i| i.level == Level::Error) {
        let message = match &refused_look {
            Some(look) if issue.message.starts_with("look file not found") => refusal(look),
            _ => {
                show = false;
                issue.message.clone()
            }
        };
        errors.push(format!("Line {}: {message}", issue.line));
    }
    let [r, g, b] = drawing.paper;
    Drawn {
        svg: drawing.svg,
        paper: u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b),
        errors,
        show,
        look: drawing.look,
    }
}

/// The first error, plus ` (and k more)` when k more follow; `None` when there are none.
pub fn band_text(errors: &[String]) -> Option<String> {
    let (first, rest) = errors.split_first()?;
    Some(if rest.is_empty() {
        first.clone()
    } else {
        format!("{first} (and {} more)", rest.len())
    })
}

/// Rasterizes the drawing no wider than `max_width` (0 = no limit) at `scale`.
pub fn rasterize(drawn: &Drawn, max_width: u32, scale: f32) -> Result<DecodedImage, String> {
    let tree = crate::preview::svg::parse(&drawn.svg).map_err(|error| error.to_string())?;
    let (natural_width, natural_height) = crate::preview::svg::natural_size(&tree);
    let (width, height) =
        crate::preview::mermaid::pixel_size((natural_width, natural_height), max_width, scale)
            .ok_or_else(|| "the diagram is larger than 64 megapixels".to_owned())?;
    let pixels =
        crate::preview::svg::rasterize(&tree, (width, height)).map_err(|e| e.to_string())?;
    Ok(DecodedImage {
        width,
        height,
        natural_width,
        natural_height,
        pixels,
    })
}

/// `.fdraw` files over this size are not read.
pub const MAX_FDRAW_BYTES: u64 = 1 << 20;

/// What a drawing was made from, to tell when the files have changed.
#[derive(Debug, Clone, PartialEq)]
pub struct Stamp {
    diagram: Option<SystemTime>,
    look: Option<(PathBuf, Option<SystemTime>)>,
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

impl Stamp {
    /// True when the diagram at `path` and its look file are as they were (both are stat-ed again).
    pub fn current(&self, path: &Path) -> bool {
        self.diagram == modified(path)
            && self
                .look
                .as_ref()
                .is_none_or(|(look, time)| *time == modified(look))
    }
}

/// Draws the `.fdraw` file at `path` as a bitmap no wider than `max_width` (0 = no limit), with
/// the stamp it was made from.
pub fn decode_file(
    path: &Path,
    dark: bool,
    max_width: u32,
) -> Result<(DecodedImage, Stamp), String> {
    let (result, stamp) = decode_stamped(path, dark, max_width);
    result.map(|image| (image, stamp))
}

/// `decode_file`, with the stamp also when the drawing failed, so a fix to the file is noticed.
pub fn decode_stamped(
    path: &Path,
    dark: bool,
    max_width: u32,
) -> (Result<DecodedImage, String>, Stamp) {
    let mut stamp = Stamp {
        diagram: modified(path),
        look: None,
    };
    let result = read_text(path).and_then(|text| {
        let drawn = draw(&text, Some(path), dark);
        if let Some(look) = &drawn.look {
            let file = path.parent().unwrap_or(Path::new("")).join(&look.path);
            let time = modified(&file);
            stamp.look = Some((file, time));
        }
        if !drawn.show {
            return Err(band_text(&drawn.errors).unwrap_or_else(|| PANIC_TEXT.to_owned()));
        }
        rasterize(&drawn, max_width, 1.0)
    });
    (result, stamp)
}

fn read_text(path: &Path) -> Result<String, String> {
    let file = std::fs::File::open(path).map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    file.take(MAX_FDRAW_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_FDRAW_BYTES {
        return Err("this diagram is larger than 1 MB".to_owned());
    }
    String::from_utf8(bytes).map_err(|_| "this diagram is not valid UTF-8 text".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn network_absolute_and_drive_look_paths_are_refused() {
        for p in [
            "//server/x.look",
            r"\\server\x.look",
            "C:/x.look",
            "C:x.look",
            "/x.look",
            r"\\?\C:\x.look",
        ] {
            assert!(refused(p), "{p}");
        }
        for p in ["./x.look", "x.look", "looks/x.look", "night"] {
            assert!(!refused(p), "{p}");
        }
    }

    #[test]
    fn a_refused_look_draws_in_the_built_in_look_with_its_own_error() {
        let d = draw(
            "look \"//server/x.look\"\na: rect \"A\" at 0,0\n",
            Some(Path::new(r"C:\t\d.fdraw")),
            false,
        );
        assert!(d.show && !d.svg.is_empty() && d.look.is_none());
        assert_eq!(
            d.errors,
            [
                r#"Line 1: FastPad reads look files only by a relative path; "//server/x.look" was not read"#
            ]
        );
    }

    #[test]
    fn band_text_counts_the_rest() {
        assert_eq!(band_text(&[]), None);
        assert_eq!(band_text(&["Line 4: x".into()]).unwrap(), "Line 4: x");
        assert_eq!(
            band_text(&["Line 4: x".into(), "Line 5: y".into(), "Line 9: z".into()]).unwrap(),
            "Line 4: x (and 2 more)"
        );
    }

    #[test]
    fn a_real_error_hides_the_drawing() {
        let d = draw("a: nosuchkind \"A\" at 0,0\n", None, false);
        assert!(!d.show);
        assert!(d.errors[0].starts_with("Line 1: "));
    }

    #[test]
    fn a_dark_diagram_uses_night_paper() {
        assert_ne!(
            draw("a: node \"A\" at 0,0\n", None, true).paper,
            draw("a: node \"A\" at 0,0\n", None, false).paper
        );
    }

    #[test]
    fn text_rasterizes_with_ink_after_fonts_are_added() {
        let d = draw("a: node \"Hello\" at 0,0\n", None, false);
        let image = rasterize(&d, 0, 1.0).unwrap();
        let paper = [
            (d.paper & 0xFF) as u8,
            (d.paper >> 8 & 0xFF) as u8,
            (d.paper >> 16 & 0xFF) as u8,
        ];
        // Some pixel in the middle of the drawing differs from the paper (BGRA).
        let (w, h) = (image.width as usize, image.height as usize);
        let inked = (h / 4..h * 3 / 4).any(|y| {
            (w / 4..w * 3 / 4).any(|x| {
                let p = &image.pixels[(y * w + x) * 4..][..3];
                p != paper
            })
        });
        assert!(inked, "{}x{}", image.width, image.height);
    }

    #[test]
    fn a_panicking_render_becomes_an_error() {
        let d = guarded(|| panic!("boom"));
        assert_eq!(d.errors, ["FastDraw could not draw this diagram"]);
        assert!(!d.show);
    }
}
