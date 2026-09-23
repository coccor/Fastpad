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
            "FastPad could not find the folder {}. Using Documents\\FastPad instead.",
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
    if recent.closed && launch.is_none() {
        host(hwnd, |host| host.folder = None);
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
        let closed = startup.as_ref().is_some_and(|startup| startup.closed);
        let (folder, notice) = match startup {
            Some(startup) => resolve_startup(startup),
            None => (folder, None),
        };
        let opens_nothing = closed && folder.is_none();
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
        ..
    } = *loaded;
    if let Some(notice) = notice {
        push_notice(hwnd, notice);
    }
    if closed {
        // The path on the command line was not a folder, and the last session closed its
        // notebook: none is open.
        host(hwnd, |host| host.folder = None);
        super::main_window::invalidate_title_strip(hwnd);
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
        Err(error) => push_notice(
            hwnd,
            format!(
                "FastPad could not load the folder {}: {error}",
                folder.display()
            ),
        ),
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
        rebind_open_tab(hwnd, &folder.join(old), folder.join(new));
    }
    if first_time && truncated {
        push_notice(
            hwnd,
            "This folder has more than 10,000 notes. FastPad indexed the first 10,000.".to_owned(),
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

/// A note moved outside FastPad: an open tab for it follows the file. The old path is gone, so
/// the tab is found by its stored path, not through the disk. A move leaves the content alone,
/// so when the new file's stamp is the one the tab knows, autosave carries on (or resumes, if it
/// paused when the old file vanished); otherwise the tab keeps what it knew, and its next
/// autosave pauses on the changed file.
fn rebind_open_tab(hwnd: HWND, old: &Path, new: PathBuf) {
    let stamp = library::disk_stamp(&new);
    let changed = unsafe { app_ptr(hwnd) }.is_some_and(|mut app| {
        let app = unsafe { app.as_mut() };
        let Some(id) = app.tabs.find_stored_path(old) else {
            return false;
        };
        if app.tabs.rebind_path(id, new).is_err() {
            return false;
        }
        if let Some(document) = app.tabs.document_mut(id)
            && document.disk_stamp.is_some()
            && document.disk_stamp == stamp
        {
            document.autosave_paused = false;
        }
        true
    });
    if changed {
        super::main_window::invalidate_title_strip(hwnd);
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
            format!("FastPad could not save this folder's pins: {error}"),
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

/// Opens `path` as the library, flushing the current one first. Open tabs stay open.
pub(crate) fn open_folder(hwnd: HWND, path: &Path) {
    if !notes_mode(hwnd) {
        push_notice(
            hwnd,
            "Notes mode is off. Turn it on with Notes: Toggle notes mode to open folders."
                .to_owned(),
        );
        return;
    }
    let path = library::normalize_folder(path);
    if !path.is_dir() {
        push_notice(hwnd, format!("{} is not a folder.", path.display()));
        return;
    }
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    // The old folder's dirty notes are saved while autosave still applies to them.
    autosave_all(hwnd);
    if !identity.is_live_for(hwnd) {
        return;
    }
    // Switching would drop the unsaved pins, so a failed write keeps the old folder.
    let failure = match try_flush(hwnd, false) {
        Ok(library::Flushed::Busy) => Some("its library.ini is in use by another program".into()),
        Ok(_) => None,
        Err(error) => Some(error.to_string()),
    };
    if let Some(error) = failure {
        schedule_write(hwnd);
        push_notice(
            hwnd,
            format!("FastPad kept this folder open because it could not save its pins: {error}"),
        );
        return;
    }
    host(hwnd, |host| {
        host.state = None;
        host.folder = Some(path.clone());
    });
    if let Some(data) = data_dir(hwnd) {
        remember_folder(&data, &path);
    }
    start_load(hwnd);
    super::main_window::invalidate_title_strip(hwnd);
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

fn recent_folders(hwnd: HWND) -> Vec<PathBuf> {
    data_dir(hwnd)
        .map(|data| library::local::read_folders(&library::local::folders_file(&data)).folders)
        .unwrap_or_default()
}

pub(crate) fn open_recent_folder_picker(hwnd: HWND) {
    let folders = recent_folders(hwnd);
    if folders.is_empty() {
        push_notice(
            hwnd,
            "No recent folders yet. Use File: Open folder.".to_owned(),
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
    let changed = unsafe { app_ptr(hwnd) }.is_some_and(|mut app| {
        let app = unsafe { app.as_mut() };
        let Some(editor) = app.editor.as_ref() else {
            return false;
        };
        let Some(active) = app.tabs.active() else {
            return false;
        };
        if active.path.is_some() {
            return false;
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
        let Some(document) = app.tabs.document_mut(id) else {
            return false;
        };
        document.label_watch = label.watch_through;
        document.first_line_label = label.text;
        if !shown || document.untitled_label == document.first_line_label {
            return false;
        }
        document.untitled_label = document.first_line_label.clone();
        true
    });
    if changed {
        super::main_window::refresh_tab_view(hwnd);
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
        let suffix = format!("in {}", folder_display_name(hwnd));
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
        .and_then(|f| f.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "the notes folder".to_owned())
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
    let Some(folder) = folder(hwnd) else {
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
    // The extension may have changed, and with it the language.
    unsafe {
        PostMessageW(hwnd, crate::window::WM_FASTPAD_APPLY_LANGUAGE, 0, 0);
    }
}

/// Note: Delete: after a confirm, sends the file to the Recycle Bin and closes its tab. Any
/// record stays, flagged deleted and marked missing, so the 30-day purge removes it.
pub(crate) fn delete_note(hwnd: HWND) {
    let Some(path) = active_file(hwnd) else {
        return;
    };
    let Some((id, dirty)) = unsafe { app_ptr(hwnd) }.and_then(|app| {
        let active = unsafe { app.as_ref() }.tabs.active()?;
        Some((active.id, active.dirty))
    }) else {
        return;
    };
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    // The tab's unsaved edits go with the file: they are not autosaved first.
    let question = if dirty {
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
    let recycled = crate::platform::files::recycle(hwnd, &path);
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
    let record = with_state(hwnd, |state| state.record_for(&path).map(|r| r.id)).flatten();
    if let Some(note_id) = record {
        report(
            hwnd,
            apply_op(hwnd, |state, ids| {
                Some(PendingOp::SetDeleted {
                    note: state.note_ref(ids, &path),
                    value: true,
                })
            }),
        );
        with_state(hwnd, |state| state.local.set_missing(note_id, now));
    }
    with_state(hwnd, |state| state.remove_note(&path));
    super::main_window::close_document_without_prompt(hwnd, id);
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
        push_notice(hwnd, "Loading folder…".to_owned());
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
            "Autosave is on for this folder.".to_owned()
        } else {
            "Autosave is off for this folder. Use Ctrl+S to save.".to_owned()
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

/// After any successful save: remember the file's disk stamp and index it if it is a note.
pub(crate) fn document_saved(hwnd: HWND) {
    let path = unsafe { app_ptr(hwnd) }.and_then(|mut app| {
        let app = unsafe { app.as_mut() };
        let id = app.tabs.active()?.id;
        let document = app.tabs.document_mut(id)?;
        let path = document.path.clone()?;
        document.disk_stamp = library::disk_stamp(&path);
        document.autosave_paused = false;
        Some(path)
    });
    if let Some(path) = path {
        with_state(hwnd, |state| state.add_note(&path));
    }
    close_stale_name_box(hwnd);
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
        "Notes mode is on. The open folder is your note library."
    } else {
        "Notes mode is off. FastPad works as a plain file editor."
    }
}

const READ_ONLY: &str = "This folder's .fastpad\\library.ini is damaged or from a newer FastPad, so pins are read-only.";
const BUSY: &str = "This folder's .fastpad\\library.ini is in use by another program. FastPad reads it again when you come back to the window.";

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
                "Loading folder…"
            } else {
                "Open a folder first."
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

/// A picker row was chosen. The recent-folder picker is the only one; its row opens the folder
/// that row showed, even if `folders.ini` changed meanwhile.
pub(crate) fn picked(hwnd: HWND, kind: PickerKind, choice: PickerChoice) {
    #[cfg(test)]
    LAST_PICK.with(|last| *last.borrow_mut() = Some((kind, choice.clone())));
    let (PickerKind::RecentFolder, PickerChoice::Item(index)) = (kind, choice) else {
        return;
    };
    let shown = host(hwnd, |host| std::mem::take(&mut host.shown_recent_folders));
    if let Some(folder) = shown.unwrap_or_default().get(index) {
        open_folder(hwnd, folder);
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
