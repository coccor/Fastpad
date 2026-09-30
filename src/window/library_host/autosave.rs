//! Autosave: the notebook's switch, the idle timer, saving the active tab or every tab
//! before close, a changed file's Reload or Keep mine, and the disk stamps after loads and saves.

use super::*;
use crate::library::title;
use crate::library::{self};
use crate::window::main_window::{app_ptr, push_notice, window_identity};
use std::path::PathBuf;
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::{KillTimer, SetTimer};

/// The open notebook's autosave switch, or `None` while no notebook is open or its state is still
/// loading. The Settings dialog greys its row out then (settings dialog spec §3.2).
pub(crate) fn notebook_autosave(hwnd: HWND) -> Option<bool> {
    host(hwnd, |host| {
        host.state.as_ref().map(|state| state.local.autosave)
    })
    .flatten()
}

pub(super) fn folder_autosave(hwnd: HWND) -> bool {
    host(hwnd, |host| {
        // Until the folder's state has loaded, its autosave setting is unknown: do not save.
        host.state
            .as_ref()
            .is_some_and(|state| state.local.autosave)
    })
    .unwrap_or(false)
}

/// The active tab's path, if autosave applies to it right now.
pub(super) fn autosave_target(hwnd: HWND) -> Option<PathBuf> {
    if !notes_mode(hwnd)
        || !folder_autosave(hwnd)
        || crate::window::main_window::file_population_active(hwnd)
    {
        return None;
    }
    let folder = folder(hwnd)?;
    unsafe { app_ptr(hwnd) }.and_then(|app| {
        let active = unsafe { app.as_ref() }.tabs.active()?;
        let path = active.path.clone()?;
        // An image tab is never dirty; the check keeps a stray flag from writing over an image.
        (active.dirty
            && !active.is_image()
            && !active.autosave_paused
            && library::is_inside(&folder, &path))
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
    if crate::window::main_window::complete_autosave(hwnd, &identity) {
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
        if !identity.is_live_for(hwnd)
            || !crate::window::main_window::activate_document_by_id(hwnd, id)
        {
            return;
        }
        autosave_active(hwnd);
    }
    // The session records the tab the user had active, not the last one saved.
    if let Some(active) = active
        && identity.is_live_for(hwnd)
    {
        crate::window::main_window::activate_document_by_id(hwnd, active);
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
            crate::window::main_window::report_open_failure(hwnd, &path, &error);
            return;
        }
    };
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    let Some(editor) =
        unsafe { app_ptr(hwnd) }.and_then(|app| unsafe { app.as_ref() }.editor().cloned())
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
    crate::window::main_window::remove_saved_document_snapshots(hwnd);
    crate::window::main_window::invalidate_title_strip(hwnd);
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
    let _ = crate::window::main_window::complete_save(hwnd, &identity, None);
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
    if inserted || crate::window::notebook_view::stale(hwnd) {
        crate::window::side_panel::refresh(hwnd);
    }
    if promoted {
        // The tab's name is no longer italic.
        crate::window::main_window::invalidate_title_strip(hwnd);
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
