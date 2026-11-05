//! The Notebook view (spec §6): the open notebook's folder tree in the side panel, with its
//! header, pins, type-ahead and the loading, no-notebook and empty states. The tree itself is
//! built on the scan worker (`LibraryState.tree`). This module flattens the expanded part into
//! rows, paints only the rows on screen, and turns clicks and keys into `open_note` calls.

use crate::document::DocumentId;
use crate::library::tree::{self, NoteTree, RowKind, TreeRow};
use crate::window::design::metrics::scale;
use crate::window::design::metrics::sidebar_row;
use crate::window::design::text_scale::scale_text;
use crate::window::drag_label::DragLabel;
use crate::window::icon_sets::images::IconImages;
use crate::window::inline_name::InlineName;
use crate::window::library_host::NotebookId;
use crate::window::open_editors::OpenEditors;
use crate::window::panel_cursor::Cursor;
use crate::window::row_list::RowListState;
use crate::window::tooltip::Tooltip;
use crate::window::tree_drag::Drag;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{HWND, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    DT_CENTER, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER,
};
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;

mod accessibility;
mod context_menu;
mod drag;
mod geometry;
mod input;
mod paint;
mod rebuild;

pub(crate) use context_menu::*;
pub(crate) use drag::*;
pub(crate) use input::*;
pub(crate) use paint::*;
pub(crate) use rebuild::*;

// Sizes at 96 DPI; everything is scaled with `design::metrics::scale`.
const INDENT: i32 = 12;
const LEFT_PAD: i32 = 8;
const GLYPH_BOX: i32 = 16;
const GAP: i32 = 6;
const PIN_BOX: i32 = 24;
const TYPE_AHEAD_RESET: Duration = Duration::from_secs(1);

/// Selects the root row of the open notebook `id` and scrolls it into view (opening a notebook
/// that is already open does this).
pub(crate) fn reveal_notebook(hwnd: HWND, id: NotebookId) {
    crate::window::side_panel::refresh(hwnd);
    with_view(hwnd, |view| {
        let root = view_row_index(&view.rows, id, &RowKind::Root)
            .or_else(|| view.rows.iter().position(|row| row.notebook == id));
        if let Some(index) = root {
            view.cursor = Cursor::Tree;
            view.select(index);
        }
    });
    crate::window::inline_name::place(hwnd);
}

pub(crate) const TRUNCATED_ROW: &str = "Showing the first 10,000 notes";

/// What a placeholder row says (multi-root notebooks spec §4.3).
pub(crate) const PLACEHOLDER_LOADING: &str = "Loading…";
pub(crate) const PLACEHOLDER_FAILED: &str = "Couldn't load. Retry";
pub(crate) const PLACEHOLDER_EMPTY: &str = "No notes yet";
/// The default root's marker's tooltip (spec §4.2).
pub(crate) const DEFAULT_MARKER_TIP: &str = "Default notebook";

/// What a root row's label shows (multi-root notebooks spec §4.2): the notebook's name, the
/// parent folder hint when two open notebooks share that name, and the default marker's tooltip
/// on the default notebook.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RootLabel {
    pub(crate) name: String,
    pub(crate) hint: Option<String>,
    pub(crate) marker: Option<&'static str>,
}

/// The root row label of the open notebook `id`, as the view last built it.
#[cfg(test)]
pub(crate) fn root_label(hwnd: HWND, id: NotebookId) -> Option<RootLabel> {
    with_view(hwnd, |view| view.root(id).map(|root| root.label.clone())).flatten()
}

/// What the view knows of one open notebook, for its root row (multi-root notebooks spec §4.2).
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RootInfo {
    pub(crate) id: NotebookId,
    pub(crate) folder: PathBuf,
    pub(crate) label: RootLabel,
    pub(crate) favorite: bool,
    /// Some folder of it is expanded: its toggle button offers Collapse all.
    pub(crate) folders_open: bool,
}

/// What one root's rows depend on besides its library state (multi-root notebooks spec §4.1).
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RootKey {
    pub(crate) id: NotebookId,
    pub(crate) folder: PathBuf,
    /// Its state has arrived from the worker.
    pub(crate) loaded: bool,
    /// Its last load failed (`library_host::load_failed_of`).
    pub(crate) failed: bool,
    /// `library_host::expansion_revision`, bumped whenever a folder or root is expanded or
    /// collapsed.
    pub(crate) expansion: u64,
    /// `library_host::root_expanded_of`.
    pub(crate) root_expanded: bool,
    /// Its scan stopped at the limit: a row after its last says so.
    pub(crate) truncated: bool,
}

/// The root `…` menu's "Set as default notebook", a local action on a reused command id.
pub(crate) const ROOT_MENU_SET_DEFAULT: crate::window::commands::CommandId =
    crate::window::commands::CommandId::ShowNotebookView;

pub(crate) const fn placeholder_text(placeholder: tree::Placeholder) -> &'static str {
    match placeholder {
        tree::Placeholder::Loading => PLACEHOLDER_LOADING,
        tree::Placeholder::Failed => PLACEHOLDER_FAILED,
        tree::Placeholder::Empty => PLACEHOLDER_EMPTY,
        tree::Placeholder::Truncated => TRUNCATED_ROW,
    }
}

/// The panel's timer while a drag is under way (tree drag spec §3.3).
pub(crate) const DRAG_TIMER: usize = 0x4452;

// The drag label (tree drag spec §3.2), at 96 DPI: its height, the padding at either end, and
// the widest its name gets before an ellipsis.
const LABEL_HEIGHT: i32 = 24;
const LABEL_PAD: i32 = 8;
const LABEL_MAX_TEXT: i32 = 300;

// Segoe MDL2 Assets, the font the title bar already uses.
const GLYPH_CHEVRON_RIGHT: &str = "\u{E76C}";
const GLYPH_CHEVRON_DOWN: &str = "\u{E70D}";
const GLYPH_FOLDER: &str = "\u{E8B7}";
/// The tilted pin's outline (Segoe's Pinned), needle included.
const GLYPH_PIN: &str = "\u{E840}";
/// The tilted pin's head fill (PinFill), with no needle: drawn under `GLYPH_PIN`.
const GLYPH_PINNED: &str = "\u{E842}";
/// Segoe's Home: the default notebook's marker after its root row's name.
const GLYPH_DEFAULT: &str = "\u{E80F}";
const GLYPH_ADD: &str = "\u{E710}";
const GLYPH_MORE: &str = "\u{E712}";
const GLYPH_NEW_FOLDER: &str = "\u{E8F4}";
const GLYPH_REFRESH: &str = "\u{E72C}";
/// ChevronUpMed and ChevronDownMed: collapse all and expand all.
const GLYPH_COLLAPSE_ALL: &str = "\u{E971}";
const GLYPH_EXPAND_ALL: &str = "\u{E972}";

// Tooltip tool IDs.
const TOOL_ROW: usize = 1;
const TOOL_TITLE: usize = 2;
/// The default root's marker.
const TOOL_DEFAULT: usize = 3;
const TOOL_NEW: usize = 4;
const TOOL_MORE: usize = 5;
const TOOL_NEW_FOLDER: usize = 6;
const TOOL_REFRESH: usize = 7;
const TOOL_TOGGLE_FOLDERS: usize = 8;

/// What the view shows (multi-root notebooks spec §4.1).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Mode {
    /// No notebook: "Open a notebook to see its notes.", "Open notebook…", "New notebook…" and
    /// the RECENT list.
    NoNotebook,
    /// One root row per open notebook, each with its tree or a placeholder under it.
    Tree,
}

/// What the Search view says about a target notebook whose load failed.
pub(crate) const LOAD_FAILED: &str = "Couldn't load this notebook.";

/// A root row's buttons (multi-root notebooks spec §4.2).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HeaderButton {
    NewNote,
    NewFolder,
    /// Scans the notebook's folder again.
    Refresh,
    /// Collapse all while a folder is open, Expand all when none is.
    ToggleFolders,
    More,
}

/// How a note row is being opened (spec §6.4).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Activation {
    /// A mouse click: the preview tab, focus stays in the tree (F2 and Del act on the row).
    Click,
    /// Enter, Ctrl+Enter or a double-click: a normal tab, focus to the editor.
    Permanent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RowPart {
    Chevron,
    Pin,
    Body,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Hit {
    /// The Open Editors header row: toggles the section.
    EditorsHeader,
    /// An Open Editors row; `close` on a clean tab's close box.
    Editor {
        index: usize,
        close: bool,
    },
    /// The root row of a notebook, off its buttons: toggles that notebook's tree.
    Root(NotebookId),
    /// A button of the root row of a notebook.
    Header(NotebookId, HeaderButton),
    /// "Open notebook…", with no notebook open.
    StateButton,
    /// "New notebook…" under "Open notebook…", with no notebook open.
    SecondButton,
    /// The scroll thumb, with where on it the press landed.
    Thumb(i32),
    Row {
        index: usize,
        part: RowPart,
    },
    Empty,
}

/// What a list index stands for right now.
#[derive(Clone, Debug)]
enum Target {
    Recent(PathBuf),
    /// A row of the open notebook `NotebookId`: its root row, a placeholder, a folder or a note.
    Row(NotebookId, TreeRow),
    Truncated,
    Nothing,
}

/// One row of the Notebook view's list (multi-root notebooks spec §4.1): a row of an open
/// notebook's tree, its root row or a placeholder, and the notebook it belongs to. Paths stay
/// relative to that notebook.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ViewRow {
    pub(crate) notebook: NotebookId,
    pub(crate) row: TreeRow,
}

impl std::ops::Deref for ViewRow {
    type Target = TreeRow;

    fn deref(&self) -> &TreeRow {
        &self.row
    }
}

impl AsRef<TreeRow> for ViewRow {
    fn as_ref(&self) -> &TreeRow {
        &self.row
    }
}

/// The index of the row of notebook `id` that shows `kind`.
pub(crate) fn view_row_index(rows: &[ViewRow], id: NotebookId, kind: &RowKind) -> Option<usize> {
    rows.iter()
        .position(|row| row.notebook == id && tree::same_kind(&row.kind, kind))
}

// windows-sys RECT is only Clone + Copy, so the layout structs are too.
#[derive(Clone, Copy)]
pub(crate) struct RowParts {
    pub chevron: RECT,
    pub icon: RECT,
    pub name: RECT,
    pub pin: RECT,
}

/// A tree row's pieces: the chevron (folders only) and icon, indented by `depth`, the name, and
/// the pin button at the right edge. Every rectangle stays inside `row`, even in a narrow panel.
pub(crate) fn row_parts(row: RECT, depth: u16, dpi: u32) -> RowParts {
    let glyph = scale(GLYPH_BOX, dpi);
    let chevron_left =
        (row.left + scale(LEFT_PAD, dpi) + i32::from(depth) * scale(INDENT, dpi)).min(row.right);
    let chevron = RECT {
        left: chevron_left,
        top: row.top,
        right: (chevron_left + glyph).min(row.right),
        bottom: row.bottom,
    };
    let icon = RECT {
        left: chevron.right,
        top: row.top,
        right: (chevron.right + glyph).min(row.right),
        bottom: row.bottom,
    };
    let pin_left = (row.right - scale(PIN_BOX, dpi)).max(icon.right);
    let pin = RECT {
        left: pin_left,
        top: row.top,
        right: row.right,
        bottom: row.bottom,
    };
    let name = RECT {
        left: (icon.right + scale(GAP, dpi)).min(pin_left),
        top: row.top,
        right: pin_left,
        bottom: row.bottom,
    };
    RowParts {
        chevron,
        icon,
        name,
        pin,
    }
}

#[derive(Clone, Copy)]
pub(crate) struct StateLayout {
    pub message: RECT,
    pub button: RECT,
    /// "New notebook…" (no notebook) or "Open notebook…" (failed load) under `button`.
    pub second: RECT,
    /// "RECENT", in the no-notebook state.
    pub label: RECT,
    /// The recent rows.
    pub list: RECT,
}

/// Where the no-notebook and empty states put their message, button and RECENT list.
pub(crate) fn state_layout(body: RECT, dpi: u32) -> StateLayout {
    let pad = scale(12, dpi);
    let left = body.left + pad;
    let right = (body.right - pad).max(left);
    let message = RECT {
        left,
        top: body.top + scale_text(8, dpi),
        right,
        bottom: body.top + scale_text(48, dpi),
    };
    let button = RECT {
        left,
        top: message.bottom + scale_text(4, dpi),
        right: (left + scale(140, dpi)).min(right),
        bottom: message.bottom + scale_text(32, dpi),
    };
    let second = RECT {
        top: button.bottom + scale_text(8, dpi),
        bottom: button.bottom + scale_text(8, dpi) + (button.bottom - button.top),
        ..button
    };
    // RECENT is painted only with no notebook, under both of its buttons.
    let label = RECT {
        left,
        top: second.bottom + scale_text(16, dpi),
        right,
        bottom: second.bottom + scale_text(36, dpi),
    };
    let list = RECT {
        left: body.left,
        top: label.bottom.min(body.bottom),
        right: body.right,
        bottom: body.bottom,
    };
    StateLayout {
        message,
        button,
        second,
        label,
        list,
    }
}

/// The row that stands for `kind` of notebook `id` after a rebuild: the same path in the same
/// notebook if it is still there, else the row that took its index (clamped to the list).
/// Nothing selected stays nothing.
pub(crate) fn follow(
    rows: &[ViewRow],
    kind: Option<(NotebookId, &RowKind)>,
    old: Option<usize>,
) -> Option<usize> {
    let (id, kind) = kind?;
    if let Some(index) = view_row_index(rows, id, kind) {
        return Some(index);
    }
    let last = rows.len().checked_sub(1)?;
    Some(old.unwrap_or(0).min(last))
}

/// How `flatten` keys an expanded folder: the same lowercasing as `model::same_path`.
fn expanded_key(path: &Path) -> String {
    path.as_os_str().to_string_lossy().to_lowercase()
}

/// The visible rows of `tree` with `expanded` folders open. The expanded set is hashed once, so
/// each folder row costs one lookup, not a scan of every expanded entry.
pub(crate) fn flatten(tree: &NoteTree, expanded: &[PathBuf]) -> Vec<TreeRow> {
    let open: HashSet<String> = expanded.iter().map(|path| expanded_key(path)).collect();
    tree.rows(&|path: &Path| !open.is_empty() && open.contains(&expanded_key(path)))
}

/// Marks the rows of notes changed outside FastPad, and of collapsed folders that hold one
/// (second brain phase 1 spec §5.5). `changed` is relative to the notebook.
pub(crate) fn mark_changed_rows(rows: &mut [TreeRow], changed: &[PathBuf]) {
    if changed.is_empty() {
        return;
    }
    let keys: HashSet<String> = changed.iter().map(|path| expanded_key(path)).collect();
    for row in rows {
        match &row.kind {
            RowKind::Note(path) => row.changed = keys.contains(&expanded_key(path)),
            RowKind::Folder(path) if !row.expanded => {
                let prefix = format!("{}\\", expanded_key(path));
                row.changed = keys.iter().any(|key| key.starts_with(&prefix));
            }
            _ => {}
        }
    }
}

/// The rows "Notes: Show changed notes" shows: the marked notes, each under the folders it is
/// in, those folders open.
pub(crate) fn changed_rows(tree: &NoteTree, changed: &[PathBuf]) -> Vec<TreeRow> {
    let folders: Vec<PathBuf> = changed
        .iter()
        .flat_map(|path| tree::ancestors(path))
        .collect();
    let folder_keys: HashSet<String> = folders.iter().map(|path| expanded_key(path)).collect();
    let mut rows = flatten(tree, &folders);
    mark_changed_rows(&mut rows, changed);
    rows.retain(|row| match &row.kind {
        RowKind::Note(_) => row.changed,
        RowKind::Folder(path) => folder_keys.contains(&expanded_key(path)),
        RowKind::Draft | RowKind::Root | RowKind::Placeholder(_) => false,
    });
    rows
}

/// Letters typed into the tree within a second of each other form one prefix.
#[derive(Debug, Default)]
pub(crate) struct TypeAhead {
    text: String,
    at: Option<Instant>,
}

impl TypeAhead {
    pub(crate) fn push(&mut self, ch: char, now: Instant) -> &str {
        if self
            .at
            .is_none_or(|at| now.duration_since(at) >= TYPE_AHEAD_RESET)
        {
            self.text.clear();
        }
        self.text.push(ch);
        self.at = Some(now);
        &self.text
    }
}

/// Everything the rows depend on besides the library itself, whose changes always come through
/// `side_panel::refresh` (a full rebuild): one key per open notebook, in sidebar order, the
/// default notebook and the changed-notes filter. A tab switch that leaves this unchanged
/// re-selects a row without flattening the trees again.
#[derive(Clone, Debug, Eq, PartialEq)]
struct RebuildKey {
    roots: Vec<RootKey>,
    /// The default notebook, whose root row shows the marker.
    default: Option<NotebookId>,
    /// `library_host::changed_only`: the view shows only notes changed outside FastPad, in
    /// every notebook.
    changed_only: bool,
}

/// The Notebook view's state, owned by `side_panel::Sidebar`.
pub(crate) struct NotebookView {
    pub(crate) panel: HWND,
    pub(crate) mode: Mode,
    /// Root by root: each open notebook's root row, then its tree or one placeholder while it
    /// is expanded (multi-root notebooks spec §4.1).
    pub(crate) rows: Vec<ViewRow>,
    pub(crate) list: RowListState,
    /// The no-notebook state's RECENT notebooks and their display names.
    pub(crate) recent: Vec<PathBuf>,
    recent_names: Vec<(String, Option<String>)>,
    /// The open notebooks, in sidebar order, as the rows were last built.
    pub(crate) roots: Vec<RootInfo>,
    /// The root row button (or state button) under the pointer.
    hover: Option<Hit>,
    /// The pointer is over the hovered row's pin button.
    pub(crate) hover_pin: bool,
    tooltip: Option<Tooltip>,
    tooltip_failed: bool,
    typed: TypeAhead,
    thumb_grab: Option<i32>,
    /// A drag of a tree or Open Editors row, armed by a press and under way past the drag
    /// distance, which moves or copies (tree drag spec §3, open editors spec §4.3); or an
    /// Explorer drag over the panel.
    pub(crate) drag: Option<Drag>,
    /// The notebook `drag` moves or copies within (multi-root notebooks spec §4.5).
    pub(crate) drag_notebook: Option<NotebookId>,
    /// The right press that cancelled a drag: its release opens no menu.
    pub(crate) eat_right_up: bool,
    /// The label following the pointer while a drag is under way (tree drag spec §3.2).
    pub(crate) drag_label: Option<DragLabel>,
    tracking_leave: bool,
    /// What the rows were last built from (`None` before the first rebuild).
    built: Option<RebuildKey>,
    /// Bumped whenever a rebuild changes the rows' order or the RECENT list, so screen readers
    /// hear a reorder even at the same count (`AccessibleView::accessible_generation`).
    order: u64,
    /// The inline name field and its edit (inline naming spec §3).
    pub(crate) inline: InlineName,
    /// The tree's Material bitmaps, made on first draw (icon sets spec §6).
    images: IconImages,
    /// The Open Editors section's rows (open editors spec §3.2).
    pub(crate) editors: OpenEditors,
    /// The Open Editors section is expanded (`main_window::open_editors_expanded`).
    editors_expanded: bool,
    /// The tab under a middle press on an Open Editors row: its release there closes it.
    middle_press: Option<DocumentId>,
    /// The one keyboard selection through the Open Editors rows and the tree (open editors spec
    /// §3.5). `Cursor::Tree` leaves it to `list.selected`.
    pub(crate) cursor: Cursor,
    /// Full rebuilds so far, for the tests that check a tab switch skips one.
    #[cfg(test)]
    pub(crate) rebuilds: usize,
}

impl std::fmt::Debug for NotebookView {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NotebookView")
            .field("mode", &self.mode)
            .field("rows", &self.rows.len())
            .field("selected", &self.list.selected)
            .finish_non_exhaustive()
    }
}

const fn height(rect: RECT) -> i32 {
    let height = rect.bottom - rect.top;
    if height > 0 { height } else { 0 }
}

const fn contains(rect: RECT, x: i32, y: i32) -> bool {
    x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom
}

const LINE: u32 = DT_SINGLELINE | DT_VCENTER | DT_LEFT | DT_END_ELLIPSIS | DT_NOPREFIX;
const CENTERED: u32 = DT_SINGLELINE | DT_VCENTER | DT_CENTER | DT_NOPREFIX;

/// A row of the notebook `notebook` at `depth`.
fn view_row(notebook: NotebookId, kind: RowKind, depth: u16, name: String) -> ViewRow {
    ViewRow {
        notebook,
        row: TreeRow {
            kind,
            depth,
            name,
            pinned: false,
            expanded: false,
            changed: false,
        },
    }
}

/// A placeholder row under the root of `notebook` (multi-root notebooks spec §4.3).
fn placeholder_row(notebook: NotebookId, placeholder: tree::Placeholder) -> ViewRow {
    view_row(
        notebook,
        RowKind::Placeholder(placeholder),
        1,
        placeholder_text(placeholder).to_owned(),
    )
}

impl NotebookView {
    pub(crate) fn new(panel: HWND) -> Self {
        let dpi = unsafe { GetDpiForWindow(panel) }.max(96);
        Self {
            panel,
            mode: Mode::NoNotebook,
            rows: Vec::new(),
            list: RowListState::new(sidebar_row(dpi)),
            recent: Vec::new(),
            recent_names: Vec::new(),
            roots: Vec::new(),
            hover: None,
            hover_pin: false,
            tooltip: None,
            tooltip_failed: false,
            typed: TypeAhead::default(),
            thumb_grab: None,
            drag: None,
            drag_notebook: None,
            eat_right_up: false,
            drag_label: None,
            tracking_leave: false,
            built: None,
            order: 0,
            inline: InlineName::new(),
            images: IconImages::new(),
            editors: OpenEditors::new(sidebar_row(dpi)),
            editors_expanded: true,
            middle_press: None,
            cursor: Cursor::Tree,
            #[cfg(test)]
            rebuilds: 0,
        }
    }

    fn target(&self, index: usize) -> Target {
        match self.mode {
            Mode::NoNotebook => self
                .recent
                .get(index)
                .cloned()
                .map_or(Target::Nothing, Target::Recent),
            Mode::Tree => match self.rows.get(index) {
                Some(ViewRow {
                    row:
                        TreeRow {
                            kind: RowKind::Placeholder(tree::Placeholder::Truncated),
                            ..
                        },
                    ..
                }) => Target::Truncated,
                Some(row) => Target::Row(row.notebook, row.row.clone()),
                None => Target::Nothing,
            },
        }
    }

    /// The open notebook `id`'s root row information.
    pub(crate) fn root(&self, id: NotebookId) -> Option<&RootInfo> {
        self.roots.iter().find(|root| root.id == id)
    }

    fn select(&mut self, index: usize) {
        let height = self.list_height();
        self.list.select(index, height);
        self.invalidate();
    }

    /// The rows, root by root (multi-root notebooks spec §4.1): each notebook's root row and,
    /// while it is expanded, its tree one level under it, or one placeholder. The inline edit is
    /// fitted to its own notebook's rows; it ends when that notebook, its folder or its row is
    /// no longer shown (inline naming spec §5.4).
    fn build_rows(&mut self, roots: Vec<RootSnapshot>) -> Vec<ViewRow> {
        let edit_root = self.inline.root().map(Path::to_path_buf);
        let mut fitted = edit_root.is_none();
        let mut rows = Vec::new();
        for root in roots {
            let id = root.info.id;
            let mut root_row = view_row(id, RowKind::Root, 0, root.info.label.name.clone());
            root_row.row.expanded = root.expanded;
            rows.push(root_row);
            if !root.expanded {
                continue;
            }
            let Some(mut tree_rows) = root.rows else {
                let placeholder = if root.failed {
                    tree::Placeholder::Failed
                } else {
                    tree::Placeholder::Loading
                };
                rows.push(placeholder_row(id, placeholder));
                continue;
            };
            let edited = edit_root
                .as_deref()
                .is_some_and(|edited| crate::library::model::same_path(edited, &root.info.folder));
            if edited && self.inline.fit(&mut tree_rows) {
                fitted = true;
                self.inline.shift(rows.len());
            }
            if tree_rows.is_empty() {
                rows.push(placeholder_row(id, tree::Placeholder::Empty));
            }
            rows.extend(tree_rows.into_iter().map(|mut row| {
                row.depth = row.depth.saturating_add(1);
                ViewRow { notebook: id, row }
            }));
            // The scan stopped at its limit: said after its root's last row.
            if root.truncated {
                rows.push(placeholder_row(id, tree::Placeholder::Truncated));
            }
        }
        if !fitted {
            self.inline.end();
        }
        rows
    }

    fn apply(&mut self, snapshot: Snapshot, names: Vec<(String, Option<String>)>) {
        let reset = snapshot.mode != self.mode;
        if reset {
            self.list = RowListState::new(self.list.row_height);
            self.typed = TypeAhead::default();
        }
        let selected = self
            .list
            .selected
            .and_then(|index| self.rows.get(index))
            .map(|row| (row.notebook, row.kind.clone()));
        let top = (!reset)
            .then(|| {
                self.rows
                    .get(self.list.top)
                    .map(|row| (row.notebook, row.kind.clone()))
            })
            .flatten();
        let (old_selected, old_top) = (self.list.selected, self.list.top);
        let infos: Vec<RootInfo> = snapshot
            .roots
            .iter()
            .map(|root| root.info.clone())
            .collect();
        let rows = self.build_rows(snapshot.roots);
        let reordered = reset
            || snapshot.recent != self.recent
            || rows.len() != self.rows.len()
            || rows
                .iter()
                .zip(&self.rows)
                .any(|(new, old)| new.notebook != old.notebook || new.kind != old.kind);
        if reordered {
            self.order = self.order.wrapping_add(1);
        }
        self.mode = snapshot.mode;
        let expanding = snapshot.editors_expanded && !self.editors_expanded;
        self.editors_expanded = snapshot.editors_expanded;
        // A tab row's keyboard selection goes with its section to the Open Editors header,
        // which is always there.
        if matches!(self.cursor, Cursor::Editor(_)) && !self.editors_expanded {
            self.cursor = Cursor::EditorsHeader;
        }
        // The section's rows fit its height again, the active row in view once it shows.
        self.fit_editors(expanding);
        self.roots = infos;
        self.rows = rows;
        self.recent = snapshot.recent;
        self.recent_names = names;
        self.built = Some(snapshot.key);
        #[cfg(test)]
        {
            self.rebuilds += 1;
        }
        let count = match self.mode {
            Mode::Tree => self.rows.len(),
            Mode::NoNotebook => self.recent.len(),
        };
        self.list.set_count(count);
        // Measured in the new mode's list area.
        let height = self.list_height();
        if self.mode == Mode::Tree {
            self.list.selected = self.follow_selection(selected.as_ref(), old_selected);
            self.list.top = follow(
                &self.rows,
                top.as_ref().map(|(id, kind)| (*id, kind)),
                Some(old_top),
            )
            .unwrap_or(0);
            self.list.scroll_lines(0, height);
        }
        self.hover_pin = false;
    }

    /// Where the selection goes after a rebuild: its row if still shown; the root row of its
    /// notebook when a collapsed root hid it (open editors spec §3.5); else the row that took
    /// its place.
    fn follow_selection(
        &self,
        selected: Option<&(NotebookId, RowKind)>,
        old: Option<usize>,
    ) -> Option<usize> {
        let (id, kind) = selected?;
        if view_row_index(&self.rows, *id, kind).is_none()
            && let Some(root) = view_row_index(&self.rows, *id, &RowKind::Root)
            && !self.rows[root].expanded
        {
            return Some(root);
        }
        follow(&self.rows, Some((*id, kind)), old)
    }
}

#[cfg(test)]
mod tests;
