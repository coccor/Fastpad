//! The layout of the editor groups (split editors spec §4.3): rows and columns whose leaves are
//! groups. Pure: it computes rectangles and never touches a window.
#![cfg_attr(not(test), allow(dead_code))]

use super::titlebar::{Point, Rect, scale};
use crate::session::{SessionAxis, SessionLayout};

/// The sash between two groups, and the smallest a group may get, at 96 DPI (spec §4.3).
pub(crate) const SASH_96: i32 = 4;
pub(crate) const MIN_WIDTH_96: i32 = 160;
pub(crate) const MIN_HEIGHT_96: i32 = 100;

/// A group's identity for as long as it exists; never reused while the window lives.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct GroupId(pub(crate) u32);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Axis {
    /// Children side by side, left to right.
    Row,
    /// Children stacked, top to bottom.
    Column,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Direction {
    Left,
    Right,
    Up,
    Down,
}

impl Direction {
    pub(crate) const fn axis(self) -> Axis {
        match self {
            Self::Left | Self::Right => Axis::Row,
            Self::Up | Self::Down => Axis::Column,
        }
    }

    /// Left and up put the new group before the one split; right and down after it.
    const fn before(self) -> bool {
        matches!(self, Self::Left | Self::Up)
    }
}

/// A leaf is a group; a branch's children's shares sum to 1.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Node {
    Leaf(GroupId),
    Branch {
        axis: Axis,
        children: Vec<(Node, f32)>,
    },
}

/// A sash: between child `index` and `index + 1` of the branch reached by `path` (child indices
/// from the root).
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SashId {
    pub(crate) path: Vec<usize>,
    pub(crate) index: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Sash {
    pub(crate) id: SashId,
    /// The axis of its branch: a row's sashes are vertical bars dragged left and right.
    pub(crate) axis: Axis,
    pub(crate) rect: Rect,
    /// Its branch's rectangle, which a drag measures against.
    pub(crate) branch: Rect,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct TreeLayout {
    /// Every group's rectangle, in leaf order.
    pub(crate) groups: Vec<(GroupId, Rect)>,
    pub(crate) sashes: Vec<Sash>,
}

impl TreeLayout {
    pub(crate) fn rect_of(&self, id: GroupId) -> Option<Rect> {
        self.groups
            .iter()
            .find(|(group, _)| *group == id)
            .map(|(_, rect)| *rect)
    }

    pub(crate) fn sash_at(&self, x: i32, y: i32) -> Option<&Sash> {
        self.sashes
            .iter()
            .find(|sash| sash.rect.contains(Point::new(x, y)))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SplitTree {
    root: Node,
}

impl SplitTree {
    pub(crate) fn new(first: GroupId) -> Self {
        Self {
            root: Node::Leaf(first),
        }
    }

    /// The groups in numbering order: left to right in a row, top to bottom in a column
    /// (spec §4.3).
    pub(crate) fn leaves(&self) -> Vec<GroupId> {
        let mut leaves = Vec::new();
        collect_leaves(&self.root, &mut leaves);
        leaves
    }

    /// A group's number for Ctrl+1..8 and the Open Editors headers, from 1.
    pub(crate) fn number_of(&self, id: GroupId) -> Option<usize> {
        self.leaves()
            .iter()
            .position(|group| *group == id)
            .map(|index| index + 1)
    }

    /// Puts `new` beside `target` (spec §4.3). False when `target` isn't in the tree.
    pub(crate) fn split(&mut self, target: GroupId, direction: Direction, new: GroupId) -> bool {
        split_node(&mut self.root, target, direction, new)
    }

    /// Whether splitting `target` would leave both halves at least the minimum size.
    pub(crate) fn fits_split(
        &self,
        target: GroupId,
        direction: Direction,
        new: GroupId,
        area: Rect,
        dpi: u32,
    ) -> bool {
        let mut trial = self.clone();
        if !trial.split(target, direction, new) {
            return false;
        }
        let layout = trial.layout(area, dpi);
        let (width, height) = (scale(MIN_WIDTH_96, dpi), scale(MIN_HEIGHT_96, dpi));
        [target, new].iter().all(|id| {
            layout.rect_of(*id).is_some_and(|rect| {
                rect.right - rect.left >= width && rect.bottom - rect.top >= height
            })
        })
    }

    /// Removes `id`, giving its share to its siblings. The last group can't be removed.
    pub(crate) fn remove(&mut self, id: GroupId) -> bool {
        if self.root == Node::Leaf(id) || !remove_leaf(&mut self.root, id) {
            return false;
        }
        collapse(&mut self.root);
        true
    }

    pub(crate) fn layout(&self, area: Rect, dpi: u32) -> TreeLayout {
        let mut layout = TreeLayout::default();
        lay_out(
            &self.root,
            area,
            scale(SASH_96, dpi),
            &mut Vec::new(),
            &mut layout,
        );
        layout
    }

    /// Moves `sash` so its centre is at `at` (x for a row, y for a column), keeping both groups
    /// beside it at least the minimum size.
    pub(crate) fn set_sash(&mut self, sash: &Sash, at: i32, dpi: u32) -> bool {
        let sash_width = scale(SASH_96, dpi);
        let Some(Node::Branch { axis, children }) = node_at_mut(&mut self.root, &sash.id.path)
        else {
            return false;
        };
        let index = sash.id.index;
        if index + 1 >= children.len() {
            return false;
        }
        let (start, end) = extent(*axis, sash.branch);
        let gaps = sash_width * (children.len() as i32 - 1);
        let available = (end - start - gaps).max(1) as f32;
        let before: f32 = children[..index].iter().map(|(_, share)| share).sum();
        let pair = children[index].1 + children[index + 1].1;
        let first_start = start as f32 + before * available + (sash_width * index as i32) as f32;
        let pair_pixels = pair * available;
        let min_first = min_extent(&children[index].0, *axis, dpi) as f32;
        let min_second = min_extent(&children[index + 1].0, *axis, dpi) as f32;
        let first = if pair_pixels < min_first + min_second {
            pair_pixels * children[index].1 / pair
        } else {
            let wanted = at as f32 - (sash_width / 2) as f32 - first_start;
            wanted.clamp(min_first, pair_pixels - min_second)
        };
        children[index].1 = first / available;
        children[index + 1].1 = pair - children[index].1;
        true
    }

    /// Gives every child of the sash's branch the same share (a sash double-click).
    pub(crate) fn equalize(&mut self, sash: &SashId) -> bool {
        let Some(Node::Branch { children, .. }) = node_at_mut(&mut self.root, &sash.path) else {
            return false;
        };
        let share = 1.0 / children.len() as f32;
        for (_, child) in children.iter_mut() {
            *child = share;
        }
        true
    }

    pub(crate) fn to_session(&self, number: &dyn Fn(GroupId) -> usize) -> SessionLayout {
        session_node(&self.root, number)
    }

    /// The tree a saved layout describes, with `group` mapping each saved number to a live group.
    /// `None` when a number has no group.
    pub(crate) fn from_session(
        layout: &SessionLayout,
        group: &dyn Fn(usize) -> Option<GroupId>,
    ) -> Option<Self> {
        let mut root = tree_node(layout, group)?;
        collapse(&mut root);
        Some(Self { root })
    }
}

fn collect_leaves(node: &Node, leaves: &mut Vec<GroupId>) {
    match node {
        Node::Leaf(id) => leaves.push(*id),
        Node::Branch { children, .. } => {
            for (child, _) in children {
                collect_leaves(child, leaves);
            }
        }
    }
}

fn split_node(node: &mut Node, target: GroupId, direction: Direction, new: GroupId) -> bool {
    match node {
        Node::Leaf(id) if *id == target => {
            let pair = if direction.before() {
                vec![(Node::Leaf(new), 0.5), (Node::Leaf(target), 0.5)]
            } else {
                vec![(Node::Leaf(target), 0.5), (Node::Leaf(new), 0.5)]
            };
            *node = Node::Branch {
                axis: direction.axis(),
                children: pair,
            };
            true
        }
        Node::Leaf(_) => false,
        Node::Branch { axis, children } => {
            if *axis == direction.axis()
                && let Some(index) = children
                    .iter()
                    .position(|(child, _)| *child == Node::Leaf(target))
            {
                let half = children[index].1 / 2.0;
                children[index].1 = half;
                let at = if direction.before() { index } else { index + 1 };
                children.insert(at, (Node::Leaf(new), half));
                return true;
            }
            children
                .iter_mut()
                .any(|(child, _)| split_node(child, target, direction, new))
        }
    }
}

fn remove_leaf(node: &mut Node, id: GroupId) -> bool {
    let Node::Branch { children, .. } = node else {
        return false;
    };
    if let Some(index) = children
        .iter()
        .position(|(child, _)| *child == Node::Leaf(id))
    {
        children.remove(index);
        normalize(children);
        return true;
    }
    children.iter_mut().any(|(child, _)| remove_leaf(child, id))
}

/// Replaces one-child branches by their child and flattens a branch into a parent on the same
/// axis, bottom up.
fn collapse(node: &mut Node) {
    let Node::Branch { axis, children } = node else {
        return;
    };
    for (child, _) in children.iter_mut() {
        collapse(child);
    }
    let axis = *axis;
    let mut flat = Vec::with_capacity(children.len());
    for (child, share) in children.drain(..) {
        match child {
            Node::Branch {
                axis: inner,
                children: grandchildren,
            } if inner == axis => flat.extend(
                grandchildren
                    .into_iter()
                    .map(|(grandchild, part)| (grandchild, part * share)),
            ),
            other => flat.push((other, share)),
        }
    }
    *children = flat;
    if children.len() == 1
        && let Some((only, _)) = children.pop()
    {
        *node = only;
    }
}

fn normalize(children: &mut [(Node, f32)]) {
    let sum: f32 = children.iter().map(|(_, share)| share).sum();
    if sum > 0.0 {
        for (_, share) in children.iter_mut() {
            *share /= sum;
        }
    }
}

fn extent(axis: Axis, rect: Rect) -> (i32, i32) {
    match axis {
        Axis::Row => (rect.left, rect.right),
        Axis::Column => (rect.top, rect.bottom),
    }
}

fn along(axis: Axis, rect: Rect, from: i32, to: i32) -> Rect {
    match axis {
        Axis::Row => Rect::new(from, rect.top, to, rect.bottom),
        Axis::Column => Rect::new(rect.left, from, rect.right, to),
    }
}

fn lay_out(node: &Node, rect: Rect, sash: i32, path: &mut Vec<usize>, layout: &mut TreeLayout) {
    let Node::Branch { axis, children } = node else {
        if let Node::Leaf(id) = node {
            layout.groups.push((*id, rect));
        }
        return;
    };
    let (start, end) = extent(*axis, rect);
    let count = children.len() as i32;
    let available = (end - start - sash * (count - 1)).max(0);
    let mut cumulative = 0.0_f32;
    let mut offset = start;
    for (index, (child, share)) in children.iter().enumerate() {
        cumulative += share;
        let last = index + 1 == children.len();
        let child_end = if last {
            end
        } else {
            (start + (available as f32 * cumulative).round() as i32 + sash * index as i32)
                .max(offset)
        };
        path.push(index);
        lay_out(
            child,
            along(*axis, rect, offset, child_end),
            sash,
            path,
            layout,
        );
        path.pop();
        if !last {
            layout.sashes.push(Sash {
                id: SashId {
                    path: path.clone(),
                    index,
                },
                axis: *axis,
                rect: along(*axis, rect, child_end, child_end + sash),
                branch: rect,
            });
            offset = child_end + sash;
        }
    }
}

/// The least room `node` needs along `axis`.
fn min_extent(node: &Node, axis: Axis, dpi: u32) -> i32 {
    match node {
        Node::Leaf(_) => match axis {
            Axis::Row => scale(MIN_WIDTH_96, dpi),
            Axis::Column => scale(MIN_HEIGHT_96, dpi),
        },
        Node::Branch {
            axis: inner,
            children,
        } => {
            let mins = children
                .iter()
                .map(|(child, _)| min_extent(child, axis, dpi));
            if *inner == axis {
                mins.sum::<i32>() + scale(SASH_96, dpi) * (children.len() as i32 - 1)
            } else {
                mins.max().unwrap_or(0)
            }
        }
    }
}

fn node_at_mut<'a>(node: &'a mut Node, path: &[usize]) -> Option<&'a mut Node> {
    let Some((first, rest)) = path.split_first() else {
        return Some(node);
    };
    match node {
        Node::Branch { children, .. } => node_at_mut(&mut children.get_mut(*first)?.0, rest),
        Node::Leaf(_) => None,
    }
}

fn session_node(node: &Node, number: &dyn Fn(GroupId) -> usize) -> SessionLayout {
    match node {
        Node::Leaf(id) => SessionLayout::Leaf(number(*id)),
        Node::Branch { axis, children } => SessionLayout::Branch {
            axis: match axis {
                Axis::Row => SessionAxis::Row,
                Axis::Column => SessionAxis::Column,
            },
            children: children
                .iter()
                .map(|(child, share)| (session_node(child, number), *share))
                .collect(),
        },
    }
}

fn tree_node(layout: &SessionLayout, group: &dyn Fn(usize) -> Option<GroupId>) -> Option<Node> {
    Some(match layout {
        SessionLayout::Leaf(number) => Node::Leaf(group(*number)?),
        SessionLayout::Branch { axis, children } => Node::Branch {
            axis: match axis {
                SessionAxis::Row => Axis::Row,
                SessionAxis::Column => Axis::Column,
            },
            children: children
                .iter()
                .map(|(child, share)| Some((tree_node(child, group)?, *share)))
                .collect::<Option<Vec<_>>>()?,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: GroupId = GroupId(1);
    const B: GroupId = GroupId(2);
    const C: GroupId = GroupId(3);
    const D: GroupId = GroupId(4);

    fn area() -> Rect {
        Rect::new(0, 0, 1000, 600)
    }

    #[test]
    fn a_split_on_a_new_axis_wraps_the_leaf_in_a_half_and_half_branch() {
        // Break caught: Split Right turning the whole tree into a row, or the new group landing
        // on the wrong side.
        let mut tree = SplitTree::new(A);
        assert!(tree.split(A, Direction::Right, B));
        assert_eq!(tree.leaves(), vec![A, B]);
        let mut left = SplitTree::new(A);
        assert!(left.split(A, Direction::Left, B));
        assert_eq!(left.leaves(), vec![B, A]);
        let mut down = SplitTree::new(A);
        assert!(down.split(A, Direction::Down, B));
        assert_eq!(
            down.root,
            Node::Branch {
                axis: Axis::Column,
                children: vec![(Node::Leaf(A), 0.5), (Node::Leaf(B), 0.5)],
            }
        );
    }

    #[test]
    fn a_split_on_the_parents_axis_halves_only_the_split_groups_share() {
        // Break caught: a third column resetting every column to a third, or nesting a row in a
        // row.
        let mut tree = SplitTree::new(A);
        tree.split(A, Direction::Right, B);
        tree.split(B, Direction::Right, C);
        assert_eq!(
            tree.root,
            Node::Branch {
                axis: Axis::Row,
                children: vec![
                    (Node::Leaf(A), 0.5),
                    (Node::Leaf(B), 0.25),
                    (Node::Leaf(C), 0.25)
                ],
            }
        );
        tree.split(B, Direction::Up, D);
        assert_eq!(tree.leaves(), vec![A, D, B, C]);
        assert_eq!(tree.number_of(D), Some(2));
        assert!(!tree.split(GroupId(9), Direction::Right, GroupId(10)));
    }

    #[test]
    fn removing_a_group_gives_its_share_away_and_collapses_and_flattens() {
        // Break caught: a closed group leaving a gap, a one-child branch left in the tree, or a
        // row nested directly in a row after a collapse.
        let mut tree = SplitTree::new(A);
        tree.split(A, Direction::Right, B);
        tree.split(B, Direction::Down, C);
        tree.split(C, Direction::Right, D);
        // row(A, column(B, row(C, D)))
        assert!(tree.remove(B));
        // row(A, row(C, D)) flattens into row(A, C, D).
        assert_eq!(
            tree.root,
            Node::Branch {
                axis: Axis::Row,
                children: vec![
                    (Node::Leaf(A), 0.5),
                    (Node::Leaf(C), 0.25),
                    (Node::Leaf(D), 0.25)
                ],
            }
        );
        assert!(tree.remove(C));
        assert!(tree.remove(D));
        assert_eq!(tree.root, Node::Leaf(A));
        assert!(!tree.remove(A), "the last group stays");
    }

    #[test]
    fn layout_places_groups_and_sashes_at_each_dpi() {
        // Break caught: rounding that leaves a pixel gap, overlaps a sash, or forgets to scale.
        for dpi in [96, 144, 192] {
            let mut tree = SplitTree::new(A);
            tree.split(A, Direction::Right, B);
            tree.split(B, Direction::Down, C);
            let layout = tree.layout(area(), dpi);
            let sash = scale(SASH_96, dpi);
            let a = layout.rect_of(A).unwrap();
            let b = layout.rect_of(B).unwrap();
            let c = layout.rect_of(C).unwrap();
            assert_eq!(a.left, 0);
            assert_eq!(b.left, a.right + sash);
            assert_eq!(b.right, 1000);
            assert_eq!(c.top, b.bottom + sash);
            assert_eq!(c.bottom, 600);
            assert_eq!(layout.sashes.len(), 2);
            let row = layout
                .sashes
                .iter()
                .find(|sash| sash.axis == Axis::Row)
                .unwrap();
            assert_eq!(row.axis, Axis::Row);
            assert_eq!(row.rect, Rect::new(a.right, 0, b.left, 600));
            assert!(layout.sash_at(a.right, 300).is_some());
            assert!(layout.sash_at(a.right - 1, 300).is_none());
        }
    }

    #[test]
    fn a_sash_drag_stops_at_the_minimum_group_size() {
        // Break caught: a sash dragged past the edge giving a group zero or negative width.
        let mut tree = SplitTree::new(A);
        tree.split(A, Direction::Right, B);
        let sash = tree.layout(area(), 96).sashes[0].clone();
        assert!(tree.set_sash(&sash, -500, 96));
        let layout = tree.layout(area(), 96);
        assert_eq!(layout.rect_of(A).unwrap().right, MIN_WIDTH_96);
        assert!(tree.set_sash(&sash, 5000, 96));
        let layout = tree.layout(area(), 96);
        assert_eq!(1000 - layout.rect_of(B).unwrap().left, MIN_WIDTH_96);
        assert!(tree.set_sash(&sash, 400, 96));
        let a = tree.layout(area(), 96).rect_of(A).unwrap();
        assert_eq!(a.right, 400 - SASH_96 / 2);
    }

    #[test]
    fn a_double_clicked_sash_equalizes_its_branch() {
        // Break caught: equalizing the whole tree, or only the two groups beside the sash.
        let mut tree = SplitTree::new(A);
        tree.split(A, Direction::Right, B);
        tree.split(B, Direction::Right, C);
        let sash = tree.layout(area(), 96).sashes[0].id.clone();
        assert!(tree.equalize(&sash));
        let Node::Branch { children, .. } = &tree.root else {
            panic!("a row")
        };
        for (_, share) in children {
            assert!((share - 1.0 / 3.0).abs() < 1e-6);
        }
    }

    #[test]
    fn a_split_that_would_not_fit_is_refused() {
        // Break caught: a split into groups narrower than the minimum, which then can't be
        // dragged wider.
        let tree = SplitTree::new(A);
        assert!(tree.fits_split(A, Direction::Right, B, Rect::new(0, 0, 400, 600), 96));
        assert!(!tree.fits_split(A, Direction::Right, B, Rect::new(0, 0, 300, 600), 96));
        assert!(!tree.fits_split(A, Direction::Down, B, Rect::new(0, 0, 800, 190), 96));
        assert!(!tree.fits_split(A, Direction::Right, B, Rect::new(0, 0, 400, 600), 192));
    }

    #[test]
    fn the_tree_round_trips_through_the_session_layout() {
        // Break caught: groups renumbered or ratios lost across a restart.
        let mut tree = SplitTree::new(A);
        tree.split(A, Direction::Right, B);
        tree.split(B, Direction::Down, C);
        let number = |id: GroupId| tree.number_of(id).unwrap();
        let saved = tree.to_session(&number);
        assert_eq!(saved.encode(), "row(1:0.5,column(2:0.5,3:0.5):0.5)");
        let ids = [A, B, C];
        let back = SplitTree::from_session(&saved, &|n| ids.get(n - 1).copied()).unwrap();
        assert_eq!(back, tree);
        assert!(SplitTree::from_session(&saved, &|n| (n < 3).then(|| ids[n - 1])).is_none());
    }
}
