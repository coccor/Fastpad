//! Per-PC state that must not travel with the notebook: the scan cache (file IDs only mean
//! something on one volume), which subfolders are expanded in the sidebar, when records went
//! missing, the notebook's autosave switch, and `folders.ini` (recent and favorite notebooks).
//! Losing any of it is harmless, so reads never fail.

use super::ids::{NoteId, fnv1a};
use super::model::same_path;
use crate::Result;
use std::path::{Path, PathBuf};

const VERSION: &str = "1";
pub const FOLDER_LIMIT: usize = 10;
pub const FAVORITE_LIMIT: usize = 50;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CachedFile {
    pub volume: u32,
    pub file_id: u64,
    pub mtime: u64,
    pub size: u64,
    pub path: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalState {
    pub folder: PathBuf,
    pub autosave: bool,
    /// Subfolders expanded in the Notebook view, relative to the notebook. The root is always
    /// expanded and never listed.
    pub expanded: Vec<PathBuf>,
    pub missing: Vec<(u64, NoteId)>,
    pub files: Vec<CachedFile>,
}

/// Everything in the local file except the scan cache: what the UI thread changes.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Conveniences {
    pub autosave: bool,
    pub expanded: Vec<PathBuf>,
    pub missing: Vec<(u64, NoteId)>,
}

impl LocalState {
    pub fn conveniences(&self) -> Conveniences {
        Conveniences {
            autosave: self.autosave,
            expanded: self.expanded.clone(),
            missing: self.missing.clone(),
        }
    }

    pub fn new(folder: PathBuf) -> Self {
        Self {
            folder,
            autosave: true,
            expanded: Vec::new(),
            missing: Vec::new(),
            files: Vec::new(),
        }
    }

    pub fn encode(&self) -> String {
        let mut output = format!(
            "version={VERSION}\r\nfolder={}\r\nautosave={}\r\n",
            self.folder.to_string_lossy(),
            self.autosave
        );
        for path in &self.expanded {
            output.push_str(&format!("expanded={}\r\n", path.to_string_lossy()));
        }
        for (time, id) in &self.missing {
            output.push_str(&format!("missing={time}|{}\r\n", id.to_hex()));
        }
        for file in &self.files {
            output.push_str(&format!(
                "file={}|{}|{}|{}|{}\r\n",
                file.volume,
                file.file_id,
                file.mtime,
                file.size,
                file.path.to_string_lossy()
            ));
        }
        output
    }

    /// `None` for anything but a version-1 file written for `folder`. Unknown lines are ignored.
    pub fn parse(source: &str, folder: &Path) -> Option<Self> {
        let source = source.strip_prefix('\u{feff}').unwrap_or(source);
        let mut version = None;
        let mut stored_folder = None;
        let mut state = Self::new(folder.to_path_buf());
        for line in source.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            match key {
                "version" => version = Some(value),
                "folder" => stored_folder = Some(PathBuf::from(value)),
                "autosave" => state.autosave = value != "false",
                "expanded" if !value.is_empty() => state.expanded.push(PathBuf::from(value)),
                "missing" => {
                    if let Some((time, id)) = value.split_once('|')
                        && let (Ok(time), Some(id)) = (time.parse(), NoteId::parse_hex(id))
                    {
                        state.missing.push((time, id));
                    }
                }
                "file" => state.files.extend(parse_file(value)),
                _ => {}
            }
        }
        let same_folder = stored_folder.is_some_and(|stored| {
            same_path(
                &super::normalize_folder(&stored),
                &super::normalize_folder(folder),
            )
        });
        if version != Some(VERSION) || !same_folder {
            return None;
        }
        state.folder = folder.to_path_buf();
        Some(state)
    }

    pub fn is_expanded(&self, path: &Path) -> bool {
        self.expanded
            .iter()
            .any(|existing| same_path(existing, path))
    }

    /// Records a subfolder as expanded or collapsed. Setting what is already set changes
    /// nothing, so it causes no write.
    pub fn set_expanded(&mut self, path: &Path, expanded: bool) {
        let present = self.is_expanded(path);
        if expanded && !present {
            self.expanded.push(path.to_path_buf());
        } else if !expanded && present {
            self.expanded.retain(|existing| !same_path(existing, path));
        }
    }

    pub fn missing_since(&self, id: NoteId) -> Option<u64> {
        self.missing
            .iter()
            .find(|(_, missing)| *missing == id)
            .map(|(time, _)| *time)
    }

    pub fn set_missing(&mut self, id: NoteId, now: u64) {
        if self.missing_since(id).is_none() {
            self.missing.push((now, id));
        }
    }

    pub fn clear_missing(&mut self, id: NoteId) {
        self.missing.retain(|(_, missing)| *missing != id);
    }
}

fn parse_file(value: &str) -> Option<CachedFile> {
    let mut fields = value.splitn(5, '|');
    let volume = fields.next()?.parse().ok()?;
    let file_id = fields.next()?.parse().ok()?;
    let mtime = fields.next()?.parse().ok()?;
    let size = fields.next()?.parse().ok()?;
    let path = fields.next()?;
    if path.is_empty() {
        return None;
    }
    Some(CachedFile {
        volume,
        file_id,
        mtime,
        size,
        path: PathBuf::from(path),
    })
}

/// The same key for every spelling of one folder (letter case, a trailing separator).
pub fn folder_key(folder: &Path) -> String {
    let folder = super::normalize_folder(folder);
    format!(
        "{:016x}",
        fnv1a(folder.to_string_lossy().to_lowercase().as_bytes())
    )
}

pub fn local_file(data_dir: &Path, folder: &Path) -> PathBuf {
    data_dir
        .join("libraries")
        .join(format!("{}.ini", folder_key(folder)))
}

pub fn read(path: &Path, folder: &Path) -> LocalState {
    read_with_source(path, folder).0
}

/// The state, and the file's text when it could be read, so a writer can skip an unchanged file.
pub fn read_with_source(path: &Path, folder: &Path) -> (LocalState, Option<String>) {
    let source = std::fs::read_to_string(path).ok();
    let state = source
        .as_deref()
        .and_then(|source| LocalState::parse(source, folder))
        .unwrap_or_else(|| LocalState::new(folder.to_path_buf()));
    (state, source)
}

pub fn write(path: &Path, state: &LocalState) -> Result<()> {
    write_text(path, &state.encode())
}

fn write_text(path: &Path, text: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    crate::file::saver::save_atomic(path, text.as_bytes())
}

static NEXT_WRITE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
/// Per local file, the newest write that landed. Writes come from the scan worker and from
/// one-off writer threads, so an older one finishing last must not win.
static LANDED: std::sync::Mutex<Vec<(PathBuf, u64)>> = std::sync::Mutex::new(Vec::new());

/// A number for a write about to be handed off: a later number means newer content.
pub fn next_write() -> u64 {
    NEXT_WRITE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// Writes `text` unless a write numbered after `order` already landed for `path`. Writes to one
/// path are serialized.
pub fn write_text_in_order(path: &Path, text: &str, order: u64) -> Result<()> {
    let mut landed = LANDED.lock().unwrap_or_else(|error| error.into_inner());
    let index = match landed.iter().position(|(p, _)| same_path(p, path)) {
        Some(index) => index,
        None => {
            landed.push((path.to_path_buf(), 0));
            landed.len() - 1
        }
    };
    if landed[index].1 > order {
        return Ok(());
    }
    write_text(path, text)?;
    landed[index].1 = order;
    Ok(())
}

pub fn write_in_order(path: &Path, state: &LocalState, order: u64) -> Result<()> {
    write_text_in_order(path, &state.encode(), order)
}

/// `folders.ini`: recent notebooks (most recent first; the first opens at startup), favorite
/// notebooks, and whether the last session ended with no notebook open. Unknown keys are
/// ignored.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RecentFolders {
    pub folders: Vec<PathBuf>,
    /// In the order they were added; the sidebar sorts them by name.
    pub favorites: Vec<PathBuf>,
    /// `open=none`: the last session closed its notebook, so startup opens none.
    pub closed: bool,
}

impl RecentFolders {
    pub fn parse(source: &str) -> Self {
        let source = source.strip_prefix('\u{feff}').unwrap_or(source);
        let mut version = None;
        let mut parsed = Self::default();
        for line in source.lines() {
            match line.split_once('=') {
                Some(("version", value)) => version = Some(value),
                Some(("open", "none")) => parsed.closed = true,
                Some(("folder", value)) if !value.is_empty() => {
                    parsed.folders.push(PathBuf::from(value));
                }
                Some(("favorite", value)) if !value.is_empty() => {
                    parsed.favorites.push(PathBuf::from(value));
                }
                _ => {}
            }
        }
        if version != Some(VERSION) {
            return Self::default();
        }
        parsed.folders.truncate(FOLDER_LIMIT);
        parsed.favorites.truncate(FAVORITE_LIMIT);
        parsed
    }

    pub fn encode(&self) -> String {
        let mut output = format!("version={VERSION}\r\n");
        if self.closed {
            output.push_str("open=none\r\n");
        }
        for folder in &self.folders {
            output.push_str(&format!("folder={}\r\n", folder.to_string_lossy()));
        }
        for favorite in &self.favorites {
            output.push_str(&format!("favorite={}\r\n", favorite.to_string_lossy()));
        }
        output
    }

    /// A notebook was opened: it moves to the front of the recent list, and the next start
    /// opens it.
    pub fn push(&mut self, folder: PathBuf) {
        let folder = super::normalize_folder(&folder);
        self.folders
            .retain(|existing| !same_path(&super::normalize_folder(existing), &folder));
        self.folders.insert(0, folder);
        self.folders.truncate(FOLDER_LIMIT);
        self.closed = false;
    }

    /// Adds `folder` to the favorites, or removes it when it is one, and says whether it is a
    /// favorite now. At `FAVORITE_LIMIT`, a new favorite is refused and this returns false. The
    /// recent list is left alone.
    pub fn toggle_favorite(&mut self, folder: &Path) -> bool {
        let folder = super::normalize_folder(folder);
        let before = self.favorites.len();
        self.favorites
            .retain(|existing| !same_path(&super::normalize_folder(existing), &folder));
        if self.favorites.len() != before || self.favorites.len() >= FAVORITE_LIMIT {
            return false;
        }
        self.favorites.push(folder);
        true
    }

    pub fn is_favorite(&self, folder: &Path) -> bool {
        let folder = super::normalize_folder(folder);
        self.favorites
            .iter()
            .any(|existing| same_path(&super::normalize_folder(existing), &folder))
    }

    pub fn set_closed(&mut self, closed: bool) {
        self.closed = closed;
    }
}

/// What the sidebar calls each notebook: its folder's name, plus a dim hint only when another
/// entry has the same name (ignoring case). The hint is the parent folder's name, or the parent's
/// whole path when those clash too. Touches no disk.
pub fn display_names(folders: &[PathBuf]) -> Vec<(String, Option<String>)> {
    fn name(folder: &Path) -> String {
        folder.file_name().map_or_else(
            || folder.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        )
    }
    let names: Vec<String> = folders.iter().map(|folder| name(folder)).collect();
    let keys: Vec<String> = names.iter().map(|name| name.to_lowercase()).collect();
    let parent_keys: Vec<Option<String>> = folders
        .iter()
        .map(|folder| folder.parent().map(|parent| name(parent).to_lowercase()))
        .collect();
    folders
        .iter()
        .enumerate()
        .map(|(index, folder)| {
            let clashes: Vec<usize> = (0..folders.len())
                .filter(|&other| other != index && keys[other] == keys[index])
                .collect();
            if clashes.is_empty() {
                return (names[index].clone(), None);
            }
            let parent_clashes = clashes
                .iter()
                .any(|&other| parent_keys[other] == parent_keys[index]);
            let hint = folder.parent().map(|parent| {
                if parent_clashes {
                    parent.display().to_string()
                } else {
                    name(parent)
                }
            });
            (names[index].clone(), hint)
        })
        .collect()
}

pub fn folders_file(data_dir: &Path) -> PathBuf {
    data_dir.join("folders.ini")
}

pub fn read_folders(path: &Path) -> RecentFolders {
    std::fs::read_to_string(path)
        .map(|source| RecentFolders::parse(&source))
        .unwrap_or_default()
}

pub fn write_folders(path: &Path, folders: &RecentFolders) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    crate::file::saver::save_atomic(path, folders.encode().as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> LocalState {
        let mut state = LocalState::new(PathBuf::from(r"D:\Notes"));
        state.autosave = false;
        state.expanded = vec![PathBuf::from("sub"), PathBuf::from(r"sub\a|x")];
        state.missing = vec![(5, NoteId(9))];
        state.files = vec![CachedFile {
            volume: 0x1234,
            file_id: 77,
            mtime: 133_000,
            size: 42,
            path: PathBuf::from(r"sub\a|x\n.md"),
        }];
        state
    }

    #[test]
    fn local_state_round_trips_and_rejects_another_folders_file() {
        // Break caught: two folders whose keys collide sharing one cache, or `|` in a path
        // breaking the scan cache or the expanded folders.
        let text = sample().encode();
        assert!(text.contains("expanded=sub\\a|x\r\n"), "{text:?}");
        assert_eq!(
            LocalState::parse(&text, Path::new(r"D:\Notes")),
            Some(sample())
        );
        assert!(
            LocalState::parse(&text, Path::new(r"d:\notes")).is_some(),
            "case is ignored"
        );
        assert_eq!(LocalState::parse(&text, Path::new(r"D:\Other")), None);
        assert_eq!(
            LocalState::parse("version=2\r\n", Path::new(r"D:\Notes")),
            None
        );
    }

    #[test]
    fn unknown_lines_in_a_local_file_are_ignored() {
        // Break caught: a line FastPad does not know (a key a later build adds, or one a hand
        // edit left) making the whole file unreadable, which throws away the scan cache, the
        // expanded folders and the notebook's autosave switch.
        let text = "version=1\r\nfolder=D:\\Notes\r\nautosave=false\r\nfuture=x|y\r\n\
                    no equals sign\r\nexpanded=sub\r\n\
                    missing=5|00000000000000000000000000000009\r\n";
        let state = LocalState::parse(text, Path::new(r"D:\Notes")).unwrap();
        assert!(!state.autosave);
        assert_eq!(state.expanded, [PathBuf::from("sub")]);
        assert_eq!(state.missing, [(5, NoteId(9))]);
        assert!(!state.encode().contains("future="));
    }

    #[test]
    fn expanded_folders_are_kept_once_ignoring_case_and_collapse_away() {
        // Break caught: expanding a folder twice writing two lines, or collapsing `Sub` leaving
        // `sub` expanded.
        let mut state = LocalState::new(PathBuf::from(r"D:\Notes"));
        state.set_expanded(Path::new(r"Sub\Deep"), true);
        state.set_expanded(Path::new(r"sub\deep"), true);
        assert_eq!(state.expanded, [PathBuf::from(r"Sub\Deep")]);
        assert!(state.is_expanded(Path::new(r"SUB\DEEP")));
        assert!(!state.is_expanded(Path::new("Sub")));
        let before = state.conveniences();
        state.set_expanded(Path::new(r"sub\deep"), true);
        assert_eq!(state.conveniences(), before, "no change, no rewrite");
        state.set_expanded(Path::new(r"SUB\deep"), false);
        assert!(state.expanded.is_empty());
    }

    #[test]
    fn missing_times_are_recorded_once_and_cleared() {
        let mut state = LocalState::new(PathBuf::from(r"D:\Notes"));
        state.set_missing(NoteId(1), 100);
        state.set_missing(NoteId(1), 200);
        assert_eq!(state.missing_since(NoteId(1)), Some(100));
        state.clear_missing(NoteId(1));
        assert_eq!(state.missing_since(NoteId(1)), None);
    }

    #[test]
    fn folder_keys_ignore_case_and_name_the_local_file() {
        let data = Path::new(r"C:\Users\u\AppData\Local\FastPad");
        assert_eq!(
            folder_key(Path::new(r"D:\Notes")),
            folder_key(Path::new(r"d:\NOTES"))
        );
        assert_eq!(folder_key(Path::new(r"D:\Notes")).len(), 16);
        assert_eq!(
            local_file(data, Path::new(r"D:\Notes")),
            data.join("libraries")
                .join(format!("{}.ini", folder_key(Path::new(r"D:\Notes"))))
        );
    }

    #[test]
    fn recent_folders_dedupe_ignoring_case_put_the_newest_first_and_cap_at_ten() {
        let mut folders = RecentFolders::default();
        for index in 0..12 {
            folders.push(PathBuf::from(format!(r"D:\F{index}")));
        }
        folders.push(PathBuf::from(r"d:\f5"));
        assert_eq!(folders.folders.len(), FOLDER_LIMIT);
        assert_eq!(folders.folders[0], PathBuf::from(r"d:\f5"));
        assert_eq!(RecentFolders::parse(&folders.encode()), folders);
        assert_eq!(
            RecentFolders::parse("version=7\r\nfolder=D:\\x\r\n"),
            RecentFolders::default()
        );
    }

    #[test]
    fn folders_ini_round_trips_recent_favorites_and_open_none() {
        // Break caught: favorites or a closed notebook lost on restart, or a file with no
        // favorite= or open= lines failing to load.
        let mut folders = RecentFolders::default();
        folders.push(PathBuf::from(r"D:\Work"));
        assert!(folders.toggle_favorite(Path::new(r"E:\Recipes")));
        folders.set_closed(true);
        let text = folders.encode();
        assert_eq!(
            text,
            "version=1\r\nopen=none\r\nfolder=D:\\Work\r\nfavorite=E:\\Recipes\r\n"
        );
        assert_eq!(RecentFolders::parse(&text), folders);
        let sparse = RecentFolders::parse("version=1\r\nfolder=D:\\Work\r\nfuture=x\r\n");
        assert_eq!(sparse.folders, [PathBuf::from(r"D:\Work")]);
        assert!(sparse.favorites.is_empty() && !sparse.closed);
        assert!(!RecentFolders::parse("version=1\r\nopen=D:\\Work\r\n").closed);
    }

    #[test]
    fn opening_a_notebook_clears_open_none_and_favoriting_leaves_the_recent_list_alone() {
        // Break caught: a favorite star reordering the recent list, or a notebook opened after
        // a close still reading as closed at the next start.
        let mut folders = RecentFolders::default();
        folders.set_closed(true);
        assert!(folders.toggle_favorite(Path::new(r"D:\Notes")));
        assert!(folders.folders.is_empty(), "favoriting is not opening");
        assert!(folders.closed);
        folders.push(PathBuf::from(r"D:\Notes"));
        assert!(!folders.closed);
    }

    #[test]
    fn favorites_toggle_ignoring_case_and_spelling_and_cap_at_fifty() {
        let mut folders = RecentFolders::default();
        assert!(folders.toggle_favorite(Path::new(r"D:\Notes\")));
        assert_eq!(folders.favorites, [PathBuf::from(r"D:\Notes")]);
        assert!(folders.is_favorite(Path::new(r"d:\notes")));
        assert!(
            !folders.toggle_favorite(Path::new(r"d:\NOTES")),
            "a second toggle removes it"
        );
        assert!(folders.favorites.is_empty());
        for index in 0..FAVORITE_LIMIT + 3 {
            folders.toggle_favorite(&PathBuf::from(format!(r"D:\F{index}")));
        }
        assert_eq!(folders.favorites.len(), FAVORITE_LIMIT);
        assert!(
            !folders.is_favorite(Path::new(r"D:\F50")),
            "the 51st is refused"
        );
        let many: String = (0..60).map(|i| format!("favorite=D:\\F{i}\r\n")).collect();
        let parsed = RecentFolders::parse(&format!("version=1\r\n{many}"));
        assert_eq!(parsed.favorites.len(), FAVORITE_LIMIT);
    }

    #[test]
    fn display_names_are_folder_names_with_a_parent_hint_only_on_a_clash() {
        // Break caught: two favorites both reading "Notes" with no way to tell them apart, or
        // every row carrying a path it does not need.
        let folders = [
            r"D:\Work\Notes",
            r"E:\Home\notes",
            r"D:\Recipes",
            r"D:\A\Plans",
            r"E:\A\Plans",
            r"D:\",
        ]
        .map(PathBuf::from);
        assert_eq!(
            display_names(&folders),
            [
                ("Notes".to_owned(), Some("Work".to_owned())),
                ("notes".to_owned(), Some("Home".to_owned())),
                ("Recipes".to_owned(), None),
                ("Plans".to_owned(), Some(r"D:\A".to_owned())),
                ("Plans".to_owned(), Some(r"E:\A".to_owned())),
                (r"D:\".to_owned(), None),
            ]
        );
    }

    #[test]
    fn missing_or_damaged_local_files_read_as_fresh_state() {
        let dir = std::env::temp_dir().join(format!("fastpad-local-io-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let folder = Path::new(r"D:\Notes");
        let path = local_file(&dir, folder);
        assert_eq!(read(&path, folder), LocalState::new(folder.to_path_buf()));
        write(&path, &sample()).unwrap();
        assert_eq!(read(&path, folder), sample());
        std::fs::write(&path, [0xff, 0x00]).unwrap();
        assert_eq!(read(&path, folder), LocalState::new(folder.to_path_buf()));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
