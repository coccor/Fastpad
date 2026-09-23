//! The note library: a notebook (a folder) of plain text files seen as notes, plus sparse pins
//! kept in `.fastpad\library.ini` and attached to files by path, file ID and content fingerprint.
//! Nothing here touches a window.

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
    /// `library.ini` could not be read this time (for example OneDrive held it open). Organizing
    /// waits for the next rescan, which reads it again; nothing is written meanwhile.
    Busy,
}

/// What `flush` did.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Flushed {
    Wrote,
    /// Nothing to write, or nothing may be written (the metadata is not ready).
    Nothing,
    /// `library.ini` changed on disk and could not be re-read right now. The pending operations
    /// are kept; try again later.
    Busy,
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
    /// Paths (as records store them) FastPad itself added, removed or renamed in `notes` since
    /// the running rescan started. Only these are re-checked when its result is merged.
    pub touched: Vec<PathBuf>,
    /// The recent list, autosave switch and missing times as the local file last written holds
    /// them, so the UI thread rewrites that file only when one of them changed.
    pub written_local: local::Conveniences,
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

/// One spelling per folder: absolute, without a trailing separator or `.` components, so
/// `D:\Notes\` and `D:\Notes` key the same local state and recent-list entry. Touches no disk.
pub fn normalize_folder(folder: &Path) -> PathBuf {
    std::path::absolute(folder)
        .unwrap_or_else(|_| folder.to_path_buf())
        .components()
        .collect()
}

#[cfg(test)]
thread_local! {
    static FOLDER_CHECKS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// How many folder existence checks this thread has made, so tests can prove the UI thread
/// makes none at startup.
#[cfg(test)]
pub fn folder_checks() -> usize {
    FOLDER_CHECKS.with(std::cell::Cell::get)
}

/// Whether `folder` is an existing directory. It can block for a long time on an offline network
/// drive, so it belongs on a worker thread.
pub fn folder_exists(folder: &Path) -> bool {
    #[cfg(test)]
    FOLDER_CHECKS.with(|checks| checks.set(checks.get() + 1));
    folder.is_dir()
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
        ReadOutcome::Busy => (Library::default(), Metadata::Busy, None),
    };
    let (mut local, local_source) = local::read_with_source(local_path, folder);
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
    // Written here, on the worker, so the UI thread never encodes the scan cache.
    let encoded = local.encode();
    if local_source.as_deref() != Some(encoded.as_str()) {
        let _ = local::write_text_in_order(local_path, &encoded, local::next_write());
    }
    Ok(LibraryState {
        folder: folder.to_path_buf(),
        local_path: local_path.to_path_buf(),
        library,
        metadata,
        stamp,
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
        touched: Vec::new(),
        written_local: local.conveniences(),
        local,
    })
}

/// Writes pending operations to `library.ini`. If the file changed on disk since it was read,
/// it is re-read and the pending operations are replayed on top first.
pub fn flush(state: &mut LibraryState) -> Result<Flushed> {
    if state.metadata != Metadata::Ready || state.pending.is_empty() {
        return Ok(Flushed::Nothing);
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
            // Keeps the pending operations and the library as they are.
            ReadOutcome::Busy => return Ok(Flushed::Busy),
        };
        ops::replay(&mut fresh, &state.pending);
        state.library = fresh;
    }
    state.library.prune();
    if state.stamp.is_none() && state.library == Library::default() {
        state.pending.clear();
        return Ok(Flushed::Nothing);
    }
    state.stamp = Some(store::write(&path, &state.library)?);
    state.pending.clear();
    Ok(Flushed::Wrote)
}

/// The per-PC state to write, when its recent list, autosave switch or missing times changed
/// since the local file was last written; it is then counted as written. The scan cache alone
/// never needs a write here: the worker wrote it with the scan.
pub fn take_local_changes(state: &mut LibraryState) -> Option<LocalState> {
    let current = state.local.conveniences();
    if current == state.written_local {
        return None;
    }
    state.written_local = current;
    Some(state.local.clone())
}

/// The notes list for a merged state: the rescan's own list, with every path FastPad touched
/// while it ran (a save, a rename, a remove) re-checked on disk. Entries only in the rescan are
/// already proven by it, and entries only in the old list that FastPad did not touch are gone,
/// so a bulk change made outside FastPad costs no extra stat at all.
fn merge_notes(folder: &Path, fresh: Vec<NoteEntry>, touched: &[PathBuf]) -> Vec<NoteEntry> {
    let mut merged = fresh;
    let mut seen = std::collections::HashSet::new();
    for path in touched {
        if !seen.insert(path.to_string_lossy().to_lowercase()) {
            continue;
        }
        merged.retain(|note| !same_path(&note.path, path));
        let is_note = path
            .extension()
            .is_some_and(|ext| title::is_note_extension(&ext.to_string_lossy()));
        if is_note && let Some(stamp) = store::stamp(&folder.join(path)) {
            merged.push(NoteEntry {
                path: path.clone(),
                size: stamp.size,
                mtime: filetime_ticks(stamp.modified),
            });
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
        pending: previous_pending,
        local: previous_local,
        touched,
        ..
    } = previous;
    let mut fresh = fresh;
    fresh.notes = merge_notes(&fresh.folder, std::mem::take(&mut fresh.notes), &touched);
    if fresh.truncated {
        // A truncated scan's own list is already capped at the limit; touched entries that
        // survived the merge must not push it past that.
        fresh.notes.truncate(scan::NOTE_LIMIT);
    }
    fresh.local.merge_recent(&previous_local);
    fresh.local.autosave = previous_local.autosave;

    if fresh.metadata == Metadata::Busy {
        // The rescan could not read library.ini: what the live state knows still stands, and the
        // next rescan reads it again.
        fresh.library = previous_library;
        fresh.metadata = previous_metadata;
        fresh.stamp = previous_stamp;
        fresh.pending = previous_pending;
        return fresh;
    }

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
    fresh
}

impl LibraryState {
    pub fn record_for(&self, path: &Path) -> Option<&NoteRecord> {
        self.library.note_by_path(&record_path(&self.folder, path))
    }

    /// Whether the note at `path` (absolute, or as records store it) is pinned. A note FastPad
    /// sent to the Recycle Bin is not.
    pub fn is_pinned(&self, path: &Path) -> bool {
        self.record_for(path)
            .is_some_and(|record| record.pinned && !record.deleted)
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
        self.touched.push(relative.clone());
        let stamp = store::stamp(path);
        let size = stamp.map_or(0, |stamp| stamp.size);
        let mtime = stamp.map_or(0, |stamp| filetime_ticks(stamp.modified));
        if let Some(existing) = self
            .notes
            .iter_mut()
            .find(|note| same_path(&note.path, &relative))
        {
            existing.size = size;
            existing.mtime = mtime;
        } else {
            self.notes.push(NoteEntry {
                path: relative,
                size,
                mtime,
            });
        }
    }

    pub fn remove_note(&mut self, path: &Path) {
        let Some(stored) = strip_folder(&self.folder, path) else {
            return;
        };
        self.notes.retain(|note| !same_path(&note.path, &stored));
        self.touched.push(stored);
    }

    /// Follows a rename FastPad made: the index, the recent list and any record.
    pub fn rename_note(&mut self, old: &Path, new: &Path) {
        let old_stored = record_path(&self.folder, old);
        let new_stored = record_path(&self.folder, new);
        self.remove_note(old);
        self.add_note(new);
        self.local.rename_path(&old_stored, &new_stored);
        if let Some(record) = self.library.note_by_path(&old_stored) {
            let note = NoteRef {
                id: record.id,
                path: old_stored,
            };
            let _ = self.apply(PendingOp::Relocate {
                note,
                path: new_stored,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::ids::NoteId;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(label: &str) -> Self {
            let root = std::env::temp_dir()
                .join(format!("fastpad-library-{label}-{}", std::process::id()));
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
        assert_eq!(
            record_path(folder, Path::new(r"d:\notes\sub\a.md")),
            PathBuf::from(r"sub\a.md")
        );
        assert_eq!(
            record_path(folder, Path::new(r"D:\Other\a.md")),
            PathBuf::from(r"D:\Other\a.md")
        );
        assert_eq!(
            record_path(folder, Path::new(r"D:\NotesArchive\a.md")),
            PathBuf::from(r"D:\NotesArchive\a.md")
        );
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
        assert_eq!(flush(&mut state).unwrap(), Flushed::Nothing);
        assert!(!scratch.folder().join(".fastpad").exists());
    }

    #[test]
    fn pinning_creates_the_library_file_and_a_reload_sees_it() {
        let scratch = Scratch::new("organize");
        let note = scratch.folder().join("a.md");
        std::fs::write(&note, "a").unwrap();
        let mut ids = IdSource::new(1, 2);
        let mut state = load(&scratch.folder(), &scratch.local(), 100).unwrap();
        let target = state.note_ref(&mut ids, &note);
        state
            .apply(PendingOp::SetPinned {
                note: target,
                value: true,
            })
            .unwrap();
        assert!(state.is_pinned(&note));
        assert_eq!(flush(&mut state).unwrap(), Flushed::Wrote);
        assert!(state.pending.is_empty());
        let reloaded = load(&scratch.folder(), &scratch.local(), 101).unwrap();
        assert!(reloaded.is_pinned(&note));
        assert_eq!(
            reloaded.record_for(&note).unwrap().path,
            PathBuf::from("a.md")
        );
    }

    #[test]
    fn a_file_changed_on_disk_is_merged_not_overwritten() {
        // Break caught: this PC's debounced write replacing the pin another PC just synced.
        let scratch = Scratch::new("merge");
        let [a, b, c] = ["a.md", "b.md", "c.md"].map(|name| {
            let path = scratch.folder().join(name);
            std::fs::write(&path, name).unwrap();
            path
        });
        let mut ids = IdSource::new(1, 2);
        let mut state = load(&scratch.folder(), &scratch.local(), 100).unwrap();
        let target = state.note_ref(&mut ids, &a);
        state
            .apply(PendingOp::SetPinned {
                note: target,
                value: true,
            })
            .unwrap();
        flush(&mut state).unwrap();

        // "Another PC" pins b.md directly in the file.
        let path = store::library_file(&scratch.folder());
        let mut other = match store::read(&path) {
            store::ReadOutcome::Loaded(library, _) => library,
            _ => panic!("expected a library"),
        };
        other
            .resolve_note(&NoteRef {
                id: NoteId(77),
                path: "b.md".into(),
            })
            .pinned = true;
        std::thread::sleep(std::time::Duration::from_millis(20));
        store::write(&path, &other).unwrap();

        let target = state.note_ref(&mut ids, &c);
        state
            .apply(PendingOp::SetPinned {
                note: target,
                value: true,
            })
            .unwrap();
        flush(&mut state).unwrap();
        let final_state = load(&scratch.folder(), &scratch.local(), 102).unwrap();
        assert!(final_state.is_pinned(&a));
        assert!(final_state.is_pinned(&b), "the synced pin survives");
        assert!(final_state.is_pinned(&c));
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
        let _ = state.apply(PendingOp::SetPinned {
            note: target,
            value: true,
        });
        assert_eq!(flush(&mut state).unwrap(), Flushed::Nothing);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "version=9\r\nnote=future\r\n"
        );
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
        state
            .apply(PendingOp::SetPinned {
                note: target,
                value: true,
            })
            .unwrap();

        // Another process replaces library.ini with a file from a newer, unreadable version.
        let path = store::library_file(&scratch.folder());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "version=9\r\nnote=future\r\n").unwrap();

        assert!(flush(&mut state).is_err());
        assert_eq!(state.metadata, Metadata::Unreadable);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "version=9\r\nnote=future\r\n"
        );
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
        previous
            .apply(PendingOp::SetPinned {
                note: target,
                value: true,
            })
            .unwrap();
        previous.local.note_opened(Path::new("a.md"), 150);
        let merged = merge_rescan(previous, fresh);
        assert!(merged.record_for(&a).unwrap().pinned);
        assert_eq!(merged.pending.len(), 1);
        assert_eq!(merged.local.recent[0].0, 150);
    }

    #[test]
    fn a_rescan_does_not_revert_a_pin_already_flushed_while_it_ran() {
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
        previous
            .apply(PendingOp::SetPinned {
                note: target,
                value: true,
            })
            .unwrap();
        assert_eq!(flush(&mut previous).unwrap(), Flushed::Wrote);

        let mut merged = merge_rescan(previous, fresh);
        assert!(merged.is_pinned(&a), "the flushed pin is still visible");
        assert!(
            flush(&mut merged).unwrap() == Flushed::Nothing,
            "nothing pending: the flush is a no-op"
        );
        let reloaded = load(&scratch.folder(), &scratch.local(), 102).unwrap();
        assert!(reloaded.is_pinned(&a), "the flush did not revert it");
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
        let paths: std::collections::HashSet<_> = merged
            .notes
            .iter()
            .map(|note| note.path.to_string_lossy().to_lowercase())
            .collect();
        assert!(paths.contains("a.md"));
        assert!(
            paths.contains("b.md"),
            "a note added during the rescan survives"
        );
        assert!(
            paths.contains("renamed.md"),
            "a rename made during the rescan survives"
        );
        assert!(
            !paths.contains("old.md"),
            "the old name is gone from disk and is not kept from fresh"
        );
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
        let b = scratch.folder().join("b.md");
        std::fs::write(&b, "b").unwrap();
        let mut ids = IdSource::new(1, 2);

        // Pin once, so `library.ini` exists and `previous` loads with a real stamp.
        let mut setup = load(&scratch.folder(), &scratch.local(), 100).unwrap();
        let target = setup.note_ref(&mut ids, &a);
        setup
            .apply(PendingOp::SetPinned {
                note: target,
                value: true,
            })
            .unwrap();
        assert_eq!(flush(&mut setup).unwrap(), Flushed::Wrote);

        let previous = load(&scratch.folder(), &scratch.local(), 101).unwrap();
        assert!(previous.stamp.is_some());

        // Another PC syncs a pin directly into the file while this FastPad is inactive.
        let path = store::library_file(&scratch.folder());
        let mut outside = match store::read(&path) {
            store::ReadOutcome::Loaded(library, _) => library,
            _ => panic!("expected a library"),
        };
        outside
            .resolve_note(&NoteRef {
                id: NoteId(77),
                path: "b.md".into(),
            })
            .pinned = true;
        std::thread::sleep(std::time::Duration::from_millis(20));
        store::write(&path, &outside).unwrap();

        let fresh = load(&scratch.folder(), &scratch.local(), 102).unwrap();
        assert_ne!(fresh.stamp, previous.stamp);
        let current = store::stamp(&path);

        let merged = merge_rescan(previous, fresh);
        assert!(merged.is_pinned(&b), "the outside change is visible");
        assert!(merged.is_pinned(&a), "the earlier pin is still there");
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
        // Break caught: notes saved during the rescan surviving the notes merge and pushing a
        // truncated scan's list past the limit it was supposed to be capped at.
        let scratch = Scratch::new("rescan-cap");
        let mut previous = bare_state(&scratch, entries(0), false);
        for index in 0..5 {
            let saved = scratch.folder().join(format!("saved{index}.md"));
            std::fs::write(&saved, "x").unwrap();
            previous.add_note(&saved);
        }
        let fresh = bare_state(&scratch, entries(scan::NOTE_LIMIT), true);
        let merged = merge_rescan(previous, fresh);
        assert_eq!(merged.notes.len(), scan::NOTE_LIMIT);
    }

    /// `count` index entries that exist only in memory.
    fn entries(count: usize) -> Vec<NoteEntry> {
        (0..count)
            .map(|index| NoteEntry {
                path: PathBuf::from(format!("f{index}.md")),
                size: 0,
                mtime: 0,
            })
            .collect()
    }

    fn bare_state(scratch: &Scratch, notes: Vec<NoteEntry>, truncated: bool) -> LibraryState {
        LibraryState {
            folder: scratch.folder(),
            local_path: scratch.local(),
            library: Library::default(),
            metadata: Metadata::Ready,
            stamp: None,
            local: LocalState::new(scratch.folder()),
            notes,
            truncated,
            pending: Vec::new(),
            relocated: Vec::new(),
            touched: Vec::new(),
            written_local: local::Conveniences::default(),
        }
    }

    #[test]
    fn a_bulk_change_made_outside_fastpad_costs_no_stat_when_a_rescan_is_merged() {
        // Break caught: the merge stat-ing every path that differs between the old index and the
        // rescan, on the UI thread, so deleting a few thousand notes in Explorer froze FastPad.
        let scratch = Scratch::new("rescan-bulk");
        let previous = bare_state(&scratch, entries(2_000), false);
        let fresh = bare_state(&scratch, entries(0), false);
        let before = store::stats_taken();
        let merged = merge_rescan(previous, fresh);
        assert_eq!(store::stats_taken() - before, 0);
        assert!(merged.notes.is_empty(), "the rescan proved them gone");
    }

    #[test]
    fn a_rescan_that_cannot_read_the_library_file_keeps_the_live_library_and_is_not_unreadable() {
        // Break caught: a sharing violation while OneDrive synced library.ini turning organizing
        // off, or a rescan that met one replacing the live pins with an empty library.
        use std::os::windows::fs::OpenOptionsExt;
        let scratch = Scratch::new("busy-load");
        let a = scratch.folder().join("a.md");
        std::fs::write(&a, "a").unwrap();
        let mut ids = IdSource::new(1, 2);
        let mut previous = load(&scratch.folder(), &scratch.local(), 100).unwrap();
        let target = previous.note_ref(&mut ids, &a);
        previous
            .apply(PendingOp::SetPinned {
                note: target,
                value: true,
            })
            .unwrap();
        assert_eq!(flush(&mut previous).unwrap(), Flushed::Wrote);

        let lock = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(store::library_file(&scratch.folder()))
            .unwrap();
        let fresh = load(&scratch.folder(), &scratch.local(), 101).unwrap();
        drop(lock);
        assert_eq!(fresh.metadata, Metadata::Busy);
        let merged = merge_rescan(previous, fresh);
        assert_eq!(merged.metadata, Metadata::Ready);
        assert!(merged.is_pinned(&a));
    }

    #[test]
    fn a_flush_that_meets_a_busy_library_file_keeps_its_operations_for_a_retry() {
        // Break caught: a flush that could not re-read a synced library.ini dropping the pending
        // operations or marking the library unreadable.
        use std::os::windows::fs::OpenOptionsExt;
        let scratch = Scratch::new("busy-flush");
        let a = scratch.folder().join("a.md");
        std::fs::write(&a, "a").unwrap();
        let b = scratch.folder().join("b.md");
        std::fs::write(&b, "b").unwrap();
        let mut ids = IdSource::new(1, 2);
        let mut state = load(&scratch.folder(), &scratch.local(), 100).unwrap();
        let target = state.note_ref(&mut ids, &a);
        state
            .apply(PendingOp::SetPinned {
                note: target,
                value: true,
            })
            .unwrap();
        // Another PC's sync creates the file, so the flush must re-read it, while it is held open.
        let path = store::library_file(&scratch.folder());
        let mut other = Library::default();
        other
            .resolve_note(&NoteRef {
                id: NoteId(77),
                path: "b.md".into(),
            })
            .pinned = true;
        store::write(&path, &other).unwrap();
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&path)
            .unwrap();
        let busy = flush(&mut state).unwrap();
        drop(lock);
        assert_eq!(busy, Flushed::Busy);
        assert_eq!(state.pending.len(), 1);
        assert_eq!(state.metadata, Metadata::Ready);

        assert_eq!(flush(&mut state).unwrap(), Flushed::Wrote);
        let reloaded = load(&scratch.folder(), &scratch.local(), 101).unwrap();
        assert!(reloaded.is_pinned(&b));
        assert!(reloaded.is_pinned(&a));
    }

    #[test]
    fn the_load_writes_the_local_file_and_only_convenience_changes_need_another_write() {
        // Break caught: the UI thread encoding and writing the whole scan cache on every rescan
        // and every metadata flush, or the worker rewriting an unchanged local file.
        let scratch = Scratch::new("local-writes");
        std::fs::write(scratch.folder().join("a.md"), "a").unwrap();
        let mut state = load(&scratch.folder(), &scratch.local(), 100).unwrap();
        let written = std::fs::read_to_string(scratch.local()).unwrap();
        assert!(written.contains("|a.md\r\n"), "{written:?}");
        let modified = std::fs::metadata(scratch.local())
            .unwrap()
            .modified()
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        let _unchanged = load(&scratch.folder(), &scratch.local(), 101).unwrap();
        assert_eq!(
            std::fs::metadata(scratch.local())
                .unwrap()
                .modified()
                .unwrap(),
            modified,
            "an unchanged local file is not rewritten"
        );

        assert!(take_local_changes(&mut state).is_none());
        state.local.files.clear();
        assert!(
            take_local_changes(&mut state).is_none(),
            "the scan cache alone"
        );
        state.local.note_opened(Path::new("a.md"), 150);
        let changed = take_local_changes(&mut state).expect("a recent change is written");
        assert_eq!(changed.recent[0].0, 150);
        assert!(take_local_changes(&mut state).is_none());
    }

    #[test]
    fn folder_spellings_normalize_to_one() {
        // Break caught: `D:\Notes\` and `D:\Notes` keying two local states and two recent rows.
        assert_eq!(
            normalize_folder(Path::new(r"D:\Notes\")),
            PathBuf::from(r"D:\Notes")
        );
        assert_eq!(
            normalize_folder(Path::new(r"D:\Notes\.\")),
            PathBuf::from(r"D:\Notes")
        );
        assert_eq!(normalize_folder(Path::new(r"D:\")), PathBuf::from(r"D:\"));
        assert_eq!(
            local::folder_key(Path::new(r"D:\Notes\")),
            local::folder_key(Path::new(r"d:\notes"))
        );
        let mut recent = local::RecentFolders::default();
        recent.push(PathBuf::from(r"D:\Notes\"));
        recent.push(PathBuf::from(r"D:\Notes"));
        assert_eq!(recent.folders, [PathBuf::from(r"D:\Notes")]);
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
        state
            .apply(PendingOp::SetPinned {
                note: target,
                value: true,
            })
            .unwrap();
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

    #[test]
    fn a_version_one_library_loads_its_pins_and_is_rewritten_only_when_something_changes() {
        // Break caught: a PR #9 notebook losing its pins on the first start of this build, or
        // opening it rewriting library.ini before the user changed anything.
        let scratch = Scratch::new("v1-migrate");
        let a = scratch.folder().join("a.md");
        std::fs::write(&a, "a").unwrap();
        let b = scratch.folder().join("b.md");
        std::fs::write(&b, "b").unwrap();
        let path = store::library_file(&scratch.folder());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        // The size and hash match a.md, so the load has no fingerprint to correct.
        let v1 = format!(
            "version=1\r\ntag={}|idea\r\nnote={}|-|fp|-|1|{:016x}|a.md\r\n",
            NoteId(9).to_hex(),
            NoteId(5).to_hex(),
            ids::fnv1a(b"a")
        );
        std::fs::write(&path, &v1).unwrap();

        let mut state = load(&scratch.folder(), &scratch.local(), 100).unwrap();
        assert_eq!(state.metadata, Metadata::Ready);
        assert!(state.is_pinned(&a));
        assert_eq!(flush(&mut state).unwrap(), Flushed::Nothing);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            v1,
            "reading alone never rewrites"
        );

        let mut ids = IdSource::new(1, 2);
        let target = state.note_ref(&mut ids, &b);
        state
            .apply(PendingOp::SetPinned {
                note: target,
                value: true,
            })
            .unwrap();
        assert_eq!(flush(&mut state).unwrap(), Flushed::Wrote);
        let written = std::fs::read_to_string(&path).unwrap();
        assert!(written.starts_with("version=2\r\n"), "{written:?}");
        assert!(!written.contains("tag="), "{written:?}");
        assert!(
            written.contains(&format!("note={}|p|1|", NoteId(5).to_hex())),
            "{written:?}"
        );
        assert!(written.ends_with("|b.md\r\n"), "{written:?}");
    }
}
