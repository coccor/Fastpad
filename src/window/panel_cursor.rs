//! The Notebook view's one keyboard selection (open editors spec §3.5): Up and Down run from
//! the Open Editors header through its rows, the notebook's root row and the tree's rows, as
//! one list. Pure.

use crate::window::row_list::ListKey;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Cursor {
    EditorsHeader,
    Editor(usize),
    Root,
    /// The tree's own selection (`RowListState::selected`) is the row.
    Tree,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Shape {
    pub editors: usize,
    pub root: bool,
    pub tree: usize,
}

fn last(shape: Shape) -> usize {
    shape.editors + usize::from(shape.root) + shape.tree
}

fn index_of(cursor: Cursor, tree_selected: Option<usize>, shape: Shape) -> usize {
    match cursor {
        Cursor::EditorsHeader => 0,
        Cursor::Editor(index) => 1 + index.min(shape.editors.saturating_sub(1)),
        Cursor::Root if shape.root => shape.editors + 1,
        Cursor::Root => shape.editors,
        Cursor::Tree if shape.tree > 0 => {
            let selected = tree_selected.unwrap_or(0).min(shape.tree - 1);
            shape.editors + usize::from(shape.root) + 1 + selected
        }
        Cursor::Tree => last(shape),
    }
}

fn cursor_at(index: usize, shape: Shape) -> (Cursor, Option<usize>) {
    if index == 0 {
        return (Cursor::EditorsHeader, None);
    }
    if index <= shape.editors {
        return (Cursor::Editor(index - 1), None);
    }
    let after_editors = index - shape.editors - 1;
    if shape.root && after_editors == 0 {
        return (Cursor::Root, None);
    }
    (Cursor::Tree, Some(after_editors - usize::from(shape.root)))
}

/// Where `key` moves the selection. `None` for a page key: the section's own list pages.
pub(crate) fn step(
    cursor: Cursor,
    tree_selected: Option<usize>,
    key: ListKey,
    shape: Shape,
) -> Option<(Cursor, Option<usize>)> {
    let now = index_of(cursor, tree_selected, shape);
    let target = match key {
        ListKey::Up => now.saturating_sub(1),
        ListKey::Down => (now + 1).min(last(shape)),
        ListKey::Home => 0,
        ListKey::End => last(shape),
        ListKey::PageUp | ListKey::PageDown => return None,
    };
    Some(cursor_at(target, shape))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHAPE: Shape = Shape {
        editors: 2,
        root: true,
        tree: 3,
    };

    #[test]
    fn up_and_down_cross_from_open_editors_through_the_root_into_the_tree() {
        // Break caught: Down stuck at the last tab, the root row skipped, or Up from the first
        // tree row jumping to the top (open editors spec §3.5).
        let down = |cursor, tree| step(cursor, tree, ListKey::Down, SHAPE).unwrap();
        assert_eq!(down(Cursor::EditorsHeader, None), (Cursor::Editor(0), None));
        assert_eq!(down(Cursor::Editor(1), None), (Cursor::Root, None));
        assert_eq!(down(Cursor::Root, None), (Cursor::Tree, Some(0)));
        assert_eq!(
            down(Cursor::Tree, Some(2)),
            (Cursor::Tree, Some(2)),
            "stays on the last"
        );
        let up = |cursor, tree| step(cursor, tree, ListKey::Up, SHAPE).unwrap();
        assert_eq!(up(Cursor::Tree, Some(0)), (Cursor::Root, None));
        assert_eq!(up(Cursor::Root, None), (Cursor::Editor(1), None));
        assert_eq!(
            up(Cursor::EditorsHeader, None),
            (Cursor::EditorsHeader, None)
        );
    }

    #[test]
    fn home_end_and_collapsed_sections() {
        assert_eq!(
            step(Cursor::Root, None, ListKey::End, SHAPE),
            Some((Cursor::Tree, Some(2)))
        );
        assert_eq!(
            step(Cursor::Tree, Some(1), ListKey::Home, SHAPE),
            Some((Cursor::EditorsHeader, None))
        );
        assert_eq!(step(Cursor::Tree, Some(1), ListKey::PageDown, SHAPE), None);
        let collapsed = Shape {
            editors: 0,
            root: true,
            tree: 0,
        };
        assert_eq!(
            step(Cursor::EditorsHeader, None, ListKey::Down, collapsed),
            Some((Cursor::Root, None))
        );
        assert_eq!(
            step(Cursor::Root, None, ListKey::Down, collapsed),
            Some((Cursor::Root, None))
        );
        let no_notebook = Shape {
            editors: 1,
            root: false,
            tree: 0,
        };
        assert_eq!(
            step(Cursor::Editor(0), None, ListKey::Down, no_notebook),
            Some((Cursor::Editor(0), None))
        );
    }
}
