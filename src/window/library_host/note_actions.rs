//! Actions on a note file: Move to notebook, Reveal in Explorer and Delete.

use super::*;
use crate::library;
use crate::library::ops::PendingOp;
use crate::library::title;
use crate::library::tree::RowKind;
use crate::window::command_palette::{Picker, PickerKind};
use crate::window::main_window::{app_ptr, push_notice, window_identity};
use std::path::{Path, PathBuf};
use windows_sys::Win32::Foundation::HWND;

/// The Move to notebook picker's notebooks (multi-root notebooks spec §5.3): the other open
/// notebooks in sidebar order, then favorites by name, then recent ones; never the notebook the
/// note is in (`source`) and never twice.
pub(super) fn move_destinations(hwnd: HWND, source: &Path) -> Vec<PathBuf> {
    let known = known_folders(hwnd, true);
    let open: Vec<PathBuf> = notebooks(hwnd)
        .into_iter()
        .map(|(_, folder)| folder)
        .collect();
    let is_open = |candidate: &PathBuf| {
        open.iter()
            .any(|folder| library::model::same_path(folder, candidate))
    };
    let mut destinations: Vec<PathBuf> = open
        .iter()
        .filter(|folder| !library::model::same_path(folder, source))
        .cloned()
        .collect();
    let favorites: Vec<PathBuf> = known
        .favorites
        .iter()
        .filter(|f| !is_open(f) && !library::model::same_path(f, source))
        .cloned()
        .collect();
    let mut named: Vec<(String, PathBuf)> = library::local::display_names(&favorites)
        .into_iter()
        .map(|(name, _)| name)
        .zip(favorites)
        .collect();
    named.sort_by(|a, b| library::tree::natural_cmp(&a.0, &b.0));
    for candidate in named.into_iter().map(|(_, path)| path).chain(known.folders) {
        if !library::model::same_path(&candidate, source)
            && !destinations
                .iter()
                .any(|listed| library::model::same_path(listed, &candidate))
        {
            destinations.push(candidate);
        }
    }
    destinations
}

/// Note: Move to notebook… (spec §6.6, multi-root move spec §3.4): picks another notebook, then,
/// for an open one, a folder in it.
pub(crate) fn move_to_notebook(hwnd: HWND, path: &Path) {
    let Some(source) = note_root(hwnd, path) else {
        push_notice(
            hwnd,
            "Only notes in an open notebook can be moved to another notebook.".to_owned(),
        );
        return;
    };
    show_move_picker(hwnd, path, &source.1);
}

/// Move to notebook… on the folder `relative` of the notebook `id` (multi-root move spec §3.4).
pub(crate) fn move_folder_to_notebook(hwnd: HWND, id: NotebookId, relative: &Path) {
    let Some(root) = notebook_folder(hwnd, id) else {
        return;
    };
    if !library::tree::is_plain_relative_folder(relative) {
        return;
    }
    show_move_picker(hwnd, &root.join(relative), &root);
}

/// Step 1 of Move to notebook…: the other notebooks for `path` (absolute), of the notebook at
/// `source`.
fn show_move_picker(hwnd: HWND, path: &Path, source: &Path) {
    if crate::window::cross_move_host::refuse_item_if_moving(hwnd, path) {
        return;
    }
    let destinations = move_destinations(hwnd, source);
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
            initial: None,
        },
    );
}

/// The open notebook a note at `path` (absolute) is inside, and that notebook's folder. `None`
/// for a file no open notebook owns, or a notebook's folder itself.
pub(super) fn note_root(hwnd: HWND, path: &Path) -> Option<(NotebookId, PathBuf)> {
    let id = notebook_of(hwnd, path)?;
    let root = notebook_folder(hwnd, id)?;
    library::is_inside(&root, path).then_some((id, root))
}

/// Makes a `rebind_open_tab` failure visible: a tab is left pointing at a path that no longer
/// exists, because another tab already had the new one open. A file moved outside FastPad onto
/// a path another tab has open reaches this through the rescan; `move_note_to` refuses such a
/// move upfront, and `submit_rename` never calls `rebind_open_tab`.
pub(crate) fn report_rebind_failure(hwnd: HWND, old: &Path, new: &Path) {
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

/// Move to notebook… chose the notebook `destination` for `item` (absolute, a note or a
/// folder): an open notebook asks for a folder in it next (multi-root move spec §3.4); a folder
/// inside an open notebook (from Browse…) is that folder; any other folder receives it at its
/// root.
pub(super) fn move_item_to_notebook(hwnd: HWND, item: &Path, destination: &Path) {
    let open = notebooks(hwnd)
        .into_iter()
        .find(|(_, folder)| library::model::same_path(folder, destination));
    if let Some((id, _)) = open {
        let folders = folder_choices(hwnd, id);
        let name = notebook_name(destination);
        let mut items = vec![format!("{name} (root)")];
        items.extend(
            folders
                .iter()
                .skip(1)
                .map(|folder| folder.to_string_lossy().into_owned()),
        );
        host(hwnd, |host| {
            host.shown_move_folders = Some((item.to_path_buf(), id, folders));
        });
        crate::window::main_window::open_picker(
            hwnd,
            Picker {
                kind: PickerKind::MoveToFolder,
                items,
                create: None,
                initial: None,
            },
        );
        return;
    }
    let dest = match note_root(hwnd, destination) {
        Some((id, root)) => crate::window::cross_move_host::Destination::Open {
            id,
            folder: library::record_path(&root, destination),
        },
        None => crate::window::cross_move_host::Destination::Closed {
            root: destination.to_path_buf(),
        },
    };
    move_item_to(hwnd, item, dest);
}

/// The folders of the open notebook `id` a move may go into: the root (empty) first, then
/// every folder by relative path in natural order.
pub(super) fn folder_choices(hwnd: HWND, id: NotebookId) -> Vec<PathBuf> {
    let mut folders = with_notebook(hwnd, id, |state| state.folders.clone()).unwrap_or_default();
    folders.sort_by(|a, b| library::tree::natural_cmp(&a.to_string_lossy(), &b.to_string_lossy()));
    folders.dedup_by(|a, b| library::model::same_path(a, b));
    std::iter::once(PathBuf::new()).chain(folders).collect()
}

/// Moves `item` (absolute) of its open notebook to `dest`, through the same planner, question
/// and hand-off as a drag (multi-root move spec §3.4).
pub(super) fn move_item_to(
    hwnd: HWND,
    item: &Path,
    dest: crate::window::cross_move_host::Destination,
) {
    let Some((from, root)) = note_root(hwnd, item) else {
        push_notice(
            hwnd,
            "Only notes in an open notebook can be moved to another notebook.".to_owned(),
        );
        return;
    };
    if let crate::window::cross_move_host::Destination::Open { id, .. } = &dest
        && *id == from
    {
        push_notice(
            hwnd,
            format!(
                "{} is already in {}.",
                title::note_title(item),
                notebook_name(&root)
            ),
        );
        return;
    }
    let relative = library::record_path(&root, item);
    let kind = if item.is_dir() {
        RowKind::Folder(relative)
    } else {
        RowKind::Note(relative)
    };
    crate::window::cross_move_host::move_items(hwnd, from, vec![kind], dest);
}

/// Writes the unsaved edits of `note`'s tab, if it has one, so they move with the file. A tab
/// that is not active is saved through a brief switch to it, as `autosave_all` does. False (and
/// a notice) when the save failed: nothing may move then.
pub(crate) fn save_before_move(hwnd: HWND, note: &Path) -> bool {
    let tab = |hwnd: HWND| {
        unsafe { app_ptr(hwnd) }.and_then(|app| {
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
        })
    };
    let Some((id, _, _, known)) = tab(hwnd) else {
        return true;
    };
    // The tab takes in an append made outside FastPad, or raises the conflict, first, as
    // autosave does (second brain phase 1 spec §5.4), whether or not the watcher got there.
    if known.is_none() || known != library::disk_stamp(note) {
        super::check_tab(hwnd, id);
    }
    let Some((id, active, paused, known)) = tab(hwnd) else {
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
    if crate::window::cross_move_host::refuse_item_if_moving(hwnd, path) {
        return;
    }
    // The callers checked the default notebook; a note of another one needs its own file.
    if let Some((owner, _)) = note_root(hwnd, path)
        && !ready_library_of(hwnd, Some(owner))
    {
        return;
    }
    let dirty = unsafe { app_ptr(hwnd) }.and_then(|app| {
        let tabs = &unsafe { app.as_ref() }.tabs;
        let id = tabs.find_stored_path(path)?;
        Some(tabs.document(id)?.dirty)
    });
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    // The tab's unsaved edits go with the file: they are not autosaved first.
    let question = if dirty == Some(true) {
        format!("Move \u{201c}{name}\u{201d} to the Recycle Bin and discard unsaved changes?")
    } else {
        format!("Move \u{201c}{name}\u{201d} to the Recycle Bin?")
    };
    if !confirmed(hwnd, &question, "Delete") {
        return;
    }
    match perform_delete_note(hwnd, path) {
        Ok(()) => crate::window::side_panel::refresh(hwnd),
        Err(Some(notice)) => push_notice(hwnd, notice),
        Err(None) => {}
    }
}

/// `delete_file` without its question or the view's refresh: recycles `path`, flags its record
/// deleted and closes its tab without asking. `Err` holds the notice to show, `None` when the
/// window went away meanwhile.
fn perform_delete_note(hwnd: HWND, path: &Path) -> Result<(), Option<String>> {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return Err(None);
    };
    // The shell may show its own modal warning (a permanent delete), owned by this window.
    let recycled = crate::platform::files::recycle(hwnd, path);
    if !identity.is_live_for(hwnd) {
        return Err(None);
    }
    if let Err(error) = recycled {
        return Err(Some(format!(
            "FastPad could not delete {}: {error}",
            path.display()
        )));
    }
    // The record is the one in the notebook that owns the file (multi-root notebooks spec §5.2).
    if let Some((owner, _)) = note_root(hwnd, path) {
        let now = library::now_unix();
        let record = with_notebook(hwnd, owner, |state| {
            state.record_for(path).map(|record| record.id)
        })
        .flatten();
        if let Some(note_id) = record {
            report(
                hwnd,
                apply_op_of(hwnd, owner, |state, ids| {
                    Some(PendingOp::SetDeleted {
                        note: state.note_ref(ids, path),
                        value: true,
                    })
                }),
            );
            with_notebook(hwnd, owner, |state| state.local.set_missing(note_id, now));
        }
        with_notebook(hwnd, owner, |state| state.remove_note(path));
    }
    let tab = unsafe { app_ptr(hwnd) }
        .and_then(|app| unsafe { app.as_ref() }.tabs.find_stored_path(path));
    if let Some(id) = tab {
        // Its close activates it: the check that runs then knows the file is gone on purpose.
        mark_recycled(hwnd, id);
        crate::window::main_window::close_document_without_prompt(hwnd, id);
    }
    Ok(())
}

/// The one question of a group delete of `items` rows, `dirty` of the open tabs they take
/// having unsaved changes (tree multi-select spec, "Delete").
pub(crate) fn delete_items_question(items: usize, dirty: usize) -> String {
    match dirty {
        0 => format!("Move {items} items to the Recycle Bin?"),
        1 => format!(
            "Move {items} items to the Recycle Bin and discard unsaved changes in 1 open tab?"
        ),
        _ => format!(
            "Move {items} items to the Recycle Bin and discard unsaved changes in {dirty} open tabs?"
        ),
    }
}

/// Del or Delete… on a multi-selection (tree multi-select spec, "Delete"): one question for the
/// group of the notebook `notebook`, then each row goes as `delete_file` or `delete_folder_of`
/// sends it, and one that cannot is skipped. The view refreshes once, the selection is dropped
/// and the cursor lands on the row that took the first item's place; the failures are one
/// notice. `items` are the effective selection, so a folder is not listed along with what is
/// inside it.
pub(crate) fn delete_items(hwnd: HWND, notebook: NotebookId, items: &[RowKind]) {
    let Some(root) = notebook_folder(hwnd, notebook) else {
        return;
    };
    if !ready_library_of(hwnd, Some(notebook)) {
        return;
    }
    // Defence in depth: an empty or escaping folder path would recycle the notebook root or a
    // folder outside it.
    let items: Vec<&RowKind> = items
        .iter()
        .filter(|kind| match kind {
            RowKind::Note(_) => true,
            RowKind::Folder(relative) => library::tree::is_plain_relative_folder(relative),
            _ => false,
        })
        .collect();
    let absolute = |kind: &RowKind| match kind {
        RowKind::Note(relative) | RowKind::Folder(relative) => Some(root.join(relative)),
        _ => None,
    };
    if items.iter().any(|kind| {
        absolute(kind)
            .is_some_and(|path| crate::window::cross_move_host::refuse_item_if_moving(hwnd, &path))
    }) {
        return;
    }
    let Some(first) = items.first() else {
        return;
    };
    let dirty: usize = items
        .iter()
        .filter_map(|kind| Some((kind, absolute(kind)?)))
        .map(|(kind, path)| match kind {
            RowKind::Folder(_) => tabs_under(hwnd, &path)
                .iter()
                .filter(|(_, _, dirty)| *dirty)
                .count(),
            _ => unsafe { app_ptr(hwnd) }.map_or(0, |app| {
                let tabs = &unsafe { app.as_ref() }.tabs;
                usize::from(
                    tabs.find_stored_path(&path)
                        .and_then(|id| tabs.document(id))
                        .is_some_and(|document| document.dirty),
                )
            }),
        })
        .sum();
    if !confirmed(hwnd, &delete_items_question(items.len(), dirty), "Delete") {
        return;
    }
    // The row that takes the first item's place, remembered by what it shows: closing the tabs
    // can expand other folders above it and shift the indexes.
    let place = crate::window::notebook_view::row_in_place_of(hwnd, notebook, first);
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    let mut failed = Vec::new();
    for kind in &items {
        // The list holds notes and folders only (filtered above).
        let result = match kind {
            RowKind::Folder(relative) => perform_delete_folder(hwnd, notebook, relative),
            RowKind::Note(relative) => perform_delete_note(hwnd, &root.join(relative)),
            _ => continue,
        };
        match result {
            Ok(()) => {}
            Err(Some(notice)) => failed.push(notice),
            // Nothing was tried for this item; the rest still go unless the window went away.
            Err(None) if identity.is_live_for(hwnd) => {}
            Err(None) => return,
        }
    }
    crate::window::notebook_view::clear_selection(hwnd);
    crate::window::notebook_view::refresh_and_select_place(hwnd, place);
    if !failed.is_empty() {
        push_notice(
            hwnd,
            crate::window::tree_move::group_notice(&failed, items.len(), "deleted"),
        );
    }
}
