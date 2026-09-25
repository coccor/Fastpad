//! Window wiring for the note library: the deferred load, rescans, debounced metadata writes,
//! folder commands, first-save naming, autosave, and pins.

use super::main_window::{app_ptr, push_notice, window_identity};
use crate::library::model::LibraryError;
use crate::library::ops::PendingOp;
use crate::library::title;
use crate::library::{self, LibraryState, Metadata, ids::IdSource};
use crate::window::command_palette::{Picker, PickerChoice, PickerKind};
use crate::window::name_box::{NameBox, NamePurpose};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{HWND, LPARAM};
use windows_sys::Win32::UI::WindowsAndMessaging::{KillTimer, PostMessageW, SetTimer};

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

fn data_dir(hwnd: HWND) -> Option<PathBuf> {
    host(hwnd, |host| {
        if host.data_dir.is_none() && !cfg!(test) {
            host.data_dir = crate::platform::fastpad_data_dir().ok();
        }
        host.data_dir.clone()
    })
    .flatten()
}

/// On the worker: the folder to open at startup, a directory named on the command line, else
/// nothing when the last session closed its notebook, else the most recent folder if it still
/// exists, else `Documents\FastPad`; and a notice when the most recent folder is gone.
fn resolve_startup(startup: Startup) -> (Option<PathBuf>, Option<String>) {
    if let Some(launch) = startup.launch
        && library::folder_exists(&launch)
    {
        remember_folder(&startup.data, &launch);
        return (Some(launch), None);
    }
    if startup.closed {
        return (None, None);
    }
    let mut notice = None;
    if let Some(remembered) = startup.remembered {
        if library::folder_exists(&remembered) {
            return (Some(remembered), None);
        }
        notice = Some(format!(
            "FastPad could not find the notebook {}. Using Documents\\FastPad instead.",
            remembered.display()
        ));
    }
    let fallback = crate::platform::paths::default_notes_folder()
        .ok()
        .map(|folder| library::normalize_folder(&folder));
    (fallback, notice)
}

pub(crate) fn remember_folder(data: &Path, folder: &Path) {
    let path = library::local::folders_file(data);
    let mut recent = library::local::read_folders(&path);
    recent.push(folder.to_path_buf());
    let _ = library::local::write_folders(&path, &recent);
}

/// `WM_FASTPAD_OPEN_LIBRARY`: starts the worker on the startup folder. Reads only `folders.ini`:
/// whether the folder (or a command-line path) exists is checked on the worker, which may open a
/// different folder than the one assumed here. After a session that closed its notebook, with no
/// path on the command line, nothing opens and no worker starts.
pub(crate) fn open_library_step(hwnd: HWND) {
    if !notes_mode(hwnd) {
        return;
    }
    let Some(data) = data_dir(hwnd) else {
        return;
    };
    let launch =
        unsafe { app_ptr(hwnd) }.and_then(|app| match &unsafe { app.as_ref() }.launch.request {
            crate::launch::LaunchRequest::Open(path) => {
                Some(library::normalize_folder(Path::new(path)))
            }
            crate::launch::LaunchRequest::New => None,
        });
    let recent = library::local::read_folders(&library::local::folders_file(&data));
    host(hwnd, |host| host.folders = Some(recent.clone()));
    if recent.closed && launch.is_none() {
        host(hwnd, |host| host.folder = None);
        super::side_panel::refresh(hwnd);
        return;
    }
    let remembered = if recent.closed {
        None
    } else {
        recent
            .folders
            .first()
            .map(|folder| library::normalize_folder(folder))
    };
    // Until the worker has checked, the name box saves into the folder that usually wins.
    let assumed = if recent.closed {
        None
    } else {
        remembered.clone().or_else(|| {
            crate::platform::paths::default_notes_folder()
                .ok()
                .map(|folder| library::normalize_folder(&folder))
        })
    };
    host(hwnd, |host| host.folder = assumed);
    // The sidebar says "Loading…" for the assumed notebook until the worker answers, not "Open a
    // notebook" (a refresh earlier in startup saw no folder). No flattening: nothing is loaded.
    super::side_panel::refresh(hwnd);
    spawn_load(
        hwnd,
        Some(Startup {
            launch,
            remembered,
            closed: recent.closed,
            data,
        }),
    );
}

pub(crate) fn start_load(hwnd: HWND) {
    spawn_load(hwnd, None);
}

/// Whether the open notebook failed to load and has no state to show: the sidebar says so and
/// offers Retry. A rescan that fails keeps the state it had, so this stays false then.
pub(crate) fn load_failed(hwnd: HWND) -> bool {
    host(hwnd, |host| {
        host.load_failed && host.folder.is_some() && host.state.is_none()
    })
    .unwrap_or(false)
}

/// The sidebar's Retry after a failed load: loads the same notebook again, showing "Loading…"
/// meanwhile.
pub(crate) fn retry_load(hwnd: HWND) {
    if folder(hwnd).is_none() {
        return;
    }
    if host(hwnd, |host| host.scanning).unwrap_or(true) {
        return;
    }
    start_load(hwnd);
    super::side_panel::refresh(hwnd);
}

fn spawn_load(hwnd: HWND, startup: Option<Startup>) {
    let Some(data) = data_dir(hwnd) else {
        return;
    };
    let Some((folder, generation)) = host(hwnd, |host| {
        let folder = host.folder.clone();
        if folder.is_none() && startup.is_none() {
            return None;
        }
        host.generation = host.generation.wrapping_add(1);
        host.scanning = true;
        host.rescan_requested = false;
        host.load_failed = false;
        host.local_written_during_scan = false;
        // The merge re-checks only what FastPad changes in the index from here on.
        if let Some(state) = host.state.as_mut() {
            state.touched.clear();
        }
        Some((folder, host.generation))
    })
    .flatten() else {
        return;
    };
    let target = hwnd as isize;
    std::thread::spawn(move || {
        let read_folders = startup.is_some();
        let closed = startup.as_ref().is_some_and(|startup| startup.closed);
        let (folder, notice) = match startup {
            Some(startup) => resolve_startup(startup),
            None => (folder, None),
        };
        let opens_nothing = closed && folder.is_none();
        let folders = read_folders
            .then(|| library::local::read_folders(&library::local::folders_file(&data)));
        let (folder, result) = match folder {
            Some(folder) => {
                let local_path = library::local::local_file(&data, &folder);
                let result = library::load(&folder, &local_path, library::now_unix())
                    .map_err(|error| error.to_string());
                (folder, result)
            }
            None => (
                PathBuf::new(),
                Err("the Documents folder could not be found".to_owned()),
            ),
        };
        let payload = Box::into_raw(Box::new(Loaded {
            generation,
            folder,
            result,
            notice,
            closed: opens_nothing,
            folders,
        }));
        if unsafe {
            PostMessageW(
                target as HWND,
                crate::window::WM_FASTPAD_LIBRARY_READY,
                0,
                payload as isize,
            )
        } == 0
        {
            drop(unsafe { Box::from_raw(payload) });
        }
    });
}

#[cfg(test)]
pub(crate) fn test_ready_payload(generation: u64, result: Result<LibraryState, String>) -> LPARAM {
    Box::into_raw(Box::new(Loaded {
        generation,
        folder: PathBuf::new(),
        result,
        notice: None,
        closed: false,
        folders: None,
    })) as LPARAM
}

/// `WM_FASTPAD_LIBRARY_READY`: installs the worker's result unless a newer load superseded it.
pub(crate) fn library_ready(hwnd: HWND, lparam: LPARAM) {
    if lparam == 0 {
        return;
    }
    let loaded = unsafe { Box::from_raw(lparam as *mut Loaded) };
    let current = host(hwnd, |host| {
        if host.generation != loaded.generation {
            return false;
        }
        host.scanning = false;
        true
    })
    .unwrap_or(false);
    if !current {
        return;
    }
    let Loaded {
        folder,
        result,
        notice,
        closed,
        folders,
        ..
    } = *loaded;
    if let Some(notice) = notice {
        push_notice(hwnd, notice);
    }
    // A change the user made while the worker ran is newer than what the worker read.
    if let Some(folders) = folders {
        host(hwnd, |host| {
            if !host.folders_edited {
                host.folders = Some(folders);
            }
        });
    }
    if closed {
        // The path on the command line was not a folder, and the last session closed its
        // notebook: none is open.
        host(hwnd, |host| host.folder = None);
        super::main_window::invalidate_title_strip(hwnd);
        super::side_panel::refresh(hwnd);
        return;
    }
    // At startup the worker decides which folder exists; the UI thread only assumed one.
    let moved = !folder.as_os_str().is_empty()
        && host(hwnd, |host| {
            let moved = !host
                .folder
                .as_ref()
                .is_some_and(|f| library::model::same_path(f, &folder));
            if moved {
                host.folder = Some(folder.clone());
            }
            moved
        })
        .unwrap_or(false);
    if moved {
        super::main_window::invalidate_title_strip(hwnd);
    }
    match result {
        Ok(fresh) => install(hwnd, fresh),
        Err(error) => {
            host(hwnd, |host| host.load_failed = true);
            push_notice(
                hwnd,
                format!(
                    "FastPad could not load the notebook {}: {error}",
                    folder.display()
                ),
            );
            super::side_panel::refresh(hwnd);
        }
    }
    if host(hwnd, |host| std::mem::take(&mut host.rescan_requested)).unwrap_or(false) {
        start_load(hwnd);
    }
}

fn install(hwnd: HWND, fresh: LibraryState) {
    let Some((relocated, first_time, truncated, unreadable, has_pending)) = host(hwnd, |host| {
        let state = match host.state.take() {
            Some(previous) if library::model::same_path(&previous.folder, &fresh.folder) => {
                library::merge_rescan(previous, fresh)
            }
            _ => fresh,
        };
        let first_time = !host
            .notified
            .as_ref()
            .is_some_and(|f| library::model::same_path(f, &state.folder));
        host.notified = Some(state.folder.clone());
        let state = host.state.insert(state);
        (
            std::mem::take(&mut state.relocated),
            first_time,
            state.truncated,
            state.metadata == Metadata::Unreadable,
            !state.pending.is_empty(),
        )
    }) else {
        return;
    };
    let folder = folder(hwnd).unwrap_or_default();
    for (old, new) in relocated {
        let (old_path, new_path) = (folder.join(old), folder.join(new));
        if rebind_open_tab(hwnd, &old_path, new_path.clone()).is_err() {
            report_rebind_failure(hwnd, &old_path, &new_path);
        }
    }
    if first_time && truncated {
        push_notice(
            hwnd,
            "This notebook has more than 10,000 notes. FastPad indexed the first 10,000."
                .to_owned(),
        );
    }
    if first_time && unreadable {
        push_notice(hwnd, READ_ONLY.to_owned());
    }
    if has_pending {
        schedule_write(hwnd);
    }
    let rewrite = host(hwnd, |host| {
        std::mem::take(&mut host.local_written_during_scan)
    })
    .unwrap_or(false);
    save_local(
        hwnd,
        LocalWrite {
            wait: false,
            force: rewrite,
        },
    );
    // An edit made while the folder loaded found no state, so it armed no timer: the dirty tab
    // autosaves now that autosave is known to apply.
    schedule_autosave(hwnd);
    // A load or rescan may have seen outside edits: the Search view's query runs again.
    crate::window::text_search_host::notes_reloaded(hwnd);
    super::side_panel::refresh(hwnd);
    // A notebook's first load reveals the (restored) active note: its row is selected and its
    // folders expand (spec §6.1). A rescan leaves the user's selection where it was.
    if first_time {
        super::side_panel::active_tab_changed(hwnd);
    }
}

#[derive(Clone, Copy)]
struct LocalWrite {
    /// Write on this thread: at window close, a writer thread might not finish before the exit.
    wait: bool,
    /// Write even if the conveniences did not change (the file may hold an old scan cache).
    force: bool,
}

/// Writes the per-PC local file when its expanded folders, autosave switch or missing times changed.
/// It also carries the scan cache (about 1 MB for 10,000 notes), so it is encoded and written on
/// a one-off writer thread from a snapshot: cloning the state is cheap next to encoding it and
/// syncing the write. `local::write_in_order` keeps an older snapshot from landing last.
fn save_local(hwnd: HWND, how: LocalWrite) {
    let job = with_state(hwnd, |state| {
        let local = library::take_local_changes(state)
            .or_else(|| how.force.then(|| state.local.clone()))?;
        Some((state.local_path.clone(), local))
    })
    .flatten();
    let Some((path, local)) = job else {
        return;
    };
    host(hwnd, |host| {
        if host.scanning {
            host.local_written_during_scan = true;
        }
    });
    let order = library::local::next_write();
    // In-process tests write inline, so what they assert is on disk when they look.
    if how.wait || cfg!(test) {
        let _ = library::local::write_in_order(&path, &local, order);
    } else {
        std::thread::spawn(move || {
            let _ = library::local::write_in_order(&path, &local, order);
        });
    }
}

/// Expands or collapses `path`, a folder relative to the open notebook, and remembers it in the
/// per-PC file. The write happens on a writer thread, and only when the set changed.
pub(crate) fn set_expanded(hwnd: HWND, path: &Path, expanded: bool) {
    let changed = with_state(hwnd, |state| {
        let was = state.local.is_expanded(path);
        state.local.set_expanded(path, expanded);
        was != expanded
    })
    .unwrap_or(false);
    if changed {
        host(hwnd, |host| {
            host.expansion_revision = host.expansion_revision.wrapping_add(1);
        });
        save_local(
            hwnd,
            LocalWrite {
                wait: false,
                force: false,
            },
        );
    }
}

/// Changes whenever a folder is expanded or collapsed.
pub(crate) fn expansion_revision(hwnd: HWND) -> u64 {
    host(hwnd, |host| host.expansion_revision).unwrap_or(0)
}

/// The open notebook's expanded folders, relative to it.
#[cfg_attr(not(test), expect(dead_code, reason = "read by the window tests"))]
pub(crate) fn expanded(hwnd: HWND) -> Vec<PathBuf> {
    with_state(hwnd, |state| state.local.expanded.clone()).unwrap_or_default()
}

/// A note moved outside FastPad (or was moved/renamed by FastPad itself): an open tab for it
/// follows the file. The old path is gone, so the tab is found by its stored path, not through
/// the disk. A move leaves the content alone, so when the new file's stamp is the one the tab
/// knows, autosave carries on (or resumes, if it paused when the old file vanished); otherwise
/// the tab keeps what it knew, and its next autosave pauses on the changed file.
///
/// `Ok(true)` when a tab followed, `Ok(false)` when none had `old` open, `Err` when a tab had
/// `old` open but could not be rebound because another tab already has `new` open — the file
/// moved, but that tab is left pointing at a path that no longer exists. `move_note_to` refuses
/// upfront when the target is already open, and `submit_rename` rebinds its own tab itself (and
/// undoes the rename when it cannot), so `Err` comes from a relocation the rescan finds: a file
/// moved outside FastPad onto a path another tab already has open. The caller reports it.
fn rebind_open_tab(hwnd: HWND, old: &Path, new: PathBuf) -> Result<bool, ()> {
    let stamp = library::disk_stamp(&new);
    let outcome = unsafe { app_ptr(hwnd) }.map(|mut app| {
        let app = unsafe { app.as_mut() };
        let Some(id) = app.tabs.find_stored_path(old) else {
            return Ok(false);
        };
        if app.tabs.rebind_path(id, new).is_err() {
            return Err(());
        }
        if let Some(document) = app.tabs.document_mut(id)
            && document.disk_stamp.is_some()
            && document.disk_stamp == stamp
        {
            document.autosave_paused = false;
        }
        Ok(true)
    });
    match outcome {
        Some(Ok(true)) => {
            super::main_window::invalidate_title_strip(hwnd);
            Ok(true)
        }
        Some(Ok(false)) => Ok(false),
        Some(Err(())) => Err(()),
        None => Ok(false),
    }
}

pub(crate) fn request_rescan(hwnd: HWND) {
    let scanning = host(hwnd, |host| {
        if host.scanning {
            host.rescan_requested = true;
        }
        host.scanning
    })
    .unwrap_or(true);
    if !scanning && folder(hwnd).is_some() {
        start_load(hwnd);
    }
}

/// `WM_ACTIVATEAPP`. Coming back after at least `RESCAN_AFTER` rescans, to catch Explorer edits.
pub(crate) fn activation_changed(hwnd: HWND, active: bool) {
    if !active {
        host(hwnd, |host| host.inactive_since = Some(Instant::now()));
        autosave_active(hwnd);
        return;
    }
    let long_enough = host(hwnd, |host| {
        host.inactive_since
            .take()
            .is_some_and(|since| since.elapsed() >= RESCAN_AFTER)
    })
    .unwrap_or(false);
    if long_enough && notes_mode(hwnd) {
        request_rescan(hwnd);
    }
}

pub(crate) fn schedule_write(hwnd: HWND) {
    unsafe {
        SetTimer(hwnd, LIBRARY_WRITE_TIMER_ID, WRITE_DELAY_MS, None);
    }
}

/// Writes pending metadata now (the debounce timer; tests).
pub(crate) fn flush_now(hwnd: HWND) {
    flush_reporting(hwnd, false);
}

/// Writes pending metadata and the local file on this thread, before the window closes. A
/// `library.ini` held open by a sync gets a few short retries, since there is no later.
pub(crate) fn flush_before_close(hwnd: HWND) {
    for _ in 0..CLOSE_BUSY_RETRIES {
        if !matches!(try_flush(hwnd, true), Ok(library::Flushed::Busy)) {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    flush_reporting(hwnd, true);
}

fn flush_reporting(hwnd: HWND, wait: bool) {
    match try_flush(hwnd, wait) {
        Ok(library::Flushed::Busy) => {
            // OneDrive (or another PC's FastPad) has library.ini open: keep the operations and
            // try again shortly, without a notice for what is usually a brief sync.
            unsafe {
                SetTimer(hwnd, LIBRARY_WRITE_TIMER_ID, BUSY_RETRY_MS, None);
            }
        }
        Ok(_) => {}
        Err(error) => push_notice(
            hwnd,
            format!("FastPad could not save this notebook's pins: {error}"),
        ),
    }
}

fn try_flush(hwnd: HWND, wait: bool) -> crate::Result<library::Flushed> {
    unsafe {
        KillTimer(hwnd, LIBRARY_WRITE_TIMER_ID);
    }
    let result = with_state(hwnd, library::flush).unwrap_or(Ok(library::Flushed::Nothing));
    save_local(hwnd, LocalWrite { wait, force: false });
    result
}

/// Opens `path` as the notebook, flushing the current one first. Open tabs stay open. The folder
/// was just chosen in a dialog, dropped or named by a launch, so it is checked here.
pub(crate) fn open_folder(hwnd: HWND, path: &Path) {
    if !notes_mode(hwnd) {
        push_notice(hwnd, NOTES_MODE_OFF.to_owned());
        return;
    }
    let path = library::normalize_folder(path);
    if !path.is_dir() {
        push_notice(hwnd, format!("{} is not a folder.", path.display()));
        return;
    }
    open_checked_folder(hwnd, path);
}

const NOTES_MODE_OFF: &str =
    "Notes mode is off. Turn it on with Notes: Toggle notes mode to open notebooks.";

/// Saves the dirty notes of the notebook that is about to be replaced or closed, while autosave
/// still applies to them, and flushes its pending pins. Losing the notebook now would drop the
/// unsaved pins, so a failed write keeps it open: this schedules a retry, reports it and returns
/// false. `false` also means the window went away meanwhile.
fn save_before_leaving(hwnd: HWND) -> bool {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return false;
    };
    autosave_all(hwnd);
    if !identity.is_live_for(hwnd) {
        return false;
    }
    let failure = match try_flush(hwnd, false) {
        Ok(library::Flushed::Busy) => Some("its library.ini is in use by another program".into()),
        Ok(_) => None,
        Err(error) => Some(error.to_string()),
    };
    if let Some(error) = failure {
        schedule_write(hwnd);
        push_notice(
            hwnd,
            format!("FastPad kept this notebook open because it could not save its pins: {error}"),
        );
        return false;
    }
    true
}

/// The switch itself, once `path` is known to exist.
fn open_checked_folder(hwnd: HWND, path: PathBuf) {
    if !notes_mode(hwnd) {
        push_notice(hwnd, NOTES_MODE_OFF.to_owned());
        return;
    }
    if !save_before_leaving(hwnd) {
        return;
    }
    host(hwnd, |host| {
        host.state = None;
        host.folder = Some(path.clone());
        // The user moved on: any check still in flight for a listed notebook is stale now.
        host.check_request = host.check_request.wrapping_add(1);
    });
    update_folders(hwnd, |folders| folders.push(path.clone()));
    start_load(hwnd);
    super::main_window::invalidate_title_strip(hwnd);
    super::side_panel::refresh(hwnd);
}

/// What the existence worker found for a notebook picked from a list.
struct NotebookChecked {
    folder: PathBuf,
    exists: bool,
    /// Show the Notebook view once the notebook is open, with the focus in it for `Some(true)`.
    /// The Favorites view asks for it.
    show_notebook: Option<bool>,
    /// `LibraryHost::check_request` when this check started. If the host's has since moved on (a
    /// newer check, or an explicit switch or close), this answer is stale and is dropped.
    check_request: u64,
}

/// Opens a notebook picked from a list (recent, favorites, the no-notebook panel). Such an entry
/// may be on an offline drive, so it is checked on a worker. If it is missing, a notice says so
/// and nothing changes. Unlike startup, nothing falls back to `Documents\FastPad`.
pub(crate) fn open_listed_notebook(hwnd: HWND, folder: &Path) {
    check_listed_notebook(hwnd, folder, None);
}

/// Opens a listed notebook like `open_listed_notebook`, then shows the Notebook view once it is
/// open, with the focus in it for `focus`. A missing notebook changes nothing, the view included
/// (spec §7).
pub(crate) fn open_listed_notebook_in_view(hwnd: HWND, folder: &Path, focus: bool) {
    check_listed_notebook(hwnd, folder, Some(focus));
}

fn check_listed_notebook(hwnd: HWND, folder: &Path, show_notebook: Option<bool>) {
    if !notes_mode(hwnd) {
        push_notice(hwnd, NOTES_MODE_OFF.to_owned());
        return;
    }
    let folder = library::normalize_folder(folder);
    let check_request = host(hwnd, |host| {
        host.check_request = host.check_request.wrapping_add(1);
        host.check_request
    })
    .unwrap_or(0);
    let target = hwnd as isize;
    std::thread::spawn(move || {
        let exists = library::folder_exists(&folder);
        let payload = Box::into_raw(Box::new(NotebookChecked {
            folder,
            exists,
            show_notebook,
            check_request,
        }));
        if unsafe {
            PostMessageW(
                target as HWND,
                crate::window::WM_FASTPAD_NOTEBOOK_CHECKED,
                0,
                payload as isize,
            )
        } == 0
        {
            drop(unsafe { Box::from_raw(payload) });
        }
    });
}

/// `WM_FASTPAD_NOTEBOOK_CHECKED`: frees the worker's answer and switches if the folder exists. An
/// answer whose `check_request` has fallen behind (a newer listed check, or an explicit open or
/// close, ran since this one started) is dropped: the user has already moved on. An answer that
/// lands during a modal dialog is dropped too, as a click there could not happen.
pub(crate) fn notebook_checked(hwnd: HWND, lparam: LPARAM) {
    if lparam == 0 {
        return;
    }
    let checked = *unsafe { Box::from_raw(lparam as *mut NotebookChecked) };
    if super::modal::modal_active(hwnd) {
        return;
    }
    let current = host(hwnd, |host| host.check_request).unwrap_or(checked.check_request);
    if checked.check_request != current {
        return;
    }
    if !checked.exists {
        push_notice(
            hwnd,
            format!("{} is not available.", checked.folder.display()),
        );
        return;
    }
    let already_open =
        folder(hwnd).is_some_and(|open| library::model::same_path(&open, &checked.folder));
    if !already_open {
        open_checked_folder(hwnd, checked.folder);
    }
    if let Some(focus) = checked.show_notebook {
        super::side_panel::show_view(hwnd, crate::config::SidebarView::Notebook, focus);
    }
}

pub(crate) fn choose_and_open_folder(hwnd: HWND) {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    let choice = crate::window::modal::choose_folder(hwnd);
    if !identity.is_live_for(hwnd) {
        return;
    }
    match choice {
        Ok(Some(path)) => open_folder(hwnd, &path),
        Ok(None) => {}
        Err(error) => push_notice(
            hwnd,
            format!("FastPad could not open the folder picker: {error}"),
        ),
    }
}

/// The cached `folders.ini`. `read_if_unknown` reads the file when nothing is cached yet: only
/// for something the user just asked for (a picker), never for painting.
fn known_folders(hwnd: HWND, read_if_unknown: bool) -> library::local::RecentFolders {
    if let Some(folders) = host(hwnd, |host| host.folders.clone()).flatten() {
        return folders;
    }
    if !read_if_unknown {
        return library::local::RecentFolders::default();
    }
    let Some(data) = data_dir(hwnd) else {
        return library::local::RecentFolders::default();
    };
    let folders = library::local::read_folders(&library::local::folders_file(&data));
    host(hwnd, |host| host.folders = Some(folders.clone()));
    folders
}

/// Re-reads `folders.ini` (another window may have changed it), applies `change`, writes it and
/// caches the result. A write failure is reported; the cache still shows the change.
fn update_folders<R>(
    hwnd: HWND,
    change: impl FnOnce(&mut library::local::RecentFolders) -> R,
) -> Option<R> {
    let data = data_dir(hwnd)?;
    let path = library::local::folders_file(&data);
    let mut folders = library::local::read_folders(&path);
    let result = change(&mut folders);
    if let Err(error) = library::local::write_folders(&path, &folders) {
        push_notice(
            hwnd,
            format!("FastPad could not save {}: {error}", path.display()),
        );
    }
    host(hwnd, |host| {
        host.folders = Some(folders);
        host.folders_edited = true;
    });
    Some(result)
}

/// A notebook's display name: its folder's name.
pub(crate) fn notebook_name(folder: &Path) -> String {
    folder
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| folder.display().to_string())
}

/// Favorite notebooks in `folders.ini` order (the Favorites view sorts them by name).
pub(crate) fn favorites(hwnd: HWND) -> Vec<PathBuf> {
    known_folders(hwnd, false).favorites
}

/// Recent notebooks, most recent first.
pub(crate) fn recent_notebooks(hwnd: HWND) -> Vec<PathBuf> {
    known_folders(hwnd, false).folders
}

/// Whether the open notebook is a favorite.
pub(crate) fn is_favorite(hwnd: HWND) -> bool {
    folder(hwnd).is_some_and(|open| known_folders(hwnd, false).is_favorite(&open))
}

/// Notebook: Toggle favorite, and the Notebook view's star.
pub(crate) fn toggle_notebook_favorite(hwnd: HWND) {
    let Some(open) = folder(hwnd) else {
        push_notice(hwnd, "Open a notebook first.".to_owned());
        return;
    };
    let Some((was, now)) = update_folders(hwnd, |folders| {
        let was = folders.is_favorite(&open);
        (was, folders.toggle_favorite(&open))
    }) else {
        return;
    };
    let name = notebook_name(&open);
    let notice = match (was, now) {
        (false, true) => format!("Added {name} to favorite notebooks."),
        (true, false) => format!("Removed {name} from favorite notebooks."),
        // At the cap, `toggle_favorite` leaves the list alone and returns false.
        _ => "You can keep up to 50 favorite notebooks.".to_owned(),
    };
    push_notice(hwnd, notice);
    super::side_panel::refresh(hwnd);
}

/// Removes `folder` from the favorites; nothing happens if it is not one.
pub(crate) fn remove_favorite(hwnd: HWND, folder: &Path) {
    let removed = update_folders(hwnd, |folders| {
        folders.is_favorite(folder) && !folders.toggle_favorite(folder)
    })
    .unwrap_or(false);
    if removed {
        super::side_panel::refresh(hwnd);
    }
}

/// Notebook: Close. Saves the notebook's dirty notes while autosave still applies, writes its
/// pending pins and unloads it. Tabs stay open as plain files, and the next start opens no
/// notebook (`open=none`).
pub(crate) fn close_notebook(hwnd: HWND) {
    let Some(open) = folder(hwnd) else {
        push_notice(hwnd, "No notebook is open.".to_owned());
        return;
    };
    if !save_before_leaving(hwnd) {
        return;
    }
    // A first-save name box would save into the notebook that is going away.
    close_name_box(hwnd);
    unsafe {
        KillTimer(hwnd, LIBRARY_WRITE_TIMER_ID);
        KillTimer(hwnd, AUTOSAVE_TIMER_ID);
    }
    host(hwnd, |host| {
        host.state = None;
        host.folder = None;
        // A load or rescan still running for the closed notebook is ignored when it lands.
        host.generation = host.generation.wrapping_add(1);
        host.scanning = false;
        host.rescan_requested = false;
        // The user moved on: any check still in flight for a listed notebook is stale now.
        host.check_request = host.check_request.wrapping_add(1);
    });
    update_folders(hwnd, |folders| folders.set_closed(true));
    push_notice(
        hwnd,
        format!("Closed the notebook {}.", notebook_name(&open)),
    );
    super::main_window::invalidate_title_strip(hwnd);
    super::side_panel::refresh(hwnd);
}

pub(crate) fn open_recent_folder_picker(hwnd: HWND) {
    let folders = known_folders(hwnd, true).folders;
    if folders.is_empty() {
        push_notice(
            hwnd,
            "No recent notebooks yet. Use File: Open notebook.".to_owned(),
        );
        return;
    }
    let items = folders.iter().map(|f| f.display().to_string()).collect();
    host(hwnd, |host| host.shown_recent_folders = folders);
    super::main_window::open_picker(
        hwnd,
        Picker {
            kind: PickerKind::RecentFolder,
            items,
            create: None,
        },
    );
}

/// Scintilla's own OLE drop target refuses files and wins over `WM_DROPFILES`, so the editor gets
/// a wrapper that posts dropped files here as `WM_FASTPAD_FILES_DROPPED`. Runs in `BUILD_CHROME`.
pub(crate) fn accept_editor_file_drops(hwnd: HWND) {
    let Some(editor) = unsafe { app_ptr(hwnd) }
        .and_then(|app| unsafe { app.as_ref() }.editor.as_ref().map(|e| e.hwnd()))
    else {
        return;
    };
    let target = hwnd as isize;
    // Text drag-and-drop still works without the wrapper; only file drops on the editor are lost.
    let _ = crate::editor::file_drop::accept_file_drops(editor, move |paths| {
        let payload = Box::into_raw(Box::new(paths));
        if unsafe {
            PostMessageW(
                target as HWND,
                crate::window::WM_FASTPAD_FILES_DROPPED,
                0,
                payload as isize,
            )
        } == 0
        {
            drop(unsafe { Box::from_raw(payload) });
        }
    });
}

/// `WM_FASTPAD_FILES_DROPPED`: frees the posted paths and opens them. A drop that lands while a
/// modal dialog runs is ignored, as `WM_DROPFILES` is for a disabled window.
pub(crate) fn editor_files_dropped(hwnd: HWND, lparam: LPARAM) {
    if lparam == 0 {
        return;
    }
    let paths = *unsafe { Box::from_raw(lparam as *mut Vec<PathBuf>) };
    if !super::modal::modal_active(hwnd) {
        files_dropped(hwnd, paths);
    }
}

/// Dropped folders open as the library (the last one wins); dropped files open as tabs.
pub(crate) fn files_dropped(hwnd: HWND, paths: Vec<PathBuf>) {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    let mut folder = None;
    for path in paths {
        if !identity.is_live_for(hwnd) {
            return;
        }
        if path.is_dir() {
            folder = Some(path);
        } else if let Err(error) = super::main_window::open_path(hwnd, &path) {
            super::main_window::report_open_failure(hwnd, &path, &error);
        }
    }
    if let Some(folder) = folder
        && identity.is_live_for(hwnd)
    {
        open_folder(hwnd, &folder);
    }
}

/// Notes mode was toggled: load the last folder, or flush and forget the library.
pub(crate) fn notes_mode_changed(hwnd: HWND, enabled: bool) {
    if enabled {
        if folder(hwnd).is_none() {
            open_library_step(hwnd);
        }
        return;
    }
    // A name box left open would do nothing on Enter once the library is gone.
    close_name_box(hwnd);
    // The setting change applies either way: the user asked for it. The pending operations are
    // tried twice, and a loss is said out loud.
    let flushed = (0..2).any(|_| {
        matches!(
            try_flush(hwnd, false),
            Ok(library::Flushed::Wrote | library::Flushed::Nothing)
        )
    });
    let folder = folder(hwnd);
    let lost = !flushed && with_state(hwnd, |state| !state.pending.is_empty()).unwrap_or(false);
    if lost && let Some(folder) = folder {
        push_notice(
            hwnd,
            format!(
                "Metadata changes could not be written to {}",
                crate::library::store::library_file(&folder).display()
            ),
        );
    }
    unsafe {
        KillTimer(hwnd, LIBRARY_WRITE_TIMER_ID);
    }
    // The Search view goes with the sidebar: its search stops and its record is dropped.
    crate::window::text_search_host::forget(hwnd);
    host(hwnd, |host| {
        host.state = None;
        host.folder = None;
        host.generation = host.generation.wrapping_add(1);
        host.scanning = false;
    });
}

/// Recomputes the active untitled tab's label from its first lines. It is kept with notes mode
/// off too (only edits near the top recompute it), and shown only with notes mode on.
pub(crate) fn refresh_label(hwnd: HWND) {
    let shown = notes_mode(hwnd);
    let changed = unsafe { app_ptr(hwnd) }.and_then(|mut app| {
        let app = unsafe { app.as_mut() };
        let editor = app.editor.as_ref()?;
        let active = app.tabs.active()?;
        if active.path.is_some() {
            return None;
        }
        let id = active.id;
        let count = editor
            .line_count()
            .unwrap_or(0)
            .min(title::LABEL_SCAN_LINES);
        let lines: Vec<String> = (0..count)
            .map(|line| editor.line_text(line).unwrap_or_default())
            .collect();
        let label = title::untitled_label(lines.iter().map(String::as_str));
        let document = app.tabs.document_mut(id)?;
        document.label_watch = label.watch_through;
        document.first_line_label = label.text;
        if !shown || document.untitled_label == document.first_line_label {
            return None;
        }
        document.untitled_label = document.first_line_label.clone();
        Some((id, crate::window::notebook_view::unsaved_label(document)))
    });
    if let Some((id, label)) = changed {
        super::main_window::refresh_tab_view(hwnd);
        // Typing in the first line renames the unsaved row in place, without a rebuild.
        crate::window::notebook_view::unsaved_label_changed(hwnd, id, &label);
    }
}

/// The label a restored or recovered untitled document's text gives, set before it is shown.
pub(crate) fn label_restored_document(
    hwnd: HWND,
    document: &mut crate::document::Document,
    text: &str,
) {
    if document.path.is_some() {
        return;
    }
    let label = title::untitled_label(text.lines());
    document.label_watch = label.watch_through;
    document.first_line_label = label.text;
    if notes_mode(hwnd) {
        document.untitled_label = document.first_line_label.clone();
    }
}

/// Notes mode turned on: every untitled tab shows its label, without switching tabs.
pub(crate) fn show_labels(hwnd: HWND) {
    if let Some(mut app) = unsafe { app_ptr(hwnd) } {
        let app = unsafe { app.as_mut() };
        let ids: Vec<_> = app
            .tabs
            .documents()
            .filter(|document| document.path.is_none())
            .map(|document| document.id)
            .collect();
        for id in ids {
            if let Some(document) = app.tabs.document_mut(id) {
                document.untitled_label = document.first_line_label.clone();
            }
        }
    }
    super::main_window::refresh_tab_view(hwnd);
}

/// `SCN_MODIFIED`: only edits at or above the label's line can change it.
pub(crate) fn text_changed(hwnd: HWND, position: usize) {
    let relevant = unsafe { app_ptr(hwnd) }.is_some_and(|app| {
        let app = unsafe { app.as_ref() };
        let (Some(editor), Some(active)) = (app.editor.as_ref(), app.tabs.active()) else {
            return false;
        };
        active.path.is_none()
            && editor
                .line_from_position(position)
                .is_ok_and(|line| line <= active.label_watch)
    });
    if relevant {
        refresh_label(hwnd);
    }
}

/// Notes mode turned off: untitled tabs go back to "Untitled".
pub(crate) fn clear_labels(hwnd: HWND) {
    if let Some(mut app) = unsafe { app_ptr(hwnd) } {
        let app = unsafe { app.as_mut() };
        let ids: Vec<_> = app.tabs.documents().map(|document| document.id).collect();
        for id in ids {
            if let Some(document) = app.tabs.document_mut(id) {
                document.untitled_label = None;
            }
        }
    }
    super::main_window::refresh_tab_view(hwnd);
}

fn active_untitled(hwnd: HWND) -> Option<crate::document::DocumentId> {
    unsafe { app_ptr(hwnd) }.and_then(|app| {
        let active = unsafe { app.as_ref() }.tabs.active()?;
        active.path.is_none().then_some(active.id)
    })
}

/// "<sanitized label>.<extension for the tab's language>".
pub(crate) fn suggested_file_name(hwnd: HWND) -> String {
    unsafe { app_ptr(hwnd) }
        .and_then(|app| {
            let active = unsafe { app.as_ref() }.tabs.active()?;
            let stem = title::sanitize_stem(active.untitled_label.as_deref().unwrap_or("Untitled"));
            Some(format!(
                "{stem}.{}",
                title::default_extension(active.language)
            ))
        })
        .unwrap_or_else(|| "Untitled.md".to_owned())
}

/// Ctrl+N, the Notebook view's New note, and "New note here" (`folder`). The new untitled tab
/// remembers where its first save goes: `folder`, else the folder of the sidebar's selected row,
/// else the notebook root. With no notebook open it is a plain new tab.
pub(crate) fn new_note_in(hwnd: HWND, folder: Option<PathBuf>) {
    let destination = self::folder(hwnd).filter(|_| notes_mode(hwnd)).map(|root| {
        folder
            .or_else(|| super::notebook_view::selected_folder(hwnd))
            .filter(|candidate| {
                library::model::same_path(candidate, &root) || library::is_inside(&root, candidate)
            })
            .unwrap_or(root)
    });
    if let Err(error) = super::main_window::create_new_document(hwnd) {
        push_notice(hwnd, format!("FastPad could not create a new tab: {error}"));
        return;
    }
    if let Some(mut app) = unsafe { app_ptr(hwnd) } {
        let app = unsafe { app.as_mut() };
        if let Some(id) = app.tabs.active().map(|document| document.id)
            && let Some(document) = app.tabs.document_mut(id)
        {
            document.save_folder = destination;
        }
    }
    // `create_new_document` already showed the new unsaved row: its tab switch rebuilt the
    // Notebook view's rows and selected it (`side_panel::active_tab_changed`).
}

/// Where tab `id`'s first save goes: its remembered folder while that is still a folder of the
/// open notebook, else the notebook root (spec §6.7).
fn save_folder_for(hwnd: HWND, id: crate::document::DocumentId) -> Option<PathBuf> {
    let root = folder(hwnd)?;
    let remembered = unsafe { app_ptr(hwnd) }.and_then(|app| {
        unsafe { app.as_ref() }
            .tabs
            .document(id)?
            .save_folder
            .clone()
    });
    Some(
        remembered
            .filter(|folder| library::is_inside(&root, folder) && folder.is_dir())
            .unwrap_or(root),
    )
}

/// The active tab's first-save folder, for the name box and the Save As dialog.
pub(crate) fn first_save_folder(hwnd: HWND) -> Option<PathBuf> {
    let id = active_untitled(hwnd)?;
    save_folder_for(hwnd, id)
}

/// Ctrl+S: an untitled tab in notes mode is named in the name box; everything else as before.
pub(crate) fn save_command(hwnd: HWND) {
    if notes_mode(hwnd)
        && folder(hwnd).is_some()
        && let Some(id) = active_untitled(hwnd)
    {
        // Ctrl+S again while the box is open for this tab keeps what was typed.
        if name_box_purpose(hwnd) == Some(NamePurpose::FirstSave(id)) {
            focus_name_box(hwnd);
            return;
        }
        refresh_label(hwnd);
        let suffix = format!(
            "in {}",
            first_save_folder(hwnd)
                .as_deref()
                .map_or_else(|| folder_display_name(hwnd), notebook_name)
        );
        open_name_box(
            hwnd,
            NamePurpose::FirstSave(id),
            &suggested_file_name(hwnd),
            suffix,
            true,
        );
        return;
    }
    let _ = super::main_window::save_active_document(hwnd);
}

pub(crate) fn save_as_command(hwnd: HWND) {
    if notes_mode(hwnd) && folder(hwnd).is_some() && active_untitled(hwnd).is_some() {
        save_command(hwnd);
        return;
    }
    let _ = super::main_window::save_active_document_as(hwnd);
}

fn folder_display_name(hwnd: HWND) -> String {
    folder(hwnd)
        .map(|f| notebook_name(&f))
        .unwrap_or_else(|| "the notebook".to_owned())
}

pub(crate) fn open_name_box(
    hwnd: HWND,
    purpose: NamePurpose,
    text: &str,
    suffix: String,
    browse: bool,
) {
    super::main_window::close_find_bar(hwnd);
    let colors = super::main_window::current_palette(hwnd);
    let shown = unsafe { app_ptr(hwnd) }.is_some_and(|mut app| {
        let app = unsafe { app.as_mut() };
        if app.name_box.is_none() {
            app.name_box = NameBox::create(hwnd).ok();
        }
        match app.name_box.as_mut() {
            Some(name_box) => {
                name_box.show(purpose, text, suffix, browse, colors);
                true
            }
            None => false,
        }
    });
    if !shown {
        push_notice(hwnd, "FastPad could not show the name box.".to_owned());
        return;
    }
    super::main_window::layout_editor_and_find_bar(hwnd);
    focus_name_box(hwnd);
}

fn focus_name_box(hwnd: HWND) {
    if let Some(app) = unsafe { app_ptr(hwnd) }
        && let Some(name_box) = unsafe { app.as_ref() }.name_box.as_ref()
    {
        name_box.focus();
    }
}

/// The purpose of the visible name box, if one is showing.
fn name_box_purpose(hwnd: HWND) -> Option<NamePurpose> {
    unsafe { app_ptr(hwnd) }.and_then(|app| {
        let name_box = unsafe { app.as_ref() }.name_box.as_ref()?;
        if !name_box.is_visible() {
            return None;
        }
        name_box.purpose().cloned()
    })
}

/// Closes a name box whose tab is gone or no longer active, or whose first save already happened.
pub(crate) fn close_stale_name_box(hwnd: HWND) {
    let Some(purpose) = name_box_purpose(hwnd) else {
        return;
    };
    let Some(id) = purpose.document() else {
        return;
    };
    let stale = unsafe { app_ptr(hwnd) }.is_none_or(|app| {
        let tabs = &unsafe { app.as_ref() }.tabs;
        let Some(document) = tabs.document(id) else {
            return true;
        };
        tabs.active().is_none_or(|active| active.id != id)
            || (matches!(purpose, NamePurpose::FirstSave(_)) && document.path.is_some())
    });
    if stale {
        close_name_box(hwnd);
    }
}

pub(crate) fn close_name_box(hwnd: HWND) {
    let hidden = unsafe { app_ptr(hwnd) }.is_some_and(|mut app| {
        unsafe { app.as_mut() }
            .name_box
            .as_mut()
            .is_some_and(|name_box| {
                let was = name_box.is_visible();
                name_box.hide();
                was
            })
    });
    if hidden {
        super::main_window::layout_editor_and_find_bar(hwnd);
        super::main_window::focus_content(hwnd);
    }
}

fn name_box_state(hwnd: HWND) -> Option<(NamePurpose, String)> {
    unsafe { app_ptr(hwnd) }.and_then(|app| {
        let name_box = unsafe { app.as_ref() }.name_box.as_ref()?;
        Some((name_box.purpose()?.clone(), name_box.text()))
    })
}

fn name_box_error(hwnd: HWND, error: String) {
    let stored = unsafe { app_ptr(hwnd) }.is_some_and(|mut app| {
        unsafe { app.as_mut() }
            .name_box
            .as_mut()
            .map(|name_box| name_box.set_error(Some(error)))
            .is_some()
    });
    // The error is wider than the suffix, so the field shrinks to make room.
    if stored {
        super::main_window::layout_editor_and_find_bar(hwnd);
    }
}

/// "<name> already exists. Try <first free name>."
fn name_taken_error(folder: &Path, stem: &str, extension: &str) -> String {
    let free = title::free_name(stem, extension, |candidate| folder.join(candidate).exists());
    format!(
        "{} already exists. Try {free}.",
        title::file_name(stem, extension)
    )
}

/// Enter or Save in the name box.
pub(crate) fn name_box_submit(hwnd: HWND) {
    let Some((purpose, text)) = name_box_state(hwnd) else {
        return;
    };
    match purpose {
        NamePurpose::FirstSave(id) => submit_first_save(hwnd, id, &text),
        NamePurpose::RenameNote(_) if !ready_library(hwnd) => close_name_box(hwnd),
        NamePurpose::RenameNote(id) => submit_rename(hwnd, id, &text),
    }
}

fn submit_first_save(hwnd: HWND, id: crate::document::DocumentId, text: &str) {
    let Some(folder) = save_folder_for(hwnd, id) else {
        return;
    };
    let language = unsafe { app_ptr(hwnd) }
        .and_then(|app| Some(unsafe { app.as_ref() }.tabs.document(id)?.language))
        .unwrap_or(crate::document::Language::Markdown);
    let (stem, extension) = title::split_typed_name(text, title::default_extension(language));
    let name = format!("{stem}.{extension}");
    let target = folder.join(&name);
    if target.exists() {
        name_box_error(hwnd, name_taken_error(&folder, &stem, &extension));
        return;
    }
    if let Err(error) = std::fs::create_dir_all(&folder) {
        name_box_error(
            hwnd,
            format!("FastPad could not create {}: {error}", folder.display()),
        );
        return;
    }
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    if !super::main_window::activate_document_by_id(hwnd, id) {
        return;
    }
    let outcome = super::main_window::complete_first_save(hwnd, &identity, target);
    if !identity.is_live_for(hwnd) {
        return;
    }
    match outcome {
        super::main_window::SaveOutcome::Saved => close_name_box(hwnd),
        // A file took the name after the check above; it is left alone.
        super::main_window::SaveOutcome::NameTaken => {
            name_box_error(hwnd, name_taken_error(&folder, &stem, &extension));
        }
        super::main_window::SaveOutcome::Failed => {}
    }
}

/// Note: Rename...: the name box, prefilled with the file's current name.
pub(crate) fn rename_note(hwnd: HWND) {
    let Some(path) = active_file(hwnd) else {
        return;
    };
    let Some(id) =
        unsafe { app_ptr(hwnd) }.and_then(|app| Some(unsafe { app.as_ref() }.tabs.active()?.id))
    else {
        return;
    };
    // The tab is about to be renamed: a preview would be replaced by the next click in the tree
    // and take the name box with it, so it becomes a normal tab, as the sidebar's F2 makes it.
    promote_tab_for(hwnd, &path);
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    open_name_box(
        hwnd,
        NamePurpose::RenameNote(id),
        &name,
        "Rename".to_owned(),
        false,
    );
}

/// Renames the tab's file in place, never over another file, then follows it in the tab and the
/// library.
fn submit_rename(hwnd: HWND, id: crate::document::DocumentId, text: &str) {
    let Some(old) = unsafe { app_ptr(hwnd) }
        .and_then(|app| unsafe { app.as_ref() }.tabs.document(id)?.path.clone())
    else {
        close_name_box(hwnd);
        return;
    };
    let current_extension = old.extension().map(|e| e.to_string_lossy().into_owned());
    let (stem, extension) = title::split_rename(text, current_extension.as_deref());
    let extension = extension.unwrap_or_default();
    let new = old.with_file_name(title::file_name(&stem, &extension));
    if new == old {
        close_name_box(hwnd);
        return;
    }
    // A change of letter case only names the same file, so it is not a clash.
    let case_only = library::model::same_path(&new, &old);
    let parent = old.parent().map(Path::to_path_buf).unwrap_or_default();
    if new.exists() && !case_only {
        name_box_error(hwnd, name_taken_error(&parent, &stem, &extension));
        return;
    }
    if let Err(error) = crate::platform::files::rename_no_replace(&old, &new) {
        // A file may have taken the name after the check above; it is left alone.
        let error = if new.exists() && !case_only {
            name_taken_error(&parent, &stem, &extension)
        } else {
            format!("FastPad could not rename the file: {error}")
        };
        name_box_error(hwnd, error);
        return;
    }
    let rebound = unsafe { app_ptr(hwnd) }.is_some_and(|mut app| {
        unsafe { app.as_mut() }
            .tabs
            .rebind_path(id, new.clone())
            .is_ok()
    });
    if !rebound {
        // Undo, so the tab and the disk agree.
        let _ = crate::platform::files::rename_no_replace(&new, &old);
        name_box_error(hwnd, "Another tab already has that file open.".to_owned());
        return;
    }
    with_state(hwnd, |state| state.rename_note(&old, &new));
    schedule_write(hwnd);
    close_name_box(hwnd);
    super::main_window::invalidate_title_strip(hwnd);
    super::side_panel::refresh(hwnd);
    // The extension may have changed, and with it the language.
    unsafe {
        PostMessageW(hwnd, crate::window::WM_FASTPAD_APPLY_LANGUAGE, 0, 0);
    }
}

/// The Move to notebook picker's notebooks: favorites by name, then recent ones, never the open
/// notebook and never twice.
fn move_destinations(hwnd: HWND) -> Vec<PathBuf> {
    let open = folder(hwnd);
    let known = known_folders(hwnd, true);
    let elsewhere = |candidate: &PathBuf| {
        !open
            .as_ref()
            .is_some_and(|open| library::model::same_path(open, candidate))
    };
    let favorites: Vec<PathBuf> = known
        .favorites
        .iter()
        .filter(|f| elsewhere(f))
        .cloned()
        .collect();
    let mut named: Vec<(String, PathBuf)> = library::local::display_names(&favorites)
        .into_iter()
        .map(|(name, _)| name)
        .zip(favorites)
        .collect();
    named.sort_by(|a, b| library::tree::natural_cmp(&a.0, &b.0));
    let mut destinations: Vec<PathBuf> = named.into_iter().map(|(_, path)| path).collect();
    for recent in known.folders {
        if elsewhere(&recent)
            && !destinations
                .iter()
                .any(|listed| library::model::same_path(listed, &recent))
        {
            destinations.push(recent);
        }
    }
    destinations
}

/// Note: Move to notebook… (spec §6.6): picks another notebook, whose root receives the file.
pub(crate) fn move_to_notebook(hwnd: HWND, path: &Path) {
    if !folder(hwnd).is_some_and(|root| library::is_inside(&root, path)) {
        push_notice(
            hwnd,
            "Only notes in the open notebook can be moved to another notebook.".to_owned(),
        );
        return;
    }
    let destinations = move_destinations(hwnd);
    let mut items: Vec<String> = library::local::display_names(&destinations)
        .into_iter()
        .map(|(name, hint)| match hint {
            Some(hint) => format!("{name} ({hint})"),
            None => name,
        })
        .collect();
    items.push("Browse…".to_owned());
    host(hwnd, |host| {
        host.shown_move = Some((path.to_path_buf(), destinations));
    });
    super::main_window::open_picker(
        hwnd,
        Picker {
            kind: PickerKind::MoveToNotebook,
            items,
            create: None,
        },
    );
}

/// Whether some open tab already has `path`, by its stored path (which may no longer exist on
/// disk, e.g. after the file it names was deleted or moved outside FastPad).
fn tab_open_for(hwnd: HWND, path: &Path) -> bool {
    unsafe { app_ptr(hwnd) }.is_some_and(|app| {
        unsafe { app.as_ref() }
            .tabs
            .find_stored_path(path)
            .is_some()
    })
}

/// Makes a `rebind_open_tab` failure visible: a tab is left pointing at a path that no longer
/// exists, because another tab already had the new one open. A file moved outside FastPad onto
/// a path another tab has open reaches this through the rescan; `move_note_to` refuses such a
/// move upfront, and `submit_rename` never calls `rebind_open_tab`.
fn report_rebind_failure(hwnd: HWND, old: &Path, new: &Path) {
    push_notice(
        hwnd,
        format!(
            "{} moved to {}, but another tab already has that file open. The tab for {} still shows the old location.",
            title::note_title(old),
            new.display(),
            title::note_title(old)
        ),
    );
}

/// Moves `note` into `destination`'s root. A clash, another tab already at the target, or a
/// failure changes nothing and says so. The pin goes, because pins belong to a notebook, and an
/// open tab follows the file. A cross-volume move that copied but could not delete the source
/// (spec: `MOVEFILE_COPY_ALLOWED`) leaves the source, its pin and its index entry alone: the
/// library state must still match what's on disk.
fn move_note_to(hwnd: HWND, note: &Path, destination: &Path) {
    let Some(file_name) = note.file_name() else {
        return;
    };
    let target = destination.join(file_name);
    let notebook = notebook_name(destination);
    if library::model::same_path(&target, note) {
        push_notice(
            hwnd,
            format!("{} is already in {notebook}.", title::note_title(note)),
        );
        return;
    }
    if target.exists() {
        push_notice(
            hwnd,
            format!(
                "{} already exists in {notebook}. Nothing was moved.",
                file_name.to_string_lossy()
            ),
        );
        return;
    }
    if tab_open_for(hwnd, &target) {
        push_notice(
            hwnd,
            format!(
                "Another tab already has {} open. Nothing was moved.",
                file_name.to_string_lossy()
            ),
        );
        return;
    }
    if !save_before_move(hwnd, note) {
        return;
    }
    if let Err(error) = crate::platform::files::move_file(note, &target) {
        push_notice(
            hwnd,
            format!(
                "FastPad could not move {} to {notebook}: {error}",
                note.display()
            ),
        );
        return;
    }
    if note.exists() {
        // MOVEFILE_COPY_ALLOWED can report success after copying across volumes even when it
        // could not then delete the source (still open elsewhere, read-only, a locked volume).
        // The source is still there, so its pin and index entry still describe it correctly.
        push_notice(
            hwnd,
            format!(
                "{} was copied to {notebook}, but the original could not be removed.",
                title::note_title(note)
            ),
        );
        return;
    }
    let stays_inside = folder(hwnd).is_some_and(|root| library::is_inside(&root, &target));
    if stays_inside {
        with_state(hwnd, |state| state.rename_note(note, &target));
        schedule_write(hwnd);
    } else {
        let record = with_state(hwnd, |state| state.record_for(note).map(|r| r.id)).flatten();
        if let Some(id) = record {
            report(hwnd, apply_op(hwnd, |_, _| Some(PendingOp::Drop { id })));
        }
        with_state(hwnd, |state| state.remove_note(note));
    }
    match rebind_open_tab(hwnd, note, target.clone()) {
        // The tab followed the note out of the notebook: a preview would be replaced by the
        // next click in the tree, so it becomes a normal tab, as a rename makes it.
        Ok(true) => promote_tab_for(hwnd, &target),
        Ok(false) => {}
        Err(()) => report_rebind_failure(hwnd, note, &target),
    }
    push_notice(
        hwnd,
        format!("Moved {} to {notebook}.", title::note_title(&target)),
    );
    super::side_panel::refresh(hwnd);
}

/// Writes the unsaved edits of `note`'s tab, if it has one, so they move with the file. A tab
/// that is not active is saved through a brief switch to it, as `autosave_all` does. False (and
/// a notice) when the save failed: nothing may move then.
fn save_before_move(hwnd: HWND, note: &Path) -> bool {
    let Some((id, active, paused, known)) = unsafe { app_ptr(hwnd) }.and_then(|app| {
        let tabs = &unsafe { app.as_ref() }.tabs;
        let id = tabs.find_stored_path(note)?;
        let document = tabs.document(id)?;
        document.dirty.then(|| {
            (
                id,
                tabs.active().map(|document| document.id),
                document.autosave_paused,
                document.disk_stamp,
            )
        })
    }) else {
        return true;
    };
    // The same guard as autosave: a file changed outside FastPad (or one whose stamp FastPad
    // never knew) is never written over by a move. The user saves or reloads it first.
    let changed = known.is_none() || known != library::disk_stamp(note);
    if paused || changed {
        if changed
            && let Some(mut app) = unsafe { app_ptr(hwnd) }
            && let Some(document) = unsafe { app.as_mut() }.tabs.document_mut(id)
        {
            document.autosave_paused = true;
        }
        push_notice(
            hwnd,
            format!(
                "{} changed on disk. Nothing was moved: save it with Note: Keep my version, or use Note: Reload from disk, then move it.",
                title::note_title(note)
            ),
        );
        return false;
    }
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return false;
    };
    let saved = super::main_window::activate_document_by_id(hwnd, id)
        && super::main_window::complete_autosave(hwnd, &identity);
    if !identity.is_live_for(hwnd) {
        return false;
    }
    if let Some(active) = active.filter(|&active| active != id) {
        super::main_window::activate_document_by_id(hwnd, active);
    }
    if !saved {
        push_notice(
            hwnd,
            format!(
                "FastPad could not save {} before moving it. Nothing was moved.",
                title::note_title(note)
            ),
        );
    }
    saved
}

/// Makes the tab that has `path` open a normal tab if it is the preview.
fn promote_tab_for(hwnd: HWND, path: &Path) {
    let promoted = unsafe { app_ptr(hwnd) }.is_some_and(|mut app| {
        let tabs = &mut unsafe { app.as_mut() }.tabs;
        tabs.find_stored_path(path)
            .is_some_and(|id| tabs.promote(id))
    });
    if promoted {
        super::main_window::invalidate_title_strip(hwnd);
    }
}

/// Note: Reveal in Explorer, and the sidebar's Reveal entries.
pub(crate) fn reveal(hwnd: HWND, path: &Path) {
    if let Err(error) = crate::platform::shell::reveal_in_explorer(path) {
        push_notice(
            hwnd,
            format!(
                "FastPad could not show {} in Explorer: {error}",
                path.display()
            ),
        );
    }
}

/// Rename… from the sidebar. The note opens as a normal tab first, because the name box renames
/// a tab, then the name box opens as for Note: Rename.
pub(crate) fn rename_file(hwnd: HWND, path: &Path) {
    if let Err(error) =
        super::main_window::open_note(hwnd, path, super::main_window::OpenMode::Permanent, false)
    {
        super::main_window::report_open_failure(hwnd, path, &error);
        return;
    }
    rename_note(hwnd);
}

/// Note: Delete, on the active tab's file.
pub(crate) fn delete_note(hwnd: HWND) {
    let Some(path) = active_file(hwnd) else {
        return;
    };
    delete_file(hwnd, &path);
}

/// After a confirm, sends `path` to the Recycle Bin and closes its tab if it has one. Any record
/// stays, flagged deleted and marked missing, so the 30-day purge removes it.
pub(crate) fn delete_file(hwnd: HWND, path: &Path) {
    let tab = unsafe { app_ptr(hwnd) }.and_then(|app| {
        let tabs = &unsafe { app.as_ref() }.tabs;
        let id = tabs.find_stored_path(path)?;
        Some((id, tabs.document(id)?.dirty))
    });
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    // The tab's unsaved edits go with the file: they are not autosaved first.
    let question = if tab.is_some_and(|(_, dirty)| dirty) {
        format!("Move \u{201c}{name}\u{201d} to the Recycle Bin and discard unsaved changes?")
    } else {
        format!("Move \u{201c}{name}\u{201d} to the Recycle Bin?")
    };
    if !confirmed(hwnd, &question) {
        return;
    }
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    // The shell may show its own modal warning (a permanent delete), owned by this window.
    let recycled = crate::platform::files::recycle(hwnd, path);
    if !identity.is_live_for(hwnd) {
        return;
    }
    if let Err(error) = recycled {
        push_notice(
            hwnd,
            format!("FastPad could not delete {}: {error}", path.display()),
        );
        return;
    }
    let now = library::now_unix();
    let record = with_state(hwnd, |state| state.record_for(path).map(|r| r.id)).flatten();
    if let Some(note_id) = record {
        report(
            hwnd,
            apply_op(hwnd, |state, ids| {
                Some(PendingOp::SetDeleted {
                    note: state.note_ref(ids, path),
                    value: true,
                })
            }),
        );
        with_state(hwnd, |state| state.local.set_missing(note_id, now));
    }
    with_state(hwnd, |state| state.remove_note(path));
    if let Some((id, _)) = tab {
        super::main_window::close_document_without_prompt(hwnd, id);
    }
    super::side_panel::refresh(hwnd);
}

/// Browse… in the name box: the system Save As dialog, starting in the folder.
pub(crate) fn name_box_browse(hwnd: HWND) {
    let Some((NamePurpose::FirstSave(id), _)) = name_box_state(hwnd) else {
        return;
    };
    close_name_box(hwnd);
    if super::main_window::activate_document_by_id(hwnd, id) {
        let _ = super::main_window::save_active_document_as(hwnd);
    }
}

fn folder_autosave(hwnd: HWND) -> bool {
    host(hwnd, |host| {
        // Until the folder's state has loaded, its autosave setting is unknown: do not save.
        host.state
            .as_ref()
            .is_some_and(|state| state.local.autosave)
    })
    .unwrap_or(false)
}

/// The active tab's path, if autosave applies to it right now.
fn autosave_target(hwnd: HWND) -> Option<PathBuf> {
    if !notes_mode(hwnd)
        || !folder_autosave(hwnd)
        || super::main_window::file_population_active(hwnd)
    {
        return None;
    }
    let folder = folder(hwnd)?;
    unsafe { app_ptr(hwnd) }.and_then(|app| {
        let active = unsafe { app.as_ref() }.tabs.active()?;
        let path = active.path.clone()?;
        (active.dirty && !active.autosave_paused && library::is_inside(&folder, &path))
            .then_some(path)
    })
}

/// After an edit: (re)starts the idle timer when the active tab would autosave.
pub(crate) fn schedule_autosave(hwnd: HWND) {
    if autosave_target(hwnd).is_some() {
        unsafe {
            SetTimer(hwnd, AUTOSAVE_TIMER_ID, AUTOSAVE_DELAY_MS, None);
        }
    }
}

/// Saves the active tab if autosave applies to it, unless its file changed on disk since FastPad
/// loaded or saved it: then autosave pauses for that tab until the user reloads or keeps theirs.
/// Only a successful write marks the tab clean.
pub(crate) fn autosave_active(hwnd: HWND) -> Autosave {
    unsafe {
        KillTimer(hwnd, AUTOSAVE_TIMER_ID);
    }
    let Some(path) = autosave_target(hwnd) else {
        return Autosave::NotEligible;
    };
    // The guard runs right before the write: nothing between here and `complete_save` yields.
    let known =
        unsafe { app_ptr(hwnd) }.and_then(|app| unsafe { app.as_ref() }.tabs.active()?.disk_stamp);
    let now = library::disk_stamp(&path);
    // No known stamp (a tab restored from a snapshot): FastPad cannot tell whether the file
    // changed, or was deleted on purpose, so it is treated as changed and never re-created.
    let changed = known.is_none() || known != now;
    if changed {
        if let Some(mut app) = unsafe { app_ptr(hwnd) } {
            let app = unsafe { app.as_mut() };
            if let Some(id) = app.tabs.active().map(|document| document.id)
                && let Some(document) = app.tabs.document_mut(id)
            {
                document.autosave_paused = true;
            }
        }
        push_notice(
            hwnd,
            format!(
                "{} changed on disk. Autosave is paused for it: use Note: Reload from disk or Note: Keep my version.",
                title::note_title(&path)
            ),
        );
        return Autosave::Paused;
    }
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return Autosave::Failed;
    };
    // Quiet: on failure the named notice below replaces the generic save-failure one.
    if super::main_window::complete_autosave(hwnd, &identity) {
        Autosave::Saved
    } else {
        if identity.is_live_for(hwnd) {
            push_notice(
                hwnd,
                format!(
                    "Autosave failed for {}. Your text is kept in recovery and FastPad will try again.",
                    title::note_title(&path)
                ),
            );
        }
        Autosave::Failed
    }
}

/// Before the window closes: save every eligible dirty tab. Failures fall back to the prompt.
pub(crate) fn autosave_all(hwnd: HWND) {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    let Some(folder) = folder(hwnd) else {
        return;
    };
    if !notes_mode(hwnd) || !folder_autosave(hwnd) {
        return;
    }
    let (active, ids): (Option<_>, Vec<_>) = unsafe { app_ptr(hwnd) }
        .map(|app| {
            let tabs = &unsafe { app.as_ref() }.tabs;
            let ids = tabs
                .documents()
                .filter(|document| {
                    document.dirty
                        && !document.autosave_paused
                        && document
                            .path
                            .as_deref()
                            .is_some_and(|path| library::is_inside(&folder, path))
                })
                .map(|document| document.id)
                .collect();
            (tabs.active().map(|document| document.id), ids)
        })
        .unwrap_or_default();
    for id in ids {
        if !identity.is_live_for(hwnd) || !super::main_window::activate_document_by_id(hwnd, id) {
            return;
        }
        autosave_active(hwnd);
    }
    // The session records the tab the user had active, not the last one saved.
    if let Some(active) = active
        && identity.is_live_for(hwnd)
    {
        super::main_window::activate_document_by_id(hwnd, active);
    }
}

pub(crate) fn toggle_folder_autosave(hwnd: HWND) {
    let Some(enabled) = with_state(hwnd, |state| {
        state.local.autosave = !state.local.autosave;
        state.local.autosave
    }) else {
        push_notice(hwnd, "Loading notebook…".to_owned());
        return;
    };
    save_local(
        hwnd,
        LocalWrite {
            wait: false,
            force: false,
        },
    );
    if !enabled {
        unsafe {
            KillTimer(hwnd, AUTOSAVE_TIMER_ID);
        }
    }
    push_notice(
        hwnd,
        if enabled {
            "Autosave is on for this notebook.".to_owned()
        } else {
            "Autosave is off for this notebook. Use Ctrl+S to save.".to_owned()
        },
    );
}

/// Replaces the tab's text with the file on disk and resumes autosave.
pub(crate) fn reload_from_disk(hwnd: HWND) {
    let Some(path) = unsafe { app_ptr(hwnd) }
        .and_then(|app| unsafe { app.as_ref() }.tabs.active()?.path.clone())
    else {
        return;
    };
    // Read before the load: a change that lands during it then still pauses the next autosave.
    let stamp = library::disk_stamp(&path);
    let loaded = crate::file::loader::load(&path).and_then(|loaded| {
        // A NUL byte cannot round-trip through Scintilla's UTF-8 buffer: the file is unsupported.
        std::ffi::CString::new(loaded.text.as_str())
            .map_err(|_| crate::FastPadError::UnsupportedEncoding)?;
        Ok(loaded)
    });
    let loaded = match loaded {
        Ok(loaded) => loaded,
        Err(error) => {
            super::main_window::report_open_failure(hwnd, &path, &error);
            return;
        }
    };
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    let Some(editor) =
        unsafe { app_ptr(hwnd) }.and_then(|app| unsafe { app.as_ref() }.editor.clone())
    else {
        return;
    };
    if editor.set_text(&loaded.text).is_err() || !identity.is_live_for(hwnd) {
        return;
    }
    editor.set_save_point();
    if let Some(mut app) = unsafe { app_ptr(hwnd) } {
        let app = unsafe { app.as_mut() };
        app.tabs.set_active_dirty(false);
        if let Some(id) = app.tabs.active().map(|document| document.id)
            && let Some(document) = app.tabs.document_mut(id)
        {
            document.encoding = loaded.encoding;
            document.disk_stamp = stamp;
            document.autosave_paused = false;
        }
    }
    super::main_window::remove_saved_document_snapshots(hwnd);
    super::main_window::invalidate_title_strip(hwnd);
}

/// Saves the tab over the changed file and resumes autosave.
pub(crate) fn keep_mine(hwnd: HWND) {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    // An untitled tab has no file on disk to keep its text over.
    let has_path = unsafe { app_ptr(hwnd) }.is_some_and(|app| {
        unsafe { app.as_ref() }
            .tabs
            .active()
            .is_some_and(|d| d.path.is_some())
    });
    if !has_path {
        return;
    }
    let _ = super::main_window::complete_save(hwnd, &identity, None);
}

/// After a file is opened into a tab: remember its disk stamp, read before the load so a change
/// landing during it still pauses the next autosave.
pub(crate) fn document_loaded(hwnd: HWND, stamp: Option<library::DiskStamp>) {
    if let Some(mut app) = unsafe { app_ptr(hwnd) } {
        let app = unsafe { app.as_mut() };
        if let Some(id) = app.tabs.active().map(|document| document.id)
            && let Some(document) = app.tabs.document_mut(id)
            && document.path.is_some()
        {
            document.disk_stamp = stamp;
        }
    }
}

/// After any successful save: remember the file's disk stamp and index it if it is a note. The
/// sidebar is rebuilt only when the save changed its rows: a new note in the index, or an
/// untitled tab that is no longer untitled. A save of a note already listed leaves the rows,
/// and the Search view's selection, alone.
pub(crate) fn document_saved(hwnd: HWND) {
    let saved = unsafe { app_ptr(hwnd) }.and_then(|mut app| {
        let app = unsafe { app.as_mut() };
        let id = app.tabs.active()?.id;
        // A saved preview is kept: the next click must not replace it.
        let promoted = app.tabs.promote(id);
        let document = app.tabs.document_mut(id)?;
        let path = document.path.clone();
        if let Some(path) = &path {
            document.disk_stamp = library::disk_stamp(path);
            document.autosave_paused = false;
        }
        Some((path, promoted))
    });
    let (path, promoted) = saved.unwrap_or((None, false));
    let inserted = path
        .and_then(|path| with_state(hwnd, |state| state.add_note(&path)))
        .unwrap_or(false);
    close_stale_name_box(hwnd);
    if inserted || super::notebook_view::stale(hwnd) {
        super::side_panel::refresh(hwnd);
    }
    if promoted {
        // The tab's name is no longer italic.
        super::main_window::invalidate_title_strip(hwnd);
    }
}

#[cfg(test)]
pub(crate) fn install_for_test(hwnd: HWND, state: LibraryState) {
    host(hwnd, |host| {
        host.folder = Some(state.folder.clone());
        host.generation = host.generation.wrapping_add(1);
    });
    install(hwnd, state);
}

pub(crate) fn notes_mode_notice(enabled: bool) -> &'static str {
    if enabled {
        "Notes mode is on. The open notebook is your note library."
    } else {
        "Notes mode is off. FastPad works as a plain file editor."
    }
}

const READ_ONLY: &str = "This notebook's .fastpad\\library.ini can't be read, so pins are off until it is fixed or removed.";
const BUSY: &str = "This notebook's .fastpad\\library.ini is in use by another program. FastPad reads it again when you come back to the window.";

/// True when organizing can proceed; otherwise explains why not.
pub(crate) fn ready_library(hwnd: HWND) -> bool {
    match host(hwnd, |host| host.state.as_ref().map(|state| state.metadata)).flatten() {
        Some(Metadata::Ready) => true,
        Some(Metadata::Unreadable) => {
            push_notice(hwnd, READ_ONLY.to_owned());
            false
        }
        Some(Metadata::Busy) => {
            push_notice(hwnd, BUSY.to_owned());
            false
        }
        None => {
            let notice = if folder(hwnd).is_some() {
                "Loading notebook…"
            } else {
                "Open a notebook first."
            };
            push_notice(hwnd, notice.to_owned());
            false
        }
    }
}

/// The active tab's file; for an untitled tab, explains that it must be saved first.
pub(crate) fn active_file(hwnd: HWND) -> Option<PathBuf> {
    let path = unsafe { app_ptr(hwnd) }
        .and_then(|app| unsafe { app.as_ref() }.tabs.active()?.path.clone());
    if path.is_none() {
        push_notice(hwnd, "Save this note first to organize it.".to_owned());
    }
    path
}

/// Applies one operation built from the host's ID source, then schedules the write. The model
/// validates before the operation is recorded, so a failed one leaves nothing pending.
fn apply_op(
    hwnd: HWND,
    build: impl FnOnce(&mut LibraryState, &mut IdSource) -> Option<PendingOp>,
) -> Result<(), LibraryError> {
    let result = host(hwnd, |host| {
        let LibraryHost { state, ids, .. } = host;
        let state = state.as_mut()?;
        let op = build(state, ids)?;
        Some(state.apply(op))
    })
    .flatten()
    .unwrap_or(Ok(()));
    if result.is_ok() {
        schedule_write(hwnd);
    }
    result
}

fn report(hwnd: HWND, result: Result<(), LibraryError>) {
    if let Err(error) = result {
        push_notice(hwnd, error.to_string());
    }
}

/// Asks `question`; false also when the window went away meanwhile.
fn confirmed(hwnd: HWND, question: &str) -> bool {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return false;
    };
    crate::window::modal::confirm(hwnd, question) && identity.is_live_for(hwnd)
}

/// Pins or unpins `path`. Only a note inside the open notebook can be pinned: version 2 of
/// `library.ini` has no records for files outside it.
pub(crate) fn toggle_pin(hwnd: HWND, path: &Path) {
    if !ready_library(hwnd) {
        return;
    }
    if !folder(hwnd).is_some_and(|folder| library::is_inside(&folder, path)) {
        push_notice(
            hwnd,
            "Only notes in the open notebook can be pinned.".to_owned(),
        );
        return;
    }
    let mut now_on = false;
    let result = apply_op(hwnd, |state, ids| {
        now_on = !state.is_pinned(path);
        Some(PendingOp::SetPinned {
            note: state.note_ref(ids, path),
            value: now_on,
        })
    });
    if result.is_err() {
        report(hwnd, result);
        return;
    }
    push_notice(
        hwnd,
        if now_on { "Pinned." } else { "Unpinned." }.to_owned(),
    );
    super::side_panel::refresh(hwnd);
}

#[cfg(test)]
thread_local! {
    static LAST_PICK: std::cell::RefCell<Option<(PickerKind, PickerChoice)>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(crate) fn take_last_pick() -> Option<(PickerKind, PickerChoice)> {
    LAST_PICK.with(|last| last.borrow_mut().take())
}

/// A picker row was chosen. Every kind resolves the row against what that picker showed.
pub(crate) fn picked(hwnd: HWND, kind: PickerKind, choice: PickerChoice) {
    #[cfg(test)]
    LAST_PICK.with(|last| *last.borrow_mut() = Some((kind, choice.clone())));
    match (kind, choice) {
        (PickerKind::RecentFolder, PickerChoice::Item(index)) => {
            let shown = host(hwnd, |host| std::mem::take(&mut host.shown_recent_folders));
            if let Some(folder) = shown.unwrap_or_default().get(index) {
                open_listed_notebook(hwnd, folder);
            }
        }
        (PickerKind::MoveToNotebook, PickerChoice::Item(index)) => {
            let Some((note, destinations)) = host(hwnd, |host| host.shown_move.take()).flatten()
            else {
                return;
            };
            let destination = match destinations.get(index) {
                Some(folder) => folder.clone(),
                // The row after the notebooks is "Browse…".
                None if index == destinations.len() => {
                    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
                        return;
                    };
                    let choice = crate::window::modal::choose_folder(hwnd);
                    if !identity.is_live_for(hwnd) {
                        return;
                    }
                    match choice {
                        Ok(Some(folder)) => library::normalize_folder(&folder),
                        Ok(None) => return,
                        Err(error) => {
                            push_notice(
                                hwnd,
                                format!("FastPad could not open the folder picker: {error}"),
                            );
                            return;
                        }
                    }
                }
                None => return,
            };
            move_note_to(hwnd, &note, &destination);
        }
        (PickerKind::QuickOpen, PickerChoice::Note { path, line }) => {
            super::main_window::open_quick_open_choice(hwnd, &path, line);
        }
        (PickerKind::QuickOpen, PickerChoice::GoToLine(line)) => {
            super::main_window::go_to_line(hwnd, line);
            // A note pick focuses the editor (`open_note(.., true)`); a `:n` pick moves the
            // caret the same way, so it must land keyboard focus there too, even when the pick
            // was made with the sidebar focused.
            super::main_window::focus_content(hwnd);
        }
        _ => {}
    }
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
}
