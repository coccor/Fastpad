//! The Notebook view's note-type icons (notebook folders spec §5): for each note extension, a
//! glyph (or the `{}` label), the font it is drawn in and the Catppuccin colour role that
//! `palette::FileIcons` resolves per theme; and the type name screen readers hear. Pure: no
//! Win32, no disk.

/// Which of the sidebar's fonts an icon is drawn in.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum IconFont {
    /// Segoe MDL2 Assets (`UiFonts::glyph`).
    Glyph,
    /// The bold UI font (`UiFonts::bold`), for the `{}` label.
    Bold,
}

/// A Catppuccin colour role; `palette::FileIcons` holds each theme's colour for it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum IconColor {
    Blue,
    Yellow,
    Peach,
    Green,
    Maroon,
    Overlay2,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FileIcon {
    pub(crate) text: &'static str,
    pub(crate) font: IconFont,
    pub(crate) color: IconColor,
}

const GLYPH_DOCUMENT: &str = "\u{E8A5}";
const GLYPH_SETTINGS: &str = "\u{E713}";
const GLYPH_GRID: &str = "\u{E80A}";
const GLYPH_CODE: &str = "\u{E943}";
const GLYPH_FOLDER: &str = "\u{E8B7}";

const fn icon(text: &'static str, font: IconFont, color: IconColor) -> FileIcon {
    FileIcon { text, font, color }
}

/// A folder row's icon.
pub(crate) const FOLDER_ICON: FileIcon = icon(GLYPH_FOLDER, IconFont::Glyph, IconColor::Yellow);

/// The note types: each group of extensions that shares an icon and a name.
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
}

/// Every note extension (`title::NOTE_EXTENSIONS`) and its type.
const NOTE_KINDS: [(&str, NoteKind); 14] = [
    ("md", NoteKind::Markdown),
    ("markdown", NoteKind::Markdown),
    ("json", NoteKind::Json),
    ("yaml", NoteKind::Yaml),
    ("yml", NoteKind::Yaml),
    ("toml", NoteKind::Toml),
    ("ini", NoteKind::Ini),
    ("cfg", NoteKind::Config),
    ("conf", NoteKind::Config),
    ("csv", NoteKind::Csv),
    ("xml", NoteKind::Xml),
    ("txt", NoteKind::Text),
    ("text", NoteKind::Text),
    ("log", NoteKind::Log),
];

/// `extension`'s type, ignoring case; anything else (or no extension) is text.
pub(crate) fn note_kind(extension: Option<&str>) -> NoteKind {
    extension
        .and_then(|extension| {
            NOTE_KINDS
                .iter()
                .find(|(known, _)| known.eq_ignore_ascii_case(extension))
        })
        .map_or(NoteKind::Text, |&(_, kind)| kind)
}

/// The Minimal set's icon for a note type (spec §5.1 of notebook folders; icon sets spec §3.1).
pub(crate) fn minimal_icon(kind: NoteKind) -> FileIcon {
    match kind {
        NoteKind::Markdown => icon(GLYPH_DOCUMENT, IconFont::Glyph, IconColor::Blue),
        NoteKind::Json => icon("{}", IconFont::Bold, IconColor::Yellow),
        NoteKind::Yaml | NoteKind::Toml | NoteKind::Ini | NoteKind::Config => {
            icon(GLYPH_SETTINGS, IconFont::Glyph, IconColor::Peach)
        }
        NoteKind::Csv => icon(GLYPH_GRID, IconFont::Glyph, IconColor::Green),
        NoteKind::Xml => icon(GLYPH_CODE, IconFont::Glyph, IconColor::Maroon),
        NoteKind::Text | NoteKind::Log => {
            icon(GLYPH_DOCUMENT, IconFont::Glyph, IconColor::Overlay2)
        }
    }
}

/// A note row's Minimal icon, from its file's extension.
#[cfg(test)]
pub(crate) fn file_icon(extension: Option<&str>) -> FileIcon {
    minimal_icon(note_kind(extension))
}

/// The type a note row's accessible name carries after its name (spec §5.4).
pub(crate) fn type_name(extension: Option<&str>) -> &'static str {
    match note_kind(extension) {
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_note_type_has_its_icon_font_and_colour_role_in_any_letter_case() {
        // Break caught: a JSON note drawn with the document glyph, a config file in the text
        // colour, or `.MD` falling back to plain text (spec §5.1).
        let cases = [
            ("md", GLYPH_DOCUMENT, IconFont::Glyph, IconColor::Blue),
            ("markdown", GLYPH_DOCUMENT, IconFont::Glyph, IconColor::Blue),
            ("json", "{}", IconFont::Bold, IconColor::Yellow),
            ("yaml", GLYPH_SETTINGS, IconFont::Glyph, IconColor::Peach),
            ("yml", GLYPH_SETTINGS, IconFont::Glyph, IconColor::Peach),
            ("toml", GLYPH_SETTINGS, IconFont::Glyph, IconColor::Peach),
            ("ini", GLYPH_SETTINGS, IconFont::Glyph, IconColor::Peach),
            ("cfg", GLYPH_SETTINGS, IconFont::Glyph, IconColor::Peach),
            ("conf", GLYPH_SETTINGS, IconFont::Glyph, IconColor::Peach),
            ("csv", GLYPH_GRID, IconFont::Glyph, IconColor::Green),
            ("xml", GLYPH_CODE, IconFont::Glyph, IconColor::Maroon),
            ("txt", GLYPH_DOCUMENT, IconFont::Glyph, IconColor::Overlay2),
            ("text", GLYPH_DOCUMENT, IconFont::Glyph, IconColor::Overlay2),
            ("log", GLYPH_DOCUMENT, IconFont::Glyph, IconColor::Overlay2),
        ];
        for (extension, text, font, color) in cases {
            let expected = FileIcon { text, font, color };
            assert_eq!(file_icon(Some(extension)), expected, "{extension}");
            assert_eq!(
                file_icon(Some(&extension.to_uppercase())),
                expected,
                "{extension}"
            );
        }
        assert_eq!(file_icon(Some("Md")), file_icon(Some("md")));
        assert_eq!(file_icon(Some("JsOn")).text, "{}");
        let text = file_icon(Some("txt"));
        assert_eq!(file_icon(None), text);
        assert_eq!(file_icon(Some("py")), text);
        assert_eq!(file_icon(Some("")), text);
        assert_eq!(
            FOLDER_ICON,
            FileIcon {
                text: "\u{E8B7}",
                font: IconFont::Glyph,
                color: IconColor::Yellow
            }
        );
        assert_eq!(
            (GLYPH_DOCUMENT, GLYPH_SETTINGS, GLYPH_GRID, GLYPH_CODE),
            ("\u{E8A5}", "\u{E713}", "\u{E80A}", "\u{E943}")
        );
    }

    #[test]
    fn every_note_extension_has_a_type_name_for_screen_readers() {
        // Break caught: a note type added to NOTE_EXTENSIONS read out as "text", or `cfg` and
        // `conf` named differently (spec §5.4).
        for extension in crate::library::title::NOTE_EXTENSIONS {
            assert!(
                NOTE_KINDS.iter().any(|(known, _)| *known == extension),
                "{extension}"
            );
        }
        for (extension, name) in [
            ("md", "Markdown"),
            ("markdown", "Markdown"),
            ("json", "JSON"),
            ("yaml", "YAML"),
            ("yml", "YAML"),
            ("toml", "TOML"),
            ("ini", "INI"),
            ("cfg", "config"),
            ("conf", "config"),
            ("CSV", "CSV"),
            ("xml", "XML"),
            ("txt", "text"),
            ("text", "text"),
            ("log", "log"),
        ] {
            assert_eq!(type_name(Some(extension)), name, "{extension}");
        }
        assert_eq!(type_name(None), "text");
    }
}
