//! Per-PC state that must not travel with the folder: the scan cache (file IDs only mean something
//! on one volume), recently opened notes, when records went missing, the folder's autosave switch,
//! and the list of recent folders. Losing any of it is harmless, so reads never fail.

use super::ids::{NoteId, fnv1a};
use super::model::same_path;
use crate::Result;
use std::path::{Path, PathBuf};

const VERSION: &str = "1";
pub const RECENT_LIMIT: usize = 200;
pub const FOLDER_LIMIT: usize = 10;

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
    /// Most recent first: (unix time opened, path as stored in records).
    pub recent: Vec<(u64, PathBuf)>,
    pub missing: Vec<(u64, NoteId)>,
    pub files: Vec<CachedFile>,
}

impl LocalState {
    pub fn new(folder: PathBuf) -> Self {
        Self {
            folder,
            autosave: true,
            recent: Vec::new(),
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
        for (time, path) in &self.recent {
            output.push_str(&format!("recent={time}|{}\r\n", path.to_string_lossy()));
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

    /// `None` for anything but a version-1 file written for `folder`.
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
                "recent" => {
                    if let Some((time, path)) = value.split_once('|')
                        && let Ok(time) = time.parse()
                        && !path.is_empty()
                    {
                        state.recent.push((time, PathBuf::from(path)));
                    }
                }
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
        if version != Some(VERSION) || !stored_folder.is_some_and(|f| same_path(&f, folder)) {
            return None;
        }
        state.folder = folder.to_path_buf();
        Some(state)
    }

    pub fn note_opened(&mut self, path: &Path, now: u64) {
        self.recent.retain(|(_, existing)| !same_path(existing, path));
        self.recent.insert(0, (now, path.to_path_buf()));
        self.recent.truncate(RECENT_LIMIT);
    }

    pub fn rename_path(&mut self, old: &Path, new: &Path) {
        for (_, path) in &mut self.recent {
            if same_path(path, old) {
                *path = new.to_path_buf();
            }
        }
    }

    /// Keeps the newest open time per path from both lists, most recent first.
    pub fn merge_recent(&mut self, other: &LocalState) {
        for (time, path) in &other.recent {
            match self.recent.iter_mut().find(|(_, existing)| same_path(existing, path)) {
                Some(entry) if entry.0 < *time => entry.0 = *time,
                Some(_) => {}
                None => self.recent.push((*time, path.clone())),
            }
        }
        self.recent.sort_by_key(|entry| std::cmp::Reverse(entry.0));
        self.recent.truncate(RECENT_LIMIT);
    }

    pub fn missing_since(&self, id: NoteId) -> Option<u64> {
        self.missing.iter().find(|(_, missing)| *missing == id).map(|(time, _)| *time)
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
    Some(CachedFile { volume, file_id, mtime, size, path: PathBuf::from(path) })
}

pub fn folder_key(folder: &Path) -> String {
    format!("{:016x}", fnv1a(folder.to_string_lossy().to_lowercase().as_bytes()))
}

pub fn local_file(data_dir: &Path, folder: &Path) -> PathBuf {
    data_dir.join("libraries").join(format!("{}.ini", folder_key(folder)))
}

pub fn read(path: &Path, folder: &Path) -> LocalState {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|source| LocalState::parse(&source, folder))
        .unwrap_or_else(|| LocalState::new(folder.to_path_buf()))
}

pub fn write(path: &Path, state: &LocalState) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    crate::file::saver::save_atomic(path, state.encode().as_bytes())
}

/// `folders.ini`: recent folders, most recent first. The first one opens at startup.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RecentFolders {
    pub folders: Vec<PathBuf>,
}

impl RecentFolders {
    pub fn parse(source: &str) -> Self {
        let source = source.strip_prefix('\u{feff}').unwrap_or(source);
        let mut version = None;
        let mut folders = Vec::new();
        for line in source.lines() {
            match line.split_once('=') {
                Some(("version", value)) => version = Some(value),
                Some(("folder", value)) if !value.is_empty() => folders.push(PathBuf::from(value)),
                _ => {}
            }
        }
        if version != Some(VERSION) {
            return Self::default();
        }
        folders.truncate(FOLDER_LIMIT);
        Self { folders }
    }

    pub fn encode(&self) -> String {
        let mut output = format!("version={VERSION}\r\n");
        for folder in &self.folders {
            output.push_str(&format!("folder={}\r\n", folder.to_string_lossy()));
        }
        output
    }

    pub fn push(&mut self, folder: PathBuf) {
        self.folders.retain(|existing| !same_path(existing, &folder));
        self.folders.insert(0, folder);
        self.folders.truncate(FOLDER_LIMIT);
    }
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
        state.recent = vec![(20, PathBuf::from("b.md")), (10, PathBuf::from(r"sub\a|x.md"))];
        state.missing = vec![(5, NoteId(9))];
        state.files = vec![CachedFile {
            volume: 0x1234,
            file_id: 77,
            mtime: 133_000,
            size: 42,
            path: PathBuf::from(r"sub\a|x.md"),
        }];
        state
    }

    #[test]
    fn local_state_round_trips_and_rejects_another_folders_file() {
        // Break caught: two folders whose keys collide sharing one cache, or `|` in a path
        // breaking the scan cache.
        let text = sample().encode();
        assert_eq!(LocalState::parse(&text, Path::new(r"D:\Notes")), Some(sample()));
        assert!(LocalState::parse(&text, Path::new(r"d:\notes")).is_some(), "case is ignored");
        assert_eq!(LocalState::parse(&text, Path::new(r"D:\Other")), None);
        assert_eq!(LocalState::parse("version=2\r\n", Path::new(r"D:\Notes")), None);
    }

    #[test]
    fn opening_a_note_moves_it_to_the_front_and_caps_the_list() {
        let mut state = LocalState::new(PathBuf::from(r"D:\Notes"));
        for index in 0..RECENT_LIMIT + 5 {
            state.note_opened(Path::new(&format!("{index}.md")), index as u64);
        }
        assert_eq!(state.recent.len(), RECENT_LIMIT);
        state.note_opened(Path::new("10.MD"), 999);
        assert_eq!(state.recent[0], (999, PathBuf::from("10.MD")));
        assert_eq!(state.recent.iter().filter(|(_, p)| same_path(p, Path::new("10.md"))).count(), 1);
    }

    #[test]
    fn renames_follow_recent_entries_and_merging_keeps_the_newest_open_time() {
        let mut state = sample();
        state.rename_path(Path::new(r"SUB\A|X.md"), Path::new("moved.md"));
        assert!(state.recent.iter().any(|(_, p)| p == Path::new("moved.md")));
        let mut other = LocalState::new(PathBuf::from(r"D:\Notes"));
        other.recent = vec![(50, PathBuf::from("b.md")), (1, PathBuf::from("c.md"))];
        state.merge_recent(&other);
        assert_eq!(state.recent[0], (50, PathBuf::from("b.md")));
        assert!(state.recent.iter().any(|(t, p)| *t == 1 && p == Path::new("c.md")));
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
        assert_eq!(folder_key(Path::new(r"D:\Notes")), folder_key(Path::new(r"d:\NOTES")));
        assert_eq!(folder_key(Path::new(r"D:\Notes")).len(), 16);
        assert_eq!(
            local_file(data, Path::new(r"D:\Notes")),
            data.join("libraries").join(format!("{}.ini", folder_key(Path::new(r"D:\Notes"))))
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
        assert_eq!(RecentFolders::parse("version=7\r\nfolder=D:\\x\r\n"), RecentFolders::default());
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
