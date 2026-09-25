//! The Notebook view's Open Editors section (open editors spec §3.2): one row per tab in
//! tab-strip order, with its type icon and name, a dot while it has unsaved changes and a close
//! box on hover. Built from the tab list in memory: no disk.

use crate::document::{Document, DocumentId};
use crate::window::file_icons::note_kind;
use crate::window::icon_sets::TreeItem;
use crate::window::icon_sets::images::IconImages;
use crate::window::main_window::app_ptr;
use crate::window::notebook_view::draw_item_icon;
use crate::window::panel::scale;
use crate::window::row_list::{RowListState, RowLook, row_foreground};
use crate::window::side_panel::{ViewPaint, draw_text};
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

/// One row per document, in the order given (the strip's).
pub(crate) fn editor_rows<'a>(
    documents: impl Iterator<Item = &'a Document>,
    active: Option<DocumentId>,
) -> Vec<EditorRow> {
    documents
        .map(|document| EditorRow {
            id: document.id,
            name: match &document.path {
                Some(path) => path.file_name().map_or_else(
                    || path.display().to_string(),
                    |name| name.to_string_lossy().into_owned(),
                ),
                None => unsaved_label(document),
            },
            path: document.path.clone(),
            dirty: document.dirty,
            active: Some(document.id) == active,
        })
        .collect()
}

/// The rows for the window's tabs now. Borrows the App on its own, never nested.
pub(crate) fn snapshot(hwnd: HWND) -> Vec<EditorRow> {
    unsafe { app_ptr(hwnd) }
        .map(|app| {
            let tabs = &unsafe { app.as_ref() }.tabs;
            editor_rows(tabs.documents(), tabs.active().map(|active| active.id))
        })
        .unwrap_or_default()
}

/// What screen readers hear for a row (spec §7).
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "wired to the panel by a later open editors task")
)]
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
    let extension = row
        .path
        .as_ref()
        .and_then(|path| path.extension())
        .map(|extension| extension.to_string_lossy());
    TreeItem::Note(note_kind(extension.as_deref()))
}

/// The section's rows and its own list state (scroll, hover, the active row as selected).
#[derive(Debug)]
pub(crate) struct OpenEditors {
    pub rows: Vec<EditorRow>,
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
    pub(crate) fn set_rows(&mut self, rows: Vec<EditorRow>) -> bool {
        if rows == self.rows {
            return false;
        }
        self.rows = rows;
        self.list.set_count(self.rows.len());
        self.list.selected = self.active_index();
        true
    }

    pub(crate) fn active_index(&self) -> Option<usize> {
        self.rows.iter().position(|row| row.active)
    }
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
        let rows = editor_rows(docs.iter(), Some(DocumentId(2)));
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
    fn names_tips_and_the_close_box() {
        let row = EditorRow {
            id: DocumentId(1),
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
        let mut editors = OpenEditors::new(26);
        assert!(editors.set_rows(editor_rows(docs.iter(), Some(DocumentId(2)))));
        assert_eq!((editors.list.count, editors.list.selected), (2, Some(1)));
        assert!(!editors.set_rows(editor_rows(docs.iter(), Some(DocumentId(2)))));
        assert!(editors.set_rows(editor_rows(docs.iter(), Some(DocumentId(1)))));
        assert_eq!(editors.list.selected, Some(0));
    }
}
