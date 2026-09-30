//! Loading the notebook: resolving the startup folder on a worker, the deferred first load,
//! Retry after a failed load, and installing a loaded `LibraryState` into the window.

use super::*;
use crate::library::{self, LibraryState, Metadata};
use crate::window::main_window::{app_ptr, push_notice};
use std::path::{Path, PathBuf};
use windows_sys::Win32::Foundation::{HWND, LPARAM};
use windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW;

/// On the worker: the folder to open at startup, a directory named on the command line, else
/// nothing when the last session closed its notebook, else the most recent folder if it still
/// exists, else `Documents\FastPad`; and a notice when the most recent folder is gone.
pub(super) fn resolve_startup(startup: Startup) -> (Option<PathBuf>, Option<String>) {
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
        crate::window::side_panel::refresh(hwnd);
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
    crate::window::side_panel::refresh(hwnd);
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
    crate::window::side_panel::refresh(hwnd);
}

pub(super) fn spawn_load(hwnd: HWND, startup: Option<Startup>) {
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
            state.folder_changes.clear();
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
        crate::window::main_window::invalidate_title_strip(hwnd);
        crate::window::side_panel::refresh(hwnd);
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
        crate::window::main_window::invalidate_title_strip(hwnd);
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
            crate::window::side_panel::refresh(hwnd);
        }
    }
    if host(hwnd, |host| std::mem::take(&mut host.rescan_requested)).unwrap_or(false) {
        start_load(hwnd);
    }
}

pub(super) fn install(hwnd: HWND, fresh: LibraryState) {
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
    // A folder box whose folder the rescan no longer finds closes.
    close_stale_name_box(hwnd);
    crate::window::side_panel::refresh(hwnd);
    // A notebook's first load reveals the (restored) active note: its row is selected and its
    // folders expand (spec §6.1). A rescan leaves the user's selection where it was.
    if first_time {
        crate::window::side_panel::active_tab_changed(hwnd);
    }
}
