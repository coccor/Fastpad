//! Window wiring for the note library: the deferred load, rescans, debounced metadata writes,
//! folder deletes, first-save naming, autosave, and pins.

use super::main_window::app_ptr;
use crate::library::{self, LibraryState, ids::IdSource};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::HWND;

mod autosave;
mod folders;
mod load;
mod local_state;
mod naming;
mod note_actions;
mod notebooks;
mod pickers;
mod pins;
mod untitled;

pub(crate) use autosave::*;
pub(crate) use folders::*;
pub(crate) use load::*;
pub(crate) use local_state::*;
pub(crate) use naming::*;
pub(crate) use note_actions::*;
pub(crate) use notebooks::*;
pub(crate) use pickers::*;
pub(crate) use pins::*;
pub(crate) use untitled::*;

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
    /// The open folder. Set as soon as the step runs, before its state has loaded.
    pub(crate) folder: Option<PathBuf>,
    pub(crate) state: Option<LibraryState>,
    /// `%LOCALAPPDATA%\FastPad`, resolved lazily. Under `cfg(test)` it is only ever pre-seeded.
    pub(crate) data_dir: Option<PathBuf>,
    pub(crate) generation: u64,
    pub(crate) scanning: bool,
    pub(crate) rescan_requested: bool,
    /// The UI thread wrote the local file while a scan ran, possibly with the old scan cache, so
    /// the merge that follows writes it again.
    local_written_during_scan: bool,
    pub(crate) inactive_since: Option<Instant>,
    /// IDs for new note records.
    pub(crate) ids: IdSource,
    notified: Option<PathBuf>,
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
    /// The last load of the open notebook failed, so the sidebar offers Retry instead of saying
    /// "Loading…" forever. Starting a load clears it.
    pub(crate) load_failed: bool,
    /// Copies into the tree, queued on one worker thread (open editors spec §4.5).
    pub(crate) copy_worker: super::copy_host::CopyWorker,
}

impl LibraryHost {
    pub(crate) fn new(process_start: u64) -> Self {
        Self {
            folder: None,
            state: None,
            data_dir: None,
            generation: 0,
            scanning: false,
            rescan_requested: false,
            local_written_during_scan: false,
            inactive_since: None,
            ids: IdSource::new(process_start, std::process::id()),
            notified: None,
            shown_recent_folders: Vec::new(),
            folders: None,
            folders_edited: false,
            check_request: 0,
            expansion_revision: 0,
            shown_move: None,
            load_failed: false,
            copy_worker: super::copy_host::CopyWorker::default(),
        }
    }
}

struct Loaded {
    generation: u64,
    folder: PathBuf,
    result: Result<LibraryState, String>,
    /// Why the worker opened a different folder than the one the UI thread expected.
    notice: Option<String>,
    /// The last session closed its notebook and the command line named no folder: nothing was
    /// opened, on purpose.
    closed: bool,
    /// `folders.ini` as the startup worker read it, after any change it made itself.
    folders: Option<library::local::RecentFolders>,
}

/// The startup candidates, checked on the worker so an offline drive cannot stall the UI thread.
struct Startup {
    /// A path named on the command line, which may be a file.
    launch: Option<PathBuf>,
    /// The most recent folder from `folders.ini`, unless the last session closed its notebook.
    remembered: Option<PathBuf>,
    /// `open=none`: the last session ended with no notebook open.
    closed: bool,
    data: PathBuf,
}

fn host<R>(hwnd: HWND, f: impl FnOnce(&mut LibraryHost) -> R) -> Option<R> {
    unsafe { app_ptr(hwnd) }.map(|mut app| f(&mut unsafe { app.as_mut() }.library))
}

fn notes_mode(hwnd: HWND) -> bool {
    unsafe { app_ptr(hwnd) }.is_some_and(|app| unsafe { app.as_ref() }.settings.notes_mode)
}

pub(crate) fn folder(hwnd: HWND) -> Option<PathBuf> {
    host(hwnd, |host| host.folder.clone()).flatten()
}

pub(crate) fn with_state<R>(hwnd: HWND, f: impl FnOnce(&mut LibraryState) -> R) -> Option<R> {
    host(hwnd, |host| host.state.as_mut().map(f)).flatten()
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

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(label: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "fastpad-host-startup-{label}-{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(root.join("data")).unwrap();
            std::fs::create_dir_all(root.join("notes")).unwrap();
            Self(root)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_session_that_closed_its_notebook_opens_none_and_checks_no_folder() {
        // Break caught: startup reopening the last notebook, or falling back to
        // Documents\FastPad, after the user closed it.
        let scratch = Scratch::new("closed");
        let before = library::folder_checks();
        let resolved = resolve_startup(Startup {
            launch: None,
            remembered: None,
            closed: true,
            data: scratch.0.join("data"),
        });
        assert_eq!(resolved, (None, None));
        assert_eq!(library::folder_checks(), before);
    }

    #[test]
    fn a_folder_on_the_command_line_opens_after_a_close_and_clears_it() {
        // Break caught: `fastpad.exe D:\Notes` refused after a close, or leaving open=none so the
        // next plain start opens nothing again.
        let scratch = Scratch::new("closed-launch");
        let data = scratch.0.join("data");
        let mut folders = library::local::RecentFolders::default();
        folders.set_closed(true);
        library::local::write_folders(&library::local::folders_file(&data), &folders).unwrap();
        let notes = library::normalize_folder(&scratch.0.join("notes"));
        let (folder, notice) = resolve_startup(Startup {
            launch: Some(notes.clone()),
            remembered: None,
            closed: true,
            data: data.clone(),
        });
        assert_eq!((folder, notice), (Some(notes.clone()), None));
        let saved = library::local::read_folders(&library::local::folders_file(&data));
        assert!(!saved.closed);
        assert_eq!(saved.folders, [notes]);
    }

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
