//! The note library: a notebook (a folder) of plain text files seen as notes, plus sparse pins
//! kept in `.fastpad\library.ini` and attached to files by path, file ID and content fingerprint.
//! Nothing here touches a window.

pub mod ids;
pub mod local;
pub mod model;
pub mod ops;
pub mod quick_open;
pub mod reconcile;
pub mod scan;
pub mod store;
pub mod text_replace;
pub mod text_search;
pub mod title;
pub mod tree;

use crate::Result;
use ids::IdSource;
use local::LocalState;
use model::{Library, LibraryError, NoteRecord, NoteRef, same_path};
use ops::PendingOp;
use std::collections::{HashMap, HashSet};
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
    /// The file's data is only in the cloud (a OneDrive "online only" file): reading it would
    /// download it, so text search skips it. Set by the scan; a save by FastPad clears it.
    pub online_only: bool,
}

/// A folder change FastPad made itself (notebook folders spec §3.2). Each is kept until the next
/// rescan starts, and replayed onto that rescan's result, which may have listed the folders
/// before the change.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FolderChange {
    Added(PathBuf),
    Removed(PathBuf),
    Renamed { old: PathBuf, new: PathBuf },
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
    /// Every folder the scan walked, relative and spelled as on disk, the root not included
    /// (notebook folders spec §3.1). A rescan replaces it; FastPad's own folder commands update
    /// it in place.
    pub folders: Vec<PathBuf>,
    /// FastPad's own folder changes since the running rescan started, replayed onto its result.
    pub folder_changes: Vec<FolderChange>,
    /// The notes as the sidebar's folder tree. Built with the scan on the worker; every later
    /// change to `notes` or to a pin updates it in place.
    pub tree: tree::NoteTree,
    pub truncated: bool,
    pub pending: Vec<PendingOp>,
    pub relocated: Vec<(PathBuf, PathBuf)>,
    /// Paths (as records store them) FastPad itself added, removed or renamed in `notes` since
    /// the running rescan started. Only these are re-checked when its result is merged.
    pub touched: Vec<PathBuf>,
    /// The expanded folders, autosave switch and missing times as the local file last written
    /// holds them, so the UI thread rewrites that file only when one of them changed.
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

/// The notes a tree shows as pinned: pinned records that are not flagged deleted.
fn pinned_paths(library: &Library) -> Vec<PathBuf> {
    library
        .notes
        .iter()
        .filter(|record| record.pinned && !record.deleted && !record.path.is_absolute())
        .map(|record| record.path.clone())
        .collect()
}

/// A note path as a key that compares ignoring case: the rule of `model::same_path`.
pub(crate) fn path_key(path: &Path) -> String {
    path.to_string_lossy().to_lowercase()
}

/// Brings the tree's pins from `before` to `after` without a rebuild.
fn sync_pins(tree: &mut tree::NoteTree, before: &[PathBuf], after: &[PathBuf]) {
    let before_keys: HashSet<String> = before.iter().map(|path| path_key(path)).collect();
    let after_keys: HashSet<String> = after.iter().map(|path| path_key(path)).collect();
    for path in after {
        if !before_keys.contains(&path_key(path)) {
            tree.set_pinned(path, true);
        }
    }
    for path in before {
        if !after_keys.contains(&path_key(path)) {
            tree.set_pinned(path, false);
        }
    }
}

/// Whether `path` is `folder` itself or inside it, component-wise and ignoring case.
pub fn at_or_under(path: &Path, folder: &Path) -> bool {
    same_path(path, folder) || strip_folder(folder, path).is_some()
}

/// `path` moved from `old` to `new`, when it is `old` itself or inside it.
pub fn reroot(path: &Path, old: &Path, new: &Path) -> Option<PathBuf> {
    if same_path(path, old) {
        return Some(new.to_path_buf());
    }
    strip_folder(old, path).map(|rest| new.join(rest))
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
    let notes: Vec<NoteEntry> = scan
        .entries
        .iter()
        .map(|entry| NoteEntry {
            path: entry.path.clone(),
            size: entry.size,
            mtime: entry.mtime,
            online_only: entry.online_only,
        })
        .collect();
    // Built from the note list itself: a copy of 10,000 paths would outlive the scan as freed
    // heap in the idle working set.
    let tree = tree::NoteTree::build(
        notes.iter().map(|note| note.path.as_path()),
        &scan.folders,
        &pinned_paths(&library),
    );
    Ok(LibraryState {
        folder: folder.to_path_buf(),
        local_path: local_path.to_path_buf(),
        library,
        metadata,
        stamp,
        notes,
        tree,
        truncated: scan.truncated,
        folders: scan.folders,
        folder_changes: Vec::new(),
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
        let before = pinned_paths(&state.library);
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
        // Another PC's pins arrived with the re-read.
        sync_pins(&mut state.tree, &before, &pinned_paths(&state.library));
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

/// The per-PC state to write, when its expanded folders, autosave switch or missing times changed
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
        // A rename leaves a file online only, and the rescan's own entry says whether it still is.
        let online_only = merged
            .iter()
            .find(|note| same_path(&note.path, path))
            .is_some_and(|note| note.online_only);
        merged.retain(|note| !same_path(&note.path, path));
        let is_note = title::is_listed_path(path);
        if is_note && let Some(stamp) = store::stamp(&folder.join(path)) {
            merged.push(NoteEntry {
                path: path.clone(),
                size: stamp.size,
                mtime: filetime_ticks(stamp.modified),
                online_only,
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
        notes: previous_notes,
        touched,
        folder_changes,
        ..
    } = previous;
    let mut fresh = fresh;
    // The rescan built its tree from these pins; what the merge changes is applied to it below.
    let built_pins = pinned_paths(&fresh.library);
    // FastPad's folder changes while the rescan ran, which its result may not have seen: first,
    // so the touched notes below are re-checked at their current paths.
    for change in &folder_changes {
        fresh.change_folders(change);
    }
    // A rescan that listed a folder just before FastPad renamed it found it gone when it read
    // inside: it has none of the notes now under the new name. The live state's are current.
    let mut carried = Vec::new();
    for change in &folder_changes {
        if let FolderChange::Renamed { new, .. } = change
            && !fresh.notes.iter().any(|note| at_or_under(&note.path, new))
        {
            for note in previous_notes
                .iter()
                .filter(|note| at_or_under(&note.path, new))
            {
                carried.push(note.path.clone());
                fresh.notes.push(note.clone());
            }
        }
    }
    fresh.notes = merge_notes(&fresh.folder, std::mem::take(&mut fresh.notes), &touched);
    if fresh.truncated {
        // A truncated scan's own list is already capped at the limit; touched entries that
        // survived the merge must not push it past that.
        fresh.notes.truncate(scan::NOTE_LIMIT);
    }
    // The UI thread owns these conveniences: what it set while the rescan ran wins.
    fresh.local.autosave = previous_local.autosave;
    fresh.local.expanded = previous_local.expanded;
    fresh.local.root_collapsed = previous_local.root_collapsed;

    if fresh.metadata == Metadata::Busy {
        // The rescan could not read library.ini: what the live state knows still stands, and the
        // next rescan reads it again.
        fresh.library = previous_library;
        fresh.metadata = previous_metadata;
        fresh.stamp = previous_stamp;
        fresh.pending = previous_pending;
    } else {
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

        let mut pending = std::mem::take(&mut fresh.pending);
        pending.extend(previous_pending);
        fresh.pending = pending;
    }
    carried.extend(touched);
    update_merged_tree(&mut fresh, &carried, &built_pins);
    fresh
}

/// Brings the rescan's tree up to the merged notes and pins. Only the paths FastPad touched while
/// the rescan ran can differ from the notes it was built from, and only a pin the merge changed
/// can differ from its pins, so this runs on the UI thread at the cost of those few paths.
fn update_merged_tree(state: &mut LibraryState, touched: &[PathBuf], built_pins: &[PathBuf]) {
    let pins = pinned_paths(&state.library);
    let pinned: HashSet<String> = pins.iter().map(|path| path_key(path)).collect();
    for path in touched {
        if state.notes.iter().any(|note| same_path(&note.path, path)) {
            state.list_folders_of(path);
            state
                .tree
                .insert_note(path, pinned.contains(&path_key(path)));
        } else {
            state.tree.remove_note(path);
        }
    }
    sync_pins(&mut state.tree, built_pins, &pins);
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

    /// Applies `op` to the live library and keeps it for the next write. A pin change moves the
    /// note's row in the tree.
    pub fn apply(&mut self, op: PendingOp) -> std::result::Result<(), LibraryError> {
        ops::apply(&mut self.library, &op)?;
        if let PendingOp::SetPinned { note, value } = &op {
            let path = self
                .library
                .note(note.id)
                .or_else(|| self.library.note_by_path(&note.path))
                .map(|record| record.path.clone());
            if let Some(path) = path {
                self.tree.set_pinned(&path, *value);
            }
        }
        self.pending.push(op);
        Ok(())
    }

    /// Adds a file FastPad just saved to the index and the tree, if it is a note inside the
    /// folder. Updates the entry in place when the note is already indexed. Returns whether it
    /// added a new entry, so the tree gained a row.
    pub fn add_note(&mut self, path: &Path) -> bool {
        let Some(relative) = strip_folder(&self.folder, path) else {
            return false;
        };
        if !title::is_listed_path(&relative) {
            return false;
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
            // FastPad just wrote the file, so its data is on this PC.
            existing.online_only = false;
            false
        } else {
            let pinned = self.is_pinned(&relative);
            self.list_folders_of(&relative);
            self.tree.insert_note(&relative, pinned);
            self.notes.push(NoteEntry {
                path: relative,
                size,
                mtime,
                online_only: false,
            });
            true
        }
    }

    /// Lists every folder above the note at `relative` that `folders` lacks, spelled as in its
    /// path: a note saved into a folder the scan never saw (one made in the save dialog) gets
    /// the folder rows a rebuild from `folders` would give it. Touches no disk.
    fn list_folders_of(&mut self, relative: &Path) {
        let Some(parent) = relative.parent() else {
            return;
        };
        let mut missing: Vec<PathBuf> = parent
            .ancestors()
            .filter(|folder| !folder.as_os_str().is_empty())
            .filter(|folder| !self.folders.iter().any(|listed| same_path(listed, folder)))
            .map(Path::to_path_buf)
            .collect();
        missing.reverse();
        self.folders.extend(missing);
    }

    /// Records a note FastPad just wrote outside the editor (a Search replace) from the stamp
    /// the writer took of the saved file, so the next rescan reads no outside change. It is the
    /// update `add_note` makes for a note already listed (size, time, `online_only` cleared, a
    /// `touched` entry), with no disk access: the UI thread calls it for every written note. A
    /// single note is found by a linear `same_path` scan, the same way `add_note` finds one,
    /// rather than building the batch key map `record_written_all` needs. Returns false, changing
    /// nothing, when `relative` isn't listed.
    pub fn record_written(&mut self, relative: &Path, stamp: text_search::Stamp) -> bool {
        let Some(existing) = self
            .notes
            .iter_mut()
            .find(|note| same_path(&note.path, relative))
        else {
            return false;
        };
        Self::apply_written(existing, stamp);
        self.touched.push(relative.to_path_buf());
        true
    }

    /// Records every note in `written` the way `record_written` records one, in a single pass
    /// over `notes` (a path key to index map, built once), so a batch of many written notes costs
    /// one scan of the list instead of one `same_path` scan per note. Returns how many of
    /// `written`'s paths were listed.
    pub fn record_written_all(&mut self, written: &[(PathBuf, text_search::Stamp)]) -> usize {
        let index_of: HashMap<String, usize> = self
            .notes
            .iter()
            .enumerate()
            .map(|(index, note)| (path_key(&note.path), index))
            .collect();
        let mut recorded = 0;
        for (relative, stamp) in written {
            let Some(&index) = index_of.get(&path_key(relative)) else {
                continue;
            };
            Self::apply_written(&mut self.notes[index], *stamp);
            self.touched.push(relative.clone());
            recorded += 1;
        }
        recorded
    }

    /// The update a written note's entry gets, shared by `record_written` and
    /// `record_written_all`: the new size and time, and `online_only` cleared, because FastPad
    /// just wrote the file, so its data is on this PC.
    fn apply_written(entry: &mut NoteEntry, stamp: text_search::Stamp) {
        entry.size = stamp.size;
        entry.mtime = stamp.mtime;
        entry.online_only = false;
    }

    pub fn remove_note(&mut self, path: &Path) {
        let Some(stored) = strip_folder(&self.folder, path) else {
            return;
        };
        self.notes.retain(|note| !same_path(&note.path, &stored));
        self.tree.remove_note(&stored);
        self.touched.push(stored);
    }

    /// Follows a rename FastPad made: the record, the index and the tree. The record moves first,
    /// so the entry added for the new name finds its pin.
    pub fn rename_note(&mut self, old: &Path, new: &Path) {
        // A rename moves no data: an online-only note stays online only.
        let online_only = strip_folder(&self.folder, old).is_some_and(|stored| {
            self.notes
                .iter()
                .any(|note| note.online_only && same_path(&note.path, &stored))
        });
        let old_stored = record_path(&self.folder, old);
        let new_stored = record_path(&self.folder, new);
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
        self.remove_note(old);
        let _ = self.add_note(new);
        if online_only
            && let Some(relative) = strip_folder(&self.folder, new)
            && let Some(entry) = self
                .notes
                .iter_mut()
                .find(|note| same_path(&note.path, &relative))
        {
            entry.online_only = true;
        }
    }

    /// Whether the tree has a folder at `relative`, ignoring case.
    pub fn is_folder(&self, relative: &Path) -> bool {
        self.tree.contains_folder(relative)
    }

    /// Drops a folder's row from the tree, as a rescan that no longer finds it would, without
    /// touching `notes` or disk (tree drag spec §3.3 tests).
    #[cfg(test)]
    pub fn remove_folder_for_test(&mut self, relative: &Path) {
        self.tree.remove_folder(relative);
    }

    /// Whether a folder or a listed note already has the path `relative`, ignoring case: what a
    /// new or renamed folder may not take (notebook folders spec §4.1).
    pub fn is_listed(&self, relative: &Path) -> bool {
        self.is_folder(relative)
            || self
                .notes
                .iter()
                .any(|note| same_path(&note.path, relative))
    }

    /// How many listed notes are inside the folder `relative`, at any depth.
    pub fn notes_under(&self, relative: &Path) -> usize {
        self.notes
            .iter()
            .filter(|note| at_or_under(&note.path, relative))
            .count()
    }

    /// Follows a folder FastPad just created: it is listed and has a row.
    pub fn add_folder(&mut self, relative: &Path) {
        let change = FolderChange::Added(relative.to_path_buf());
        self.change_folders(&change);
        self.folder_changes.push(change);
    }

    /// Follows a folder rename FastPad just made on disk: the records of the notes under it move
    /// (`Relocate`, so their IDs and pins survive), and so do the notes, the folder list, the
    /// expanded folders and the tree. Touches no disk. Does nothing unless both paths are plain
    /// relative folders.
    pub fn rename_folder(&mut self, old: &Path, new: &Path) {
        if !tree::is_plain_relative_folder(old) || !tree::is_plain_relative_folder(new) {
            return;
        }
        let relocations: Vec<PendingOp> = self
            .library
            .notes
            .iter()
            .filter(|record| !record.deleted && !record.path.is_absolute())
            .filter_map(|record| {
                let path = reroot(&record.path, old, new)?;
                Some(PendingOp::Relocate {
                    note: NoteRef {
                        id: record.id,
                        path: record.path.clone(),
                    },
                    path,
                })
            })
            .collect();
        for op in relocations {
            let _ = self.apply(op);
        }
        for entry in self
            .local
            .expanded
            .iter_mut()
            .chain(self.touched.iter_mut())
        {
            if let Some(moved) = reroot(entry, old, new) {
                *entry = moved;
            }
        }
        let change = FolderChange::Renamed {
            old: old.to_path_buf(),
            new: new.to_path_buf(),
        };
        self.change_folders(&change);
        self.folder_changes.push(change);
    }

    /// Follows a folder FastPad just sent to the Recycle Bin: its notes leave the list and the
    /// tree, their records are flagged deleted and marked missing at `now`, as a deleted note's
    /// are, and its expanded entries go. Touches no disk. Does nothing unless `relative` is a
    /// plain relative folder: an empty path would otherwise take every note.
    pub fn remove_folder(&mut self, relative: &Path, now: u64) {
        if !tree::is_plain_relative_folder(relative) {
            return;
        }
        let doomed: Vec<NoteRef> = self
            .library
            .notes
            .iter()
            .filter(|record| {
                !record.deleted && !record.path.is_absolute() && at_or_under(&record.path, relative)
            })
            .map(|record| NoteRef {
                id: record.id,
                path: record.path.clone(),
            })
            .collect();
        for note in doomed {
            let id = note.id;
            let _ = self.apply(PendingOp::SetDeleted { note, value: true });
            self.local.set_missing(id, now);
        }
        self.local
            .expanded
            .retain(|folder| !at_or_under(folder, relative));
        self.touched.retain(|path| !at_or_under(path, relative));
        let change = FolderChange::Removed(relative.to_path_buf());
        self.change_folders(&change);
        self.folder_changes.push(change);
    }

    /// The in-memory part of a folder change: the folder list, the notes and the tree. A
    /// rescan's result gets only this part replayed: its records come from the pending
    /// operations, and its expanded folders from the live state.
    fn change_folders(&mut self, change: &FolderChange) {
        match change {
            FolderChange::Added(path) => {
                if !self.folders.iter().any(|folder| same_path(folder, path)) {
                    self.folders.push(path.clone());
                }
                self.tree.insert_folder(path);
            }
            FolderChange::Removed(path) => {
                self.folders.retain(|folder| !at_or_under(folder, path));
                self.notes.retain(|note| !at_or_under(&note.path, path));
                self.tree.remove_folder(path);
            }
            FolderChange::Renamed { old, new } => {
                for folder in &mut self.folders {
                    if let Some(moved) = reroot(folder, old, new) {
                        *folder = moved;
                    }
                }
                if !self.folders.iter().any(|folder| same_path(folder, new)) {
                    self.folders.push(new.clone());
                }
                for note in &mut self.notes {
                    if let Some(moved) = reroot(&note.path, old, new) {
                        note.path = moved;
                    }
                }
                self.tree.rename_folder(old, new);
            }
        }
    }
}

#[cfg(test)]
mod tests;
