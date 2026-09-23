//! Window wiring for the note library: the deferred load, rescans, debounced metadata writes,
//! folder commands, first-save naming, autosave, and the organizing commands.

use super::main_window::{app_ptr, push_notice};
use crate::library::{self, LibraryState, Metadata, ids::IdSource};
use crate::window::command_palette::{PickerChoice, PickerKind};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{HWND, LPARAM};
use windows_sys::Win32::UI::WindowsAndMessaging::{KillTimer, PostMessageW, SetTimer};

pub(crate) const LIBRARY_WRITE_TIMER_ID: usize = 0x4650_4C57;
pub(crate) const RESCAN_AFTER: Duration = Duration::from_secs(5);
const WRITE_DELAY_MS: u32 = 500;

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
    pub(crate) inactive_since: Option<Instant>,
    #[expect(
        dead_code,
        reason = "IDs for new records; the organizing commands of later tasks read it"
    )]
    pub(crate) ids: IdSource,
    notified: Option<PathBuf>,
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
            inactive_since: None,
            ids: IdSource::new(process_start, std::process::id()),
            notified: None,
        }
    }
}

struct Loaded {
    generation: u64,
    folder: PathBuf,
    result: Result<LibraryState, String>,
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

/// The folder to open at startup: a directory named on the command line, else the most recent
/// folder that still exists, else `Documents\FastPad`.
fn startup_folder(hwnd: HWND, data: &Path) -> Option<PathBuf> {
    let launch_dir =
        unsafe { app_ptr(hwnd) }.and_then(|app| match &unsafe { app.as_ref() }.launch.request {
            crate::launch::LaunchRequest::Open(path) => {
                let path = std::path::absolute(path).ok()?;
                path.is_dir().then_some(path)
            }
            crate::launch::LaunchRequest::New => None,
        });
    if let Some(path) = launch_dir {
        remember_folder(data, &path);
        return Some(path);
    }
    let recent = library::local::read_folders(&library::local::folders_file(data));
    if let Some(first) = recent.folders.first() {
        if first.is_dir() {
            return Some(first.clone());
        }
        push_notice(
            hwnd,
            format!(
                "FastPad could not find the folder {}. Using Documents\\FastPad instead.",
                first.display()
            ),
        );
    }
    crate::platform::paths::default_notes_folder().ok()
}

pub(crate) fn remember_folder(data: &Path, folder: &Path) {
    let path = library::local::folders_file(data);
    let mut recent = library::local::read_folders(&path);
    recent.push(folder.to_path_buf());
    let _ = library::local::write_folders(&path, &recent);
}

/// `WM_FASTPAD_OPEN_LIBRARY`: picks the folder and starts the worker. Reads only `folders.ini`.
pub(crate) fn open_library_step(hwnd: HWND) {
    if !notes_mode(hwnd) {
        return;
    }
    let Some(data) = data_dir(hwnd) else {
        return;
    };
    let Some(folder) = startup_folder(hwnd, &data) else {
        return;
    };
    host(hwnd, |host| host.folder = Some(folder));
    start_load(hwnd);
}

pub(crate) fn start_load(hwnd: HWND) {
    let Some(data) = data_dir(hwnd) else {
        return;
    };
    let Some((folder, generation)) = host(hwnd, |host| {
        let folder = host.folder.clone()?;
        host.generation = host.generation.wrapping_add(1);
        host.scanning = true;
        host.rescan_requested = false;
        Some((folder, host.generation))
    })
    .flatten() else {
        return;
    };
    let local_path = library::local::local_file(&data, &folder);
    let target = hwnd as isize;
    std::thread::spawn(move || {
        let result = library::load(&folder, &local_path, library::now_unix())
            .map_err(|error| error.to_string());
        let payload = Box::into_raw(Box::new(Loaded {
            generation,
            folder,
            result,
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
    let Loaded { folder, result, .. } = *loaded;
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
        push_notice(hwnd, "This folder's .fastpad\\library.ini is damaged or from a newer FastPad, so notebooks and tags are read-only.".to_owned());
    }
    if has_pending {
        schedule_write(hwnd);
    }
    with_state(hwnd, |state| library::write_local(state));
}

/// A note moved outside FastPad: an open tab for it follows the file.
fn rebind_open_tab(hwnd: HWND, old: &Path, new: PathBuf) {
    let changed = unsafe { app_ptr(hwnd) }.is_some_and(|mut app| {
        let app = unsafe { app.as_mut() };
        match app.tabs.find_path(old) {
            Some(id) => app.tabs.rebind_path(id, new).is_ok(),
            None => false,
        }
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

/// Writes pending metadata now (timer, folder switch, close).
pub(crate) fn flush_now(hwnd: HWND) {
    unsafe {
        KillTimer(hwnd, LIBRARY_WRITE_TIMER_ID);
    }
    let Some(result) = with_state(hwnd, |state| {
        let result = library::flush(state);
        library::write_local(state);
        result
    }) else {
        return;
    };
    if let Err(error) = result {
        push_notice(
            hwnd,
            format!("FastPad could not save this folder's notebooks and tags: {error}"),
        );
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
    flush_now(hwnd);
    host(hwnd, |host| {
        host.state = None;
        host.folder = None;
        host.generation = host.generation.wrapping_add(1);
        host.scanning = false;
    });
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

#[cfg(test)]
thread_local! {
    static LAST_PICK: std::cell::RefCell<Option<(PickerKind, PickerChoice)>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(crate) fn take_last_pick() -> Option<(PickerKind, PickerChoice)> {
    LAST_PICK.with(|last| last.borrow_mut().take())
}

/// A picker row was chosen. Tasks 15 and 19 add one arm per kind.
pub(crate) fn picked(hwnd: HWND, kind: PickerKind, choice: PickerChoice) {
    #[cfg(test)]
    LAST_PICK.with(|last| *last.borrow_mut() = Some((kind, choice.clone())));
    let _ = (hwnd, kind, choice);
}
