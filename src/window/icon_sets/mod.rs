//! The Notebook tree's icon sets (icon sets spec): which icon a row draws, from the chosen set,
//! the theme and high contrast.

#[cfg(test)]
mod generate;
pub(crate) mod images;
pub(crate) mod masks;
pub(crate) mod material;
pub(crate) mod resample;

use crate::config::FileIconSet;
use crate::window::file_icons::{IconColor, NoteKind};
use masks::{MaskIcon, MaskSet, mask_icon};
use material::MaterialIcon;

/// The icon a tree row draws.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TreeIcon {
    /// A Material bitmap, drawn in its own colours.
    Image(MaterialIcon),
    /// A Minimal or Solid shape, tinted with a Catppuccin colour role (or, in high contrast, the
    /// muted system colour).
    Mask {
        set: MaskSet,
        icon: MaskIcon,
        color: IconColor,
    },
}

/// What a tree row shows an icon for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TreeItem {
    Folder { expanded: bool },
    Note(NoteKind),
}

/// `item`'s Minimal icon: high contrast's in every set, and what a Material bitmap that cannot
/// be made falls back to.
pub(crate) fn minimal(item: TreeItem) -> TreeIcon {
    let (icon, color) = mask_icon(item);
    TreeIcon::Mask {
        set: MaskSet::Minimal,
        icon,
        color,
    }
}

/// The icon `item` draws in `set` (spec §3.1, §3.2). High contrast draws Minimal in every set.
pub(crate) fn tree_icon(
    set: FileIconSet,
    item: TreeItem,
    light_theme: bool,
    high_contrast: bool,
) -> TreeIcon {
    if high_contrast {
        return minimal(item);
    }
    let mask = |set| {
        let (icon, color) = mask_icon(item);
        TreeIcon::Mask { set, icon, color }
    };
    match set {
        FileIconSet::Minimal => return mask(MaskSet::Minimal),
        FileIconSet::Solid => return mask(MaskSet::Solid),
        FileIconSet::Material => {}
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
    use crate::window::file_icons::note_kind;

    #[test]
    fn every_note_type_and_folder_state_draws_its_icon_in_each_set() {
        // Break caught: a config file drawn as a document, the dark TOML icon on a light theme,
        // Solid drawn with Minimal's outlines, or a Material bitmap in high contrast (spec §3).
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
            for light in [false, true] {
                assert_eq!(
                    material(note(extension), light),
                    TreeIcon::Image(icon),
                    "{extension}"
                );
            }
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
        for item in [closed, open, note("md"), note("csv")] {
            let (icon, color) = mask_icon(item);
            for (set, mask_set) in [
                (FileIconSet::Minimal, MaskSet::Minimal),
                (FileIconSet::Solid, MaskSet::Solid),
            ] {
                assert_eq!(
                    tree_icon(set, item, false, false),
                    TreeIcon::Mask {
                        set: mask_set,
                        icon,
                        color
                    }
                );
            }
            for set in [
                FileIconSet::Material,
                FileIconSet::Minimal,
                FileIconSet::Solid,
            ] {
                assert_eq!(tree_icon(set, item, true, true), minimal(item), "{set:?}");
            }
        }
    }
}
