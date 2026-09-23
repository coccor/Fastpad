//! Untitled tab labels, turning a label or typed name into a safe filename, and the list of file
//! extensions that count as notes.

use crate::document::Language;
use std::path::Path;

pub const LABEL_LIMIT: usize = 40;
pub const LABEL_SCAN_LINES: usize = 16;

pub const NOTE_EXTENSIONS: [&str; 14] = [
    "md", "markdown", "txt", "text", "json", "log", "ini", "cfg", "conf", "yaml", "yml", "toml",
    "csv", "xml",
];

pub fn is_note_extension(extension: &str) -> bool {
    NOTE_EXTENSIONS
        .iter()
        .any(|known| known.eq_ignore_ascii_case(extension))
}

/// An untitled tab's label, and the last line whose edits can change it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Label {
    pub text: Option<String>,
    pub watch_through: usize,
}

pub fn untitled_label<'a>(lines: impl IntoIterator<Item = &'a str>) -> Label {
    for (index, line) in lines.into_iter().take(LABEL_SCAN_LINES).enumerate() {
        let text = line.trim().trim_start_matches('#').trim();
        if !text.is_empty() {
            return Label {
                text: Some(truncate(text)),
                watch_through: index,
            };
        }
    }
    Label {
        text: None,
        watch_through: LABEL_SCAN_LINES - 1,
    }
}

fn truncate(text: &str) -> String {
    if text.chars().count() <= LABEL_LIMIT {
        return text.to_owned();
    }
    let mut cut: String = text.chars().take(LABEL_LIMIT - 1).collect();
    cut.truncate(cut.trim_end().len());
    cut.push('…');
    cut
}

const RESERVED: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// A filename stem Windows accepts: no `<>:"/\|?*` or control characters, no trailing dots,
/// spaces or label ellipsis, not a reserved device name, never empty.
pub fn sanitize_stem(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .filter(|c| !matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'))
        .filter(|c| !c.is_control())
        .collect();
    let trimmed = cleaned
        .trim()
        .trim_end_matches(['.', ' ', '…'])
        .trim()
        .to_owned();
    if trimmed.is_empty() {
        return "Untitled".to_owned();
    }
    // Windows reads "con.txt" and "con .txt" as the device too: neutralise the part before the
    // first dot.
    let device = trimmed.split('.').next().unwrap_or("").trim_end();
    if RESERVED
        .iter()
        .any(|reserved| reserved.eq_ignore_ascii_case(device))
    {
        let (name, rest) = trimmed.split_at(device.len());
        return format!("{name}_{rest}");
    }
    trimmed
}

pub fn default_extension(language: Language) -> &'static str {
    match language {
        Language::Json => "json",
        Language::Markdown | Language::PlainText => "md",
    }
}

/// Splits what the user typed into a sanitized stem and an extension. A typed extension is kept
/// only when it is a note extension, so "v1.2 plan" stays one stem.
pub fn split_typed_name(input: &str, default_extension: &str) -> (String, String) {
    let input = input.trim();
    if let Some((stem, extension)) = input.rsplit_once('.')
        && is_note_extension(extension)
    {
        return (sanitize_stem(stem), extension.to_owned());
    }
    (sanitize_stem(input), default_extension.to_owned())
}

/// `stem.extension`, or `stem N.extension` with the first free N from 2.
pub fn free_name(stem: &str, extension: &str, exists: impl Fn(&str) -> bool) -> String {
    let first = format!("{stem}.{extension}");
    if !exists(&first) {
        return first;
    }
    (2..10_000)
        .map(|number| format!("{stem} {number}.{extension}"))
        .find(|candidate| !exists(candidate))
        .unwrap_or(first)
}

pub fn note_title(path: &Path) -> String {
    path.file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Untitled".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_label_is_the_first_non_empty_line_without_heading_marks() {
        let label = untitled_label(["", "   ", "## Meeting notes  ", "body"]);
        assert_eq!(label.text.as_deref(), Some("Meeting notes"));
        assert_eq!(label.watch_through, 2);
        let empty = untitled_label(["", "  "]);
        assert_eq!(empty.text, None);
        assert_eq!(empty.watch_through, LABEL_SCAN_LINES - 1);
    }

    #[test]
    fn labels_cut_on_char_boundaries_and_skip_hash_only_lines() {
        // Break caught: slicing a long emoji or Arabic first line at a byte index (a panic), or
        // labelling a tab "###".
        let long = "😀".repeat(60);
        let label = untitled_label(["###", "   #  ", &long]).text.unwrap();
        assert_eq!(label.chars().count(), LABEL_LIMIT);
        assert!(label.ends_with('…'));
        let arabic = "ملاحظات الاجتماع الأسبوعي حول خطة الإصدار القادم والمزيد";
        assert!(untitled_label([arabic]).text.unwrap().chars().count() <= LABEL_LIMIT);
        assert_eq!(untitled_label(["#idea"]).text.as_deref(), Some("idea"));
    }

    #[test]
    fn only_the_first_sixteen_lines_are_considered() {
        let mut lines = vec![""; 20];
        lines[18] = "late";
        assert_eq!(untitled_label(lines).text, None);
    }

    #[test]
    fn stems_drop_invalid_characters_trailing_dots_and_reserved_names() {
        // Break caught: a first line like `a/b: c?` or `CON` producing a name Windows refuses,
        // so the first save fails with a confusing error.
        assert_eq!(sanitize_stem(r#"a/b: "c"? <d>|*"#), "ab c d");
        assert_eq!(sanitize_stem("Notes...  "), "Notes");
        assert_eq!(sanitize_stem("Long title…"), "Long title");
        assert_eq!(sanitize_stem("con"), "con_");
        assert_eq!(sanitize_stem("LPT9.draft"), "LPT9_.draft");
        assert_eq!(sanitize_stem("con.foo"), "con_.foo");
        assert_eq!(sanitize_stem("con .foo"), "con_ .foo");
        assert_eq!(sanitize_stem("console"), "console");
        assert_eq!(sanitize_stem("  \t "), "Untitled");
        assert_eq!(sanitize_stem("tab\there"), "tabhere");
    }

    #[test]
    fn typed_names_keep_a_known_extension_and_otherwise_get_the_default() {
        assert_eq!(
            split_typed_name("plan.txt", "md"),
            ("plan".into(), "txt".into())
        );
        assert_eq!(
            split_typed_name("v1.2 plan", "md"),
            ("v1.2 plan".into(), "md".into())
        );
        assert_eq!(
            split_typed_name("data.JSON", "md"),
            ("data".into(), "JSON".into())
        );
        assert_eq!(
            split_typed_name(".md", "md"),
            ("Untitled".into(), "md".into())
        );
        assert_eq!(default_extension(Language::Json), "json");
        assert_eq!(default_extension(Language::PlainText), "md");
    }

    #[test]
    fn clashing_names_get_the_first_free_number() {
        let taken = ["Plan.md", "Plan 2.md"];
        let exists = |name: &str| taken.iter().any(|t| t.eq_ignore_ascii_case(name));
        assert_eq!(free_name("Plan", "md", exists), "Plan 3.md");
        assert_eq!(free_name("plan", "md", |_| false), "plan.md");
        assert_eq!(free_name("PLAN", "MD", exists), "PLAN 3.MD");
    }

    #[test]
    fn a_saved_notes_title_is_its_file_stem() {
        assert_eq!(
            note_title(Path::new(r"D:\Notes\sub\Meeting notes.md")),
            "Meeting notes"
        );
        assert!(is_note_extension("YML"));
        assert!(!is_note_extension("png"));
    }
}
