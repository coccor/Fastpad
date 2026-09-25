//! The Notebook tree's icon sets (icon sets spec): which icon a row draws, from the chosen set,
//! the theme and high contrast.

#[cfg(test)]
mod generate;
pub(crate) mod images;
pub(crate) mod material;
pub(crate) mod resample;

use crate::config::FileIconSet;
use crate::window::file_icons::{FOLDER_ICON, FileIcon, NoteKind, minimal_icon};
use material::MaterialIcon;

/// What a tree row shows an icon for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TreeItem {
    Folder { expanded: bool },
    Note(NoteKind),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TreeIcon {
    /// A Segoe glyph or label in a Catppuccin colour role (the Minimal set).
    Glyph(FileIcon),
    Image(MaterialIcon),
}

/// The icon `item` draws in `set` (spec §3.1, §3.2). High contrast draws Minimal in every set.
pub(crate) fn tree_icon(
    set: FileIconSet,
    item: TreeItem,
    light_theme: bool,
    high_contrast: bool,
) -> TreeIcon {
    if high_contrast || set == FileIconSet::Minimal {
        return TreeIcon::Glyph(match item {
            TreeItem::Folder { .. } => FOLDER_ICON,
            TreeItem::Note(kind) => minimal_icon(kind),
        });
    }
    TreeIcon::Image(match item {
        TreeItem::Folder { expanded: false } => MaterialIcon::Folder,
        TreeItem::Folder { expanded: true } => MaterialIcon::FolderOpen,
        TreeItem::Note(kind) => match kind {
            NoteKind::Markdown => MaterialIcon::Markdown,
            NoteKind::Json => MaterialIcon::Json,
            NoteKind::Yaml => MaterialIcon::Yaml,
            NoteKind::Toml if light_theme => MaterialIcon::TomlLight,
            NoteKind::Toml => MaterialIcon::Toml,
            NoteKind::Ini | NoteKind::Config => MaterialIcon::Settings,
            NoteKind::Csv => MaterialIcon::Table,
            NoteKind::Xml => MaterialIcon::Xml,
            NoteKind::Text => MaterialIcon::Document,
            NoteKind::Log => MaterialIcon::Log,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::window::file_icons::{file_icon, note_kind};

    #[test]
    fn every_note_type_and_folder_state_draws_its_icon_in_each_set() {
        // Break caught: a config file drawn as a document, the open folder in Minimal, the dark
        // TOML icon on a light theme, or a Material bitmap in high contrast (spec §3).
        let note = |extension: &str| TreeItem::Note(note_kind(Some(extension)));
        let material = |item, light| tree_icon(FileIconSet::Material, item, light, false);
        for (extension, icon) in [
            ("md", MaterialIcon::Markdown),
            ("MARKDOWN", MaterialIcon::Markdown),
            ("json", MaterialIcon::Json),
            ("yml", MaterialIcon::Yaml),
            ("yaml", MaterialIcon::Yaml),
            ("ini", MaterialIcon::Settings),
            ("cfg", MaterialIcon::Settings),
            ("Conf", MaterialIcon::Settings),
            ("csv", MaterialIcon::Table),
            ("xml", MaterialIcon::Xml),
            ("txt", MaterialIcon::Document),
            ("text", MaterialIcon::Document),
            ("py", MaterialIcon::Document),
            ("log", MaterialIcon::Log),
        ] {
            assert_eq!(
                material(note(extension), false),
                TreeIcon::Image(icon),
                "{extension}"
            );
            assert_eq!(
                material(note(extension), true),
                TreeIcon::Image(icon),
                "{extension}"
            );
            assert_eq!(
                tree_icon(FileIconSet::Minimal, note(extension), false, false),
                TreeIcon::Glyph(file_icon(Some(extension))),
                "{extension}"
            );
        }
        assert_eq!(
            material(TreeItem::Note(note_kind(None)), false),
            TreeIcon::Image(MaterialIcon::Document)
        );
        assert_eq!(
            material(note("toml"), true),
            TreeIcon::Image(MaterialIcon::TomlLight)
        );
        assert_eq!(
            material(note("toml"), false),
            TreeIcon::Image(MaterialIcon::Toml)
        );
        let (closed, open) = (
            TreeItem::Folder { expanded: false },
            TreeItem::Folder { expanded: true },
        );
        assert_eq!(
            material(closed, false),
            TreeIcon::Image(MaterialIcon::Folder)
        );
        assert_eq!(
            material(open, false),
            TreeIcon::Image(MaterialIcon::FolderOpen)
        );
        for set in [FileIconSet::Material, FileIconSet::Minimal] {
            assert_eq!(
                tree_icon(set, open, false, true),
                TreeIcon::Glyph(FOLDER_ICON)
            );
            assert_eq!(
                tree_icon(set, note("md"), false, true),
                TreeIcon::Glyph(file_icon(Some("md")))
            );
        }
        assert_eq!(
            tree_icon(FileIconSet::Minimal, open, false, false),
            TreeIcon::Glyph(FOLDER_ICON)
        );
    }
}
