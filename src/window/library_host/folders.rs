//! Folder names, renames and deletes: the name errors, rerooting first-save folders, undoing
//! a rename a tab could not follow, and Delete on a folder row.

use super::*;
use crate::library;
use crate::library::tree::RowKind;
use crate::window::main_window::{app_ptr, push_notice, window_identity};
use std::path::{Path, PathBuf};
use windows_sys::Win32::Foundation::{
    ERROR_ALREADY_EXISTS, ERROR_FILE_EXISTS, ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, HWND,
};

/// A new or renamed folder's name is taken by a folder or file (spec §4.1).
pub(crate) fn folder_taken_error(name: &str) -> String {
    format!("A folder or file named \u{201c}{name}\u{201d} already exists")
}

/// A folder named like one the scan skips would vanish at the next rescan (scan §3.1).
pub(crate) fn hidden_folder_error(name: &str) -> String {
    format!("FastPad hides folders named \u{201c}{name}\u{201d}. Choose another name.")
}

/// `folder` (absolute, inside the notebook `root`) relative to it; empty for the root itself.
pub(crate) fn relative_folder(root: &Path, folder: &Path) -> PathBuf {
    if library::model::same_path(root, folder) {
        PathBuf::new()
    } else {
        library::record_path(root, folder)
    }
}

/// After a folder rename on disk from `old` to `new` (both absolute): the untitled tabs whose
/// first save was to go at or under `old` go at the same place under `new`.
pub(crate) fn reroot_save_folders(hwnd: HWND, old: &Path, new: &Path) {
    let Some(mut app) = (unsafe { app_ptr(hwnd) }) else {
        return;
    };
    let app = unsafe { app.as_mut() };
    let moved: Vec<_> = app
        .tabs
        .documents()
        .filter_map(|document| {
            let folder = document.save_folder.as_deref()?;
            Some((document.id, library::reroot(folder, old, new)?))
        })
        .collect();
    for (id, folder) in moved {
        if let Some(document) = app.tabs.document_mut(id) {
            document.save_folder = Some(folder);
        }
    }
}

/// Whether a failed `MoveFileExW` means the target name is taken.
pub(crate) fn already_exists(error: &crate::FastPadError) -> bool {
    matches!(
        error,
        crate::FastPadError::Win32(code) if *code == ERROR_ALREADY_EXISTS || *code == ERROR_FILE_EXISTS
    )
}

/// Whether a failed `MoveFileExW` means the source is gone.
pub(crate) fn not_found(error: &crate::FastPadError) -> bool {
    matches!(
        error,
        crate::FastPadError::Win32(code) if *code == ERROR_FILE_NOT_FOUND || *code == ERROR_PATH_NOT_FOUND
    )
}

/// The open tabs whose file is inside `folder` (absolute), with their stored paths and whether
/// each has unsaved edits. Read from the tabs' stored paths: no disk access.
pub(crate) fn tabs_under(
    hwnd: HWND,
    folder: &Path,
) -> Vec<(crate::document::DocumentId, PathBuf, bool)> {
    unsafe { app_ptr(hwnd) }
        .map(|app| {
            unsafe { app.as_ref() }
                .tabs
                .documents()
                .filter_map(|document| {
                    let path = document.path.as_ref()?;
                    library::is_inside(folder, path)
                        .then(|| (document.id, path.clone(), document.dirty))
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
thread_local! {
    static FAIL_NEXT_FOLDER_RENAME_BACK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Makes the next undo of a folder rename fail, as a locked folder would.
#[cfg(test)]
#[allow(
    dead_code,
    reason = "read by the lib window tests, not by the source-linked integration targets"
)]
pub(crate) fn fail_next_folder_rename_back() {
    FAIL_NEXT_FOLDER_RENAME_BACK.with(|fail| fail.set(true));
}

#[cfg(test)]
thread_local! {
    static FAIL_NEXT_NOTE_RENAME_BACK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Makes the next undo of a note rename fail, as a locked file would.
#[cfg(test)]
#[allow(
    dead_code,
    reason = "read by the lib window tests, not by the source-linked integration targets"
)]
pub(crate) fn fail_next_note_rename_back() {
    FAIL_NEXT_NOTE_RENAME_BACK.with(|fail| fail.set(true));
}

/// Renames a note back after its tab could not follow its rename.
pub(crate) fn rename_note_back(from: &Path, to: &Path) -> crate::Result<()> {
    #[cfg(test)]
    if FAIL_NEXT_NOTE_RENAME_BACK.with(|fail| fail.replace(false)) {
        return Err(crate::FastPadError::Invariant(
            "rename back refused for a test",
        ));
    }
    crate::platform::files::rename_no_replace(from, to)
}

/// Renames a folder back after a tab could not follow its rename.
pub(crate) fn rename_folder_back(from: &Path, to: &Path) -> crate::Result<()> {
    #[cfg(test)]
    if FAIL_NEXT_FOLDER_RENAME_BACK.with(|fail| fail.replace(false)) {
        return Err(crate::FastPadError::Invariant(
            "rename back refused for a test",
        ));
    }
    crate::platform::files::rename_no_replace(from, to)
}

/// The notice when a folder or note rename could not be undone after `stuck` (the tabs' old
/// paths) could not follow it.
pub(crate) fn rename_undo_failed_notice(old: &str, new: &str, stuck: &[PathBuf]) -> String {
    let name = |path: &PathBuf| {
        path.file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()
    };
    let still = match stuck {
        [one] => format!(
            "\u{201c}{}\u{201d} is still open at its old path.",
            name(one)
        ),
        [first, rest @ ..] => format!(
            "\u{201c}{}\u{201d} and {} more are still open at their old paths.",
            name(first),
            rest.len()
        ),
        [] => String::new(),
    };
    format!(
        "FastPad could not undo renaming \u{201c}{old}\u{201d} to \u{201c}{new}\u{201d}. {still}"
    )
    .trim_end()
    .to_owned()
}

/// The Delete confirmation for a folder named `name` holding `notes` listed notes, `dirty` of
/// its open tabs with unsaved changes (spec §4.3).
pub(super) fn delete_folder_question(name: &str, notes: usize, dirty: usize) -> String {
    let mut question = match notes {
        0 => format!("Move the folder \u{201c}{name}\u{201d} to the Recycle Bin?"),
        1 => format!("Move \u{201c}{name}\u{201d} and its 1 note to the Recycle Bin?"),
        _ => format!("Move \u{201c}{name}\u{201d} and its {notes} notes to the Recycle Bin?"),
    };
    match dirty {
        0 => {}
        1 => question.push_str("\n1 open note has unsaved changes, which will be lost."),
        _ => question.push_str(&format!(
            "\n{dirty} open notes have unsaved changes, which will be lost."
        )),
    }
    question
}

/// Del or Delete… on a folder row (spec §4.3): after a confirm, sends the folder and everything
/// in it to the Recycle Bin, closes the tabs under it without asking, and drops its notes, their
/// records flagged deleted. A failure changes nothing in the library.
pub(crate) fn delete_folder(hwnd: HWND, relative: &Path) {
    let Some(root) = folder(hwnd) else {
        return;
    };
    // Defence in depth: an empty or escaping path would recycle the notebook root or a folder
    // outside it.
    if !library::tree::is_plain_relative_folder(relative) {
        return;
    }
    let absolute = root.join(relative);
    let notes = with_state(hwnd, |state| state.notes_under(relative)).unwrap_or(0);
    let tabs = tabs_under(hwnd, &absolute);
    let dirty = tabs.iter().filter(|(_, _, dirty)| *dirty).count();
    let name = relative.file_name().unwrap_or_default().to_string_lossy();
    if !confirmed(hwnd, &delete_folder_question(&name, notes, dirty)) {
        return;
    }
    // The row that takes the folder's place, remembered by what it shows: closing the tabs
    // can expand other folders above it and shift the indexes.
    let place = crate::window::notebook_view::row_in_place_of(
        hwnd,
        &RowKind::Folder(relative.to_path_buf()),
    );
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    // The shell may show its own modal warning (a permanent delete), owned by this window.
    let recycled = crate::platform::files::recycle_folder(hwnd, &absolute);
    if !identity.is_live_for(hwnd) {
        return;
    }
    if let Err(error) = recycled {
        push_notice(
            hwnd,
            format!("FastPad could not delete {}: {error}", absolute.display()),
        );
        return;
    }
    // Each close activates the tab it closes, which autosaves the tab being left; a doomed tab
    // left that way has no file any more and would report it changed on disk.
    if let Some(mut app) = unsafe { app_ptr(hwnd) } {
        let app = unsafe { app.as_mut() };
        for (id, _, _) in &tabs {
            if let Some(document) = app.tabs.document_mut(*id) {
                document.autosave_paused = true;
            }
        }
    }
    for (id, _, _) in tabs {
        crate::window::main_window::close_document_without_prompt(hwnd, id);
    }
    with_state(hwnd, |state| {
        state.remove_folder(relative, library::now_unix())
    });
    save_local(
        hwnd,
        LocalWrite {
            wait: false,
            force: false,
        },
    );
    schedule_write(hwnd);
    close_stale_name_box(hwnd);
    crate::window::side_panel::with_accessible_events(hwnd, || {
        crate::window::side_panel::refresh(hwnd);
        // The row that took the folder's place: the next one after its subtree, or the previous
        // at the end; by index only when that row went too.
        if let Some((index, kind)) = place
            && !kind.is_some_and(|kind| crate::window::notebook_view::select_row(hwnd, &kind))
        {
            crate::window::notebook_view::select_index(hwnd, index);
        }
    });
}
