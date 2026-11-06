//! Rebuilding the Notebook view from the App (library, tabs and `folders.ini`), and the
//! window-level queries and selection calls the rest of the app makes on the view.

use super::*;
use crate::library::tree::{self, RowKind, TreeRow};
use crate::window::commands::CommandId;
use crate::window::library_host;
use crate::window::main_window::app_ptr;
use crate::window::panel_cursor::Cursor;
use crate::window::side_panel::ViewPaint;
use crate::window::tree_drag::DragSource;
use std::path::PathBuf;
use std::time::Instant;
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetFocus;
use windows_sys::Win32::UI::WindowsAndMessaging::{SendMessageW, WM_COMMAND};

/// What a rebuild read of one open notebook.
pub(super) struct RootSnapshot {
    pub(super) info: RootInfo,
    /// Its root row is expanded.
    pub(super) expanded: bool,
    /// Its tree's visible rows, relative to it at depth 0 for its top level; `None` while its
    /// state has not loaded, or with its root collapsed.
    pub(super) rows: Option<Vec<TreeRow>>,
    /// Its last load failed.
    pub(super) failed: bool,
    /// Its scan stopped at the limit.
    pub(super) truncated: bool,
    /// A folder of it moving to another drive, and its progress row's text.
    pub(super) moving: Option<(PathBuf, String)>,
}

/// What a rebuild read from the library, the tabs and `folders.ini`'s cache.
pub(super) struct Snapshot {
    pub(super) mode: Mode,
    /// The open notebooks, in sidebar order.
    pub(super) roots: Vec<RootSnapshot>,
    pub(super) recent: Vec<PathBuf>,
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
    let expansion = library_host::expansion_revision(hwnd);
    let roots = library_host::notebooks(hwnd)
        .into_iter()
        .map(|(id, folder)| {
            let truncated = library_host::with_notebook(hwnd, id, |state| state.truncated);
            RootKey {
                id,
                folder,
                loaded: truncated.is_some(),
                failed: library_host::load_failed_of(hwnd, id),
                expansion,
                root_expanded: library_host::root_expanded_of(hwnd, id),
                truncated: truncated.unwrap_or(false),
                moving: crate::window::cross_move_host::moving_row(hwnd, id),
            }
        })
        .collect();
    RebuildKey {
        roots,
        default: library_host::default_notebook(hwnd),
        changed_only: library_host::changed_only(hwnd),
    }
}

/// Reads everything the rows need, notebook by notebook. Each call borrows the App on its own,
/// never nested.
pub(super) fn snapshot(hwnd: HWND) -> Snapshot {
    let key = rebuild_key(hwnd);
    let editors_expanded = crate::window::main_window::open_editors_expanded(hwnd);
    if key.roots.is_empty() {
        return Snapshot {
            mode: Mode::NoNotebook,
            roots: Vec::new(),
            recent: library_host::recent_notebooks(hwnd),
            editors_expanded,
            key,
        };
    }
    let folders: Vec<PathBuf> = key.roots.iter().map(|root| root.folder.clone()).collect();
    let names = crate::library::local::display_names(&folders);
    let roots = key
        .roots
        .iter()
        .zip(names)
        .map(|(root, (name, hint))| {
            let id = root.id;
            let rows = root
                .root_expanded
                .then(|| {
                    library_host::with_notebook(hwnd, id, |state| {
                        if key.changed_only {
                            changed_rows(&state.tree, &state.local.changed)
                        } else {
                            let mut rows = flatten(&state.tree, &state.local.expanded);
                            mark_changed_rows(&mut rows, &state.local.changed);
                            rows
                        }
                    })
                })
                .flatten();
            let marker = (key.default == Some(id)).then_some(DEFAULT_MARKER_TIP);
            RootSnapshot {
                info: RootInfo {
                    id,
                    folder: root.folder.clone(),
                    label: RootLabel { name, hint, marker },
                    favorite: library_host::is_favorite_folder(hwnd, &root.folder),
                    folders_open: library_host::any_folder_expanded_of(hwnd, id),
                },
                expanded: root.root_expanded,
                rows,
                failed: root.failed,
                truncated: root.truncated,
                moving: root.moving.clone(),
            }
        })
        .collect();
    Snapshot {
        mode: Mode::Tree,
        roots,
        recent: Vec::new(),
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
        // The drag's notebook now at another folder (its slot reused for another notebook): a
        // row that happens to share a relative path there is not the same row.
        let folder_before = view.drag_notebook.and_then(|id| view.notebook_folder(id));
        view.apply(snapshot, names);
        view.invalidate();
        let folder_after = view.drag_notebook.and_then(|id| view.notebook_folder(id));
        let moved = folder_before.is_some_and(|before| {
            !folder_after
                .as_deref()
                .is_some_and(|after| crate::library::model::same_path(&before, after))
        });
        // A drag whose row went ends, as does one whose notebook closed: a row that happens to
        // share a relative path in another notebook is not the same row (tree drag spec §3.3).
        // A target folder that went is found again at the next move. A tab or files are not
        // tree rows: a rebuild never loses them.
        let rows = &view.rows;
        let notebook = view.drag_notebook;
        let find = |kind: &RowKind| notebook.and_then(|id| view_row_index(rows, id, kind));
        let lost = view.drag.as_mut().is_some_and(|drag| {
            if let DragSource::Row(kind) = &drag.source
                && (moved || find(kind).is_none())
            {
                return true;
            }
            if drag.target.as_ref().is_some_and(|folder| {
                !folder.as_os_str().is_empty() && find(&RowKind::Folder(folder.clone())).is_none()
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
    // The Pinned part follows the library's pins.
    editors_changed(hwnd);
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

/// The row for the active tab: its note inside the open notebook that holds it. `None` for an
/// untitled tab or a file of no open notebook.
pub(super) fn active_target(hwnd: HWND) -> Option<(NotebookId, RowKind)> {
    let path = unsafe { app_ptr(hwnd) }
        .and_then(|app| unsafe { app.as_ref() }.tabs.active()?.path.clone())?;
    let id = crate::window::library_host::notebook_of(hwnd, &path)?;
    let root = crate::window::library_host::notebook_folder(hwnd, id)?;
    crate::library::is_inside(&root, &path)
        .then(|| (id, RowKind::Note(crate::library::record_path(&root, &path))))
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
    if let Some((id, RowKind::Note(relative))) = &target {
        for folder in tree::ancestors(relative) {
            crate::window::library_host::set_expanded_of(hwnd, *id, &folder, true);
        }
    }
    if stale(hwnd) {
        rebuild(hwnd);
    }
    with_view(hwnd, |view| {
        if let Some(index) = target
            .as_ref()
            .and_then(|(id, kind)| view_row_index(&view.rows, *id, kind))
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

/// The selected row's notebook and what it shows, while the keyboard selection is on the tree.
fn selected_row(view: &NotebookView) -> Option<(NotebookId, TreeRow)> {
    if view.cursor != Cursor::Tree {
        return None;
    }
    match view.target(view.list.selected?) {
        Target::Row(id, row) => Some((id, row)),
        _ => None,
    }
}

/// The notebook the Notebook view puts first for `target_notebook` (multi-root notebooks spec
/// §5.1): while the view shows, the selected row's notebook. The view shares the panel with
/// Search and Tags, so a selection it keeps while they show does not count.
pub(crate) fn selected_notebook(hwnd: HWND) -> Option<NotebookId> {
    if crate::window::side_panel::current_view(hwnd) != crate::config::SidebarView::Notebook {
        return None;
    }
    with_view(hwnd, |view| selected_row(view).map(|(id, _)| id)).flatten()
}

/// The selected note's absolute path while the panel has the keyboard focus on the tree, so
/// palette and accelerator commands act on it rather than on the active tab (spec §6.3).
pub(crate) fn focused_note(hwnd: HWND) -> Option<PathBuf> {
    let (id, relative) = with_view(hwnd, |view| {
        if unsafe { GetFocus() } != view.panel {
            return None;
        }
        match selected_row(view)? {
            (
                id,
                TreeRow {
                    kind: RowKind::Note(relative),
                    ..
                },
            ) => Some((id, relative)),
            _ => None,
        }
    })
    .flatten()?;
    Some(crate::window::library_host::notebook_folder(hwnd, id)?.join(relative))
}

/// The selected folder row's notebook and path, relative to that notebook, while the panel has
/// the keyboard focus on the tree: Rename and Delete act on it (notebook folders spec §4.2,
/// §4.3).
pub(crate) fn focused_folder(hwnd: HWND) -> Option<(NotebookId, PathBuf)> {
    with_view(hwnd, |view| {
        if unsafe { GetFocus() } != view.panel {
            return None;
        }
        match selected_row(view)? {
            (
                id,
                TreeRow {
                    kind: RowKind::Folder(relative),
                    ..
                },
            ) => Some((id, relative)),
            _ => None,
        }
    })
    .flatten()
}

/// Where a new note goes from the selection (spec §6.7): the selected row's notebook and, relative
/// to it, a folder row's own folder or a note's parent; the notebook's root for its root row or a
/// placeholder. `None` for a draft row, no selection, or the keyboard selection outside the tree.
pub(crate) fn selected_place(hwnd: HWND) -> Option<(NotebookId, PathBuf)> {
    let (id, row) = with_view(hwnd, |view| selected_row(view)).flatten()?;
    let folder = match row.kind {
        RowKind::Folder(relative) => relative,
        RowKind::Note(relative) => relative.parent().map(PathBuf::from).unwrap_or_default(),
        RowKind::Root | RowKind::Placeholder(_) => PathBuf::new(),
        RowKind::Draft => return None,
    };
    Some((id, folder))
}

/// `selected_place` as an absolute folder: where an untitled tab's first save goes.
pub(crate) fn selected_folder(hwnd: HWND) -> Option<PathBuf> {
    let (id, folder) = selected_place(hwnd)?;
    let root = crate::window::library_host::notebook_folder(hwnd, id)?;
    Some(if folder.as_os_str().is_empty() {
        root
    } else {
        root.join(folder)
    })
}

/// Selects the row of the notebook `id` showing `kind` and scrolls it into view; false when no
/// row shows it.
pub(crate) fn select_row(hwnd: HWND, id: NotebookId, kind: &RowKind) -> bool {
    let selected = with_view(hwnd, |view| {
        let Some(index) = view_row_index(&view.rows, id, kind) else {
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

/// Where the row of the notebook `id` showing `kind` is, and the row that takes its place once
/// it and everything shown under it go (`tree::row_in_place_of`), by its notebook and kind. The
/// next row is never another notebook's (its root row follows a notebook's last row): the row
/// before takes the place then, so the target notebook stays the same.
pub(crate) fn row_in_place_of(
    hwnd: HWND,
    id: NotebookId,
    kind: &RowKind,
) -> Option<(usize, Option<(NotebookId, RowKind)>)> {
    with_view(hwnd, |view| {
        let index = view_row_index(&view.rows, id, kind)?;
        let next = tree::row_in_place_of(&view.rows, index)
            .filter(|row| row.notebook == id)
            .or_else(|| {
                index
                    .checked_sub(1)
                    .and_then(|before| view.rows.get(before))
            })
            .map(|row| (row.notebook, row.kind.clone()));
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
