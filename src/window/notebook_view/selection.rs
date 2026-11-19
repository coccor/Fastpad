//! The Notebook tree's multi-selection: which note and folder rows are ticked, in one notebook.
//! Pure state with no window, so the rules are testable on their own.

// Later tasks wire the model into the tree view; until then only the tests use it.
#![allow(dead_code)]

use crate::library::tree::{RowKind, ancestors};
use crate::window::library_host::NotebookId;

/// Whether a row can be part of a multi-selection: notes and folders only. Roots, drafts and
/// placeholders never are.
pub(crate) fn selectable(kind: &RowKind) -> bool {
    matches!(kind, RowKind::Note(_) | RowKind::Folder(_))
}

/// The ticked rows of one notebook. Fewer than two items is "no multi-selection"; the tree's own
/// cursor row stands for a single one.
#[derive(Debug, Default)]
pub(crate) struct TreeSelection {
    notebook: Option<NotebookId>,
    items: Vec<RowKind>,
    anchor: Option<RowKind>,
}

impl TreeSelection {
    /// Two or more rows are selected.
    pub(crate) fn is_multi(&self) -> bool {
        self.items.len() >= 2
    }

    /// Whether `kind` of notebook `id` is selected.
    pub(crate) fn contains(&self, id: NotebookId, kind: &RowKind) -> bool {
        self.notebook == Some(id) && self.items.contains(kind)
    }

    /// The notebook the selection lives in.
    pub(crate) fn notebook(&self) -> Option<NotebookId> {
        self.notebook
    }

    /// Drops the selection and its anchor.
    pub(crate) fn clear(&mut self) {
        *self = Self::default();
    }

    /// Ctrl+click on `kind`. The first toggle also takes the cursor row; a toggle in another
    /// notebook starts over with just `kind`. Toggling a selected row takes it out, and a lone
    /// leftover is no selection at all.
    pub(crate) fn toggle(
        &mut self,
        id: NotebookId,
        kind: &RowKind,
        cursor: Option<(NotebookId, RowKind)>,
    ) {
        if !selectable(kind) {
            return;
        }
        if self.notebook != Some(id) {
            self.clear();
            self.notebook = Some(id);
        }
        if self.items.is_empty()
            && let Some((cursor_id, cursor_kind)) = cursor
            && cursor_id == id
            && selectable(&cursor_kind)
            && cursor_kind != *kind
        {
            self.items.push(cursor_kind);
        }
        if let Some(at) = self.items.iter().position(|item| item == kind) {
            self.items.remove(at);
            if self.items.len() < 2 {
                self.items.clear();
            }
        } else {
            self.items.push(kind.clone());
        }
        self.anchor = Some(kind.clone());
    }

    /// Shift+click on `rows[to]`: selects the selectable rows of notebook `id` from the anchor to
    /// `to`, replacing the items. The anchor is the existing one, else the cursor row (an index
    /// into `rows`), else `to`, and it stays put.
    pub(crate) fn select_range(
        &mut self,
        rows: &[(NotebookId, RowKind)],
        id: NotebookId,
        to: usize,
        cursor: Option<usize>,
    ) {
        if to >= rows.len() {
            return;
        }
        if self.notebook != Some(id) {
            self.clear();
        }
        let anchor_at = self
            .anchor
            .as_ref()
            .and_then(|anchor| rows.iter().position(|(rid, k)| *rid == id && k == anchor))
            .or_else(|| {
                cursor.filter(|&at| {
                    rows.get(at)
                        .is_some_and(|(rid, k)| *rid == id && selectable(k))
                })
            })
            .unwrap_or(to);
        if self.anchor.is_none() {
            self.anchor = Some(rows[anchor_at].1.clone());
        }
        let (low, high) = (anchor_at.min(to), anchor_at.max(to));
        self.notebook = Some(id);
        self.items = rows[low..=high]
            .iter()
            .filter(|(rid, k)| *rid == id && selectable(k))
            .map(|(_, k)| k.clone())
            .collect();
    }

    /// Ctrl+A: every selectable row of notebook `id` that is shown.
    pub(crate) fn select_all(&mut self, rows: &[(NotebookId, RowKind)], id: NotebookId) {
        self.items = rows
            .iter()
            .filter(|(rid, k)| *rid == id && selectable(k))
            .map(|(_, k)| k.clone())
            .collect();
        self.notebook = Some(id);
        self.anchor = self.items.first().cloned();
        if self.items.len() < 2 {
            self.clear();
        }
    }

    /// After a rebuild: drops items (and the anchor) that are neither shown in `rows` nor `known`
    /// (still listed in the library, as under a collapsed folder). One item left is no selection.
    pub(crate) fn retain_present(
        &mut self,
        rows: &[(NotebookId, RowKind)],
        known: impl Fn(&RowKind) -> bool,
    ) {
        let Some(id) = self.notebook else { return };
        let present =
            |kind: &RowKind| rows.iter().any(|(rid, k)| *rid == id && k == kind) || known(kind);
        self.items.retain(|item| present(item));
        if self.anchor.as_ref().is_some_and(|anchor| !present(anchor)) {
            self.anchor = None;
        }
        if self.items.len() < 2 {
            self.clear();
        }
    }

    /// The items an action applies to: those not inside another selected folder, so moving a
    /// folder and its contents moves each once.
    pub(crate) fn effective(&self) -> Vec<RowKind> {
        let folders: Vec<&std::path::PathBuf> = self
            .items
            .iter()
            .filter_map(|item| match item {
                RowKind::Folder(path) => Some(path),
                _ => None,
            })
            .collect();
        self.items
            .iter()
            .filter(|item| {
                let path = match item {
                    RowKind::Folder(path) | RowKind::Note(path) => path,
                    _ => return true,
                };
                !ancestors(path)
                    .iter()
                    .any(|ancestor| folders.contains(&ancestor))
            })
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const NB: NotebookId = NotebookId(1);
    const OTHER: NotebookId = NotebookId(2);

    fn note(path: &str) -> RowKind {
        RowKind::Note(PathBuf::from(path))
    }

    fn folder(path: &str) -> RowKind {
        RowKind::Folder(PathBuf::from(path))
    }

    #[test]
    fn a_first_ctrl_toggle_takes_the_cursor_row_too() {
        let mut selection = TreeSelection::default();
        selection.toggle(NB, &note("b.md"), Some((NB, note("a.md"))));
        assert_eq!(selection.items, vec![note("a.md"), note("b.md")]);
        assert!(selection.is_multi());
        assert!(selection.contains(NB, &note("a.md")));
        assert_eq!(selection.notebook(), Some(NB));
    }

    #[test]
    fn toggling_a_selected_row_takes_it_out_and_one_left_is_no_selection() {
        let mut selection = TreeSelection::default();
        selection.toggle(NB, &note("b.md"), Some((NB, note("a.md"))));
        selection.toggle(NB, &note("b.md"), None);
        assert!(!selection.is_multi());
        assert!(selection.items.is_empty());
    }

    #[test]
    fn a_toggle_in_another_notebook_restarts_there() {
        let mut selection = TreeSelection::default();
        selection.toggle(NB, &note("b.md"), Some((NB, note("a.md"))));
        selection.toggle(OTHER, &note("c.md"), Some((NB, note("a.md"))));
        assert_eq!(selection.items, vec![note("c.md")]);
        assert_eq!(selection.notebook(), Some(OTHER));
        assert!(!selection.contains(NB, &note("a.md")));
    }

    #[test]
    fn a_range_selects_the_rows_between_anchor_and_target_of_that_notebook_only() {
        let rows = vec![
            (NB, RowKind::Root),
            (NB, folder("x")),
            (NB, note("x\\a.md")),
            (OTHER, note("zz.md")),
            (NB, note("b.md")),
            (OTHER, RowKind::Root),
            (NB, note("c.md")),
        ];
        let mut selection = TreeSelection::default();
        selection.toggle(NB, &folder("x"), None);
        selection.select_range(&rows, NB, 4, None);
        assert_eq!(
            selection.items,
            vec![folder("x"), note("x\\a.md"), note("b.md")]
        );
        assert_eq!(selection.anchor, Some(folder("x")));
        // Without an anchor the cursor row starts the range.
        let mut fresh = TreeSelection::default();
        fresh.select_range(&rows, NB, 4, Some(2));
        assert_eq!(fresh.items, vec![note("x\\a.md"), note("b.md")]);
    }

    #[test]
    fn roots_drafts_and_placeholders_are_never_selected() {
        let rows = vec![
            (NB, RowKind::Root),
            (NB, note("a.md")),
            (NB, RowKind::Draft),
            (NB, note("b.md")),
        ];
        let mut selection = TreeSelection::default();
        selection.select_all(&rows, NB);
        assert_eq!(selection.items, vec![note("a.md"), note("b.md")]);
        selection.toggle(NB, &RowKind::Root, None);
        assert_eq!(selection.items, vec![note("a.md"), note("b.md")]);
        assert!(!selectable(&RowKind::Draft));
        assert!(!selectable(&RowKind::Root));
    }

    #[test]
    fn retain_drops_items_that_are_gone_but_keeps_hidden_ones() {
        let rows = vec![(NB, RowKind::Root), (NB, note("a.md"))];
        let mut selection = TreeSelection::default();
        selection.toggle(NB, &note("hidden.md"), Some((NB, note("a.md"))));
        selection.toggle(NB, &note("gone.md"), None);
        selection.retain_present(&rows, |kind| *kind == note("hidden.md"));
        assert_eq!(selection.items, vec![note("a.md"), note("hidden.md")]);
        selection.retain_present(&rows, |_| false);
        assert!(selection.items.is_empty());
        assert_eq!(selection.notebook(), None);
    }

    #[test]
    fn effective_drops_items_inside_a_selected_folder() {
        let selection = TreeSelection {
            notebook: Some(NB),
            items: vec![folder("x"), note("x\\a.md"), note("b.md")],
            anchor: None,
        };
        assert_eq!(selection.effective(), vec![folder("x"), note("b.md")]);
    }
}
