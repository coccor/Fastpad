//! Opening, checking, closing and favoriting notebooks (`folders.ini`), and files or
//! folders dropped on the window or an editor.

use super::*;
use crate::library;
use crate::window::command_palette::{Picker, PickerKind};
use crate::window::main_window::{app_ptr, push_notice, window_identity};
use std::path::{Path, PathBuf};
use windows_sys::Win32::Foundation::{HWND, LPARAM};
use windows_sys::Win32::UI::WindowsAndMessaging::{KillTimer, PostMessageW};

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

pub(super) const NOTES_MODE_OFF: &str =
    "Notes mode is off. Turn it on with Notes: Toggle notes mode to open notebooks.";

/// Saves the dirty notes of the notebook that is about to be replaced or closed, while autosave
/// still applies to them, and flushes its pending pins. Losing the notebook now would drop the
/// unsaved pins, so a failed write keeps it open: this schedules a retry, reports it and returns
/// false. `false` also means the window went away meanwhile.
pub(super) fn save_before_leaving(hwnd: HWND) -> bool {
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
pub(super) fn open_checked_folder(hwnd: HWND, path: PathBuf) {
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
    // A folder box names a folder of the notebook that just went.
    close_stale_name_box(hwnd);
    update_folders(hwnd, |folders| folders.push(path.clone()));
    start_load(hwnd);
    crate::window::main_window::invalidate_title_strip(hwnd);
    crate::window::side_panel::refresh(hwnd);
}

/// What the existence worker found for a notebook picked from a list.
pub(super) struct NotebookChecked {
    pub(super) folder: PathBuf,
    pub(super) exists: bool,
    /// Show the Notebook view once the notebook is open, with the focus in it for `Some(true)`.
    /// The Favorites view asks for it.
    pub(super) show_notebook: Option<bool>,
    /// `LibraryHost::check_request` when this check started. If the host's has since moved on (a
    /// newer check, or an explicit switch or close), this answer is stale and is dropped.
    pub(super) check_request: u64,
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

pub(super) fn check_listed_notebook(hwnd: HWND, folder: &Path, show_notebook: Option<bool>) {
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
    if crate::window::modal::modal_active(hwnd) {
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
        crate::window::side_panel::show_view(hwnd, crate::config::SidebarView::Notebook, focus);
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
pub(super) fn known_folders(hwnd: HWND, read_if_unknown: bool) -> library::local::RecentFolders {
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
pub(super) fn update_folders<R>(
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
    crate::window::side_panel::refresh(hwnd);
}

/// Removes `folder` from the favorites; nothing happens if it is not one.
pub(crate) fn remove_favorite(hwnd: HWND, folder: &Path) {
    let removed = update_folders(hwnd, |folders| {
        folders.is_favorite(folder) && !folders.toggle_favorite(folder)
    })
    .unwrap_or(false);
    if removed {
        crate::window::side_panel::refresh(hwnd);
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
    crate::window::main_window::invalidate_title_strip(hwnd);
    crate::window::side_panel::refresh(hwnd);
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
    crate::window::main_window::open_picker(
        hwnd,
        Picker {
            kind: PickerKind::RecentFolder,
            items,
            create: None,
        },
    );
}

/// Scintilla's own OLE drop target refuses files and wins over `WM_DROPFILES`, so every group's
/// editor gets a wrapper that posts dropped files here as `WM_FASTPAD_FILES_DROPPED`, with its
/// group window (split editors spec §6.2). The sidebar panel's own drop target is registered
/// here too. Runs in `BUILD_CHROME`; `create_group` wraps groups made later.
pub(crate) fn accept_editor_file_drops(hwnd: HWND) {
    let editors = unsafe { app_ptr(hwnd) }.map(|mut app| {
        let app = unsafe { app.as_mut() };
        app.file_drops_accepted = true;
        app.groups
            .iter()
            .map(|group| (group.hwnd, group.editor.hwnd()))
            .collect::<Vec<_>>()
    });
    for (group, editor) in editors.unwrap_or_default() {
        wrap_group_drop_target(hwnd, group, editor);
    }
    crate::window::side_panel::accept_file_drops(hwnd);
}

/// Wraps `editor`'s drop target so its files open in group window `group`.
pub(crate) fn wrap_group_drop_target(hwnd: HWND, group: HWND, editor: HWND) {
    let (target, group) = (hwnd as isize, group as usize);
    // Text drag-and-drop still works without the wrapper; only file drops on the editor are lost.
    let _ = crate::editor::file_drop::accept_file_drops(editor, move |paths| {
        let payload = Box::into_raw(Box::new(paths));
        if unsafe {
            PostMessageW(
                target as HWND,
                crate::window::WM_FASTPAD_FILES_DROPPED,
                group,
                payload as isize,
            )
        } == 0
        {
            drop(unsafe { Box::from_raw(payload) });
        }
    });
}

/// `WM_FASTPAD_FILES_DROPPED`: frees the posted paths and opens them in the group of window
/// `wparam`, else the active one. A drop that lands while a modal dialog runs is ignored, as
/// `WM_DROPFILES` is for a disabled window.
pub(crate) fn editor_files_dropped(
    hwnd: HWND,
    wparam: windows_sys::Win32::Foundation::WPARAM,
    lparam: LPARAM,
) {
    if lparam == 0 {
        return;
    }
    let paths = *unsafe { Box::from_raw(lparam as *mut Vec<PathBuf>) };
    if crate::window::modal::modal_active(hwnd) {
        return;
    }
    if let Some(group) = crate::window::main_window::group_id_of(hwnd, wparam as HWND) {
        crate::window::main_window::activate_group(hwnd, group);
    }
    files_dropped(hwnd, paths);
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
        } else if let Err(error) = crate::window::main_window::open_path(hwnd, &path) {
            crate::window::main_window::report_open_failure(hwnd, &path, &error);
        }
    }
    if let Some(folder) = folder
        && identity.is_live_for(hwnd)
    {
        open_folder(hwnd, &folder);
    }
}
