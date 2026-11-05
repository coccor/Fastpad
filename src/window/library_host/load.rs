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
            crate::launch::LaunchRequest::New | crate::launch::LaunchRequest::Capture(_) => None,
        });
    let recent = library::local::read_folders(&library::local::folders_file(&data));
    host(hwnd, |host| host.folders = Some(recent.clone()));
    if recent.closed && launch.is_none() {
        host(hwnd, |host| host.startup_load = None);
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
    // The assumed notebook gets its slot now; with none assumed, the worker's answer opens one.
    let target = host(hwnd, |host| match (host.default_notebook, assumed) {
        (Some(id), Some(assumed)) => {
            if let Some(slot) = host.slot_mut(id) {
                slot.folder = assumed;
            }
            Some(id)
        }
        (_, Some(assumed)) => Some(host.add_notebook(assumed)),
        (id, None) => id,
    })
    .flatten();
    // The sidebar says "Loading…" for the assumed notebook until the worker answers, not "Open a
    // notebook" (a refresh earlier in startup saw no folder). No flattening: nothing is loaded.
    crate::window::side_panel::refresh(hwnd);
    spawn_load(
        hwnd,
        target,
        Some(Startup {
            launch,
            remembered,
            closed: recent.closed,
            data,
        }),
    );
}

/// Loads (or rescans) the notebook `id`.
pub(crate) fn start_load_of(hwnd: HWND, id: NotebookId) {
    spawn_load(hwnd, Some(id), None);
}

/// Whether the default notebook failed to load and has no state to show.
pub(crate) fn load_failed(hwnd: HWND) -> bool {
    default_notebook(hwnd).is_some_and(|id| load_failed_of(hwnd, id))
}

/// Whether the notebook `id` failed to load and has no state to show: the sidebar says so and
/// offers Retry. A rescan that fails keeps the state it had, so this stays false then.
pub(crate) fn load_failed_of(hwnd: HWND, id: NotebookId) -> bool {
    slot(hwnd, id, |slot| slot.load_failed && slot.state.is_none()).unwrap_or(false)
}

/// The sidebar's Retry for the default notebook.
pub(crate) fn retry_load(hwnd: HWND) {
    if let Some(id) = default_notebook(hwnd) {
        retry_load_of(hwnd, id);
    }
}

/// The sidebar's Retry after a failed load: loads the same notebook again, showing "Loading…"
/// meanwhile.
pub(crate) fn retry_load_of(hwnd: HWND, id: NotebookId) {
    if slot(hwnd, id, |slot| slot.scanning).unwrap_or(true) {
        return;
    }
    start_load_of(hwnd, id);
    crate::window::side_panel::refresh(hwnd);
}

/// Starts the load worker for the notebook `id`, or, at startup with no notebook assumed, for
/// whichever folder the worker settles on.
pub(super) fn spawn_load(hwnd: HWND, id: Option<NotebookId>, startup: Option<Startup>) {
    let Some(data) = data_dir(hwnd) else {
        return;
    };
    let Some((folder, generation)) = host(hwnd, |host| {
        let is_startup = startup.is_some();
        if id.is_none_or(|id| host.slot_mut(id).is_none()) {
            if !is_startup {
                return None;
            }
            let generation = host.fresh_generation();
            host.startup_load = Some(generation);
            return Some((None, generation));
        }
        let generation = host.fresh_generation();
        if is_startup {
            host.startup_load = None;
        }
        let slot = host.slot_mut(id?)?;
        slot.generation = generation;
        slot.scanning = true;
        slot.rescan_requested = false;
        slot.load_failed = false;
        slot.local_written_during_scan = false;
        // The merge re-checks only what FastPad changes in the index from here on.
        if let Some(state) = slot.state.as_mut() {
            state.touched.clear();
            state.folder_changes.clear();
        }
        Some((Some(slot.folder.clone()), generation))
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
    // The slot the load was for, or `None` for a startup load that opened no slot yet. A result
    // for a notebook since closed or reloaded matches neither and is dropped.
    let Some(mut id) = host(hwnd, |host| {
        if let Some(id) = host.slot_by_load_generation(loaded.generation) {
            if let Some(slot) = host.slot_mut(id) {
                slot.scanning = false;
            }
            return Some(Some(id));
        }
        if host.startup_load == Some(loaded.generation) {
            host.startup_load = None;
            return Some(None);
        }
        None
    })
    .flatten() else {
        return;
    };
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
        if let Some(id) = id {
            host(hwnd, |host| host.remove_notebook(id));
        }
        crate::window::main_window::invalidate_title_strip(hwnd);
        crate::window::side_panel::refresh(hwnd);
        return;
    }
    // At startup the worker decides which folder exists; the UI thread only assumed one.
    let moved = !folder.as_os_str().is_empty()
        && host(hwnd, |host| match id.and_then(|id| host.slot_mut(id)) {
            Some(slot) => {
                let moved = !library::model::same_path(&slot.folder, &folder);
                if moved {
                    slot.folder = folder.clone();
                }
                moved
            }
            // Nothing was assumed: the worker's folder opens now.
            None => {
                id = Some(host.add_notebook(folder.clone()));
                true
            }
        })
        .unwrap_or(false);
    if moved {
        crate::window::main_window::invalidate_title_strip(hwnd);
    }
    match result {
        Ok(fresh) => {
            if let Some(id) = id {
                install_of(hwnd, id, fresh);
            }
        }
        Err(error) => {
            if let Some(id) = id {
                slot(hwnd, id, |slot| slot.load_failed = true);
            }
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
    if let Some(id) = id
        && slot(hwnd, id, |slot| std::mem::take(&mut slot.rescan_requested)).unwrap_or(false)
    {
        start_load_of(hwnd, id);
    }
}

/// Installs a loaded state into the default notebook.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "loads install into the slot they were for")
)]
pub(super) fn install(hwnd: HWND, fresh: LibraryState) {
    if let Some(id) = default_notebook(hwnd) {
        install_of(hwnd, id, fresh);
    }
}

/// Installs a loaded state into the notebook `id`, merging it into the state it had.
pub(super) fn install_of(hwnd: HWND, id: NotebookId, fresh: LibraryState) {
    let Some((relocated, first_time, truncated, unreadable, has_pending)) =
        slot(hwnd, id, |slot| {
            let mut state = match slot.state.take() {
                Some(previous) if library::model::same_path(&previous.folder, &fresh.folder) => {
                    library::merge_rescan(previous, fresh)
                }
                _ => fresh,
            };
            let first_time = !slot
                .notified
                .as_ref()
                .is_some_and(|f| library::model::same_path(f, &state.folder));
            slot.notified = Some(state.folder.clone());
            if !state.renamed_outside.is_empty() {
                let found = std::mem::take(&mut state.renamed_outside);
                slot.renamed_outside.extend(found);
                slot.renamed_outside_told = false;
            }
            let state = slot.state.insert(state);
            (
                std::mem::take(&mut state.relocated),
                first_time,
                state.truncated,
                state.metadata == Metadata::Unreadable,
                !state.pending.is_empty(),
            )
        })
    else {
        return;
    };
    let folder = notebook_folder(hwnd, id).unwrap_or_default();
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
    let rewrite = slot(hwnd, id, |slot| {
        std::mem::take(&mut slot.local_written_during_scan)
    })
    .unwrap_or(false);
    save_local_of(
        hwnd,
        id,
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
    // A note the rescan found changed is no news when a tab shows it.
    clear_visible_marks_of(hwnd, id);
    crate::window::side_panel::refresh(hwnd);
    // A notebook's first load reveals the (restored) active note: its row is selected and its
    // folders expand (spec §6.1). A rescan leaves the user's selection where it was.
    if first_time {
        crate::window::side_panel::active_tab_changed(hwnd);
    }
    // From now on outside changes are reported as they happen (second brain phase 1 §5.1).
    ensure_watcher_of(hwnd, id);
    // The notebook's conventions may have changed with it (second brain phase 1 §9.1).
    reload_notebook_settings_of(hwnd, id);
    // The note list may have changed: the index reads what did (§6.4).
    request_index_of(hwnd, id);
}
