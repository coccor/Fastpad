//! What the MSAA provider reads of the Notebook view: the flat accessible index over push
//! buttons, the Open Editors section and the tree (root rows among its rows) or RECENT rows.

use super::*;
use crate::window::notebook_layout;
use crate::window::panel_cursor::Cursor;
use crate::window::row_list::RowListState;
use windows_sys::Win32::Foundation::{POINT, RECT};

/// What the MSAA provider reads of the view.
impl NotebookView {
    pub(crate) fn rows(&self) -> &[ViewRow] {
        &self.rows
    }

    /// The tree rows screen readers see: all but the draft row (inline naming spec §6).
    pub(super) fn accessible_rows(&self) -> usize {
        self.rows.len() - usize::from(self.inline.draft_at().is_some())
    }

    /// The children after the push buttons, in order: the Open Editors header and its rows, then
    /// the RECENT notebooks or the tree rows, root rows among them.
    pub(super) fn accessible_parts(&self) -> (usize, usize) {
        let editors = if self.editors_expanded {
            self.editors.rows.len()
        } else {
            0
        };
        (1, editors)
    }

    /// The tree rows screen readers see: `accessible_rows`, while the tree shows.
    pub(super) fn accessible_tree_rows(&self) -> usize {
        if self.tree_shown() {
            self.accessible_rows()
        } else {
            0
        }
    }

    /// The row that accessible tree row `index` stands for.
    pub(super) fn row_of_accessible(&self, index: usize) -> usize {
        match self.inline.draft_at() {
            Some(draft) if index >= draft => index + 1,
            _ => index,
        }
    }

    /// Row `index`'s accessible tree row; `None` for the draft row.
    pub(super) fn accessible_of_row(&self, index: usize) -> Option<usize> {
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

    /// Every push button painted, in paint order, with its accessible name: the buttons of each
    /// root row that shows them (hovered or selected, multi-root notebooks spec §4.2), or the
    /// no-notebook state's own buttons.
    pub(crate) fn buttons(&self, client: RECT, dpi: u32) -> Vec<(String, RECT)> {
        let mut buttons = Vec::new();
        match self.mode {
            Mode::Tree => {
                let list = self.list_area(client, dpi);
                for (index, row) in self.rows.iter().enumerate() {
                    if row.kind != RowKind::Root || !self.buttons_shown(index) {
                        continue;
                    }
                    let (Some(root), Some(top)) =
                        (self.root(row.notebook), self.list.row_top(index))
                    else {
                        continue;
                    };
                    let rect = RECT {
                        left: list.left,
                        top: list.top + top,
                        right: list.right,
                        bottom: list.top + top + self.list.row_height,
                    };
                    if rect.bottom > list.bottom {
                        continue;
                    }
                    for (button, rect) in notebook_layout::root_parts(rect, dpi).shown() {
                        buttons.push((paint::button_name(button, root).to_owned(), rect));
                    }
                }
            }
            Mode::NoNotebook => {
                let state = state_layout(self.layout(client, dpi).body, dpi);
                buttons.push(("Open notebook…".to_owned(), state.button));
                buttons.push(("New notebook…".to_owned(), state.second));
            }
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

impl crate::window::sidebar_accessibility::AccessibleView for NotebookView {
    /// Push buttons first, then the Open Editors header and its rows, then the RECENT notebooks
    /// (no-notebook state) or the tree rows, root rows among them, the draft row left out.
    fn accessible_count(&self, client: RECT, dpi: u32) -> usize {
        let (header, editors) = self.accessible_parts();
        self.buttons(client, dpi).len()
            + header
            + editors
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
        let (header, editors) = self.accessible_parts();
        let index = index - buttons.len();
        if index < header {
            return Some(section_item(
                &format!("Open editors, {}", self.editors.view_count()),
                self.editors_expanded,
                self.cursor == Cursor::EditorsHeader,
                focused,
                layout.editors_header,
            ));
        }
        let index = index - header;
        if index < editors {
            let (rect, visible) = row_rect(layout.editors_list, &self.editors.list, index);
            let name = match self.editors.rows.get(index)? {
                crate::window::open_editors::EditorEntry::Header(number) => {
                    return Some(crate::window::sidebar_accessibility::text_item(
                        &crate::window::open_editors::header_label(*number),
                        rect,
                    ));
                }
                crate::window::open_editors::EditorEntry::PinnedHeader => {
                    return Some(crate::window::sidebar_accessibility::text_item(
                        crate::window::open_editors::PINNED_LABEL,
                        rect,
                    ));
                }
                crate::window::open_editors::EditorEntry::View(row) => {
                    crate::window::open_editors::accessible_name(row)
                }
                crate::window::open_editors::EditorEntry::Pinned(row) => {
                    crate::window::open_editors::pinned_accessible_name(row)
                }
            };
            return Some(editor_item(
                &name,
                self.cursor == Cursor::Editor(index),
                focused,
                rect,
                visible,
            ));
        }
        let index = index - editors;
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
        let (header, editors) = self.accessible_parts();
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
        let area = self.list_area(client, dpi);
        if !inside(&area) {
            return None;
        }
        let row = self.list().row_at(point.y - area.top)?;
        let rows = start + header + editors;
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
        let (header, editors) = self.accessible_parts();
        match self.cursor {
            Cursor::EditorsHeader => return Some(start),
            Cursor::Editor(index) => return (index < editors).then_some(start + header + index),
            Cursor::Tree => {}
        }
        let rows = start + header + editors;
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
        let (header, editors) = self.accessible_parts();
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
        let (header, editors) = self.accessible_parts();
        let index = index.checked_sub(self.buttons(client, dpi).len())?;
        if index < header {
            return Some(identity_of(&"open-editors"));
        }
        let index = index - header;
        if index < editors {
            return Some(match self.editors.rows.get(index)? {
                crate::window::open_editors::EditorEntry::Header(number) => {
                    identity_of(&("editor-group", *number))
                }
                crate::window::open_editors::EditorEntry::View(row) => {
                    identity_of(&("editor", row.group.0, row.id.0))
                }
                crate::window::open_editors::EditorEntry::PinnedHeader => {
                    identity_of(&"pinned-notes")
                }
                crate::window::open_editors::EditorEntry::Pinned(row) => {
                    identity_of(&("pinned-note", &row.path))
                }
            });
        }
        let index = index - editors;
        if let Some(folder) = self.recent.get(index) {
            return Some(identity_of(folder));
        }
        let index = index - self.recent.len();
        if index >= self.accessible_tree_rows() {
            return None;
        }
        let row = self.rows().get(self.row_of_accessible(index))?;
        Some(identity_of(&(row.notebook.0, &row.kind)))
    }

    fn accessible_generation(&self) -> u64 {
        self.order
    }
}
