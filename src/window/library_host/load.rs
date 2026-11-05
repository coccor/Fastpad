//! Loading the notebook: resolving the startup folder on a worker, the deferred first load,
//! Retry after a failed load, and installing a loaded `LibraryState` into the window.

use super::*;
use crate::library::{self, LibraryState, Metadata};
use crate::window::main_window::{app_ptr, push_notice};
use std::path::{Path, PathBuf};
use windows_sys::Win32::Foundation::{HWND, LPARAM};
use windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW;

/// What the startup worker found.
pub(super) struct StartupRoots {
    /// The notebooks to open, in order: the remembered ones that exist, then the folder named on
    /// the command line if it exists and is not one of them. A remembered notebook that is
    /// missing is left out here, but stays on its `open=` line: nothing rewrites it.
    pub(super) roots: Vec<PathBuf>,
    /// The default notebook: the remembered one if it opened, else the first root that did.
    pub(super) default: Option<PathBuf>,
    /// "X is not available." for the remembered notebooks that are missing.
    pub(super) notice: Option<String>,
}

/// On the worker: checks every remembered notebook (each on its own thread, so one offline drive
/// delays only itself) and the folder named on the command line. `Documents\FastPad` opens only
/// when nothing is remembered, the last session did not close its notebook and no folder was
/// named.
pub(super) fn resolve_startup(startup: Startup) -> StartupRoots {
    let Startup {
        launch,
        remembered,
        default,
        closed,
    } = startup;
    let (exists, launch_exists) = std::thread::scope(|scope| {
        let checks: Vec<_> = remembered
            .iter()
            .map(|folder| scope.spawn(|| library::folder_exists(folder)))
            .collect();
        let launch_check = launch
            .as_ref()
            .map(|folder| scope.spawn(|| library::folder_exists(folder)));
        (
            checks
                .into_iter()
                .map(|check| check.join().unwrap_or(false))
                .collect::<Vec<_>>(),
            launch_check.is_some_and(|check| check.join().unwrap_or(false)),
        )
    });
    let mut roots = Vec::new();
    let mut missing = Vec::new();
    for (folder, exists) in remembered.iter().zip(exists) {
        if exists {
            roots.push(folder.clone());
        } else {
            missing.push(folder.clone());
        }
    }
    if let Some(launch) = &launch
        && launch_exists
        && !roots
            .iter()
            .any(|root| library::model::same_path(root, launch))
    {
        roots.push(launch.clone());
    }
    if remembered.is_empty()
        && !closed
        && launch.is_none()
        && let Ok(folder) = crate::platform::paths::default_notes_folder()
    {
        roots.push(library::normalize_folder(&folder));
    }
    let default = default
        .filter(|wanted| {
            roots
                .iter()
                .any(|root| library::model::same_path(root, wanted))
        })
        .or_else(|| roots.first().cloned());
    StartupRoots {
        roots,
        default,
        notice: library::roots::missing_notice(&missing),
    }
}

/// The remembered notebooks FastPad can open together: a duplicate of an earlier one, or one
/// nested inside or around an earlier one, only comes from editing `folders.ini` by hand, and
/// is left out without a word. The file is not rewritten.
fn restorable_roots(remembered: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut kept: Vec<PathBuf> = Vec::new();
    for folder in remembered {
        let folder = library::normalize_folder(&folder);
        let clashes = kept
            .iter()
            .any(|root| library::model::same_path(root, &folder))
            || library::roots::nesting(&kept, &folder).is_some();
        if !clashes {
            kept.push(folder);
        }
    }
    kept
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
    let (remembered, default) = recent.startup_roots();
    let remembered = restorable_roots(remembered);
    let default = default.map(|folder| library::normalize_folder(&folder));
    // With nothing remembered, no closed notebook and no folder named, the notebook that usually
    // opens is `Documents\FastPad`.
    let fallback = if remembered.is_empty() && !recent.closed && launch.is_none() {
        crate::platform::paths::default_notes_folder()
            .ok()
            .map(|folder| library::normalize_folder(&folder))
    } else {
        None
    };
    let wanted: Vec<PathBuf> = remembered.iter().cloned().chain(fallback).collect();
    if wanted.is_empty() && launch.is_none() {
        // Nothing to open (`open=none`, or no Documents folder): no notebook, and no worker.
        host(hwnd, |host| {
            host.notebooks.clear();
            host.default_notebook = None;
            host.startup_load = None;
        });
        crate::window::main_window::invalidate_title_strip(hwnd);
        crate::window::side_panel::refresh(hwnd);
        return;
    }
    // Every notebook gets its slot now, so the sidebar says "Loading…" under each until its
    // own worker answers. The worker removes the slots of the notebooks that are missing.
    let Some((slots, launch_generation)) = host(hwnd, |host| {
        let mut slots = Vec::new();
        for folder in &wanted {
            let existing = host
                .notebooks
                .iter()
                .find(|slot| library::model::same_path(&slot.folder, folder))
                .map(|slot| slot.id);
            let id = existing.unwrap_or_else(|| host.add_notebook(folder.clone()));
            if let Some(load) = begin_load(host, id) {
                slots.push(load);
            }
        }
        let wanted_default = default
            .as_ref()
            .and_then(|wanted| {
                host.notebooks
                    .iter()
                    .find(|slot| library::model::same_path(&slot.folder, wanted))
            })
            .map(|slot| slot.id);
        if let Some(id) = wanted_default {
            host.default_notebook = Some(id);
        }
        let launch_generation = launch.as_ref().map(|_| host.fresh_generation());
        host.startup_load = launch_generation;
        (slots, launch_generation)
    }) else {
        return;
    };
    crate::window::main_window::invalidate_title_strip(hwnd);
    crate::window::side_panel::refresh(hwnd);
    spawn_startup(
        hwnd,
        data,
        Startup {
            launch,
            remembered,
            default,
            closed: recent.closed,
        },
        slots,
        launch_generation,
    );
}

/// Loads (or rescans) the notebook `id`.
pub(crate) fn start_load_of(hwnd: HWND, id: NotebookId) {
    spawn_load(hwnd, id);
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

/// Marks the notebook `id` as loading under a fresh generation and returns its folder and that
/// generation, so only the answer to this load is installed.
fn begin_load(host: &mut LibraryHost, id: NotebookId) -> Option<(PathBuf, u64)> {
    let generation = host.fresh_generation();
    let slot = host.slot_mut(id)?;
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
    Some((slot.folder.clone(), generation))
}

fn post_loaded(target: isize, loaded: Loaded) {
    let payload = Box::into_raw(Box::new(loaded));
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
}

fn load_folder(data: &Path, folder: &Path) -> Result<LibraryState, String> {
    let local_path = library::local::local_file(data, folder);
    library::load(folder, &local_path, library::now_unix()).map_err(|error| error.to_string())
}

/// Starts the load worker for the notebook `id`.
pub(super) fn spawn_load(hwnd: HWND, id: NotebookId) {
    let Some(data) = data_dir(hwnd) else {
        return;
    };
    let Some((folder, generation)) = host(hwnd, |host| begin_load(host, id)).flatten() else {
        return;
    };
    let target = hwnd as isize;
    std::thread::spawn(move || {
        let result = load_folder(&data, &folder);
        post_loaded(
            target,
            Loaded {
                generation,
                folder,
                result,
                notice: None,
                closed: false,
                default: None,
            },
        );
    });
}

/// Starts the startup worker. `slots` are the notebooks the UI thread opened a slot for, with the
/// generation of each one's load; `launch_generation` is the load of the folder named on the
/// command line. The worker checks which folders exist, then loads each notebook on its own
/// thread and answers for each as it finishes: a missing notebook's slot is removed by its
/// answer, which carries the one notice and the default.
fn spawn_startup(
    hwnd: HWND,
    data: PathBuf,
    startup: Startup,
    slots: Vec<(PathBuf, u64)>,
    launch_generation: Option<u64>,
) {
    let target = hwnd as isize;
    std::thread::spawn(move || {
        let launch = startup.launch.clone();
        let StartupRoots {
            roots,
            default,
            notice,
        } = resolve_startup(startup);
        let generation_of = |folder: &Path| {
            slots
                .iter()
                .find(|(slot, _)| library::model::same_path(slot, folder))
                .map(|(_, generation)| *generation)
        };
        let nothing = |generation: u64| Loaded {
            generation,
            folder: PathBuf::new(),
            result: Err(String::new()),
            notice: None,
            closed: true,
            default: None,
        };
        let mut carried = Some((notice, default));
        for (folder, generation) in &slots {
            if roots
                .iter()
                .any(|root| library::model::same_path(root, folder))
            {
                continue;
            }
            let (notice, default) = carried.take().unwrap_or_default();
            post_loaded(
                target,
                Loaded {
                    notice,
                    default,
                    ..nothing(*generation)
                },
            );
        }
        if let (Some(launch), Some(generation)) = (&launch, launch_generation) {
            let exists = roots
                .iter()
                .any(|root| library::model::same_path(root, launch));
            if !exists {
                // Not a folder: nothing to add.
                post_loaded(target, nothing(generation));
            } else if generation_of(launch).is_some() {
                // Also a remembered notebook: it is revealed, not loaded again.
                post_loaded(
                    target,
                    Loaded {
                        folder: launch.clone(),
                        closed: false,
                        ..nothing(generation)
                    },
                );
            }
        }
        std::thread::scope(|scope| {
            for root in &roots {
                let generation = match generation_of(root) {
                    Some(generation) => generation,
                    // The folder named on the command line, which is not a remembered one.
                    None => match launch_generation {
                        Some(generation) => generation,
                        None => continue,
                    },
                };
                let data = &data;
                scope.spawn(move || {
                    let result = load_folder(data, root);
                    post_loaded(
                        target,
                        Loaded {
                            generation,
                            folder: root.clone(),
                            result,
                            notice: None,
                            closed: false,
                            default: None,
                        },
                    );
                });
            }
        });
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
        default: None,
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
    let Some(id) = host(hwnd, |host| {
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
        default,
        ..
    } = *loaded;
    if let Some(notice) = notice {
        push_notice(hwnd, notice);
    }
    // A missing default hands over to the first notebook that opened (in memory only: `open=`
    // and `default=` are rewritten when the open set next changes).
    if let Some(default) = default {
        host(hwnd, |host| {
            let slot = host
                .notebooks
                .iter()
                .find(|slot| library::model::same_path(&slot.folder, &default));
            if let Some(slot) = slot {
                host.default_notebook = Some(slot.id);
            }
        });
    }
    if closed {
        // The notebook is missing, or the path on the command line was not a folder: its slot,
        // if it had one, goes. `open=` keeps remembering it.
        if let Some(id) = id {
            host(hwnd, |host| host.remove_notebook(id));
        }
        crate::window::main_window::invalidate_title_strip(hwnd);
        crate::window::side_panel::refresh(hwnd);
        return;
    }
    // The folder named on the command line has no slot yet: it opens like any other folder, so
    // it is revealed when already open and refused when nested in an open notebook.
    let id = match id {
        Some(id) => id,
        None => {
            let Some(id) = add_root(hwnd, &folder) else {
                return;
            };
            notebook_opened(hwnd, id);
            id
        }
    };
    match result {
        Ok(fresh) => install_of(hwnd, id, fresh),
        Err(error) => {
            slot(hwnd, id, |slot| slot.load_failed = true);
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
    if slot(hwnd, id, |slot| std::mem::take(&mut slot.rescan_requested)).unwrap_or(false) {
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
