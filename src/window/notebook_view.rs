//! The Notebook view (spec §6): the open notebook's folder tree in the side panel, with its
//! header, pins, type-ahead and the loading, no-notebook and empty states. The tree itself is
//! built on the scan worker (`LibraryState.tree`). This module flattens the expanded part into
//! rows, paints only the rows on screen, and turns clicks and keys into `open_note` calls.

use super::main_window::{OpenMode, app_ptr};
use super::side_panel::{UiFonts, ViewPaint, draw_text, point_of};
use crate::document::{Document, DocumentId};
use crate::library::tree::{self, NoteTree, RowKind, TreeRow, UnsavedEntry};
use crate::window::commands::CommandId;
use crate::window::file_icons::{FOLDER_ICON, FileIcon, IconFont, file_icon};
use crate::window::menus::MenuEntry;
use crate::window::palette::{FileIcons, Palette};
use crate::window::panel::{fill, scale};
use crate::window::row_list::{self, ListKey, RowListState, RowLook, row_foreground};
use crate::window::tooltip::Tooltip;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    ClientToScreen, DT_CENTER, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER,
    DT_WORDBREAK, GetDC, GetTextExtentPoint32W, HDC, HFONT, InvalidateRect, ReleaseDC,
    ScreenToClient, SelectObject,
};
use windows_sys::Win32::UI::Controls::WM_MOUSELEAVE;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetFocus, GetKeyState, ReleaseCapture, SetCapture, SetFocus, TME_LEAVE, TRACKMOUSEEVENT,
    TrackMouseEvent, VK_CONTROL, VK_DELETE, VK_F2, VK_LEFT, VK_RETURN, VK_RIGHT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetClientRect, GetParent, SendMessageW, WM_CAPTURECHANGED, WM_CHAR, WM_COMMAND, WM_CONTEXTMENU,
    WM_KEYDOWN, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL,
    WM_RBUTTONDOWN,
};

// Sizes at 96 DPI; everything is scaled with `panel::scale`.
const ROW_HEIGHT: i32 = 26;
const HEADER_HEIGHT: i32 = 38;
const INDENT: i32 = 12;
const LEFT_PAD: i32 = 8;
const GLYPH_BOX: i32 = 16;
const GAP: i32 = 6;
const PIN_BOX: i32 = 24;
const HEADER_BUTTON: i32 = 28;
const TYPE_AHEAD_RESET: Duration = Duration::from_secs(1);

pub(crate) const TRUNCATED_ROW: &str = "Showing the first 10,000 notes";

// Segoe MDL2 Assets, the font the title bar already uses.
const GLYPH_CHEVRON_RIGHT: &str = "\u{E76C}";
const GLYPH_CHEVRON_DOWN: &str = "\u{E70D}";
const GLYPH_FOLDER: &str = "\u{E8B7}";
const GLYPH_NOTE: &str = "\u{E8A5}";
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
    /// Loaded, with no notes and no untitled tabs.
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
    /// Enter: the preview tab, focus stays in the tree for further browsing.
    Enter,
    /// Ctrl+Enter or a double-click: a normal tab, focus to the editor.
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
    Header(HeaderButton),
    Title,
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
pub(crate) struct HeaderLayout {
    pub title: RECT,
    /// Left to right: star, New note, New folder, "…".
    pub buttons: [(HeaderButton, RECT); 4],
}

pub(crate) fn header_layout(area: RECT, dpi: u32) -> HeaderLayout {
    let height = scale(HEADER_HEIGHT, dpi);
    let size = scale(HEADER_BUTTON, dpi);
    let top = area.top + (height - size) / 2;
    let right = area.right - scale(6, dpi);
    let slot = |from_right: i32| RECT {
        left: right - (from_right + 1) * size,
        top,
        right: right - from_right * size,
        bottom: top + size,
    };
    let buttons = [
        (HeaderButton::Favorite, slot(3)),
        (HeaderButton::NewNote, slot(2)),
        (HeaderButton::NewFolder, slot(1)),
        (HeaderButton::More, slot(0)),
    ];
    let title_left = area.left + scale(12, dpi);
    let title = RECT {
        left: title_left,
        top: area.top,
        right: (buttons[0].1.left - scale(4, dpi)).max(title_left),
        bottom: area.top + height,
    };
    HeaderLayout { title, buttons }
}

/// Everything below the header.
pub(crate) fn body_rect(area: RECT, dpi: u32) -> RECT {
    RECT {
        top: (area.top + scale(HEADER_HEIGHT, dpi)).min(area.bottom),
        ..area
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

/// An untitled tab's row name: its tab label, else its first line, else "Untitled".
pub(crate) fn unsaved_label(document: &Document) -> String {
    document
        .untitled_label
        .clone()
        .or_else(|| document.first_line_label.clone())
        .unwrap_or_else(|| "Untitled".to_owned())
}

/// One entry per untitled tab, keyed by its `DocumentId`, labelled like its tab (spec §6.2).
pub(crate) fn unsaved_entries<'a>(
    documents: impl Iterator<Item = &'a Document>,
) -> Vec<UnsavedEntry> {
    documents
        .filter(|document| document.path.is_none())
        .map(|document| UnsavedEntry {
            key: document.id.0,
            label: unsaved_label(document),
        })
        .collect()
}

/// How `flatten` keys an expanded folder: the same lowercasing as `model::same_path`.
fn expanded_key(path: &Path) -> String {
    path.as_os_str().to_string_lossy().to_lowercase()
}

/// The visible rows of `tree` with `expanded` folders open. The expanded set is hashed once, so
/// each folder row costs one lookup, not a scan of every expanded entry.
pub(crate) fn flatten(
    tree: &NoteTree,
    expanded: &[PathBuf],
    unsaved: &[UnsavedEntry],
) -> Vec<TreeRow> {
    let open: HashSet<String> = expanded.iter().map(|path| expanded_key(path)).collect();
    tree.rows(
        &|path: &Path| !open.is_empty() && open.contains(&expanded_key(path)),
        unsaved,
    )
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
    unsaved: Vec<UnsavedEntry>,
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
    tracking_leave: bool,
    /// What the rows were last built from (`None` before the first rebuild).
    built: Option<RebuildKey>,
    /// Bumped whenever a rebuild changes the rows' order or the RECENT list, so screen readers
    /// hear a reorder even at the same count (`AccessibleView::accessible_generation`).
    order: u64,
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

#[allow(
    clippy::too_many_arguments,
    reason = "one row's paint inputs, called from one closure"
)]
fn draw_tree_row(
    dc: HDC,
    row: Option<&TreeRow>,
    rect: RECT,
    look: RowLook,
    palette: &Palette,
    icons: &FileIcons,
    fonts: UiFonts,
    dpi: u32,
    pin_hot: bool,
) {
    let foreground = row_foreground(look, palette);
    let muted = if look.selected && look.focused {
        foreground
    } else {
        palette.muted_foreground
    };
    let Some(row) = row else {
        let text = RECT {
            left: rect.left + scale(LEFT_PAD, dpi),
            ..rect
        };
        unsafe { draw_text(dc, TRUNCATED_ROW, text, fonts.italic, muted, LINE) };
        return;
    };
    let parts = row_parts(rect, row.depth, dpi);
    // A type icon keeps its colour on a selected or hovered row: the colours are mid-tones that
    // read on the selection. High contrast draws every icon in the muted system pair, as before
    // (notebook folders spec §5.2).
    let draw_icon = |icon: FileIcon| {
        let color = if palette.high_contrast {
            muted
        } else {
            icons.color(icon.color)
        };
        let font = match icon.font {
            IconFont::Glyph => fonts.glyph,
            IconFont::Bold => fonts.bold,
        };
        unsafe { draw_text(dc, icon.text, parts.icon, font, color, CENTERED) };
    };
    match &row.kind {
        RowKind::Folder(_) => {
            let chevron = if row.expanded {
                GLYPH_CHEVRON_DOWN
            } else {
                GLYPH_CHEVRON_RIGHT
            };
            unsafe { draw_text(dc, chevron, parts.chevron, fonts.glyph, muted, CENTERED) };
            draw_icon(FOLDER_ICON);
        }
        RowKind::Note(path) => {
            let extension = path
                .extension()
                .map(|extension| extension.to_string_lossy());
            draw_icon(file_icon(extension.as_deref()));
        }
        RowKind::Unsaved(_) => {
            unsafe { draw_text(dc, GLYPH_NOTE, parts.icon, fonts.glyph, muted, CENTERED) };
        }
        RowKind::Draft => {}
    }
    if matches!(row.kind, RowKind::Note(_)) {
        // Pinned is a filled glyph, never color alone (spec §10). Segoe's PinFill is only the
        // head's fill, with no needle: its tilted outline (Pinned) is drawn over it to complete
        // the shape.
        if row.pinned {
            for glyph in [GLYPH_PINNED, GLYPH_PIN] {
                unsafe { draw_text(dc, glyph, parts.pin, fonts.glyph, foreground, CENTERED) };
            }
        } else if look.hover || look.selected {
            let color = if pin_hot { foreground } else { muted };
            unsafe { draw_text(dc, GLYPH_PIN, parts.pin, fonts.glyph, color, CENTERED) };
        }
    }
    let font = if matches!(row.kind, RowKind::Unsaved(_)) {
        fonts.italic
    } else {
        fonts.text
    };
    unsafe { draw_text(dc, &row.name, parts.name, font, foreground, LINE) };
}

fn draw_recent_row(
    dc: HDC,
    name: Option<&(String, Option<String>)>,
    rect: RECT,
    look: RowLook,
    palette: &Palette,
    fonts: UiFonts,
    dpi: u32,
) {
    let Some((name, hint)) = name else {
        return;
    };
    let foreground = row_foreground(look, palette);
    let parts = row_parts(rect, 0, dpi);
    unsafe {
        draw_text(
            dc,
            GLYPH_FOLDER,
            parts.icon,
            fonts.glyph,
            palette.muted_foreground,
            CENTERED,
        )
    };
    let text = RECT {
        right: rect.right - scale(LEFT_PAD, dpi),
        ..parts.name
    };
    // A clash between two notebook names shows the parent folder, dimmed, after the name.
    let label = match hint {
        Some(hint) => format!("{name}  {hint}"),
        None => name.clone(),
    };
    unsafe { draw_text(dc, &label, text, fonts.text, foreground, LINE) };
}

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
            tracking_leave: false,
            built: None,
            order: 0,
            #[cfg(test)]
            rebuilds: 0,
        }
    }

    /// The list's rectangle for the current mode, in the panel's `client` coordinates at `dpi`:
    /// the tree rows, the RECENT rows, or an empty band while there are none.
    pub(crate) fn list_area(&self, client: RECT, dpi: u32) -> RECT {
        let body = body_rect(client, dpi);
        match self.mode {
            Mode::Tree => body,
            Mode::NoNotebook => state_layout(body, dpi).list,
            Mode::Loading | Mode::Empty | Mode::Failed => RECT {
                bottom: body.top,
                ..body
            },
        }
    }

    fn dpi(&self) -> u32 {
        unsafe { GetDpiForWindow(self.panel) }.max(96)
    }

    fn client(&self) -> RECT {
        let mut rect = RECT::default();
        unsafe {
            GetClientRect(self.panel, &mut rect);
        }
        rect
    }

    fn invalidate(&self) {
        unsafe {
            InvalidateRect(self.panel, std::ptr::null(), 0);
        }
    }

    /// The list's rectangle in panel coordinates for the current mode.
    fn list_rect(&self, area: RECT) -> RECT {
        self.list_area(area, self.dpi())
    }

    fn list_height(&self) -> i32 {
        height(self.list_rect(self.client())).max(self.list.row_height)
    }

    fn row_rect(&self, list: RECT, index: usize) -> Option<RECT> {
        let top = self.list.row_top(index)?;
        Some(RECT {
            left: list.left,
            top: list.top + top,
            right: list.right,
            bottom: list.top + top + self.list.row_height,
        })
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

    fn apply(&mut self, snapshot: Snapshot, names: Vec<(String, Option<String>)>) {
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

    fn hit_test(&self, x: i32, y: i32) -> Hit {
        let area = self.client();
        let dpi = self.dpi();
        if y < area.top + scale(HEADER_HEIGHT, dpi) {
            let header = header_layout(area, dpi);
            if self.mode != Mode::NoNotebook {
                for (button, rect) in header.buttons {
                    if contains(rect, x, y) {
                        return Hit::Header(button);
                    }
                }
            }
            return if contains(header.title, x, y) {
                Hit::Title
            } else {
                Hit::Empty
            };
        }
        let body = body_rect(area, dpi);
        match self.mode {
            Mode::NoNotebook | Mode::Empty | Mode::Failed => {
                let layout = state_layout(body, dpi);
                if contains(layout.button, x, y) {
                    return Hit::StateButton;
                }
                if self.mode == Mode::Failed && contains(layout.second, x, y) {
                    return Hit::SecondButton;
                }
                if self.mode == Mode::NoNotebook
                    && contains(layout.list, x, y)
                    && let Some(index) = self.list.row_at(y - layout.list.top)
                {
                    return Hit::Row {
                        index,
                        part: RowPart::Body,
                    };
                }
                Hit::Empty
            }
            Mode::Loading => Hit::Empty,
            Mode::Tree => {
                let list = self.list_rect(area);
                if let Some(grab) = self.list.thumb_hit(
                    x - list.left,
                    y - list.top,
                    list.right - list.left,
                    height(list),
                ) {
                    return Hit::Thumb(grab);
                }
                let Some(index) = self.list.row_at(y - list.top) else {
                    return Hit::Empty;
                };
                let (Some(row), Some(rect)) = (self.rows.get(index), self.row_rect(list, index))
                else {
                    return Hit::Row {
                        index,
                        part: RowPart::Body,
                    };
                };
                let parts = row_parts(rect, row.depth, dpi);
                let part = match row.kind {
                    RowKind::Folder(_) if x < parts.icon.right => RowPart::Chevron,
                    RowKind::Note(_) if x >= parts.pin.left => RowPart::Pin,
                    _ => RowPart::Body,
                };
                Hit::Row { index, part }
            }
        }
    }

    /// `point` in panel coordinates, converted to the main window's client coordinates, which
    /// `menus::track_popup` takes.
    fn to_main(&self, point: POINT) -> POINT {
        let mut point = point;
        unsafe {
            ClientToScreen(self.panel, &mut point);
            ScreenToClient(GetParent(self.panel), &mut point);
        }
        point
    }

    fn text_width(&mut self, text: &str, font: HFONT) -> i32 {
        let wide = text.encode_utf16().collect::<Vec<_>>();
        if wide.is_empty() {
            return 0;
        }
        let mut size = SIZE::default();
        unsafe {
            let dc = GetDC(self.panel);
            if dc.is_null() {
                return 0;
            }
            let previous = SelectObject(dc, font);
            GetTextExtentPoint32W(dc, wide.as_ptr(), wide.len() as i32, &mut size);
            SelectObject(dc, previous);
            ReleaseDC(self.panel, dc);
        }
        size.cx
    }

    fn track_leave(&mut self) {
        if self.tracking_leave {
            return;
        }
        let mut track = TRACKMOUSEEVENT {
            cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
            dwFlags: TME_LEAVE,
            hwndTrack: self.panel,
            dwHoverTime: 0,
        };
        self.tracking_leave = unsafe { TrackMouseEvent(&mut track) } != 0;
    }

    /// Destroys the view's tooltip, if it made one. The popup is owned by the main window, so
    /// destroying the panel does not take it along (`side_panel::destroy_windows` calls this).
    pub(crate) fn destroy_tooltip(&self) {
        if let Some(tooltip) = self.tooltip {
            tooltip.destroy();
        }
    }

    /// The hovered row's tip: a truncated note name in full, or a recent notebook's path.
    fn row_tip(&mut self, fonts: UiFonts) -> (RECT, String) {
        let none = (RECT::default(), String::new());
        let Some(index) = self.list.hover else {
            return none;
        };
        let list = self.list_rect(self.client());
        let Some(rect) = self.row_rect(list, index) else {
            return none;
        };
        match self.mode {
            Mode::NoNotebook => self
                .recent
                .get(index)
                .map_or(none, |path| (rect, path.display().to_string())),
            Mode::Tree => {
                let Some(row) = self.rows.get(index).cloned() else {
                    return none;
                };
                let parts = row_parts(rect, row.depth, self.dpi());
                let font = if matches!(row.kind, RowKind::Unsaved(_)) {
                    fonts.italic
                } else {
                    fonts.text
                };
                if self.text_width(&row.name, font) > parts.name.right - parts.name.left {
                    (parts.name, row.name)
                } else {
                    none
                }
            }
            Mode::Loading | Mode::Empty | Mode::Failed => none,
        }
    }

    /// Every tool's ID, rectangle and text, for `apply_tooltips`. `fonts` measures whether the
    /// hovered name is cut off. A tool with an empty text is removed, so a hidden button shows
    /// no tip.
    fn tooltip_tools(&mut self, fonts: UiFonts) -> Vec<(usize, RECT, String)> {
        let area = self.client();
        let header = header_layout(area, self.dpi());
        let title = self
            .root
            .as_ref()
            .map(|root| root.display().to_string())
            .unwrap_or_default();
        let favorite = if self.favorite {
            "Remove from favorites"
        } else {
            "Add to favorites"
        };
        let buttons_shown = self.mode != Mode::NoNotebook;
        let (row_rect, row_text) = self.row_tip(fonts);
        let mut tools = vec![(TOOL_TITLE, header.title, title)];
        for (button, rect) in header.buttons {
            let (id, text) = match button {
                HeaderButton::Favorite => (TOOL_FAVORITE, favorite),
                HeaderButton::NewNote => (TOOL_NEW, "New note"),
                HeaderButton::NewFolder => (TOOL_NEW_FOLDER, "New folder"),
                HeaderButton::More => (TOOL_MORE, "More actions"),
            };
            let text = if buttons_shown { text } else { "" };
            tools.push((id, rect, text.to_owned()));
        }
        tools.push((TOOL_ROW, row_rect, row_text));
        tools
    }

    fn paint(&mut self, paint: &ViewPaint) {
        let (dc, area, dpi, fonts, focused) = (
            paint.hdc,
            paint.client,
            paint.dpi,
            paint.fonts,
            paint.focused,
        );
        let palette = &paint.palette;
        self.list.row_height = scale(ROW_HEIGHT, dpi);
        self.paint_header(dc, area, palette, fonts, dpi);
        let body = body_rect(area, dpi);
        let layout = state_layout(body, dpi);
        match self.mode {
            Mode::Loading => {
                unsafe {
                    draw_text(
                        dc,
                        "Loading…",
                        layout.message,
                        fonts.text,
                        palette.muted_foreground,
                        LINE,
                    )
                };
            }
            Mode::NoNotebook => {
                let message = "Open a notebook to see its notes.";
                unsafe {
                    draw_text(
                        dc,
                        message,
                        layout.message,
                        fonts.text,
                        palette.editor_foreground,
                        DT_WORDBREAK | DT_NOPREFIX,
                    )
                };
                self.paint_button(
                    dc,
                    layout.button,
                    "Open notebook…",
                    Hit::StateButton,
                    palette,
                    fonts,
                );
                if !self.recent.is_empty() {
                    unsafe {
                        draw_text(
                            dc,
                            "RECENT",
                            layout.label,
                            fonts.bold,
                            palette.muted_foreground,
                            LINE,
                        )
                    };
                }
                let names = &self.recent_names;
                row_list::paint(
                    dc,
                    layout.list,
                    &self.list,
                    palette,
                    focused,
                    &mut |dc, index, rect, look| {
                        draw_recent_row(dc, names.get(index), rect, look, palette, fonts, dpi);
                    },
                );
            }
            Mode::Empty => {
                let message = format!("No notes in {} yet.", self.name);
                unsafe {
                    draw_text(
                        dc,
                        &message,
                        layout.message,
                        fonts.text,
                        palette.editor_foreground,
                        DT_WORDBREAK | DT_NOPREFIX,
                    )
                };
                self.paint_button(
                    dc,
                    layout.button,
                    "New note",
                    Hit::StateButton,
                    palette,
                    fonts,
                );
            }
            Mode::Failed => {
                unsafe {
                    draw_text(
                        dc,
                        LOAD_FAILED,
                        layout.message,
                        fonts.text,
                        palette.editor_foreground,
                        DT_WORDBREAK | DT_NOPREFIX,
                    )
                };
                self.paint_button(dc, layout.button, "Retry", Hit::StateButton, palette, fonts);
                self.paint_button(
                    dc,
                    layout.second,
                    "Open notebook…",
                    Hit::SecondButton,
                    palette,
                    fonts,
                );
            }
            Mode::Tree => {
                // `row_list::paint` draws the rows in view and the scroll thumb.
                let list = self.list_rect(area);
                let rows = &self.rows;
                let hover_pin = self.hover_pin;
                let icons = &paint.icons;
                row_list::paint(
                    dc,
                    list,
                    &self.list,
                    palette,
                    focused,
                    &mut |dc, index, rect, look| {
                        draw_tree_row(
                            dc,
                            rows.get(index),
                            rect,
                            look,
                            palette,
                            icons,
                            fonts,
                            dpi,
                            hover_pin && look.hover,
                        );
                    },
                );
            }
        }
    }

    fn paint_header(&self, dc: HDC, area: RECT, palette: &Palette, fonts: UiFonts, dpi: u32) {
        let layout = header_layout(area, dpi);
        let title = match self.mode {
            Mode::NoNotebook => "NOTEBOOK".to_owned(),
            _ => self.name.to_uppercase(),
        };
        unsafe {
            draw_text(
                dc,
                &title,
                layout.title,
                fonts.bold,
                palette.muted_foreground,
                LINE,
            )
        };
        if self.mode == Mode::NoNotebook {
            return;
        }
        for (button, rect) in layout.buttons {
            let hot = self.hover == Some(Hit::Header(button));
            if hot {
                unsafe { fill(dc, rect, palette.hover_background) };
            }
            let glyph = match button {
                HeaderButton::Favorite if self.favorite => GLYPH_STAR_FILLED,
                HeaderButton::Favorite => GLYPH_STAR,
                HeaderButton::NewNote => GLYPH_ADD,
                HeaderButton::NewFolder => GLYPH_NEW_FOLDER,
                HeaderButton::More => GLYPH_MORE,
            };
            let color = if hot {
                palette.hover_foreground
            } else {
                palette.muted_foreground
            };
            unsafe { draw_text(dc, glyph, rect, fonts.glyph, color, CENTERED) };
        }
    }

    fn paint_button(
        &self,
        dc: HDC,
        rect: RECT,
        text: &str,
        hit: Hit,
        palette: &Palette,
        fonts: UiFonts,
    ) {
        let background = if self.hover == Some(hit) {
            palette.hover_background
        } else {
            palette.pressed_background
        };
        unsafe { fill(dc, rect, background) };
        unsafe {
            draw_text(
                dc,
                text,
                rect,
                fonts.text,
                palette.editor_foreground,
                CENTERED,
            )
        };
    }
}

/// What the MSAA provider reads of the view.
impl NotebookView {
    pub(crate) fn rows(&self) -> &[TreeRow] {
        &self.rows
    }

    pub(crate) fn list(&self) -> &RowListState {
        &self.list
    }

    pub(crate) fn list_mut(&mut self) -> &mut RowListState {
        &mut self.list
    }

    /// Every push button painted, in paint order, with its accessible name: the header's star,
    /// New note and "…" (not in the no-notebook state), then the state's own button.
    pub(crate) fn buttons(&self, client: RECT, dpi: u32) -> Vec<(String, RECT)> {
        let mut buttons = Vec::new();
        if self.mode != Mode::NoNotebook {
            for (button, rect) in header_layout(client, dpi).buttons {
                let name = match button {
                    HeaderButton::Favorite if self.favorite => "Remove from favorites",
                    HeaderButton::Favorite => "Add to favorites",
                    HeaderButton::NewNote => "New note",
                    HeaderButton::NewFolder => "New folder",
                    HeaderButton::More => "More actions",
                };
                buttons.push((name.to_owned(), rect));
            }
        }
        let state = state_layout(body_rect(client, dpi), dpi);
        match self.mode {
            Mode::NoNotebook => buttons.push(("Open notebook…".to_owned(), state.button)),
            Mode::Empty => buttons.push(("New note".to_owned(), state.button)),
            Mode::Failed => {
                buttons.push(("Retry".to_owned(), state.button));
                buttons.push(("Open notebook…".to_owned(), state.second));
            }
            Mode::Loading | Mode::Tree => {}
        }
        buttons
    }

    /// RECENT notebook `index`'s accessible name, with the parent-folder hint on a name clash.
    pub(crate) fn recent_name(&self, index: usize) -> String {
        match self.recent_names.get(index) {
            Some((name, Some(hint))) => format!("{name}, {hint}"),
            Some((name, None)) => name.clone(),
            None => String::new(),
        }
    }
}

/// What a rebuild read from the library, the tabs and `folders.ini`'s cache.
struct Snapshot {
    mode: Mode,
    rows: Vec<TreeRow>,
    truncated: bool,
    recent: Vec<PathBuf>,
    root: Option<PathBuf>,
    favorite: bool,
    key: RebuildKey,
}

fn with_view<R>(hwnd: HWND, f: impl FnOnce(&mut NotebookView) -> R) -> Option<R> {
    unsafe { app_ptr(hwnd) }.and_then(|mut app| {
        unsafe { app.as_mut() }
            .sidebar
            .as_mut()
            .map(|sidebar| f(&mut sidebar.notebook))
    })
}

/// What the rows would be built from now. Cheap: no flattening, no disk.
fn rebuild_key(hwnd: HWND) -> RebuildKey {
    let root = super::library_host::folder(hwnd);
    let unsaved = if root.is_some() {
        unsafe { app_ptr(hwnd) }
            .map(|app| unsaved_entries(unsafe { app.as_ref() }.tabs.documents()))
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    RebuildKey {
        loaded: super::library_host::with_state(hwnd, |_| ()).is_some(),
        failed: super::library_host::load_failed(hwnd),
        expansion: super::library_host::expansion_revision(hwnd),
        root,
        unsaved,
    }
}

/// Reads everything the rows need. Each call borrows the App on its own, never nested.
fn snapshot(hwnd: HWND) -> Snapshot {
    let key = rebuild_key(hwnd);
    let Some(root) = key.root.clone() else {
        return Snapshot {
            mode: Mode::NoNotebook,
            rows: Vec::new(),
            truncated: false,
            recent: super::library_host::recent_notebooks(hwnd),
            root: None,
            favorite: false,
            key,
        };
    };
    let favorite = super::library_host::is_favorite(hwnd);
    let built = super::library_host::with_state(hwnd, |state| {
        let rows = flatten(&state.tree, &state.local.expanded, &key.unsaved);
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
        key,
    }
}

/// Rebuilds the rows from the library, the tabs and the notebook lists, keeping the selection
/// and the scroll position by path. `side_panel::refresh` calls it.
pub(crate) fn rebuild(hwnd: HWND) {
    let snapshot = snapshot(hwnd);
    let names = crate::library::local::display_names(&snapshot.recent);
    with_view(hwnd, |view| {
        view.apply(snapshot, names);
        view.invalidate();
    });
}

/// The row for the active tab: its note inside the open notebook, or its unsaved entry.
fn active_target(hwnd: HWND) -> Option<RowKind> {
    let root = super::library_host::folder(hwnd)?;
    let (id, path) = unsafe { app_ptr(hwnd) }.and_then(|app| {
        let active = unsafe { app.as_ref() }.tabs.active()?;
        Some((active.id, active.path.clone()))
    })?;
    match path {
        None => Some(RowKind::Unsaved(id.0)),
        Some(path) => crate::library::is_inside(&root, &path)
            .then(|| RowKind::Note(crate::library::record_path(&root, &path))),
    }
}

/// Whether something the rows are built from, besides the library itself, changed since the
/// last rebuild: another notebook, its state arriving, a folder expanded or collapsed, or an
/// untitled tab added, closed, relabelled or saved. Cheap: no flattening.
pub(crate) fn stale(hwnd: HWND) -> bool {
    let key = rebuild_key(hwnd);
    with_view(hwnd, |view| view.built.as_ref() != Some(&key)).unwrap_or(false)
}

/// Every tab switch: the active note's row is selected and its folders expand (remembered per
/// PC), without moving the keyboard focus (spec §6.1). The tree is flattened again only when
/// something the rows depend on changed (a folder newly expanded, an untitled tab added, closed
/// or relabelled, another notebook); otherwise the row is just selected.
pub(crate) fn active_tab_changed(hwnd: HWND) {
    let target = active_target(hwnd);
    if let Some(RowKind::Note(relative)) = &target {
        for folder in tree::ancestors(relative) {
            super::library_host::set_expanded(hwnd, &folder, true);
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
}

/// An untitled tab's label changed (`library_host::refresh_label`): its row is renamed in place,
/// with no rebuild. A row that is not there yet (its tab is new) comes with a rebuild.
pub(crate) fn unsaved_label_changed(hwnd: HWND, id: DocumentId, label: &str) {
    let renamed = with_view(hwnd, |view| {
        if let Some(entry) = view
            .built
            .as_mut()
            .and_then(|built| built.unsaved.iter_mut().find(|entry| entry.key == id.0))
        {
            entry.label = label.to_owned();
        }
        let Some(index) = tree::row_index(&view.rows, &RowKind::Unsaved(id.0)) else {
            // Without a notebook, or while it loads, there are no rows to rename.
            return !matches!(view.mode, Mode::Tree | Mode::Empty);
        };
        label.clone_into(&mut view.rows[index].name);
        view.invalidate();
        true
    });
    if renamed == Some(false) {
        rebuild(hwnd);
    }
}

/// The panel's `WM_PAINT` while the Notebook view shows (`side_panel::paint_view`). The panel
/// has already filled its background.
pub(crate) fn paint(hwnd: HWND, paint: &ViewPaint) {
    with_view(hwnd, |view| view.paint(paint));
}

/// Whether panel point (`x`, `y`) is over the header's name or a header button, which must stay
/// client area. The rest of the header is a window drag area (spec §6.5).
pub(crate) fn header_hit(hwnd: HWND, x: i32, y: i32) -> bool {
    with_view(hwnd, |view| {
        matches!(view.hit_test(x, y), Hit::Header(_) | Hit::Title)
    })
    .unwrap_or(false)
}

/// Runs a main-window command as the menus do.
fn run(hwnd: HWND, command: CommandId) {
    unsafe {
        SendMessageW(hwnd, WM_COMMAND, command as usize, 0);
    }
}

/// The selected note's absolute path while the panel has the keyboard focus, so palette and
/// accelerator commands act on it rather than on the active tab (spec §6.3).
pub(crate) fn focused_note(hwnd: HWND) -> Option<PathBuf> {
    let root = super::library_host::folder(hwnd)?;
    with_view(hwnd, |view| {
        let focused = unsafe { GetFocus() } == view.panel;
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
/// focus: Rename and Delete act on it (notebook folders spec §4.2, §4.3).
pub(crate) fn focused_folder(hwnd: HWND) -> Option<PathBuf> {
    with_view(hwnd, |view| {
        let focused = unsafe { GetFocus() } == view.panel;
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
/// selected note's parent. `None` (the root) for an unsaved row or no selection.
pub(crate) fn selected_folder(hwnd: HWND) -> Option<PathBuf> {
    let root = super::library_host::folder(hwnd)?;
    let target = with_view(hwnd, |view| {
        view.list.selected.map(|index| view.target(index))
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
    with_view(hwnd, |view| {
        let Some(index) = tree::row_index(&view.rows, kind) else {
            return false;
        };
        view.select(index);
        true
    })
    .unwrap_or(false)
}

/// Gives the tree the keyboard focus, after a folder command closed its name box.
pub(crate) fn focus_tree(hwnd: HWND) {
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
}

impl NotebookView {
    /// Under row `index`, in main-window client coordinates, for a menu opened from the
    /// keyboard.
    fn row_menu_point(&self, index: usize) -> POINT {
        let list = self.list_rect(self.client());
        let rect = self.row_rect(list, index).unwrap_or(list);
        self.to_main(POINT {
            x: rect.left + scale(24, self.dpi()),
            y: rect.bottom,
        })
    }
}

/// Row `index`'s context menu (spec §6.6), at `at` (main-window client coordinates) or under
/// the row when opened from the keyboard. The chosen entry acts on that row, not the active
/// tab. "Open in new tab" is `CommandId::Open` and "New note here" is `CommandId::New` here.
pub(crate) fn open_context_menu(hwnd: HWND, index: usize, at: Option<POINT>) {
    let Some((target, point)) = with_view(hwnd, |view| {
        if view.mode != Mode::Tree {
            return None;
        }
        view.select(index);
        Some((
            view.target(index),
            at.unwrap_or_else(|| view.row_menu_point(index)),
        ))
    })
    .flatten() else {
        return;
    };
    let Target::Row(row) = target else {
        return;
    };
    let Some(root) = super::library_host::folder(hwnd) else {
        return;
    };
    match &row.kind {
        RowKind::Note(relative) => {
            let path = root.join(relative);
            let entries = [
                MenuEntry::command("Open in new tab", CommandId::Open),
                MenuEntry::command(
                    if row.pinned { "Unpin" } else { "Pin" },
                    CommandId::NoteTogglePin,
                ),
                MenuEntry::Separator,
                MenuEntry::command(
                    "Move to notebook...\tCtrl+Shift+M",
                    CommandId::NoteMoveToNotebook,
                ),
                MenuEntry::command("Rename...\tF2", CommandId::NoteRename),
                MenuEntry::command("Reveal in Explorer", CommandId::NoteRevealInExplorer),
                MenuEntry::Separator,
                MenuEntry::command("Delete...\tDel", CommandId::NoteDelete),
            ];
            match super::menus::track_popup(hwnd, &entries, point) {
                Some(CommandId::Open) => {
                    if let Err(error) =
                        super::main_window::open_note(hwnd, &path, OpenMode::Permanent, true)
                    {
                        super::main_window::report_open_failure(hwnd, &path, &error);
                    }
                }
                Some(CommandId::NoteTogglePin) => super::library_host::toggle_pin(hwnd, &path),
                Some(CommandId::NoteMoveToNotebook) => {
                    super::library_host::move_to_notebook(hwnd, &path);
                }
                Some(CommandId::NoteRename) => {
                    if super::library_host::ready_library(hwnd) {
                        super::library_host::rename_file(hwnd, &path);
                    }
                }
                Some(CommandId::NoteRevealInExplorer) => super::library_host::reveal(hwnd, &path),
                Some(CommandId::NoteDelete) if super::library_host::ready_library(hwnd) => {
                    super::library_host::delete_file(hwnd, &path);
                }
                _ => {}
            }
        }
        RowKind::Folder(relative) => {
            let path = root.join(relative);
            let entries = [
                MenuEntry::command("New note here", CommandId::New),
                MenuEntry::command("New folder here", CommandId::NoteNewFolder),
                MenuEntry::Separator,
                MenuEntry::command("Rename...\tF2", CommandId::NoteRename),
                MenuEntry::command("Reveal in Explorer", CommandId::NoteRevealInExplorer),
                MenuEntry::Separator,
                MenuEntry::command("Delete...\tDel", CommandId::NoteDelete),
            ];
            match super::menus::track_popup(hwnd, &entries, point) {
                Some(CommandId::New) => super::library_host::new_note_in(hwnd, Some(path)),
                Some(CommandId::NoteNewFolder) => {
                    super::library_host::new_folder(hwnd, Some(relative.clone()));
                }
                Some(CommandId::NoteRename) if super::library_host::ready_library(hwnd) => {
                    super::library_host::rename_folder(hwnd, relative);
                }
                Some(CommandId::NoteRevealInExplorer) => super::library_host::reveal(hwnd, &path),
                Some(CommandId::NoteDelete) if super::library_host::ready_library(hwnd) => {
                    super::library_host::delete_folder(hwnd, relative);
                }
                _ => {}
            }
        }
        RowKind::Unsaved(key) => {
            let entries = [MenuEntry::command("Close tab", CommandId::CloseTab)];
            if super::menus::track_popup(hwnd, &entries, point) == Some(CommandId::CloseTab)
                && super::main_window::activate_document_by_id(hwnd, DocumentId(*key))
            {
                run(hwnd, CommandId::CloseTab);
            }
        }
        RowKind::Draft => {}
    }
}

/// `WM_CONTEXTMENU`: from a right-click (screen coordinates) or from Shift+F10 or the
/// context-menu key (`lparam` of -1, for the selected row).
fn context_menu(hwnd: HWND, lparam: LPARAM) {
    let keyboard = lparam as u32 == u32::MAX;
    let target = with_view(hwnd, |view| {
        if keyboard {
            return view.list.selected.map(|index| (index, None));
        }
        let (x, y) = point_of(lparam);
        let mut client = POINT { x, y };
        unsafe {
            ScreenToClient(view.panel, &mut client);
        }
        match view.hit_test(client.x, client.y) {
            Hit::Row { index, .. } => Some((index, Some(view.to_main(client)))),
            _ => None,
        }
    })
    .flatten();
    if let Some((index, at)) = target {
        open_context_menu(hwnd, index, at);
    }
}

/// The panel's input while the Notebook view is shown (`side_panel::view_mouse` and `view_key`).
/// `None` leaves the message to `DefWindowProcW`. The panel handles its resize edge itself.
pub(crate) fn handle(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
    match message {
        WM_MOUSEMOVE => {
            let (x, y) = point_of(lparam);
            mouse_move(hwnd, x, y);
            Some(0)
        }
        // The panel class has CS_DBLCLKS: the second press of a double-click comes as this.
        WM_LBUTTONDBLCLK => {
            let (x, y) = point_of(lparam);
            double_click(hwnd, x, y);
            Some(0)
        }
        WM_MOUSELEAVE => {
            with_view(hwnd, |view| {
                view.tracking_leave = false;
                view.list.hover = None;
                view.hover = None;
                view.hover_pin = false;
                view.invalidate();
            });
            Some(0)
        }
        WM_LBUTTONDOWN => {
            let (x, y) = point_of(lparam);
            left_down(hwnd, x, y);
            Some(0)
        }
        WM_LBUTTONUP => {
            // Released after the borrow ends: ReleaseCapture sends WM_CAPTURECHANGED here.
            if with_view(hwnd, |view| view.thumb_grab.take().is_some()).unwrap_or(false) {
                unsafe {
                    ReleaseCapture();
                }
            }
            Some(0)
        }
        WM_CAPTURECHANGED => {
            with_view(hwnd, |view| view.thumb_grab = None);
            Some(0)
        }
        WM_RBUTTONDOWN => {
            // Selects the row; DefWindowProc turns the button-up into WM_CONTEXTMENU.
            let (x, y) = point_of(lparam);
            focus_panel(hwnd);
            with_view(hwnd, |view| {
                if let Hit::Row { index, .. } = view.hit_test(x, y) {
                    view.select(index);
                }
            });
            Some(0)
        }
        WM_CONTEXTMENU => {
            context_menu(hwnd, lparam);
            Some(0)
        }
        WM_KEYDOWN => key_down(hwnd, wparam as u16).then_some(0),
        WM_CHAR => {
            let ch = char::from_u32(wparam as u32).filter(|ch| !ch.is_control())?;
            typed(hwnd, ch);
            Some(0)
        }
        WM_MOUSEWHEEL => {
            let delta = i32::from((wparam >> 16) as u16 as i16);
            let lines = row_list::wheel_lines();
            with_view(hwnd, |view| {
                let height = view.list_height();
                if view.list.wheel(delta, lines, height) {
                    view.invalidate();
                }
            });
            Some(0)
        }
        _ => None,
    }
}

fn mouse_move(hwnd: HWND, x: i32, y: i32) {
    // Read before the view is borrowed: `ui_fonts` borrows the App itself.
    let fonts = super::main_window::ui_fonts(hwnd);
    let tools = with_view(hwnd, |view| {
        if let Some(grab) = view.thumb_grab {
            let list = view.list_rect(view.client());
            if view.list.drag_thumb(grab, y - list.top, height(list)) {
                view.invalidate();
            }
            return None;
        }
        view.track_leave();
        let hit = view.hit_test(x, y);
        let (row, pin) = match hit {
            Hit::Row { index, part } => (Some(index), part == RowPart::Pin),
            _ => (None, false),
        };
        let hot =
            matches!(hit, Hit::Header(_) | Hit::StateButton | Hit::SecondButton).then_some(hit);
        let row_changed = view.list.set_hover(row);
        if row_changed || view.hover_pin != pin || view.hover != hot {
            view.hover_pin = pin;
            view.hover = hot;
            view.invalidate();
            return Some(view.tooltip_tools(fonts));
        }
        None
    })
    .flatten();
    if let Some(tools) = tools {
        apply_tooltips(hwnd, &tools);
    }
}

/// Gives the tooltip `tools`, making the tooltip first if the view has none yet. Runs with
/// nothing of the App borrowed: creating the control and adding tools send messages.
fn apply_tooltips(hwnd: HWND, tools: &[(usize, RECT, String)]) {
    let Some((existing, failed, panel)) =
        with_view(hwnd, |view| (view.tooltip, view.tooltip_failed, view.panel))
    else {
        return;
    };
    let tooltip = match existing {
        Some(tooltip) => tooltip,
        None if failed => return,
        None => {
            let created = Tooltip::create(panel);
            let kept = with_view(hwnd, |view| {
                view.tooltip = created;
                view.tooltip_failed = created.is_none();
            });
            match (created, kept) {
                (Some(tooltip), Some(())) => tooltip,
                (Some(tooltip), None) => {
                    tooltip.destroy();
                    return;
                }
                (None, _) => return,
            }
        }
    };
    for (id, rect, text) in tools {
        tooltip.set_tool(*id, *rect, text);
    }
}

/// Gives the panel the keyboard focus with nothing of the App borrowed: SetFocus sends
/// WM_KILLFOCUS and WM_SETFOCUS, whose handlers borrow it again.
fn focus_panel(hwnd: HWND) {
    let Some(panel) = with_view(hwnd, |view| view.panel) else {
        return;
    };
    unsafe {
        if GetFocus() != panel {
            SetFocus(panel);
        }
    }
}

fn left_down(hwnd: HWND, x: i32, y: i32) {
    focus_panel(hwnd);
    let Some(hit) = with_view(hwnd, |view| view.hit_test(x, y)) else {
        return;
    };
    match hit {
        Hit::Header(button) => header_clicked(hwnd, button),
        Hit::StateButton => state_button(hwnd),
        Hit::SecondButton => super::library_host::choose_and_open_folder(hwnd),
        Hit::Thumb(grab) => {
            // Captured after the borrow ends: SetCapture sends WM_CAPTURECHANGED to the window
            // that held the capture before.
            if let Some(panel) = with_view(hwnd, |view| {
                view.thumb_grab = Some(grab);
                view.panel
            }) {
                unsafe {
                    SetCapture(panel);
                }
            }
        }
        Hit::Row { index, part } => {
            with_view(hwnd, |view| view.select(index));
            row_clicked(hwnd, index, part, false);
        }
        Hit::Title | Hit::Empty => {}
    }
}

/// `WM_LBUTTONDBLCLK`: a row's second press opens it as a normal tab, which also promotes its
/// preview (spec §6.4). Anywhere else it is one more click.
fn double_click(hwnd: HWND, x: i32, y: i32) {
    match with_view(hwnd, |view| view.hit_test(x, y)) {
        Some(Hit::Row { index, part }) => row_clicked(hwnd, index, part, true),
        Some(_) => left_down(hwnd, x, y),
        None => {}
    }
}

/// A press on row `index`. `double` is the second press of a double-click, whose first press
/// already toggled a pin or a folder, or started opening a recent notebook.
fn row_clicked(hwnd: HWND, index: usize, part: RowPart, double: bool) {
    let Some(target) = with_view(hwnd, |view| view.target(index)) else {
        return;
    };
    match target {
        Target::Row(TreeRow {
            kind: RowKind::Note(relative),
            ..
        }) if part == RowPart::Pin => {
            if !double && let Some(root) = super::library_host::folder(hwnd) {
                super::library_host::toggle_pin(hwnd, &root.join(relative));
            }
        }
        Target::Row(TreeRow {
            kind: RowKind::Folder(_),
            ..
        })
        | Target::Recent(_)
            if double => {}
        _ => activate(
            hwnd,
            index,
            if double {
                Activation::Permanent
            } else {
                Activation::Click
            },
        ),
    }
}

fn set_folder_expanded(hwnd: HWND, relative: &Path, expanded: bool) {
    super::library_host::set_expanded(hwnd, relative, expanded);
    rebuild(hwnd);
}

/// Opens or toggles row `index` (spec §6.4). A folder toggles, a note opens, an unsaved row
/// switches to its tab, and a recent notebook opens.
pub(crate) fn activate(hwnd: HWND, index: usize, how: Activation) {
    let Some(target) = with_view(hwnd, |view| view.target(index)) else {
        return;
    };
    match target {
        Target::Recent(folder) => super::library_host::open_listed_notebook(hwnd, &folder),
        Target::Row(row) => match row.kind {
            RowKind::Folder(relative) => set_folder_expanded(hwnd, &relative, !row.expanded),
            RowKind::Note(relative) => {
                let Some(root) = super::library_host::folder(hwnd) else {
                    return;
                };
                let path = root.join(relative);
                let (mode, focus) = match how {
                    // A click keeps the keyboard in the tree, as VS Code's explorer does, so F2
                    // and Del act on the row just clicked.
                    Activation::Click | Activation::Enter => (OpenMode::Preview, false),
                    Activation::Permanent => (OpenMode::Permanent, true),
                };
                if let Err(error) = super::main_window::open_note(hwnd, &path, mode, focus) {
                    super::main_window::report_open_failure(hwnd, &path, &error);
                }
            }
            RowKind::Unsaved(key) => {
                if super::main_window::activate_document_by_id(hwnd, DocumentId(key))
                    && how != Activation::Enter
                {
                    super::main_window::focus_content(hwnd);
                }
            }
            RowKind::Draft => {}
        },
        Target::Truncated | Target::Nothing => {}
    }
}

/// The header's buttons (spec §6.5).
pub(crate) fn header_clicked(hwnd: HWND, button: HeaderButton) {
    match button {
        HeaderButton::Favorite => super::library_host::toggle_notebook_favorite(hwnd),
        HeaderButton::NewNote => run(hwnd, CommandId::New),
        HeaderButton::NewFolder => run(hwnd, CommandId::NoteNewFolder),
        HeaderButton::More => more_menu(hwnd),
    }
}

/// "…": the notebook's own actions.
fn more_menu(hwnd: HWND) {
    let Some(at) = with_view(hwnd, |view| {
        let rect = header_layout(view.client(), view.dpi()).buttons[3].1;
        view.to_main(POINT {
            x: rect.left,
            y: rect.bottom,
        })
    }) else {
        return;
    };
    let entries = [
        MenuEntry::command("Reveal in Explorer", CommandId::NoteRevealInExplorer),
        MenuEntry::command("Close notebook", CommandId::CloseNotebook),
    ];
    match super::menus::track_popup(hwnd, &entries, at) {
        Some(CommandId::NoteRevealInExplorer) => {
            if let Some(root) = super::library_host::folder(hwnd) {
                super::library_host::reveal(hwnd, &root);
            }
        }
        Some(command) => run(hwnd, command),
        None => {}
    }
}

fn state_button(hwnd: HWND) {
    match with_view(hwnd, |view| view.mode) {
        Some(Mode::NoNotebook) => super::library_host::choose_and_open_folder(hwnd),
        Some(Mode::Empty) => run(hwnd, CommandId::New),
        Some(Mode::Failed) => super::library_host::retry_load(hwnd),
        _ => {}
    }
}

/// The tree's keys (spec §10). Returns false for keys it leaves to the panel.
pub(crate) fn key_down(hwnd: HWND, key: u16) -> bool {
    if let Some(list_key) = ListKey::from_virtual_key(u32::from(key)) {
        with_view(hwnd, |view| {
            let height = view.list_height();
            if view.list.move_selection(list_key, height) {
                view.invalidate();
            }
        });
        return true;
    }
    let Some(selected) = with_view(hwnd, |view| view.list.selected).flatten() else {
        return matches!(key, VK_RETURN | VK_LEFT | VK_RIGHT | VK_F2 | VK_DELETE);
    };
    match key {
        VK_RETURN => {
            let ctrl = unsafe { GetKeyState(VK_CONTROL as i32) } < 0;
            let how = if ctrl {
                Activation::Permanent
            } else {
                Activation::Enter
            };
            activate(hwnd, selected, how);
            true
        }
        VK_RIGHT => {
            right(hwnd, selected);
            true
        }
        VK_LEFT => {
            left(hwnd, selected);
            true
        }
        VK_F2 | VK_DELETE => {
            let kind = with_view(hwnd, |view| match view.target(selected) {
                Target::Row(row) => Some(row.kind),
                _ => None,
            })
            .flatten();
            let Some(root) = super::library_host::folder(hwnd) else {
                return true;
            };
            match kind {
                Some(RowKind::Note(relative)) if super::library_host::ready_library(hwnd) => {
                    let path = root.join(relative);
                    if key == VK_F2 {
                        super::library_host::rename_file(hwnd, &path);
                    } else {
                        super::library_host::delete_file(hwnd, &path);
                    }
                }
                Some(RowKind::Folder(relative)) if super::library_host::ready_library(hwnd) => {
                    if key == VK_F2 {
                        super::library_host::rename_folder(hwnd, &relative);
                    } else {
                        super::library_host::delete_folder(hwnd, &relative);
                    }
                }
                _ => {}
            }
            true
        }
        _ => false,
    }
}

/// Right expands a folder, or moves into an expanded one.
fn right(hwnd: HWND, index: usize) {
    let Some(Target::Row(row)) = with_view(hwnd, |view| view.target(index)) else {
        return;
    };
    let RowKind::Folder(relative) = &row.kind else {
        return;
    };
    if !row.expanded {
        set_folder_expanded(hwnd, relative, true);
        return;
    }
    with_view(hwnd, |view| {
        if view
            .rows
            .get(index + 1)
            .is_some_and(|child| child.depth > row.depth)
        {
            view.select(index + 1);
        }
    });
}

/// Left collapses an expanded folder, or moves to the parent folder.
fn left(hwnd: HWND, index: usize) {
    let Some(Target::Row(row)) = with_view(hwnd, |view| view.target(index)) else {
        return;
    };
    if let RowKind::Folder(relative) = &row.kind
        && row.expanded
    {
        set_folder_expanded(hwnd, relative, false);
        return;
    }
    with_view(hwnd, |view| {
        if let Some(parent) = tree::parent_index(&view.rows, index) {
            view.select(parent);
        }
    });
}

/// Type-ahead: the next row whose name starts with what was typed in the last second. A single
/// letter searches from the row after the selection, so repeating it steps through matches.
fn typed(hwnd: HWND, ch: char) {
    with_view(hwnd, |view| {
        if view.mode != Mode::Tree {
            return;
        }
        let prefix = view.typed.push(ch, Instant::now()).to_owned();
        let from = match view.list.selected {
            Some(selected) if prefix.chars().count() == 1 => selected + 1,
            Some(selected) => selected,
            None => 0,
        };
        if let Some(index) = tree::type_ahead(&view.rows, from, &prefix) {
            view.select(index);
        }
    });
}

impl crate::window::sidebar_accessibility::AccessibleView for NotebookView {
    /// Push buttons first, then the RECENT notebooks (no-notebook state), then the tree rows.
    fn accessible_count(&self, client: RECT, dpi: u32) -> usize {
        self.buttons(client, dpi).len() + self.recent.len() + self.rows().len()
    }

    fn accessible_item(
        &self,
        index: usize,
        client: RECT,
        dpi: u32,
        focused: bool,
    ) -> Option<crate::window::sidebar_accessibility::AccessibleItem> {
        use crate::window::sidebar_accessibility::{button_item, list_item, row_rect, tree_item};
        let buttons = self.buttons(client, dpi);
        if let Some((name, rect)) = buttons.get(index) {
            return Some(button_item(name, false, false, *rect));
        }
        let index = index - buttons.len();
        if index < self.recent.len() {
            // The no-notebook list holds the RECENT rows, indexed by list position.
            let (rect, visible) = row_rect(self.list_area(client, dpi), self.list(), index);
            return Some(list_item(
                &self.recent_name(index),
                self.list().selected == Some(index),
                focused,
                rect,
                visible,
            ));
        }
        let index = index - self.recent.len();
        let row = self.rows().get(index)?;
        let (rect, visible) = row_rect(self.list_area(client, dpi), self.list(), index);
        Some(tree_item(
            row,
            self.list().selected == Some(index),
            focused,
            rect,
            visible,
        ))
    }

    fn accessible_hit(&self, point: POINT, client: RECT, dpi: u32) -> Option<usize> {
        let inside = |rect: &RECT| {
            point.x >= rect.left
                && point.x < rect.right
                && point.y >= rect.top
                && point.y < rect.bottom
        };
        let buttons = self.buttons(client, dpi);
        if let Some(index) = buttons.iter().position(|(_, rect)| inside(rect)) {
            return Some(index);
        }
        let area = self.list_area(client, dpi);
        if !inside(&area) {
            return None;
        }
        let row = self.list().row_at(point.y - area.top)?;
        if self.mode == Mode::NoNotebook {
            (row < self.recent.len()).then_some(buttons.len() + row)
        } else {
            (row < self.rows().len()).then_some(buttons.len() + self.recent.len() + row)
        }
    }

    fn accessible_current(&self, client: RECT, dpi: u32) -> Option<usize> {
        let buttons = self.buttons(client, dpi).len();
        let selected = self.list().selected?;
        // In the no-notebook state the list's selection is a RECENT row, not a tree row.
        if self.mode == Mode::NoNotebook {
            (selected < self.recent.len()).then_some(buttons + selected)
        } else {
            (selected < self.rows().len()).then_some(buttons + self.recent.len() + selected)
        }
    }

    fn accessible_select(&mut self, index: usize, client: RECT, dpi: u32) {
        let buttons = self.buttons(client, dpi).len();
        let (offset, rows) = if self.mode == Mode::NoNotebook {
            (buttons, self.recent.len())
        } else {
            (buttons + self.recent.len(), self.rows().len())
        };
        let Some(row) = index.checked_sub(offset).filter(|&row| row < rows) else {
            return;
        };
        let area = self.list_area(client, dpi);
        self.list_mut().select(row, area.bottom - area.top);
    }

    fn accessible_identity(&self, index: usize, client: RECT, dpi: u32) -> Option<u64> {
        use crate::window::sidebar_accessibility::identity_of;
        let index = index.checked_sub(self.buttons(client, dpi).len())?;
        if let Some(folder) = self.recent.get(index) {
            return Some(identity_of(folder));
        }
        let row = self.rows().get(index - self.recent.len())?;
        Some(identity_of(&row.kind))
    }

    fn accessible_generation(&self) -> u64 {
        self.order
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `RECT` has no `PartialEq` or `Debug` in windows-sys.
    fn edges(rect: RECT) -> (i32, i32, i32, i32) {
        (rect.left, rect.top, rect.right, rect.bottom)
    }

    fn row(kind: RowKind, depth: u16) -> TreeRow {
        TreeRow {
            kind,
            depth,
            name: String::new(),
            pinned: false,
            expanded: false,
        }
    }

    #[test]
    fn a_row_indents_by_depth_and_keeps_the_pin_at_the_right_edge() {
        // Break caught: deep rows pushing the pin off the row, names drawn over the chevron, or
        // an inverted name rectangle in a panel narrower than the indent.
        let rect = RECT {
            left: 0,
            top: 26,
            right: 260,
            bottom: 52,
        };
        let top = row_parts(rect, 0, 96);
        let deep = row_parts(rect, 3, 96);
        assert_eq!(top.chevron.left, 8);
        assert_eq!(deep.chevron.left, 8 + 3 * 12);
        assert_eq!(edges(top.pin), (236, 26, 260, 52));
        assert_eq!(edges(deep.pin), edges(top.pin));
        assert!(deep.name.left >= deep.icon.right);
        assert_eq!(deep.name.right, deep.pin.left);
        let cramped = row_parts(
            RECT {
                left: 0,
                top: 0,
                right: 40,
                bottom: 26,
            },
            9,
            96,
        );
        assert!(cramped.name.left <= cramped.name.right);
        assert_eq!(row_parts(rect, 1, 192).chevron.left, 16 + 24);
    }

    #[test]
    fn the_header_buttons_sit_right_to_left_and_the_title_stops_before_them() {
        // Break caught: the notebook name drawn under the star, or buttons that do not follow
        // the panel's right edge when it is resized.
        let area = RECT {
            left: 0,
            top: 0,
            right: 260,
            bottom: 600,
        };
        let layout = header_layout(area, 96);
        let [
            (first, star),
            (second, new),
            (third, folder),
            (fourth, more),
        ] = layout.buttons;
        assert_eq!(
            (first, second, third, fourth),
            (
                HeaderButton::Favorite,
                HeaderButton::NewNote,
                HeaderButton::NewFolder,
                HeaderButton::More
            )
        );
        assert_eq!(edges(more), (226, 5, 254, 33));
        assert_eq!(
            (star.right, new.right, folder.right),
            (new.left, folder.left, more.left),
            "New folder sits right of New note"
        );
        assert!(layout.title.right <= star.left);
        assert_eq!(layout.title.bottom, 38);
        assert_eq!(body_rect(area, 96).top, 38);
    }

    #[test]
    fn the_no_notebook_state_lists_recent_notebooks_below_its_button() {
        // Break caught: the RECENT rows painted over the Open notebook… button, or a list rect
        // that turns inside out in a short panel.
        let body = RECT {
            left: 0,
            top: 38,
            right: 260,
            bottom: 600,
        };
        let layout = state_layout(body, 96);
        assert!(layout.message.bottom <= layout.button.top);
        assert!(layout.button.bottom <= layout.label.top);
        assert_eq!(layout.list.top, layout.label.bottom);
        assert_eq!(layout.list.bottom, 600);
        let short = state_layout(
            RECT {
                left: 0,
                top: 38,
                right: 260,
                bottom: 60,
            },
            96,
        );
        assert!(short.list.top <= short.list.bottom);
    }

    #[test]
    fn a_vanished_selection_moves_to_the_row_that_took_its_place() {
        // Break caught: a stale index past the end after a rescan removed rows, or a selection
        // that jumps to the top instead of staying where it was.
        let rows = vec![
            row(RowKind::Folder("sub".into()), 0),
            row(RowKind::Note(r"sub\a.md".into()), 1),
            row(RowKind::Note("c.md".into()), 0),
        ];
        let a = RowKind::Note(r"sub\a.md".into());
        assert_eq!(
            follow(&rows, Some(&a), Some(7)),
            Some(1),
            "found by path first"
        );
        let gone = RowKind::Note(r"sub\b.md".into());
        assert_eq!(follow(&rows, Some(&gone), Some(2)), Some(2));
        assert_eq!(follow(&rows, Some(&gone), Some(9)), Some(2));
        assert_eq!(follow(&[], Some(&gone), Some(1)), None);
        assert_eq!(
            follow(&rows, None, Some(1)),
            None,
            "nothing selected stays so"
        );
    }

    #[test]
    fn unsaved_rows_come_from_untitled_tabs_labelled_by_their_first_line() {
        // Break caught: saved tabs listed twice (as a note and as unsaved), or untitled tabs
        // with a blank first line shown with no label at all.
        let mut labelled = Document::test_fixture(DocumentId(4), true);
        labelled.first_line_label = Some("Groceries".to_owned());
        let blank = Document::test_fixture(DocumentId(5), false);
        let mut saved = Document::test_fixture(DocumentId(6), false);
        saved.path = Some(PathBuf::from(r"C:\n\a.md"));
        let entries = unsaved_entries([&labelled, &blank, &saved].into_iter());
        let entries: Vec<_> = entries.into_iter().map(|e| (e.key, e.label)).collect();
        assert_eq!(
            entries,
            [(4, "Groceries".to_owned()), (5, "Untitled".to_owned())]
        );
    }

    #[test]
    fn type_ahead_extends_the_prefix_within_a_second_and_starts_over_after() {
        // Break caught: a prefix that never resets, so a second search a minute later matches
        // nothing.
        let start = Instant::now();
        let mut typed = TypeAhead::default();
        assert_eq!(typed.push('n', start), "n");
        assert_eq!(typed.push('o', start + Duration::from_millis(900)), "no");
        assert_eq!(typed.push('x', start + Duration::from_millis(2_000)), "x");
    }

    #[test]
    fn a_thousand_expanded_folders_of_ten_notes_flatten_within_a_frame() {
        // Break caught: a linear scan of the expanded list per folder row (with two lowercased
        // allocations per comparison), which made a tab switch in a big, fully expanded
        // notebook take close to 100 ms.
        let notes: Vec<PathBuf> = (0..1_000)
            .flat_map(|folder| {
                (0..10).map(move |note| PathBuf::from(format!(r"Folder {folder}\Note {note}.md")))
            })
            .collect();
        let tree = NoteTree::build(&notes, &[], &[]);
        // Stored as the per-PC file may spell them: case differences still match.
        let expanded: Vec<PathBuf> = (0..1_000)
            .map(|folder| PathBuf::from(format!("folder {folder}")))
            .collect();
        let started = Instant::now();
        let rows = flatten(&tree, &expanded, &[]);
        let elapsed = started.elapsed();
        assert_eq!(rows.len(), 11_000);
        assert!(rows[0].expanded);
        assert_eq!(flatten(&tree, &[], &[]).len(), 1_000);
        if !cfg!(debug_assertions) {
            assert!(elapsed < Duration::from_millis(16), "{elapsed:?}");
        }
    }
}
