//! The note library: a folder of plain text files seen as notes, plus sparse organizational
//! metadata (notebooks, tags, favorites, pins) kept in `.fastpad\library.ini` and attached to
//! files by path, file ID and content fingerprint. Nothing here touches a window.

pub mod ids;
pub mod local;
pub mod model;
pub mod ops;
pub mod reconcile;
pub mod scan;
pub mod store;
pub mod title;

use crate::Result;
use ids::IdSource;
use local::LocalState;
use model::{Library, LibraryError, NoteRecord, NoteRef, same_path};
use ops::PendingOp;
use std::path::{Component, Path, PathBuf};
use store::{FileStamp, ReadOutcome};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Metadata {
    Ready,
    /// `library.ini` is damaged or from a newer FastPad: organizing is off and it is never written.
    Unreadable,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NoteEntry {
    pub path: PathBuf,
    pub size: u64,
    pub mtime: u64,
}

#[derive(Debug)]
pub struct LibraryState {
    pub folder: PathBuf,
    pub local_path: PathBuf,
    pub library: Library,
    pub metadata: Metadata,
    pub stamp: Option<FileStamp>,
    pub local: LocalState,
    pub notes: Vec<NoteEntry>,
    pub truncated: bool,
    pub pending: Vec<PendingOp>,
    pub relocated: Vec<(PathBuf, PathBuf)>,
}

pub fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

fn component_key(component: Component<'_>) -> String {
    component.as_os_str().to_string_lossy().to_lowercase()
}

fn strip_folder(folder: &Path, path: &Path) -> Option<PathBuf> {
    let mut folder_parts = folder.components();
    let mut path_parts = path.components();
    loop {
        match (folder_parts.next(), path_parts.next()) {
            (None, Some(first)) => {
                let mut rest = PathBuf::from(first.as_os_str());
                rest.extend(path_parts);
                return Some(rest);
            }
            (Some(a), Some(b)) if component_key(a) == component_key(b) => {}
            _ => return None,
        }
    }
}

/// How a record stores `path`: relative inside `folder` (compared ignoring case), else absolute.
pub fn record_path(folder: &Path, path: &Path) -> PathBuf {
    strip_folder(folder, path).unwrap_or_else(|| path.to_path_buf())
}

pub fn is_inside(folder: &Path, path: &Path) -> bool {
    strip_folder(folder, path).is_some()
}

/// Size and write time of a file on disk, to notice edits made outside FastPad.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DiskStamp {
    pub size: u64,
    pub modified: u64,
}

pub fn disk_stamp(path: &Path) -> Option<DiskStamp> {
    store::stamp(path).map(|stamp| DiskStamp {
        size: stamp.size,
        modified: stamp.modified,
    })
}

/// Windows FILETIME ticks (100 ns intervals) between 1601-01-01 and the Unix epoch.
const FILETIME_UNIX_EPOCH_TICKS: u64 = 116_444_736_000_000_000;

/// Converts a Unix-epoch nanosecond timestamp (as `store::stamp` reports) to Windows FILETIME
/// ticks, the unit `scan::ScanEntry::mtime` (and so `NoteEntry::mtime`) is stored in.
fn filetime_ticks(unix_nanos: u64) -> u64 {
    unix_nanos / 100 + FILETIME_UNIX_EPOCH_TICKS
}

/// Reads both library files, scans the folder and reconciles. Runs on the worker thread.
pub fn load(folder: &Path, local_path: &Path, now: u64) -> Result<LibraryState> {
    let library_path = store::library_file(folder);
    let (mut library, metadata, stamp) = match store::read(&library_path) {
        ReadOutcome::Absent => (Library::default(), Metadata::Ready, None),
        ReadOutcome::Loaded(library, stamp) => (library, Metadata::Ready, Some(stamp)),
        ReadOutcome::Unreadable => (
            Library::default(),
            Metadata::Unreadable,
            store::stamp(&library_path),
        ),
    };
    let mut local = local::read(local_path, folder);
    let scan = if folder.is_dir() {
        scan::scan(folder, scan::NOTE_LIMIT)?
    } else {
        scan::Scan::default()
    };
    let reconciled = if metadata == Metadata::Ready {
        reconcile::reconcile(&mut library, &mut local, &scan, now, &mut |relative| {
            ids::hash_file(&folder.join(relative), reconcile::HASH_LIMIT)
        })
    } else {
        reconcile::Reconciled::default()
    };
    Ok(LibraryState {
        folder: folder.to_path_buf(),
        local_path: local_path.to_path_buf(),
        library,
        metadata,
        stamp,
        local,
        notes: scan
            .entries
            .iter()
            .map(|entry| NoteEntry {
                path: entry.path.clone(),
                size: entry.size,
                mtime: entry.mtime,
            })
            .collect(),
        truncated: scan.truncated,
        pending: reconciled.ops,
        relocated: reconciled.relocated,
    })
}

/// Writes pending operations to `library.ini`. If the file changed on disk since it was read,
/// it is re-read and the pending operations are replayed on top first. Returns whether it wrote.
pub fn flush(state: &mut LibraryState) -> Result<bool> {
    if state.metadata == Metadata::Unreadable || state.pending.is_empty() {
        return Ok(false);
    }
    let path = store::library_file(&state.folder);
    if store::stamp(&path) != state.stamp {
        let mut fresh = match store::read(&path) {
            ReadOutcome::Loaded(library, stamp) => {
                state.stamp = Some(stamp);
                library
            }
            ReadOutcome::Absent => {
                state.stamp = None;
                Library::default()
            }
            ReadOutcome::Unreadable => {
                state.metadata = Metadata::Unreadable;
                return Err(crate::FastPadError::Invariant(
                    "library.ini was replaced by a file this FastPad cannot read",
                ));
            }
        };
        ops::replay(&mut fresh, &state.pending);
        state.library = fresh;
    }
    state.library.prune();
    if state.stamp.is_none() && state.library == Library::default() {
        state.pending.clear();
        return Ok(false);
    }
    state.stamp = Some(store::write(&path, &state.library)?);
    state.pending.clear();
    Ok(true)
}

/// Saves the per-PC state. Failures are ignored: it only holds caches and conveniences.
pub fn write_local(state: &LibraryState) {
    let _ = local::write(&state.local_path, &state.local);
}

/// The notes list for a merged state: an entry whose path is in both lists comes from `fresh`;
/// an entry only in one of the two lists (the index changed during the rescan: a save, a rename,
/// a remove) is kept, restamped from disk, only if the file still exists.
fn merge_notes(folder: &Path, previous: Vec<NoteEntry>, fresh: Vec<NoteEntry>) -> Vec<NoteEntry> {
    let key = |note: &NoteEntry| note.path.to_string_lossy().to_lowercase();
    let previous_keys: std::collections::HashSet<String> = previous.iter().map(key).collect();
    let fresh_keys: std::collections::HashSet<String> = fresh.iter().map(key).collect();

    let mut merged = Vec::new();
    let mut differing = Vec::new();
    for note in fresh {
        if previous_keys.contains(&key(&note)) {
            merged.push(note);
        } else {
            differing.push(note);
        }
    }
    for note in previous {
        if !fresh_keys.contains(&key(&note)) {
            differing.push(note);
        }
    }
    for note in differing {
        if let Some(stamp) = store::stamp(&folder.join(&note.path)) {
            merged.push(NoteEntry { path: note.path, size: stamp.size, mtime: filetime_ticks(stamp.modified) });
        }
    }
    merged
}

/// Installs a rescan's result without losing what changed while it ran.
pub fn merge_rescan(previous: LibraryState, fresh: LibraryState) -> LibraryState {
    let LibraryState {
        library: previous_library,
        metadata: previous_metadata,
        stamp: previous_stamp,
        notes: previous_notes,
        pending: previous_pending,
        local: previous_local,
        ..
    } = previous;
    let mut fresh = fresh;

    // A stamp mismatch alone does not say which side is current: the live library may have
    // flushed while the rescan was reading (previous is current and belongs on disk), or the
    // file may have changed outside FastPad while it was inactive, e.g. another PC's sync
    // (the rescan's own read is current). One more stamp, taken now, tells them apart.
    let previous_is_current = previous_metadata == Metadata::Ready
        && fresh.metadata == Metadata::Ready
        && previous_stamp != fresh.stamp
        && store::stamp(&store::library_file(&fresh.folder)) == previous_stamp;

    if previous_is_current {
        let mut library = previous_library;
        ops::replay(&mut library, &fresh.pending);
        fresh.library = library;
        fresh.stamp = previous_stamp;
    } else if fresh.metadata == Metadata::Ready {
        ops::replay(&mut fresh.library, &previous_pending);
    }

    let mut pending = fresh.pending;
    pending.extend(previous_pending);
    fresh.pending = pending;

    fresh.notes = merge_notes(&fresh.folder, previous_notes, fresh.notes);
    if fresh.truncated {
        // A truncated scan's own list is already capped at the limit; previous-only entries
        // that survived the merge must not push it past that.
        fresh.notes.truncate(scan::NOTE_LIMIT);
    }

    fresh.local.merge_recent(&previous_local);
    fresh.local.autosave = previous_local.autosave;
    fresh
}

impl LibraryState {
    pub fn record_for(&self, path: &Path) -> Option<&NoteRecord> {
        self.library.note_by_path(&record_path(&self.folder, path))
    }

    /// The existing record's ID for `path`, or a new ID.
    pub fn note_ref(&self, ids: &mut IdSource, path: &Path) -> NoteRef {
        let stored = record_path(&self.folder, path);
        let id = self
            .library
            .note_by_path(&stored)
            .map_or_else(|| ids::NoteId(ids.next()), |record| record.id);
        NoteRef { id, path: stored }
    }

    /// Applies `op` to the live library and keeps it for the next write.
    pub fn apply(&mut self, op: PendingOp) -> std::result::Result<(), LibraryError> {
        ops::apply(&mut self.library, &op)?;
        self.pending.push(op);
        Ok(())
    }

    /// Adds a file FastPad just saved to the index, if it is a note inside the folder. Updates
    /// the entry in place when the note is already indexed.
    pub fn add_note(&mut self, path: &Path) {
        let Some(relative) = strip_folder(&self.folder, path) else {
            return;
        };
        let is_note = relative
            .extension()
            .is_some_and(|ext| title::is_note_extension(&ext.to_string_lossy()));
        if !is_note {
            return;
        }
        let stamp = store::stamp(path);
        let size = stamp.map_or(0, |stamp| stamp.size);
        let mtime = stamp.map_or(0, |stamp| filetime_ticks(stamp.modified));
        if let Some(existing) = self.notes.iter_mut().find(|note| same_path(&note.path, &relative)) {
            existing.size = size;
            existing.mtime = mtime;
        } else {
            self.notes.push(NoteEntry { path: relative, size, mtime });
        }
    }

    pub fn remove_note(&mut self, path: &Path) {
        let stored = record_path(&self.folder, path);
        self.notes.retain(|note| !same_path(&note.path, &stored));
    }

    /// Follows a rename FastPad made: the index, the recent list and any record.
    pub fn rename_note(&mut self, old: &Path, new: &Path) {
        let old_stored = record_path(&self.folder, old);
        let new_stored = record_path(&self.folder, new);
        self.remove_note(old);
        self.add_note(new);
        self.local.rename_path(&old_stored, &new_stored);
        if let Some(record) = self.library.note_by_path(&old_stored) {
            let note = NoteRef { id: record.id, path: old_stored };
            let _ = self.apply(PendingOp::Relocate { note, path: new_stored });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::ids::{NotebookId, TagId};

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(label: &str) -> Self {
            let root = std::env::temp_dir().join(format!("fastpad-library-{label}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(root.join("notes")).unwrap();
            Self(root)
        }
        fn folder(&self) -> PathBuf {
            self.0.join("notes")
        }
        fn local(&self) -> PathBuf {
            self.0.join("local.ini")
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn record_paths_are_relative_inside_the_folder_ignoring_case_and_absolute_outside() {
        let folder = Path::new(r"D:\Notes");
        assert_eq!(record_path(folder, Path::new(r"d:\notes\sub\a.md")), PathBuf::from(r"sub\a.md"));
        assert_eq!(record_path(folder, Path::new(r"D:\Other\a.md")), PathBuf::from(r"D:\Other\a.md"));
        assert_eq!(record_path(folder, Path::new(r"D:\NotesArchive\a.md")), PathBuf::from(r"D:\NotesArchive\a.md"));
        assert!(is_inside(folder, Path::new(r"D:\NOTES\a.md")));
        assert!(!is_inside(folder, folder));
    }

    #[test]
    fn opening_a_folder_writes_nothing_into_it() {
        // Break caught: opening a repo as a folder creating .fastpad\ before anything is organized.
        let scratch = Scratch::new("readonly-open");
        std::fs::write(scratch.folder().join("a.md"), "a").unwrap();
        let mut state = load(&scratch.folder(), &scratch.local(), 100).unwrap();
        assert_eq!(state.notes.len(), 1);
        assert!(!flush(&mut state).unwrap());
        assert!(!scratch.folder().join(".fastpad").exists());
    }

    #[test]
    fn organizing_creates_the_library_file_and_a_reload_sees_it() {
        let scratch = Scratch::new("organize");
        let note = scratch.folder().join("a.md");
        std::fs::write(&note, "a").unwrap();
        let mut ids = IdSource::new(1, 2);
        let mut state = load(&scratch.folder(), &scratch.local(), 100).unwrap();
        let target = state.note_ref(&mut ids, &note);
        state.apply(PendingOp::SetFavorite { note: target, value: true }).unwrap();
        assert!(flush(&mut state).unwrap());
        assert!(state.pending.is_empty());
        let reloaded = load(&scratch.folder(), &scratch.local(), 101).unwrap();
        assert!(reloaded.record_for(&note).unwrap().favorite);
        assert_eq!(reloaded.record_for(&note).unwrap().path, PathBuf::from("a.md"));
    }

    #[test]
    fn a_file_changed_on_disk_is_merged_not_overwritten() {
        // Break caught: this PC's debounced write replacing the notebook another PC just synced.
        let scratch = Scratch::new("merge");
        let a = scratch.folder().join("a.md");
        std::fs::write(&a, "a").unwrap();
        let mut ids = IdSource::new(1, 2);
        let mut state = load(&scratch.folder(), &scratch.local(), 100).unwrap();
        let target = state.note_ref(&mut ids, &a);
        state.apply(PendingOp::SetFavorite { note: target.clone(), value: true }).unwrap();
        flush(&mut state).unwrap();

        // "Another PC" adds a notebook directly in the file.
        let path = store::library_file(&scratch.folder());
        let mut other = match store::read(&path) {
            store::ReadOutcome::Loaded(library, _) => library,
            _ => panic!("expected a library"),
        };
        other.create_notebook(NotebookId(77), "Synced", 5).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        store::write(&path, &other).unwrap();

        state.apply(PendingOp::AddTag { note: target, tag: TagId(5), name: "idea".into() }).unwrap();
        flush(&mut state).unwrap();
        let final_state = load(&scratch.folder(), &scratch.local(), 102).unwrap();
        assert!(final_state.library.notebook(NotebookId(77)).is_some());
        let record = final_state.record_for(&a).unwrap();
        assert!(record.favorite);
        assert_eq!(record.tags.len(), 1);
    }

    #[test]
    fn an_unreadable_library_file_is_never_overwritten() {
        let scratch = Scratch::new("unreadable");
        let a = scratch.folder().join("a.md");
        std::fs::write(&a, "a").unwrap();
        let path = store::library_file(&scratch.folder());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "version=9\r\nnote=future\r\n").unwrap();
        let mut ids = IdSource::new(1, 2);
        let mut state = load(&scratch.folder(), &scratch.local(), 100).unwrap();
        assert_eq!(state.metadata, Metadata::Unreadable);
        assert_eq!(state.notes.len(), 1, "notes are still listed");
        let target = state.note_ref(&mut ids, &a);
        let _ = state.apply(PendingOp::SetFavorite { note: target, value: true });
        assert!(!flush(&mut state).unwrap());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "version=9\r\nnote=future\r\n");
    }

    #[test]
    fn flush_refuses_to_overwrite_a_library_file_replaced_by_an_unreadable_one() {
        // Break caught: a flush re-reading the file mid-write, finding it replaced by something
        // this FastPad cannot parse, and writing over it anyway.
        let scratch = Scratch::new("flush-unreadable");
        let a = scratch.folder().join("a.md");
        std::fs::write(&a, "a").unwrap();
        let mut ids = IdSource::new(1, 2);
        let mut state = load(&scratch.folder(), &scratch.local(), 100).unwrap();
        let target = state.note_ref(&mut ids, &a);
        state.apply(PendingOp::SetFavorite { note: target, value: true }).unwrap();

        // Another process replaces library.ini with a file from a newer, unreadable version.
        let path = store::library_file(&scratch.folder());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "version=9\r\nnote=future\r\n").unwrap();

        assert!(flush(&mut state).is_err());
        assert_eq!(state.metadata, Metadata::Unreadable);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "version=9\r\nnote=future\r\n");
    }

    #[test]
    fn a_rescan_keeps_changes_made_while_it_ran() {
        let scratch = Scratch::new("rescan");
        let a = scratch.folder().join("a.md");
        std::fs::write(&a, "a").unwrap();
        let mut ids = IdSource::new(1, 2);
        let mut previous = load(&scratch.folder(), &scratch.local(), 100).unwrap();
        let fresh = load(&scratch.folder(), &scratch.local(), 101).unwrap();
        let target = previous.note_ref(&mut ids, &a);
        previous.apply(PendingOp::SetPinned { note: target, value: true }).unwrap();
        previous.local.note_opened(Path::new("a.md"), 150);
        let merged = merge_rescan(previous, fresh);
        assert!(merged.record_for(&a).unwrap().pinned);
        assert_eq!(merged.pending.len(), 1);
        assert_eq!(merged.local.recent[0].0, 150);
    }

    #[test]
    fn a_rescan_does_not_revert_a_favorite_already_flushed_while_it_ran() {
        // Break caught: merge_rescan replaying an empty pending list onto the rescan's own
        // (stale) snapshot of the library and installing the rescan's stamp, silently reverting
        // a change the live library had already flushed to disk before the merge happened.
        let scratch = Scratch::new("rescan-flush");
        let a = scratch.folder().join("a.md");
        std::fs::write(&a, "a").unwrap();
        let mut ids = IdSource::new(1, 2);
        let mut previous = load(&scratch.folder(), &scratch.local(), 100).unwrap();
        let fresh = load(&scratch.folder(), &scratch.local(), 101).unwrap();
        let target = previous.note_ref(&mut ids, &a);
        previous.apply(PendingOp::SetFavorite { note: target, value: true }).unwrap();
        assert!(flush(&mut previous).unwrap());

        let mut merged = merge_rescan(previous, fresh);
        assert!(merged.record_for(&a).unwrap().favorite, "the flushed favorite is still visible");
        assert!(!flush(&mut merged).unwrap(), "nothing pending: the flush is a no-op");
        let reloaded = load(&scratch.folder(), &scratch.local(), 102).unwrap();
        assert!(reloaded.record_for(&a).unwrap().favorite, "the flush did not revert it");
    }

    #[test]
    fn merging_notes_keeps_index_edits_made_during_the_rescan() {
        // Break caught: merge_rescan taking fresh.notes verbatim, losing a note added or a
        // rename made through the live index while a background rescan was still running.
        let scratch = Scratch::new("rescan-notes");
        let a = scratch.folder().join("a.md");
        std::fs::write(&a, "a").unwrap();
        let old = scratch.folder().join("old.md");
        std::fs::write(&old, "x").unwrap();

        let mut previous = load(&scratch.folder(), &scratch.local(), 100).unwrap();
        let fresh = load(&scratch.folder(), &scratch.local(), 101).unwrap();

        // A note saved through FastPad while the rescan was running: only in `previous`'s index.
        let b = scratch.folder().join("b.md");
        std::fs::write(&b, "b").unwrap();
        previous.add_note(&b);

        // A rename made through FastPad: `fresh` still has the old name (the file no longer
        // exists there), `previous` has the new one.
        let renamed = scratch.folder().join("renamed.md");
        std::fs::rename(&old, &renamed).unwrap();
        previous.rename_note(&old, &renamed);

        let merged = merge_rescan(previous, fresh);
        let paths: std::collections::HashSet<_> =
            merged.notes.iter().map(|note| note.path.to_string_lossy().to_lowercase()).collect();
        assert!(paths.contains("a.md"));
        assert!(paths.contains("b.md"), "a note added during the rescan survives");
        assert!(paths.contains("renamed.md"), "a rename made during the rescan survives");
        assert!(!paths.contains("old.md"), "the old name is gone from disk and is not kept from fresh");
        assert_eq!(merged.notes.len(), 3);
    }

    #[test]
    fn a_rescan_absorbs_a_change_made_outside_fastpad_while_it_was_inactive() {
        // Break caught: treating any stamp mismatch as "the live library flushed during the
        // scan" and keeping a stale library forever when the file actually changed from
        // outside (another PC's sync) while this FastPad made no local changes of its own.
        let scratch = Scratch::new("rescan-outside-sync");
        let a = scratch.folder().join("a.md");
        std::fs::write(&a, "a").unwrap();
        let mut ids = IdSource::new(1, 2);

        // Organize once, so `library.ini` exists and `previous` loads with a real stamp.
        let mut setup = load(&scratch.folder(), &scratch.local(), 100).unwrap();
        let target = setup.note_ref(&mut ids, &a);
        setup.apply(PendingOp::SetFavorite { note: target, value: true }).unwrap();
        assert!(flush(&mut setup).unwrap());

        let previous = load(&scratch.folder(), &scratch.local(), 101).unwrap();
        assert!(previous.stamp.is_some());

        // Another PC syncs a change directly into the file while this FastPad is inactive.
        let path = store::library_file(&scratch.folder());
        let mut outside = match store::read(&path) {
            store::ReadOutcome::Loaded(library, _) => library,
            _ => panic!("expected a library"),
        };
        outside.create_notebook(NotebookId(77), "Synced", 5).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        store::write(&path, &outside).unwrap();

        let fresh = load(&scratch.folder(), &scratch.local(), 102).unwrap();
        assert_ne!(fresh.stamp, previous.stamp);
        let current = store::stamp(&path);

        let merged = merge_rescan(previous, fresh);
        assert!(merged.library.notebook(NotebookId(77)).is_some(), "the outside change is visible");
        assert!(merged.record_for(&a).unwrap().favorite, "the earlier favorite is still there");
        assert_eq!(merged.stamp, current);
    }

    #[test]
    fn note_entry_mtimes_are_filetime_ticks_like_a_scan_reports() {
        // Break caught: add_note (and merge_notes) storing mtime as Unix nanoseconds while
        // scan.rs reports Windows FILETIME ticks (100 ns since 1601), so entries from different
        // sources in the same list could not be compared or sorted.
        let scratch = Scratch::new("mtime-scale");
        let a = scratch.folder().join("a.md");
        std::fs::write(&a, "a").unwrap();
        let raw_nanos = disk_stamp(&a).unwrap().modified;
        let expected = raw_nanos / 100 + FILETIME_UNIX_EPOCH_TICKS;

        let mut state = load(&scratch.folder(), &scratch.local(), 100).unwrap();
        let from_scan = state.notes[0].mtime;
        state.notes.clear();
        state.add_note(&a);
        let from_add_note = state.notes[0].mtime;

        assert_eq!(from_add_note, expected);
        // Both are FILETIME ticks for the same write; allow a little slack for rounding between
        // the OS-reported FILETIME and the nanosecond-precision `SystemTime` conversion.
        let ticks_per_second = 10_000_000;
        assert!(
            from_scan.abs_diff(from_add_note) < ticks_per_second,
            "scan: {from_scan}, add_note: {from_add_note}"
        );
    }

    #[test]
    fn a_truncated_rescan_caps_the_merged_notes_list_at_the_scan_limit() {
        // Break caught: previous-only entries surviving the notes merge and pushing a truncated
        // scan's list past the limit it was supposed to be capped at.
        let scratch = Scratch::new("rescan-cap");
        let folder = scratch.folder();
        let entries = |count: usize| -> Vec<NoteEntry> {
            (0..count)
                .map(|index| NoteEntry { path: PathBuf::from(format!("f{index}.md")), size: 0, mtime: 0 })
                .collect()
        };
        let state = |notes: Vec<NoteEntry>, truncated: bool| LibraryState {
            folder: folder.clone(),
            local_path: scratch.local(),
            library: Library::default(),
            metadata: Metadata::Ready,
            stamp: None,
            local: LocalState::new(folder.clone()),
            notes,
            truncated,
            pending: Vec::new(),
            relocated: Vec::new(),
        };
        // Identical lists: every entry is common to both, so none needs to exist on disk.
        let previous = state(entries(scan::NOTE_LIMIT + 5), false);
        let fresh = state(entries(scan::NOTE_LIMIT + 5), true);
        let merged = merge_rescan(previous, fresh);
        assert_eq!(merged.notes.len(), scan::NOTE_LIMIT);
    }

    #[test]
    fn a_missing_folder_loads_as_an_empty_library() {
        let scratch = Scratch::new("missing-folder");
        let state = load(&scratch.0.join("not-yet"), &scratch.local(), 100).unwrap();
        assert!(state.notes.is_empty());
        assert_eq!(state.metadata, Metadata::Ready);
    }

    #[test]
    fn the_index_follows_saves_renames_and_deletes() {
        let scratch = Scratch::new("index");
        let mut state = load(&scratch.folder(), &scratch.local(), 100).unwrap();
        let a = scratch.folder().join("a.md");
        std::fs::write(&a, "a").unwrap();
        state.add_note(&a);
        state.add_note(&a);
        state.add_note(Path::new(r"C:\elsewhere\x.md"));
        assert_eq!(state.notes.len(), 1);
        let mut ids = IdSource::new(1, 2);
        let target = state.note_ref(&mut ids, &a);
        state.apply(PendingOp::SetFavorite { note: target, value: true }).unwrap();
        let b = scratch.folder().join("b.md");
        state.rename_note(&a, &b);
        assert_eq!(state.notes[0].path, PathBuf::from("b.md"));
        assert_eq!(state.record_for(&b).unwrap().path, PathBuf::from("b.md"));
        state.remove_note(&b);
        assert!(state.notes.is_empty());
    }

    #[test]
    fn disk_stamps_change_when_a_file_is_rewritten() {
        let scratch = Scratch::new("stamp");
        let a = scratch.folder().join("a.md");
        std::fs::write(&a, "a").unwrap();
        let first = disk_stamp(&a).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&a, "ab").unwrap();
        assert_ne!(disk_stamp(&a), Some(first));
        assert_eq!(disk_stamp(&scratch.folder().join("none.md")), None);
    }
}
