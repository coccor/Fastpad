//! Actions on a note file: Move to notebook, Reveal in Explorer and Delete.

use super::*;
use crate::library;
use crate::library::ops::PendingOp;
use crate::library::title;
use crate::window::command_palette::{Picker, PickerKind};
use crate::window::main_window::{app_ptr, push_notice, window_identity};
use std::path::{Path, PathBuf};
use windows_sys::Win32::Foundation::HWND;

/// The Move to notebook picker's notebooks: favorites by name, then recent ones, never the open
/// notebook and never twice.
pub(super) fn move_destinations(hwnd: HWND) -> Vec<PathBuf> {
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
    crate::window::main_window::open_picker(
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
pub(super) fn tab_open_for(hwnd: HWND, path: &Path) -> bool {
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
pub(super) fn report_rebind_failure(hwnd: HWND, old: &Path, new: &Path) {
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
pub(super) fn move_note_to(hwnd: HWND, note: &Path, destination: &Path) {
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
    crate::window::side_panel::refresh(hwnd);
}

/// Writes the unsaved edits of `note`'s tab, if it has one, so they move with the file. A tab
/// that is not active is saved through a brief switch to it, as `autosave_all` does. False (and
/// a notice) when the save failed: nothing may move then.
pub(super) fn save_before_move(hwnd: HWND, note: &Path) -> bool {
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
    let saved = crate::window::main_window::activate_document_by_id(hwnd, id)
        && crate::window::main_window::complete_autosave(hwnd, &identity);
    if !identity.is_live_for(hwnd) {
        return false;
    }
    if let Some(active) = active.filter(|&active| active != id) {
        crate::window::main_window::activate_document_by_id(hwnd, active);
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
pub(crate) fn promote_tab_for(hwnd: HWND, path: &Path) {
    let promoted = unsafe { app_ptr(hwnd) }.is_some_and(|mut app| {
        let tabs = &mut unsafe { app.as_mut() }.tabs;
        tabs.find_stored_path(path)
            .is_some_and(|id| tabs.promote(id))
    });
    if promoted {
        crate::window::main_window::invalidate_title_strip(hwnd);
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
    if !confirmed(hwnd, &question, "Delete") {
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
        crate::window::main_window::close_document_without_prompt(hwnd, id);
    }
    crate::window::side_panel::refresh(hwnd);
}
