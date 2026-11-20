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

/// Whether FastPad refuses to read a look file named `look`: an absolute path, a network path,
/// a drive prefix (`C:x.look` included), or any `..` segment that could leave the diagram folder.
pub fn refused(look: &str) -> bool {
    Path::new(look).is_absolute()
        || look.starts_with(['/', '\u{5c}'])
        || look.get(1..2) == Some(":")
        || look
            .split(['/', '\\'])
            .any(|segment| segment == "..")
}

fn refusal(look: &str) -> String {
    format!("FastPad reads look files only by a relative path; \"{look}\" was not read")
}

/// Look files over this size are not read (the renderer's own limit).
const MAX_LOOK_BYTES: u64 = 1_000_000;

/// FastDraw's whitespace (JavaScript's), which separates words and marks continuation lines.
fn is_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{B}' | '\u{C}' | '\r' | ' ' | '\u{A0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200A}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202F}'
                | '\u{205F}'
                | '\u{3000}'
                | '\u{FEFF}'
    )
}

/// `line` without its `//` comment (one outside quotes).
fn strip_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut quoted = false;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' if quoted => i += 1,
            b'"' => quoted = !quoted,
            b'/' if !quoted && bytes.get(i + 1) == Some(&b'/') => return &line[..i],
            _ => {}
        }
        i += 1;
    }
    line
}

/// One word of a statement: its value, and whether it is a plain word (not quoted, not `key=value`).
struct Word {
    value: String,
    plain: bool,
}

/// The words of `line` as FastDraw splits them: quotes group and are dropped, a backslash in
/// quotes escapes, and `key=value` keeps the value.
fn words(line: &str) -> Vec<Word> {
    let mut out = Vec::new();
    let mut chars = line.chars().peekable();
    loop {
        while chars.next_if(|&c| is_space(c)).is_some() {}
        let Some(&first) = chars.peek() else {
            return out;
        };
        let mut value = String::new();
        let mut quoted = false;
        while let Some(c) = chars.next_if(|&c| quoted || !is_space(c)) {
            if c == '"' {
                quoted = !quoted;
            } else if c == '\\' && quoted && chars.peek().is_some() {
                let escaped = chars.next().unwrap_or_default();
                value.push(if escaped == 'n' { '\n' } else { escaped });
            } else {
                value.push(c);
            }
        }
        if first == '"' {
            out.push(Word {
                value,
                plain: false,
            });
        } else if let Some(key) = property_key_len(&value) {
            out.push(Word {
                value: value[key + 1..].to_owned(),
                plain: false,
            });
        } else {
            out.push(Word { value, plain: true });
        }
    }
}

/// The key's length when `word` is `key=value` (a key is a letter, then letters, digits, `_./#-`).
fn property_key_len(word: &str) -> Option<usize> {
    let bytes = word.as_bytes();
    if !bytes.first()?.is_ascii_alphabetic() {
        return None;
    }
    let end = bytes.iter().position(|&c| {
        !(c.is_ascii_alphanumeric() || matches!(c, b'_' | b'.' | b'/' | b'#' | b'-'))
    })?;
    (bytes[end] == b'=').then_some(end)
}

/// FastDraw's statements: each line with its 1-based number, plus the indented lines after it.
fn statements(text: &str) -> Vec<(u32, Vec<Word>)> {
    let mut out: Vec<(u32, Vec<Word>)> = Vec::new();
    for (index, raw) in text.split('\n').enumerate() {
        let raw = raw.strip_suffix('\r').unwrap_or(raw);
        let trimmed = raw.trim_matches(is_space);
        if trimmed.is_empty() || trimmed == "#" || trimmed.starts_with("# ") {
            continue;
        }
        let code = strip_comment(raw);
        if code.trim_matches(is_space).is_empty() {
            continue;
        }
        let line_words = words(code);
        match out.last_mut() {
            Some((_, last)) if raw.starts_with(is_space) => last.extend(line_words),
            _ => out.push((index as u32 + 1, line_words)),
        }
    }
    out
}

/// The statements of `text` that start with the plain word `head`.
fn statements_named(text: &str, head: &str) -> impl Iterator<Item = (u32, Vec<Word>)> {
    statements(text)
        .into_iter()
        .filter(move |(_, words)| words.first().is_some_and(|w| w.plain && w.value == head))
}

/// What each `dark` statement of a look file names.
fn dark_values(look: &str) -> Vec<String> {
    statements_named(look, "dark")
        .filter_map(|(_, words)| words.into_iter().nth(1).map(|w| w.value))
        .collect()
}

/// The line of the diagram's `look` statement.
fn look_line(text: &str) -> Option<u32> {
    statements_named(text, "look").next().map(|(line, _)| line)
}

/// The diagram's own look file as `render` reads it: named by a relative path ending in `.look`,
/// relative to `file`, at most 1 MB. `None` for a bundled look, a refused path or a file that
/// cannot be read.
pub fn read_look(text: &str, file: &Path) -> Option<fastdraw_render::LookFile> {
    let path = fastdraw_render::look_path(text)?;
    if refused(&path) || !path.to_lowercase().ends_with(".look") {
        return None;
    }
    let full = file.parent().unwrap_or(Path::new("")).join(&path);
    let mut bytes = Vec::new();
    std::fs::File::open(full)
        .ok()?
        .take(MAX_LOOK_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_LOOK_BYTES {
        return None;
    }
    Some(fastdraw_render::LookFile {
        path,
        text: String::from_utf8_lossy(&bytes).into_owned(),
    })
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
    // The renderer reads the dark look a look file names by any path, so a refused one keeps the
    // diagram light: nothing beyond its own look file is read.
    let refused_dark = file
        .filter(|_| dark)
        .and_then(|file| read_look(text, file))
        .and_then(|look| dark_values(&look.text).into_iter().find(|v| refused(v)));
    let dark = dark && refused_dark.is_none();
    let drawing = fastdraw_render::render(&Source { text, file, dark });
    crate::preview::svg::add_fastdraw_fonts();
    crate::preview::svg::add_font_files(&drawing.font_files);

    let mut show = true;
    let mut errors = Vec::new();
    if let Some(value) = &refused_dark {
        let line = look_line(text).unwrap_or(1);
        errors.push(format!("Line {line}: {}", refusal(value)));
    }
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
/// the stamp it was made from; the stamp comes also when the drawing failed, so a fix to the file
/// is noticed.
pub fn decode_file(
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
        for p in ["../x.look", "looks/../x.look", r"..\x.look"] {
            assert!(refused(p), "{p}");
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
        let night = draw("look night\na: rect \"A\" at 0,0\n", None, false).paper;
        assert_eq!(draw("a: rect \"A\" at 0,0\n", None, true).paper, night);
        assert_ne!(draw("a: rect \"A\" at 0,0\n", None, false).paper, night);
    }

    /// A fresh folder in the temp directory, removed when dropped.
    struct Folder(PathBuf);

    impl Folder {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("fastpad-fastdraw-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn write(&self, name: &str, text: &str) -> PathBuf {
            let path = self.0.join(name);
            std::fs::write(&path, text).unwrap();
            path
        }
    }

    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_look_files_absolute_dark_line_is_not_read() {
        // Break caught: a relative look file naming a network or absolute dark look, which the
        // renderer would read (an SMB connection for `\\server\x.look`).
        let elsewhere = Folder::new("elsewhere");
        let evil = elsewhere.write("evil.look", "color paper #123456\n");
        let evil = evil.to_string_lossy().replace('\\', "/");
        let folder = Folder::new("dark-refused");
        folder.write(
            "x.look",
            &format!("color paper #0e2440 // light\ndark \"{evil}\"\n"),
        );
        let diagram = folder.write("d.fdraw", "");
        let d = draw(
            "a: rect \"A\" at 0,0\nlook \"x.look\"\n",
            Some(&diagram),
            true,
        );
        assert_eq!(d.paper, 0x0e_2440);
        assert!(d.show && !d.svg.is_empty());
        assert_eq!(
            d.errors,
            [format!(
                "Line 2: FastPad reads look files only by a relative path; \"{evil}\" was not read"
            )]
        );
    }

    #[test]
    fn a_look_files_bundled_or_relative_dark_line_still_draws_dark() {
        let night = draw("look night\na: rect \"A\" at 0,0\n", None, false).paper;
        let folder = Folder::new("dark-allowed");
        folder.write("x.look", "color paper #0e2440\ndark night\n");
        folder.write("y.look", "color paper #0e2440\ndark \"./y-night.look\"\n");
        folder.write("y-night.look", "color paper #345678\n");
        let diagram = folder.write("d.fdraw", "");
        let named = draw(
            "look \"x.look\"\na: rect \"A\" at 0,0\n",
            Some(&diagram),
            true,
        );
        assert_eq!((named.paper, named.errors.len()), (night, 0));
        let relative = draw(
            "look \"y.look\"\na: rect \"A\" at 0,0\n",
            Some(&diagram),
            true,
        );
        assert_eq!((relative.paper, relative.errors.len()), (0x34_5678, 0));
    }

    #[test]
    fn dark_values_are_found_as_the_renderer_parses_them() {
        let text = "# a note\n\
            dark \"C:/a.look\" // dark c:/no.look\n\
            canvas\n  dark \"//inside/canvas.look\"\n\
            dark\n  \"\\\\\\\\server\\\\b.look\"\n\
            dark key=D:/c.look\n\
            dark night\n";
        assert_eq!(
            dark_values(text),
            ["C:/a.look", r"\\server\b.look", "D:/c.look", "night"]
        );
    }

    #[test]
    fn text_rasterizes_with_ink_after_fonts_are_added() {
        let d = draw("a: rect \"Hello\" at 0,0\n", None, false);
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
