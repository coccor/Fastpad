//! Material Icon Theme's 12 tree icons (icon sets spec §3, §5): premultiplied BGRA pixels at each
//! stored size, generated from `assets/icons/material/svg/` by `generate.rs` and embedded. An
//! icon's place in the blob follows from `MaterialIcon::ALL` and `SIZES`: icons in list order,
//! sizes smallest first, each `size * size * 4` bytes.

/// The pixel sizes stored for every icon: 100%, 125%, 150%, 200% and 300% scaling.
pub(crate) const SIZES: [u32; 5] = [16, 20, 24, 32, 48];

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "read by icon_sets::pixels, wired up by a later task's drawing"
    )
)]
const PER_ICON: usize = {
    let mut total = 0;
    let mut index = 0;
    while index < SIZES.len() {
        total += (SIZES[index] * SIZES[index] * 4) as usize;
        index += 1;
    }
    total
};

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "read by icon_sets::pixels, wired up by a later task's drawing"
    )
)]
static BLOB: &[u8] = include_bytes!("../../../assets/icons/material/icons.bin");

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
#[cfg_attr(
    not(test),
    allow(dead_code, reason = "the icon set lookup wired up by a later task")
)]
pub(crate) enum MaterialIcon {
    Markdown,
    Json,
    Yaml,
    Toml,
    TomlLight,
    Settings,
    Table,
    Xml,
    Document,
    Log,
    Folder,
    FolderOpen,
}

impl MaterialIcon {
    /// Every icon, in blob order (`ALL[icon as usize] == icon`).
    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "the icon set lookup wired up by a later task")
    )]
    pub(crate) const ALL: [Self; 12] = [
        Self::Markdown,
        Self::Json,
        Self::Yaml,
        Self::Toml,
        Self::TomlLight,
        Self::Settings,
        Self::Table,
        Self::Xml,
        Self::Document,
        Self::Log,
        Self::Folder,
        Self::FolderOpen,
    ];

    /// The SVG in `assets/icons/material/svg/` this icon is made from.
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "read by the generator and by a later task's drawing"
        )
    )]
    pub(crate) const fn file_name(self) -> &'static str {
        match self {
            Self::Markdown => "markdown.svg",
            Self::Json => "json.svg",
            Self::Yaml => "yaml.svg",
            Self::Toml => "toml.svg",
            Self::TomlLight => "toml_light.svg",
            Self::Settings => "settings.svg",
            Self::Table => "table.svg",
            Self::Xml => "xml.svg",
            Self::Document => "document.svg",
            Self::Log => "log.svg",
            Self::Folder => "folder.svg",
            Self::FolderOpen => "folder-open.svg",
        }
    }
}

/// `icon` at `size` px: premultiplied BGRA, top-down rows, `size * size * 4` bytes. `None` for a
/// size not in `SIZES`.
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "wired up by the icon set drawing added in a later task"
    )
)]
pub(crate) fn pixels(icon: MaterialIcon, size: u32) -> Option<&'static [u8]> {
    let index = SIZES.iter().position(|&stored| stored == size)?;
    let before: usize = SIZES[..index]
        .iter()
        .map(|&stored| (stored * stored * 4) as usize)
        .sum();
    let start = icon as usize * PER_ICON + before;
    BLOB.get(start..start + (size * size * 4) as usize)
}

/// The SVG sources' folder, for the generator and the data tests.
#[cfg(test)]
pub(crate) fn svg_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(r"assets\icons\material\svg")
}

/// 64-bit FNV-1a over every SVG's bytes, in `ALL` order: what `icons.source-hash` records.
#[cfg(test)]
pub(crate) fn source_hash() -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for icon in MaterialIcon::ALL {
        for byte in std::fs::read(svg_dir().join(icon.file_name())).unwrap() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_icon_list_names_exactly_the_bundled_svgs_and_the_blob_matches_them() {
        // Break caught: an SVG added or renamed without a list entry, a regeneration forgotten
        // after an SVG changed, or a blob cut short (icon sets spec §5.1, §9).
        let mut listed: Vec<&str> = MaterialIcon::ALL
            .iter()
            .map(|icon| icon.file_name())
            .collect();
        listed.sort_unstable();
        let mut files: Vec<String> = std::fs::read_dir(svg_dir())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        files.sort_unstable();
        assert_eq!(listed, files);
        for (index, icon) in MaterialIcon::ALL.into_iter().enumerate() {
            assert_eq!(icon as usize, index);
        }
        assert_eq!(BLOB.len(), MaterialIcon::ALL.len() * PER_ICON);
        let recorded =
            std::fs::read_to_string(svg_dir().parent().unwrap().join("icons.source-hash")).unwrap();
        assert_eq!(
            recorded.trim(),
            format!("{:016x}", source_hash()),
            "run tools/generate-file-icons.ps1"
        );
    }

    #[test]
    fn every_icon_has_visible_premultiplied_pixels_at_every_size() {
        // Break caught: an icon rendered blank (an SVG Direct2D cannot draw), straight rather
        // than premultiplied alpha, or `pixels` reading the wrong slice.
        for icon in MaterialIcon::ALL {
            for size in SIZES {
                let pixels = pixels(icon, size).unwrap();
                assert_eq!(pixels.len(), (size * size * 4) as usize);
                let (chunks, _) = pixels.as_chunks::<4>();
                assert!(chunks.iter().any(|p| p[3] > 0), "{icon:?} {size}");
                assert!(
                    chunks
                        .iter()
                        .all(|p| p[0] <= p[3] && p[1] <= p[3] && p[2] <= p[3]),
                    "{icon:?} {size}"
                );
            }
        }
        assert_eq!(pixels(MaterialIcon::Markdown, 28), None);
    }
}
