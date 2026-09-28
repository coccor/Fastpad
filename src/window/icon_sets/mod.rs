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
            NoteKind::Ini | NoteKind::Config | NoteKind::Properties => MaterialIcon::Settings,
            NoteKind::Csv => MaterialIcon::Table,
            NoteKind::Xml => MaterialIcon::Xml,
            NoteKind::Text => MaterialIcon::Document,
            NoteKind::Log => MaterialIcon::Log,
            NoteKind::Image => MaterialIcon::Image,
            NoteKind::Html => MaterialIcon::Html,
            NoteKind::Css => MaterialIcon::Css,
            NoteKind::JavaScript => MaterialIcon::JavaScript,
            NoteKind::TypeScript => MaterialIcon::TypeScript,
            NoteKind::Python => MaterialIcon::Python,
            NoteKind::Rust => MaterialIcon::Rust,
            NoteKind::C => MaterialIcon::C,
            NoteKind::Cpp => MaterialIcon::Cpp,
            NoteKind::CSharp => MaterialIcon::CSharp,
            NoteKind::PowerShell => MaterialIcon::PowerShell,
            NoteKind::Bash | NoteKind::Batch => MaterialIcon::Console,
            NoteKind::Sql => MaterialIcon::Database,
            NoteKind::Env => MaterialIcon::Tune,
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
        let note = |path: &str| TreeItem::Note(note_kind(std::path::Path::new(path)));
        let material = |item, light| tree_icon(FileIconSet::Material, item, light, false);
        for (extension, icon) in [
            ("a.md", MaterialIcon::Markdown),
            ("a.MARKDOWN", MaterialIcon::Markdown),
            ("a.json", MaterialIcon::Json),
            ("a.yml", MaterialIcon::Yaml),
            ("a.yaml", MaterialIcon::Yaml),
            ("a.ini", MaterialIcon::Settings),
            ("a.cfg", MaterialIcon::Settings),
            ("a.Conf", MaterialIcon::Settings),
            ("a.csv", MaterialIcon::Table),
            ("a.xml", MaterialIcon::Xml),
            ("a.txt", MaterialIcon::Document),
            ("a.text", MaterialIcon::Document),
            ("a.exe", MaterialIcon::Document),
            ("a.log", MaterialIcon::Log),
            ("a.png", MaterialIcon::Image),
            ("a.SVG", MaterialIcon::Image),
            ("a.html", MaterialIcon::Html),
            ("a.css", MaterialIcon::Css),
            ("a.mjs", MaterialIcon::JavaScript),
            ("a.ts", MaterialIcon::TypeScript),
            ("a.py", MaterialIcon::Python),
            ("a.rs", MaterialIcon::Rust),
            ("a.c", MaterialIcon::C),
            ("a.cpp", MaterialIcon::Cpp),
            ("a.cs", MaterialIcon::CSharp),
            ("a.ps1", MaterialIcon::PowerShell),
            ("a.sh", MaterialIcon::Console),
            (".bashrc", MaterialIcon::Console),
            ("a.bat", MaterialIcon::Console),
            ("a.sql", MaterialIcon::Database),
            (".env", MaterialIcon::Tune),
            ("a.properties", MaterialIcon::Settings),
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
            material(note("README"), false),
            TreeIcon::Image(MaterialIcon::Document)
        );
        assert_eq!(
            material(note("a.toml"), true),
            TreeIcon::Image(MaterialIcon::TomlLight)
        );
        assert_eq!(
            material(note("a.toml"), false),
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
        for item in [closed, open, note("a.md"), note("a.csv"), note("a.rs")] {
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
