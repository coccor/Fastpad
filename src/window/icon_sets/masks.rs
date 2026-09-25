//! The Minimal and Solid sets' tree icons (icon sets spec §3, §5): FastPad's own drawings, one
//! shape per icon with no colour of its own. Each set's SVGs in `assets/icons/<set>/svg/` are
//! rendered by `generate.rs` into coverage masks (one alpha byte per pixel) at each of
//! `material::SIZES` and embedded; the drawing code tints a mask with the row's colour. An
//! icon's place in a set's blob follows from `MaskIcon::ALL` and `SIZES`: icons in list order,
//! sizes smallest first, each `size * size` bytes.

use super::TreeItem;
use super::material::SIZES;
use crate::window::file_icons::{IconColor, NoteKind};

const PER_ICON: usize = {
    let mut total = 0;
    let mut index = 0;
    while index < SIZES.len() {
        total += (SIZES[index] * SIZES[index]) as usize;
        index += 1;
    }
    total
};

static MINIMAL: &[u8] = include_bytes!("../../../assets/icons/minimal/icons.bin");
static SOLID: &[u8] = include_bytes!("../../../assets/icons/solid/icons.bin");

/// A set drawn from masks: Minimal is outlines, Solid is filled shapes.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub(crate) enum MaskSet {
    Minimal,
    Solid,
}

impl MaskSet {
    #[cfg(test)]
    pub(crate) const ALL: [Self; 2] = [Self::Minimal, Self::Solid];

    const fn blob(self) -> &'static [u8] {
        match self {
            Self::Minimal => MINIMAL,
            Self::Solid => SOLID,
        }
    }

    /// The set's folder under `assets/icons/`.
    #[cfg(test)]
    pub(crate) const fn folder(self) -> &'static str {
        match self {
            Self::Minimal => "minimal",
            Self::Solid => "solid",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub(crate) enum MaskIcon {
    Folder,
    FolderOpen,
    Markdown,
    Braces,
    Settings,
    Table,
    Code,
    Document,
    Log,
}

impl MaskIcon {
    /// Every icon, in blob order (`ALL[icon as usize] == icon`).
    #[cfg(test)]
    pub(crate) const ALL: [Self; 9] = [
        Self::Folder,
        Self::FolderOpen,
        Self::Markdown,
        Self::Braces,
        Self::Settings,
        Self::Table,
        Self::Code,
        Self::Document,
        Self::Log,
    ];

    /// The SVG in each set's `svg/` folder this icon is made from.
    #[cfg(test)]
    pub(crate) const fn file_name(self) -> &'static str {
        match self {
            Self::Folder => "folder.svg",
            Self::FolderOpen => "folder-open.svg",
            Self::Markdown => "markdown.svg",
            Self::Braces => "braces.svg",
            Self::Settings => "settings.svg",
            Self::Table => "table.svg",
            Self::Code => "code.svg",
            Self::Document => "document.svg",
            Self::Log => "log.svg",
        }
    }
}

/// The mask icon `item` draws and the Catppuccin role it is tinted with (spec §3.1): the same in
/// both mask sets.
pub(crate) fn mask_icon(item: TreeItem) -> (MaskIcon, IconColor) {
    match item {
        TreeItem::Folder { expanded: false } => (MaskIcon::Folder, IconColor::Yellow),
        TreeItem::Folder { expanded: true } => (MaskIcon::FolderOpen, IconColor::Yellow),
        TreeItem::Note(kind) => match kind {
            NoteKind::Markdown => (MaskIcon::Markdown, IconColor::Blue),
            NoteKind::Json => (MaskIcon::Braces, IconColor::Yellow),
            NoteKind::Yaml | NoteKind::Toml | NoteKind::Ini | NoteKind::Config => {
                (MaskIcon::Settings, IconColor::Peach)
            }
            NoteKind::Csv => (MaskIcon::Table, IconColor::Green),
            NoteKind::Xml => (MaskIcon::Code, IconColor::Maroon),
            NoteKind::Text => (MaskIcon::Document, IconColor::Overlay2),
            NoteKind::Log => (MaskIcon::Log, IconColor::Overlay2),
        },
    }
}

/// `icon`'s coverage in `set` at `size` px: top-down rows, `size * size` bytes, 255 = fully
/// covered. `None` for a size not in `SIZES`.
pub(crate) fn coverage(set: MaskSet, icon: MaskIcon, size: u32) -> Option<&'static [u8]> {
    let index = SIZES.iter().position(|&stored| stored == size)?;
    let before: usize = SIZES[..index]
        .iter()
        .map(|&stored| (stored * stored) as usize)
        .sum();
    let start = icon as usize * PER_ICON + before;
    set.blob().get(start..start + (size * size) as usize)
}

/// `set`'s folder under `assets/icons/`, for the generator and the data tests.
#[cfg(test)]
pub(crate) fn set_dir(set: MaskSet) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(r"assets\icons")
        .join(set.folder())
}

/// 64-bit FNV-1a over every SVG's bytes of `set`, in `ALL` order: what its `icons.source-hash`
/// records.
#[cfg(test)]
pub(crate) fn source_hash(set: MaskSet) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for icon in MaskIcon::ALL {
        let path = set_dir(set).join("svg").join(icon.file_name());
        for byte in std::fs::read(path).unwrap() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::window::file_icons::note_kind;

    #[test]
    fn each_mask_set_bundles_exactly_its_svgs_and_its_blob_matches_them() {
        // Break caught: an SVG added or renamed without a list entry, a regeneration forgotten
        // after a designer changed an SVG, or a blob cut short (icon sets spec §5.1, §9).
        let mut listed: Vec<&str> = MaskIcon::ALL.iter().map(|icon| icon.file_name()).collect();
        listed.sort_unstable();
        for (index, icon) in MaskIcon::ALL.into_iter().enumerate() {
            assert_eq!(icon as usize, index);
        }
        for set in MaskSet::ALL {
            let mut files: Vec<String> = std::fs::read_dir(set_dir(set).join("svg"))
                .unwrap()
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            files.sort_unstable();
            assert_eq!(listed, files, "{set:?}");
            assert_eq!(set.blob().len(), MaskIcon::ALL.len() * PER_ICON, "{set:?}");
            let recorded = std::fs::read_to_string(set_dir(set).join("icons.source-hash")).unwrap();
            assert_eq!(
                recorded.trim(),
                format!("{:016x}", source_hash(set)),
                "{set:?}: run tools/generate-file-icons.ps1"
            );
        }
    }

    #[test]
    fn every_mask_is_visible_and_never_fills_its_square_at_every_size() {
        // Break caught: an icon rendered blank (a shape or stroke Direct2D cannot draw), one
        // filling its whole square (a missing fill="none" or viewBox), or `coverage` reading the
        // wrong slice.
        for set in MaskSet::ALL {
            for icon in MaskIcon::ALL {
                for size in SIZES {
                    let mask = coverage(set, icon, size).unwrap();
                    assert_eq!(mask.len(), (size * size) as usize);
                    let covered = mask.iter().filter(|&&alpha| alpha > 127).count();
                    assert!(covered > mask.len() / 20, "{set:?} {icon:?} {size}");
                    assert!(covered < mask.len() * 3 / 4, "{set:?} {icon:?} {size}");
                }
            }
        }
        assert_eq!(coverage(MaskSet::Solid, MaskIcon::Folder, 28), None);
    }

    #[test]
    fn minimal_outlines_are_lighter_than_solid_shapes() {
        // Break caught: the two sets' folders swapped, or an outline SVG filled (spec §3). Only
        // the icons that are the same shape in both sets: Minimal's Markdown adds a frame.
        for icon in [
            MaskIcon::Folder,
            MaskIcon::FolderOpen,
            MaskIcon::Settings,
            MaskIcon::Document,
            MaskIcon::Log,
        ] {
            let ink = |set| -> u32 {
                coverage(set, icon, 24)
                    .unwrap()
                    .iter()
                    .map(|&alpha| u32::from(alpha))
                    .sum()
            };
            assert!(ink(MaskSet::Minimal) < ink(MaskSet::Solid), "{icon:?}");
        }
    }

    #[test]
    fn every_note_type_and_folder_state_has_its_mask_icon_and_colour_role() {
        // Break caught: a config file drawn as a document, the closed folder shown open, or a
        // type's colour role changed from the Minimal glyphs' (spec §3.1).
        let note = |extension: &str| mask_icon(TreeItem::Note(note_kind(Some(extension))));
        for (extension, expected) in [
            ("md", (MaskIcon::Markdown, IconColor::Blue)),
            ("markdown", (MaskIcon::Markdown, IconColor::Blue)),
            ("json", (MaskIcon::Braces, IconColor::Yellow)),
            ("yaml", (MaskIcon::Settings, IconColor::Peach)),
            ("yml", (MaskIcon::Settings, IconColor::Peach)),
            ("toml", (MaskIcon::Settings, IconColor::Peach)),
            ("ini", (MaskIcon::Settings, IconColor::Peach)),
            ("cfg", (MaskIcon::Settings, IconColor::Peach)),
            ("conf", (MaskIcon::Settings, IconColor::Peach)),
            ("csv", (MaskIcon::Table, IconColor::Green)),
            ("xml", (MaskIcon::Code, IconColor::Maroon)),
            ("txt", (MaskIcon::Document, IconColor::Overlay2)),
            ("py", (MaskIcon::Document, IconColor::Overlay2)),
            ("log", (MaskIcon::Log, IconColor::Overlay2)),
        ] {
            assert_eq!(note(extension), expected, "{extension}");
        }
        assert_eq!(
            mask_icon(TreeItem::Note(note_kind(None))),
            (MaskIcon::Document, IconColor::Overlay2)
        );
        assert_eq!(
            mask_icon(TreeItem::Folder { expanded: false }),
            (MaskIcon::Folder, IconColor::Yellow)
        );
        assert_eq!(
            mask_icon(TreeItem::Folder { expanded: true }),
            (MaskIcon::FolderOpen, IconColor::Yellow)
        );
    }
}
