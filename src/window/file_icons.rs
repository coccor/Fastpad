//! The Notebook view's file types (notebook folders spec §5): each listed file's type, the
//! Catppuccin colour roles that `palette::FileIcons` resolves per theme for the tinted icon sets
//! (`icon_sets::masks`), and the type name screen readers hear. Pure: no Win32, no disk.

use crate::document::Language;
use std::path::Path;

/// A Catppuccin colour role; `palette::FileIcons` holds each theme's colour for it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum IconColor {
    Blue,
    Yellow,
    Peach,
    Green,
    Maroon,
    Overlay2,
    Pink,
    Mauve,
    Sky,
}

/// The note types: each group of files that shares an icon and a name.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NoteKind {
    Markdown,
    Json,
    Yaml,
    Toml,
    Ini,
    Config,
    Csv,
    Xml,
    Text,
    Log,
    /// Every image extension, SVG included (image preview spec §9).
    Image,
    Html,
    Css,
    JavaScript,
    TypeScript,
    Python,
    Rust,
    C,
    Cpp,
    CSharp,
    PowerShell,
    Bash,
    Batch,
    Sql,
    Env,
    Properties,
}

/// The listed extensions whose type is not their language's: `cfg` and `conf` are INI to the
/// lexer but "config" in the tree, and text, logs, CSV and images have no language of their own.
const FILE_KINDS: [(&str, NoteKind); 22] = [
    ("cfg", NoteKind::Config),
    ("conf", NoteKind::Config),
    ("csv", NoteKind::Csv),
    ("txt", NoteKind::Text),
    ("text", NoteKind::Text),
    ("log", NoteKind::Log),
    ("png", NoteKind::Image),
    ("jpg", NoteKind::Image),
    ("jpeg", NoteKind::Image),
    ("jpe", NoteKind::Image),
    ("jfif", NoteKind::Image),
    ("gif", NoteKind::Image),
    ("bmp", NoteKind::Image),
    ("dib", NoteKind::Image),
    ("ico", NoteKind::Image),
    ("tif", NoteKind::Image),
    ("tiff", NoteKind::Image),
    ("webp", NoteKind::Image),
    ("heic", NoteKind::Image),
    ("heif", NoteKind::Image),
    ("avif", NoteKind::Image),
    ("svg", NoteKind::Image),
];

/// `path`'s type, from its extension (ignoring case) or else the language FastPad highlights it
/// as, so `.env` and `Cargo.lock` get theirs; anything else is text.
pub(crate) fn note_kind(path: &Path) -> NoteKind {
    let listed = path.extension().and_then(|extension| {
        let extension = extension.to_string_lossy();
        FILE_KINDS
            .iter()
            .find(|(known, _)| known.eq_ignore_ascii_case(&extension))
            .map(|&(_, kind)| kind)
    });
    listed.unwrap_or_else(|| language_kind(crate::languages::detect_language(path)))
}

const fn language_kind(language: Language) -> NoteKind {
    match language {
        Language::PlainText => NoteKind::Text,
        Language::Markdown => NoteKind::Markdown,
        Language::Json => NoteKind::Json,
        Language::Yaml => NoteKind::Yaml,
        Language::Toml => NoteKind::Toml,
        Language::Ini => NoteKind::Ini,
        Language::Xml => NoteKind::Xml,
        Language::Svg => NoteKind::Image,
        Language::Html => NoteKind::Html,
        Language::Css => NoteKind::Css,
        Language::JavaScript => NoteKind::JavaScript,
        Language::TypeScript => NoteKind::TypeScript,
        Language::Python => NoteKind::Python,
        Language::Rust => NoteKind::Rust,
        Language::C => NoteKind::C,
        Language::Cpp => NoteKind::Cpp,
        Language::CSharp => NoteKind::CSharp,
        Language::PowerShell => NoteKind::PowerShell,
        Language::Bash => NoteKind::Bash,
        Language::Batch => NoteKind::Batch,
        Language::Sql => NoteKind::Sql,
        Language::Env => NoteKind::Env,
        Language::Properties => NoteKind::Properties,
    }
}

/// The type a note row's accessible name carries after its name (spec §5.4).
pub(crate) fn type_name(path: &Path) -> &'static str {
    match note_kind(path) {
        NoteKind::Markdown => "Markdown",
        NoteKind::Json => "JSON",
        NoteKind::Yaml => "YAML",
        NoteKind::Toml => "TOML",
        NoteKind::Ini => "INI",
        NoteKind::Config => "config",
        NoteKind::Csv => "CSV",
        NoteKind::Xml => "XML",
        NoteKind::Text => "text",
        NoteKind::Log => "log",
        NoteKind::Image => "image",
        NoteKind::Html => "HTML",
        NoteKind::Css => "CSS",
        NoteKind::JavaScript => "JavaScript",
        NoteKind::TypeScript => "TypeScript",
        NoteKind::Python => "Python",
        NoteKind::Rust => "Rust",
        NoteKind::C => "C",
        NoteKind::Cpp => "C++",
        NoteKind::CSharp => "C#",
        NoteKind::PowerShell => "PowerShell",
        NoteKind::Bash => "Bash",
        NoteKind::Batch => "Batch",
        NoteKind::Sql => "SQL",
        NoteKind::Env => "Env",
        NoteKind::Properties => "Properties",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_listed_file_has_a_type_name_for_screen_readers() {
        // Break caught: a note type added to NOTE_EXTENSIONS read out as "text", `cfg` and
        // `conf` named differently (spec §5.4), an image read out as "text", or a highlighted
        // language's file read out as "text".
        for extension in crate::library::title::NOTE_EXTENSIONS
            .iter()
            .chain(crate::library::title::IMAGE_EXTENSIONS.iter())
            .filter(|extension| !matches!(**extension, "txt" | "text"))
        {
            let path = format!("a.{extension}");
            assert_ne!(note_kind(Path::new(&path)), NoteKind::Text, "{path}");
        }
        for row in crate::languages::LANGUAGES
            .iter()
            .filter(|row| row.language != crate::document::Language::PlainText)
        {
            let extensions = row
                .extensions
                .iter()
                .map(|extension| format!("a.{extension}"));
            let names = row.file_names.iter().map(|name| name.replace('*', "local"));
            for path in extensions.chain(names) {
                assert_ne!(note_kind(Path::new(&path)), NoteKind::Text, "{path}");
            }
        }
        for (path, name) in [
            ("a.md", "Markdown"),
            ("a.markdown", "Markdown"),
            ("a.json", "JSON"),
            ("a.yaml", "YAML"),
            ("a.yml", "YAML"),
            ("a.toml", "TOML"),
            ("Cargo.lock", "TOML"),
            ("a.ini", "INI"),
            (".editorconfig", "INI"),
            ("a.cfg", "config"),
            ("a.conf", "config"),
            ("a.CSV", "CSV"),
            ("a.xml", "XML"),
            ("a.csproj", "XML"),
            ("a.txt", "text"),
            ("a.text", "text"),
            ("a.log", "log"),
            ("a.png", "image"),
            ("a.svg", "image"),
            ("a.html", "HTML"),
            ("a.css", "CSS"),
            ("a.js", "JavaScript"),
            ("a.ts", "TypeScript"),
            ("a.py", "Python"),
            ("a.rs", "Rust"),
            ("a.c", "C"),
            ("a.hpp", "C++"),
            ("a.cs", "C#"),
            ("a.ps1", "PowerShell"),
            ("a.sh", "Bash"),
            (".bashrc", "Bash"),
            ("a.bat", "Batch"),
            ("a.sql", "SQL"),
            (".env", "Env"),
            (".env.local", "Env"),
            ("a.properties", "Properties"),
            ("README", "text"),
            ("a.exe", "text"),
        ] {
            assert_eq!(type_name(Path::new(path)), name, "{path}");
        }
    }
}
