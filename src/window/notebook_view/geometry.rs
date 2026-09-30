//! The Notebook view's geometry: where the header, Open Editors, list and rows sit in the
//! panel, the inline field's layout, and what a point hits (rows, buttons, the scroll thumb and
//! drop targets).

use super::*;
use crate::library::tree::RowKind;
use crate::window::inline_name::FieldLayout;
use crate::window::notebook_layout::{self, PanelLayout};
use crate::window::panel_cursor;
#[cfg(test)]
use crate::window::row_list;
use crate::window::tree_drag::Hover;
use std::time::Instant;
use windows_sys::Win32::Foundation::{POINT, RECT};
use windows_sys::Win32::Graphics::Gdi::{ClientToScreen, InvalidateRect, ScreenToClient};
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::WindowsAndMessaging::{GetClientRect, GetParent};

impl NotebookView {
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
    pub(super) fn tree_hidden(&self) -> bool {
        self.mode == Mode::Tree && !self.root_expanded
    }

    /// How many rows the keyboard selection runs through in each part (`panel_cursor::step`).
    pub(super) fn shape(&self) -> panel_cursor::Shape {
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
    pub(super) fn fit_editors(&mut self, active_in_view: bool) {
        let height = height(self.layout(self.client(), self.dpi()).editors_list);
        if height <= 0 {
            return;
        }
        if active_in_view && let Some(active) = self.editors.active_index() {
            self.editors.list.ensure_visible(active, height);
        }
        self.editors.list.scroll_lines(0, height);
    }

    pub(super) fn editors_row_rect(&self, list: RECT, index: usize) -> Option<RECT> {
        let top = self.editors.list.row_top(index)?;
        Some(RECT {
            left: list.left,
            top: list.top + top,
            right: list.right,
            bottom: list.top + top + self.editors.list.row_height,
        })
    }

    pub(super) fn dpi(&self) -> u32 {
        unsafe { GetDpiForWindow(self.panel) }.max(96)
    }

    pub(super) fn client(&self) -> RECT {
        let mut rect = RECT::default();
        unsafe {
            GetClientRect(self.panel, &mut rect);
        }
        rect
    }

    pub(super) fn invalidate(&self) {
        unsafe {
            InvalidateRect(self.panel, std::ptr::null(), 0);
        }
    }

    /// The list's rectangle in panel coordinates for the current mode.
    pub(super) fn list_rect(&self, area: RECT) -> RECT {
        self.list_area(area, self.dpi())
    }

    pub(super) fn list_height(&self) -> i32 {
        height(self.list_rect(self.client())).max(self.list.row_height)
    }

    pub(super) fn row_rect(&self, list: RECT, index: usize) -> Option<RECT> {
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
    pub(super) fn inline_layout_in(&self, area: RECT, dpi: u32) -> Option<FieldLayout> {
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
}

impl NotebookView {
    pub(super) fn hit_test(&self, x: i32, y: i32) -> Hit {
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
            // A header row has no close box.
            let clean = self.editors.row(index).is_some_and(|row| !row.dirty);
            let close = clean
                && row.is_some_and(|row| {
                    contains(crate::window::open_editors::close_rect(row, dpi), x, y)
                });
            return Hit::Editor { index, close };
        }
        if contains(layout.root, x, y) {
            if self.mode != Mode::NoNotebook {
                let parts = notebook_layout::root_parts(layout.root, dpi);
                for (button, rect) in parts.shown() {
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
    pub(super) fn drag_hover(&self, x: i32, y: i32) -> Hover {
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
    pub(super) fn opens_at(&self, x: i32, y: i32) -> bool {
        let area = self.client();
        let layout = self.layout(area, self.dpi());
        if self.root.is_none() {
            return contains(area, x, y) && y >= layout.title.bottom;
        }
        contains(layout.editors_header, x, y) || contains(layout.editors_list, x, y)
    }

    /// The drag moved to `x`, `y`: the target follows, and the highlight repaints when it
    /// changed. Whether a release there moves or copies the item.
    pub(super) fn drag_to(&mut self, x: i32, y: i32, now: Instant) -> bool {
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
    pub(super) fn to_main(&self, point: POINT) -> POINT {
        let mut point = point;
        unsafe {
            ClientToScreen(self.panel, &mut point);
            ScreenToClient(GetParent(self.panel), &mut point);
        }
        point
    }
}
