# Split editors, PR 2 (splits): implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The editor area becomes a grid of editor groups: Split Right and Split Down, draggable sashes, the same document shown in several groups, per-group find bar and preview, group commands and keys, Open Editors group headers, group-aware Ctrl+P, and sessions that bring the layout back.

**Architecture:**
- A pure `SplitTree` (`src/window/split_tree.rs`) holds the layout: rows and columns whose leaves are `GroupId`s. It computes each group's rectangle and each sash.
- `Tabs` stays the one façade over the documents (spec §3.3 amended, see below). It keeps the `DocumentStore` and gains a list of per-group view lists (`GroupTabs`) and the active group. Every existing `Tabs` method keeps its meaning for the **active group**, so its ~130 call sites stay as they are. New methods name a group explicitly.
- `App.group: Option<GroupWindow>` becomes `App.groups: Vec<GroupWindow>`. Each `GroupWindow` owns its Scintilla editor, find bar, preview host and image host. `App.editor`, `App.find_bar`, `App.preview` and `App.image` go away. Accessors on `App` return the active group's.
- Commands keep one routing path: they act on the active group. Anything that happens in a group (a click, focus arriving in its editor, find bar or preview) makes that group active first.
- The main window lays the groups out with the split tree, owns the sashes, and paints them.

**Tech Stack:** Rust 2024 edition, `windows-sys`, Scintilla 5.6.6 plus Lexilla 5.5.3 (`native/`), MSAA `IAccessible`.

**Spec:** `docs/superpowers/specs/2026-09-28-split-editors-design.md`. This plan implements its §10 item 2. It is stacked on PR #28 (`feat/split-editors`), on branch `feat/split-editors-grid`. That branch already carries one commit: Enter in the notes tree and in search results opens a normal tab.

## Global Constraints

- **Latency:** nothing new runs before first paint. At start-up there is one group, created where it is today. Extra groups are created only by a command, a key or a session restore, which runs in the deferred chain after first paint.
- **One group looks like 0.2.0.** With one group: no sash, no group headers in Open Editors, no "Group N" in Ctrl+P, and no active-tab accent.
- **Scintilla owns the text.** Two groups showing one document share one Scintilla document (`SCI_SETDOCPOINTER`). Never copy text between views.
- **No new crates.** A new `windows-sys` feature must be added to `tools/audit-dependencies.ps1`'s allowlist in the same commit.
- **Compile gate:** `cargo clippy --all-targets --all-features -- -D warnings`.
- **Targeted tests:** `cargo test --lib -- <filter> --test-threads=1`. Window tests register window classes and must run serially.
- **The full suite runs once, at the end:** `cargo test --all-targets --no-fail-fast -- --test-threads=1`. It needs the display awake (window tests wait for paints).
- **Live runs** back up and restore `%LOCALAPPDATA%\FastPad` (fastpad.ini, session.ini, Recovery) around them.
- **Numbers from the spec:** sash 4 px at 96 DPI; minimum group 160×100 px at 96 DPI; both scaled with `titlebar::scale`.
- **Session keys** (spec §8): `version=2`, `layout=`, `active_group=`, `group=<n>|active=<i>`, then `file=` / `snapshot=` lines `<caret>|<anchor>|<firstline>|<target>`.
- **Comments and test style:** match the surrounding code. Each test starts with a `// Break caught: …` comment naming the regression it guards against.
- Commit prefixes `feat(split-editors): …`, `refactor(split-editors): …`, `test(split-editors): …`, `fix(split-editors): …`. No attribution lines.

## Plan-time amendments to the spec

Record these in the spec's §10, under a new "PR 2 plan-time amendments" list, as part of Task 11.

1. **`Tabs` stays the façade** (§3.3). It holds the `DocumentStore` plus one `GroupTabs` per group and the active group, instead of separate `store` and `groups` fields on `App`. The windows live in `App.groups`. The data half and the window half of a group are joined by `GroupId`.
2. **A view is identified by (group, document)** (§3.2). A group never has two views of one document (§5.3 activates the existing one), so no `ViewId` type is needed.
3. **The preview (italic) flag stays on `Document`** (§3.2). A document that gets a second view is promoted to a normal tab. So a preview tab always has exactly one view, and "one preview per group" still holds.
4. **Ctrl+Tab keeps strip order** (§5.1 said MRU). Today it cycles the strip in order; changing that is not part of splits.
5. **"Not enough room to split" is a notice** (`push_notice`), not a status-bar hint. There is no transient hint API.
6. **Group activation from focus:** the editor reports `SCN_FOCUSIN`. The preview view, the image view and the find bar's fields post a new `WM_FASTPAD_CONTENT_FOCUSED` with their own window handle, and the main window resolves the group with `IsChild`.
7. **The active-tab accent** (§4.2) is a 2 px bar along the top edge of each group's active tab: `editor_foreground` for the active group, `muted_foreground` for the others. It is drawn only when there are two or more groups.
8. **New command ids** are 197–210. The palette completeness test widens from `100..200` to `100..300`.
9. **Focus Group 1–8 and Focus Last Group** are not palette entries, like Select Tab 1–9.
10. **Close all tabs** closes the active group's tabs (with one group, every tab, as today). The last one closing removes the group when it is not the only one.
11. **Zoom** applies to every group's editor at once, so the zoom level stays one setting.
12. **The tab context menu** (§7) is new: Close tab, Close all tabs, then Split Right, Split Down and Move to Next Group.
13. **The shared Direct2D graphics** move from `PreviewHost` to `App.graphics`, because every group's preview and image view share them.
14. **A sash double-click** is detected by time and position, as the tab strip does (`GetDoubleClickTime`). The main window class has no `CS_DBLCLKS`, and adding it would turn the menu band's and the status bar's second clicks into double-clicks.
15. **Session group numbers** are the groups' positions in layout order, starting at 1.

## Review Focus

These are the conditions most likely to hurt a user that no task's feature test covers directly. Each has a test in the task named in brackets.

1. **Closing a group whose tab is a dirty document also shown in another group** must not prompt, must not lose text, and must leave the other view dirty. [Task 6]
2. **One edit to a document shown in two groups** must bump its generation once, set dirty once, promote a preview once, and leave the other group's caret where it was. [Task 4]
3. **A session with a dirty untitled document in two groups** restores one document with two views: not two copies, and not a failure notice. [Task 10]
4. **A small window with several groups:** a sash drag stops at the minimum group size, a split that would not fit is refused, and no group gets a negative size. [Task 1, Task 5]
5. **Routing after a click in another group:** once focus is in group 2 (its editor or its find bar), Ctrl+F acts on group 2, and a strip menu command runs in the group that was right-clicked. [Task 4]

---

## Task 1: The split tree

Afterwards `src/window/split_tree.rs` holds the pure layout logic and converts to and from `session::SessionLayout`.

**Files:**
- Create: `src/window/split_tree.rs`
- Modify: `src/window/mod.rs` (add `pub(crate) mod split_tree;` next to `editor_group`)

**Interfaces:**
- Consumes: `super::titlebar::{Point, Rect, scale}`; `crate::session::{SessionAxis, SessionLayout}`.
- Produces:
  - `GroupId(pub(crate) u32)` (Copy, Eq, Hash, Ord), `Axis {Row, Column}`, `Direction {Left, Right, Up, Down}` with `axis()`.
  - `SplitTree::new(GroupId)`, `leaves() -> Vec<GroupId>`, `number_of(GroupId) -> Option<usize>` (1-based), `split(target, Direction, new) -> bool`, `fits_split(target, Direction, new, area: Rect, dpi) -> bool`, `remove(GroupId) -> bool`, `layout(area: Rect, dpi) -> TreeLayout`, `set_sash(&Sash, at: i32, dpi) -> bool`, `equalize(&SashId) -> bool`, `to_session(&dyn Fn(GroupId) -> usize) -> SessionLayout`, `from_session(&SessionLayout, &dyn Fn(usize) -> Option<GroupId>) -> Option<SplitTree>`.
  - `TreeLayout { groups: Vec<(GroupId, Rect)>, sashes: Vec<Sash> }` with `rect_of(GroupId) -> Option<Rect>` and `sash_at(x, y) -> Option<&Sash>`.
  - `Sash { id: SashId, axis: Axis, rect: Rect, branch: Rect }`, `SashId { path: Vec<usize>, index: usize }`.
  - `pub(crate) const SASH_96: i32 = 4; MIN_WIDTH_96: i32 = 160; MIN_HEIGHT_96: i32 = 100;`

- [ ] **Step 1: Write the failing tests.** Create `src/window/split_tree.rs` with only the test module below plus `pub(crate) mod split_tree;` in `src/window/mod.rs`:

```rust
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
                children: vec![(Node::Leaf(A), 0.5), (Node::Leaf(B), 0.25), (Node::Leaf(C), 0.25)],
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
                children: vec![(Node::Leaf(A), 0.5), (Node::Leaf(C), 0.25), (Node::Leaf(D), 0.25)],
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
            let row = &layout.sashes[layout.sashes.len() - 1];
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
```

- [ ] **Step 2: Run them to confirm they fail.**

Run: `cargo test --lib -- split_tree --test-threads=1`
Expected: a compile error, because `SplitTree`, `GroupId`, `Node` and the other types don't exist yet.

- [ ] **Step 3: Implement.** Put this above the test module:

```rust
//! The layout of the editor groups (split editors spec §4.3): rows and columns whose leaves are
//! groups. Pure: it computes rectangles and never touches a window.

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
        lay_out(child, along(*axis, rect, offset, child_end), sash, path, layout);
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
```

If `titlebar::Point::new` or `Rect::contains` has a different name, use the existing one; don't add a second.

- [ ] **Step 4: Run the tests.**

Run: `cargo test --lib -- split_tree --test-threads=1`
Expected: 8 passed. If `the_tree_round_trips_through_the_session_layout` fails on the encoded text only because `format_ratio` writes `0.5` differently, fix the expected string to what `SessionLayout::encode` writes for 0.5 and nothing else.

- [ ] **Step 5: Clippy and commit.**

Run: `cargo clippy --all-targets --all-features -- -D warnings`. `split_tree` is unused outside its tests until Task 5, so add `#![cfg_attr(not(test), allow(dead_code))]` at the top of the file and remove it in Task 5.

```bash
git add src/window/split_tree.rs src/window/mod.rs
git commit -m "feat(split-editors): the split tree lays out groups and sashes and round-trips the session layout"
```

---

## Task 2: Group data in `Tabs`

Afterwards `Tabs` holds one view list per group plus the active group. Existing methods act on the active group; new methods name a group. A document can have views in several groups and leaves the store with its last view.

**Files:**
- Modify: `src/window/tabs.rs` (`Tabs` at :132, `EditorTab` at :122, the methods listed below, and its tests module)
- Modify: every caller of `Tabs::activation_order` (`main_window.rs` `quick_open_rows`, ~1758) and `Tabs::documents` (grep `.documents()`), as described in Step 3.

**Interfaces:**
- Consumes: `split_tree::GroupId`.
- Produces (all on `Tabs`):
  - `active_group() -> GroupId`, `set_active_group(GroupId) -> bool`, `add_group() -> GroupId`, `remove_group(GroupId) -> bool` (only an empty group that isn't the last), `group_ids() -> Vec<GroupId>` (creation order), `group(GroupId) -> Option<&GroupTabs>`.
  - `GroupTabs`: `id`, `len()`, `is_empty()`, `active_index()`, `active_document() -> Option<DocumentId>`, `document_ids() -> Vec<DocumentId>` (strip order), `contains(DocumentId) -> bool`, `selection() -> TabSelection`, `view() -> TabView`, `scroll_offset() -> i32`, `view_state(DocumentId) -> ViewState`.
  - `group_documents(GroupId) -> Vec<&Document>` (strip order), `views_of(DocumentId) -> Vec<GroupId>`, `add_view(GroupId, DocumentId, ViewState) -> bool`, `move_view(from: GroupId, DocumentId, to: GroupId) -> bool`, `activate_in(GroupId, DocumentId) -> bool`, `view_state_in(GroupId, DocumentId) -> ViewState`, `set_view_state_in(GroupId, DocumentId, ViewState)`, `note_text_change(DocumentId) -> bool`, `set_dirty(DocumentId, bool) -> bool`, `refresh_views()`.
  - `activation_order() -> &[(GroupId, DocumentId)]`, global across groups (spec §3.3 `recent_views`).
  - `documents()` now returns every open document once, groups in creation order, each in strip order.

- [ ] **Step 1: Write the failing tests** at the end of `tabs.rs`'s tests module. `text_document(id, path)` and `untitled(id)` stand for the module's existing document fixtures. Use whatever the module already calls them (it builds documents for `push` and `replace_preview` tests).

```rust
    #[test]
    fn a_document_can_have_a_view_in_each_group_and_leaves_with_its_last_view() {
        // Break caught: a second group's tab removing the document from the store when the first
        // group's tab closes, or a close leaving an orphan in the store.
        let mut tabs = Tabs::new();
        tabs.push(untitled(1)).unwrap();
        let first = tabs.active_group();
        let second = tabs.add_group();
        assert!(tabs.add_view(second, DocumentId(1), ViewState::default()));
        assert_eq!(tabs.views_of(DocumentId(1)), vec![first, second]);
        let kept = tabs.close_active(CloseDecision::Discard).unwrap();
        assert!(kept.is_none(), "not the last view: the document stays");
        assert_eq!(tabs.views_of(DocumentId(1)), vec![second]);
        assert!(tabs.document(DocumentId(1)).is_some());
        tabs.set_active_group(second);
        let closed = tabs.close_active(CloseDecision::Discard).unwrap().unwrap();
        assert_eq!(closed.id, DocumentId(1));
        assert!(tabs.document(DocumentId(1)).is_none());
    }

    #[test]
    fn the_facade_methods_act_on_the_active_group() {
        // Break caught: `active()` or `len()` reading the first group after the user moved to
        // another one, so commands act on the wrong tab.
        let mut tabs = Tabs::new();
        tabs.push(untitled(1)).unwrap();
        let second = tabs.add_group();
        assert!(tabs.set_active_group(second));
        assert!(tabs.is_empty());
        assert!(tabs.active().is_none());
        tabs.push(untitled(2)).unwrap();
        assert_eq!(tabs.active().map(|document| document.id), Some(DocumentId(2)));
        assert_eq!(tabs.len(), 1);
        assert_eq!(tabs.documents().count(), 2, "documents() spans every group");
        assert_eq!(
            tabs.activation_order(),
            &[(second, DocumentId(2)), (tabs.group_ids()[0], DocumentId(1))]
        );
    }

    #[test]
    fn a_second_view_promotes_a_preview_and_a_preview_is_replaced_only_in_its_group() {
        // Break caught: an italic tab whose document is also open elsewhere being replaced by the
        // next tree click, closing a view the user did not click away from.
        let mut tabs = Tabs::new();
        let mut preview = untitled(1);
        preview.preview = true;
        tabs.push(preview).unwrap();
        let first = tabs.active_group();
        let second = tabs.add_group();
        tabs.set_active_group(second);
        let mut other = untitled(2);
        other.preview = true;
        assert!(tabs.replace_preview(other).is_none(), "group 2 had no preview");
        assert_eq!(tabs.group(first).unwrap().len(), 1);
        assert_eq!(tabs.preview_id(), Some(DocumentId(2)));
        assert!(tabs.add_view(first, DocumentId(2), ViewState::default()));
        assert!(!tabs.document(DocumentId(2)).unwrap().preview);
    }

    #[test]
    fn moving_a_view_keeps_its_state_and_the_document() {
        // Break caught: Move to Next Group dropping the caret, or removing the document between
        // the removal and the insertion.
        let mut tabs = Tabs::new();
        tabs.push(untitled(1)).unwrap();
        let first = tabs.active_group();
        let second = tabs.add_group();
        let state = ViewState {
            caret: 7,
            anchor: 3,
            first_line: 2,
            x_offset: 0,
        };
        tabs.set_view_state_in(first, DocumentId(1), state);
        assert!(tabs.move_view(first, DocumentId(1), second));
        assert!(tabs.group(first).unwrap().is_empty());
        assert_eq!(tabs.view_state_in(second, DocumentId(1)), state);
        assert!(tabs.document(DocumentId(1)).is_some());
        assert!(tabs.remove_group(first));
        assert!(!tabs.remove_group(second), "the last group stays");
    }

    #[test]
    fn a_text_change_is_recorded_once_on_the_document() {
        // Break caught: a document-level change applied per view, bumping the generation twice
        // for one keystroke.
        let mut tabs = Tabs::new();
        tabs.push(untitled(1)).unwrap();
        let second = tabs.add_group();
        tabs.add_view(second, DocumentId(1), ViewState::default());
        let before = tabs.document(DocumentId(1)).unwrap().generation;
        tabs.note_text_change(DocumentId(1));
        assert_eq!(tabs.document(DocumentId(1)).unwrap().generation, before + 1);
        assert!(tabs.set_dirty(DocumentId(1), true));
        assert!(!tabs.set_dirty(DocumentId(1), true));
    }
```

- [ ] **Step 2: Run them to confirm they fail.**

Run: `cargo test --lib -- window::tabs --test-threads=1`
Expected: a compile error on `add_group`, `add_view` and the other new methods.

- [ ] **Step 3: Implement.**

Split the per-group fields out of `Tabs`:

```rust
/// One group's tabs: views onto documents in the store, in strip order (split editors spec
/// §3.2). A group never has two views of the same document.
#[derive(Debug)]
pub(crate) struct GroupTabs {
    pub(crate) id: GroupId,
    tabs: Vec<EditorTab>,
    selection: TabSelection,
    view: TabView,
}

pub struct Tabs {
    store: DocumentStore,
    groups: Vec<GroupTabs>,
    /// Index into `groups` of the active group.
    active: usize,
    next_group: u32,
    /// Every view, the most recently activated first, across groups (spec §3.3, quick-open spec
    /// §3.2). Kept in memory only.
    recent: Vec<(GroupId, DocumentId)>,
}
```

- `Tabs::new()` creates one group, `GroupId(1)`, and sets `next_group: 2`. `add_group` hands out `GroupId(next_group)` and increments it.
- Add private `fn current(&self) -> &GroupTabs` and `fn current_mut(&mut self) -> &mut GroupTabs` (the group at `active`), and `fn group_index(&self, id) -> Option<usize>`.
- Rewrite each existing method against `current()` / `current_mut()`: `strip`, `document_at(_mut)`, `position`, `len`, `is_empty`, `active_index`, `scroll_offset`, `set_scroll_offset`, `selection`, `view`, `refresh_view`, `set_preview_buttons`, `active`, `active_mut`, `view_state`, `set_view_state`, `replace_active_untitled`, `titles`, `activate`, `activate_index`, `push`, `close_active`, `close_reviewed`, `close_clean_background`, `preview_id`, `replace_preview`, `active_handle`. `touch` records `(active group id, document)`.
- `remove_tab(index)` removes from the current group and returns the document only when `views_of(document)` is empty afterwards. `close_active` and `close_reviewed` already turn a `None` into `Err(..)`. Change them so that closing a view whose document still has another view **succeeds** without returning a document: return `Result<Option<Document>, _>` and update their callers (`close_reviewed_document` in `main_window.rs`, and the existing tests, which add an `.unwrap()` or match `Ok(Some(_))`).
- `replace_preview` only replaces a preview in the current group whose document has exactly one view.
- `add_view(group, document, state)`: if that group already has the document, select it and return true. Otherwise append an `EditorTab { document, view_state: state }`, select it, `touch`, and clear `preview` on the document when it now has two or more views. Refresh the views of every group whose titles changed (`refresh_views`). Return false for an unknown group or document.
- `move_view(from, document, to)`: take the `EditorTab` out of `from` (fix `from`'s selection as `select_after_removal` does, without touching the store), then `add_view(to, document, tab.view_state)`. When `to` already had the document, only its existing view is selected. Update `recent`: remove `(from, document)`.
- `remove_group(id)` succeeds only for an empty group when another group exists. It fixes `active` so that it still indexes the same active group; if the removed group was active, it picks index `min(removed, len - 1)`.
- `note_text_change(id)`: the old `note_active_text_change` body for `id` (generation bump, preview cleared, returns whether the preview flag was cleared). Keep `note_active_text_change` as a one-line call with the active document until Task 4 replaces its caller.
- `set_dirty(id, dirty)`: the old `set_active_dirty` body for `id`; keep `set_active_dirty` delegating.
- `documents()`: every group's strip in turn, skipping a document already yielded.
- `activation_order()` returns `&[(GroupId, DocumentId)]`. In `quick_open_rows` (main_window.rs ~1758) iterate `.iter().map(|(_, id)| *id)` for now. Task 9 uses the group.
- `reset_activation_order()` puts the active group's active view first, then every group in creation order, strip order.
- `clear_for_shutdown()` clears every group's tabs and leaves one group.

Callers of `documents()` that meant "the strip" (grep `.documents()`): `open_editors::snapshot` and `build_session`. Switch them to `group_documents(tabs.active_group())` for now, so that with one group nothing changes. Tasks 9 and 10 make them walk every group.

- [ ] **Step 4: Run the tests.**

Run: `cargo test --lib -- window::tabs --test-threads=1`
Expected: all of the module's tests pass, the 5 new ones included.

Then run the callers' tests: `cargo test --lib -- main_window::tests::session main_window::tests::quick main_window::tests::preview --test-threads=1`.
Expected: all pass (one group behaves as before).

- [ ] **Step 5: Clippy and commit.**

```bash
git add src/window/tabs.rs src/window/main_window.rs src/window/open_editors.rs
git commit -m "refactor(split-editors): tabs keep one view list per group; documents leave the store with their last view"
```

---

## Task 3: Group windows own their editor, find bar, preview and image view

Afterwards `App.groups` holds every group window with its own Scintilla, find bar, preview host and image host. Everything that used `App.editor` / `find_bar` / `preview` / `image` goes through the active group. A second group can be created and destroyed. There is still no command that does it; Task 6 adds those.

**Files:**
- Modify: `src/app.rs` (fields :37–112, `App::new` :117, `ensure_accessibility` :248)
- Modify: `src/window/editor_group.rs` (`GroupWindow`, `create`)
- Modify: `src/window/main_window.rs`: `initialize_editor_with` (:886), `install_editor` (:6487), `editor_hwnd` (:1002), `with_editor` (:1009), `group_hwnd` (:1057), `content_parent` (:1067), `layout_group` (:1073), `paint_group` (:1205), `with_group` (:2365), `strip_layout` (:2386), `tab_snapshot` (:2349), `group_accessible_object` (:2680), `ensure_find_bar` (:1320), `find_bar_owns`, `apply_theme` (:3472), and every `app.editor` / `.find_bar` line listed in Step 3.
- Modify: `src/window/preview_host.rs` (`with_host` :248, `editor` :263, `layout` :1115, `shared_graphics` :497, `PreviewHost.graphics`)
- Modify: `src/window/image_host.rs` (`with_host` :29, `layout` :125)
- Modify: `src/window/preview_buttons.rs` (`hwnd` :63, `layout` :114, `create` :144)
- Modify: `src/window/find_bar.rs` (`with_bar` :1008)
- Test: `src/window/main_window.rs` tests module

**Interfaces:**
- Consumes: Task 2's group API on `Tabs`, `split_tree::GroupId`.
- Produces:
  - `GroupWindow { id: GroupId, hwnd, editor: Editor, find_bar: Option<FindBar>, image: ImageHost, preview: PreviewHost, pointer, thumb_grab, middle_press, last_tab_click, accessibility, preview_buttons }`. `image` is declared before `preview`.
  - `App.groups: Vec<GroupWindow>`, `App.graphics: Option<Rc<Graphics>>`.
  - On `App`: `active_group() -> Option<&GroupWindow>`, `active_group_mut()`, `group(GroupId)`, `group_mut(GroupId)`, `editor() -> Option<&Editor>`, `find_bar() -> Option<&FindBar>`, `find_bar_mut() -> Option<&mut FindBar>`, `group_containing(HWND) -> Option<GroupId>` (the group whose window is `hwnd` or `IsChild` of it).
  - In `main_window`: `pub(crate) fn create_group(hwnd: HWND) -> crate::Result<GroupId>`, `pub(crate) fn destroy_group(hwnd: HWND, id: GroupId)`, `pub(crate) fn with_group_id<R>(hwnd, GroupId, impl FnOnce(&mut GroupWindow) -> R) -> Option<R>`, `pub(crate) fn group_editor(hwnd, GroupId) -> Option<Editor>`, `fn configure_editor(hwnd, &Editor)`.
  - `preview_host::with_group_host(hwnd, GroupId, f)` and `image_host::with_group_host(hwnd, GroupId, f)`. The existing `with_host(hwnd, f)` in both means the active group.
  - `preview_host::layout(hwnd, GroupId, area, dpi)`, `image_host::layout(hwnd, GroupId, area)`, `preview_buttons::layout(hwnd, group: HWND, area, dpi)` (it resolves the group's id itself).

- [ ] **Step 1: Write the failing test** in `main_window.rs`'s tests module, next to the PR 1 test that checks the group's children (~6963):

```rust
    #[test]
    fn a_second_group_gets_its_own_editor_find_bar_and_preview() {
        // Break caught: a second group sharing the first one's editor or find bar, so a find in
        // one group moves the caret in the other, or its preview state leaking across.
        use windows_sys::Win32::UI::WindowsAndMessaging::GetParent;
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        let first = app_mut(window.hwnd).tabs.active_group();
        let second = super::create_group(window.hwnd).expect("second group");
        assert_ne!(first, second);
        let (first_editor, second_editor) = (
            super::group_editor(window.hwnd, first).unwrap(),
            super::group_editor(window.hwnd, second).unwrap(),
        );
        assert_ne!(first_editor.hwnd(), second_editor.hwnd());
        let second_window = app_mut(window.hwnd).group(second).unwrap().hwnd;
        assert_eq!(unsafe { GetParent(second_editor.hwnd()) }, second_window);
        assert!(second_editor.shares_documents_with(&first_editor));

        app_mut(window.hwnd).tabs.set_active_group(second);
        execute_command(window.hwnd, CommandId::Find);
        let panel = app_mut(window.hwnd).find_bar().unwrap().panel;
        assert_eq!(unsafe { GetParent(panel) }, second_window);
        assert!(app_mut(window.hwnd).group(first).unwrap().find_bar.is_none());
        assert_eq!(
            app_mut(window.hwnd).group_containing(first_editor.hwnd()),
            Some(first)
        );

        super::destroy_group(window.hwnd, second);
        assert!(app_mut(window.hwnd).group(second).is_none());
        assert_eq!(app_mut(window.hwnd).tabs.group_ids(), vec![first]);
    }
```

If `FindBar::panel` is private, read it through the existing `find_bar_owns` path: assert `find_bar_owns(window.hwnd, panel)` with the handle from `GetParent(query_hwnd)`. Don't widen visibility for the test alone.

- [ ] **Step 2: Run it to confirm it fails.**

Run: `cargo test --lib -- a_second_group_gets_its_own --test-threads=1`
Expected: a compile error, because `create_group`, `group_editor`, `destroy_group` and the `App` accessors don't exist yet.

- [ ] **Step 3: Implement.**

**`GroupWindow`** gains `id`, `editor`, `find_bar`, `image` and `preview`. `GroupWindow::new(id, hwnd, editor)` sets the rest to their defaults. Update the doc comment ("One per editor group; the main window keeps them in `App.groups`").

**`App`**: remove `editor`, `group`, `find_bar`, `image` and `preview`. Add `groups: Vec<GroupWindow>` and `graphics: Option<Rc<crate::preview::Graphics>>`, both declared where `group` was. Add the accessors:

```rust
    pub(crate) fn active_group(&self) -> Option<&GroupWindow> {
        self.group(self.tabs.active_group())
    }

    pub(crate) fn active_group_mut(&mut self) -> Option<&mut GroupWindow> {
        let id = self.tabs.active_group();
        self.group_mut(id)
    }

    pub(crate) fn group(&self, id: GroupId) -> Option<&GroupWindow> {
        self.groups.iter().find(|group| group.id == id)
    }

    pub(crate) fn group_mut(&mut self, id: GroupId) -> Option<&mut GroupWindow> {
        self.groups.iter_mut().find(|group| group.id == id)
    }

    /// The active group's editor.
    pub(crate) fn editor(&self) -> Option<&Editor> {
        self.active_group().map(|group| &group.editor)
    }

    pub(crate) fn find_bar(&self) -> Option<&FindBar> {
        self.active_group()?.find_bar.as_ref()
    }

    pub(crate) fn find_bar_mut(&mut self) -> Option<&mut FindBar> {
        self.active_group_mut()?.find_bar.as_mut()
    }

    /// The group whose window is `hwnd` or holds it.
    pub(crate) fn group_containing(&self, hwnd: HWND) -> Option<GroupId> {
        self.groups
            .iter()
            .find(|group| group.hwnd == hwnd || unsafe { IsChild(group.hwnd, hwnd) } != 0)
            .map(|group| group.id)
    }
```

**Mechanical replacements**, in production code and tests alike:

| Old | New |
|---|---|
| `app.editor.clone()` | `app.editor().cloned()` |
| `app.editor.as_ref()` | `app.editor()` |
| `app.editor.is_some()` | `app.editor().is_some()` |
| `app.find_bar.as_ref()` / `.as_mut()` | `app.find_bar()` / `app.find_bar_mut()` |
| `app.find_bar = Some(bar)` (in `ensure_find_bar`) | `if let Some(group) = app.active_group_mut() { group.find_bar = Some(bar) }` |
| `app.group.as_ref()` / `.as_mut()` | `app.active_group()` / `app.active_group_mut()` |
| `&mut app.preview` (in `preview_host::with_host`) | `&mut app.active_group_mut()?.preview` |
| `&mut app.image` (in `image_host::with_host`) | `&mut app.active_group_mut()?.image` |
| `host.graphics` (in `shared_graphics`) | `app.graphics` |

**Functions that act on a named group** (they are called for a specific group window, which may not be the active one):
- `layout_group(hwnd, group: HWND)`: resolve `let Some(id) = app.group_containing(group)`, then use that group's editor, find bar and host. Pass `id` to `preview_host::layout` and `image_host::layout`.
- `paint_group(hwnd, group)` and `tab_snapshot(hwnd, id)`: read `app.tabs.group(id)` and `group_documents(id)`. `tab_snapshot` takes the group id. `strip_layout(hwnd)` keeps its meaning (the active group); add `strip_layout_of(hwnd, id)`, used by `paint_group`, `group_hit_test`, `strip_target` and `caption_strip_point`, which already know their group window.
- `group_accessible_object(hwnd, group, wparam)`: use `app.tabs.group(id)`'s `view()` and `selection()`, and `app.group_mut(id).accessibility`.
- `preview_host::layout(hwnd, id, area, dpi)`: its body uses `with_group_host(hwnd, id, …)`. `image_host::layout(hwnd, id, area)` shows the view only when *that* group's active tab is an image (`app.tabs.group(id)`'s active document).
- `preview_buttons::layout(hwnd, group, area, dpi)`: the visibility test is that group's active document's language (Markdown or SVG), not the active group's. Its `create` stores the buttons' window in `app.group_mut(id).preview_buttons`.
- `find_bar_owns(hwnd, panel_or_field)` looks through every group's bar. The `WM_CTLCOLOREDIT` arm colours with the bar that owns the field. `find_bar::with_bar` finds the bar by panel across groups.

**Every setting applied to the editor** (theme colours, font, font size, word wrap, line numbers, tab width, zoom, the input filter, the scroll width) goes into `fn configure_editor(hwnd: HWND, editor: &Editor)`. Build it from the calls that `load_settings`, `apply_theme` and the toggle commands make on `app.editor` today. The toggles and `apply_theme` then loop over `app.groups` and apply to every group's editor. Zoom commands (`ZoomIn`/`ZoomOut`/`ZoomReset`) apply to every group's editor (amendment 11).

**`create_group(hwnd)`**:

```rust
/// Creates an empty group window with its own editor, set up like the others. The caller puts it
/// in the layout (split editors spec §4.2).
pub(crate) fn create_group(hwnd: HWND) -> crate::Result<GroupId> {
    #[cfg(test)]
    if FAIL_NEXT_GROUP.with(|fail| fail.replace(false)) {
        return Err(crate::Error::message("test: group creation failed"));
    }
    let app = unsafe { app_ptr(hwnd) }.ok_or_else(|| crate::Error::message("no window state"))?;
    let host = unsafe { app.as_ref() }
        .document_host
        .clone()
        .ok_or_else(|| crate::Error::message("no document host"))?;
    let window = crate::window::editor_group::create(hwnd)?;
    let editor = match Editor::create_with_host(window, &host) {
        Ok(editor) => editor,
        Err(error) => {
            unsafe { DestroyWindow(window) };
            return Err(error);
        }
    };
    configure_editor(hwnd, &editor);
    // A new group shows nothing until a view is added.
    unsafe { ShowWindow(editor.hwnd(), SW_HIDE) };
    let app = unsafe { &mut *app.as_ptr() };
    let id = app.tabs.add_group();
    app.groups
        .push(crate::window::editor_group::GroupWindow::new(id, window, editor));
    Ok(id)
}

#[cfg(test)]
thread_local! {
    static FAIL_NEXT_GROUP: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
pub(crate) fn fail_next_group_creation() {
    FAIL_NEXT_GROUP.with(|fail| fail.set(true));
}
```

Use the crate's existing error constructor in place of `crate::Error::message` (grep how `initialize_editor_with` reports "no window state"). `document_host` holds an `Editor`, and `Editor` is `Clone` over `Rc`s.

**`destroy_group(hwnd, id)`** removes the group from `app.groups` (dropping its hosts and editor), then calls `DestroyWindow` on its window and `app.tabs.remove_group(id)`. The caller has already emptied it and taken it out of the layout.

**`initialize_editor_with`** keeps its signature (tests inject editors through `create_editor`). It creates the first group window with `editor_group::create`, calls `create_editor(group)`, and pushes `GroupWindow::new(app.tabs.active_group(), group, editor)`. `install_editor` stops setting `app.editor`.

**`shared_graphics`** reads and fills `app.graphics`.

- [ ] **Step 4: Run the new test and the group's neighbours.**

Run: `cargo test --lib -- a_second_group_gets_its_own main_window::tests::find preview_host image_host main_window::tests::the_group --test-threads=1`
Expected: all pass.

- [ ] **Step 5: Run the library tests once**, since this touches every editor access.

Run: `cargo test --lib -- --test-threads=1 > "$SCRATCH/task3.txt" 2>&1; tail -5 "$SCRATCH/task3.txt"` (`$SCRATCH` is the scratchpad directory).
Expected: `test result: ok.`

- [ ] **Step 6: Clippy and commit.**

```bash
git add -A src
git commit -m "refactor(split-editors): each group window owns its editor, find bar, preview and image view"
```

---

## Task 4: Routing by group

Afterwards a Scintilla notification, a content focus change and a preview view's messages all reach the group they came from. Document-level effects run once per change. Focus arriving in a group makes it active.

**Files:**
- Modify: `tools/generate-scintilla-constants.ps1` (add `"SCN_FOCUSIN"` to the name list, ~:55) and the regenerated `src/editor/scintilla_constants.rs`
- Modify: `src/window/mod.rs` (a new `WM_FASTPAD_CONTENT_FOCUSED`: the next free `WM_APP + n`; grep the existing constants)
- Modify: `src/window/main_window.rs`: `handle_editor_notification` (:6104), the preview message arms (:657–687), the `PREVIEW_TIMER_ID` arm, `remember_active_view` (:4374), `activate_document` (:4313), `refresh_tabs` (:2700), `group_strip_message` (:2544), `focus_group_content` (:1262)
- Modify: `src/window/editor_group.rs` (`WM_SETFOCUS`)
- Modify: `src/window/preview_host.rs` (`editor_scrolled` :1043, `preview_scrolled` :1065, `record_edit` :782, `parsed` :740, `spawn_parse` :713, `flush` :858, `follow_link`, `hover_link`, `escape` :407, `refresh`)
- Modify: `src/preview/view.rs` (posts at :811, :1140, :1155, :1434, :1538; `WM_SETFOCUS` at :1366)
- Modify: `src/image_view/mod.rs` (`WM_SETFOCUS` at :914)
- Modify: `src/window/find_bar.rs` (`find_field_proc` `WM_SETFOCUS` at :1188)
- Modify: `src/window/library_host.rs` (`text_changed` :1257)

**Interfaces:**
- Consumes: Task 3's `App` accessors, `group_containing`, `group_editor`, `with_group_id`.
- Produces:
  - `pub(crate) fn activate_group(hwnd: HWND, id: GroupId) -> bool`: makes `id` active; repaints both strips, the status bar and Open Editors; returns whether it changed.
  - `pub(crate) fn show_group_view(hwnd: HWND, id: GroupId)`: that group's editor shows its active view's document with its saved `ViewState`; it hides the editor for an image tab and when the group is empty.
  - `pub(crate) fn remember_view(hwnd: HWND, id: GroupId)`: saves that group's editor's view state into its active view.
  - `pub(crate) fn activate_document_in(hwnd, GroupId, DocumentId) -> bool`: the old `activate_document` for a named group.
  - `fn reporting_group(hwnd, DocumentId) -> Option<GroupId>`: the first group in layout order (`app.layout.leaves()`, see Task 5; until then `tabs.group_ids()`) whose active view is the document.
  - `preview_host` functions that act on a group take `id: GroupId`: `editor_scrolled(hwnd, id)`, `preview_scrolled(hwnd, id, line)`, `record_edit(hwnd, id, modification)`, `follow_link(hwnd, id, lparam)`, `hover_link(hwnd, id, lparam)`, `escape(hwnd, id)`, `refresh(hwnd, id)`. `parsed(hwnd, lparam)` reads the group from its payload. `flush(hwnd)` flushes every group.

- [ ] **Step 1: Write the failing tests** in `main_window.rs`'s tests module. `second_group_showing_the_active_document(hwnd) -> (GroupId, GroupId)` is a new test helper: `create_group`, `tabs.add_view(second, active id, ViewState::default())`, then `show_group_view(hwnd, second)`. It returns `(first, second)` and leaves the first group active.

```rust
    fn second_group_showing_the_active_document(hwnd: HWND) -> (GroupId, GroupId) {
        let first = app_mut(hwnd).tabs.active_group();
        let second = super::create_group(hwnd).expect("second group");
        let id = app_mut(hwnd).tabs.active().unwrap().id;
        assert!(app_mut(hwnd).tabs.add_view(second, id, ViewState::default()));
        super::show_group_view(hwnd, second);
        (first, second)
    }

    #[test]
    fn an_edit_in_one_group_shows_in_the_other_and_counts_once() {
        // Break caught: document-level effects run by every editor showing the document, so one
        // keystroke bumps the generation twice, or the other group's caret jumps to the edit.
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        editor.set_text("one\ntwo\nthree").unwrap();
        let (_, second) = second_group_showing_the_active_document(window.hwnd);
        let other = super::group_editor(window.hwnd, second).unwrap();
        editor.apply_view_state(ViewState { caret: 0, anchor: 0, first_line: 0, x_offset: 0 }).unwrap();
        other.apply_view_state(ViewState { caret: 8, anchor: 8, first_line: 0, x_offset: 0 }).unwrap();
        let id = app_mut(window.hwnd).tabs.active().unwrap().id;
        let before = app_mut(window.hwnd).tabs.document(id).unwrap().generation;

        editor.insert_text(0, "x").unwrap();

        assert_eq!(other.text().unwrap(), "xone\ntwo\nthree");
        let document = app_mut(window.hwnd).tabs.document(id).unwrap();
        assert_eq!(document.generation, before + 1);
        assert!(document.dirty);
        assert_eq!(other.view_state().caret, 9, "the other caret moves with its text only");
    }

    #[test]
    fn focus_in_a_groups_editor_or_find_bar_makes_that_group_active() {
        // Break caught: a click into group 2 leaving group 1 active, so Ctrl+F, the status bar
        // and the title act on the group the user left.
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus;
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        let (first, second) = second_group_showing_the_active_document(window.hwnd);
        let other = super::group_editor(window.hwnd, second).unwrap();

        unsafe { SetFocus(other.hwnd()) };
        assert_eq!(app_mut(window.hwnd).tabs.active_group(), second);
        execute_command(window.hwnd, CommandId::Find);
        let query = app_mut(window.hwnd).find_bar().unwrap().query_hwnd();

        let first_editor = super::group_editor(window.hwnd, first).unwrap();
        unsafe { SetFocus(first_editor.hwnd()) };
        assert_eq!(app_mut(window.hwnd).tabs.active_group(), first);
        unsafe { SetFocus(query) };
        pump_posted(window.hwnd);
        assert_eq!(app_mut(window.hwnd).tabs.active_group(), second);
    }

    #[test]
    fn a_strip_menu_command_acts_on_the_group_that_was_right_clicked() {
        // Break caught: the strip's menu running New in the active group instead of the group
        // under the pointer.
        use windows_sys::Win32::UI::WindowsAndMessaging::{WM_RBUTTONDOWN, WM_RBUTTONUP};
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        let (first, second) = second_group_showing_the_active_document(window.hwnd);
        super::layout_editor_and_find_bar(window.hwnd);
        let group = app_mut(window.hwnd).group(second).unwrap().hwnd;
        let empty = empty_strip_point(window.hwnd, second);
        answer_next_popup_menu(Some(CommandId::New));
        unsafe {
            SendMessageW(group, WM_RBUTTONDOWN, 0, empty);
            SendMessageW(group, WM_RBUTTONUP, 0, empty);
        }
        assert_eq!(app_mut(window.hwnd).tabs.active_group(), second);
        assert_eq!(app_mut(window.hwnd).tabs.group(second).unwrap().len(), 2);
        assert_eq!(app_mut(window.hwnd).tabs.group(first).unwrap().len(), 1);
    }
```

A fourth test pins the per-group preview (spec §12):

```rust
    #[test]
    fn a_preview_opened_in_one_group_leaves_the_other_group_alone() {
        // Break caught: one preview mode shared by every group, so Full preview in group 2 hides
        // group 1's editor or opens a preview there too.
        use windows_sys::Win32::UI::WindowsAndMessaging::IsWindowVisible;
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        editor.set_text("# title").unwrap();
        app_mut(window.hwnd)
            .tabs
            .set_active_language(crate::document::Language::Markdown);
        let (first, second) = second_group_showing_the_active_document(window.hwnd);
        super::activate_group(window.hwnd, second);
        execute_command(window.hwnd, CommandId::MarkdownPreviewFull);
        let mode = |id| {
            crate::window::preview_host::with_group_host(window.hwnd, id, |host| host.mode())
        };
        assert_eq!(mode(second), Some(crate::window::preview_host::PreviewMode::Full));
        assert_eq!(mode(first), Some(crate::window::preview_host::PreviewMode::Off));
        assert!(unsafe { IsWindowVisible(editor.hwnd()) } != 0);
    }
```

Add `pub(crate) fn mode(&self) -> PreviewMode` to `PreviewHost` if it has no such accessor (`preview_host::mode(hwnd)` reads the active group's). PR 1's floating-button tests set the language this way too, because `LanguageMarkdown` does not take effect in tests.

Supporting test helpers:
- `pump_posted(hwnd)` runs `PeekMessageW(PM_REMOVE)` + `DispatchMessageW` for `hwnd`'s queue until it is empty. Reuse an existing pump helper if the module has one (grep `fn pump`).
- `empty_strip_point(hwnd, id) -> LPARAM` returns a point on the group's strip past its last tab: `strip_layout_of(hwnd, id)`'s last tab's right edge plus 10, at half the strip's height, packed with the module's existing point-packing helper.
- The third test needs Task 3's layout to have given the second group a width. Until Task 5 lays groups out, set it directly with `MoveWindow(group, 0, 0, 600, 400, 0)` before the clicks.

- [ ] **Step 2: Run them to confirm they fail.**

Run: `cargo test --lib -- an_edit_in_one_group focus_in_a_groups a_strip_menu_command_acts a_preview_opened_in_one_group --test-threads=1`
Expected:
- the first fails on `generation`, which moved by 2;
- the second fails on `active_group`, which is unchanged;
- the third fails on the tab counts, because New ran in the first group;
- the fourth fails to compile until `with_group_host` exists (Task 3), then fails because both groups' modes change together while the preview posts no group.

- [ ] **Step 3: Implement.**

**Constants.** Add `"SCN_FOCUSIN"` to the generator's name list and run `./tools/generate-scintilla-constants.ps1`. Expected: `pub const SCN_FOCUSIN: u32 = 2028;` is the only addition (check with `git diff --stat`). Add `WM_FASTPAD_CONTENT_FOCUSED` in `window/mod.rs`, with a doc comment: "A content child (preview view, image view, find field) got the focus; wparam is its window."

**`handle_editor_notification`**:

```rust
    let Some(group) = app.groups.iter().find(|group| group.editor.hwnd() == notification.hwndFrom)
        .map(|group| group.id) else {
        return;
    };
    let active = app.tabs.active_group() == group;
```

- `SCN_FOCUSIN` → `activate_group(hwnd, group)`.
- `SCN_UPDATEUI` → `invalidate_status_bar` only when `active`. On `SC_UPDATE_V_SCROLL`, `preview_host::editor_scrolled(hwnd, group)`.
- `SCN_ZOOM` → that group's editor `remeasure_line_numbers()`.
- `SCN_MODIFIED` (insert or delete):
  - Every sender: `refresh_line_numbers()` on its own editor when `lines_added != 0`.
  - Only when `reporting_group(hwnd, document) == Some(group)`, where `document` is that group's active view's document:
    - `promoted = app.tabs.note_text_change(document)`, and `invalidate_title_strip` if promoted;
    - `library_host::text_changed(hwnd, group, position)`, which reads the line through that group's editor;
    - `library_host::schedule_autosave(hwnd)`;
    - `preview_host::record_edit(hwnd, g, modification)` for **every** group `g` whose active view is the document.
- `SCN_SAVEPOINTLEFT` / `SCN_SAVEPOINTREACHED`, only for the reporting group: `app.tabs.set_dirty(document, dirty)`, then `invalidate_title_strip` if it changed, then `notebook_view::editors_changed`.

Replace the last caller of `note_active_text_change` and delete that method. Also delete `set_active_dirty` if nothing else calls it.

**`activate_group(hwnd, id)`**: if `id` is already active, return false. Otherwise:
1. `app.tabs.set_active_group(id)`;
2. `invalidate_group_strip` for both groups;
3. `invalidate_status_bar`;
4. `InvalidateRect(hwnd)` of the title row (the caption text follows at `WM_PAINT`);
5. `notebook_view::editors_changed`, `side_panel::active_tab_changed`;
6. `preview_host::sync_visibility(hwnd)` (the floating buttons follow the active group);
7. return true.

It does not move the focus.

**Group presses activate first.** In `group_strip_message`, before any `WM_LBUTTONDOWN`, `WM_LBUTTONDBLCLK`, `WM_RBUTTONDOWN` or `WM_MBUTTONDOWN` handling, call `activate_group(hwnd, id)` with `id = app.group_containing(group)`. Everything after that stays active-group code. The group's `WM_SETFOCUS` activates its group, then calls `focus_group_content`.

**Content focus.** In the preview view's `WM_SETFOCUS`, the image view's `WM_SETFOCUS` and `find_field_proc`'s `WM_SETFOCUS`, add:

```rust
unsafe {
    PostMessageW(
        crate::platform::win32::root_window(hwnd),
        crate::window::WM_FASTPAD_CONTENT_FOCUSED,
        hwnd as usize,
        0,
    )
};
```

Keep their existing repaint. In the main window proc, handle `WM_FASTPAD_CONTENT_FOCUSED` with `if let Some(id) = app.group_containing(wparam as HWND) { activate_group(hwnd, id); }`.

**Preview view messages carry their window.** `WM_FASTPAD_PREVIEW_SCROLLED` puts the view's `hwnd` in lparam. LINK, HOVER, ESCAPE and REFRESH put it in wparam. The main window resolves `app.group_containing(view)` and passes the id on; if nothing contains it, it drops the message and frees the payload as the existing arms do. The image view's `WM_FASTPAD_IMAGE_STATUS` stays as it is.

**Parse results.** The payload that `spawn_parse` boxes gains `group: GroupId`, and `parsed` applies the result to that group's host only if its `document` and `parse_generation` still match.

**Timer.** The `PREVIEW_TIMER_ID` arm calls `preview_host::flush(hwnd)`, which walks every group. Kill the timer only when no group has pending edits.

**`activate_document` → `activate_document_in(hwnd, group, id, revision)`:**
1. `remember_view(hwnd, group)`;
2. `app.tabs.activate_in(group, id)`;
3. `show_group_view(hwnd, group)`;
4. `refresh_tabs(hwnd)` when `group` is active, otherwise `invalidate_group_strip`.

The old `activate_document(hwnd, id, revision)` becomes a call with the active group. `remember_active_view(hwnd)` becomes `remember_view(hwnd, active group)`.

**Autosave when leaving a tab** stays keyed to the tab being left in the active group, as today.

- [ ] **Step 4: Run the tests.**

Run: `cargo test --lib -- an_edit_in_one_group focus_in_a_groups a_strip_menu_command_acts a_preview_opened_in_one_group preview_host main_window::tests::preview main_window::tests::find --test-threads=1`
Expected: all pass.

- [ ] **Step 5: Clippy and commit.**

```bash
git add -A src tools
git commit -m "feat(split-editors): notifications, focus and preview messages reach the group they came from"
```

---

## Task 5: Laying out several groups

Afterwards the main window lays the groups out with the split tree:
- A top group reaches into the title row and draws its strip there.
- A lower group draws its strip at its own top.
- The main window draws the sashes and drags them.
- Only the group under the caption buttons gives them up.
- The active tab of each group carries the accent bar when there are two or more groups.

**Files:**
- Modify: `src/app.rs`: add `layout: SplitTree`, `sash_drag: Option<Sash>` and `last_sash_click: Option<(SashId, u32)>`. The first group's id seeds `layout` in `initialize_editor_with`.
- Modify: `src/window/main_window.rs`:
  - `layout_editor_and_find_bar` (:1022), `set_group_region` (:1138), `group_hit_test` (:1160), `caption_strip_point` (:1188), `group_strip_bounds` (:2375);
  - the main proc's `WM_PAINT` (:308), `WM_SETCURSOR`, `WM_LBUTTONDOWN` (:373), `WM_MOUSEMOVE` (:350), `WM_LBUTTONUP` and `WM_CAPTURECHANGED`.
- Modify: `src/window/titlebar.rs` (`TitlePaint.covered` :469 and its use :499)
- Modify: `src/window/group_strip.rs` (`StripPaint` and `draw`)
- Modify: `src/window/split_tree.rs` (drop the `dead_code` allowance)

**Interfaces:**
- Consumes: Task 1's `SplitTree`, `TreeLayout`, `Sash`; Task 3's `App.groups`.
- Produces:
  - `fn tree_area(hwnd) -> Option<Rect>`: the editor area below the title row, above the status bar, right of the sidebar, in main-client coordinates.
  - `pub(crate) fn tree_layout(hwnd) -> Option<TreeLayout>`.
  - `pub(crate) fn group_order(hwnd) -> Vec<GroupId>` (= `app.layout.leaves()`).
  - `TitlePaint.covered: Vec<Rect>`.
  - `group_strip::tab_accent(active_group: bool, group_count: usize, palette: &Palette) -> Option<u32>`.

- [ ] **Step 1: Write the failing tests** in `main_window.rs`'s tests module. `split_for_test(hwnd, direction) -> GroupId` is a test helper: `create_group`, `app.layout.split(active, direction, new)`, `add_view` of the active document, `show_group_view`, then `layout_editor_and_find_bar`.

```rust
    fn split_for_test(hwnd: HWND, direction: Direction) -> GroupId {
        let active = app_mut(hwnd).tabs.active_group();
        let new = super::create_group(hwnd).expect("group");
        assert!(app_mut(hwnd).layout.split(active, direction, new));
        let id = app_mut(hwnd).tabs.active().unwrap().id;
        app_mut(hwnd).tabs.add_view(new, id, ViewState::default());
        super::show_group_view(hwnd, new);
        super::layout_editor_and_find_bar(hwnd);
        new
    }

    fn window_rect_in_main(hwnd: HWND, child: HWND) -> RECT {
        let mut rect = RECT::default();
        unsafe {
            GetWindowRect(child, &mut rect);
            MapWindowPoints(std::ptr::null_mut(), hwnd, &mut rect as *mut RECT as *mut POINT, 2);
        }
        rect
    }

    #[test]
    fn groups_side_by_side_both_reach_into_the_title_row_with_a_sash_between() {
        // Break caught: a second column pushed below the title row (wasting a row), overlapping
        // the first, or leaving no gap for the sash.
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        let first = app_mut(window.hwnd).tabs.active_group();
        let second = split_for_test(window.hwnd, Direction::Right);
        let (a, b) = (
            window_rect_in_main(window.hwnd, app_mut(window.hwnd).group(first).unwrap().hwnd),
            window_rect_in_main(window.hwnd, app_mut(window.hwnd).group(second).unwrap().hwnd),
        );
        assert_eq!((a.top, b.top), (0, 0));
        let dpi = unsafe { GetDpiForWindow(window.hwnd) };
        assert_eq!(b.left - a.right, crate::window::titlebar::scale(SASH_96, dpi));
        assert_eq!(super::tree_layout(window.hwnd).unwrap().sashes.len(), 1);
        assert_eq!(super::group_strip_bounds(window.hwnd).len(), 2);
    }

    #[test]
    fn a_group_below_another_draws_its_strip_at_its_own_top() {
        // Break caught: a lower group's strip hit-tested as caption, so clicking its tabs drags
        // the window.
        use windows_sys::Win32::UI::WindowsAndMessaging::{HTCLIENT, WM_NCHITTEST};
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        let second = split_for_test(window.hwnd, Direction::Down);
        let group = app_mut(window.hwnd).group(second).unwrap().hwnd;
        let rect = window_rect_in_main(window.hwnd, group);
        assert!(rect.top > 0);
        let mut screen = POINT { x: rect.right - 20, y: rect.top + 5 };
        unsafe { ClientToScreen(window.hwnd, &mut screen) };
        let packed = ((screen.y as u32 & 0xffff) << 16 | (screen.x as u32 & 0xffff)) as LPARAM;
        assert_eq!(unsafe { SendMessageW(group, WM_NCHITTEST, 0, packed) }, HTCLIENT as LRESULT);
        assert_eq!(super::group_strip_bounds(window.hwnd).len(), 1);
    }

    #[test]
    fn dragging_a_sash_resizes_both_groups_and_stops_at_the_minimum() {
        // Break caught: a sash that doesn't follow the pointer, or one dragged over the edge
        // collapsing a group to nothing.
        use windows_sys::Win32::UI::WindowsAndMessaging::{WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE};
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        let first = app_mut(window.hwnd).tabs.active_group();
        split_for_test(window.hwnd, Direction::Right);
        let sash = super::tree_layout(window.hwnd).unwrap().sashes[0].clone();
        let (x, y) = (sash.rect.left + 1, (sash.rect.top + sash.rect.bottom) / 2);
        let area = super::tree_area(window.hwnd).unwrap();
        unsafe {
            SendMessageW(window.hwnd, WM_LBUTTONDOWN, 1, pack(x, y));
            SendMessageW(window.hwnd, WM_MOUSEMOVE, 1, pack(area.left, y));
            SendMessageW(window.hwnd, WM_LBUTTONUP, 0, pack(area.left, y));
        }
        let dpi = unsafe { GetDpiForWindow(window.hwnd) };
        let a = super::tree_layout(window.hwnd).unwrap().rect_of(first).unwrap();
        assert_eq!(a.right - a.left, crate::window::titlebar::scale(MIN_WIDTH_96, dpi));
        let group = app_mut(window.hwnd).group(first).unwrap().hwnd;
        let rect = window_rect_in_main(window.hwnd, group);
        assert_eq!(rect.right - rect.left, a.right - a.left);
    }

    #[test]
    fn a_double_click_on_a_sash_equalizes() {
        // Break caught: a sash double-click read as two presses and ignored.
        use windows_sys::Win32::UI::WindowsAndMessaging::{WM_LBUTTONDOWN, WM_LBUTTONUP};
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        let first = app_mut(window.hwnd).tabs.active_group();
        split_for_test(window.hwnd, Direction::Right);
        let sash = super::tree_layout(window.hwnd).unwrap().sashes[0].clone();
        app_mut(window.hwnd).layout.set_sash(&sash, sash.rect.left - 100, 96);
        super::layout_editor_and_find_bar(window.hwnd);
        let sash = super::tree_layout(window.hwnd).unwrap().sashes[0].clone();
        let (x, y) = (sash.rect.left + 1, (sash.rect.top + sash.rect.bottom) / 2);
        for _ in 0..2 {
            unsafe {
                SendMessageW(window.hwnd, WM_LBUTTONDOWN, 1, pack(x, y));
                SendMessageW(window.hwnd, WM_LBUTTONUP, 0, pack(x, y));
            }
        }
        let layout = super::tree_layout(window.hwnd).unwrap();
        let area = super::tree_area(window.hwnd).unwrap();
        let a = layout.rect_of(first).unwrap();
        assert!(((a.right - area.left) - (area.right - area.left) / 2).abs() <= 3);
    }

    #[test]
    fn only_the_group_under_the_caption_buttons_gives_them_up() {
        // Break caught: every top group cutting the caption area out of its region, leaving a
        // hole in the left group's strip.
        use windows_sys::Win32::Graphics::Gdi::{CreateRectRgn, DeleteObject, GetWindowRgn, PtInRegion};
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        let first = app_mut(window.hwnd).tabs.active_group();
        let second = split_for_test(window.hwnd, Direction::Right);
        let region_has = |group: HWND, x: i32, y: i32| unsafe {
            let region = CreateRectRgn(0, 0, 0, 0);
            let kind = GetWindowRgn(group, region);
            let inside = kind == 0 /* ERROR: no region, the whole window */ || PtInRegion(region, x, y) != 0;
            DeleteObject(region);
            inside
        };
        let a = app_mut(window.hwnd).group(first).unwrap().hwnd;
        let b = app_mut(window.hwnd).group(second).unwrap().hwnd;
        let a_rect = window_rect_in_main(window.hwnd, a);
        let b_rect = window_rect_in_main(window.hwnd, b);
        assert!(region_has(a, a_rect.right - a_rect.left - 2, 2));
        assert!(!region_has(b, b_rect.right - b_rect.left - 2, 2));
    }
```

`pack(x, y) -> LPARAM` is the module's existing point packer; reuse it under its existing name. `Direction`, `SASH_96` and `MIN_WIDTH_96` come from `crate::window::split_tree`.

Add a pure test to `group_strip.rs`:

```rust
    #[test]
    fn the_accent_marks_the_active_group_only_when_there_are_several() {
        // Break caught: one group gaining a bar 0.2.0 never had, or every group's active tab
        // looking equally active.
        let palette = Palette::neutral();
        assert_eq!(tab_accent(true, 1, &palette), None);
        assert_eq!(tab_accent(true, 2, &palette), Some(palette.editor_foreground));
        assert_eq!(tab_accent(false, 2, &palette), Some(palette.muted_foreground));
    }
```

- [ ] **Step 2: Run them to confirm they fail.**

Run: `cargo test --lib -- groups_side_by_side a_group_below dragging_a_sash a_double_click_on_a_sash only_the_group_under the_accent_marks --test-threads=1`
Expected: a compile error, because `app.layout`, `tree_layout`, `tree_area` and `tab_accent` don't exist yet.

- [ ] **Step 3: Implement.**

**`layout_editor_and_find_bar`** replaces the single `MoveWindow(group, left, 0, width, bottom)`:

```rust
    let Some(area) = tree_area(hwnd) else { return; };
    let layout = app.layout.layout(area, dpi);
    for (id, rect) in &layout.groups {
        let Some(group) = app.group(*id).map(|group| group.hwnd) else { continue; };
        // A top group reaches up into the title row and draws its strip there (spec §4.1).
        let top = if rect.top == area.top { 0 } else { rect.top };
        unsafe { MoveWindow(group, rect.left, top, rect.right - rect.left, rect.bottom - top, 1) };
        layout_group(hwnd, group);
    }
```

- `tree_area` is `(sidebar right, strip_height(dpi), client right, status bar top)`.
- A lower group's window starts at its own rectangle, and its own strip takes the first `strip_height` of it.
- `layout_group` already puts the strip at the group's top. It must only add the menu band plus name box band when the group is a top group (`origin.y == 0`).

**`set_group_region(hwnd, group, client, strip_height, band)`**:
- The caption cut becomes the intersection of the caption buttons' rectangle (`title_chrome` / `TitleBarLayout.minimize.left..client right`, rows `0..strip_height`) with the group's window rectangle, mapped into group coordinates. An empty intersection cuts nothing.
- The band cut applies to top groups only.
- `group_strip::strip_width` already stops at the caption buttons; keep it.

**`group_strip_bounds(hwnd) -> Vec<Rect>`** returns the strip rectangle of every top group, in main-client coordinates. `TitlePaint.covered` becomes `Vec<Rect>`, and `titlebar::paint` excludes each one (`ExcludeClipRect` in a loop).

**`caption_strip_point(hwnd, lparam)`** walks every top group and returns the one whose strip contains the point, with the point in its coordinates. `WM_NCLBUTTONDBLCLK` / `WM_NCRBUTTONUP` call `activate_group` for it before running New or the strip menu.

**Sashes in the main window proc:**
- `WM_SETCURSOR`: when `tree_layout(hwnd)?.sash_at(point)` hits (point from `GetCursorPos` + `ScreenToClient`), set `IDC_SIZEWE` for a row sash and `IDC_SIZENS` for a column sash and return 1. Put this before the existing arm's other checks.
- `WM_LBUTTONDOWN`, before the title-pointer code:
  - If a sash is hit and `last_sash_click` holds the same `SashId` within `GetDoubleClickTime()` of `GetMessageTime()`: `app.layout.equalize(&id)`, relayout, clear `last_sash_click`, return 0.
  - Otherwise record `last_sash_click`, set `sash_drag = Some(sash)`, `SetCapture(hwnd)`, return 0.
- `WM_MOUSEMOVE` while `sash_drag` is set: `app.layout.set_sash(&sash, x or y, dpi)`, then `layout_editor_and_find_bar`, `UpdateWindow`, return 0.
- `WM_LBUTTONUP` while `sash_drag` is set: `ReleaseCapture`, clear it, return 0. `WM_CAPTURECHANGED` clears it too.
- `WM_PAINT`: fill every sash rectangle with `palette.strip_background`. Groups cover the rest.

**Accent.** Add `accent: Option<u32>` to `StripPaint`. In `draw`, when it is `Some(color)`, fill `(tab.left, 0, tab.right, scale(2, dpi))` of the active tab with `color`. Then add:

```rust
/// The bar along the top of a group's active tab (split editors spec §4.2): only with several
/// groups, so one group looks as it always has.
pub(crate) fn tab_accent(active_group: bool, group_count: usize, palette: &Palette) -> Option<u32> {
    (group_count > 1).then_some(if active_group {
        palette.editor_foreground
    } else {
        palette.muted_foreground
    })
}
```

`paint_group` passes `tab_accent(id == app.tabs.active_group(), app.groups.len(), &palette)`.

Remove `split_tree`'s `dead_code` allowance. From here on, `reporting_group` (Task 4) uses `app.layout.leaves()`.

- [ ] **Step 4: Run the tests.**

Run: `cargo test --lib -- groups_side_by_side a_group_below dragging_a_sash a_double_click_on_a_sash only_the_group_under the_accent_marks main_window::tests::the_group main_window::tests::a_caption titlebar --test-threads=1`
Expected: all pass, including PR 1's title-row tests with one group.

- [ ] **Step 5: Clippy and commit.**

```bash
git add -A src
git commit -m "feat(split-editors): groups are laid out by the split tree, with sashes and the active-tab accent"
```

---

## Task 6: Split Right, Split Down and Close Group

Afterwards Ctrl+\ and Ctrl+Shift+\ split the active group (from the menus and the palette too), Close Group closes a group, and closing a group's last tab removes it.

**Files:**
- Modify: `src/window/commands.rs` (the enum, `COMMANDS` array :220–317 and its length, `needs_document` :123, new `group_index`, tests :339–539)
- Modify: `src/window/menus.rs` (`MenuEntry` :268, `create_popup` :279, `MenuBar::create` :143, `show_tab_strip_menu` :315, `accelerator_specs` :32 and its length, tests :546–620)
- Modify: `src/window/command_palette.rs` (`ENTRIES` :54 and its length 73, the length assert :1440, the completeness test :1478, `shortcut_text` :341)
- Modify: `src/window/main_window.rs`:
  - `execute_command_with_note` (:2790);
  - `close_reviewed_document` (:4536), `close_all_documents` (:4621), `close_active_document` (:4400), `active_close_review`;
  - new `split_active_group`, `split_group`, `close_group`, `remove_empty_group`.

**Interfaces:**
- Consumes: Tasks 1–5.
- Produces:
  - `CommandId::{SplitRight = 197, SplitDown = 198, CloseGroup = 199, FocusGroup1 = 200 … FocusGroup8 = 207, FocusLastGroup = 208, MoveTabToNextGroup = 209, MoveTabToPreviousGroup = 210}`, and `CommandId::group_index() -> Option<usize>` (0..=7 for FocusGroupN, `usize::MAX` for FocusLastGroup).
  - `MenuEntry::Submenu(&'static str, &'static [MenuEntry])`.
  - `pub(crate) const NO_ROOM_TO_SPLIT: &str = "Not enough room to split";`
  - `pub(crate) fn split_group(hwnd, target: GroupId, direction: Direction) -> Option<GroupId>`: an empty new group beside `target`, or `None` after a notice.
  - `pub(crate) fn split_active_group(hwnd, direction)`.
  - `pub(crate) fn close_group(hwnd, GroupId)`.
  - `pub(crate) fn remove_empty_group(hwnd, GroupId) -> bool`.

- [ ] **Step 1: Write the failing tests.**

In `main_window.rs`'s tests module:

```rust
    #[test]
    fn split_right_opens_the_active_document_in_a_new_group_to_the_right() {
        // Break caught: Split Right opening an empty group, a copy of the document instead of a
        // second view, or the new group landing on the left.
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        editor.set_text("shared").unwrap();
        let first = app_mut(window.hwnd).tabs.active_group();
        let id = app_mut(window.hwnd).tabs.active().unwrap().id;
        execute_command(window.hwnd, CommandId::SplitRight);
        let order = super::group_order(window.hwnd);
        assert_eq!(order.len(), 2);
        assert_eq!(order[0], first);
        let second = order[1];
        assert_eq!(app_mut(window.hwnd).tabs.active_group(), second);
        assert_eq!(app_mut(window.hwnd).tabs.views_of(id), vec![first, second]);
        assert_eq!(super::group_editor(window.hwnd, second).unwrap().text().unwrap(), "shared");
    }

    #[test]
    fn a_split_without_room_is_refused_with_a_notice() {
        // Break caught: a split into a sliver narrower than the minimum group.
        use windows_sys::Win32::UI::WindowsAndMessaging::{SWP_NOMOVE, SWP_NOZORDER, SetWindowPos};
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        unsafe { SetWindowPos(window.hwnd, std::ptr::null_mut(), 0, 0, 360, 400, SWP_NOMOVE | SWP_NOZORDER) };
        super::layout_editor_and_find_bar(window.hwnd);
        execute_command(window.hwnd, CommandId::SplitRight);
        assert_eq!(super::group_order(window.hwnd).len(), 1);
        assert!(notices(window.hwnd).iter().any(|notice| notice == super::NO_ROOM_TO_SPLIT));
    }

    #[test]
    fn a_failed_group_window_leaves_the_layout_and_tabs_unchanged() {
        // Break caught: a half-made group left in the tree when its Scintilla can't be created.
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        super::fail_next_group_creation();
        execute_command(window.hwnd, CommandId::SplitRight);
        assert_eq!(super::group_order(window.hwnd).len(), 1);
        assert_eq!(app_mut(window.hwnd).tabs.group_ids().len(), 1);
        assert!(!notices(window.hwnd).is_empty());
    }

    #[test]
    fn closing_a_group_with_a_dirty_document_shown_elsewhere_does_not_prompt() {
        // Break caught: Close Group asking to save (or discarding) a document another group
        // still shows, or leaving it clean.
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        editor.set_text("unsaved").unwrap();
        let first = app_mut(window.hwnd).tabs.active_group();
        let id = app_mut(window.hwnd).tabs.active().unwrap().id;
        assert!(app_mut(window.hwnd).tabs.document(id).unwrap().dirty);
        execute_command(window.hwnd, CommandId::SplitRight);
        fail_on_any_dialog();
        execute_command(window.hwnd, CommandId::CloseGroup);
        assert_eq!(super::group_order(window.hwnd), vec![first]);
        assert_eq!(app_mut(window.hwnd).tabs.active_group(), first);
        let document = app_mut(window.hwnd).tabs.document(id).unwrap();
        assert!(document.dirty);
        assert_eq!(super::group_editor(window.hwnd, first).unwrap().text().unwrap(), "unsaved");
    }

    #[test]
    fn closing_a_groups_last_tab_removes_the_group_but_never_the_only_one() {
        // Break caught: an empty second group left on screen, or the only group destroyed.
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        let first = app_mut(window.hwnd).tabs.active_group();
        execute_command(window.hwnd, CommandId::SplitDown);
        execute_command(window.hwnd, CommandId::CloseTab);
        assert_eq!(super::group_order(window.hwnd), vec![first]);
        execute_command(window.hwnd, CommandId::CloseAllTabs);
        assert_eq!(super::group_order(window.hwnd), vec![first]);
        assert!(app_mut(window.hwnd).groups.len() == 1);
    }
```

About the test helpers:
- `notices(hwnd)` already exists.
- `fail_on_any_dialog()` is whatever the module uses to make a prompt fail the test. Grep the dirty-close tests, e.g. `session_close_records_every_tab_without_prompting`, and use the same mechanism.
- In `a_split_without_room_is_refused_with_a_notice`, 360 px wide leaves at most 360 minus the sidebar for the tree. If the test window's sidebar is hidden and 360 still fits two 160 px groups plus a sash, use 300.

In `menus.rs` tests, update `tab_and_zoom_shortcuts_are_bound…` (or add a new test) to assert these bindings:

```rust
        assert_eq!(bound(FCONTROL, VK_OEM_5), Some(CommandId::SplitRight));
        assert_eq!(bound(FCONTROL | FSHIFT, VK_OEM_5), Some(CommandId::SplitDown));
```

In `command_palette.rs` tests, add:

```rust
    #[test]
    fn split_shortcuts_are_spelled_with_a_backslash() {
        // Break caught: Ctrl+\ shown as "Ctrl+Ü", VK_OEM_5's code read as a character.
        assert_eq!(shortcut_text(CommandId::SplitRight).as_deref(), Some("Ctrl+\\"));
        assert_eq!(shortcut_text(CommandId::SplitDown).as_deref(), Some("Ctrl+Shift+\\"));
    }
```

- [ ] **Step 2: Run them to confirm they fail.**

Run: `cargo test --lib -- split_right_opens a_split_without_room a_failed_group_window closing_a_group closing_a_groups_last_tab split_shortcuts menus::tests --test-threads=1`
Expected: a compile error on the new `CommandId` variants.

- [ ] **Step 3: Implement.**

**Commands:**
- Add the 14 variants with explicit values 197–210.
- Add them to `COMMANDS` and bump its length to 100.
- In `needs_document`, add `SplitRight | SplitDown | CloseGroup | FocusGroup1..=FocusGroup8 | FocusLastGroup` to the "doesn't need a document" list. MoveTab* still need one.
- `group_index()` mirrors `tab_index()`, with `FocusLastGroup => Some(usize::MAX)`.
- Add value tests in `commands.rs`: `SplitRight as u16 == 197`, `MoveTabToPreviousGroup as u16 == 210`, and `try_from(200) == FocusGroup1`.

**Palette:**
- Add entries in the View section, next to "View: Toggle sidebar":
  - "View: Split Editor Right" (SplitRight)
  - "View: Split Editor Down" (SplitDown)
  - "View: Close Editor Group" (CloseGroup)
  - "View: Move Editor into Next Group" (MoveTabToNextGroup)
  - "View: Move Editor into Previous Group" (MoveTabToPreviousGroup)
- The count becomes 78; update the array length and the assert.
- In the completeness test, loop `100..300u16`. Add `|| command.group_index().is_some()` to the "listed zero times" condition.
- `refilter_command_palette` hides MoveTab* when `tab_count == 0` (they need a document) and CloseGroup when there is one group with no tabs.

**`shortcut_text`:** in the key match, `VK_OEM_5 => "\\".to_owned()`, `VK_LEFT => "Left"` and `VK_RIGHT => "Right"`. If the modifiers include `FALT`, spell "Alt+" after "Ctrl+" and "Shift+" as the function already does for Ctrl and Shift; check it does, and add it if not.

**Accelerators:** `virtual_key(FCONTROL, VK_OEM_5, SplitRight)` and `virtual_key(FCONTROL | FSHIFT, VK_OEM_5, SplitDown)`. The count becomes 54; update `accelerator_specs`' length and the `specs.len()` assert.

**Menus:**
- Add `MenuEntry::Submenu(label, entries)`. `create_popup` builds the child popup recursively and appends it with `MF_POPUP`, as `append_popup` does. `MenuEntry::command` is unchanged.
- In the View menu, insert `MenuEntry::Submenu("Editor &Layout", &[..])` after the Sidebar item, followed by a separator. Its entries:
  - "Split &Right\tCtrl+\\"
  - "Split &Down\tCtrl+Shift+\\"
  - Separator
  - "Move to &Next Group\tCtrl+Alt+Right"
  - "Move to &Previous Group\tCtrl+Alt+Left"
  - Separator
  - "&Close Group"
- In the File menu, add "Close &group" (CloseGroup) after "Close a&ll tabs".
- `show_tab_strip_menu` gains a separator plus "Split Right\tCtrl+\\", "Split Down\tCtrl+Shift+\\" and "Close group".
- The "set text commands enabled" pass leaves these enabled.

**Split:**

```rust
pub(crate) const NO_ROOM_TO_SPLIT: &str = "Not enough room to split";

/// Makes an empty group beside `target`, or says why not (split editors spec §4.3, §9).
pub(crate) fn split_group(hwnd: HWND, target: GroupId, direction: Direction) -> Option<GroupId> {
    let area = tree_area(hwnd)?;
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    let app = unsafe { app_ptr(hwnd) }?;
    // A trial id: the real one is only handed out once the window exists.
    let trial = GroupId(u32::MAX);
    if !unsafe { app.as_ref() }.layout.fits_split(target, direction, trial, area, dpi) {
        push_notice(hwnd, NO_ROOM_TO_SPLIT.to_owned());
        return None;
    }
    let new = match create_group(hwnd) {
        Ok(new) => new,
        Err(error) => {
            push_notice(hwnd, format!("FastPad could not open a new editor group: {error}"));
            return None;
        }
    };
    unsafe { &mut *app.as_ptr() }.layout.split(target, direction, new);
    Some(new)
}

/// Ctrl+\ and Ctrl+Shift+\: a new group beside the active one, showing a new view of the active
/// document at the same position (spec §5.1).
pub(crate) fn split_active_group(hwnd: HWND, direction: Direction) {
    let Some(app) = (unsafe { app_ptr(hwnd) }) else { return };
    let source = unsafe { app.as_ref() }.tabs.active_group();
    remember_view(hwnd, source);
    let Some(new) = split_group(hwnd, source, direction) else { return };
    let app = unsafe { &mut *app.as_ptr() };
    if let Some(id) = app.tabs.active().map(|document| document.id) {
        let state = app.tabs.view_state_in(source, id);
        app.tabs.add_view(new, id, state);
    }
    activate_group(hwnd, new);
    show_group_view(hwnd, new);
    layout_editor_and_find_bar(hwnd);
    refresh_tabs(hwnd);
    focus_content(hwnd);
}
```

Use the notice wording the codebase already uses for window-creation failures if one exists (grep `could not`). Otherwise keep the one above.

**Close Group** (`close_group(hwnd, id)`):
1. `activate_group(hwnd, id)`.
2. Loop while the group still exists and has tabs: record `len`, run `close_active_document(hwnd)`, and stop if `len` did not fall (the user cancelled a prompt).
3. If the group is now empty and another group exists, `remove_empty_group(hwnd, id)`.

**No prompt for a document with another view.** In `close_active_document`, when `app.tabs.views_of(id).len() > 1`, close the view straight away: no autosave review, no prompt. This covers Ctrl+W, middle-click and Close Group. The last view closes as today.

**`remove_empty_group(hwnd, id) -> bool`**:
1. Refuse if the group has tabs or it is the only group.
2. Pick the neighbour: the next group in `group_order`, else the previous.
3. `app.layout.remove(id)`, `destroy_group(hwnd, id)`.
4. `activate_group(hwnd, neighbour)`, `layout_editor_and_find_bar`, `refresh_tabs`, `focus_content`.

Call it at the end of `close_reviewed_document` and of the background-close path when the group that just lost a tab is empty and not the only group.

**`close_all_documents`** captures the active group id. It loops while that group is still active and non-empty, with the same "count must fall" guard. A removed group ends the loop because the active group changes.

**Dispatch** in `execute_command_with_note`: `SplitRight => split_active_group(hwnd, Direction::Right)`, `SplitDown => split_active_group(hwnd, Direction::Down)`, `CloseGroup => close_group(hwnd, app.tabs.active_group())`.

- [ ] **Step 4: Run the tests.**

Run: `cargo test --lib -- split_right_opens a_split_without_room a_failed_group_window closing_a_group closing_a_groups_last_tab split_shortcuts commands::tests menus::tests command_palette::tests main_window::tests::tab_shortcuts main_window::tests::close --test-threads=1`
Expected: all pass.

- [ ] **Step 5: Clippy and commit.**

```bash
git add -A src
git commit -m "feat(split-editors): Split Right, Split Down and Close Group, and a group closes with its last tab"
```

---

## Task 7: Group keys, moving tabs, F6 and the tab context menu

Afterwards:
- Ctrl+1..8 focuses group N, and Ctrl+9 focuses the last group. A missing group N gets the active document split to the right of the last group.
- Alt+1..9 selects tabs.
- Ctrl+Alt+Right/Left moves the active tab between groups.
- F6 visits every group.
- A right-click on a tab opens the tab menu.

**Files:**
- Modify: `src/window/menus.rs`: `accelerator_specs` and its tests, and a new `show_tab_menu`
- Modify: `src/window/main_window.rs`:
  - `execute_command_with_note`;
  - `FocusPart` (:1394), `next_focus_part` (:1403), `cycle_focus` (:1438), `return_focus_to_editor` (:1427);
  - `group_strip_message`'s `WM_RBUTTONUP`;
  - new `focus_group_number` and `move_active_view`;
  - the tests' `translate_key` helper (:15356).
- Modify: `README.md` (the shortcut table, :161 and :221)

**Interfaces:**
- Consumes: Task 6's commands, `split_group`, `remove_empty_group`.
- Produces:
  - `FocusPart::{ActivityBar, Panel, Group(usize)}`;
  - `next_focus_part(current: FocusPart, backwards: bool, sidebar: bool, panel_open: bool, groups: usize) -> FocusPart`;
  - `pub(crate) fn focus_group_number(hwnd, index: usize)`;
  - `pub(crate) fn move_active_view(hwnd, forward: bool)`;
  - `menus::show_tab_menu(hwnd, x, y) -> Option<CommandId>`.

- [ ] **Step 1: Write the failing tests.**

In `main_window.rs`'s tests module:

```rust
    #[test]
    fn ctrl_2_focuses_group_two_and_ctrl_3_without_one_splits_right_of_the_last() {
        // Break caught: Ctrl+N still selecting tabs, or a missing group N doing nothing instead
        // of VS Code's split to the right of the last group.
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        let first = app_mut(window.hwnd).tabs.active_group();
        execute_command(window.hwnd, CommandId::SplitDown);
        let second = super::group_order(window.hwnd)[1];
        execute_command(window.hwnd, CommandId::FocusGroup1);
        assert_eq!(app_mut(window.hwnd).tabs.active_group(), first);
        execute_command(window.hwnd, CommandId::FocusGroup2);
        assert_eq!(app_mut(window.hwnd).tabs.active_group(), second);
        execute_command(window.hwnd, CommandId::FocusGroup1);
        execute_command(window.hwnd, CommandId::FocusGroup3);
        let order = super::group_order(window.hwnd);
        assert_eq!(order.len(), 3);
        assert_eq!(app_mut(window.hwnd).tabs.active_group(), order[2]);
        let area = super::tree_area(window.hwnd).unwrap();
        let layout = super::tree_layout(window.hwnd).unwrap();
        assert_eq!(layout.rect_of(order[2]).unwrap().right, area.right);
        assert_eq!(layout.rect_of(order[2]).unwrap().top, area.top, "a new column");
        execute_command(window.hwnd, CommandId::FocusLastGroup);
        assert_eq!(app_mut(window.hwnd).tabs.active_group(), order[2]);
    }

    #[test]
    fn moving_a_tab_to_the_next_group_creates_one_and_the_empty_source_closes() {
        // Break caught: Ctrl+Alt+Right doing nothing with one group, copying instead of moving,
        // or leaving the emptied group behind.
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        editor.set_text("moved").unwrap();
        let first = app_mut(window.hwnd).tabs.active_group();
        let id = app_mut(window.hwnd).tabs.active().unwrap().id;
        execute_command(window.hwnd, CommandId::MoveTabToNextGroup);
        let order = super::group_order(window.hwnd);
        assert_eq!(order.len(), 1, "the emptied first group closed");
        assert_ne!(order[0], first);
        assert_eq!(app_mut(window.hwnd).tabs.views_of(id), vec![order[0]]);
        assert_eq!(super::group_editor(window.hwnd, order[0]).unwrap().text().unwrap(), "moved");
        execute_command(window.hwnd, CommandId::MoveTabToPreviousGroup);
        assert_eq!(super::group_order(window.hwnd), order, "nothing before group 1");
    }

    #[test]
    fn alt_digits_select_tabs_and_ctrl_digits_focus_groups_through_the_table() {
        // Break caught: Alt+2 eaten by the menu band's mnemonic handling, or Ctrl+2 still bound
        // to Select Tab 2.
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        execute_command(window.hwnd, CommandId::New);
        assert_eq!(translate_key_with(window.hwnd, editor.hwnd(), b'2', true, false, false), Some(CommandId::FocusGroup2));
        assert_eq!(translate_key_with(window.hwnd, editor.hwnd(), b'2', false, false, true), Some(CommandId::SelectTab2));
    }
```

`translate_key_with(hwnd, target, key, ctrl, shift, alt) -> Option<CommandId>` generalizes the existing `translate_key` helper (:15356). It sets the key state for all three modifiers, sends `WM_SYSKEYDOWN` when Alt is down (`WM_KEYDOWN` otherwise) with the Alt context bit (bit 29) set in lparam, and reports the command `TranslateAcceleratorW` produced. Keep `translate_key` as a call to it.

Pure test next to the existing F6 tests (:17285):

```rust
    #[test]
    fn f6_visits_every_group_in_order() {
        // Break caught: F6 skipping every group after the first.
        use super::FocusPart::*;
        assert_eq!(super::next_focus_part(Group(0), false, true, true, 3), Group(1));
        assert_eq!(super::next_focus_part(Group(2), false, true, true, 3), ActivityBar);
        assert_eq!(super::next_focus_part(ActivityBar, true, true, true, 3), Group(2));
        assert_eq!(super::next_focus_part(Group(0), false, false, false, 1), Group(0));
    }
```

In `menus.rs` tests, replace the Ctrl+1/Numpad9 assertions with:

```rust
        assert_eq!(bound(FCONTROL, b'1' as u16), Some(CommandId::FocusGroup1));
        assert_eq!(bound(FCONTROL, b'9' as u16), Some(CommandId::FocusLastGroup));
        assert_eq!(bound(FCONTROL, VK_NUMPAD2), Some(CommandId::FocusGroup2));
        assert_eq!(bound(FALT, b'1' as u16), Some(CommandId::SelectTab1));
        assert_eq!(bound(FALT, VK_NUMPAD9), Some(CommandId::SelectTab9));
        assert_eq!(bound(FCONTROL | FALT, VK_RIGHT), Some(CommandId::MoveTabToNextGroup));
        assert_eq!(bound(FCONTROL | FALT, VK_LEFT), Some(CommandId::MoveTabToPreviousGroup));
```

Use the test's existing `bound` helper and its key type.

- [ ] **Step 2: Run them to confirm they fail.**

Run: `cargo test --lib -- ctrl_2_focuses moving_a_tab_to_the_next alt_digits_select f6_visits menus::tests --test-threads=1`
Expected: failures on the bindings, a compile error on `FocusPart::Group`, and the group-focus assertions.

- [ ] **Step 3: Implement.**

**Accelerators:**
- Ctrl+1..8 and Ctrl+Numpad1..8 → `FocusGroup1..8`; Ctrl+9 and Ctrl+Numpad9 → `FocusLastGroup`.
- `FALT` + `'1'..'9'` and `FALT` + `VK_NUMPAD1..9` → `SelectTab1..9`.
- `FCONTROL | FALT` + `VK_RIGHT` / `VK_LEFT` → MoveTab next/previous.
- The table grows by 20 (18 Alt digits plus 2 arrows), to 74. Update the length and the assert.
- `every_shortcut_chord_maps_to_exactly_one_command` must stay green.
- `inline_name.rs:1586`'s "not bound" list doesn't include Ctrl+Alt+arrows. Leave it.

**Alt+digit and the menu band.** `translate_accelerator` already runs `TranslateAcceleratorW` before an untranslated `Alt+char` reaches `SC_KEYMENU`. The digit's `WM_SYSKEYDOWN` clears the pending lone-Alt. No change should be needed; the test proves it.

**Dispatch:** `if let Some(index) = command.group_index() { focus_group_number(hwnd, index); return; }`, placed next to the `tab_index` check (before the needs-document guard, which these commands pass anyway). `MoveTabToNextGroup => move_active_view(hwnd, true)`, `MoveTabToPreviousGroup => move_active_view(hwnd, false)`.

**`focus_group_number(hwnd, index)`:**
- `order = group_order(hwnd)`; `usize::MAX` means the last group.
- If `index < order.len()`: `activate_group(hwnd, order[index])`, then `focus_content`.
- Otherwise:
  1. `remember_view` of the active group;
  2. `split_group(hwnd, *order.last(), Direction::Right)`;
  3. add a view of the active document (if any) with its view state;
  4. `activate_group` the new group, `show_group_view`, relayout, `refresh_tabs`, `focus_content`.

**`move_active_view(hwnd, forward)`:**
1. Get the active document `id` (or return), the `source` group and `order`.
2. The target is the next (or previous) group in `order`.
3. With no target going backwards: return. With no target going forwards: `split_group(hwnd, source, Right)` and use the new group.
4. `remember_view(hwnd, source)`, `app.tabs.move_view(source, id, target)`.
5. `show_group_view` for both groups, `activate_group(hwnd, target)`.
6. If `source` is now empty, `remove_empty_group(hwnd, source)`.
7. Relayout, `refresh_tabs`, `focus_content`.

**F6:** `FocusPart::Editor` becomes `Group(usize)` (an index in `group_order`). `next_focus_part` builds its part list from ActivityBar (if the sidebar is shown), Panel (if open), then `Group(0..groups)`, and steps with wraparound, as today. `cycle_focus` classifies a focus inside a group with `app.group_containing(GetFocus())` mapped to its index in `group_order`. The Group part is handled by `activate_group` plus `focus_content`.

**Tab context menu:** in `group_strip_message`, `WM_RBUTTONUP` on `StripTarget::Tab(i)` (or `CloseTab(i)`) calls `activate_tab(hwnd, i)`, then `menus::show_tab_menu(hwnd, x, y)` (point mapped as in `show_group_strip_menu`), then `execute_command`. The menu items:
- "&Close tab\tCtrl+W"
- "Close a&ll tabs"
- Separator
- "Split &Right\tCtrl+\\"
- "Split &Down\tCtrl+Shift+\\"
- "Move to &Next Group\tCtrl+Alt+Right"

Add a test next to the strip menu tests:

```rust
    #[test]
    fn right_clicking_a_tab_activates_it_and_runs_the_chosen_item() {
        // Break caught: the tab menu acting on the previously active tab.
        use windows_sys::Win32::UI::WindowsAndMessaging::{WM_RBUTTONDOWN, WM_RBUTTONUP};
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        execute_command(window.hwnd, CommandId::New);
        let first_tab = app_mut(window.hwnd).tabs.document_ids_for_test()[0];
        let group = app_mut(window.hwnd).active_group().unwrap().hwnd;
        let point = tab_center(window.hwnd, 0);
        answer_next_popup_menu(Some(CommandId::SplitRight));
        unsafe {
            SendMessageW(group, WM_RBUTTONDOWN, 0, point);
            SendMessageW(group, WM_RBUTTONUP, 0, point);
        }
        let order = super::group_order(window.hwnd);
        assert_eq!(order.len(), 2);
        assert_eq!(app_mut(window.hwnd).tabs.views_of(first_tab), order);
    }
```

About the helpers:
- `tab_center(hwnd, index) -> LPARAM` is the existing tab-click helper from the strip tests (grep `fn tab_center` or `fn click_tab`).
- `document_ids_for_test()` is `tabs.ids().collect::<Vec<_>>()`: `Tabs::ids` already exists under `cfg(test)`, so use that instead.

**README:** in the shortcut table and at :161, replace "Go to tab 1–9 | Ctrl+1 … Ctrl+9" with "Go to tab 1–9 | Alt+1 … Alt+9". Add rows for "Split editor right / down | Ctrl+\ / Ctrl+Shift+\", "Focus editor group 1–8 | Ctrl+1 … Ctrl+8" and "Move tab to next / previous group | Ctrl+Alt+Right / Left".

- [ ] **Step 4: Run the tests.**

Run: `cargo test --lib -- ctrl_2_focuses moving_a_tab_to_the_next alt_digits_select f6_visits right_clicking_a_tab menus::tests main_window::tests::tab_shortcuts main_window::tests::f6 main_window::tests::keyboard --test-threads=1`
Expected: all pass. `tab_shortcuts_cycle_with_wrap_around_and_select_by_position` calls `execute_command(SelectTabN)` directly, so it is unaffected.

- [ ] **Step 5: Clippy and commit.**

```bash
git add -A src README.md
git commit -m "feat(split-editors): Ctrl+1..9 focus groups, Alt+1..9 select tabs, Ctrl+Alt+arrows move tabs, F6 visits groups"
```

---

## Task 8: Opening, closing and file operations across groups

Afterwards opening a file acts on the active group (spec §5.3), and deleting, renaming or moving a note, or a change on disk, acts on every view of its document (spec §5.6).

**Files:**
- Modify: `src/window/main_window.rs`: `open_path_placed` (:3819–3908), `open_note` and its preview path (:4100–4120, :3953, :4050), `activate_document_by_id` (:4654), `close_document_tab` (:4444), `replace_in_document` and the document-host edit paths (:5102, :5193), `reload_clean_document`, `review_dirty_documents`.
- Modify: `src/window/library_host.rs`: delete (`close_document_tab` callers), rename (`rebind_path` + refresh), `refresh_label`.

**Interfaces:**
- Consumes: Tasks 2–7.
- Produces:
  - `pub(super) fn activate_document_by_id(hwnd, id) -> bool`: in the active group if it has a view of `id`, otherwise in the first group in `group_order` that has one (made active).
  - `pub(crate) fn close_document_everywhere(hwnd, id)`: closes every view of `id`, each without a prompt except the last.
  - `fn editor_showing(hwnd, id) -> Option<Editor>`: the editor of the first group in `group_order` whose active view is `id`.

- [ ] **Step 1: Write the failing tests** in `main_window.rs`'s tests module, using the `LibraryScratch` and `notebook_window` helpers from the notebook tests:

```rust
    #[test]
    fn opening_a_file_open_in_another_group_adds_a_view_in_the_active_group() {
        // Break caught: the open jumping back to group 1 (leaving group 2 where the user is
        // working) or opening a second copy of the file.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("groups-open");
        let a = scratch.note("a.md", "a");
        let b = scratch.note("b.md", "b");
        let (window, _editor) = notebook_window(&scratch);
        super::open_path(window.hwnd, &a).unwrap();
        let first = app_mut(window.hwnd).tabs.active_group();
        super::open_path(window.hwnd, &b).unwrap();
        execute_command(window.hwnd, CommandId::SplitRight);
        let second = app_mut(window.hwnd).tabs.active_group();
        super::open_path(window.hwnd, &a).unwrap();
        let id = app_mut(window.hwnd).tabs.find_path(&a).unwrap();
        assert_eq!(app_mut(window.hwnd).tabs.active_group(), second);
        assert_eq!(app_mut(window.hwnd).tabs.views_of(id), vec![first, second]);
        assert_eq!(app_mut(window.hwnd).tabs.active().unwrap().id, id);
        super::open_path(window.hwnd, &a).unwrap();
        assert_eq!(app_mut(window.hwnd).tabs.group(second).unwrap().len(), 2, "no second view in one group");
    }

    #[test]
    fn a_tree_click_replaces_only_the_active_groups_preview() {
        // Break caught: a click in the tree replacing group 1's italic tab while the user works
        // in group 2.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("groups-preview");
        let a = scratch.note("a.md", "a");
        let b = scratch.note("b.md", "b");
        let c = scratch.note("c.md", "c");
        let (window, _editor) = notebook_window(&scratch);
        super::open_note(window.hwnd, &a, super::OpenMode::Preview, false).unwrap();
        let first = app_mut(window.hwnd).tabs.active_group();
        execute_command(window.hwnd, CommandId::FocusGroup2);
        let second = app_mut(window.hwnd).tabs.active_group();
        super::open_note(window.hwnd, &b, super::OpenMode::Preview, false).unwrap();
        super::open_note(window.hwnd, &c, super::OpenMode::Preview, false).unwrap();
        let a_id = app_mut(window.hwnd).tabs.find_path(&a);
        assert!(a_id.is_some(), "group 1's preview kept");
        assert!(app_mut(window.hwnd).tabs.find_path(&b).is_none(), "group 2's preview replaced");
        assert_eq!(app_mut(window.hwnd).tabs.views_of(a_id.unwrap()), vec![first]);
        assert_eq!(app_mut(window.hwnd).tabs.group(second).unwrap().len(), 2);
    }

    #[test]
    fn deleting_a_note_open_in_two_groups_closes_both_views() {
        // Break caught: a view left open on a deleted file in the group that wasn't active.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("groups-delete");
        let a = scratch.note("a.md", "a");
        scratch.note("keep.md", "k");
        let (window, _editor) = notebook_window(&scratch);
        super::open_path(window.hwnd, &scratch.folder().join("keep.md")).unwrap();
        super::open_path(window.hwnd, &a).unwrap();
        execute_command(window.hwnd, CommandId::SplitRight);
        let id = app_mut(window.hwnd).tabs.find_path(&a).unwrap();
        crate::window::library_host::delete_note_for_test(window.hwnd, &a);
        assert!(app_mut(window.hwnd).tabs.document(id).is_none());
        assert!(app_mut(window.hwnd).tabs.views_of(id).is_empty());
    }

    #[test]
    fn a_background_replace_in_a_document_shown_in_the_other_group_counts_once() {
        // Break caught: a replace through the document host double-counting a document another
        // group's editor shows (host edit plus that editor's notifications), or moving the
        // active group's caret.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("groups-replace");
        let a = scratch.note("a.md", "alpha beta");
        let b = scratch.note("b.md", "other");
        let (window, _editor) = notebook_window(&scratch);
        super::open_path(window.hwnd, &a).unwrap();
        execute_command(window.hwnd, CommandId::SplitRight);
        super::open_path(window.hwnd, &b).unwrap();
        let first = super::group_order(window.hwnd)[0];
        execute_command(window.hwnd, CommandId::FocusGroup2);
        let id = app_mut(window.hwnd).tabs.find_path(&a).unwrap();
        let before = app_mut(window.hwnd).tabs.document(id).unwrap().generation;
        let caret = super::group_editor(window.hwnd, super::group_order(window.hwnd)[1]).unwrap().view_state();
        super::replace_in_document_for_test(window.hwnd, id, "alpha", "ALPHA");
        let document = app_mut(window.hwnd).tabs.document(id).unwrap();
        assert!(document.dirty);
        assert_eq!(document.generation, before + 1);
        assert_eq!(super::group_editor(window.hwnd, first).unwrap().text().unwrap(), "ALPHA beta");
        assert_eq!(super::group_editor(window.hwnd, super::group_order(window.hwnd)[1]).unwrap().view_state(), caret);
    }
```

About the helpers:
- `delete_note_for_test` stands for the call the existing delete tests make. Grep `NoteDelete` in the tests and reuse that sequence (select the row, run the command, confirm) rather than adding a new entry point.
- `replace_in_document_for_test` likewise: use what `the Search replace in a background tab` test from PR 1 calls. Grep `replace_in_document(` in the tests.
- In the second test, `FocusGroup2` with one group creates group 2 by splitting (Task 7). That new group shows `a`, which promotes `a` because a second view appeared. Adjust the assertions to what the rules give: `a` becomes a normal tab with views in both groups, `b` then `c` replace group 2's preview, and group 2 ends with `a` and `c`. If `FocusGroup2`'s split makes the scenario unclear, create group 2 with `split_group` directly, leaving it empty, and assert the original expectations.

- [ ] **Step 2: Run them to confirm they fail.**

Run: `cargo test --lib -- opening_a_file_open_in_another a_tree_click_replaces_only deleting_a_note_open_in_two a_background_replace_in_a_document_shown --test-threads=1`
Expected: failures on `views_of` (the open activated group 1's tab) and on the delete leaving a view.

- [ ] **Step 3: Implement.**

**`open_path_placed`** (and the preview branch of `open_note`), in place of "if `tabs.find_path(path)` exists, activate that tab":
- If the active group has a view of it: activate that view.
- Else if it is in the store (another group has it): `remember_view(active)`, `app.tabs.add_view(active, id, ViewState::default())`, `show_group_view`, `refresh_tabs`.
- Else open it into the active group as today (preview or not).

`OpenMode::Preview` on a document already open elsewhere adds a normal view, since a second view promotes it.

**`activate_document_by_id`** follows the rule in Interfaces. Every caller that "switches to the tab" (Open Editors clicks, `review_dirty_documents`, library renames) then lands in the right group.

**`close_document_everywhere(hwnd, id)`:** for each group in `views_of(id)`, activate it, activate the view and close it without a prompt. For the last view, go through `close_document_tab`, which prompts or discards as the caller already decided. Deleting a note (library_host) and "file deleted on disk" paths call it in place of `close_document_tab`.

**Rename, move and relabel:** after `rebind_path` and after a label refresh, call `app.tabs.refresh_views()` and `invalidate_group_strip` for every group.

**Edits through the document host** (`replace_in_document`, `reload_clean_document`, the snapshot adoption):
- If `editor_showing(hwnd, id)` finds an editor, make the change through **it** instead of the host. Its notifications then apply the document-level effects once, through the reporting group.
- Only when no group shows the document, edit through the host and call `tabs.note_background_edit(id)` as today.
- Either way, save and restore the active group's editor's view state around the edit when that editor isn't the one editing. The caret of the group the user is in must not move.

- [ ] **Step 4: Run the tests.**

Run: `cargo test --lib -- opening_a_file_open_in_another a_tree_click_replaces_only deleting_a_note_open_in_two a_background_replace_in_a_document_shown main_window::tests::open main_window::tests::delet main_window::tests::renam main_window::tests::search library_host --test-threads=1`
Expected: all pass.

- [ ] **Step 5: Clippy and commit.**

```bash
git add -A src
git commit -m "feat(split-editors): opening, closing, renaming and deleting act on every group's views"
```

---

## Task 9: Open Editors group headers and group-aware Ctrl+P

Afterwards Open Editors lists `Group N` headers with each group's views when there are two or more groups, and Ctrl+P's empty-query rows are views across groups, each marked with its group.

**Files:**
- Modify: `src/window/open_editors.rs` (`EditorRow` :33, `editor_rows` :52, `snapshot` :74, `OpenEditors` :119, `draw_editor_row` :153, `accessible_name`)
- Modify: `src/window/notebook_view.rs`: the editors paint (~1680–1713), `hit_test` (:1165), the left-button handling (:3064–3085), the middle-click handling (:2401–2422), `section_key` (:3362), `editors_changed` (:2039), accessibility (~3495–3525).
- Modify: `src/window/panel_cursor.rs` (`Cursor::Editor` stepping skips headers)
- Modify: `src/window/command_palette.rs` (`PickerRow` :269, `draw_quick_open_row` :952, `picker_row_label`)
- Modify: `src/window/main_window.rs` (`quick_open_rows` :1725, `run_command_palette_selection` :1997, a new `focus_view`)
- Modify: `src/window/library_host.rs` (`picked` :2710)

**Interfaces:**
- Consumes: Task 8's `activate_document_by_id`, Task 7's `group_order`.
- Produces:
  - `open_editors::EditorEntry { Header(usize), View(EditorRow) }`, where `EditorRow` gains `group: GroupId`. `OpenEditors.rows: Vec<EditorEntry>`.
  - `pub(crate) fn focus_view(hwnd, GroupId, DocumentId) -> bool`: activates that group and that view there, never adding one.
  - `PickerRow::View { found: QuickMatch, group: GroupId, number: Option<usize> }`.

- [ ] **Step 1: Write the failing tests.**

In `open_editors.rs` tests:

```rust
    #[test]
    fn several_groups_get_headers_and_one_group_stays_flat() {
        // Break caught: a "Group 1" header shown with a single group, or a document open in two
        // groups listed once.
        let a = document(1, "a.md");
        let b = document(2, "b.md");
        let one = entries(&[(GroupId(1), vec![&a, &b])], Some((GroupId(1), a.id)));
        assert!(one.iter().all(|entry| matches!(entry, EditorEntry::View(_))));
        let two = entries(
            &[(GroupId(1), vec![&a]), (GroupId(3), vec![&a, &b])],
            Some((GroupId(3), b.id)),
        );
        let shape: Vec<_> = two
            .iter()
            .map(|entry| match entry {
                EditorEntry::Header(number) => format!("G{number}"),
                EditorEntry::View(row) => format!("{}{}", row.name, if row.active { "*" } else { "" }),
            })
            .collect();
        assert_eq!(shape, vec!["G1", "a", "G2", "a", "b*"]);
    }
```

`entries(groups: &[(GroupId, Vec<&Document>)], active: Option<(GroupId, DocumentId)>) -> Vec<EditorEntry>` is the new pure builder that replaces `editor_rows`. Headers are numbered by position in the list passed in, which the caller gives in `group_order`. `document(id, name)` is the module's existing fixture (or add one alongside the existing tests).

In `main_window.rs`'s tests module:

```rust
    #[test]
    fn clicking_an_open_editors_row_focuses_that_group_and_a_header_does_nothing() {
        // Break caught: a click on group 2's row opening a copy in the active group, or a header
        // row acting like a tab.
        use windows_sys::Win32::UI::WindowsAndMessaging::{WM_LBUTTONDOWN, WM_LBUTTONUP};
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("editors-groups");
        let a = scratch.note("a.md", "a");
        let (window, _editor) = notebook_window(&scratch);
        super::open_path(window.hwnd, &a).unwrap();
        execute_command(window.hwnd, CommandId::SplitRight);
        execute_command(window.hwnd, CommandId::New);
        let first = super::group_order(window.hwnd)[0];
        let panel = sidebar_windows(window.hwnd).1;
        let header = notebook_view(window.hwnd).editor_rect_at(0).unwrap();
        mouse(panel, WM_LBUTTONDOWN, 1, centre(header));
        mouse(panel, WM_LBUTTONUP, 0, centre(header));
        assert_ne!(app_mut(window.hwnd).tabs.active_group(), first);
        let row = notebook_view(window.hwnd).editor_rect_at(1).unwrap();
        mouse(panel, WM_LBUTTONDOWN, 1, centre(row));
        mouse(panel, WM_LBUTTONUP, 0, centre(row));
        assert_eq!(app_mut(window.hwnd).tabs.active_group(), first);
        let id = app_mut(window.hwnd).tabs.find_path(&a).unwrap();
        assert_eq!(app_mut(window.hwnd).tabs.views_of(id).len(), 2, "no copy made");
    }

    #[test]
    fn ctrl_p_lists_views_in_every_group_and_picking_one_focuses_it() {
        // Break caught: the MRU rows only covering the active group, or a pick adding a view to
        // the active group instead of going to the one listed.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("quick-groups");
        let a = scratch.note("a.md", "a");
        let b = scratch.note("b.md", "b");
        let (window, _editor) = notebook_window(&scratch);
        super::open_path(window.hwnd, &a).unwrap();
        execute_command(window.hwnd, CommandId::FocusGroup2);
        super::open_path(window.hwnd, &b).unwrap();
        execute_command(window.hwnd, CommandId::FocusGroup1);
        let (rows, _) = super::quick_open_rows(window.hwnd, "");
        let groups: Vec<_> = rows
            .iter()
            .filter_map(|row| match row {
                crate::window::command_palette::PickerRow::View { number, found, .. } => {
                    Some((found.name.clone(), *number))
                }
                _ => None,
            })
            .collect();
        assert!(groups.contains(&("b".to_owned(), Some(2))), "{groups:?}");
        let b_id = app_mut(window.hwnd).tabs.find_path(&b).unwrap();
        let second = super::group_order(window.hwnd)[1];
        assert!(super::focus_view(window.hwnd, second, b_id));
        assert_eq!(app_mut(window.hwnd).tabs.active_group(), second);
        assert_eq!(app_mut(window.hwnd).tabs.views_of(b_id), vec![second]);
    }
```

`FocusGroup2` with one group splits the active document into group 2 (Task 7), so group 2 shows `a` too until `b` opens. `editor_rect_at(i)` indexes the entries, headers included. `mouse`, `centre`, `sidebar_windows` and `notebook_view` are existing helpers from the Open Editors tests (:20590).

- [ ] **Step 2: Run them to confirm they fail.**

Run: `cargo test --lib -- several_groups_get_headers clicking_an_open_editors_row_focuses ctrl_p_lists_views --test-threads=1`
Expected: a compile error on `EditorEntry` and `PickerRow::View`.

- [ ] **Step 3: Implement.**

**Open Editors:**
- `snapshot(hwnd)` walks `group_order`, collecting `(id, tabs.group_documents(id))`, and calls `entries(..)` with `(active_group, active doc)`.
- `entries` emits `Header(n)` rows only when there are two or more groups.
- `OpenEditors::set_rows` keeps selection on the active view's entry.
- The painter draws a header as the group label in the section-header style (`muted_foreground`, the same font as "OPEN EDITORS"), with no icon and no close box.
- The section's count stays the number of view rows.
- `hit_test` never reports `close` on a header.
- A left click on a header does nothing.
- A click on a view row runs `focus_view(hwnd, row.group, row.id)` (the drag arm is unchanged); the close box and middle-click activate the row's group, then `close_document_tab`.
- `panel_cursor::Shape.editors` counts entries. `section_key` Up/Down skip a header, and Enter on a header does nothing.
- Accessibility exposes a header as a static-text item named "Group N".

**`focus_view(hwnd, group, id)`:** `activate_group(hwnd, group)`, then `activate_document_in(hwnd, group, id, revision)` when that group has the view, then `focus_content`. Return false when it doesn't have the view.

**Ctrl+P:**
- `quick_open_rows` with an empty query walks `tabs.activation_order()`, keeping the same notebook filter as today.
- It emits `PickerRow::View { found, group, number }`, where `number` is `Some(group_order position + 1)` only when there are two or more groups.
- `draw_quick_open_row` draws a `View` row as a `Note` row, with "Group N" appended after the folder text in the muted colour.
- `picker_row_label` appends ", group N".
- Picking a `View` row (`library_host::picked`) calls `focus_view` instead of `open_quick_open_choice`, then applies the typed `:line`, if any, with `go_to_line`.
- Typed-query rows stay `Note` rows and open as in §5.3.

- [ ] **Step 4: Run the tests.**

Run: `cargo test --lib -- several_groups_get_headers clicking_an_open_editors_row_focuses ctrl_p_lists_views open_editors notebook_view panel_cursor command_palette main_window::tests::quick main_window::tests::the_arrow_keys_cross main_window::tests::editors --test-threads=1`
Expected: all pass.

- [ ] **Step 5: Clippy and commit.**

```bash
git add -A src
git commit -m "feat(split-editors): Open Editors shows group headers and Ctrl+P lists views across groups"
```

---

## Task 10: Multi-group session

Afterwards closing FastPad with several groups saves the layout, each group's views and every view's position. The next start brings them back, and a document shown in two groups comes back as one document with two views.

**Files:**
- Modify: `src/session.rs` (`SessionRestore` :466–516)
- Modify: `src/window/main_window.rs`: `build_session` (:6026), `begin_session_restore` (:5345), `restore_session_step` (:5308), `restore_session_entry` (:5400), `open_snapshot_tab` (:5801), `finish_session_restore` (:5474), `apply_view_state` (:5532)
- Test: `src/session.rs` tests, `main_window.rs` tests (`write_session` helper :10274)

**Interfaces:**
- Consumes: Task 1's `to_session`/`from_session`, Tasks 3–8.
- Produces:
  - `SessionRestore { groups: Vec<RestoreGroup>, layout: SessionLayout, active_group: usize, next: (usize, usize), failed: usize, placeholder: Option<DocumentId>, snapshots: Vec<(RecoveryId, DocumentId)> }`
  - `RestoreGroup { number: usize, active: usize, entries: Vec<SessionEntry>, id: Option<GroupId>, restored: Vec<Option<DocumentId>> }`
  - `SessionRestore::new(&Session, placeholder)`, `next_entry(&self) -> Option<(usize, &SessionEntry)>` (group index and entry), `record(&mut self, Option<DocumentId>)`, `active_view(group) -> Option<DocumentId>`
  - `restore_session_entry(hwnd, group: GroupId, entry) -> Option<DocumentId>`

- [ ] **Step 1: Write the failing tests.**

In `session.rs` tests, replacing `restore_progress_activates_the_saved_tab_or_the_last_restored_one` with its per-group form:

```rust
    #[test]
    fn restore_progress_walks_each_group_and_finds_each_saved_active_view() {
        // Break caught: restore flattening the groups (every view in one group), or a group
        // whose saved active view failed activating nothing.
        let session = sample();
        let mut restore = SessionRestore::new(&session, None);
        let mut seen = Vec::new();
        let mut next_id = 1;
        while let Some((group, _)) = restore.next_entry() {
            seen.push(group);
            let id = (next_id != 2).then_some(DocumentId(next_id));
            next_id += 1;
            restore.record(id);
        }
        assert_eq!(seen, vec![0, 1, 1, 2]);
        assert_eq!(restore.failed, 1);
        assert_eq!(restore.active_view(1), Some(DocumentId(3)), "saved active failed: last restored");
    }
```

Adjust `seen` to `sample()`'s real entry counts per group (read the fixture). The point is one index per entry, in file order, grouped.

In `main_window.rs`'s tests module:

```rust
    #[test]
    fn a_three_group_session_comes_back_with_its_layout_views_and_positions() {
        // Break caught: the layout flattened into one group, a view's caret lost, or a document
        // shown in two groups restored twice (or failing on its second snapshot entry).
        let _scintilla = load_native_scintilla();
        let scratch = RecoveryScratch::new("session-groups");
        let file = scratch.file("plan.txt", "one\ntwo\nthree\nfour");
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        enable_session(window.hwnd, &scratch);
        super::open_path(window.hwnd, &file).unwrap();
        editor.apply_view_state(ViewState { caret: 5, anchor: 5, first_line: 1, x_offset: 0 }).unwrap();
        execute_command(window.hwnd, CommandId::SplitRight);
        execute_command(window.hwnd, CommandId::New);
        super::group_editor(window.hwnd, super::group_order(window.hwnd)[1])
            .unwrap()
            .set_text("unsaved and shared")
            .unwrap();
        let untitled = app_mut(window.hwnd).tabs.active().unwrap().id;
        execute_command(window.hwnd, CommandId::SplitDown);
        assert_eq!(super::group_order(window.hwnd).len(), 3);
        assert!(super::save_session_for_close(window.hwnd));
        let saved = std::fs::read_to_string(scratch.session()).unwrap();
        assert!(saved.contains("layout=row(1:0.5,column(2:0.5,3:0.5):0.5)"), "{saved}");
        drop(window);

        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        enable_session(window.hwnd, &scratch);
        run_session_restore(window.hwnd);
        let order = super::group_order(window.hwnd);
        assert_eq!(order.len(), 3);
        let tabs = &app_mut(window.hwnd).tabs;
        let shared = tabs
            .documents()
            .find(|document| document.path.is_none())
            .map(|document| document.id)
            .unwrap();
        assert_ne!(shared, untitled, "a new id, but one document");
        assert_eq!(tabs.views_of(shared), vec![order[1], order[2]]);
        assert_eq!(tabs.documents().count(), 2);
        let first_editor = super::group_editor(window.hwnd, order[0]).unwrap();
        assert_eq!(first_editor.view_state().caret, 5);
        assert_eq!(app_mut(window.hwnd).tabs.active_group(), order[2]);
        assert!(notices(window.hwnd).is_empty(), "{:?}", notices(window.hwnd));
    }

    #[test]
    fn a_group_whose_files_are_gone_or_whose_window_fails_folds_into_the_others() {
        // Break caught: an empty group left in the layout after its files vanished, or a group
        // whose window can't be made losing its tabs.
        let _scintilla = load_native_scintilla();
        let scratch = RecoveryScratch::new("session-groups-gone");
        let kept = scratch.file("kept.txt", "kept");
        let other = scratch.file("other.txt", "other");
        let gone = scratch.root().join("gone.txt");
        let session = crate::session::Session {
            layout: crate::session::SessionLayout::parse("row(1:0.3,2:0.3,3:0.4)").unwrap(),
            active_group: 0,
            groups: vec![
                crate::session::SessionGroup { number: 1, active: 0, entries: vec![SessionEntry::new(SessionSource::File(kept.clone()))] },
                crate::session::SessionGroup { number: 2, active: 0, entries: vec![SessionEntry::new(SessionSource::File(gone))] },
                crate::session::SessionGroup { number: 3, active: 0, entries: vec![SessionEntry::new(SessionSource::File(other.clone()))] },
            ],
        };
        crate::session::write(&scratch.session(), &session).unwrap();
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        enable_session(window.hwnd, &scratch);
        super::fail_next_group_creation();
        run_session_restore(window.hwnd);
        let order = super::group_order(window.hwnd);
        assert_eq!(order.len(), 1, "group 2 emptied, group 3's window failed");
        let tabs = &app_mut(window.hwnd).tabs;
        assert!(tabs.find_path(&kept).is_some());
        assert!(tabs.find_path(&other).is_some(), "group 3's views went to the first group");
    }
```

About the helpers and failure injection:
- `RecoveryScratch::file`, `session()` and `root()` stand for the existing scratch helpers. Grep `struct RecoveryScratch` and use its real names.
- `fail_next_group_creation` fails the *next* `create_group`. Restore creates group 2's window first and group 3's second. To fail group 3's window, the test needs "fail the Nth creation". Make the test hook a counter, `fail_group_creation_after(n: usize)`, and have this test call it with 1.

- [ ] **Step 2: Run them to confirm they fail.**

Run: `cargo test --lib -- restore_progress_walks a_three_group_session a_group_whose_files_are_gone --test-threads=1`
Expected: a compile error (the `next_entry` shape); after that, a layout with one group.

- [ ] **Step 3: Implement.**

**`build_session(hwnd, root)`:**
- Walk `group_order`, numbering from 1.
- For each group, walk `group_documents(id)` in strip order.
- A document's source is decided once per document, as today: a snapshot of a dirty document, or the path of a clean one. So a document in two groups gets the same `file=` or `snapshot=` target under each.
- The view state is `group_editor(id).view_state()` for the group's active view, and `tabs.view_state_in(id, doc)` for the others. Images get none.
- A skipped view (clean untitled, non-UTF-8 path) moves that group's `active` as today.
- Drop groups that end up with no entries. Build `layout` with `app.layout.to_session(&|id| number_of(id))`, then remove the dropped groups with `SessionLayout::without`.
- `active_group` is the active group's index among the written groups (0 if it was dropped).

**`SessionRestore::new`** keeps the groups as they are (no `flattened()`). `next` walks group by group and entry by entry. `record` fills `groups[g].restored`. `active_view(g)` is the saved active entry's document if it was restored, else that group's last restored one.

**`begin_session_restore`:**
1. Keep the placeholder as today.
2. `groups[0].id` is the existing (active) group.
3. For each other saved group, `create_group(hwnd)`. On failure, record `id: None`; its entries will go to the first group (spec §9).
4. Build the tree: `SplitTree::from_session(&session.layout, &|number| id of the group with that number, falling back to groups[0].id)`. A fallback repeats a leaf, and `from_session` has no duplicate check, so first remove every failed group's number with `SessionLayout::without` and then convert. If the conversion still fails, `SplitTree::new(first)` and a row of the created groups in order.
5. Set `app.layout`, then `layout_editor_and_find_bar`.

**`restore_session_step`** takes `(group_index, entry)`. The target group is `groups[group_index].id.unwrap_or(groups[0].id)`; it is made active with `activate_group` before `restore_session_entry(hwnd, group, &entry)`.

**`restore_session_entry(hwnd, group, entry)`:**
- `File` → `open_path`. Task 8's §5.3 rule adds a view of an already open document to the active group, so a shared file needs nothing more.
- `Snapshot(id)` → if `restore.snapshots` already maps `id` to a document, `app.tabs.add_view(group, document, state)` and `show_group_view`. Otherwise open it as today and record `(id, new document)`. Record before `adopt_restored_snapshot` removes the source file.
- Apply the entry's view state to that group's editor (`apply_view_state(hwnd, group, entry)`); the next open's `remember_view` stores it.

**`finish_session_restore`:**
1. Close the placeholder as today.
2. For each group: `activate_document_in(group, active_view(group))` and apply the saved state if it was the saved one.
3. Remove every group left empty: `app.layout.remove`, `destroy_group`.
4. Activate the saved `active_group` (fallback: the first group).
5. `layout_editor_and_find_bar`.
6. `reset_activation_order`.
7. The failure notice, then release forwarded launches, as today.

**`write_session`** (the test helper) and every in-crate test that builds `Session::single` keep working. `Session::single` stays for them and for v1 reads.

- [ ] **Step 4: Run the tests.**

Run: `cargo test --lib -- session restore_progress_walks a_three_group_session a_group_whose_files_are_gone --test-threads=1`
Expected: all pass, including every existing `session_*` test.

Then: `cargo test --test image_preview -- --test-threads=1` (it reads `session.groups[0]`).
Expected: pass.

- [ ] **Step 5: Clippy and commit.**

```bash
git add -A src tests
git commit -m "feat(split-editors): the session saves and restores every group, its layout and every view's position"
```

---

## Task 11: End to end, spec amendments, and the full suite

**Files:**
- Modify: `tests/windows/support/win32.rs` (a new `find_children_by_class(parent, class) -> Vec<HWND>`)
- Modify: `tests/windows/session.rs` (a new test)
- Modify: `docs/superpowers/specs/2026-09-28-split-editors-design.md` (§10, and the sections the amendments touch)

- [ ] **Step 1: Write the end-to-end test** in `tests/windows/session.rs`:

```rust
#[test]
fn a_split_layout_comes_back_on_the_next_launch() {
    // Break caught: the layout written but not restored by a real process, or the second
    // group's editor never created after a restart.
    let _lock = SESSION_TEST_LOCK.lock().unwrap_or_else(|poison| poison.into_inner());
    let scratch = Scratch::new("split");
    let notes = scratch.file("notes.txt", "split text");
    let mut first = FastPadProcess::spawn_with_local_app_data([notes.as_os_str()], &scratch.root)
        .expect("spawn");
    let hwnd = first.wait_for_main_window(WAIT).expect("window");
    command(hwnd, CommandId::SplitRight);
    let deadline = Deadline::after(WAIT);
    while support::win32::find_children_by_class(hwnd, "FastPadEditorGroup").len() < 2 {
        assert!(!deadline.expired(), "the split never showed");
        deadline.sleep_step();
    }
    unsafe { PostMessageW(hwnd, WM_CLOSE, 0, 0) };
    wait_for_process_exit(first.id(), WAIT).expect("exit");

    let mut second = FastPadProcess::spawn_with_local_app_data(std::iter::empty::<&OsStr>(), &scratch.root)
        .expect("respawn");
    let hwnd = second.wait_for_main_window(WAIT).expect("window");
    let deadline = Deadline::after(WAIT);
    loop {
        let editors = support::win32::find_children_by_class(hwnd, "Scintilla");
        let texts: Vec<_> = editors.iter().filter_map(|editor| support::win32::scintilla_text(*editor).ok()).collect();
        if texts.iter().filter(|text| *text == "split text").count() == 2 {
            break;
        }
        assert!(!deadline.expired(), "restored editors: {texts:?}");
        deadline.sleep_step();
    }
    second.close();
}
```

Use the module's real names for `command`, `Scratch::file`, the lock and the `OsStr` import. `find_children_by_class` is `EnumChildWindows` collecting every child whose class matches.

- [ ] **Step 2: Run it.**

Run: `cargo test --test session -- a_split_layout --test-threads=1` (with no other FastPad running)
Expected: pass.

- [ ] **Step 3: Amend the spec.** In §10, under item 2, add "PR 2 plan-time amendments (docs/superpowers/plans/2026-09-28-split-editors-2-splits.md)" with the 15 amendments from this plan, one line each. Then edit the sections they touch so they no longer contradict it:
- §3.2: the preview flag paragraph, and views identified by (group, document).
- §3.3: `Tabs` as the façade.
- §4.2: the accent sentence.
- §4.3: the "status-bar hint" becomes a notice.
- §5.1: Ctrl+Tab in strip order; Close all tabs per group.
- §7: the tab context menu items.

- [ ] **Step 4: Run the full suite once.** Back up `%LOCALAPPDATA%\FastPad` first, and make sure the display is awake.

Run: `cargo test --all-targets --no-fail-fast -- --test-threads=1 > "$SCRATCH/full.txt" 2>&1; grep -E "^test result|FAILED|panicked" "$SCRATCH/full.txt" | tail -40`
Expected: every `test result: ok.` and no `fastpad.exe` left running (`tasklist | grep -i fastpad` prints nothing).

- [ ] **Step 5: Commit.**

```bash
git add -A tests docs
git commit -m "test(split-editors): a split layout survives a real restart; the spec records PR 2's amendments"
```

- [ ] **Step 6: Manual checks before the PR** (listed in the PR body, done by the user):
  - Narrator on two strips (each group's tabs are their own tab list).
  - High contrast: the accent and the sashes.
  - 150–200% DPI: sashes, minimum sizes, strips in the title row.
  - Split with the menu band open (Alt).
  - Drag a sash while a Markdown preview is open in both groups.
  - Close the window with a dirty untitled document in two groups; restart.
