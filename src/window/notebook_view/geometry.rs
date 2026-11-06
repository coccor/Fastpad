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
use crate::window::tree_drag::{self, DragSource, Hover, Reason, Verdict};
use std::time::Instant;
use windows_sys::Win32::Foundation::{POINT, RECT};
use windows_sys::Win32::Graphics::Gdi::{ClientToScreen, InvalidateRect, ScreenToClient};
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::WindowsAndMessaging::{GetClientRect, GetParent};

impl NotebookView {
    /// The list's rectangle for the current mode, in the panel's `client` coordinates at `dpi`:
    /// the tree's rows, root rows among them, or the RECENT rows.
    pub(crate) fn list_area(&self, client: RECT, dpi: u32) -> RECT {
        let body = self.layout(client, dpi).body;
        match self.mode {
            Mode::Tree => body,
            Mode::NoNotebook => state_layout(body, dpi).list,
        }
    }

    /// The panel's bands for the view as it is (open editors spec §3.1).
    pub(crate) fn layout(&self, client: RECT, dpi: u32) -> PanelLayout {
        notebook_layout::panel_layout(client, dpi, self.editors.rows.len(), self.editors_expanded)
    }

    /// The tree is painted and hit: some notebook is open.
    pub(crate) fn tree_shown(&self) -> bool {
        self.mode == Mode::Tree
    }

    /// The folder of the open notebook `id`, as the rows were last built.
    pub(super) fn notebook_folder(&self, id: NotebookId) -> Option<PathBuf> {
        self.root(id).map(|root| root.folder.clone())
    }

    /// How many rows the keyboard selection runs through in each part (`panel_cursor::step`).
    pub(super) fn shape(&self) -> panel_cursor::Shape {
        panel_cursor::Shape {
            editors: if self.editors_expanded {
                self.editors.rows.len()
            } else {
                0
            },
            // The RECENT notebooks without a notebook, else the tree's rows.
            tree: match self.mode {
                Mode::NoNotebook => self.recent.len(),
                Mode::Tree => self.rows.len(),
            },
        }
    }

    /// The root row of the notebook `id`'s parts: its chevron, label and buttons.
    pub(crate) fn root_parts_of(&self, id: NotebookId) -> Option<notebook_layout::RootParts> {
        let index = view_row_index(&self.rows, id, &RowKind::Root)?;
        let rect = self.row_rect(self.list_rect(self.client()), index)?;
        Some(notebook_layout::root_parts(rect, self.dpi()))
    }

    /// Whether root row `index` shows its buttons: while it is hovered or selected (multi-root
    /// notebooks spec §4.2).
    pub(super) fn buttons_shown(&self, index: usize) -> bool {
        self.list.hover == Some(index) || self.list.selected == Some(index)
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

    /// The first notebook's root row, in panel coordinates.
    #[cfg(test)]
    pub(crate) fn root_rect(&self) -> RECT {
        let index = self
            .rows
            .iter()
            .position(|row| row.kind == RowKind::Root)
            .expect("a notebook is open");
        self.row_rect_at(index).expect("the root row is in view")
    }

    /// Where `button` of the root row of the notebook `id` is, in panel coordinates, while that
    /// row shows its buttons.
    #[cfg(test)]
    pub(crate) fn root_button_rect(&self, id: NotebookId, button: HeaderButton) -> Option<RECT> {
        let index = view_row_index(&self.rows, id, &RowKind::Root)?;
        if !self.buttons_shown(index) {
            return None;
        }
        self.root_parts_of(id)?
            .shown()
            .find(|(shown, _)| *shown == button)
            .map(|(_, rect)| rect)
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
        let body = layout.body;
        match self.mode {
            Mode::NoNotebook => {
                let layout = state_layout(body, dpi);
                if contains(layout.button, x, y) {
                    return Hit::StateButton;
                }
                if contains(layout.second, x, y) {
                    return Hit::SecondButton;
                }
                if contains(layout.list, x, y)
                    && let Some(index) = self.list.row_at(y - layout.list.top)
                {
                    return Hit::Row {
                        index,
                        part: RowPart::Body,
                    };
                }
                Hit::Empty
            }
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
                // A root row: its buttons, where the pointer makes it the hovered row, else
                // the row itself, which toggles its notebook.
                if row.kind == RowKind::Root {
                    // Only buttons that are drawn take a click: the row hovered by the pointer
                    // (its hover was reset by any rebuild) or selected.
                    if !self.buttons_shown(index) {
                        return Hit::Root(row.notebook);
                    }
                    let parts = notebook_layout::root_parts(rect, dpi);
                    return parts
                        .shown()
                        .find(|(_, button)| contains(*button, x, y))
                        .map_or(Hit::Root(row.notebook), |(button, _)| {
                            Hit::Header(row.notebook, button)
                        });
                }
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

    /// What a drag at panel point `x`, `y` is over (tree drag spec §3.2), as a row of the view's
    /// list: a root row, a placeholder or a tree row, or the space below the last row. The
    /// scroll thumb counts as the row under it. `drag_to` keeps the drag inside its notebook.
    pub(super) fn drag_hover(&self, x: i32, y: i32) -> Hover {
        let area = self.client();
        if self.mode != Mode::Tree || !contains(area, x, y) {
            return Hover::Outside;
        }
        // The title band and Open Editors take no drop.
        let list = self.list_rect(area);
        if y < list.top {
            return Hover::Outside;
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
        if self.roots.is_empty() {
            return contains(area, x, y) && y >= layout.title.bottom;
        }
        contains(layout.editors_header, x, y) || contains(layout.editors_list, x, y)
    }

    /// The drag moved to `x`, `y`: the target follows, and the highlight repaints when it
    /// changed. Whether a release there moves or copies the item.
    pub(super) fn drag_to(&mut self, x: i32, y: i32, now: Instant) -> bool {
        let hover = self.drag_hover(x, y);
        // The notebook the pointer is over. Files and tabs copy into it; a row stays in its own
        // and refuses the rest, with a hint (multi-root notebooks spec §4.5).
        let under = match hover {
            Hover::Row(index) => self.rows.get(index).map(|row| row.notebook),
            Hover::Below => self.rows.last().map(|row| row.notebook),
            Hover::Header | Hover::Outside => None,
        };
        let row_kind = match self.drag.as_ref().map(|drag| &drag.source) {
            Some(DragSource::Row(kind)) => Some(kind.clone()),
            _ => None,
        };
        if row_kind.is_none()
            && let Some(under) = under
        {
            self.drag_notebook = Some(under);
        }
        let refused =
            row_kind
                .zip(self.drag_notebook.zip(under))
                .and_then(|(kind, (from, under))| {
                    match tree_drag::accepts(from, under, &kind, Path::new("")) {
                        Verdict::Rejected(reason @ Reason::OtherNotebook) => Some(reason),
                        _ => None,
                    }
                });
        let span = self
            .drag_notebook
            .and_then(|id| Some((self.notebook_span(id)?, self.notebook_folder(id)?)));
        let Some(drag) = self.drag.as_mut() else {
            return false;
        };
        let changed = match span {
            Some(((contents, end), root)) => {
                let hover = match hover {
                    Hover::Row(index) if (contents..end).contains(&index) => {
                        Hover::Row(index - contents)
                    }
                    // Its own root row is its root folder.
                    Hover::Row(index) if index < contents && index + 1 == contents => {
                        match self.rows.get(index).map(|row| &row.kind) {
                            Some(RowKind::Root) => Hover::Header,
                            _ => Hover::Outside,
                        }
                    }
                    Hover::Row(_) => Hover::Outside,
                    // The space under the last row is the last root's.
                    Hover::Below if end < self.rows.len() => Hover::Outside,
                    other => other,
                };
                drag.hover(&self.rows[contents..end], &root, (x, y), hover, now)
            }
            None => drag.hover(
                &[] as &[ViewRow],
                Path::new(""),
                (x, y),
                Hover::Outside,
                now,
            ),
        };
        let hint_changed = std::mem::replace(&mut drag.refused, refused) != refused;
        let accepted = drag.target.is_some();
        if changed || hint_changed {
            self.invalidate();
        }
        accepted
    }

    /// The hint a started drag shows while its pointer is somewhere it cannot drop for a reason
    /// worth saying (multi-root notebooks spec §4.5).
    pub(crate) fn drag_hint(&self) -> Option<&'static str> {
        self.drag
            .as_ref()
            .filter(|drag| drag.started)?
            .refused?
            .hint()
    }

    /// Where the contents of the notebook `id` are in the rows: from the row after its root row
    /// to the end of its rows.
    pub(super) fn notebook_span(&self, id: NotebookId) -> Option<(usize, usize)> {
        let first = self.rows.iter().position(|row| row.notebook == id)?;
        let end = self.rows[first..]
            .iter()
            .position(|row| row.notebook != id)
            .map_or(self.rows.len(), |offset| first + offset);
        let contents = if self.rows[first].kind == RowKind::Root {
            first + 1
        } else {
            first
        };
        Some((contents, end))
    }

    /// The band a started drag's target shows, over the view's rows (tree drag spec §3.2): its
    /// folder's rows, or for its root folder, the root row and everything under it.
    pub(super) fn drop_highlight(&self) -> Option<crate::window::tree_drag::Highlight> {
        use crate::window::tree_drag::Highlight;
        let drag = self.drag.as_ref().filter(|drag| drag.started)?;
        let folder = drag.target.as_deref()?;
        let (contents, end) = self.notebook_span(self.drag_notebook?)?;
        let rows = &self.rows[contents..end];
        Some(match crate::window::tree_drag::highlight(rows, folder)? {
            Highlight::Root if contents > 0 && self.rows[contents - 1].kind == RowKind::Root => {
                Highlight::Rows {
                    start: contents - 1,
                    end,
                }
            }
            Highlight::Root => Highlight::Root,
            Highlight::Rows { start, end } => Highlight::Rows {
                start: contents + start,
                end: contents + end,
            },
        })
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
