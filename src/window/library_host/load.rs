//! Loading the notebook: resolving the startup folder on a worker, the deferred first load,
//! Retry after a failed load, and installing a loaded `LibraryState` into the window.

use super::*;
use crate::library::{self, LibraryState, Metadata};
use crate::window::main_window::{app_ptr, push_notice};
use std::path::{Path, PathBuf};
use windows_sys::Win32::Foundation::{HWND, LPARAM};
use windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW;

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

/// `WM_FASTPAD_OPEN_LIBRARY`: opens a slot for every notebook the last session had open and
/// starts the startup worker. Reads only `folders.ini`: whether each folder (or a command-line
/// path) exists is checked on the worker, one thread per folder. After a session that closed its
/// notebook, with no path on the command line, nothing opens and no worker starts.
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
    // A first run launched with a path still opens `Documents\FastPad` when that path turns out
    // not to be a folder (a note double-clicked, say): the launch worker decides, as only it
    // checks the path.
    let launch_fallback = if remembered.is_empty() && !recent.closed && launch.is_some() {
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
            host.startup_notice = None;
            host.unavailable.clear();
            host.open_order.clear();
        });
        crate::window::main_window::invalidate_title_strip(hwnd);
        crate::window::side_panel::refresh(hwnd);
        if !recent.closed {
            push_notice(
                hwnd,
                "FastPad could not open a notebook: the Documents folder could not be found."
                    .to_owned(),
            );
        }
        return;
    }
    // Every notebook gets its slot now, so the sidebar says "Loading…" under each until its
    // own worker answers. The worker removes the slots of the notebooks that are missing.
    let Some((slots, launch_generation, notice_generation)) = host(hwnd, |host| {
        host.unavailable.clear();
        host.open_order = remembered.clone();
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
        let notice_generation = host.fresh_generation();
        host.startup_notice = Some(notice_generation);
        (slots, launch_generation, notice_generation)
    }) else {
        return;
    };
    crate::window::main_window::invalidate_title_strip(hwnd);
    crate::window::side_panel::refresh(hwnd);
    let launch = launch.zip(launch_generation);
    spawn_startup(
        hwnd,
        data,
        StartupJobs {
            slots,
            remembered,
            launch,
            launch_fallback,
            notice_generation,
        },
    );
}

/// Loads (or rescans) the notebook `id`.
pub(crate) fn start_load_of(hwnd: HWND, id: NotebookId) {
    spawn_load(hwnd, id);
}

/// Whether the notebook `id` failed to load and has no state to show: the sidebar says so and
/// offers Retry. A rescan that fails keeps the state it had, so this stays false then.
pub(crate) fn load_failed_of(hwnd: HWND, id: NotebookId) -> bool {
    slot(hwnd, id, |slot| slot.load_failed && slot.state.is_none()).unwrap_or(false)
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
            },
        );
    });
}

/// What the startup worker does.
struct StartupJobs {
    /// The notebooks the UI thread opened a slot for, with the generation of each one's load.
    slots: Vec<(PathBuf, u64)>,
    /// The notebooks `folders.ini` lists (checked on the worker); a slot not among them is the
    /// `Documents\FastPad` fallback, which is not checked.
    remembered: Vec<PathBuf>,
    /// The folder named on the command line and the generation of its load.
    launch: Option<(PathBuf, u64)>,
    /// On a first run, the notebook that opens instead when the launch path is not a folder:
    /// `Documents\FastPad`, answered under the launch's generation.
    launch_fallback: Option<PathBuf>,
    /// The generation of the answer that carries the notice for missing notebooks.
    notice_generation: u64,
}

fn nothing_to_open(generation: u64) -> Loaded {
    Loaded {
        generation,
        folder: PathBuf::new(),
        result: Err(String::new()),
        notice: None,
        closed: true,
    }
}

/// Starts the startup worker: one thread per notebook, each of which checks that its folder
/// exists and then loads it, answering for itself, so an offline drive delays only its own
/// notebook. A missing notebook answers "nothing to open", which removes its slot. When the last
/// check has finished, one answer carries the notice naming every missing notebook.
fn spawn_startup(hwnd: HWND, data: PathBuf, jobs: StartupJobs) {
    let target = hwnd as isize;
    std::thread::spawn(move || {
        let StartupJobs {
            slots,
            remembered,
            launch,
            launch_fallback,
            notice_generation,
        } = jobs;
        let is_remembered = |folder: &Path| {
            remembered
                .iter()
                .any(|root| library::model::same_path(root, folder))
        };
        let checked = slots
            .iter()
            .filter(|(folder, _)| is_remembered(folder))
            .count();
        let pending = std::sync::atomic::AtomicUsize::new(checked);
        let missing = std::sync::Mutex::new(Vec::<(usize, PathBuf)>::new());
        std::thread::scope(|scope| {
            for (index, (folder, generation)) in slots.iter().enumerate() {
                let (data, pending, missing) = (&data, &pending, &missing);
                let check = is_remembered(folder);
                scope.spawn(move || {
                    let exists = !check || library::folder_exists(folder);
                    if check {
                        if !exists {
                            missing.lock().unwrap().push((index, folder.clone()));
                        }
                        // The last check to finish reports every missing notebook at once.
                        if pending.fetch_sub(1, std::sync::atomic::Ordering::SeqCst) == 1 {
                            let mut missing = std::mem::take(&mut *missing.lock().unwrap());
                            missing.sort();
                            let missing: Vec<PathBuf> =
                                missing.into_iter().map(|(_, folder)| folder).collect();
                            if let Some(notice) = library::roots::missing_notice(&missing) {
                                post_loaded(
                                    target,
                                    Loaded {
                                        notice: Some(notice),
                                        ..nothing_to_open(notice_generation)
                                    },
                                );
                            }
                        }
                    }
                    if !exists {
                        post_loaded(target, nothing_to_open(*generation));
                        return;
                    }
                    let result = load_folder(data, folder);
                    post_loaded(
                        target,
                        Loaded {
                            generation: *generation,
                            folder: folder.clone(),
                            result,
                            notice: None,
                            closed: false,
                        },
                    );
                });
            }
            if let Some((launch, generation)) = &launch {
                let (data, slots, launch_fallback) = (&data, &slots, &launch_fallback);
                scope.spawn(move || {
                    if !library::folder_exists(launch) {
                        // Not a folder: nothing to add, unless this is a first run, which opens
                        // `Documents\FastPad` as it would without a path.
                        let Some(fallback) = launch_fallback else {
                            post_loaded(target, nothing_to_open(*generation));
                            return;
                        };
                        let result = load_folder(data, fallback);
                        post_loaded(
                            target,
                            Loaded {
                                generation: *generation,
                                folder: fallback.clone(),
                                result,
                                notice: None,
                                closed: false,
                            },
                        );
                        return;
                    }
                    let listed = slots
                        .iter()
                        .any(|(folder, _)| library::model::same_path(folder, launch));
                    if listed {
                        // Also a remembered notebook: it is revealed, not loaded again.
                        post_loaded(
                            target,
                            Loaded {
                                folder: launch.clone(),
                                closed: false,
                                ..nothing_to_open(*generation)
                            },
                        );
                        return;
                    }
                    let result = load_folder(data, launch);
                    post_loaded(
                        target,
                        Loaded {
                            generation: *generation,
                            folder: launch.clone(),
                            result,
                            notice: None,
                            closed: false,
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
    })) as LPARAM
}

/// `WM_FASTPAD_LIBRARY_READY`: installs the worker's result unless a newer load superseded it.
pub(crate) fn library_ready(hwnd: HWND, lparam: LPARAM) {
    if lparam == 0 {
        return;
    }
    let loaded = unsafe { Box::from_raw(lparam as *mut Loaded) };
    // The startup notice for the notebooks that are not available belongs to no slot.
    if host(hwnd, |host| {
        let ours = host.startup_notice == Some(loaded.generation);
        if ours {
            host.startup_notice = None;
        }
        ours
    }) == Some(true)
    {
        if let Some(notice) = loaded.notice {
            push_notice(hwnd, notice);
        }
        return;
    }
    // The slot the load was for, or `None` for the load of the folder named on the command line,
    // which has no slot yet. A result for a notebook since closed or reloaded matches neither and
    // is dropped.
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
        closed,
        ..
    } = *loaded;
    if closed {
        // The notebook is missing, or the path on the command line was not a folder: its slot,
        // if it had one, goes, and a missing default hands over to the first notebook left
        // (`remove_notebook`). `open=` keeps remembering it: it is kept as unavailable.
        if let Some(id) = id {
            host(hwnd, |host| {
                if let Some(slot) = host.remove_notebook(id) {
                    host.unavailable.push(slot.folder);
                }
            });
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
