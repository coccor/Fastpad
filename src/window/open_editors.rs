//! The Notebook view's Open Editors section (open editors spec §3.2): one row per tab in
//! tab-strip order, with its type icon and name, a dot while it has unsaved changes and a close
//! box on hover. With several editor groups, each group's tabs follow a "Group N" header (split
//! editors spec §7). Built from the tab list in memory: no disk.

use crate::document::{Document, DocumentId};
use crate::window::file_icons::{NoteKind, note_kind};
use crate::window::icon_sets::TreeItem;
use crate::window::icon_sets::images::IconImages;
use crate::window::main_window::app_ptr;
use crate::window::notebook_view::draw_item_icon;
use crate::window::panel::scale;
use crate::window::row_list::{RowListState, RowLook, row_foreground};
use crate::window::side_panel::{ViewPaint, draw_text};
use crate::window::split_tree::GroupId;
use std::path::PathBuf;
use windows_sys::Win32::Foundation::{HWND, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    DT_CENTER, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, HDC,
};

// Sizes at 96 DPI.
const LEFT_PAD: i32 = 24;
const GLYPH_BOX: i32 = 16;
const GAP: i32 = 6;
const CLOSE_BOX: i32 = 24;
/// Segoe MDL2 Assets' Cancel, the tab strip's close glyph.
const GLYPH_CLOSE: &str = "\u{E711}";
const DIRTY_DOT: &str = "\u{25CF}";
const LINE: u32 = DT_SINGLELINE | DT_VCENTER | DT_LEFT | DT_END_ELLIPSIS | DT_NOPREFIX;
const CENTERED: u32 = DT_SINGLELINE | DT_VCENTER | DT_CENTER | DT_NOPREFIX;

/// One tab as the section shows it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct EditorRow {
    pub id: DocumentId,
    /// The group whose tab this is.
    pub group: GroupId,
    /// The file name with its extension, or an untitled tab's label.
    pub name: String,
    pub path: Option<PathBuf>,
    pub dirty: bool,
    pub active: bool,
}

/// An untitled tab's name: its tab label, else its first line, else "Untitled".
pub(crate) fn unsaved_label(document: &Document) -> String {
    document
        .untitled_label
        .clone()
        .or_else(|| document.first_line_label.clone())
        .unwrap_or_else(|| "Untitled".to_owned())
}

/// A row of the section: a group's header, or one of its tabs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum EditorEntry {
    /// "Group N", numbered from 1 in layout order.
    Header(usize),
    View(EditorRow),
}

impl EditorEntry {
    pub(crate) fn row(&self) -> Option<&EditorRow> {
        match self {
            Self::Header(_) => None,
            Self::View(row) => Some(row),
        }
    }

    /// What stays the same while the row only changes its look: the header's number, or the
    /// view's group and document.
    pub(crate) fn key(&self) -> (Option<usize>, Option<(GroupId, DocumentId)>) {
        match self {
            Self::Header(number) => (Some(*number), None),
            Self::View(row) => (None, Some((row.group, row.id))),
        }
    }
}

fn editor_row(
    document: &Document,
    group: GroupId,
    active: Option<(GroupId, DocumentId)>,
) -> EditorRow {
    EditorRow {
        id: document.id,
        group,
        name: match &document.path {
            Some(path) => path.file_name().map_or_else(
                || path.display().to_string(),
                |name| name.to_string_lossy().into_owned(),
            ),
            None => unsaved_label(document),
        },
        path: document.path.clone(),
        dirty: document.dirty,
        active: Some((group, document.id)) == active,
    }
}

/// Every group's tabs in strip order, the groups in the order given (layout order). Headers only
/// with two or more groups, so one group looks as it always has.
pub(crate) fn entries(
    groups: &[(GroupId, Vec<&Document>)],
    active: Option<(GroupId, DocumentId)>,
) -> Vec<EditorEntry> {
    let headers = groups.len() > 1;
    let mut entries = Vec::new();
    for (number, (group, documents)) in groups.iter().enumerate() {
        if headers {
            entries.push(EditorEntry::Header(number + 1));
        }
        entries.extend(
            documents
                .iter()
                .map(|document| EditorEntry::View(editor_row(document, *group, active))),
        );
    }
    entries
}

/// The rows for the window's tabs now. Borrows the App on its own, never nested.
pub(crate) fn snapshot(hwnd: HWND) -> Vec<EditorEntry> {
    let order = crate::window::main_window::group_order(hwnd);
    unsafe { app_ptr(hwnd) }
        .map(|app| {
            let tabs = &unsafe { app.as_ref() }.tabs;
            let groups = order
                .iter()
                .map(|id| (*id, tabs.group_documents(*id)))
                .collect::<Vec<_>>();
            let active = tabs.active().map(|active| (tabs.active_group(), active.id));
            entries(&groups, active)
        })
        .unwrap_or_default()
}

/// What screen readers hear for a row (spec §7).
pub(crate) fn accessible_name(row: &EditorRow) -> String {
    let mut name = format!("{}, open editor", row.name);
    if row.dirty {
        name.push_str(", modified");
    }
    name
}

/// A row's tooltip: the file's full path, or an untitled tab's label.
pub(crate) fn tooltip(row: &EditorRow) -> String {
    row.path
        .as_ref()
        .map_or_else(|| row.name.clone(), |path| path.display().to_string())
}

/// The close box at a row's right edge.
pub(crate) fn close_rect(row: RECT, dpi: u32) -> RECT {
    RECT {
        left: (row.right - scale(CLOSE_BOX, dpi)).max(row.left),
        ..row
    }
}

/// The icon a row (and its drag label) shows.
pub(crate) fn tree_item(row: &EditorRow) -> TreeItem {
    TreeItem::Note(row.path.as_deref().map_or(NoteKind::Text, note_kind))
}

/// The section's rows and its own list state (scroll, hover, the active row as selected).
#[derive(Debug)]
pub(crate) struct OpenEditors {
    pub rows: Vec<EditorEntry>,
    pub list: RowListState,
    /// The pointer is over the hovered row's close box.
    pub hover_close: bool,
}

impl OpenEditors {
    pub(crate) fn new(row_height: i32) -> Self {
        Self {
            rows: Vec::new(),
            list: RowListState::new(row_height),
            hover_close: false,
        }
    }

    /// Takes `rows`; true when anything shown changed. The list's selection is the active row.
    pub(crate) fn set_rows(&mut self, rows: Vec<EditorEntry>) -> bool {
        if rows == self.rows {
            return false;
        }
        self.rows = rows;
        self.list.set_count(self.rows.len());
        self.list.selected = self.active_index();
        true
    }

    pub(crate) fn active_index(&self) -> Option<usize> {
        self.rows
            .iter()
            .position(|entry| entry.row().is_some_and(|row| row.active))
    }

    /// The tab at entry `index`; `None` for a header.
    pub(crate) fn row(&self, index: usize) -> Option<&EditorRow> {
        self.rows.get(index)?.row()
    }

    pub(crate) fn is_header(&self, index: usize) -> bool {
        matches!(self.rows.get(index), Some(EditorEntry::Header(_)))
    }

    /// How many tabs the section lists, headers left out.
    pub(crate) fn view_count(&self) -> usize {
        self.rows
            .iter()
            .filter(|entry| entry.row().is_some())
            .count()
    }
}

/// The label a header row shows and screen readers hear.
pub(crate) fn header_label(number: usize) -> String {
    format!("Group {number}")
}

/// Paints a group's header row, in the section header's style: no icon and no close box.
pub(crate) fn draw_header(dc: HDC, number: usize, rect: RECT, paint: &ViewPaint) {
    let text = RECT {
        left: (rect.left + scale(LEFT_PAD, paint.dpi)).min(rect.right),
        ..rect
    };
    unsafe {
        draw_text(
            dc,
            &header_label(number),
            text,
            paint.fonts.bold,
            paint.palette.muted_foreground,
            LINE,
        )
    };
}

/// Paints one row: the icon, the name, and at the right the dot of a dirty tab or, on hover or
/// selection, a clean tab's close box.
pub(crate) fn draw_editor_row(
    dc: HDC,
    row: &EditorRow,
    rect: RECT,
    look: RowLook,
    paint: &ViewPaint,
    images: &mut IconImages,
    close_hot: bool,
) {
    let (palette, dpi) = (&paint.palette, paint.dpi);
    let foreground = row_foreground(look, palette);
    let muted = if look.selected && look.focused {
        foreground
    } else {
        palette.muted_foreground
    };
    let px = scale(GLYPH_BOX, dpi);
    let icon = RECT {
        left: (rect.left + scale(LEFT_PAD, dpi)).min(rect.right),
        top: rect.top,
        right: (rect.left + scale(LEFT_PAD, dpi) + px).min(rect.right),
        bottom: rect.bottom,
    };
    draw_item_icon(
        dc,
        tree_item(row),
        icon,
        px,
        muted,
        palette,
        &paint.icons,
        images,
        paint.icon_set,
        paint.light_theme,
    );
    let close = close_rect(rect, dpi);
    let name = RECT {
        left: (icon.right + scale(GAP, dpi)).min(close.left),
        right: close.left,
        ..rect
    };
    unsafe { draw_text(dc, &row.name, name, paint.fonts.text, foreground, LINE) };
    if row.dirty {
        unsafe { draw_text(dc, DIRTY_DOT, close, paint.fonts.text, muted, CENTERED) };
    } else if look.hover || look.selected {
        let color = if close_hot { foreground } else { muted };
        unsafe { draw_text(dc, GLYPH_CLOSE, close, paint.fonts.glyph, color, CENTERED) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn document(id: u64, path: Option<&str>, dirty: bool) -> Document {
        let mut document = Document::test_fixture(DocumentId(id), dirty);
        document.path = path.map(PathBuf::from);
        document
    }

    #[test]
    fn a_row_per_tab_in_order_named_by_file_or_label() {
        // Break caught: rows out of strip order, a full path as the name, a blank name for an
        // untitled tab, or the wrong row marked active.
        let mut untitled = document(3, None, false);
        untitled.first_line_label = Some("Groceries".to_owned());
        let docs = [
            document(1, Some(r"C:\n\todo.md"), false),
            document(2, Some(r"D:\x\draft.txt"), true),
            untitled,
            document(4, None, false),
        ];
        let entries = entries(
            &[(GroupId(1), docs.iter().collect())],
            Some((GroupId(1), DocumentId(2))),
        );
        let rows: Vec<_> = entries.iter().filter_map(EditorEntry::row).collect();
        let names: Vec<_> = rows.iter().map(|row| row.name.as_str()).collect();
        assert_eq!(names, ["todo.md", "draft.txt", "Groceries", "Untitled"]);
        assert_eq!(
            rows.iter()
                .map(|row| (row.dirty, row.active))
                .collect::<Vec<_>>(),
            [(false, false), (true, true), (false, false), (false, false)]
        );
        assert_eq!(
            rows[1].path.as_deref(),
            Some(std::path::Path::new(r"D:\x\draft.txt"))
        );
    }

    #[test]
    fn several_groups_get_headers_and_one_group_stays_flat() {
        // Break caught: a "Group 1" header shown with a single group, or a document open in two
        // groups listed once.
        let a = document(1, Some("a"), false);
        let b = document(2, Some("b"), false);
        let one = entries(&[(GroupId(1), vec![&a, &b])], Some((GroupId(1), a.id)));
        assert!(
            one.iter()
                .all(|entry| matches!(entry, EditorEntry::View(_)))
        );
        let two = entries(
            &[(GroupId(1), vec![&a]), (GroupId(3), vec![&a, &b])],
            Some((GroupId(3), b.id)),
        );
        let shape: Vec<_> = two
            .iter()
            .map(|entry| match entry {
                EditorEntry::Header(number) => format!("G{number}"),
                EditorEntry::View(row) => {
                    format!("{}{}", row.name, if row.active { "*" } else { "" })
                }
            })
            .collect();
        assert_eq!(shape, vec!["G1", "a", "G2", "a", "b*"]);
    }

    #[test]
    fn names_tips_and_the_close_box() {
        let row = EditorRow {
            id: DocumentId(1),
            group: GroupId(1),
            name: "draft.txt".into(),
            path: Some(PathBuf::from(r"D:\x\draft.txt")),
            dirty: true,
            active: false,
        };
        assert_eq!(accessible_name(&row), "draft.txt, open editor, modified");
        assert_eq!(tooltip(&row), r"D:\x\draft.txt");
        let clean = EditorRow {
            dirty: false,
            path: None,
            ..row
        };
        assert_eq!(accessible_name(&clean), "draft.txt, open editor");
        assert_eq!(tooltip(&clean), "draft.txt");
        let rect = RECT {
            left: 0,
            top: 10,
            right: 200,
            bottom: 36,
        };
        let close = close_rect(rect, 96);
        assert_eq!((close.left, close.right, close.top), (176, 200, 10));
    }

    #[test]
    fn set_rows_reports_changes_and_selects_the_active_row() {
        let docs = [
            document(1, Some(r"C:\a.md"), false),
            document(2, Some(r"C:\b.md"), false),
        ];
        let rows = |active: u64| {
            entries(
                &[(GroupId(1), docs.iter().collect())],
                Some((GroupId(1), DocumentId(active))),
            )
        };
        let mut editors = OpenEditors::new(26);
        assert!(editors.set_rows(rows(2)));
        assert_eq!((editors.list.count, editors.list.selected), (2, Some(1)));
        assert!(!editors.set_rows(rows(2)));
        assert!(editors.set_rows(rows(1)));
        assert_eq!(editors.list.selected, Some(0));
    }
}
