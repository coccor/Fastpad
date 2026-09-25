//! Moving a note or folder within the notebook (tree drag spec §4): the one no-replace rename,
//! then the open tabs, the library's records, pins and expanded folders, and the tabs' save
//! folders follow it. A tab that cannot follow undoes the move. The inline renames (inline
//! naming spec §5.2) are the same moves with a new name in the same folder.

use super::main_window::{app_ptr, push_notice};
use super::{library_host, notebook_view, side_panel, tree_drag};
use crate::library::{self, tree, tree::RowKind};
use crate::window::library_host::with_state;
use std::path::{Path, PathBuf};
use windows_sys::Win32::Foundation::HWND;

/// Why a move did not happen. Nothing on disk changed.
#[derive(Debug)]
pub(crate) enum MoveError {
    /// Something already has the new name (a change of letter case alone is not a clash).
    Taken,
    /// The source is gone from disk.
    Missing(crate::FastPadError),
    /// Windows says the path is gone, but the source is still there: the destination's parent
    /// folder is what vanished (tree drag spec §5).
    TargetGone(crate::FastPadError),
    /// Windows refused the move for another reason.
    Failed(crate::FastPadError),
    /// An open tab could not follow, and the move was undone.
    TabCantFollow,
}

/// A move that happened.
#[derive(Debug, Default)]
pub(crate) struct Moved {
    /// A note's open tab followed it.
    pub(crate) rebound: bool,
    /// The old paths of tabs that could not follow when the undo failed too: the move stands.
    pub(crate) stuck: Vec<PathBuf>,
}

/// `error` from the rename that failed to put `old_path` at its new path. A not-found error is
/// ambiguous (Windows doesn't say which path it means): the source still being there means it
/// was the destination's parent folder that vanished, not the source (tree drag spec §5).
fn disk_error(error: crate::FastPadError, case_only: bool, old_path: &Path) -> MoveError {
    if !case_only && library_host::already_exists(&error) {
        MoveError::Taken
    } else if library_host::not_found(&error) {
        if old_path.exists() {
            MoveError::TargetGone(error)
        } else {
            MoveError::Missing(error)
        }
    } else {
        MoveError::Failed(error)
    }
}

/// Moves the note `old` to `new`, both relative to the notebook `root`, never onto another
/// file. Its open tab follows; if it cannot, the note goes back, and if that fails too, the move
/// stands and `stuck` names the tab.
pub(crate) fn move_note(
    hwnd: HWND,
    root: &Path,
    old: &Path,
    new: &Path,
) -> Result<Moved, MoveError> {
    let (old_path, new_path) = (root.join(old), root.join(new));
    let case_only = library::model::same_path(&new_path, &old_path);
    crate::platform::files::rename_no_replace(&old_path, &new_path)
        .map_err(|error| disk_error(error, case_only, &old_path))?;
    let mut stuck = Vec::new();
    let rebound = match library_host::rebind_open_tab(hwnd, &old_path, new_path.clone()) {
        Ok(rebound) => rebound,
        Err(()) => {
            // Undo, so the tab and the disk agree.
            if library_host::rename_note_back(&new_path, &old_path).is_ok() {
                return Err(MoveError::TabCantFollow);
            }
            // The disk is the truth: the move stands, and the tab left on the old path is named.
            stuck.push(old_path.clone());
            false
        }
    };
    with_state(hwnd, |state| state.rename_note(&old_path, &new_path));
    library_host::schedule_write(hwnd);
    Ok(Moved { rebound, stuck })
}

/// Moves the folder `old` to `new`, both relative to the notebook `root`, never onto another
/// name. The open tabs under it follow; one that cannot undoes the whole move, and if the undo
/// fails, the move stands and `stuck` names the tabs left on their old paths.
pub(crate) fn move_folder(
    hwnd: HWND,
    root: &Path,
    old: &Path,
    new: &Path,
) -> Result<Moved, MoveError> {
    // Defence in depth: an empty or escaping path would move the notebook root or a folder
    // outside it.
    if !tree::is_plain_relative_folder(old) || !tree::is_plain_relative_folder(new) {
        return Err(MoveError::Failed(crate::FastPadError::Invariant(
            "not a folder inside the notebook",
        )));
    }
    let case_only = library::model::same_path(new, old);
    let (old_path, new_path) = (root.join(old), root.join(new));
    crate::platform::files::rename_no_replace(&old_path, &new_path)
        .map_err(|error| disk_error(error, case_only, &old_path))?;
    let tabs = library_host::tabs_under(hwnd, &old_path);
    let mut moved = Vec::with_capacity(tabs.len());
    let mut stuck = Vec::new();
    for (id, path, _) in tabs {
        let target = new_path.join(library::record_path(&old_path, &path));
        let rebound = unsafe { app_ptr(hwnd) }
            .is_some_and(|mut app| unsafe { app.as_mut() }.tabs.rebind_path(id, target).is_ok());
        if rebound {
            moved.push((id, path));
        } else {
            stuck.push(path);
        }
    }
    if !stuck.is_empty() && library_host::rename_folder_back(&new_path, &old_path).is_ok() {
        // Undone, so the tabs and the disk agree: the folder went back, then the tabs that moved.
        for (id, path) in moved.into_iter().rev() {
            if let Some(mut app) = unsafe { app_ptr(hwnd) } {
                let _ = unsafe { app.as_mut() }.tabs.rebind_path(id, path);
            }
        }
        return Err(MoveError::TabCantFollow);
    }
    library_host::reroot_save_folders(hwnd, &old_path, &new_path);
    with_state(hwnd, |state| state.rename_folder(old, new));
    library_host::save_local_soon(hwnd);
    library_host::schedule_write(hwnd);
    Ok(Moved {
        rebound: false,
        stuck,
    })
}

/// The notice for a drop onto a name `folder` already has (tree drag spec §5).
pub(crate) fn taken_notice(name: &str, folder: &str) -> String {
    format!("{name} already exists in {folder}. Nothing was moved.")
}

/// A drop's target folder as its notice names it: the notebook's name for the root.
fn folder_label(root: &Path, folder: &Path) -> String {
    match folder.file_name() {
        Some(name) => name.to_string_lossy().into_owned(),
        None => library_host::notebook_name(root),
    }
}

/// The source is gone: say so, and let a rescan catch the tree up.
fn gone(hwnd: HWND, name: &str) {
    push_notice(hwnd, format!("{name} no longer exists."));
    library_host::request_rescan(hwnd);
}

/// A drop in the tree (tree drag spec §3.4, §4, §5): moves `source` into `folder` (relative to
/// the notebook; empty for the root), keeping its name. A folder that refuses it does nothing; a
/// taken name, a vanished source, a refusal from Windows or a tab that cannot follow moves
/// nothing and says so. After a move the target folder and the folders above it expand, and
/// the moved row is selected. The focus stays where it is.
pub(crate) fn drop_into(hwnd: HWND, source: &RowKind, folder: &Path) {
    let Some(root) = library_host::folder(hwnd) else {
        return;
    };
    let (Some(old), Some(new)) = (
        tree_drag::source_path(source),
        tree_drag::destination(source, folder),
    ) else {
        return;
    };
    if !tree_drag::accepts(source, folder) {
        return;
    }
    let name = old
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let is_folder = matches!(source, RowKind::Folder(_));
    let (listed, taken) = with_state(hwnd, |state| {
        let listed = if is_folder {
            state.is_folder(old)
        } else {
            state.is_listed(old)
        };
        (listed, state.is_listed(&new))
    })
    .unwrap_or((false, false));
    if !listed {
        gone(hwnd, &name);
        return;
    }
    if taken {
        push_notice(hwnd, taken_notice(&name, &folder_label(&root, folder)));
        return;
    }
    let result = if is_folder {
        move_folder(hwnd, &root, old, &new)
    } else {
        move_note(hwnd, &root, old, &new)
    };
    let moved = match result {
        Ok(moved) => moved,
        Err(MoveError::Taken) => {
            push_notice(hwnd, taken_notice(&name, &folder_label(&root, folder)));
            return;
        }
        Err(MoveError::Missing(_)) => {
            gone(hwnd, &name);
            return;
        }
        Err(MoveError::TargetGone(error)) => {
            push_notice(hwnd, format!("Couldn't move {name}: {error}"));
            library_host::request_rescan(hwnd);
            return;
        }
        Err(MoveError::Failed(error)) => {
            push_notice(hwnd, format!("Couldn't move {name}: {error}"));
            return;
        }
        Err(MoveError::TabCantFollow) => {
            push_notice(
                hwnd,
                format!("Couldn't move {name}: another tab already has that file open."),
            );
            return;
        }
    };
    for ancestor in tree::ancestors(&new) {
        library_host::set_expanded(hwnd, &ancestor, true);
    }
    super::main_window::invalidate_title_strip(hwnd);
    let row = if is_folder {
        RowKind::Folder(new.clone())
    } else {
        RowKind::Note(new.clone())
    };
    side_panel::with_accessible_events(hwnd, || {
        side_panel::refresh(hwnd);
        notebook_view::select_row(hwnd, &row);
    });
    if !moved.stuck.is_empty() {
        push_notice(
            hwnd,
            library_host::rename_undo_failed_notice(&name, &new.to_string_lossy(), &moved.stuck),
        );
    }
}
