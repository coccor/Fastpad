//! The Tags view (second brain phase 1 spec §8.1): the open notebook's tags as a tree split on
//! `/`, each with how many notes use it or a tag under it. Expanding a tag lists its notes, then
//! its child tags. A click opens a note in the preview tab; a double click or Enter keeps it.

use crate::library::index::TagNode;
use crate::window::design::metrics::{panel_header, scale, sidebar_row};
use crate::window::library_host;
use crate::window::main_window::OpenMode;
use crate::window::panel::fill;
use crate::window::row_list::{self, ListKey, RowListState, RowLook, row_foreground};
use crate::window::side_panel::{ViewPaint, draw_text, point_of};
use std::collections::HashSet;
use std::path::PathBuf;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    DT_CENTER, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_RIGHT, DT_SINGLELINE, DT_VCENTER, HDC,
    InvalidateRect,
};
use windows_sys::Win32::UI::Controls::WM_MOUSELEAVE;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetFocus, ReleaseCapture, SetCapture, SetFocus, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent,
    VK_LEFT, VK_RETURN, VK_RIGHT, VK_SPACE,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetClientRect, WM_CAPTURECHANGED, WM_KEYDOWN, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MOUSEMOVE, WM_MOUSEWHEEL,
};

const PADDING_AT_96_DPI: i32 = 12;
const INDENT_AT_96_DPI: i32 = 12;
const CHEVRON_AT_96_DPI: i32 = 16;
const GLYPH_AT_96_DPI: i32 = 20;
const GAP_AT_96_DPI: i32 = 6;
const COUNT_AT_96_DPI: i32 = 40;

const CHEVRON_RIGHT: &str = "\u{E76C}";
const CHEVRON_DOWN: &str = "\u{E70D}";
const TAG_GLYPH: &str = "\u{E8EC}";
const NOTE_GLYPH: &str = "\u{E8A5}";

pub(crate) const HEADER_TEXT: &str = "Tags";
pub(crate) const NO_NOTEBOOK: &str = "No notebook open.";
pub(crate) const INDEXING: &str = "Indexing notes\u{2026}";
pub(crate) const EMPTY_TEXT: &str =
    "No tags yet. Add #tags to your notes, or tags: in their front matter.";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum TagRowKind {
    /// A tag, by its full lowercase name.
    Tag { full: String, expanded: bool },
    /// A note, relative to the notebook.
    Note(PathBuf),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TagRow {
    pub kind: TagRowKind,
    pub depth: u16,
    /// A tag's last segment with its `#`, or a note's file name.
    pub name: String,
    /// For a tag: the notes that use it or a tag under it.
    pub count: usize,
}

/// The visible rows of `nodes`: each tag, and inside each expanded tag its notes, then its child
/// tags, one level deeper.
pub(crate) fn tag_rows(nodes: &[TagNode], expanded: &HashSet<String>) -> Vec<TagRow> {
    fn walk(nodes: &[TagNode], depth: u16, expanded: &HashSet<String>, rows: &mut Vec<TagRow>) {
        for node in nodes {
            let open = expanded.contains(&node.full);
            rows.push(TagRow {
                kind: TagRowKind::Tag {
                    full: node.full.clone(),
                    expanded: open,
                },
                depth,
                name: format!("#{}", node.name),
                count: node.count,
            });
            if open {
                rows.extend(node.notes.iter().map(|path| TagRow {
                    kind: TagRowKind::Note(path.clone()),
                    depth: depth + 1,
                    name: path.file_name().map_or_else(
                        || path.display().to_string(),
                        |name| name.to_string_lossy().into_owned(),
                    ),
                    count: 0,
                }));
                walk(&node.children, depth + 1, expanded, rows);
            }
        }
    }
    let mut rows = Vec::new();
    walk(nodes, 0, expanded, &mut rows);
    rows
}

/// What the view shows instead of rows, or `Tree`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Mode {
    NoNotebook,
    Loading,
    Empty,
    Tree,
}

/// What a click or key asks for. The caller runs it with no view borrowed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum TagAction {
    Toggle(String),
    Open { note: PathBuf, keep: bool },
}

#[derive(Debug)]
pub(crate) struct TagsView {
    pub(crate) mode: Mode,
    pub(crate) rows: Vec<TagRow>,
    pub(crate) list: RowListState,
    /// Expanded tags, by full lowercase name.
    pub(crate) expanded: HashSet<String>,
    thumb_grab: Option<i32>,
    /// Bumped whenever the rows change order (`AccessibleView::accessible_generation`).
    order: u64,
}

fn inside(rect: RECT, point: POINT) -> bool {
    point.x >= rect.left && point.x < rect.right && point.y >= rect.top && point.y < rect.bottom
}

const fn height(rect: RECT) -> i32 {
    let height = rect.bottom - rect.top;
    if height > 0 { height } else { 0 }
}

/// What identifies a row across rebuilds.
fn row_key(row: &TagRow) -> String {
    match &row.kind {
        TagRowKind::Tag { full, .. } => format!("t:{full}"),
        TagRowKind::Note(path) => format!("n:{}:{}", row.depth, path.to_string_lossy()),
    }
}

impl TagsView {
    pub(crate) fn new(dpi: u32) -> Self {
        Self {
            mode: Mode::NoNotebook,
            rows: Vec::new(),
            list: RowListState::new(sidebar_row(dpi)),
            expanded: HashSet::new(),
            thumb_grab: None,
            order: 0,
        }
    }

    /// Replaces the rows, keeping the selection on the same tag or note.
    pub(crate) fn set_rows(&mut self, mode: Mode, rows: Vec<TagRow>, dpi: u32, height: i32) {
        let kept = self
            .list
            .selected
            .and_then(|index| self.rows.get(index))
            .map(row_key);
        let reordered = rows.len() != self.rows.len()
            || rows
                .iter()
                .zip(&self.rows)
                .any(|(new, old)| row_key(new) != row_key(old));
        if reordered {
            self.order = self.order.wrapping_add(1);
        }
        let previous = self.list.selected;
        self.mode = mode;
        self.rows = rows;
        self.list.row_height = sidebar_row(dpi);
        self.list.set_count(self.rows.len());
        let index = kept
            .and_then(|key| self.rows.iter().position(|row| row_key(row) == key))
            .or_else(|| {
                previous
                    .filter(|_| !self.rows.is_empty())
                    .map(|index| index.min(self.rows.len() - 1))
            });
        if let Some(index) = index {
            self.list.select(index, height);
        }
        self.list.scroll_lines(0, height);
    }

    pub(crate) fn list_area(&self, client: RECT, dpi: u32) -> RECT {
        RECT {
            top: (client.top + panel_header(dpi)).min(client.bottom),
            ..client
        }
    }

    pub(crate) fn paint(&self, paint: &ViewPaint) {
        let dpi = paint.dpi;
        let palette = paint.palette;
        let client = paint.client;
        let pad = scale(PADDING_AT_96_DPI, dpi);
        let line = DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX;
        let header = RECT {
            bottom: client.top + panel_header(dpi),
            ..client
        };
        unsafe {
            fill(paint.hdc, client, paint.background);
            draw_text(
                paint.hdc,
                HEADER_TEXT,
                RECT {
                    left: header.left + pad,
                    ..header
                },
                paint.fonts.bold,
                palette.muted_foreground,
                line | DT_LEFT,
            );
            let message = match self.mode {
                Mode::NoNotebook => Some(NO_NOTEBOOK),
                Mode::Loading => Some(INDEXING),
                Mode::Empty => Some(EMPTY_TEXT),
                Mode::Tree => None,
            };
            if let Some(message) = message {
                draw_text(
                    paint.hdc,
                    message,
                    RECT {
                        left: client.left + pad,
                        top: header.bottom,
                        right: client.right - pad,
                        bottom: header.bottom + sidebar_row(dpi),
                    },
                    paint.fonts.text,
                    palette.muted_foreground,
                    line | DT_LEFT | DT_END_ELLIPSIS,
                );
                return;
            }
        }
        row_list::paint(
            paint.hdc,
            self.list_area(client, dpi),
            &self.list,
            &palette,
            paint.focused,
            paint.focused,
            &|_| paint.background,
            dpi,
            &mut |hdc, index, rect, look| self.draw_row(hdc, index, rect, look, paint),
        );
    }

    fn draw_row(&self, hdc: HDC, index: usize, rect: RECT, look: RowLook, paint: &ViewPaint) {
        let Some(row) = self.rows.get(index) else {
            return;
        };
        let dpi = paint.dpi;
        let palette = paint.palette;
        let line = DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX;
        let foreground = row_foreground(look, &palette);
        let muted = if look.selected && look.focused {
            foreground
        } else {
            palette.muted_foreground
        };
        let left = rect.left
            + scale(PADDING_AT_96_DPI, dpi)
            + i32::from(row.depth) * scale(INDENT_AT_96_DPI, dpi);
        let chevron = RECT {
            left,
            right: left + scale(CHEVRON_AT_96_DPI, dpi),
            ..rect
        };
        let glyph = RECT {
            left: chevron.right,
            right: chevron.right + scale(GLYPH_AT_96_DPI, dpi),
            ..rect
        };
        let count = RECT {
            left: rect.right - scale(COUNT_AT_96_DPI + PADDING_AT_96_DPI, dpi),
            right: rect.right - scale(PADDING_AT_96_DPI, dpi),
            ..rect
        };
        let name = RECT {
            left: glyph.right + scale(GAP_AT_96_DPI, dpi),
            right: if matches!(row.kind, TagRowKind::Tag { .. }) {
                count.left
            } else {
                rect.right - scale(PADDING_AT_96_DPI, dpi)
            },
            ..rect
        };
        unsafe {
            match &row.kind {
                TagRowKind::Tag { expanded, .. } => {
                    let arrow = if *expanded {
                        CHEVRON_DOWN
                    } else {
                        CHEVRON_RIGHT
                    };
                    draw_text(
                        hdc,
                        arrow,
                        chevron,
                        paint.fonts.glyph,
                        muted,
                        line | DT_CENTER,
                    );
                    draw_text(
                        hdc,
                        TAG_GLYPH,
                        glyph,
                        paint.fonts.glyph,
                        muted,
                        line | DT_CENTER,
                    );
                    draw_text(
                        hdc,
                        &row.count.to_string(),
                        count,
                        paint.fonts.text,
                        muted,
                        line | DT_RIGHT,
                    );
                }
                TagRowKind::Note(_) => {
                    draw_text(
                        hdc,
                        NOTE_GLYPH,
                        glyph,
                        paint.fonts.glyph,
                        muted,
                        line | DT_CENTER,
                    );
                }
            }
            draw_text(
                hdc,
                &row.name,
                name,
                paint.fonts.text,
                foreground,
                line | DT_LEFT | DT_END_ELLIPSIS,
            );
        }
    }

    fn hover(&mut self, point: POINT, client: RECT, dpi: u32) -> bool {
        let area = self.list_area(client, dpi);
        if let Some(grab) = self.thumb_grab {
            return self.list.drag_thumb(grab, point.y - area.top, height(area));
        }
        let row = inside(area, point)
            .then(|| self.list.row_at(point.y - area.top))
            .flatten();
        self.list.set_hover(row)
    }

    fn thumb_at(&self, point: POINT, client: RECT, dpi: u32) -> Option<i32> {
        let area = self.list_area(client, dpi);
        self.list.thumb_hit(
            point.x - area.left,
            point.y - area.top,
            area.right - area.left,
            height(area),
        )
    }

    /// `WM_LBUTTONDOWN` (or `WM_LBUTTONDBLCLK` with `double`): selects the row under `point`.
    pub(crate) fn click(
        &mut self,
        point: POINT,
        client: RECT,
        dpi: u32,
        double: bool,
    ) -> Option<TagAction> {
        let area = self.list_area(client, dpi);
        if self.mode != Mode::Tree || !inside(area, point) {
            return None;
        }
        let index = self.list.row_at(point.y - area.top)?;
        self.list.select(index, height(area));
        match &self.rows.get(index)?.kind {
            // The second press of a double click on a tag would undo the first: it does nothing.
            TagRowKind::Tag { .. } if double => None,
            TagRowKind::Tag { full, .. } => Some(TagAction::Toggle(full.clone())),
            TagRowKind::Note(note) => Some(TagAction::Open {
                note: note.clone(),
                keep: double,
            }),
        }
    }

    pub(crate) fn key(&mut self, key: u16, client: RECT, dpi: u32) -> Option<Option<TagAction>> {
        if let Some(movement) = ListKey::from_virtual_key(u32::from(key)) {
            let area = self.list_area(client, dpi);
            self.list.move_selection(movement, height(area));
            return Some(None);
        }
        let row = self.list.selected.and_then(|index| self.rows.get(index))?;
        Some(match (key, &row.kind) {
            (VK_RETURN | VK_SPACE, TagRowKind::Tag { full, .. }) => {
                Some(TagAction::Toggle(full.clone()))
            }
            (
                VK_RIGHT,
                TagRowKind::Tag {
                    full,
                    expanded: false,
                },
            ) => Some(TagAction::Toggle(full.clone())),
            (
                VK_LEFT,
                TagRowKind::Tag {
                    full,
                    expanded: true,
                },
            ) => Some(TagAction::Toggle(full.clone())),
            (VK_RETURN | VK_SPACE, TagRowKind::Note(note)) => Some(TagAction::Open {
                note: note.clone(),
                keep: true,
            }),
            (VK_LEFT | VK_RIGHT, _) => None,
            _ => return None,
        })
    }
}

fn with_view<R>(hwnd: HWND, f: impl FnOnce(&mut TagsView) -> R) -> Option<R> {
    let mut app = unsafe { super::main_window::app_ptr(hwnd) }?;
    unsafe { app.as_mut() }
        .sidebar
        .as_mut()
        .map(|sidebar| f(&mut sidebar.tags))
}

fn geometry(panel: HWND) -> (RECT, u32) {
    let mut client = RECT::default();
    unsafe {
        GetClientRect(panel, &mut client);
    }
    (client, unsafe { GetDpiForWindow(panel) }.max(96))
}

fn invalidate(panel: HWND) {
    unsafe {
        InvalidateRect(panel, std::ptr::null(), 0);
    }
}

fn point(lparam: LPARAM) -> POINT {
    let (x, y) = point_of(lparam);
    POINT { x, y }
}

/// Re-reads the tags from the index. Part of `side_panel::refresh`.
pub(crate) fn refresh(hwnd: HWND, panel: HWND) {
    let mode = if library_host::folder(hwnd).is_none() {
        Mode::NoNotebook
    } else if !library_host::index_loaded(hwnd) {
        Mode::Loading
    } else {
        Mode::Tree
    };
    let nodes = library_host::note_index(hwnd).tags();
    let mode = if mode == Mode::Tree && nodes.is_empty() {
        Mode::Empty
    } else {
        mode
    };
    let (client, dpi) = geometry(panel);
    with_view(hwnd, |view| {
        let rows = if mode == Mode::Tree {
            tag_rows(&nodes, &view.expanded)
        } else {
            Vec::new()
        };
        let area = view.list_area(client, dpi);
        view.set_rows(mode, rows, dpi, height(area));
    });
    invalidate(panel);
}

pub(crate) fn refresh_metrics(hwnd: HWND, panel: HWND) {
    let (client, dpi) = geometry(panel);
    with_view(hwnd, |view| {
        let height = height(view.list_area(client, dpi));
        view.list.row_height = sidebar_row(dpi);
        view.list.scroll_lines(0, height);
    });
    invalidate(panel);
}

pub(crate) fn paint(hwnd: HWND, paint: &ViewPaint) {
    if let Some(app) = unsafe { super::main_window::app_ptr(hwnd) }
        && let Some(sidebar) = unsafe { app.as_ref() }.sidebar.as_ref()
    {
        sidebar.tags.paint(paint);
    }
}

/// Shows the Tags view with `tag` (any case, `#` optional) selected, its parents expanded: a
/// click on a tag in the preview.
pub(crate) fn reveal(hwnd: HWND, tag: &str) {
    let full = tag.trim_start_matches('#').to_lowercase();
    with_view(hwnd, |view| {
        for parent in crate::library::facts::parent_tags(&full) {
            view.expanded.insert(parent.to_owned());
        }
    });
    crate::window::side_panel::show_view(hwnd, crate::config::SidebarView::Tags, true);
    let Some((_, panel)) = crate::window::side_panel::windows(hwnd) else {
        return;
    };
    refresh(hwnd, panel);
    let (client, dpi) = geometry(panel);
    with_view(hwnd, |view| {
        let area = view.list_area(client, dpi);
        if let Some(index) = view.rows.iter().position(
            |row| matches!(&row.kind, TagRowKind::Tag { full: name, .. } if *name == full),
        ) {
            view.list.select(index, height(area));
        }
    });
    invalidate(panel);
}

/// Runs an action with no view borrowed.
fn run(hwnd: HWND, panel: HWND, action: TagAction) {
    match action {
        TagAction::Toggle(full) => {
            with_view(hwnd, |view| {
                if !view.expanded.remove(&full) {
                    view.expanded.insert(full);
                }
            });
            refresh(hwnd, panel);
        }
        TagAction::Open { note, keep } => {
            let Some(folder) = library_host::folder(hwnd) else {
                return;
            };
            let path = folder.join(note);
            let (mode, focus) = if keep {
                (OpenMode::Permanent, true)
            } else {
                (OpenMode::Preview, false)
            };
            if let Err(error) = crate::window::main_window::open_note(hwnd, &path, mode, focus) {
                crate::window::main_window::report_open_failure(hwnd, &path, &error);
            }
        }
    }
}

/// Input for the Tags view. `None` leaves the message to the panel's default handling.
pub(crate) fn handle(
    hwnd: HWND,
    panel: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> Option<LRESULT> {
    let (client, dpi) = geometry(panel);
    match message {
        WM_MOUSEMOVE => {
            let mut track = TRACKMOUSEEVENT {
                cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: panel,
                dwHoverTime: 0,
            };
            unsafe {
                TrackMouseEvent(&mut track);
            }
            let at = point(lparam);
            if with_view(hwnd, |view| view.hover(at, client, dpi)).unwrap_or(false) {
                invalidate(panel);
            }
            Some(0)
        }
        WM_MOUSELEAVE => {
            if with_view(hwnd, |view| view.list.set_hover(None)).unwrap_or(false) {
                invalidate(panel);
            }
            Some(0)
        }
        WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
            if unsafe { GetFocus() } != panel {
                unsafe {
                    SetFocus(panel);
                }
            }
            let at = point(lparam);
            let grabbed = message == WM_LBUTTONDOWN
                && with_view(hwnd, |view| {
                    view.thumb_grab = view.thumb_at(at, client, dpi);
                    view.thumb_grab.is_some()
                })
                .unwrap_or(false);
            if grabbed {
                unsafe {
                    SetCapture(panel);
                }
                return Some(0);
            }
            let double = message == WM_LBUTTONDBLCLK;
            let action = with_view(hwnd, |view| view.click(at, client, dpi, double)).flatten();
            invalidate(panel);
            if let Some(action) = action {
                run(hwnd, panel, action);
            }
            Some(0)
        }
        WM_LBUTTONUP => {
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
        WM_MOUSEWHEEL => {
            let delta = i32::from((wparam >> 16) as u16 as i16);
            let lines = row_list::wheel_lines();
            let area = with_view(hwnd, |view| view.list_area(client, dpi));
            if let Some(area) = area
                && with_view(hwnd, |view| view.list.wheel(delta, lines, height(area)))
                    .unwrap_or(false)
            {
                invalidate(panel);
            }
            Some(0)
        }
        WM_KEYDOWN => {
            let result = with_view(hwnd, |view| view.key(wparam as u16, client, dpi)).flatten();
            let action = result?;
            invalidate(panel);
            if let Some(action) = action {
                run(hwnd, panel, action);
            }
            Some(0)
        }
        _ => None,
    }
}

/// The rows the view lists, for in-process tests.
#[cfg(test)]
pub(crate) fn shown_rows(hwnd: HWND) -> Vec<TagRow> {
    with_view(hwnd, |view| view.rows.clone()).unwrap_or_default()
}

impl crate::window::sidebar_accessibility::AccessibleView for TagsView {
    fn accessible_count(&self, _client: RECT, _dpi: u32) -> usize {
        if self.mode == Mode::Tree {
            self.rows.len()
        } else {
            1
        }
    }

    fn accessible_item(
        &self,
        index: usize,
        client: RECT,
        dpi: u32,
        focused: bool,
    ) -> Option<crate::window::sidebar_accessibility::AccessibleItem> {
        use crate::window::sidebar_accessibility::{outline_item, row_rect, text_item};
        if self.mode != Mode::Tree {
            let text = match self.mode {
                Mode::NoNotebook => NO_NOTEBOOK,
                Mode::Loading => INDEXING,
                _ => EMPTY_TEXT,
            };
            let area = self.list_area(client, dpi);
            return (index == 0).then(|| {
                text_item(
                    text,
                    RECT {
                        bottom: area.top + sidebar_row(dpi),
                        ..area
                    },
                )
            });
        }
        let row = self.rows.get(index)?;
        let (rect, visible) = row_rect(self.list_area(client, dpi), &self.list, index);
        let selected = self.list.selected == Some(index);
        let (name, expanded) = match &row.kind {
            TagRowKind::Tag { expanded, .. } => (
                format!(
                    "{}, {} {}",
                    row.name,
                    row.count,
                    if row.count == 1 { "note" } else { "notes" }
                ),
                Some(*expanded),
            ),
            TagRowKind::Note(_) => (row.name.clone(), None),
        };
        Some(outline_item(
            name, expanded, row.depth, selected, focused, rect, visible,
        ))
    }

    fn accessible_hit(&self, point: POINT, client: RECT, dpi: u32) -> Option<usize> {
        let area = self.list_area(client, dpi);
        if self.mode != Mode::Tree || !inside(area, point) {
            return None;
        }
        self.list.row_at(point.y - area.top)
    }

    fn accessible_current(&self, _client: RECT, _dpi: u32) -> Option<usize> {
        self.list.selected
    }

    fn accessible_select(&mut self, index: usize, client: RECT, dpi: u32) {
        if index < self.rows.len() {
            let area = self.list_area(client, dpi);
            self.list.select(index, area.bottom - area.top);
        }
    }

    fn accessible_identity(&self, index: usize, _client: RECT, _dpi: u32) -> Option<u64> {
        Some(crate::window::sidebar_accessibility::identity_of(&row_key(
            self.rows.get(index)?,
        )))
    }

    fn accessible_generation(&self) -> u64 {
        self.order
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(name: &str, full: &str, notes: &[&str], children: Vec<TagNode>) -> TagNode {
        TagNode {
            name: name.to_owned(),
            full: full.to_owned(),
            notes: notes.iter().map(PathBuf::from).collect(),
            count: notes.len() + children.iter().map(|child| child.count).sum::<usize>(),
            children,
        }
    }

    fn tree() -> Vec<TagNode> {
        vec![
            node(
                "project",
                "project",
                &["plan.md"],
                vec![node("atlas", "project/atlas", &[r"work\atlas.md"], vec![])],
            ),
            node("todo", "todo", &["a.md", "b.md"], vec![]),
        ]
    }

    fn names(rows: &[TagRow]) -> Vec<(String, u16)> {
        rows.iter()
            .map(|row| (row.name.clone(), row.depth))
            .collect()
    }

    #[test]
    fn collapsed_tags_show_only_the_top_level() {
        let rows = tag_rows(&tree(), &HashSet::new());
        assert_eq!(
            names(&rows),
            [("#project".to_owned(), 0), ("#todo".to_owned(), 0)]
        );
        assert_eq!(rows[0].count, 2);
    }

    #[test]
    fn an_expanded_tag_lists_its_notes_then_its_child_tags() {
        let expanded: HashSet<String> = ["project".to_owned(), "project/atlas".to_owned()].into();
        let rows = tag_rows(&tree(), &expanded);
        assert_eq!(
            names(&rows),
            [
                ("#project".to_owned(), 0),
                ("plan.md".to_owned(), 1),
                ("#atlas".to_owned(), 1),
                ("atlas.md".to_owned(), 2),
                ("#todo".to_owned(), 0),
            ]
        );
        assert_eq!(
            rows[3].kind,
            TagRowKind::Note(PathBuf::from(r"work\atlas.md"))
        );
    }

    #[test]
    fn keys_expand_collapse_and_open() {
        let mut view = TagsView::new(96);
        let client = RECT {
            left: 0,
            top: 0,
            right: 260,
            bottom: 400,
        };
        view.set_rows(Mode::Tree, tag_rows(&tree(), &HashSet::new()), 96, 400);
        view.list.select(0, 400);
        assert_eq!(
            view.key(VK_RIGHT, client, 96),
            Some(Some(TagAction::Toggle("project".to_owned())))
        );
        assert_eq!(
            view.key(VK_LEFT, client, 96),
            Some(None),
            "already collapsed"
        );
        view.expanded.insert("todo".to_owned());
        view.set_rows(
            Mode::Tree,
            tag_rows(&tree(), &view.expanded.clone()),
            96,
            400,
        );
        view.list.select(2, 400);
        assert_eq!(
            view.key(VK_RETURN, client, 96),
            Some(Some(TagAction::Open {
                note: PathBuf::from("a.md"),
                keep: true
            }))
        );
    }
}
