//! The Notebook view (spec §6): the open notebook's folder tree in the side panel, with its
//! header, pins, type-ahead and the loading, no-notebook and empty states. The tree itself is
//! built on the scan worker (`LibraryState.tree`). This module flattens the expanded part into
//! rows, paints only the rows on screen, and turns clicks and keys into `open_note` calls.

use crate::document::DocumentId;
use crate::library::tree::{self, NoteTree, RowKind, TreeRow};
use crate::window::drag_label::DragLabel;
use crate::window::icon_sets::images::IconImages;
use crate::window::inline_name::InlineName;
use crate::window::notebook_layout::ROW_HEIGHT;
use crate::window::open_editors::OpenEditors;
use crate::window::panel::scale;
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

// Sizes at 96 DPI; everything is scaled with `panel::scale`.
const INDENT: i32 = 12;
const LEFT_PAD: i32 = 8;
const GLYPH_BOX: i32 = 16;
const GAP: i32 = 6;
const PIN_BOX: i32 = 24;
const TYPE_AHEAD_RESET: Duration = Duration::from_secs(1);

pub(crate) const TRUNCATED_ROW: &str = "Showing the first 10,000 notes";

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
const GLYPH_STAR: &str = "\u{E734}";
const GLYPH_STAR_FILLED: &str = "\u{E735}";
const GLYPH_ADD: &str = "\u{E710}";
const GLYPH_MORE: &str = "\u{E712}";
const GLYPH_NEW_FOLDER: &str = "\u{E8F4}";

// Tooltip tool IDs.
const TOOL_ROW: usize = 1;
const TOOL_TITLE: usize = 2;
const TOOL_FAVORITE: usize = 3;
const TOOL_NEW: usize = 4;
const TOOL_MORE: usize = 5;
const TOOL_NEW_FOLDER: usize = 6;

/// What the view shows.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Mode {
    /// No notebook: "Open a notebook to see its notes.", a button and the RECENT list.
    NoNotebook,
    /// A notebook is open but its state has not arrived from the worker.
    Loading,
    /// Loaded, with no notes.
    Empty,
    Tree,
    /// The notebook's load failed: a message, "Retry" and "Open notebook…".
    Failed,
}

pub(crate) const LOAD_FAILED: &str = "Couldn't load this notebook.";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HeaderButton {
    Favorite,
    NewNote,
    NewFolder,
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
    /// The root row's chevron or name: toggles the tree.
    Root,
    /// A root row button.
    Header(HeaderButton),
    /// "Open notebook…" (no notebook), "New note" (empty notebook) or "Retry" (failed load).
    StateButton,
    /// "Open notebook…" under "Retry", after a failed load.
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
    Row(TreeRow),
    Truncated,
    Nothing,
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
    /// "Open notebook…" under "Retry", in the failed state.
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
        top: body.top + scale(8, dpi),
        right,
        bottom: body.top + scale(48, dpi),
    };
    let button = RECT {
        left,
        top: message.bottom + scale(4, dpi),
        right: (left + scale(140, dpi)).min(right),
        bottom: message.bottom + scale(32, dpi),
    };
    let second = RECT {
        top: button.bottom + scale(8, dpi),
        bottom: button.bottom + scale(8, dpi) + (button.bottom - button.top),
        ..button
    };
    let label = RECT {
        left,
        top: button.bottom + scale(16, dpi),
        right,
        bottom: button.bottom + scale(36, dpi),
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

/// The row that stands for `kind` after a rebuild: the same path if it is still there, else the
/// row that took its index (clamped to the list). Nothing selected stays nothing.
pub(crate) fn follow(
    rows: &[TreeRow],
    kind: Option<&RowKind>,
    old: Option<usize>,
) -> Option<usize> {
    let kind = kind?;
    if let Some(index) = tree::row_index(rows, kind) {
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
/// `side_panel::refresh` (a full rebuild). A tab switch that leaves this unchanged re-selects a
/// row without flattening the tree again.
#[derive(Clone, Debug, Eq, PartialEq)]
struct RebuildKey {
    root: Option<PathBuf>,
    loaded: bool,
    /// The last load failed (`library_host::load_failed`).
    failed: bool,
    /// `library_host::expansion_revision`, bumped whenever a folder is expanded or collapsed.
    expansion: u64,
    /// `library_host::root_expanded`: collapsing the root row is a rebuild.
    root_expanded: bool,
}

/// The Notebook view's state, owned by `side_panel::Sidebar`.
pub(crate) struct NotebookView {
    pub(crate) panel: HWND,
    pub(crate) mode: Mode,
    pub(crate) rows: Vec<TreeRow>,
    /// The scan stopped at its limit: one more row, after the last, says so.
    pub(crate) truncated: bool,
    pub(crate) list: RowListState,
    /// The no-notebook state's RECENT notebooks and their display names.
    pub(crate) recent: Vec<PathBuf>,
    recent_names: Vec<(String, Option<String>)>,
    root: Option<PathBuf>,
    name: String,
    favorite: bool,
    /// The header button (or state button) under the pointer.
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
    /// The notebook's root row is expanded (`library_host::root_expanded`).
    root_expanded: bool,
    /// The tab under a middle press on an Open Editors row: its release there closes it.
    middle_press: Option<DocumentId>,
    /// The one keyboard selection through the header rows, the Open Editors rows and the tree
    /// (open editors spec §3.5). `Cursor::Tree` leaves it to `list.selected`.
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

impl NotebookView {
    pub(crate) fn new(panel: HWND) -> Self {
        let dpi = unsafe { GetDpiForWindow(panel) }.max(96);
        Self {
            panel,
            mode: Mode::Loading,
            rows: Vec::new(),
            truncated: false,
            list: RowListState::new(scale(ROW_HEIGHT, dpi)),
            recent: Vec::new(),
            recent_names: Vec::new(),
            root: None,
            name: String::new(),
            favorite: false,
            hover: None,
            hover_pin: false,
            tooltip: None,
            tooltip_failed: false,
            typed: TypeAhead::default(),
            thumb_grab: None,
            drag: None,
            eat_right_up: false,
            drag_label: None,
            tracking_leave: false,
            built: None,
            order: 0,
            inline: InlineName::new(),
            images: IconImages::new(),
            editors: OpenEditors::new(scale(ROW_HEIGHT, dpi)),
            editors_expanded: true,
            root_expanded: true,
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
                Some(row) => Target::Row(row.clone()),
                None if self.truncated && index == self.rows.len() => Target::Truncated,
                None => Target::Nothing,
            },
            Mode::Loading | Mode::Empty | Mode::Failed => Target::Nothing,
        }
    }

    fn select(&mut self, index: usize) {
        let height = self.list_height();
        self.list.select(index, height);
        self.invalidate();
    }

    fn apply(&mut self, mut snapshot: Snapshot, names: Vec<(String, Option<String>)>) {
        // A draft row needs the tree, even in a notebook with nothing listed yet (inline naming
        // spec §3.1), as long as the draft lasts.
        let empty = snapshot.mode == Mode::Empty;
        if empty && self.inline.wants_draft() {
            snapshot.mode = Mode::Tree;
        }
        // The edit ends with its notebook, when the tree goes, or when its folder or row is no
        // longer listed (spec §5.4); `fit` puts the draft row back in. An empty notebook whose
        // draft ended keeps its empty state.
        if snapshot.root != self.root
            || snapshot.mode != Mode::Tree
            || !self.inline.fit(&mut snapshot.rows)
        {
            self.inline.end();
            if empty {
                snapshot.mode = Mode::Empty;
            }
        }
        let reset = snapshot.mode != self.mode || snapshot.root != self.root;
        if reset {
            self.list = RowListState::new(self.list.row_height);
            self.typed = TypeAhead::default();
        }
        let selected = self
            .list
            .selected
            .and_then(|index| self.rows.get(index))
            .map(|row| row.kind.clone());
        let top = (!reset)
            .then(|| self.rows.get(self.list.top).map(|row| row.kind.clone()))
            .flatten();
        let (old_selected, old_top) = (self.list.selected, self.list.top);
        let reordered = reset
            || snapshot.recent != self.recent
            || snapshot.rows.len() != self.rows.len()
            || snapshot
                .rows
                .iter()
                .zip(&self.rows)
                .any(|(new, old)| new.kind != old.kind);
        if reordered {
            self.order = self.order.wrapping_add(1);
        }
        self.mode = snapshot.mode;
        self.root_expanded = snapshot.root_expanded;
        let expanding = snapshot.editors_expanded && !self.editors_expanded;
        self.editors_expanded = snapshot.editors_expanded;
        // The keyboard selection leaves a row that is gone: a tree row hidden by its collapsed
        // root to the root row (open editors spec §3.5); the root row with its notebook and a tab
        // row with its section to the Open Editors header, which is always there.
        if self.cursor == Cursor::Tree && self.tree_hidden() {
            self.cursor = Cursor::Root;
        }
        let gone = match self.cursor {
            Cursor::Root => self.mode == Mode::NoNotebook,
            Cursor::Editor(_) => !self.editors_expanded,
            Cursor::EditorsHeader | Cursor::Tree => false,
        };
        if gone {
            self.cursor = Cursor::EditorsHeader;
        }
        // The section's rows fit its height again, the active row in view once it shows.
        self.fit_editors(expanding);
        self.name = snapshot
            .root
            .as_deref()
            .map(super::library_host::notebook_name)
            .unwrap_or_default();
        self.root = snapshot.root;
        self.favorite = snapshot.favorite;
        self.rows = snapshot.rows;
        self.truncated = snapshot.truncated;
        self.recent = snapshot.recent;
        self.recent_names = names;
        self.built = Some(snapshot.key);
        #[cfg(test)]
        {
            self.rebuilds += 1;
        }
        let count = match self.mode {
            Mode::Tree => self.rows.len() + usize::from(self.truncated),
            Mode::NoNotebook => self.recent.len(),
            Mode::Loading | Mode::Empty | Mode::Failed => 0,
        };
        self.list.set_count(count);
        // Measured in the new mode's list area.
        let height = self.list_height();
        if self.mode == Mode::Tree {
            self.list.selected = follow(&self.rows, selected.as_ref(), old_selected);
            self.list.top = follow(&self.rows, top.as_ref(), Some(old_top)).unwrap_or(0);
            self.list.scroll_lines(0, height);
        }
        self.hover_pin = false;
    }
}

#[cfg(test)]
mod tests;
