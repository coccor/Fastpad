//! The Notebook view's note types (notebook folders spec §5): each note extension's type, the
//! Catppuccin colour roles that `palette::FileIcons` resolves per theme for the tinted icon sets
//! (`icon_sets::masks`), and the type name screen readers hear. Pure: no Win32, no disk.

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
