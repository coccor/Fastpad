//! Window wiring for the note library: the deferred load, rescans, debounced metadata writes,
//! folder deletes, first-save naming, autosave, and pins.

use super::main_window::app_ptr;
use crate::library::{self, LibraryState, ids::IdSource};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::HWND;

mod autosave;
mod daily;
mod external;
mod folders;
mod index;
mod links;
mod load;
mod local_state;
mod naming;
mod note_actions;
mod notebooks;
mod pickers;
mod pins;
mod slots;
mod untitled;
mod watch;

pub(crate) use autosave::*;
pub(crate) use daily::*;
pub(crate) use external::*;
pub(crate) use folders::*;
pub(crate) use index::*;
pub(crate) use links::*;
pub(crate) use load::*;
pub(crate) use local_state::*;
pub(crate) use naming::*;
pub(crate) use note_actions::*;
pub(crate) use notebooks::*;
pub(crate) use pickers::*;
pub(crate) use pins::*;
pub(crate) use slots::*;
pub(crate) use untitled::*;
pub(crate) use watch::*;

pub(crate) const LIBRARY_WRITE_TIMER_ID: usize = 0x4650_4C57;
pub(crate) const RESCAN_AFTER: Duration = Duration::from_secs(5);
const WRITE_DELAY_MS: u32 = 500;
/// How soon a write retries after `library.ini` could not be re-read (held open by a sync).
const BUSY_RETRY_MS: u32 = 2_000;
const CLOSE_BUSY_RETRIES: usize = 3;
pub(crate) const AUTOSAVE_TIMER_ID: usize = 0x4650_4153;
pub(crate) const AUTOSAVE_DELAY_MS: u32 = 1_000;

/// What one autosave attempt did.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Autosave {
    /// Nothing to do: notes mode or the folder's autosave is off, or the active tab is clean,
    /// untitled, outside the folder, or paused.
    NotEligible,
    Saved,
    /// The file changed on disk since FastPad loaded or saved it; autosave is now paused for it.
    Paused,
    /// The write failed; the tab stays dirty.
    Failed,
}

#[derive(Debug)]
pub(crate) struct LibraryHost {
    /// The open notebooks, in sidebar order (multi-root notebooks spec §3.1).
    pub(crate) notebooks: Vec<NotebookSlot>,
    /// The notebook capture and the single-notebook commands go to.
    pub(crate) default_notebook: Option<NotebookId>,
    /// The last `NotebookId` handed out.
    next_id: u32,
    /// The last generation `fresh_generation` handed out.
    next_generation: u64,
    /// The generation of the load of the folder named on the command line, which has no slot
    /// yet: the UI thread adds the root when the worker's answer lands, after checking it
    /// against the open notebooks. `None` once it lands or notes mode goes off.
    startup_load: Option<u64>,
    /// The generation of the answer that carries the startup notice for notebooks that are not
    /// available. It belongs to no slot, so closing one cannot lose it.
    startup_notice: Option<u64>,
    /// Remembered notebooks that were not available at startup. They stay on their `open=` lines
    /// until they open again.
    unavailable: Vec<PathBuf>,
    /// The notebooks as `open=` last listed them: where `unavailable` ones go when it is rewritten.
    open_order: Vec<PathBuf>,
    /// `%LOCALAPPDATA%\FastPad`, resolved lazily. Under `cfg(test)` it is only ever pre-seeded.
    pub(crate) data_dir: Option<PathBuf>,
    pub(crate) inactive_since: Option<Instant>,
    /// IDs for new note records.
    pub(crate) ids: IdSource,
    /// The folders the open recent-folder picker lists, in its row order.
    shown_recent_folders: Vec<PathBuf>,
    /// `folders.ini` as last read or written, so the sidebar lists recent and favorite notebooks
    /// without reading the disk. `None` until the startup step or a first change fills it.
    folders: Option<library::local::RecentFolders>,
    /// The user changed `folders.ini` since startup, so the startup worker's copy is older.
    folders_edited: bool,
    /// Bumped by each `open_listed_notebook` check and by any switch or close that makes an
    /// outstanding one stale: `notebook_checked` drops an answer whose value has fallen behind.
    check_request: u64,
    /// Bumped whenever `set_expanded` changes the expanded set, so the Notebook view knows its
    /// rows are stale without comparing the sets.
    expansion_revision: u64,
    /// The note an open Move to notebook picker moves, and the notebooks it lists, in row order.
    /// The row after the last is "Browse…".
    pub(crate) shown_move: Option<(PathBuf, Vec<PathBuf>)>,
    /// Copies into the tree, queued on one worker thread (open editors spec §4.5).
    pub(crate) copy_worker: super::copy_host::CopyWorker,
    /// Files FastPad itself is writing on a worker (replace across notes, copies), by lowercase
    /// path: the watcher's report of them is not an outside change. `None` while the write runs,
    /// then until when a late report is still FastPad's own.
    pub(crate) own_writes: std::collections::HashMap<String, Option<Instant>>,
    /// The Notebook view shows only notes changed outside FastPad ("Notes: Show changed notes").
    pub(crate) changed_only: bool,
    /// The folder an open template picker makes its note in, and the templates it lists.
    pub(crate) shown_templates: Option<(Option<PathBuf>, Vec<PathBuf>)>,
}

impl LibraryHost {
    pub(crate) fn new(process_start: u64) -> Self {
        Self {
            notebooks: Vec::new(),
            default_notebook: None,
            next_id: 0,
            next_generation: 0,
            startup_load: None,
            startup_notice: None,
            unavailable: Vec::new(),
            open_order: Vec::new(),
            data_dir: None,
            inactive_since: None,
            ids: IdSource::new(process_start, std::process::id()),
            shown_recent_folders: Vec::new(),
            folders: None,
            folders_edited: false,
            check_request: 0,
            expansion_revision: 0,
            shown_move: None,
            copy_worker: super::copy_host::CopyWorker::default(),
            own_writes: std::collections::HashMap::new(),
            changed_only: false,
            shown_templates: None,
        }
    }
}

struct Loaded {
    generation: u64,
    folder: PathBuf,
    result: Result<LibraryState, String>,
    /// At startup, the notice for the remembered notebooks that are not available (it names no
    /// slot: its generation is `startup_notice`).
    notice: Option<String>,
    /// There is nothing to open: the notebook (or the path on the command line) is not
    /// available, so its slot, if it had one, goes.
    closed: bool,
}

fn host<R>(hwnd: HWND, f: impl FnOnce(&mut LibraryHost) -> R) -> Option<R> {
    unsafe { app_ptr(hwnd) }.map(|mut app| f(&mut unsafe { app.as_mut() }.library))
}

fn notes_mode(hwnd: HWND) -> bool {
    unsafe { app_ptr(hwnd) }.is_some_and(|app| unsafe { app.as_ref() }.settings.notes_mode)
}

/// The default notebook's folder (a shim until every caller names its notebook).
pub(crate) fn folder(hwnd: HWND) -> Option<PathBuf> {
    notebook_folder(hwnd, default_notebook(hwnd)?)
}

/// Runs `f` on the default notebook's loaded state (a shim until every caller names its
/// notebook).
pub(crate) fn with_state<R>(hwnd: HWND, f: impl FnOnce(&mut LibraryState) -> R) -> Option<R> {
    with_notebook(hwnd, default_notebook(hwnd)?, f)
}

/// Runs `f` on the window's copy worker.
pub(crate) fn with_copy_worker<R>(
    hwnd: HWND,
    f: impl FnOnce(&mut super::copy_host::CopyWorker) -> R,
) -> Option<R> {
    host(hwnd, |host| f(&mut host.copy_worker))
}

fn data_dir(hwnd: HWND) -> Option<PathBuf> {
    host(hwnd, |host| {
        if host.data_dir.is_none() && !cfg!(test) {
            host.data_dir = crate::platform::fastpad_data_dir().ok();
        }
        host.data_dir.clone()
    })
    .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_folder_delete_question_counts_notes_and_unsaved_tabs() {
        // Break caught: an empty folder asked about as if it held notes, "1 notes", or unsaved
        // edits lost without a word (spec §4.3).
        assert_eq!(
            delete_folder_question("Old", 0, 0),
            "Move the folder \u{201c}Old\u{201d} to the Recycle Bin?"
        );
        assert_eq!(
            delete_folder_question("Old", 1, 0),
            "Move \u{201c}Old\u{201d} and its 1 note to the Recycle Bin?"
        );
        assert_eq!(
            delete_folder_question("Old", 3, 1),
            "Move \u{201c}Old\u{201d} and its 3 notes to the Recycle Bin?\n1 open note has unsaved changes, which will be lost."
        );
        assert_eq!(
            delete_folder_question("Old", 3, 2),
            "Move \u{201c}Old\u{201d} and its 3 notes to the Recycle Bin?\n2 open notes have unsaved changes, which will be lost."
        );
    }

    #[test]
    fn the_failed_folder_rename_undo_notice_names_the_tabs_left_behind() {
        // Break caught: a rename that could not be undone reported as a clash, or the tab left
        // on its old path not named (Task 6 review).
        let stuck = [
            PathBuf::from(r"C:\n\sub\a.md"),
            PathBuf::from(r"C:\n\sub\b.md"),
        ];
        assert_eq!(
            rename_undo_failed_notice("sub", "Moved", &stuck[..1]),
            "FastPad could not undo renaming \u{201c}sub\u{201d} to \u{201c}Moved\u{201d}. \u{201c}a.md\u{201d} is still open at its old path."
        );
        assert_eq!(
            rename_undo_failed_notice("sub", "Moved", &stuck),
            "FastPad could not undo renaming \u{201c}sub\u{201d} to \u{201c}Moved\u{201d}. \u{201c}a.md\u{201d} and 1 more are still open at their old paths."
        );
    }
}
