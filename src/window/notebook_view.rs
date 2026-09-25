//! The Notebook view (spec §6): the open notebook's folder tree in the side panel, with its
//! header, pins, type-ahead and the loading, no-notebook and empty states. The tree itself is
//! built on the scan worker (`LibraryState.tree`). This module flattens the expanded part into
//! rows, paints only the rows on screen, and turns clicks and keys into `open_note` calls.

use super::main_window::{OpenMode, app_ptr};
use super::side_panel::{UiFonts, ViewPaint, draw_text, point_of};
use crate::config::FileIconSet;
use crate::document::DocumentId;
use crate::library::tree::{self, NoteTree, RowKind, TreeRow};
use crate::window::commands::CommandId;
use crate::window::drag_label::{DragLabel, LabelImage};
use crate::window::file_icons::note_kind;
use crate::window::icon_sets::images::IconImages;
use crate::window::icon_sets::{TreeIcon, TreeItem, minimal, tree_icon};
use crate::window::inline_name::{FieldLayout, InlineName};
use crate::window::menus::MenuEntry;
use crate::window::notebook_layout::{self, PanelLayout, ROW_HEIGHT};
use crate::window::open_editors::OpenEditors;
use crate::window::palette::{FileIcons, Palette};
use crate::window::panel::{fill, inset, scale};
use crate::window::panel_cursor::{self, Cursor};
use crate::window::row_list::{self, ListKey, RowListState, RowLook, row_foreground};
use crate::window::sidebar_accessibility::MK_LBUTTON;
use crate::window::tooltip::Tooltip;
use crate::window::tree_drag::{self, Drag, DragSource, Hover};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    ClientToScreen, DT_CALCRECT, DT_CENTER, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_SINGLELINE,
    DT_VCENTER, DT_WORDBREAK, DrawTextW, GetDC, GetTextExtentPoint32W, HDC, HFONT, InvalidateRect,
    ReleaseDC, ScreenToClient, SelectObject,
};
use windows_sys::Win32::UI::Controls::WM_MOUSELEAVE;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetCapture, GetFocus, GetKeyState, ReleaseCapture, SetCapture, SetFocus, TME_LEAVE,
    TRACKMOUSEEVENT, TrackMouseEvent, VK_CONTROL, VK_DELETE, VK_F2, VK_LEFT, VK_RETURN, VK_RIGHT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetClientRect, GetCursorPos, GetParent, GetSystemMetrics, IDC_ARROW, IDC_NO, KillTimer,
    LoadCursorW, SM_CXDRAG, SM_CYDRAG, SendMessageW, SetCursor, SetTimer, WM_CAPTURECHANGED,
    WM_CHAR, WM_COMMAND, WM_CONTEXTMENU, WM_KEYDOWN, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_RBUTTONDOWN,
    WM_RBUTTONUP, WM_TIMER,
};

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
    /// A drag of a row, armed by a press and under way past the drag distance (tree drag spec
    /// §3).
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

/// Whether row `index` is the one a started drag carries (tree drag spec §3.2): both sides
/// absent (no drag, and `index` past the rows the list actually has, such as the truncated row)
/// must not read as a match. Only a dragged row matches: a tab or files are not tree rows.
fn is_dragged_row(dragged: Option<&DragSource>, rows: &[TreeRow], index: usize) -> bool {
    let Some(DragSource::Row(dragged)) = dragged else {
        return false;
    };
    rows.get(index).is_some_and(|row| &row.kind == dragged)
}

/// Draws `item`'s icon from `set` at `px` square, centred in `rect` and clipped to it (a deep row
/// in a narrow panel gets a box narrower than `px`). `muted` is every icon's colour in high
/// contrast.
#[allow(
    clippy::too_many_arguments,
    reason = "one icon's paint inputs, shared by a tree row and the drag label"
)]
pub(crate) fn draw_item_icon(
    dc: HDC,
    item: TreeItem,
    rect: RECT,
    px: i32,
    muted: u32,
    palette: &Palette,
    icons: &FileIcons,
    images: &mut IconImages,
    set: FileIconSet,
    light_theme: bool,
) {
    // A type icon keeps its colour on a selected or hovered row: the colours are mid-tones that
    // read on the selection. High contrast draws Minimal in the muted system pair in every set
    // (icon sets spec §3.2), and a Material bitmap that cannot be made falls back to Minimal
    // (§6).
    let mask = match tree_icon(set, item, light_theme, palette.high_contrast) {
        TreeIcon::Image(icon) if images.draw(dc, icon, rect, px as u32) => return,
        TreeIcon::Image(_) => minimal(item),
        mask => mask,
    };
    if let TreeIcon::Mask { set, icon, color } = mask {
        let color = if palette.high_contrast {
            muted
        } else {
            icons.color(color)
        };
        images.draw_mask(dc, set, icon, color, rect, px as u32);
    }
}

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
    editing: Option<TreeItem>,
    images: &mut IconImages,
    set: FileIconSet,
    light_theme: bool,
    // The row being dragged: its name draws muted (tree drag spec §3.2).
    dimmed: bool,
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
    // `px` is the full icon box, not the box clamped by `row_parts` to fit a narrow panel: a
    // clipped box draws part of the icon instead of resampling it smaller and caching a bitmap
    // per clipped width.
    let px = scale(GLYPH_BOX, dpi);
    let mut draw_icon = |item: TreeItem| {
        draw_item_icon(
            dc,
            item,
            parts.icon,
            px,
            muted,
            palette,
            icons,
            images,
            set,
            light_theme,
        );
    };
    match &row.kind {
        RowKind::Folder(_) => {
            let chevron = if row.expanded {
                GLYPH_CHEVRON_DOWN
            } else {
                GLYPH_CHEVRON_RIGHT
            };
            unsafe { draw_text(dc, chevron, parts.chevron, fonts.glyph, muted, CENTERED) };
            draw_icon(TreeItem::Folder {
                expanded: row.expanded,
            });
        }
        RowKind::Note(path) => {
            let extension = path
                .extension()
                .map(|extension| extension.to_string_lossy());
            draw_icon(TreeItem::Note(note_kind(extension.as_deref())));
        }
        RowKind::Draft => {
            if let Some(item) = editing {
                draw_icon(item);
            }
        }
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
    // The field covers an edited row's name (inline naming spec §3.3).
    if editing.is_some() {
        return;
    }
    let font = fonts.text;
    let color = if dimmed {
        palette.muted_foreground
    } else {
        foreground
    };
    unsafe { draw_text(dc, &row.name, parts.name, font, color, LINE) };
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

/// The icon a dragged row's label shows (tree drag spec §3.2): a closed folder or the note's
/// type. `None` for rows that can't be dragged.
fn drag_item(kind: &RowKind) -> Option<TreeItem> {
    match kind {
        RowKind::Folder(_) => Some(TreeItem::Folder { expanded: false }),
        RowKind::Note(path) => {
            let extension = path
                .extension()
                .map(|extension| extension.to_string_lossy());
            Some(TreeItem::Note(note_kind(extension.as_deref())))
        }
        RowKind::Draft => None,
    }
}

/// The drag label's size for a name `text` pixels wide: padding, the icon, a gap, the name,
/// padding.
fn drag_label_size(text: i32, dpi: u32) -> SIZE {
    SIZE {
        cx: 2 * scale(LABEL_PAD, dpi) + scale(GLYPH_BOX, dpi) + scale(GAP, dpi) + text,
        cy: scale(LABEL_HEIGHT, dpi),
    }
}

/// The drag label's fill, border and text colours: the hover fill with a muted border, or in
/// high contrast only the system window pair.
fn drag_label_colors(palette: &Palette) -> (u32, u32, u32) {
    if palette.high_contrast {
        (
            palette.editor_background,
            palette.editor_foreground,
            palette.editor_foreground,
        )
    } else {
        (
            palette.hover_background,
            palette.muted_foreground,
            palette.editor_foreground,
        )
    }
}

/// Paints the drag label for `item` and `name` over the whole of `dc`'s `size` (tree drag spec
/// §3.2): a 1 px (scaled) border, the icon the row shows, and the name, cut with an ellipsis.
fn paint_drag_label(
    dc: HDC,
    size: SIZE,
    item: TreeItem,
    name: &str,
    paint: &ViewPaint,
    images: &mut IconImages,
) {
    let (palette, dpi) = (&paint.palette, paint.dpi);
    let (background, border, text) = drag_label_colors(palette);
    let whole = RECT {
        left: 0,
        top: 0,
        right: size.cx,
        bottom: size.cy,
    };
    unsafe {
        fill(dc, whole, border);
        fill(dc, inset(whole, scale(1, dpi).max(1)), background);
    }
    let (pad, px) = (scale(LABEL_PAD, dpi), scale(GLYPH_BOX, dpi));
    let top = (size.cy - px) / 2;
    let icon = RECT {
        left: pad,
        top,
        right: pad + px,
        bottom: top + px,
    };
    draw_item_icon(
        dc,
        item,
        icon,
        px,
        palette.muted_foreground,
        palette,
        &paint.icons,
        images,
        paint.icon_set,
        paint.light_theme,
    );
    let name_rect = RECT {
        left: icon.right + scale(GAP, dpi),
        top: 0,
        right: size.cx - pad,
        bottom: size.cy,
    };
    unsafe { draw_text(dc, name, name_rect, paint.fonts.text, text, LINE) };
}

/// Where `highlight` shows in the tree's `list` area (tree drag spec §3.2): the whole list for
/// the root, else the part of the folder's rows in view; `None` when none of them is.
pub(crate) fn band_rect(
    list: RECT,
    state: &RowListState,
    highlight: tree_drag::Highlight,
) -> Option<RECT> {
    match highlight {
        tree_drag::Highlight::Root => Some(list),
        tree_drag::Highlight::Rows { start, end } => {
            let visible = state.visible_rows(height(list));
            let first = start.max(state.top);
            let last = end.min(state.top + visible);
            (first < last).then(|| RECT {
                left: list.left,
                top: list.top + (first - state.top) as i32 * state.row_height,
                right: list.right,
                bottom: (list.top + (last - state.top) as i32 * state.row_height).min(list.bottom),
            })
        }
    }
}

/// The drop target's band (tree drag spec §3.2). Called before the rows paint
/// (`before_rows`), it fills the band with the softer selection colour; after, in high
/// contrast only, it outlines the band in the system highlight, since a blend is not allowed
/// there.
fn paint_band(dc: HDC, band: RECT, palette: &Palette, dpi: u32, before_rows: bool) {
    if before_rows && !palette.high_contrast {
        unsafe { fill(dc, band, palette.inactive_selection_background) };
    } else if !before_rows && palette.high_contrast {
        paint_outline(dc, band, palette.selection_background, dpi);
    }
}

/// A 1 px (scaled) outline just inside `rect`.
fn paint_outline(dc: HDC, rect: RECT, color: u32, dpi: u32) {
    let t = scale(1, dpi).max(1);
    for edge in [
        RECT {
            bottom: rect.top + t,
            ..rect
        },
        RECT {
            top: rect.bottom - t,
            ..rect
        },
        RECT {
            right: rect.left + t,
            ..rect
        },
        RECT {
            left: rect.right - t,
            ..rect
        },
    ] {
        unsafe { fill(dc, edge, color) };
    }
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

    /// The list's rectangle for the current mode, in the panel's `client` coordinates at `dpi`:
    /// the tree rows, the RECENT rows, or an empty band while there are none.
    pub(crate) fn list_area(&self, client: RECT, dpi: u32) -> RECT {
        let body = self.layout(client, dpi).body;
        let empty = RECT {
            bottom: body.top,
            ..body
        };
        match self.mode {
            Mode::Tree if self.root_expanded => body,
            Mode::NoNotebook => state_layout(body, dpi).list,
            _ => empty,
        }
    }

    /// The panel's bands for the view as it is (open editors spec §3.1).
    pub(crate) fn layout(&self, client: RECT, dpi: u32) -> PanelLayout {
        notebook_layout::panel_layout(client, dpi, self.editors.rows.len(), self.editors_expanded)
    }

    /// The tree is painted and hit: loaded rows under an expanded root.
    pub(crate) fn tree_shown(&self) -> bool {
        self.mode == Mode::Tree && self.root_expanded
    }

    /// Whether the tree has rows the collapsed root hides, which the keyboard must not reach
    /// (open editors spec §3.5).
    fn tree_hidden(&self) -> bool {
        self.mode == Mode::Tree && !self.root_expanded
    }

    /// How many rows the keyboard selection runs through in each part (`panel_cursor::step`).
    fn shape(&self) -> panel_cursor::Shape {
        panel_cursor::Shape {
            editors: if self.editors_expanded {
                self.editors.rows.len()
            } else {
                0
            },
            root: self.mode != Mode::NoNotebook,
            // The list after the root row: the RECENT notebooks without a notebook, else the
            // tree's rows while it shows.
            tree: match self.mode {
                Mode::NoNotebook => self.recent.len(),
                _ if self.tree_shown() => self.rows.len(),
                _ => 0,
            },
        }
    }

    /// Keeps the Open Editors rows' scroll within their list as it is now, the active row in
    /// view. Not while the list has no height (collapsed, or a panel not yet sized): a 0 px list
    /// would scroll the active row to the top and leave the rows above it off screen once shown.
    fn fit_editors(&mut self, active_in_view: bool) {
        let height = height(self.layout(self.client(), self.dpi()).editors_list);
        if height <= 0 {
            return;
        }
        if active_in_view && let Some(active) = self.editors.active_index() {
            self.editors.list.ensure_visible(active, height);
        }
        self.editors.list.scroll_lines(0, height);
    }

    fn editors_row_rect(&self, list: RECT, index: usize) -> Option<RECT> {
        let top = self.editors.list.row_top(index)?;
        Some(RECT {
            left: list.left,
            top: list.top + top,
            right: list.right,
            bottom: list.top + top + self.editors.list.row_height,
        })
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

    /// The inline field's layout in `area` (the panel's client rectangle) at `dpi`: over the
    /// edited row's name, clipped to the list. `None` while nothing is edited or the row is out
    /// of view.
    fn inline_layout_in(&self, area: RECT, dpi: u32) -> Option<FieldLayout> {
        if !self.tree_shown() {
            return None;
        }
        let index = self.inline.row()?;
        let list = self.list_area(area, dpi);
        let top = self.list.row_top(index)?;
        let row = RECT {
            left: list.left,
            top: list.top + top,
            right: list.right,
            bottom: list.top + top + self.list.row_height,
        };
        crate::window::inline_name::field_layout(
            row,
            list,
            self.rows.get(index)?.depth,
            dpi,
            self.inline.text_height(),
        )
    }

    /// `inline_layout_in` for the panel as it is now (`inline_name::place`).
    pub(crate) fn inline_layout(&self) -> Option<FieldLayout> {
        self.inline_layout_in(self.client(), self.dpi())
    }

    /// Brings the edited row into view: a draft row scrolled to, a renamed row selected too
    /// (inline naming spec §3.4).
    pub(crate) fn reveal_edit(&mut self) {
        let Some(index) = self.inline.row() else {
            return;
        };
        let height = self.list_height();
        if self.inline.draft_at().is_some() {
            self.list.ensure_visible(index, height);
        } else {
            self.list.select(index, height);
        }
        self.invalidate();
    }

    /// Row `index`'s rectangle in panel coordinates, for tests that click a row.
    #[cfg(test)]
    pub(crate) fn row_rect_at(&self, index: usize) -> Option<RECT> {
        self.row_rect(self.list_rect(self.client()), index)
    }

    /// Open Editors row `index`'s rectangle in panel coordinates, for tests that click it.
    #[cfg(test)]
    pub(crate) fn editor_rect_at(&self, index: usize) -> Option<RECT> {
        let list = self.layout(self.client(), self.dpi()).editors_list;
        self.editors_row_rect(list, index)
    }

    /// The Open Editors header row, in panel coordinates.
    #[cfg(test)]
    pub(crate) fn editors_header_rect(&self) -> RECT {
        self.layout(self.client(), self.dpi()).editors_header
    }

    /// The notebook's root row, in panel coordinates.
    #[cfg(test)]
    pub(crate) fn root_rect(&self) -> RECT {
        self.layout(self.client(), self.dpi()).root
    }

    /// A point on the scroll thumb in panel coordinates, while the list scrolls: its left edge,
    /// clear of the sidebar's resize grip, halfway down.
    #[cfg(test)]
    pub(crate) fn thumb_point(&self) -> Option<(i32, i32)> {
        let list = self.list_rect(self.client());
        let (top, length) = self.list.thumb(height(list))?;
        let left = list.right - row_list::thumb_width(self.list.row_height);
        Some((left, list.top + top + length / 2))
    }

    /// The inline field's frame, in the accent colour or the error colour while a problem
    /// shows, and the problem under the field, or above it without room below (inline naming
    /// spec §4.4).
    fn paint_inline(
        &self,
        dc: HDC,
        layout: FieldLayout,
        list: RECT,
        palette: &Palette,
        fonts: UiFonts,
        dpi: u32,
    ) {
        let problem = self.inline.problem();
        let outline = if problem.is_some() {
            palette.error_foreground
        } else {
            palette.selection_background
        };
        unsafe {
            fill(dc, layout.frame, outline);
            fill(dc, inset(layout.frame, 1), palette.editor_background);
        }
        let Some(problem) = problem else {
            return;
        };
        let pad = scale(4, dpi);
        let text: Vec<u16> = problem.encode_utf16().collect();
        let mut measured = RECT {
            left: 0,
            top: 0,
            right: (layout.frame.right - layout.frame.left - 2 * pad).max(1),
            bottom: 0,
        };
        unsafe {
            let previous = SelectObject(dc, fonts.text);
            DrawTextW(
                dc,
                text.as_ptr(),
                text.len() as i32,
                &mut measured,
                DT_CALCRECT | DT_WORDBREAK | DT_NOPREFIX,
            );
            SelectObject(dc, previous);
        }
        let rect =
            crate::window::inline_name::message_rect(layout.frame, list, measured.bottom + 2 * pad);
        let inner = RECT {
            left: rect.left + pad,
            top: rect.top + pad,
            right: rect.right - pad,
            bottom: rect.bottom - pad,
        };
        unsafe {
            fill(dc, rect, palette.error_foreground);
            fill(dc, inset(rect, 1), palette.editor_background);
            draw_text(
                dc,
                problem,
                inner,
                fonts.text,
                palette.error_foreground,
                DT_WORDBREAK | DT_NOPREFIX,
            );
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

    fn hit_test(&self, x: i32, y: i32) -> Hit {
        let area = self.client();
        let dpi = self.dpi();
        let layout = self.layout(area, dpi);
        if y < layout.title.bottom {
            return Hit::Empty;
        }
        if contains(layout.editors_header, x, y) {
            return Hit::EditorsHeader;
        }
        if contains(layout.editors_list, x, y) {
            let Some(index) = self.editors.list.row_at(y - layout.editors_list.top) else {
                return Hit::Empty;
            };
            let row = self.editors_row_rect(layout.editors_list, index);
            let clean = self.editors.rows.get(index).is_some_and(|row| !row.dirty);
            let close = clean
                && row.is_some_and(|row| contains(super::open_editors::close_rect(row, dpi), x, y));
            return Hit::Editor { index, close };
        }
        if contains(layout.root, x, y) {
            if self.mode != Mode::NoNotebook {
                let parts = notebook_layout::root_parts(layout.root, dpi);
                for (button, rect) in parts.buttons {
                    if contains(rect, x, y) {
                        return Hit::Header(button);
                    }
                }
                return Hit::Root;
            }
            return Hit::Empty;
        }
        let body = layout.body;
        match self.mode {
            // The states under a collapsed root are not painted.
            Mode::Empty | Mode::Failed if !self.root_expanded => Hit::Empty,
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
            Mode::Tree if !self.tree_shown() => Hit::Empty,
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

    /// What a drag at panel point `x`, `y` is over (tree drag spec §3.2). The scroll thumb
    /// counts as the row under it. A notebook with no notes has the root row and the space under
    /// it, both the root (open editors spec §4.1); a tree row can't be dragged there.
    fn drag_hover(&self, x: i32, y: i32) -> Hover {
        let area = self.client();
        if !matches!(self.mode, Mode::Tree | Mode::Empty) || !contains(area, x, y) {
            return Hover::Outside;
        }
        // The title band and Open Editors take no drop; the root row is the notebook's root.
        let layout = self.layout(area, self.dpi());
        if y < layout.root.top {
            return Hover::Outside;
        }
        if y < layout.root.bottom {
            return Hover::Header;
        }
        if !self.tree_shown() {
            return Hover::Below;
        }
        let list = self.list_rect(area);
        if y < list.top {
            return Hover::Header;
        }
        self.list
            .row_at(y - list.top)
            .map_or(Hover::Below, Hover::Row)
    }

    /// Whether an Explorer drop at panel point `x`, `y` opens its files rather than copying them
    /// (open editors spec §4.1): over Open Editors, or anywhere below the title band with no
    /// notebook.
    fn opens_at(&self, x: i32, y: i32) -> bool {
        let area = self.client();
        let layout = self.layout(area, self.dpi());
        if self.root.is_none() {
            return contains(area, x, y) && y >= layout.title.bottom;
        }
        contains(layout.editors_header, x, y) || contains(layout.editors_list, x, y)
    }

    /// The drag moved to `x`, `y`: the target follows, and the highlight repaints when it
    /// changed. Whether a release there moves the item.
    fn drag_to(&mut self, x: i32, y: i32, now: Instant) -> bool {
        let hover = self.drag_hover(x, y);
        let Some(drag) = self.drag.as_mut() else {
            return false;
        };
        let root = self.root.clone().unwrap_or_default();
        let changed = drag.hover(&self.rows, &root, (x, y), hover, now);
        let accepted = drag.target.is_some();
        if changed {
            self.invalidate();
        }
        accepted
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

    /// The started drag's label painted with `paint`, and the dragged row's or tab's name (tree
    /// drag spec §3.2). `None` without a started drag, for dropped files, or if GDI can't make
    /// the image.
    fn drag_label_image(&mut self, paint: &ViewPaint) -> Option<(LabelImage, String)> {
        let source = self
            .drag
            .as_ref()
            .filter(|drag| drag.started)?
            .source
            .clone();
        let (item, name) = match &source {
            DragSource::Row(kind) => {
                let item = drag_item(kind)?;
                let name = self.rows.iter().find(|row| &row.kind == kind)?.name.clone();
                (item, name)
            }
            DragSource::Tab { path, .. } => {
                let extension = path
                    .extension()
                    .map(|extension| extension.to_string_lossy());
                (
                    TreeItem::Note(note_kind(extension.as_deref())),
                    super::tree_copy::item_name(path),
                )
            }
            DragSource::Files(_) => return None,
        };
        let text = self
            .text_width(&name, paint.fonts.text)
            .min(scale(LABEL_MAX_TEXT, paint.dpi));
        let size = drag_label_size(text, paint.dpi);
        let image = LabelImage::new(size.cx, size.cy)?;
        paint_drag_label(image.dc, size, item, &name, paint, &mut self.images);
        Some((image, name))
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
                let font = fonts.text;
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
        let dpi = self.dpi();
        let layout = self.layout(area, dpi);
        let parts = notebook_layout::root_parts(layout.root, dpi);
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
        // An Open Editors row shows its path; a tree row its cut-off name.
        let editor_tip = self.editors.list.hover.and_then(|index| {
            let rect = self.editors_row_rect(layout.editors_list, index)?;
            let row = self.editors.rows.get(index)?;
            Some((rect, super::open_editors::tooltip(row)))
        });
        let (row_rect, row_text) = match editor_tip {
            Some(tip) => tip,
            None => self.row_tip(fonts),
        };
        let mut tools = vec![(TOOL_TITLE, parts.name, title)];
        for (button, rect) in parts.buttons {
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
        self.editors.list.row_height = scale(ROW_HEIGHT, dpi);
        let sections = self.layout(area, dpi);
        // A panel sized after the rows came (startup) or resized: the scroll stays in range.
        let editors_height = height(sections.editors_list);
        if editors_height > 0 {
            self.editors.list.scroll_lines(0, editors_height);
        }
        self.paint_sections(paint, sections);
        let layout = state_layout(sections.body, dpi);
        match self.mode {
            // The states under the root show only while it is expanded; without a notebook there
            // is no root to collapse.
            Mode::Loading | Mode::Empty | Mode::Failed | Mode::Tree if !self.root_expanded => {}
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
                self.inline.set_colors(*palette);
                let rows = &self.rows;
                let images = &mut self.images;
                let (icon_set, light_theme) = (paint.icon_set, paint.light_theme);
                let hover_pin = self.hover_pin;
                let icons = &paint.icons;
                let drag = self.drag.as_ref().filter(|drag| drag.started);
                let dragged = drag.map(|drag| drag.source.clone());
                let band = drag
                    .and_then(|drag| drag.target.as_deref())
                    .and_then(|folder| tree_drag::highlight(rows, folder))
                    .and_then(|highlight| band_rect(list, &self.list, highlight));
                if let Some(band) = band {
                    paint_band(dc, band, palette, dpi, true);
                }
                // The edited row leaves its name to the field; a draft row shows the icon for
                // what is typed so far (inline naming spec §3.1, §3.3).
                let edited = self
                    .inline
                    .row()
                    .map(|index| (index, self.inline.draft_icon()));
                row_list::paint(
                    dc,
                    list,
                    &self.list,
                    palette,
                    focused,
                    &mut |dc, index, rect, look| {
                        let editing =
                            edited.and_then(|(edited, icon)| (edited == index).then_some(icon));
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
                            editing,
                            images,
                            icon_set,
                            light_theme,
                            is_dragged_row(dragged.as_ref(), rows, index),
                        );
                    },
                );
                if let Some(band) = band {
                    paint_band(dc, band, palette, dpi, false);
                }
                if let Some(layout) = self.inline_layout_in(area, dpi) {
                    self.paint_inline(dc, layout, list, palette, fonts, dpi);
                }
            }
        }
    }

    fn paint_sections(&mut self, paint: &ViewPaint, layout: PanelLayout) {
        let (dc, palette, fonts, dpi) = (paint.hdc, &paint.palette, paint.fonts, paint.dpi);
        let bold = |text: &str, rect: RECT, color: u32| unsafe {
            draw_text(dc, text, rect, fonts.bold, color, LINE)
        };
        let title = RECT {
            left: layout.title.left + scale(12, dpi),
            ..layout.title
        };
        bold("NOTEBOOK", title, palette.muted_foreground);
        // Open Editors header: chevron, label and count.
        let chevron = notebook_layout::section_chevron(layout.editors_header, dpi);
        let glyph = if self.editors_expanded {
            GLYPH_CHEVRON_DOWN
        } else {
            GLYPH_CHEVRON_RIGHT
        };
        unsafe {
            draw_text(
                dc,
                glyph,
                chevron,
                fonts.glyph,
                palette.muted_foreground,
                CENTERED,
            )
        };
        let label = RECT {
            left: chevron.right,
            ..layout.editors_header
        };
        let count = self.editors.rows.len();
        bold(
            &format!("OPEN EDITORS  {count}"),
            label,
            palette.muted_foreground,
        );
        // The rows.
        let editors = &self.editors;
        let images = &mut self.images;
        let hover_close = editors.hover_close;
        row_list::paint(
            dc,
            layout.editors_list,
            &editors.list,
            palette,
            paint.focused,
            &mut |dc, index, rect, look| {
                if let Some(row) = editors.rows.get(index) {
                    super::open_editors::draw_editor_row(
                        dc,
                        row,
                        rect,
                        look,
                        paint,
                        images,
                        hover_close && look.hover,
                    );
                }
            },
        );
        // The root row: chevron, name, and its buttons (not without a notebook).
        let parts = notebook_layout::root_parts(layout.root, dpi);
        if self.mode == Mode::NoNotebook {
            bold(
                "NO NOTEBOOK",
                RECT {
                    left: parts.chevron.right,
                    ..layout.root
                },
                palette.muted_foreground,
            );
        } else {
            let glyph = if self.root_expanded {
                GLYPH_CHEVRON_DOWN
            } else {
                GLYPH_CHEVRON_RIGHT
            };
            unsafe {
                draw_text(
                    dc,
                    glyph,
                    parts.chevron,
                    fonts.glyph,
                    palette.muted_foreground,
                    CENTERED,
                )
            };
            bold(
                &self.name.to_uppercase(),
                parts.name,
                palette.muted_foreground,
            );
            for (button, rect) in parts.buttons {
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
        // The keyboard selection on a header or tab row; the tree shows its own.
        if paint.focused {
            let outlined = match self.cursor {
                Cursor::EditorsHeader => Some(layout.editors_header),
                Cursor::Editor(index) if self.editors_expanded => {
                    let list = layout.editors_list;
                    self.editors_row_rect(list, index)
                        .filter(|row| row.top < list.bottom)
                        .map(|row| RECT {
                            bottom: row.bottom.min(list.bottom),
                            ..row
                        })
                }
                Cursor::Root if self.mode != Mode::NoNotebook => Some(layout.root),
                Cursor::Root | Cursor::Editor(_) | Cursor::Tree => None,
            };
            if let Some(rect) = outlined {
                paint_outline(dc, rect, palette.selection_background, dpi);
            }
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

    /// The tree rows screen readers see: all but the draft row (inline naming spec §6).
    fn accessible_rows(&self) -> usize {
        self.rows.len() - usize::from(self.inline.draft_at().is_some())
    }

    /// The children after the push buttons, in order: the Open Editors header, its rows, the
    /// root row, then the RECENT notebooks or the tree rows.
    fn accessible_parts(&self) -> (usize, usize, usize) {
        let editors = if self.editors_expanded {
            self.editors.rows.len()
        } else {
            0
        };
        let root = usize::from(self.mode != Mode::NoNotebook);
        (1, editors, root)
    }

    /// The tree rows screen readers see: `accessible_rows`, while the tree shows.
    fn accessible_tree_rows(&self) -> usize {
        if self.tree_shown() {
            self.accessible_rows()
        } else {
            0
        }
    }

    /// The row that accessible tree row `index` stands for.
    fn row_of_accessible(&self, index: usize) -> usize {
        match self.inline.draft_at() {
            Some(draft) if index >= draft => index + 1,
            _ => index,
        }
    }

    /// Row `index`'s accessible tree row; `None` for the draft row.
    fn accessible_of_row(&self, index: usize) -> Option<usize> {
        match self.inline.draft_at() {
            Some(draft) if index == draft => None,
            Some(draft) if index > draft => Some(index - 1),
            _ => Some(index),
        }
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
            let root = self.layout(client, dpi).root;
            for (button, rect) in notebook_layout::root_parts(root, dpi).buttons {
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
        let state = state_layout(self.layout(client, dpi).body, dpi);
        match self.mode {
            // The states under a collapsed root are not painted.
            Mode::Empty | Mode::Failed if !self.root_expanded => {}
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
    /// The notebook's root row is expanded (true without a notebook).
    root_expanded: bool,
    /// The Open Editors section is expanded.
    editors_expanded: bool,
    key: RebuildKey,
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
fn rebuild_key(hwnd: HWND) -> RebuildKey {
    RebuildKey {
        root: super::library_host::folder(hwnd),
        loaded: super::library_host::with_state(hwnd, |_| ()).is_some(),
        failed: super::library_host::load_failed(hwnd),
        expansion: super::library_host::expansion_revision(hwnd),
        root_expanded: super::library_host::root_expanded(hwnd),
    }
}

/// Reads everything the rows need. Each call borrows the App on its own, never nested.
fn snapshot(hwnd: HWND) -> Snapshot {
    let key = rebuild_key(hwnd);
    let editors_expanded = super::main_window::open_editors_expanded(hwnd);
    let Some(root) = key.root.clone() else {
        return Snapshot {
            mode: Mode::NoNotebook,
            rows: Vec::new(),
            truncated: false,
            recent: super::library_host::recent_notebooks(hwnd),
            root: None,
            favorite: false,
            root_expanded: true,
            editors_expanded,
            key,
        };
    };
    let favorite = super::library_host::is_favorite(hwnd);
    let built = super::library_host::with_state(hwnd, |state| {
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
    super::inline_name::place(hwnd);
}

/// The tabs changed in some way the Open Editors rows show (a tab opened, closed, switched,
/// renamed, saved, made dirty or clean): the rows follow, and the panel repaints only if they
/// changed. Cheap: the tab list in memory, no rebuild of the tree.
pub(crate) fn editors_changed(hwnd: HWND) {
    let rows = super::open_editors::snapshot(hwnd);
    let changed = with_view(hwnd, |view| {
        let reordered = rows.len() != view.editors.rows.len()
            || rows
                .iter()
                .zip(&view.editors.rows)
                .any(|(new, old)| new.id != old.id);
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
        super::inline_name::place(hwnd);
    }
}

/// The row for the active tab: its note inside the open notebook. `None` for an untitled tab.
fn active_target(hwnd: HWND) -> Option<RowKind> {
    let root = super::library_host::folder(hwnd)?;
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
    super::inline_name::place(hwnd);
}

/// The panel's `WM_PAINT` while the Notebook view shows (`side_panel::paint_view`). The panel
/// has already filled its background.
pub(crate) fn paint(hwnd: HWND, paint: &ViewPaint) {
    with_view(hwnd, |view| view.paint(paint));
}

/// Runs a main-window command as the menus do.
fn run(hwnd: HWND, command: CommandId) {
    unsafe {
        SendMessageW(hwnd, WM_COMMAND, command as usize, 0);
    }
}

/// The selected note's absolute path while the panel has the keyboard focus on the tree, so
/// palette and accelerator commands act on it rather than on the active tab (spec §6.3).
pub(crate) fn focused_note(hwnd: HWND) -> Option<PathBuf> {
    let root = super::library_host::folder(hwnd)?;
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
    let root = super::library_host::folder(hwnd)?;
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
    super::inline_name::place(hwnd);
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
    super::inline_name::place(hwnd);
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
/// tab. "Open in new tab" is `CommandId::Open` and "New note here" is `CommandId::NoteNew` here.
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
                Some(CommandId::NoteRename) => super::inline_name::rename(hwnd, &row.kind),
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
                MenuEntry::command("New note here", CommandId::NoteNew),
                MenuEntry::command("New folder here", CommandId::NoteNewFolder),
                MenuEntry::Separator,
                MenuEntry::command("Rename...\tF2", CommandId::NoteRename),
                MenuEntry::command("Reveal in Explorer", CommandId::NoteRevealInExplorer),
                MenuEntry::Separator,
                MenuEntry::command("Delete...\tDel", CommandId::NoteDelete),
            ];
            match super::menus::track_popup(hwnd, &entries, point) {
                Some(CommandId::NoteNew) => {
                    super::inline_name::new_note(hwnd, Some(relative.clone()));
                }
                Some(CommandId::NoteNewFolder) => {
                    super::inline_name::new_folder(hwnd, Some(relative.clone()));
                }
                Some(CommandId::NoteRename) => super::inline_name::rename(hwnd, &row.kind),
                Some(CommandId::NoteRevealInExplorer) => super::library_host::reveal(hwnd, &path),
                Some(CommandId::NoteDelete) if super::library_host::ready_library(hwnd) => {
                    super::library_host::delete_folder(hwnd, relative);
                }
                _ => {}
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
            // A row's menu is for the tree's row, not a header or a tab row.
            if view.cursor != Cursor::Tree {
                return None;
            }
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
            if !drag_move(hwnd, x, y, wparam) {
                mouse_move(hwnd, x, y);
            }
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
                view.editors.list.hover = None;
                view.editors.hover_close = false;
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
        WM_MBUTTONDOWN => {
            let (x, y) = point_of(lparam);
            let pressed = with_view(hwnd, |view| match view.hit_test(x, y) {
                Hit::Editor { index, .. } => view.editors.rows.get(index).map(|row| row.id),
                _ => None,
            })
            .flatten();
            with_view(hwnd, |view| view.middle_press = pressed);
            Some(0)
        }
        WM_MBUTTONUP => {
            let (x, y) = point_of(lparam);
            let pressed = with_view(hwnd, |view| view.middle_press.take()).flatten();
            let released = with_view(hwnd, |view| match view.hit_test(x, y) {
                Hit::Editor { index, .. } => view.editors.rows.get(index).map(|row| row.id),
                _ => None,
            })
            .flatten();
            if let Some(id) = pressed.filter(|id| Some(*id) == released) {
                super::main_window::close_document_tab(hwnd, id);
            }
            Some(0)
        }
        WM_LBUTTONUP => {
            let (x, y) = point_of(lparam);
            if drag_release(hwnd, x, y) {
                return Some(0);
            }
            // Released after the borrow ends: ReleaseCapture sends WM_CAPTURECHANGED here.
            if with_view(hwnd, |view| view.thumb_grab.take().is_some()).unwrap_or(false) {
                unsafe {
                    ReleaseCapture();
                }
            }
            Some(0)
        }
        WM_CAPTURECHANGED => {
            with_view(hwnd, |view| {
                view.thumb_grab = None;
                // Taken by someone else before our own release arrived: nothing to eat now.
                view.eat_right_up = false;
            });
            // Capture taken away mid-drag (a task switch, a dialog): nothing moves.
            cancel_drag(hwnd);
            Some(0)
        }
        WM_RBUTTONDOWN => {
            // A right press cancels a drag and does nothing else (tree drag spec §3.3). The
            // capture stays until its own release reaches the panel (spec §10).
            if cancel_drag_for_right_press(hwnd) {
                with_view(hwnd, |view| view.eat_right_up = true);
                return Some(0);
            }
            // An earlier cancel's release may never have come here (it went to another window):
            // this press is an ordinary one, so it opens the menu as usual.
            drop_right_release_wait(hwnd);
            // Selects the row; DefWindowProc turns the button-up into WM_CONTEXTMENU.
            let (x, y) = point_of(lparam);
            let hit = hit_after_commit(hwnd, x, y);
            focus_panel_for(hwnd, hit.as_ref());
            if let Some(Hit::Row { index, .. }) = hit {
                with_view(hwnd, |view| {
                    view.cursor = Cursor::Tree;
                    view.select(index);
                });
            }
            Some(0)
        }
        WM_RBUTTONUP => {
            // The release of a right press that cancelled a drag opens no menu; its capture,
            // kept until now, is released here (spec §10).
            drop_right_release_wait(hwnd).then_some(0)
        }
        WM_CONTEXTMENU => {
            context_menu(hwnd, lparam);
            Some(0)
        }
        WM_TIMER if wparam == DRAG_TIMER => {
            drag_tick(hwnd, Instant::now());
            Some(0)
        }
        // A started drag owns the keyboard until it ends: Esc already cancels it, before this
        // (side_panel routes it to cancel_drag first), so nothing here needs to (tree drag spec
        // §3.3).
        WM_KEYDOWN if drag_started(hwnd) => Some(0),
        WM_CHAR if drag_started(hwnd) => Some(0),
        WM_KEYDOWN => key_down(hwnd, wparam as u16).then_some(0),
        WM_CHAR => {
            let ch = char::from_u32(wparam as u32).filter(|ch| !ch.is_control())?;
            typed(hwnd, ch);
            Some(0)
        }
        WM_MOUSEWHEEL => {
            let delta = i32::from((wparam >> 16) as u16 as i16);
            let lines = row_list::wheel_lines();
            // Over the Open Editors rows the section scrolls; anywhere else the tree does.
            let mut pointer = POINT::default();
            unsafe { GetCursorPos(&mut pointer) };
            let scrolled = with_view(hwnd, |view| {
                unsafe { ScreenToClient(view.panel, &mut pointer) };
                let editors = view.layout(view.client(), view.dpi()).editors_list;
                if contains(editors, pointer.x, pointer.y) {
                    if view.editors.list.wheel(delta, lines, height(editors)) {
                        view.invalidate();
                    }
                    return false;
                }
                let height = view.list_height();
                let scrolled = view.list.wheel(delta, lines, height);
                if scrolled {
                    view.invalidate();
                }
                scrolled
            })
            .unwrap_or(false);
            // The field moves with its row (inline naming spec §5.4).
            if scrolled {
                super::inline_name::place(hwnd);
                // A started drag's band and cursor follow the rows a wheel scroll moved under
                // its pointer (tree drag spec §3.2, §3.3).
                retarget_drag(hwnd, Instant::now());
            }
            Some(0)
        }
        _ => None,
    }
}

fn mouse_move(hwnd: HWND, x: i32, y: i32) {
    // Read before the view is borrowed: `ui_fonts` borrows the App itself.
    let fonts = super::main_window::ui_fonts(hwnd);
    let (tools, scrolled) = with_view(hwnd, |view| {
        if let Some(grab) = view.thumb_grab {
            let list = view.list_rect(view.client());
            let scrolled = view.list.drag_thumb(grab, y - list.top, height(list));
            if scrolled {
                view.invalidate();
            }
            return (None, scrolled);
        }
        view.track_leave();
        let hit = view.hit_test(x, y);
        let (row, pin) = match hit {
            Hit::Row { index, part } => (Some(index), part == RowPart::Pin),
            _ => (None, false),
        };
        let (editor, close) = match hit {
            Hit::Editor { index, close } => (Some(index), close),
            _ => (None, false),
        };
        let hot = matches!(
            hit,
            Hit::Header(_) | Hit::StateButton | Hit::SecondButton | Hit::EditorsHeader | Hit::Root
        )
        .then_some(hit);
        let row_changed = view.list.set_hover(row);
        let editor_changed = view.editors.list.set_hover(editor);
        if row_changed
            || editor_changed
            || view.hover_pin != pin
            || view.editors.hover_close != close
            || view.hover != hot
        {
            view.hover_pin = pin;
            view.editors.hover_close = close;
            view.hover = hot;
            view.invalidate();
            return (Some(view.tooltip_tools(fonts)), false);
        }
        (None, false)
    })
    .unwrap_or((None, false));
    if let Some(tools) = tools {
        apply_tooltips(hwnd, &tools);
    }
    // The field moves with its row while the thumb is dragged (inline naming spec §5.4).
    if scrolled {
        super::inline_name::place(hwnd);
    }
}

/// The cursor a drag shows: the arrow over a folder that takes the item, "no" elsewhere
/// (tree drag spec §3.2). Called with nothing of the App borrowed.
fn set_drag_cursor(accepted: bool) {
    let cursor = if accepted { IDC_ARROW } else { IDC_NO };
    unsafe { SetCursor(LoadCursorW(std::ptr::null_mut(), cursor)) };
}

/// Whether a drag is under way (armed does not count): the keyboard is its while it lasts (tree
/// drag spec §3.3).
fn drag_started(hwnd: HWND) -> bool {
    with_view(hwnd, |view| {
        view.drag.as_ref().is_some_and(|drag| drag.started)
    })
    .unwrap_or(false)
}

/// Re-targets a started drag from its last pointer position and updates the cursor to match
/// (tree drag spec §3.2, §3.3): after the rows moved under it without the pointer moving — a
/// rebuild, a mouse-wheel scroll — the band and cursor should still match what is now under the
/// pointer. Does nothing without a started drag.
fn retarget_drag(hwnd: HWND, now: Instant) {
    let Some((pointer, external)) = with_view(hwnd, |view| {
        view.drag
            .as_ref()
            .filter(|drag| drag.started)
            .map(|drag| (drag.pointer, is_external(&drag.source)))
    })
    .flatten() else {
        return;
    };
    let accepted = with_view(hwnd, |view| view.drag_to(pointer.0, pointer.1, now)).unwrap_or(false);
    // OLE owns the cursor during an Explorer drag.
    if !external {
        set_drag_cursor(accepted);
    }
}

/// Whether a drag of `source` came from outside FastPad, through OLE.
fn is_external(source: &DragSource) -> bool {
    matches!(source, DragSource::Files(_))
}

/// Ends a drag's timer, with nothing of the App borrowed. Leaves the capture alone: most cancels
/// release it here too (`end_drag_input`), but a right-press cancel keeps it until its own
/// release reaches the panel (tree drag spec §3.3, §10).
fn end_drag_timer(panel: HWND) {
    unsafe {
        KillTimer(panel, DRAG_TIMER);
    }
}

/// Ends a drag's timer, capture and cursor, with nothing of the App borrowed: ReleaseCapture
/// sends WM_CAPTURECHANGED here.
fn end_drag_input(panel: HWND) {
    end_drag_timer(panel);
    unsafe {
        if GetCapture() == panel {
            ReleaseCapture();
        }
    }
    set_drag_cursor(true);
}

/// Panel point (`x`, `y`) on the screen.
fn screen_point(panel: HWND, x: i32, y: i32) -> POINT {
    let mut point = POINT { x, y };
    unsafe { ClientToScreen(panel, &mut point) };
    point
}

/// Shows the label of the drag that just started, next to panel point (`x`, `y`) (tree drag spec
/// §3.2). Called with nothing of the App borrowed: it makes a window. A label that can't be made
/// leaves the drag without one.
fn show_drag_label(hwnd: HWND, panel: HWND, x: i32, y: i32) {
    let paint = super::side_panel::view_paint(hwnd, panel, std::ptr::null_mut(), RECT::default());
    let Some((image, name)) = with_view(hwnd, |view| view.drag_label_image(&paint)).flatten()
    else {
        return;
    };
    let pointer = screen_point(panel, x, y);
    let Some(label) = DragLabel::show(hwnd, &image, &name, pointer, paint.dpi) else {
        return;
    };
    match with_view(hwnd, |view| view.drag_label.replace(label)) {
        // A label no drag end took (none is known): it must not stay on screen.
        Some(Some(stale)) => stale.destroy(),
        Some(None) => {}
        None => label.destroy(),
    }
}

/// Moves the drag's label, if it has one, next to panel point (`x`, `y`). Called with nothing of
/// the App borrowed.
fn move_drag_label(hwnd: HWND, panel: HWND, x: i32, y: i32) {
    if let Some(label) = with_view(hwnd, |view| view.drag_label).flatten() {
        label.move_to(screen_point(panel, x, y));
    }
}

/// Destroys the drag's label, if it has one: every way a drag ends comes here. Called with
/// nothing of the App borrowed.
fn end_drag_label(hwnd: HWND) {
    if let Some(label) = with_view(hwnd, |view| view.drag_label.take()).flatten() {
        label.destroy();
    }
}

/// A press on a row's body or an Open Editors row arms a drag of `source` (tree drag spec §3.1,
/// open editors spec §4.3), unless an inline edit is still open.
fn arm_drag(hwnd: HWND, source: DragSource, x: i32, y: i32) {
    if super::inline_name::is_open(hwnd) {
        return;
    }
    with_view(hwnd, |view| view.drag = Drag::armed(source, x, y));
}

/// `WM_MOUSEMOVE` with a drag armed or under way (tree drag spec §3.1, §3.2). False leaves the
/// move to the hover code: no drag, or one that has not started.
fn drag_move(hwnd: HWND, x: i32, y: i32, buttons: WPARAM) -> bool {
    let Some((started, origin, panel)) = with_view(hwnd, |view| {
        view.drag
            .as_ref()
            .map(|drag| (drag.started, drag.origin, view.panel))
    })
    .flatten() else {
        return false;
    };
    if buttons & MK_LBUTTON == 0 {
        // The release went elsewhere: a menu, a dialog, another window.
        if started {
            cancel_drag(hwnd);
        } else {
            with_view(hwnd, |view| view.drag = None);
        }
        return started;
    }
    if !started {
        let (cx, cy) = unsafe { (GetSystemMetrics(SM_CXDRAG), GetSystemMetrics(SM_CYDRAG)) };
        if !tree_drag::past_threshold(origin, (x, y), cx, cy) {
            return false;
        }
        with_view(hwnd, |view| {
            if let Some(drag) = view.drag.as_mut() {
                drag.started = true;
            }
            view.list.hover = None;
            view.hover = None;
            view.hover_pin = false;
            view.invalidate();
        });
        unsafe {
            SetCapture(panel);
            SetTimer(panel, DRAG_TIMER, tree_drag::TICK.as_millis() as u32, None);
        }
        show_drag_label(hwnd, panel, x, y);
    } else {
        move_drag_label(hwnd, panel, x, y);
    }
    let accepted = with_view(hwnd, |view| view.drag_to(x, y, Instant::now())).unwrap_or(false);
    set_drag_cursor(accepted);
    true
}

/// `WM_LBUTTONUP`: a drag under way drops where the button went up (tree drag spec §3.4). An
/// armed drag was a click. True when a drag was under way.
fn drag_release(hwnd: HWND, x: i32, y: i32) -> bool {
    let Some((drag, panel)) = with_view(hwnd, |view| {
        if view.drag.as_ref().is_some_and(|drag| drag.started) {
            view.drag_to(x, y, Instant::now());
            view.invalidate();
        }
        (view.drag.take(), view.panel)
    }) else {
        return false;
    };
    let Some(drag) = drag.filter(|drag| drag.started) else {
        return false;
    };
    end_drag_input(panel);
    end_drag_label(hwnd);
    if let Some(folder) = drag.target {
        match &drag.source {
            DragSource::Row(kind) => super::tree_move::drop_into(hwnd, kind, &folder),
            DragSource::Tab { id, path } => {
                super::copy_host::copy_tab_into(hwnd, *id, path, &folder);
            }
            DragSource::Files(_) => {}
        }
    }
    true
}

/// Takes a started drag, invalidating the row it painted over. `None` when there was no drag, or
/// it had not started (an armed one just goes, with nothing left to undo).
fn take_started_drag(hwnd: HWND) -> Option<HWND> {
    let (drag, panel) = with_view(hwnd, |view| {
        let drag = view.drag.take();
        if drag.as_ref().is_some_and(|drag| drag.started) {
            view.invalidate();
        }
        (drag, view.panel)
    })?;
    drag.is_some_and(|drag| drag.started).then_some(panel)
}

/// Ends a drag without moving anything (tree drag spec §3.3): Esc, a lost capture, another view,
/// the sidebar hiding, or the dragged row gone. An armed drag just goes. True when a drag was
/// under way.
pub(crate) fn cancel_drag(hwnd: HWND) -> bool {
    let Some(panel) = take_started_drag(hwnd) else {
        return false;
    };
    end_drag_input(panel);
    end_drag_label(hwnd);
    true
}

/// A right press cancels a drag too, but keeps the capture until its own `WM_RBUTTONUP` reaches
/// the panel (tree drag spec §3.3, spec §10): releasing it immediately would let that release,
/// even over the editor, fall through to `DefWindowProc` there and open its context menu. True
/// when a drag was under way.
fn cancel_drag_for_right_press(hwnd: HWND) -> bool {
    let Some(panel) = take_started_drag(hwnd) else {
        return false;
    };
    end_drag_timer(panel);
    end_drag_label(hwnd);
    set_drag_cursor(true);
    true
}

/// Ends a right press's wait for its own release (`cancel_drag_for_right_press`) and releases
/// the capture kept for it, if it still has it. Every place that drops the wait comes here: a
/// wait dropped without releasing would leave the panel with the mouse until another window
/// took it. No drag or thumb grab owns the capture while the wait lasts: a left press, which
/// starts either, ends the wait first. True when there was a wait. Called with nothing of the
/// App borrowed: ReleaseCapture sends WM_CAPTURECHANGED here.
pub(crate) fn drop_right_release_wait(hwnd: HWND) -> bool {
    let Some((true, panel)) = with_view(hwnd, |view| {
        (std::mem::take(&mut view.eat_right_up), view.panel)
    }) else {
        return false;
    };
    unsafe {
        if GetCapture() == panel {
            ReleaseCapture();
        }
    }
    true
}

/// The drag timer (tree drag spec §3.3): near the list's top or bottom edge the list scrolls,
/// and a collapsed folder the pointer has rested on long enough expands. `now` comes in so the
/// tests need not wait.
pub(crate) fn drag_tick(hwnd: HWND, now: Instant) {
    let Some((scrolled, expand, pointer, external)) = with_view(hwnd, |view| {
        let drag = view.drag.as_ref().filter(|drag| drag.started)?;
        let pointer = drag.pointer;
        let external = is_external(&drag.source);
        let expand = tree_drag::expand_due(drag.resting.as_ref(), now);
        let list = view.list_rect(view.client());
        let lines = tree_drag::scroll_step(pointer.1, list.top, list.bottom, view.list.row_height);
        let scrolled = lines != 0 && view.list.scroll_lines(lines, height(list));
        if scrolled {
            view.invalidate();
        }
        Some((scrolled, expand, pointer, external))
    })
    .flatten() else {
        return;
    };
    if let Some(folder) = &expand {
        with_view(hwnd, |view| {
            if let Some(drag) = view.drag.as_mut() {
                drag.resting = None;
            }
        });
        set_folder_expanded(hwnd, folder, true);
    }
    if scrolled || expand.is_some() {
        let accepted =
            with_view(hwnd, |view| view.drag_to(pointer.0, pointer.1, now)).unwrap_or(false);
        if !external {
            set_drag_cursor(accepted);
        }
    }
}

/// What an Explorer drag at panel point `x`, `y` does (open editors spec §4.1, §4.3): over Open
/// Editors, or with no notebook, it opens (COPY, no highlight); over the tree, the root row or
/// the body, it copies into the folder under it when that folder takes one of `paths` (the
/// band shows); elsewhere nothing. The drag is kept as a started `DragSource::Files` drag with
/// no capture and no label, so the tree drag's band, auto-expand and auto-scroll apply.
pub(crate) fn external_over(hwnd: HWND, x: i32, y: i32, paths: &[PathBuf]) -> bool {
    let opens = with_view(hwnd, |view| view.opens_at(x, y)).unwrap_or(false);
    if opens {
        external_leave(hwnd);
        return true;
    }
    let started = with_view(hwnd, |view| {
        if !view
            .drag
            .as_ref()
            .is_some_and(|drag| is_external(&drag.source))
        {
            view.drag = Drag::armed(DragSource::Files(paths.to_vec()), x, y).map(|mut drag| {
                drag.started = true;
                drag
            });
            return true;
        }
        false
    })
    .unwrap_or(false);
    if started && let Some(panel) = with_view(hwnd, |view| view.panel) {
        unsafe { SetTimer(panel, DRAG_TIMER, tree_drag::TICK.as_millis() as u32, None) };
    }
    with_view(hwnd, |view| view.drag_to(x, y, Instant::now())).unwrap_or(false)
}

/// The Explorer drag left the panel or was cancelled: its band and timer go.
pub(crate) fn external_leave(hwnd: HWND) {
    let panel = with_view(hwnd, |view| {
        let external = view
            .drag
            .as_ref()
            .is_some_and(|drag| is_external(&drag.source));
        if external {
            view.drag = None;
            view.invalidate();
        }
        external.then_some(view.panel)
    })
    .flatten();
    if let Some(panel) = panel {
        end_drag_timer(panel);
    }
}

/// An Explorer drop at panel point `x`, `y`: posts what to do and returns at once, so Explorer
/// never waits on a prompt (spec §6). False when nothing here takes it.
pub(crate) fn external_drop(hwnd: HWND, x: i32, y: i32, paths: Vec<PathBuf>) -> bool {
    let opens = with_view(hwnd, |view| view.opens_at(x, y)).unwrap_or(false);
    let folder = if opens {
        None
    } else {
        with_view(hwnd, |view| view.drag_to(x, y, Instant::now()))
            .filter(|&accepted| accepted)
            .and_then(|_| {
                with_view(hwnd, |view| {
                    view.drag.as_ref().and_then(|drag| drag.target.clone())
                })
                .flatten()
            })
    };
    external_leave(hwnd);
    if !opens && folder.is_none() {
        return false;
    }
    super::copy_host::post_panel_drop(hwnd, paths, folder)
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

/// What a press at `x`, `y` hits. An inline edit open when the press comes ends first, as if
/// focus had left it (inline naming spec §5.3), and a row hit follows the row it landed on to
/// wherever the commit moved it. `None` when there is no view, or that row went with the commit
/// (the draft row, or the row just renamed). A press on the scroll thumb leaves the edit open.
fn hit_after_commit(hwnd: HWND, x: i32, y: i32) -> Option<Hit> {
    let hit = with_view(hwnd, |view| view.hit_test(x, y))?;
    // The scroll thumb only scrolls, and the field scrolls with its row (spec §5.4).
    if !super::inline_name::is_open(hwnd) || matches!(hit, Hit::Thumb(_)) {
        return Some(hit);
    }
    let clicked = match hit {
        Hit::Row { index, .. } => with_view(hwnd, |view| {
            view.rows.get(index).map(|row| row.kind.clone())
        })
        .flatten(),
        _ => None,
    };
    super::inline_name::commit(hwnd, super::inline_name::How::FocusLeft);
    match (hit, clicked) {
        (Hit::Row { part, .. }, Some(kind)) => {
            let index = with_view(hwnd, |view| tree::row_index(&view.rows, &kind)).flatten()?;
            Some(Hit::Row { index, part })
        }
        (Hit::Row { .. }, None) => None,
        (hit, _) => Some(hit),
    }
}

/// Moves the keyboard focus to the panel for a press that `hit`, unless it is on the scroll
/// thumb while an inline edit is open: the field keeps the focus and goes on editing while the
/// drag scrolls (inline naming spec §5.4).
fn focus_panel_for(hwnd: HWND, hit: Option<&Hit>) {
    if !(matches!(hit, Some(Hit::Thumb(_))) && super::inline_name::is_open(hwnd)) {
        focus_panel(hwnd);
    }
}

fn left_down(hwnd: HWND, x: i32, y: i32) {
    // A drag armed by an earlier press whose release never came here. `cancel_drag` also ends
    // one that had started, label and all, though its capture should have ended it already.
    cancel_drag(hwnd);
    // A right press's cancel whose own release has not come yet: this press ends the wait, and
    // the capture kept for it (the release then opens the menu as any other would).
    drop_right_release_wait(hwnd);
    let hit = hit_after_commit(hwnd, x, y);
    focus_panel_for(hwnd, hit.as_ref());
    let Some(hit) = hit else {
        return;
    };
    // The keyboard selection follows the click (open editors spec §3.5).
    let cursor = match hit {
        Hit::EditorsHeader => Some(Cursor::EditorsHeader),
        Hit::Editor { index, .. } => Some(Cursor::Editor(index)),
        Hit::Root => Some(Cursor::Root),
        Hit::Row { .. } => Some(Cursor::Tree),
        // A root row button acts as the header's did (open editors spec §3.3): New note and New
        // folder still go to the tree's selected folder.
        Hit::Header(_) | Hit::StateButton | Hit::SecondButton | Hit::Thumb(_) | Hit::Empty => None,
    };
    if let Some(cursor) = cursor {
        with_view(hwnd, |view| {
            view.cursor = cursor;
            view.invalidate();
        });
    }
    match hit {
        Hit::EditorsHeader => {
            let expanded = with_view(hwnd, |view| view.editors_expanded).unwrap_or(true);
            super::main_window::set_open_editors_expanded(hwnd, !expanded);
            rebuild(hwnd);
        }
        Hit::Editor { index, close } => {
            let Some(row) = with_view(hwnd, |view| view.editors.rows.get(index).cloned()).flatten()
            else {
                return;
            };
            if close {
                super::main_window::close_document_tab(hwnd, row.id);
            } else {
                super::main_window::activate_document_by_id(hwnd, row.id);
                // The path is taken now, so the drag outlives its tab closing (open editors spec
                // §4.3). An untitled tab has no file to copy: no drag.
                if let Some(path) = row.path {
                    arm_drag(hwnd, DragSource::Tab { id: row.id, path }, x, y);
                }
            }
        }
        Hit::Root => {
            let expanded = super::library_host::root_expanded(hwnd);
            super::library_host::set_root_expanded(hwnd, !expanded);
            rebuild(hwnd);
        }
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
            // Read before the click acts: opening a note or toggling a folder can move rows.
            let source = (part == RowPart::Body)
                .then(|| {
                    with_view(hwnd, |view| {
                        view.rows.get(index).map(|row| row.kind.clone())
                    })
                })
                .flatten()
                .flatten();
            row_clicked(hwnd, index, part, false);
            if let Some(source) = source {
                arm_drag(hwnd, DragSource::Row(source), x, y);
            }
        }
        Hit::Empty => {}
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

/// Opens or toggles row `index` (spec §6.4). A folder toggles, a note opens, and a recent
/// notebook opens.
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
            RowKind::Draft => {}
        },
        Target::Truncated | Target::Nothing => {}
    }
}

/// The header's buttons (spec §6.5).
pub(crate) fn header_clicked(hwnd: HWND, button: HeaderButton) {
    match button {
        HeaderButton::Favorite => super::library_host::toggle_notebook_favorite(hwnd),
        HeaderButton::NewNote => run(hwnd, CommandId::NoteNew),
        HeaderButton::NewFolder => run(hwnd, CommandId::NoteNewFolder),
        HeaderButton::More => more_menu(hwnd),
    }
}

/// "…": the notebook's own actions.
fn more_menu(hwnd: HWND) {
    let Some(at) = with_view(hwnd, |view| {
        let dpi = view.dpi();
        let root = view.layout(view.client(), dpi).root;
        let rect = notebook_layout::root_parts(root, dpi).buttons[3].1;
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

/// `pub(crate)` for the tests that click the state button directly.
pub(crate) fn state_button(hwnd: HWND) {
    match with_view(hwnd, |view| view.mode) {
        Some(Mode::NoNotebook) => super::library_host::choose_and_open_folder(hwnd),
        // The empty notebook's own "New note" puts a draft row in the tree, not an untitled tab
        // (inline naming spec §3.1): there is no tree yet, but `apply` makes one for a draft.
        Some(Mode::Empty) => run(hwnd, CommandId::NoteNew),
        Some(Mode::Failed) => super::library_host::retry_load(hwnd),
        _ => {}
    }
}

/// The panel's keys (spec §10, open editors spec §3.5): the arrows run one selection through
/// the header rows, the Open Editors rows and the tree. Returns false for keys it leaves to the
/// panel.
pub(crate) fn key_down(hwnd: HWND, key: u16) -> bool {
    if let Some(list_key) = ListKey::from_virtual_key(u32::from(key)) {
        with_view(hwnd, |view| {
            let shape = view.shape();
            // A list with nothing selected yet moves as one list does: the first Up or Down
            // selects a row in view.
            let own_list = view.cursor == Cursor::Tree
                && shape.tree > 0
                && view.list.selected.is_none()
                && matches!(list_key, ListKey::Up | ListKey::Down);
            if own_list {
                let height = view.list_height();
                view.list.move_selection(list_key, height);
                view.invalidate();
                return;
            }
            match panel_cursor::step(view.cursor, view.list.selected, list_key, shape) {
                Some((cursor, tree)) => {
                    view.cursor = cursor;
                    if let Some(index) = tree {
                        view.select(index);
                    }
                    if let Cursor::Editor(index) = cursor {
                        let height = height(view.layout(view.client(), view.dpi()).editors_list);
                        view.editors.list.ensure_visible(index, height);
                    }
                }
                None if view.cursor == Cursor::Tree => {
                    let height = view.list_height();
                    view.list.move_selection(list_key, height);
                }
                // Page Up and Page Down move within the Open Editors rows (open editors spec
                // §3.5). The list's own selection is the active tab's row, so it is put back.
                None => {
                    if let Cursor::Editor(index) = view.cursor {
                        let height = height(view.layout(view.client(), view.dpi()).editors_list);
                        let list = &mut view.editors.list;
                        let active = list.selected.replace(index);
                        list.move_selection(list_key, height);
                        let moved = list.selected.unwrap_or(index);
                        list.selected = active;
                        view.cursor = Cursor::Editor(moved);
                    }
                }
            }
            view.invalidate();
        });
        return true;
    }
    let (cursor, hidden) =
        with_view(hwnd, |view| (view.cursor, view.tree_hidden())).unwrap_or((Cursor::Tree, false));
    if cursor != Cursor::Tree {
        return section_key(hwnd, cursor, key);
    }
    // A row the collapsed root hides is not acted on.
    let selected = with_view(hwnd, |view| view.list.selected)
        .flatten()
        .filter(|_| !hidden);
    let Some(selected) = selected else {
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
                Some(kind @ (RowKind::Note(_) | RowKind::Folder(_))) if key == VK_F2 => {
                    super::inline_name::rename(hwnd, &kind);
                }
                Some(RowKind::Note(relative)) if super::library_host::ready_library(hwnd) => {
                    super::library_host::delete_file(hwnd, &root.join(relative));
                }
                Some(RowKind::Folder(relative)) if super::library_host::ready_library(hwnd) => {
                    super::library_host::delete_folder(hwnd, &relative);
                }
                _ => {}
            }
            true
        }
        _ => false,
    }
}

/// A key on a header row or an Open Editors row. F2 and Del do nothing there: they act on tree
/// rows only.
fn section_key(hwnd: HWND, cursor: Cursor, key: u16) -> bool {
    match (cursor, key) {
        (Cursor::Editor(index), VK_RETURN) => {
            let Some(id) =
                with_view(hwnd, |view| view.editors.rows.get(index).map(|row| row.id)).flatten()
            else {
                return true;
            };
            super::main_window::activate_document_by_id(hwnd, id);
            super::main_window::focus_content(hwnd);
        }
        (Cursor::EditorsHeader, VK_RETURN | VK_LEFT | VK_RIGHT) => {
            let expanded = super::main_window::open_editors_expanded(hwnd);
            let wanted = expanded_after(key, expanded);
            if wanted != expanded {
                super::main_window::set_open_editors_expanded(hwnd, wanted);
                rebuild(hwnd);
            }
        }
        (Cursor::Root, VK_RETURN | VK_LEFT | VK_RIGHT) => {
            if super::library_host::folder(hwnd).is_none() {
                return true;
            }
            let expanded = super::library_host::root_expanded(hwnd);
            let wanted = expanded_after(key, expanded);
            if wanted != expanded {
                super::library_host::set_root_expanded(hwnd, wanted);
                rebuild(hwnd);
            }
        }
        (_, VK_RETURN | VK_LEFT | VK_RIGHT | VK_F2 | VK_DELETE) => {}
        _ => return false,
    }
    true
}

/// Whether a header row is expanded after `key`: Left collapses it, Right expands it, Enter
/// toggles it.
fn expanded_after(key: u16, expanded: bool) -> bool {
    match key {
        VK_LEFT => false,
        VK_RIGHT => true,
        _ => !expanded,
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
        match tree::parent_index(&view.rows, index) {
            Some(parent) => view.select(parent),
            // A top-level row: the notebook's root row is its parent (open editors spec §3.5).
            None if view.mode == Mode::Tree => {
                view.cursor = Cursor::Root;
                view.invalidate();
            }
            None => {}
        }
    });
}

/// Type-ahead: the next row whose name starts with what was typed in the last second. A single
/// letter searches from the row after the selection, so repeating it steps through matches.
fn typed(hwnd: HWND, ch: char) {
    with_view(hwnd, |view| {
        if !view.tree_shown() {
            return;
        }
        let prefix = view.typed.push(ch, Instant::now()).to_owned();
        let from = match view.list.selected {
            Some(selected) if prefix.chars().count() == 1 => selected + 1,
            Some(selected) => selected,
            None => 0,
        };
        if let Some(index) = tree::type_ahead(&view.rows, from, &prefix) {
            view.cursor = Cursor::Tree;
            view.select(index);
        }
    });
}

impl crate::window::sidebar_accessibility::AccessibleView for NotebookView {
    /// Push buttons first, then the Open Editors header and its rows, the notebook's root row,
    /// then the RECENT notebooks (no-notebook state) or the tree rows, the draft row left out.
    fn accessible_count(&self, client: RECT, dpi: u32) -> usize {
        let (header, editors, root) = self.accessible_parts();
        self.buttons(client, dpi).len()
            + header
            + editors
            + root
            + self.recent.len()
            + self.accessible_tree_rows()
    }

    fn accessible_item(
        &self,
        index: usize,
        client: RECT,
        dpi: u32,
        focused: bool,
    ) -> Option<crate::window::sidebar_accessibility::AccessibleItem> {
        use crate::window::sidebar_accessibility::{
            button_item, editor_item, list_item, row_rect, section_item, tree_item,
        };
        let buttons = self.buttons(client, dpi);
        if let Some((name, rect)) = buttons.get(index) {
            return Some(button_item(name, false, false, *rect));
        }
        let layout = self.layout(client, dpi);
        let (header, editors, root) = self.accessible_parts();
        let index = index - buttons.len();
        if index < header {
            return Some(section_item(
                &format!("Open editors, {}", self.editors.rows.len()),
                self.editors_expanded,
                self.cursor == Cursor::EditorsHeader,
                focused,
                layout.editors_header,
            ));
        }
        let index = index - header;
        if index < editors {
            let row = self.editors.rows.get(index)?;
            let (rect, visible) = row_rect(layout.editors_list, &self.editors.list, index);
            return Some(editor_item(
                &super::open_editors::accessible_name(row),
                self.cursor == Cursor::Editor(index),
                focused,
                rect,
                visible,
            ));
        }
        let index = index - editors;
        if index < root {
            return Some(section_item(
                &self.name,
                self.root_expanded,
                self.cursor == Cursor::Root,
                focused,
                layout.root,
            ));
        }
        let index = index - root;
        // The list's selection has the focus only while the keyboard selection is in it.
        let focused = focused && self.cursor == Cursor::Tree;
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
        if index >= self.accessible_tree_rows() {
            return None;
        }
        let index = self.row_of_accessible(index);
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
        let inside = |rect: &RECT| contains(*rect, point.x, point.y);
        let buttons = self.buttons(client, dpi);
        if let Some(index) = buttons.iter().position(|(_, rect)| inside(rect)) {
            return Some(index);
        }
        let layout = self.layout(client, dpi);
        let (header, editors, root) = self.accessible_parts();
        let start = buttons.len();
        if inside(&layout.editors_header) {
            return Some(start);
        }
        if inside(&layout.editors_list) {
            let row = self
                .editors
                .list
                .row_at(point.y - layout.editors_list.top)?;
            return (row < editors).then_some(start + header + row);
        }
        if inside(&layout.root) {
            return (root == 1).then_some(start + header + editors);
        }
        let area = self.list_area(client, dpi);
        if !inside(&area) {
            return None;
        }
        let row = self.list().row_at(point.y - area.top)?;
        let rows = start + header + editors + root;
        if self.mode == Mode::NoNotebook {
            (row < self.recent.len()).then_some(rows + row)
        } else {
            (self.tree_shown() && row < self.rows().len())
                .then(|| self.accessible_of_row(row))
                .flatten()
                .map(|row| rows + self.recent.len() + row)
        }
    }

    fn accessible_current(&self, client: RECT, dpi: u32) -> Option<usize> {
        let start = self.buttons(client, dpi).len();
        let (header, editors, root) = self.accessible_parts();
        match self.cursor {
            Cursor::EditorsHeader => return Some(start),
            Cursor::Editor(index) => return (index < editors).then_some(start + header + index),
            Cursor::Root => return (root == 1).then_some(start + header + editors),
            Cursor::Tree => {}
        }
        let rows = start + header + editors + root;
        let selected = self.list().selected?;
        // In the no-notebook state the list's selection is a RECENT row, not a tree row.
        if self.mode == Mode::NoNotebook {
            (selected < self.recent.len()).then_some(rows + selected)
        } else {
            (self.tree_shown() && selected < self.rows().len())
                .then(|| self.accessible_of_row(selected))
                .flatten()
                .map(|row| rows + self.recent.len() + row)
        }
    }

    fn accessible_select(&mut self, index: usize, client: RECT, dpi: u32) {
        let (header, editors, root) = self.accessible_parts();
        let Some(index) = index.checked_sub(self.buttons(client, dpi).len()) else {
            return;
        };
        if index < header {
            self.cursor = Cursor::EditorsHeader;
            return;
        }
        let index = index - header;
        if index < editors {
            self.cursor = Cursor::Editor(index);
            let list = self.layout(client, dpi).editors_list;
            self.editors.list.ensure_visible(index, height(list));
            return;
        }
        let index = index - editors;
        if index < root {
            self.cursor = Cursor::Root;
            return;
        }
        let index = index - root;
        let (offset, rows) = if self.mode == Mode::NoNotebook {
            (0, self.recent.len())
        } else {
            (self.recent.len(), self.accessible_tree_rows())
        };
        let Some(row) = index.checked_sub(offset).filter(|&row| row < rows) else {
            return;
        };
        let row = if self.mode == Mode::NoNotebook {
            row
        } else {
            self.row_of_accessible(row)
        };
        self.cursor = Cursor::Tree;
        let area = self.list_area(client, dpi);
        self.list_mut().select(row, area.bottom - area.top);
    }

    fn accessible_identity(&self, index: usize, client: RECT, dpi: u32) -> Option<u64> {
        use crate::window::sidebar_accessibility::identity_of;
        let (header, editors, root) = self.accessible_parts();
        let index = index.checked_sub(self.buttons(client, dpi).len())?;
        if index < header {
            return Some(identity_of(&"open-editors"));
        }
        let index = index - header;
        if index < editors {
            let row = self.editors.rows.get(index)?;
            return Some(identity_of(&("editor", row.id.0)));
        }
        let index = index - editors;
        if index < root {
            return Some(identity_of(&"notebook-root"));
        }
        let index = index - root;
        if let Some(folder) = self.recent.get(index) {
            return Some(identity_of(folder));
        }
        let index = index - self.recent.len();
        if index >= self.accessible_tree_rows() {
            return None;
        }
        let row = self.rows().get(self.row_of_accessible(index))?;
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
        let rows = flatten(&tree, &expanded);
        let elapsed = started.elapsed();
        assert_eq!(rows.len(), 11_000);
        assert!(rows[0].expanded);
        assert_eq!(flatten(&tree, &[]).len(), 1_000);
        if !cfg!(debug_assertions) {
            assert!(elapsed < Duration::from_millis(16), "{elapsed:?}");
        }
    }

    #[test]
    fn a_tree_row_draws_the_chosen_sets_icon_and_minimal_in_high_contrast() {
        // Break caught: the closed folder icon on an expanded folder, Solid drawn as Minimal, or
        // a Material bitmap in high contrast (icon sets spec §3.2, §6). The `assert_ne!`s below
        // catch one set chosen and another drawn; `blue > red + 60` only checks that the
        // Markdown bitmap's own blue (#42a5f5) is what got drawn into the icon box.
        use crate::window::icon_sets::images::TestTarget;
        use crate::window::icon_sets::material::MaterialIcon;
        use crate::window::titlebar::create_ui_font;
        use windows_sys::Win32::Graphics::Gdi::{DeleteObject, FW_NORMAL, FW_SEMIBOLD};
        let normal = FW_NORMAL as i32;
        let fonts = UiFonts {
            text: create_ui_font(12, "Segoe UI", normal, false),
            bold: create_ui_font(11, "Segoe UI", FW_SEMIBOLD as i32, false),
            italic: create_ui_font(12, "Segoe UI", normal, true),
            glyph: create_ui_font(12, "Segoe MDL2 Assets", normal, false),
            ..UiFonts::default()
        };
        let rect = RECT {
            left: 0,
            top: 0,
            right: 200,
            bottom: 22,
        };
        let icon_box = row_parts(rect, 0, 96).icon;
        let px = (icon_box.right - icon_box.left) as u32;
        let look = RowLook {
            selected: false,
            hover: false,
            focused: false,
        };
        let note = row(RowKind::Note("a.md".into()), 0);
        let open_folder = TreeRow {
            expanded: true,
            ..row(RowKind::Folder("f".into()), 0)
        };
        let target = TestTarget::new(200, 22);
        let mut images = IconImages::new();
        // The icon box's pixels after drawing `row` in `set` under `palette`.
        let mut draw = |row: &TreeRow, set: FileIconSet, palette: &Palette| {
            unsafe { fill(target.dc, rect, palette.editor_background) };
            draw_tree_row(
                target.dc,
                Some(row),
                rect,
                look,
                palette,
                &FileIcons::neutral(),
                fonts,
                96,
                false,
                None,
                &mut images,
                set,
                true,
                false,
            );
            target.area(icon_box)
        };
        // The icon box's pixels after blending `icon` straight into a fresh target.
        let direct = |icon: MaterialIcon, palette: &Palette| {
            let target = TestTarget::new(200, 22);
            unsafe { fill(target.dc, rect, palette.editor_background) };
            assert!(IconImages::new().draw(target.dc, icon, icon_box, px));
            target.area(icon_box)
        };
        let palette = Palette::neutral();
        let material = draw(&note, FileIconSet::Material, &palette);
        assert!(
            material.iter().any(|&pixel| {
                let (red, blue) = ((pixel >> 16) & 0xFF, pixel & 0xFF);
                blue > red + 60
            }),
            "the Markdown bitmap (#42a5f5) is drawn"
        );
        let folder = draw(&open_folder, FileIconSet::Material, &palette);
        assert_eq!(folder, direct(MaterialIcon::FolderOpen, &palette));
        assert_ne!(folder, direct(MaterialIcon::Folder, &palette));
        let minimal = draw(&note, FileIconSet::Minimal, &palette);
        assert_ne!(
            minimal, material,
            "Minimal draws its outline, not the bitmap"
        );
        assert!(
            minimal.iter().any(|&pixel| pixel != minimal[0]),
            "Minimal draws"
        );
        let solid = draw(&note, FileIconSet::Solid, &palette);
        assert_ne!(
            solid, minimal,
            "Solid draws its filled shape, not the outline"
        );
        assert_ne!(solid, material);
        let contrast = Palette {
            high_contrast: true,
            ..palette
        };
        for row in [&note, &open_folder] {
            let minimal = draw(row, FileIconSet::Minimal, &contrast);
            for set in [FileIconSet::Material, FileIconSet::Solid] {
                assert_eq!(
                    draw(row, set, &contrast),
                    minimal,
                    "high contrast draws Minimal in every set"
                );
            }
        }
        for font in [fonts.text, fonts.bold, fonts.italic, fonts.glyph] {
            unsafe { DeleteObject(font) };
        }
    }

    #[test]
    fn a_clipped_icon_box_draws_part_of_the_icon_not_a_shrunken_one() {
        // Break caught: a deep row in a narrow sidebar clamps `parts.icon` below the icon box,
        // and an icon resampled down to that clipped width and cached per clipped size, or one
        // painted past the box over the name.
        use crate::window::icon_sets::images::TestTarget;
        use crate::window::titlebar::create_ui_font;
        use windows_sys::Win32::Graphics::Gdi::{DeleteObject, FW_NORMAL, FW_SEMIBOLD};
        let normal = FW_NORMAL as i32;
        let fonts = UiFonts {
            text: create_ui_font(12, "Segoe UI", normal, false),
            bold: create_ui_font(11, "Segoe UI", FW_SEMIBOLD as i32, false),
            italic: create_ui_font(12, "Segoe UI", normal, true),
            glyph: create_ui_font(12, "Segoe MDL2 Assets", normal, false),
            ..UiFonts::default()
        };
        // At depth 0 and 96 DPI: chevron sits at [8, 24), leaving only 6 px for the icon box
        // inside a 30 px wide row, well under the 16 px glyph box.
        let rect = RECT {
            left: 0,
            top: 0,
            right: 30,
            bottom: 22,
        };
        let icon_box = row_parts(rect, 0, 96).icon;
        assert!(
            icon_box.right - icon_box.left < GLYPH_BOX,
            "the row must actually clip the icon box for this test to mean anything"
        );
        let look = RowLook {
            selected: false,
            hover: false,
            focused: false,
        };
        let note = row(RowKind::Note("a.md".into()), 0);
        let target = TestTarget::new(30, 22);
        let mut images = IconImages::new();
        let palette = Palette::neutral();
        let mut draw = |set: FileIconSet| {
            unsafe { fill(target.dc, rect, palette.editor_background) };
            draw_tree_row(
                target.dc,
                Some(&note),
                rect,
                look,
                &palette,
                &FileIcons::neutral(),
                fonts,
                96,
                false,
                None,
                &mut images,
                set,
                true,
                false,
            );
            target.area(icon_box)
        };
        for set in [
            FileIconSet::Material,
            FileIconSet::Minimal,
            FileIconSet::Solid,
        ] {
            let drawn = draw(set);
            assert!(
                drawn.iter().any(|&pixel| pixel != drawn[0]),
                "{set:?} draws part of its icon in the clipped box"
            );
        }
        let sizes = images.cached_pixel_sizes();
        assert!(
            !sizes.is_empty() && sizes.iter().all(|&px| px == GLYPH_BOX as u32),
            "a clipped row must never resample and cache a bitmap at the clipped width: {sizes:?}"
        );
        for font in [fonts.text, fonts.bold, fonts.italic, fonts.glyph] {
            unsafe { DeleteObject(font) };
        }
    }

    #[test]
    fn the_drop_band_covers_the_folders_rows_in_view_or_the_whole_list() {
        use crate::window::tree_drag::Highlight;
        let list_rect = RECT {
            left: 0,
            top: 100,
            right: 200,
            bottom: 230,
        };
        let mut state = RowListState::new(26);
        state.set_count(20);
        state.top = 3;
        let band = |highlight| band_rect(list_rect, &state, highlight);
        assert_eq!(band(Highlight::Root).map(edges), Some((0, 100, 200, 230)));
        assert_eq!(
            band(Highlight::Rows { start: 4, end: 6 }).map(edges),
            Some((0, 126, 200, 178))
        );
        assert_eq!(
            band(Highlight::Rows { start: 0, end: 5 }).map(edges),
            Some((0, 100, 200, 152)),
            "clipped at the top of the view"
        );
        assert_eq!(
            band(Highlight::Rows { start: 6, end: 20 }).map(edges),
            Some((0, 178, 200, 230)),
            "clipped at the bottom of the list"
        );
        assert!(
            band(Highlight::Rows { start: 0, end: 2 }).is_none(),
            "above the view"
        );
    }

    #[test]
    fn the_drop_band_fills_outside_high_contrast_and_outlines_in_it() {
        use crate::window::icon_sets::images::TestTarget;
        let target = TestTarget::new(60, 40);
        let whole = RECT {
            left: 0,
            top: 0,
            right: 60,
            bottom: 40,
        };
        let band = RECT {
            left: 10,
            top: 10,
            right: 50,
            bottom: 30,
        };
        let inside = RECT {
            left: 20,
            top: 15,
            right: 21,
            bottom: 16,
        };
        let edge = RECT {
            left: 10,
            top: 20,
            right: 11,
            bottom: 21,
        };
        let reference = |color: u32| {
            let target = TestTarget::new(1, 1);
            unsafe {
                fill(
                    target.dc,
                    RECT {
                        left: 0,
                        top: 0,
                        right: 1,
                        bottom: 1,
                    },
                    color,
                )
            };
            target.area(RECT {
                left: 0,
                top: 0,
                right: 1,
                bottom: 1,
            })[0]
        };
        let palette = Palette::neutral();
        unsafe { fill(target.dc, whole, palette.editor_background) };
        paint_band(target.dc, band, &palette, 96, true);
        paint_band(target.dc, band, &palette, 96, false);
        assert_eq!(
            target.area(inside)[0],
            reference(palette.inactive_selection_background)
        );

        let contrast = Palette {
            high_contrast: true,
            ..palette
        };
        unsafe { fill(target.dc, whole, contrast.editor_background) };
        paint_band(target.dc, band, &contrast, 96, true);
        assert_eq!(
            target.area(inside)[0],
            reference(contrast.editor_background),
            "no blend"
        );
        paint_band(target.dc, band, &contrast, 96, false);
        assert_eq!(
            target.area(edge)[0],
            reference(contrast.selection_background)
        );
        assert_eq!(
            target.area(inside)[0],
            reference(contrast.editor_background)
        );
    }

    #[test]
    fn the_dragged_row_draws_its_name_dimmed() {
        use crate::window::icon_sets::images::TestTarget;
        use crate::window::titlebar::create_ui_font;
        use windows_sys::Win32::Graphics::Gdi::{DeleteObject, FW_NORMAL};
        let fonts = UiFonts {
            text: create_ui_font(12, "Segoe UI", FW_NORMAL as i32, false),
            ..UiFonts::default()
        };
        let rect = RECT {
            left: 0,
            top: 0,
            right: 200,
            bottom: 22,
        };
        let name = row_parts(rect, 0, 96).name;
        let look = RowLook {
            selected: false,
            hover: false,
            focused: false,
        };
        let palette = Palette::neutral();
        let note = TreeRow {
            name: "dragged".into(),
            ..row(RowKind::Note("a.md".into()), 0)
        };
        let mut images = IconImages::new();
        let mut draw = |dimmed: bool| {
            let target = TestTarget::new(200, 22);
            unsafe { fill(target.dc, rect, palette.editor_background) };
            draw_tree_row(
                target.dc,
                Some(&note),
                rect,
                look,
                &palette,
                &FileIcons::neutral(),
                fonts,
                96,
                false,
                None,
                &mut images,
                FileIconSet::Minimal,
                true,
                dimmed,
            );
            target.area(name)
        };
        assert_ne!(draw(true), draw(false));
        unsafe { DeleteObject(fonts.text) };
    }

    #[test]
    fn the_drag_label_has_a_border_a_fill_an_icon_and_its_name() {
        // Break caught: a label painted in colours outside the system pairs in high contrast,
        // without its border, or with no name or icon (tree drag spec §3.2).
        use crate::window::icon_sets::images::TestTarget;
        use crate::window::titlebar::create_ui_font;
        use windows_sys::Win32::Graphics::Gdi::{DeleteObject, FW_NORMAL};
        let fonts = UiFonts {
            text: create_ui_font(12, "Segoe UI", FW_NORMAL as i32, false),
            ..UiFonts::default()
        };
        let size = drag_label_size(80, 96);
        assert_eq!((size.cx, size.cy), (8 + 16 + 6 + 80 + 8, 24));
        let reference = |color: u32| {
            let target = TestTarget::new(1, 1);
            let pixel = RECT {
                left: 0,
                top: 0,
                right: 1,
                bottom: 1,
            };
            unsafe { fill(target.dc, pixel, color) };
            target.pixel(0, 0)
        };
        let mut images = IconImages::new();
        let mut check = |palette: Palette| {
            let (background, border, _) = drag_label_colors(&palette);
            let target = TestTarget::new(size.cx, size.cy);
            let paint = ViewPaint {
                hdc: target.dc,
                client: RECT::default(),
                palette,
                icons: FileIcons::neutral(),
                icon_set: FileIconSet::Minimal,
                light_theme: true,
                background: palette.panel_background(),
                fonts,
                dpi: 96,
                focused: false,
            };
            paint_drag_label(
                target.dc,
                size,
                TreeItem::Note(note_kind(Some("md"))),
                "notes.md",
                &paint,
                &mut images,
            );
            assert_eq!(target.pixel(0, 12), reference(border), "the left border");
            assert_eq!(target.pixel(size.cx - 1, 0), reference(border), "a corner");
            assert_eq!(
                target.pixel(size.cx - 3, 3),
                reference(background),
                "the fill past the name"
            );
            let drawn = |left: i32, right: i32| {
                target
                    .area(RECT {
                        left,
                        top: 2,
                        right,
                        bottom: size.cy - 2,
                    })
                    .iter()
                    .any(|&pixel| pixel != reference(background))
            };
            assert!(drawn(8, 24), "the icon");
            assert!(drawn(30, size.cx - 8), "the name");
        };
        check(Palette::neutral());
        check(Palette {
            high_contrast: true,
            ..Palette::neutral()
        });
        let contrast = Palette {
            high_contrast: true,
            ..Palette::neutral()
        };
        assert_eq!(
            drag_label_colors(&contrast),
            (
                contrast.editor_background,
                contrast.editor_foreground,
                contrast.editor_foreground
            )
        );
        unsafe { DeleteObject(fonts.text) };
    }

    #[test]
    fn the_drag_label_shows_a_closed_folder_or_the_note_type() {
        assert_eq!(
            drag_item(&RowKind::Folder("work".into())),
            Some(TreeItem::Folder { expanded: false })
        );
        assert_eq!(
            drag_item(&RowKind::Note(r"work\a.json".into())),
            Some(TreeItem::Note(note_kind(Some("json"))))
        );
        assert_eq!(drag_item(&RowKind::Draft), None);
    }

    #[test]
    fn is_dragged_row_never_matches_with_no_drag_even_past_the_last_row() {
        // Break caught: `None == None` reading as a match, dimming a row (e.g. the truncated
        // row, past the last real one) while nothing is being dragged (tree drag spec §3.2).
        let rows = vec![row(RowKind::Note("a.md".into()), 0)];
        assert!(!is_dragged_row(None, &rows, 0));
        assert!(!is_dragged_row(None, &rows, 5), "past the last row too");
        let dragged = DragSource::Row(RowKind::Note("a.md".into()));
        assert!(is_dragged_row(Some(&dragged), &rows, 0));
        assert!(!is_dragged_row(Some(&dragged), &rows, 5), "no row there");
        let other = DragSource::Row(RowKind::Note("b.md".into()));
        assert!(!is_dragged_row(Some(&other), &rows, 0), "a different row");
    }
}
