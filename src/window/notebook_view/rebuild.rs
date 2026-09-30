//! Rebuilding the Notebook view from the App (library, tabs and `folders.ini`), and the
//! window-level queries and selection calls the rest of the app makes on the view.

use super::*;
use crate::library::tree::{self, RowKind, TreeRow};
use crate::window::commands::CommandId;
use crate::window::main_window::app_ptr;
use crate::window::panel_cursor::Cursor;
use crate::window::side_panel::ViewPaint;
use crate::window::tree_drag::DragSource;
use std::path::PathBuf;
use std::time::Instant;
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetFocus;
use windows_sys::Win32::UI::WindowsAndMessaging::{SendMessageW, WM_COMMAND};

/// What a rebuild read from the library, the tabs and `folders.ini`'s cache.
pub(super) struct Snapshot {
    pub(super) mode: Mode,
    pub(super) rows: Vec<TreeRow>,
    pub(super) truncated: bool,
    pub(super) recent: Vec<PathBuf>,
    pub(super) root: Option<PathBuf>,
    pub(super) favorite: bool,
    /// Some folder is expanded.
    pub(super) folders_open: bool,
    /// The notebook's root row is expanded (true without a notebook).
    pub(super) root_expanded: bool,
    /// The Open Editors section is expanded.
    pub(super) editors_expanded: bool,
    pub(super) key: RebuildKey,
}

pub(crate) fn with_view<R>(hwnd: HWND, f: impl FnOnce(&mut NotebookView) -> R) -> Option<R> {
    unsafe { app_ptr(hwnd) }.and_then(|mut app| {
        unsafe { app.as_mut() }
            .sidebar
            .as_mut()
            .map(|sidebar| f(&mut sidebar.notebook))
    })
}

/// What the rows would be built from now. Cheap: no flattening, no disk.
pub(super) fn rebuild_key(hwnd: HWND) -> RebuildKey {
    RebuildKey {
        root: crate::window::library_host::folder(hwnd),
        loaded: crate::window::library_host::with_state(hwnd, |_| ()).is_some(),
        failed: crate::window::library_host::load_failed(hwnd),
        expansion: crate::window::library_host::expansion_revision(hwnd),
        root_expanded: crate::window::library_host::root_expanded(hwnd),
    }
}

/// Reads everything the rows need. Each call borrows the App on its own, never nested.
pub(super) fn snapshot(hwnd: HWND) -> Snapshot {
    let key = rebuild_key(hwnd);
    let editors_expanded = crate::window::main_window::open_editors_expanded(hwnd);
    let Some(root) = key.root.clone() else {
        return Snapshot {
            mode: Mode::NoNotebook,
            rows: Vec::new(),
            truncated: false,
            recent: crate::window::library_host::recent_notebooks(hwnd),
            root: None,
            favorite: false,
            folders_open: false,
            root_expanded: true,
            editors_expanded,
            key,
        };
    };
    let favorite = crate::window::library_host::is_favorite(hwnd);
    let built = crate::window::library_host::with_state(hwnd, |state| {
        let rows = flatten(&state.tree, &state.local.expanded);
        (rows, state.truncated)
    });
    let (mode, rows, truncated) = match built {
        None if key.failed => (Mode::Failed, Vec::new(), false),
        None => (Mode::Loading, Vec::new(), false),
        Some((rows, _)) if rows.is_empty() => (Mode::Empty, rows, false),
        Some((rows, truncated)) => (Mode::Tree, rows, truncated),
    };
    Snapshot {
        mode,
        rows,
        truncated,
        recent: Vec::new(),
        root: Some(root),
        favorite,
        folders_open: crate::window::library_host::any_folder_expanded(hwnd),
        root_expanded: key.root_expanded,
        editors_expanded,
        key,
    }
}

/// Rebuilds the rows from the library, the tabs and the notebook lists, keeping the selection
/// and the scroll position by path. `side_panel::refresh` calls it.
pub(crate) fn rebuild(hwnd: HWND) {
    let snapshot = snapshot(hwnd);
    let names = crate::library::local::display_names(&snapshot.recent);
    let lost = with_view(hwnd, |view| {
        // The notebook itself changed under the drag (root switched): a row that happens to
        // share a relative path in the new notebook is not the same row (tree drag spec §3.3).
        let root_changed = snapshot.root != view.root;
        view.apply(snapshot, names);
        view.invalidate();
        // A drag whose row went ends; a target folder that went is found again at the next
        // move. A tab or files are not tree rows: a rebuild never loses them.
        let rows = &view.rows;
        let lost = view.drag.as_mut().is_some_and(|drag| {
            if let DragSource::Row(kind) = &drag.source
                && (root_changed || tree::row_index(rows, kind).is_none())
            {
                return true;
            }
            if drag.target.as_ref().is_some_and(|folder| {
                !folder.as_os_str().is_empty()
                    && tree::row_index(rows, &RowKind::Folder(folder.clone())).is_none()
            }) {
                drag.target = None;
            }
            false
        });
        // The rows moved: a resting folder's timer starts over at the next move, once it is
        // known to still be under the pointer.
        if !lost && let Some(drag) = view.drag.as_mut() {
            drag.resting = None;
        }
        lost
    })
    .unwrap_or(false);
    if lost {
        cancel_drag(hwnd);
    }
    // A started drag's band and cursor follow the rows that moved under its pointer.
    retarget_drag(hwnd, Instant::now());
    // The field follows its row, or goes with an edit the rebuild ended (inline naming spec §5.4).
    crate::window::inline_name::place(hwnd);
}

/// The tabs changed in some way the Open Editors rows show (a tab opened, closed, switched,
/// renamed, saved, made dirty or clean): the rows follow, and the panel repaints only if they
/// changed. Cheap: the tab list in memory, no rebuild of the tree.
pub(crate) fn editors_changed(hwnd: HWND) {
    let rows = crate::window::open_editors::snapshot(hwnd);
    let changed = with_view(hwnd, |view| {
        let reordered = rows.len() != view.editors.rows.len()
            || rows
                .iter()
                .zip(&view.editors.rows)
                .any(|(new, old)| new.key() != old.key());
        let changed = view.editors.set_rows(rows);
        if reordered {
            // Screen readers hear the reorder (`accessible_generation`).
            view.order = view.order.wrapping_add(1);
        }
        // A tab row that went takes the keyboard selection to the row in its place.
        if let Cursor::Editor(index) = view.cursor
            && index >= view.editors.rows.len()
        {
            view.cursor = match view.editors.rows.len().checked_sub(1) {
                Some(last) => Cursor::Editor(last),
                None => Cursor::EditorsHeader,
            };
        }
        if changed {
            view.fit_editors(true);
            view.invalidate();
        }
        changed
    })
    .unwrap_or(false);
    // A row more or less moves the tree, and an inline field with it (inline naming spec §5.4).
    if changed {
        crate::window::inline_name::place(hwnd);
    }
}

/// The row for the active tab: its note inside the open notebook. `None` for an untitled tab.
pub(super) fn active_target(hwnd: HWND) -> Option<RowKind> {
    let root = crate::window::library_host::folder(hwnd)?;
    let path = unsafe { app_ptr(hwnd) }
        .and_then(|app| unsafe { app.as_ref() }.tabs.active()?.path.clone())?;
    crate::library::is_inside(&root, &path)
        .then(|| RowKind::Note(crate::library::record_path(&root, &path)))
}

/// Whether something the rows are built from, besides the library itself, changed since the
/// last rebuild: another notebook, its state arriving, or a folder expanded or collapsed. Cheap:
/// no flattening.
pub(crate) fn stale(hwnd: HWND) -> bool {
    let key = rebuild_key(hwnd);
    with_view(hwnd, |view| view.built.as_ref() != Some(&key)).unwrap_or(false)
}

/// Every tab switch: the active note's row is selected and its folders expand (remembered per
/// PC), without moving the keyboard focus (spec §6.1). The tree is flattened again only when
/// something the rows depend on changed (a folder newly expanded, another notebook); otherwise
/// the row is just selected.
pub(crate) fn active_tab_changed(hwnd: HWND) {
    editors_changed(hwnd);
    let target = active_target(hwnd);
    if let Some(RowKind::Note(relative)) = &target {
        for folder in tree::ancestors(relative) {
            crate::window::library_host::set_expanded(hwnd, &folder, true);
        }
    }
    if stale(hwnd) {
        rebuild(hwnd);
    }
    with_view(hwnd, |view| {
        if let Some(index) = target
            .as_ref()
            .and_then(|kind| tree::row_index(&view.rows, kind))
        {
            view.select(index);
        }
        view.invalidate();
    });
    crate::window::inline_name::place(hwnd);
}

/// The panel's `WM_PAINT` while the Notebook view shows (`side_panel::paint_view`). The panel
/// has already filled its background.
pub(crate) fn paint(hwnd: HWND, paint: &ViewPaint) {
    with_view(hwnd, |view| view.paint(paint));
}

/// Runs a main-window command as the menus do.
pub(super) fn run(hwnd: HWND, command: CommandId) {
    unsafe {
        SendMessageW(hwnd, WM_COMMAND, command as usize, 0);
    }
}

/// The selected note's absolute path while the panel has the keyboard focus on the tree, so
/// palette and accelerator commands act on it rather than on the active tab (spec §6.3).
pub(crate) fn focused_note(hwnd: HWND) -> Option<PathBuf> {
    let root = crate::window::library_host::folder(hwnd)?;
    with_view(hwnd, |view| {
        let focused = unsafe { GetFocus() } == view.panel && view.cursor == Cursor::Tree;
        match view.list.selected.map(|index| view.target(index)) {
            Some(Target::Row(TreeRow {
                kind: RowKind::Note(relative),
                ..
            })) if focused => Some(root.join(relative)),
            _ => None,
        }
    })
    .flatten()
}

/// The selected folder row's path, relative to the notebook, while the panel has the keyboard
/// focus on the tree: Rename and Delete act on it (notebook folders spec §4.2, §4.3).
pub(crate) fn focused_folder(hwnd: HWND) -> Option<PathBuf> {
    with_view(hwnd, |view| {
        let focused = unsafe { GetFocus() } == view.panel && view.cursor == Cursor::Tree;
        match view.list.selected.map(|index| view.target(index)) {
            Some(Target::Row(TreeRow {
                kind: RowKind::Folder(relative),
                ..
            })) if focused => Some(relative),
            _ => None,
        }
    })
    .flatten()
}

/// The folder a new note goes to (spec §6.7): a selected folder row's own folder, or a
/// selected note's parent. `None` (the root) for a draft row, no selection, or the keyboard
/// selection outside the tree.
pub(crate) fn selected_folder(hwnd: HWND) -> Option<PathBuf> {
    let root = crate::window::library_host::folder(hwnd)?;
    let target = with_view(hwnd, |view| {
        (view.cursor == Cursor::Tree)
            .then(|| view.list.selected.map(|index| view.target(index)))
            .flatten()
    })
    .flatten()?;
    match target {
        Target::Row(TreeRow {
            kind: RowKind::Folder(relative),
            ..
        }) => Some(root.join(relative)),
        Target::Row(TreeRow {
            kind: RowKind::Note(relative),
            ..
        }) => Some(root.join(relative).parent()?.to_path_buf()),
        _ => None,
    }
}

/// Selects the row showing `kind` and scrolls it into view; false when no row shows it.
pub(crate) fn select_row(hwnd: HWND, kind: &RowKind) -> bool {
    let selected = with_view(hwnd, |view| {
        let Some(index) = tree::row_index(&view.rows, kind) else {
            return false;
        };
        view.select(index);
        true
    })
    .unwrap_or(false);
    // The field moves with its row (inline naming spec §5.4).
    crate::window::inline_name::place(hwnd);
    selected
}

/// Gives the tree the keyboard focus, after an inline name edit ended with Enter or Esc
/// (inline naming spec §5.1).
pub(crate) fn focus_tree(hwnd: HWND) {
    with_view(hwnd, |view| {
        view.cursor = Cursor::Tree;
        view.invalidate();
    });
    focus_panel(hwnd);
}

/// Where the row showing `kind` is, and the kind of the row that takes its place once it and
/// everything shown under it go (`tree::row_in_place_of`).
pub(crate) fn row_in_place_of(hwnd: HWND, kind: &RowKind) -> Option<(usize, Option<RowKind>)> {
    with_view(hwnd, |view| {
        let index = tree::row_index(&view.rows, kind)?;
        let next = tree::row_in_place_of(&view.rows, index).map(|row| row.kind.clone());
        Some((index, next))
    })
    .flatten()
}

/// Selects row `index` (the last row when past the end) and scrolls it into view.
pub(crate) fn select_index(hwnd: HWND, index: usize) {
    with_view(hwnd, |view| {
        if view.mode == Mode::Tree {
            view.select(index);
        }
    });
    crate::window::inline_name::place(hwnd);
}
