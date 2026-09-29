# Split editors, PR 3 (drag and drop): implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Tabs can be dragged: reordered in their strip, moved or (with Ctrl) copied to another group's strip or content, dropped on a group's edge to split it, and dropped on a notebook folder to copy the file. Open Editors rows drag onto groups with the same rules, and Explorer drops open in the group under the pointer.

**Architecture:**
- A pure `src/window/group_drop.rs` decides everything that doesn't need a window: which zone of a content rectangle a point is in, and what a drop of a given tab on a given target does (`Action`). `StripLayout` gains the insertion point geometry.
- `Tabs` gains positional inserts (`add_view_at`, `move_view_at`) and `reorder`. The "one preview per group" rule moves into `add_view_at`, which also fixes PR 2's deferred "two preview tabs after a move".
- `src/window/tab_drag.rs` is the window half. It uses an in-window `SetCapture` drag on the source group window, the same pattern as the notebook tree drag, not OLE. Its state is `App.tab_drag`. `tab_drag::apply` carries out an `Action` with `main_window::place_view` and `split_group`. Open Editors row drags, which already run in the notebook panel, call the same resolver and the same `apply`.
- `src/window/drop_overlay.rs` is a layered, click-through popup that tints the drop area or draws the insertion bar. The existing `drag_label` shows the tab's icon and name.
- Explorer drops: every group's editor gets the OLE file-drop wrapper, and the posted message carries its group. `WM_DROPFILES` (a drop on a strip, a preview or an image) resolves the group from the drop point.

**Tech Stack:** Rust 2024 edition, `windows-sys` (Win32 GDI, windows and messaging, Shell, OLE), Scintilla 5.6.6 (`native/`).

**Spec:** `docs/superpowers/specs/2026-09-28-split-editors-design.md`. This plan implements its §6 and §10 item 3. It is stacked on PR #29 (`feat/split-editors-grid`), on branch `feat/split-editors-dnd`.

## Global Constraints

- **Latency:** nothing new runs before first paint. The overlay window class is registered on the first drag. Explorer drop wrappers are still installed in `BUILD_CHROME`, and after that by `create_group`.
- **One group looks like 0.2.0.** With one group, a tab drag can only reorder, or split by dropping on an edge.
- **Scintilla owns the text.** A move or copy changes views only (`Tabs`). A document is never copied or reloaded.
- **No new crates.** A new `windows-sys` feature must be added to `tools/audit-dependencies.ps1`'s allowlist in the same commit. The plan needs none: `SetLayeredWindowAttributes` and `LWA_ALPHA` are in `Win32_UI_WindowsAndMessaging`, and `DragQueryPoint` is in `Win32_UI_Shell`, both already enabled.
- **Compile gate:** `cargo clippy --all-targets --all-features -- -D warnings`.
- **Targeted tests:** `cargo test --lib -- <filter> --test-threads=1`. Window tests register window classes and must run serially.
- **The full suite runs once, at the end:** `cargo test --all-targets --no-fail-fast -- --test-threads=1`. It needs the display awake.
- **Live runs** back up and restore `%LOCALAPPDATA%\FastPad` around them.
- **No backward-compatibility readers.** Nothing here touches a file format.
- **Drag distance** is `GetSystemMetrics(SM_CXDRAG)` / `SM_CYDRAG`, through `tree_drag::past_threshold`.
- **Zones (spec §6.1):** the outer third of a group's content rectangle on each side is that side's edge zone. The rest is the middle.
- **Comments and test style:** match the surrounding code. Each test starts with a `// Break caught: …` comment naming the regression it guards against.
- Commit prefixes `feat(split-editors): …`, `refactor(split-editors): …`, `test(split-editors): …`, `fix(split-editors): …`. No attribution lines.

## Plan-time amendments to the spec

Record these in the spec's §10, under item 3 as "PR 3 plan-time amendments", as part of Task 9.

1. **Corners** (§6.1): "nearer" is measured in pixels, from the point to each edge whose outer third contains it. On a tie the top or bottom edge wins, since those are the horizontal edges.
2. **Ctrl on its own group:** in its own strip, a Ctrl drag reorders; there's no second view of a document in one group. On its own content's middle, it does nothing.
3. **One preview per group** (§3.2, PR 2 amendment 3): a view that arrives in a group already holding a different preview tab is promoted to a normal tab. This covers drops, Ctrl+Alt+Left/Right and Open Editors drags.
4. **Overlay colours:** an area tint in `palette.selection_background` at alpha 80 of 255. The strip's insertion bar is 2 px (scaled) of `palette.editor_foreground`, opaque.
5. **Open Editors untitled rows** can now be dragged. They drop on groups only: the tree refuses them because there is no file to copy.
6. **Explorer drops:** a drop on an editor goes to that editor's group. A drop that reaches the main window's `WM_DROPFILES` (a strip, a preview, an image view) goes to the group under the drop point, and otherwise to the active group.
7. **Keys during a started tab drag** are swallowed. Esc cancels. Ctrl down or up re-targets, switching between move and copy.
8. **A right press cancels a tab drag** and keeps the capture until its own release, as the tree drag does, so the release never opens a menu.
9. **A tab drag starts** from a press on a tab's body, not its close button. A tab that closes mid-drag makes the drop do nothing.

## Review Focus

These are the conditions most likely to hurt a user that no task's feature test covers directly. Each has a test in the task named in brackets.

1. **A press and release on a tab with a small wobble** (under the drag distance) is still a click. It activates the tab, a double-click still keeps a preview tab, and the close button still closes. [Task 4]
2. **A drag that ends unexpectedly:** the dragged tab closes mid-drag (a file deleted on disk, autosave), or the capture is lost (Alt+Tab, a dialog). The drop does nothing, and the label and overlay are gone. [Task 4, Task 5]
3. **Moving a preview (italic) tab into a group that already has a preview** leaves exactly one preview there. This was a deferred minor from PR 2. [Task 2]
4. **Dragging a group's last tab:** onto its own group's middle or edge, nothing happens. Onto another group, the source group closes and the focus is in the target. [Task 5]
5. **An edge drop without room to split** shows the "Not enough room to split" notice and leaves the tab where it was: not lost, not moved. [Task 5]

---

## Task 1: Drop decisions and strip insertion points

Afterwards `group_drop.rs` holds the pure rules of spec §6.1–§6.2, and `StripLayout` knows where an insertion point is.

**Files:**
- Create: `src/window/group_drop.rs`
- Modify: `src/window/mod.rs` (add `pub(crate) mod group_drop;` after `group_strip`)
- Modify: `src/window/group_strip.rs` (`impl StripLayout`, after `bounds`)

**Interfaces:**
- Consumes: `crate::document::DocumentId`; `super::split_tree::{Direction, GroupId}`; `super::titlebar::{Point, Rect}`.
- Produces:
  - `pub(crate) enum Zone { Middle, Edge(Direction) }` (Copy, Eq, Debug).
  - `pub(crate) fn zone(content: Rect, point: Point) -> Zone`.
  - `pub(crate) enum Target { Strip { group: GroupId, index: usize }, Content { group: GroupId, zone: Zone } }` (Copy, Eq, Debug).
  - `pub(crate) struct Source { pub(crate) group: GroupId, pub(crate) document: DocumentId, pub(crate) index: usize, pub(crate) group_len: usize }` (Copy, Eq, Debug).
  - `pub(crate) enum Action { Reorder { group: GroupId, from: usize, to: usize }, Place { group: GroupId, index: Option<usize>, copy: bool }, Split { group: GroupId, direction: Direction, copy: bool } }` (Copy, Eq, Debug).
  - `pub(crate) fn decide(source: Source, target: Target, copy: bool) -> Option<Action>`.
  - `pub(crate) fn reorder_destination(from: usize, insertion: usize) -> Option<usize>`.
  - `StripLayout::insertion_index(&self, x: i32) -> usize` and `StripLayout::insertion_x(&self, index: usize) -> i32`.

- [ ] **Step 1: Write the failing tests.** Create `src/window/group_drop.rs` with only this test module, plus `pub(crate) mod group_drop;` in `src/window/mod.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::{Action, Source, Target, Zone, decide, reorder_destination, zone};
    use crate::document::DocumentId;
    use crate::window::split_tree::{Direction, GroupId};
    use crate::window::titlebar::{Point, Rect};

    const CONTENT: Rect = Rect::new(0, 0, 900, 300);

    fn source(group: u32, index: usize, group_len: usize) -> Source {
        Source {
            group: GroupId(group),
            document: DocumentId(7),
            index,
            group_len,
        }
    }

    #[test]
    fn the_outer_thirds_are_edges_and_the_rest_is_the_middle() {
        // Break caught: halves instead of thirds, so every drop off-centre splits.
        assert_eq!(zone(CONTENT, Point::new(450, 150)), Zone::Middle);
        assert_eq!(zone(CONTENT, Point::new(10, 150)), Zone::Edge(Direction::Left));
        assert_eq!(zone(CONTENT, Point::new(890, 150)), Zone::Edge(Direction::Right));
        assert_eq!(zone(CONTENT, Point::new(450, 10)), Zone::Edge(Direction::Up));
        assert_eq!(zone(CONTENT, Point::new(450, 290)), Zone::Edge(Direction::Down));
        // Just inside and just outside the left third (300 px).
        assert_eq!(zone(CONTENT, Point::new(299, 150)), Zone::Edge(Direction::Left));
        assert_eq!(zone(CONTENT, Point::new(300, 150)), Zone::Middle);
    }

    #[test]
    fn in_a_corner_the_nearer_edge_wins_and_a_tie_goes_to_top_or_bottom() {
        // Break caught: a corner always splitting sideways, or the tie flipping between runs.
        assert_eq!(zone(CONTENT, Point::new(5, 50)), Zone::Edge(Direction::Left));
        assert_eq!(zone(CONTENT, Point::new(50, 5)), Zone::Edge(Direction::Up));
        assert_eq!(zone(CONTENT, Point::new(20, 20)), Zone::Edge(Direction::Up));
        assert_eq!(zone(CONTENT, Point::new(879, 279)), Zone::Edge(Direction::Down));
    }

    #[test]
    fn a_zone_is_found_in_an_offset_rectangle() {
        // Break caught: zones measured from the window origin instead of the content's.
        let content = Rect::new(200, 100, 500, 400);
        assert_eq!(zone(content, Point::new(350, 250)), Zone::Middle);
        assert_eq!(zone(content, Point::new(210, 250)), Zone::Edge(Direction::Left));
    }

    #[test]
    fn reordering_onto_its_own_place_is_nothing() {
        // Break caught: a click-sized drag in the strip reshuffling tabs.
        assert_eq!(reorder_destination(2, 2), None);
        assert_eq!(reorder_destination(2, 3), None);
        assert_eq!(reorder_destination(2, 0), Some(0));
        assert_eq!(reorder_destination(0, 3), Some(2));
    }

    #[test]
    fn its_own_strip_reorders_even_with_ctrl() {
        // Break caught: Ctrl in its own strip making a second view of a document in one group.
        let target = Target::Strip {
            group: GroupId(1),
            index: 0,
        };
        let expected = Some(Action::Reorder {
            group: GroupId(1),
            from: 2,
            to: 0,
        });
        assert_eq!(decide(source(1, 2, 3), target, false), expected);
        assert_eq!(decide(source(1, 2, 3), target, true), expected);
        let same = Target::Strip {
            group: GroupId(1),
            index: 3,
        };
        assert_eq!(decide(source(1, 2, 3), same, false), None);
    }

    #[test]
    fn another_strip_places_at_the_insertion_point_moving_or_copying() {
        // Break caught: Ctrl ignored, or the insertion point dropped for the end of the strip.
        let target = Target::Strip {
            group: GroupId(2),
            index: 1,
        };
        assert_eq!(
            decide(source(1, 0, 1), target, false),
            Some(Action::Place {
                group: GroupId(2),
                index: Some(1),
                copy: false
            })
        );
        assert_eq!(
            decide(source(1, 0, 1), target, true),
            Some(Action::Place {
                group: GroupId(2),
                index: Some(1),
                copy: true
            })
        );
    }

    #[test]
    fn a_content_middle_appends_to_another_group_and_is_nothing_on_its_own() {
        // Break caught: a drop back onto its own editor moving the tab to the end.
        let other = Target::Content {
            group: GroupId(2),
            zone: Zone::Middle,
        };
        assert_eq!(
            decide(source(1, 0, 2), other, false),
            Some(Action::Place {
                group: GroupId(2),
                index: None,
                copy: false
            })
        );
        let own = Target::Content {
            group: GroupId(1),
            zone: Zone::Middle,
        };
        assert_eq!(decide(source(1, 0, 2), own, false), None);
        assert_eq!(decide(source(1, 0, 2), own, true), None);
    }

    #[test]
    fn an_edge_splits_except_its_own_group_holding_only_that_tab_without_ctrl() {
        // Break caught: dragging a lone tab to its own edge closing its group and reopening it
        // beside the hole it left.
        let own_edge = Target::Content {
            group: GroupId(1),
            zone: Zone::Edge(Direction::Right),
        };
        assert_eq!(decide(source(1, 0, 1), own_edge, false), None);
        assert_eq!(
            decide(source(1, 0, 1), own_edge, true),
            Some(Action::Split {
                group: GroupId(1),
                direction: Direction::Right,
                copy: true
            })
        );
        assert_eq!(
            decide(source(1, 0, 2), own_edge, false),
            Some(Action::Split {
                group: GroupId(1),
                direction: Direction::Right,
                copy: false
            })
        );
        let other_edge = Target::Content {
            group: GroupId(2),
            zone: Zone::Edge(Direction::Down),
        };
        assert_eq!(
            decide(source(1, 0, 1), other_edge, false),
            Some(Action::Split {
                group: GroupId(2),
                direction: Direction::Down,
                copy: false
            })
        );
    }
}
```

At the end of `group_strip.rs`'s existing tests module, add:

```rust
    #[test]
    fn the_insertion_point_is_the_nearest_tab_boundary() {
        // Break caught: a drop on the right half of a tab landing before it.
        let layout = StripLayout::calculate(1200, 96, 3, 0);
        let width = layout.tab(0).unwrap().right;
        assert_eq!(layout.insertion_index(-5), 0);
        assert_eq!(layout.insertion_index(width / 2 - 1), 0);
        assert_eq!(layout.insertion_index(width / 2 + 1), 1);
        assert_eq!(layout.insertion_index(10_000), 3);
        assert_eq!(layout.insertion_x(0), 0);
        assert_eq!(layout.insertion_x(1), width);
        assert_eq!(StripLayout::calculate(1200, 96, 0, 0).insertion_index(300), 0);
    }

    #[test]
    fn the_insertion_point_follows_the_scroll_and_its_bar_stays_in_view() {
        // Break caught: insertion points computed as if the strip were not scrolled.
        let layout = StripLayout::calculate(300, 96, 10, 100);
        let width = layout.tab(1).unwrap().right - layout.tab(1).unwrap().left;
        assert_eq!(layout.insertion_index(0), ((100 + width / 2) / width) as usize);
        assert_eq!(layout.insertion_x(0), 0, "clamped to the viewport");
        assert_eq!(layout.insertion_x(10), layout.tabs.right);
    }
```

- [ ] **Step 2: Run them to confirm they fail.**

Run: `cargo test --lib -- group_drop:: group_strip:: --test-threads=1`
Expected: FAIL to compile. `zone`, `decide`, `insertion_index` and the others don't exist yet.

- [ ] **Step 3: Implement.** Put this above the test module in `group_drop.rs`:

```rust
//! Where a dragged tab would land and what dropping it there does (split editors spec §6.1, §6.2).
//! Pure: no window. The window half is `tab_drag`.

use crate::document::DocumentId;
use crate::window::split_tree::{Direction, GroupId};
use crate::window::titlebar::{Point, Rect};

/// Which part of a group's content a point is in (spec §6.1).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Zone {
    Middle,
    /// The outer third on that side: a drop there splits the group that way.
    Edge(Direction),
}

/// What a drag's pointer is over.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Target {
    /// Insertion point `index` (0 to the tab count) in `group`'s strip.
    Strip { group: GroupId, index: usize },
    Content { group: GroupId, zone: Zone },
}

/// The dragged view: its group, its document, its strip index and how many tabs its group has.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Source {
    pub(crate) group: GroupId,
    pub(crate) document: DocumentId,
    pub(crate) index: usize,
    pub(crate) group_len: usize,
}

/// What a drop does.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Action {
    /// Moves the tab at `from` to `to` in its own strip.
    Reorder { group: GroupId, from: usize, to: usize },
    /// Puts the view into `group` at strip `index` (`None`: the end), moving it or, with `copy`,
    /// adding a second view. A group that already shows the document activates that view.
    Place {
        group: GroupId,
        index: Option<usize>,
        copy: bool,
    },
    /// Splits `group` towards `direction` and puts the view in the new group.
    Split {
        group: GroupId,
        direction: Direction,
        copy: bool,
    },
}

/// The zone of `content` that `point` is in: an edge's outer third, else the middle. In a corner
/// the nearer edge wins, in pixels; on a tie the top or bottom edge (plan amendment 1).
pub(crate) fn zone(content: Rect, point: Point) -> Zone {
    let width = (content.right - content.left).max(1);
    let height = (content.bottom - content.top).max(1);
    // Top and bottom first, so they win a tie.
    let edges = [
        (Direction::Up, point.y - content.top, height),
        (Direction::Down, content.bottom - 1 - point.y, height),
        (Direction::Left, point.x - content.left, width),
        (Direction::Right, content.right - 1 - point.x, width),
    ];
    let mut nearest: Option<(Direction, i32)> = None;
    for (direction, distance, extent) in edges {
        if i64::from(distance) * 3 >= i64::from(extent) {
            continue;
        }
        if nearest.is_none_or(|(_, best)| distance < best) {
            nearest = Some((direction, distance));
        }
    }
    nearest.map_or(Zone::Middle, |(direction, _)| Zone::Edge(direction))
}

/// The index a tab at `from` ends up at when dropped at insertion point `insertion` of its own
/// strip; `None` when that is where it already is.
pub(crate) fn reorder_destination(from: usize, insertion: usize) -> Option<usize> {
    let to = if insertion > from {
        insertion - 1
    } else {
        insertion
    };
    (to != from).then_some(to)
}

/// What dropping `source` on `target` does, `copy` being Ctrl; `None` for a drop that does
/// nothing (spec §6.1, §6.2, plan amendment 2).
pub(crate) fn decide(source: Source, target: Target, copy: bool) -> Option<Action> {
    match target {
        Target::Strip { group, index } if group == source.group => {
            reorder_destination(source.index, index).map(|to| Action::Reorder {
                group,
                from: source.index,
                to,
            })
        }
        Target::Strip { group, index } => Some(Action::Place {
            group,
            index: Some(index),
            copy,
        }),
        Target::Content {
            zone: Zone::Middle,
            group,
        } if group == source.group => None,
        Target::Content {
            zone: Zone::Middle,
            group,
        } => Some(Action::Place {
            group,
            index: None,
            copy,
        }),
        Target::Content {
            zone: Zone::Edge(_),
            group,
        } if group == source.group && source.group_len == 1 && !copy => None,
        Target::Content {
            zone: Zone::Edge(direction),
            group,
        } => Some(Action::Split {
            group,
            direction,
            copy,
        }),
    }
}
```

In `group_strip.rs`, after `bounds`:

```rust
    /// The insertion point (0 to the tab count) nearest strip `x`: the boundary between two tabs
    /// the point is closest to (split editors spec §6.1).
    pub fn insertion_index(&self, x: i32) -> usize {
        if self.tab_width == 0 {
            return 0;
        }
        let count = self.tab_rects.len() as i32;
        (x + self.scroll + self.tab_width / 2)
            .div_euclid(self.tab_width)
            .clamp(0, count) as usize
    }

    /// Where insertion point `index`'s bar is drawn, kept inside the tab viewport.
    pub fn insertion_x(&self, index: usize) -> i32 {
        (index as i32 * self.tab_width - self.scroll).clamp(self.tabs.left, self.tabs.right)
    }
```

- [ ] **Step 4: Run the tests.**

Run: `cargo test --lib -- group_drop:: group_strip:: --test-threads=1`
Expected: PASS.

- [ ] **Step 5: Clippy and commit.**

Run: `cargo clippy --all-targets --all-features -- -D warnings`
Expected: no warnings. Until Task 4 uses the new items they are dead code; add `#![allow(dead_code)]` at the top of `group_drop.rs` with the comment `// Wired into the window in Task 4.` Task 4 removes it.

```bash
git add src/window/group_drop.rs src/window/mod.rs src/window/group_strip.rs
git commit -m "feat(split-editors): drop zones, drop decisions and strip insertion points"
```

---

## Task 2: Positional views, reordering, and one preview per group

Afterwards `Tabs` can put a view at a strip position and reorder a strip. A view arriving in a group that already has a different preview is promoted: that's plan amendment 3, and it fixes PR 2's deferred minor.

**Files:**
- Modify: `src/window/tabs.rs` (`add_view` ~354, `move_view` ~385, new `reorder`, tests at the end)

**Interfaces:**
- Consumes: nothing new.
- Produces:
  - `Tabs::add_view_at(&mut self, group: GroupId, id: DocumentId, state: ViewState, index: Option<usize>) -> bool`. `add_view` becomes `self.add_view_at(group, id, state, None)`.
  - `Tabs::move_view_at(&mut self, from: GroupId, id: DocumentId, to: GroupId, index: Option<usize>) -> bool`. `move_view` becomes `self.move_view_at(from, id, to, None)`.
  - `Tabs::reorder(&mut self, group: GroupId, from: usize, to: usize) -> bool`. The active *document* stays selected.

- [ ] **Step 1: Write the failing tests** at the end of `tabs.rs`'s tests module:

```rust
    #[test]
    fn a_view_goes_in_at_the_insertion_point() {
        // Break caught: a tab dropped between two tabs landing at the end of the strip.
        let mut tabs = Tabs::new();
        let first = tabs.active_group();
        tabs.push(document(1)).unwrap();
        let second = tabs.add_group();
        tabs.set_active_group(second);
        tabs.push(document(2)).unwrap();
        tabs.push(document(3)).unwrap();
        assert!(tabs.move_view_at(first, DocumentId(1), second, Some(1)));
        assert_eq!(
            tabs.group(second).unwrap().document_ids(),
            [DocumentId(2), DocumentId(1), DocumentId(3)]
        );
        assert_eq!(tabs.group(second).unwrap().active_document(), Some(DocumentId(1)));
        assert!(tabs.group(first).unwrap().is_empty());
        assert!(tabs.add_view_at(first, DocumentId(3), ViewState::default(), Some(9)));
        assert_eq!(tabs.group(first).unwrap().document_ids(), [DocumentId(3)]);
    }

    #[test]
    fn a_view_already_in_the_target_is_selected_where_it_is() {
        // Break caught: a second view of one document in one group (spec §6.2).
        let mut tabs = Tabs::new();
        let first = tabs.active_group();
        tabs.push(document(1)).unwrap();
        let second = tabs.add_group();
        assert!(tabs.add_view(second, DocumentId(1), ViewState::default()));
        tabs.set_active_group(second);
        tabs.push(document(2)).unwrap();
        assert!(tabs.move_view_at(first, DocumentId(1), second, Some(2)));
        assert_eq!(
            tabs.group(second).unwrap().document_ids(),
            [DocumentId(1), DocumentId(2)]
        );
        assert_eq!(tabs.group(second).unwrap().active_document(), Some(DocumentId(1)));
        assert!(tabs.group(first).unwrap().is_empty(), "a move still removes the source view");
    }

    #[test]
    fn reordering_moves_the_tab_and_keeps_the_active_document_selected() {
        // Break caught: the selection index left behind, so the strip highlights one tab while
        // the editor shows another.
        let mut tabs = Tabs::new();
        let group = tabs.active_group();
        for id in 1..=3 {
            tabs.push(document(id)).unwrap();
        }
        tabs.activate(DocumentId(2)).unwrap();
        assert!(tabs.reorder(group, 0, 2));
        assert_eq!(
            tabs.group(group).unwrap().document_ids(),
            [DocumentId(2), DocumentId(3), DocumentId(1)]
        );
        assert_eq!(tabs.active().unwrap().id, DocumentId(2));
        assert!(!tabs.reorder(group, 0, 3), "out of range");
    }

    #[test]
    fn a_preview_arriving_where_a_preview_already_is_becomes_a_normal_tab() {
        // Break caught (Review Focus 3): two italic tabs in one group, so the next tree click
        // replaces one of them and the other lingers forever.
        let mut tabs = Tabs::new();
        let first = tabs.active_group();
        tabs.push(preview(1)).unwrap();
        let second = tabs.add_group();
        tabs.set_active_group(second);
        tabs.push(preview(2)).unwrap();
        assert!(tabs.move_view(first, DocumentId(1), second));
        assert!(!tabs.document(DocumentId(1)).unwrap().preview);
        assert!(tabs.document(DocumentId(2)).unwrap().preview);
        assert_eq!(tabs.preview_id(), Some(DocumentId(2)));
    }

    #[test]
    fn a_preview_moved_into_a_group_without_one_stays_a_preview() {
        // Break caught: every move pinning its tab, so a quick look can never be replaced.
        let mut tabs = Tabs::new();
        let first = tabs.active_group();
        tabs.push(preview(1)).unwrap();
        let second = tabs.add_group();
        assert!(tabs.move_view(first, DocumentId(1), second));
        assert!(tabs.document(DocumentId(1)).unwrap().preview);
    }
```

- [ ] **Step 2: Run them to confirm they fail.**

Run: `cargo test --lib -- tabs::tests --test-threads=1`
Expected: FAIL to compile (`move_view_at`, `add_view_at` and `reorder` don't exist).

- [ ] **Step 3: Implement.** Replace `add_view` and `move_view` with:

```rust
    /// Shows the open document `id` in `group` too, selected there. A document with two or more
    /// views is never a preview (plan amendment 3). False for an unknown group or document.
    pub(crate) fn add_view(&mut self, group: GroupId, id: DocumentId, state: ViewState) -> bool {
        self.add_view_at(group, id, state, None)
    }

    /// `add_view` with a new view going in at strip `index` (clamped; `None`: the end). A group
    /// that already shows `id` selects that view where it is. A preview arriving where another
    /// preview already is becomes a normal tab: one preview per group (PR 3 amendment 3).
    pub(crate) fn add_view_at(
        &mut self,
        group: GroupId,
        id: DocumentId,
        state: ViewState,
        index: Option<usize>,
    ) -> bool {
        let Some(group_index) = self.group_index(group) else {
            return false;
        };
        if self.store.get(id).is_none() {
            return false;
        }
        let target = &mut self.groups[group_index];
        let position = match target.position(id) {
            Some(position) => position,
            None => {
                let at = index.unwrap_or(target.tabs.len()).min(target.tabs.len());
                target.tabs.insert(
                    at,
                    EditorTab {
                        document: id,
                        view_state: state,
                    },
                );
                at
            }
        };
        target.select(position);
        let other_preview = self.groups[group_index].tabs.iter().any(|tab| {
            tab.document != id
                && self
                    .store
                    .get(tab.document)
                    .is_some_and(|document| document.preview)
        });
        self.touch_in(group, id);
        if (self.views_of(id).len() > 1 || other_preview)
            && let Some(document) = self.store.get_mut(id)
        {
            document.preview = false;
        }
        self.refresh_views();
        true
    }

    /// Moves the view of `id` from one group to another, keeping its view state. When `to`
    /// already shows `id`, its view is selected and the moved one is dropped.
    pub(crate) fn move_view(&mut self, from: GroupId, id: DocumentId, to: GroupId) -> bool {
        self.move_view_at(from, id, to, None)
    }

    /// `move_view` into strip position `index` of `to` (`None`: the end).
    pub(crate) fn move_view_at(
        &mut self,
        from: GroupId,
        id: DocumentId,
        to: GroupId,
        index: Option<usize>,
    ) -> bool {
        if from == to || self.group_index(to).is_none() {
            return false;
        }
        let Some(source) = self.group_index(from) else {
            return false;
        };
        let Some(position) = self.groups[source].position(id) else {
            return false;
        };
        let tab = self.groups[source].tabs.remove(position);
        let selected = self.groups[source].active_index();
        if position < selected {
            self.groups[source]
                .selection
                .active
                .store(selected - 1, Ordering::Release);
        } else {
            self.groups[source].select_after_removal(position);
        }
        self.recent.retain(|recent| *recent != (from, id));
        self.add_view_at(to, id, tab.view_state, index)
    }

    /// Moves `group`'s tab at `from` to `to` in its strip. The document that was active stays
    /// selected. False for an unknown group or an index past the end.
    pub(crate) fn reorder(&mut self, group: GroupId, from: usize, to: usize) -> bool {
        let Some(index) = self.group_index(group) else {
            return false;
        };
        let target = &mut self.groups[index];
        if from >= target.tabs.len() || to >= target.tabs.len() {
            return false;
        }
        let active = target.active_document();
        let tab = target.tabs.remove(from);
        target.tabs.insert(to, tab);
        if let Some(position) = active.and_then(|active| target.position(active)) {
            target.select(position);
        }
        self.refresh_views();
        true
    }
```

`move_view_at` is today's `move_view` body with its last line calling `add_view_at` with `index`.

- [ ] **Step 4: Run the tests.**

Run: `cargo test --lib -- tabs:: --test-threads=1`
Expected: PASS, including the existing `moving_a_view_keeps_its_state_and_the_document` and `a_second_view_promotes_a_preview_and_a_preview_is_replaced_only_in_its_group`.

- [ ] **Step 5: Clippy and commit.**

Run: `cargo clippy --all-targets --all-features -- -D warnings`
Expected: no warnings.

```bash
git add src/window/tabs.rs
git commit -m "feat(split-editors): views go in at a strip position, strips reorder, one preview per group"
```

---

## Task 3: The drop overlay and a tab's drag label

Afterwards `DropOverlay` shows a translucent rectangle over the screen, and a tab's drag label can be painted without the notebook view.

**Files:**
- Create: `src/window/drop_overlay.rs`
- Modify: `src/window/mod.rs` (add `pub(crate) mod drop_overlay;` after `drag_label`)
- Modify: `src/window/notebook_view.rs` (a new `pub(crate) fn tab_label_image` next to `drag_label_image` ~1313)
- Modify: `src/window/icon_sets/images.rs` only if `IconImages::new` isn't already `pub(crate)` (it is at line 29)
- Test: `src/window/main_window.rs` tests module

**Interfaces:**
- Consumes: `super::side_panel::view_paint(main, window, hdc, client) -> ViewPaint`; `paint_drag_label(dc, size, item, name, paint, images)`; `drag_label_size(text, dpi)`; `super::drag_label::LabelImage`.
- Produces:
  - `pub(crate) struct DropOverlay` with `show(owner: HWND, rect: RECT, color: u32, alpha: u8) -> Option<Self>`, `place(&self, rect: RECT, color: u32, alpha: u8)`, `destroy(self)`, `rect(&self) -> RECT` (screen), `#[cfg(test)] hwnd(&self) -> HWND`. It implements `Clone, Copy` and a hand-written `Debug`.
  - `pub(crate) const TINT_ALPHA: u8 = 80; pub(crate) const BAR_ALPHA: u8 = 255;`
  - `notebook_view::tab_label_image(hwnd: HWND, window: HWND, item: TreeItem, name: &str) -> Option<LabelImage>`.

- [ ] **Step 1: Write the failing tests** in `main_window.rs`'s tests module:

```rust
    #[test]
    fn the_drop_overlay_covers_its_rectangle_and_lets_the_pointer_through() {
        // Break caught: an overlay that steals the drag's clicks, or lands off by the frame.
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GWL_EXSTYLE, GetWindowRect, WS_EX_LAYERED, WS_EX_TRANSPARENT,
        };
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let rect = RECT {
            left: 100,
            top: 120,
            right: 300,
            bottom: 220,
        };
        let overlay = crate::window::drop_overlay::DropOverlay::show(
            window.hwnd,
            rect,
            0x00ff_0000,
            crate::window::drop_overlay::TINT_ALPHA,
        )
        .expect("overlay");
        let mut shown = RECT::default();
        unsafe { GetWindowRect(overlay.hwnd(), &mut shown) };
        assert_eq!((shown.left, shown.top, shown.right, shown.bottom), (100, 120, 300, 220));
        let style = unsafe { GetWindowLongPtrW(overlay.hwnd(), GWL_EXSTYLE) } as u32;
        assert_ne!(style & WS_EX_LAYERED, 0);
        assert_ne!(style & WS_EX_TRANSPARENT, 0);
        let moved = RECT {
            left: 10,
            top: 20,
            right: 30,
            bottom: 40,
        };
        overlay.place(moved, 0x0000_ff00, crate::window::drop_overlay::BAR_ALPHA);
        assert_eq!(overlay.rect().right, 30);
        let hwnd = overlay.hwnd();
        overlay.destroy();
        assert_eq!(unsafe { super::IsWindow(hwnd) }, 0);
    }

    #[test]
    fn a_tab_label_is_painted_without_the_sidebar() {
        // Break caught: the tab drag reaching into the notebook view, which is gone when the
        // sidebar is hidden.
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        let group = super::group_hwnd(window.hwnd).unwrap();
        let image = crate::window::notebook_view::tab_label_image(
            window.hwnd,
            group,
            crate::library::tree::TreeItem::Note(crate::window::file_icons::NoteKind::Text),
            "notes.txt",
        )
        .expect("label image");
        assert!(image.size.cx > image.size.cy);
    }
```

If `IsWindow` isn't imported in the `super` scope, qualify it as `windows_sys::Win32::UI::WindowsAndMessaging::IsWindow`. The same goes for `RECT` and `GetWindowLongPtrW`: use whatever the tests module already imports.

- [ ] **Step 2: Run them to confirm they fail.**

Run: `cargo test --lib -- drop_overlay a_tab_label_is_painted --test-threads=1`
Expected: FAIL to compile (`drop_overlay` and `tab_label_image` don't exist yet).

- [ ] **Step 3: Implement.** `src/window/drop_overlay.rs`:

```rust
//! The drop overlay (split editors spec §6): a layered, click-through popup over the rectangle a
//! dragged tab would end up in, or a thin bar at a strip's insertion point. The same technique
//! as `drag_label`, painted with one flat colour.

use crate::platform::wide_null;
use std::sync::atomic::{AtomicBool, Ordering};
use windows_sys::Win32::Foundation::{
    ERROR_CLASS_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, RECT, WPARAM,
};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, DeleteObject, EndPaint, FillRect, InvalidateRect, PAINTSTRUCT,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GWLP_USERDATA, GetWindowLongPtrW,
    GetWindowRect, HTTRANSPARENT, LWA_ALPHA, RegisterClassW, SW_SHOWNOACTIVATE,
    SWP_NOACTIVATE, SWP_NOZORDER, SetLayeredWindowAttributes, SetWindowLongPtrW, SetWindowPos,
    ShowWindow, WM_ERASEBKGND, WM_NCHITTEST, WM_PAINT, WNDCLASSW, WS_EX_LAYERED,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT, WS_POPUP,
};

const CLASS: &str = "FastPadDropOverlay";
/// An area's tint: the target shows through (PR 3 amendment 4).
pub(crate) const TINT_ALPHA: u8 = 80;
/// The strip's insertion bar: opaque.
pub(crate) const BAR_ALPHA: u8 = 255;

/// The overlay popup while a drag has a target.
#[derive(Clone, Copy)]
pub(crate) struct DropOverlay {
    hwnd: HWND,
}

impl std::fmt::Debug for DropOverlay {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("DropOverlay").finish_non_exhaustive()
    }
}

impl DropOverlay {
    /// Shows `color` over screen rectangle `rect` at `alpha`, owned by `owner`. `None` if the
    /// popup can't be made: the drag goes on without it. Call it with nothing of the App borrowed.
    pub(crate) fn show(owner: HWND, rect: RECT, color: u32, alpha: u8) -> Option<Self> {
        let instance = unsafe { GetModuleHandleW(std::ptr::null()) };
        let class = wide_null(CLASS);
        static REGISTERED: AtomicBool = AtomicBool::new(false);
        if !REGISTERED.load(Ordering::Relaxed) {
            let window_class = WNDCLASSW {
                lpfnWndProc: Some(overlay_proc),
                hInstance: instance,
                lpszClassName: class.as_ptr(),
                ..Default::default()
            };
            if unsafe { RegisterClassW(&window_class) } == 0
                && unsafe { GetLastError() } != ERROR_CLASS_ALREADY_EXISTS
            {
                return None;
            }
            REGISTERED.store(true, Ordering::Relaxed);
        }
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
                class.as_ptr(),
                std::ptr::null(),
                WS_POPUP,
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
                owner,
                std::ptr::null_mut(),
                instance,
                std::ptr::null(),
            )
        };
        if hwnd.is_null() {
            return None;
        }
        let overlay = Self { hwnd };
        overlay.place(rect, color, alpha);
        unsafe { ShowWindow(hwnd, SW_SHOWNOACTIVATE) };
        Some(overlay)
    }

    /// Moves the overlay to screen rectangle `rect` and repaints it in `color` at `alpha`.
    pub(crate) fn place(&self, rect: RECT, color: u32, alpha: u8) {
        unsafe {
            SetWindowLongPtrW(self.hwnd, GWLP_USERDATA, color as isize);
            SetLayeredWindowAttributes(self.hwnd, 0, alpha, LWA_ALPHA);
            SetWindowPos(
                self.hwnd,
                std::ptr::null_mut(),
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            InvalidateRect(self.hwnd, std::ptr::null(), 0);
        }
    }

    /// Where the overlay is, on the screen.
    pub(crate) fn rect(&self) -> RECT {
        let mut rect = RECT::default();
        unsafe { GetWindowRect(self.hwnd, &mut rect) };
        rect
    }

    /// Destroys the popup. Call it with nothing of the App borrowed.
    pub(crate) fn destroy(self) {
        unsafe { DestroyWindow(self.hwnd) };
    }

    #[cfg(test)]
    pub(crate) fn hwnd(&self) -> HWND {
        self.hwnd
    }
}

unsafe extern "system" fn overlay_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_NCHITTEST => HTTRANSPARENT as LRESULT,
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            let mut paint = PAINTSTRUCT::default();
            let dc = unsafe { BeginPaint(hwnd, &mut paint) };
            let color = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as u32;
            let brush = unsafe { CreateSolidBrush(color) };
            unsafe {
                FillRect(dc, &paint.rcPaint, brush);
                DeleteObject(brush);
                EndPaint(hwnd, &paint);
            }
            0
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}
```

In `notebook_view.rs`, next to `drag_label_image`:

```rust
/// A tab's drag label for window `window` (a group window): its icon and name, painted like a
/// tree drag's (split editors spec §6). `None` if GDI can't make the image. Needs no notebook
/// view, so it works with the sidebar hidden.
pub(crate) fn tab_label_image(
    hwnd: HWND,
    window: HWND,
    item: TreeItem,
    name: &str,
) -> Option<LabelImage> {
    let paint = super::side_panel::view_paint(hwnd, window, std::ptr::null_mut(), RECT::default());
    let wide = name.encode_utf16().collect::<Vec<_>>();
    let mut extent = SIZE::default();
    if !wide.is_empty() {
        unsafe {
            let dc = GetDC(window);
            if dc.is_null() {
                return None;
            }
            let previous = SelectObject(dc, paint.fonts.text);
            GetTextExtentPoint32W(dc, wide.as_ptr(), wide.len() as i32, &mut extent);
            SelectObject(dc, previous);
            ReleaseDC(window, dc);
        }
    }
    let size = drag_label_size(extent.cx.min(scale(LABEL_MAX_TEXT, paint.dpi)), paint.dpi);
    let image = LabelImage::new(size.cx, size.cy)?;
    let mut images = crate::window::icon_sets::images::IconImages::new();
    paint_drag_label(image.dc, size, item, name, &paint, &mut images);
    Some(image)
}
```

Use the module path to `IconImages` that `notebook_view.rs` already imports. If it doesn't import one, `crate::window::icon_sets::images::IconImages` is where it lives; make `images` `pub(crate)` in `icon_sets/mod.rs` if it isn't.

- [ ] **Step 4: Run the tests.**

Run: `cargo test --lib -- drop_overlay a_tab_label_is_painted --test-threads=1`
Expected: PASS.

- [ ] **Step 5: Clippy and commit.** Until Task 4, keep `drop_overlay`'s and `tab_label_image`'s unused-warning quiet with `#[allow(dead_code)] // Used by tab drags from Task 4.` on the items that need it.

Run: `cargo clippy --all-targets --all-features -- -D warnings`
Expected: no warnings.

```bash
git add src/window/drop_overlay.rs src/window/mod.rs src/window/notebook_view.rs src/window/main_window.rs
git commit -m "feat(split-editors): a drop overlay and a tab drag label"
```

---

## Task 4: Tab drags: start, cancel, and reorder in the strip

Afterwards a left press on a tab followed by movement past the drag distance starts a drag. The group window has the capture, the label follows the pointer, and the overlay marks the insertion point in the strip under the pointer. A release in the strip reorders. Esc, a right press or a lost capture cancels. A press and release without that movement is a click, as today. Over a group's content the overlay tints the drop zone: the whole content for a middle drop, the half the new group would take near an edge (outer third), and nothing where the drop would do nothing. Drops outside the strip resolve and show that feedback but do nothing yet; Task 5 applies them.

**Files:**
- Create: `src/window/tab_drag.rs`
- Modify: `src/window/mod.rs` (add `pub(crate) mod tab_drag;` after `tabs`)
- Modify: `src/app.rs` (new `App` fields `tab_drag` and `drop_overlay`)
- Modify: `src/window/editor_group.rs` (`GroupWindow.content: RECT`)
- Modify: `src/window/main_window.rs`:
  - `layout_group` (~1428): store `area` in `content`;
  - `group_strip_message` (~3060): arm, move, release, right press and capture;
  - `translate_accelerator` (~7752): keys during a drag;
  - `destroy_group`: cancel a drag from that group;
  - make `update_strip_pointer`, `refresh_tabs`, `strip_target` and `invalidate_group_strip` `pub(crate)`;
  - new `group_at` and `place_view`; `move_active_view` uses `place_view`.
- Modify: `src/window/group_drop.rs` (remove the `dead_code` allow)
- Test: `src/window/main_window.rs` tests module

**Interfaces:**
- Consumes: Task 1's `group_drop::{zone, decide, Source, Target, Action, Zone}` and `StripLayout::{insertion_index, insertion_x}`; Task 2's `Tabs::{reorder, add_view_at, move_view_at}`; Task 3's `DropOverlay`, `TINT_ALPHA`, `BAR_ALPHA` and `notebook_view::tab_label_image`; the existing `drag_label::DragLabel`, `tree_drag::past_threshold`, `main_window::{with_group_id, group_id_of, strip_layout_of, remember_view, focus_view, activate_group, show_group_view, remove_empty_group, split_group, layout_editor_and_find_bar, focus_content, current_palette}`.
- Produces:
  - `pub(crate) struct TabDrag { pub(crate) source: Source, pub(crate) window: HWND, pub(crate) origin: (i32, i32), pub(crate) started: bool, pub(crate) action: Option<Action>, pub(crate) label: Option<DragLabel>, pub(crate) eat_right_up: bool }`, with a hand-written `Debug`.
  - `App.tab_drag: Option<TabDrag>` and `App.drop_overlay: Option<DropOverlay>`, both `None` in `App::new`.
  - `GroupWindow.content: RECT`: the group's content area in group-client coordinates, as `layout_group` last laid it out.
  - `main_window::group_at(hwnd: HWND, point: POINT) -> Option<(GroupId, HWND)>`: the group whose window contains screen `point`.
  - `main_window::place_view(hwnd: HWND, from: GroupId, id: DocumentId, to: GroupId, index: Option<usize>, copy: bool) -> bool`.
  - `tab_drag::target_at(hwnd: HWND, point: POINT) -> Option<Target>`
  - `tab_drag::source_of(hwnd: HWND, group: GroupId, id: DocumentId) -> Option<Source>`
  - `tab_drag::show_feedback(hwnd: HWND, target: Option<Target>, action: Option<Action>)` and `tab_drag::hide_feedback(hwnd: HWND)`
  - `tab_drag::apply(hwnd: HWND, source: Source, action: Action)`
  - `tab_drag::arm(hwnd: HWND, group: GroupId, window: HWND, index: usize, x: i32, y: i32)`
  - `tab_drag::mouse_move(hwnd: HWND, window: HWND, x: i32, y: i32, buttons: WPARAM) -> bool`
  - `tab_drag::release(hwnd: HWND, window: HWND, x: i32, y: i32, buttons: WPARAM) -> bool`
  - `tab_drag::cancel(hwnd: HWND) -> bool`, `tab_drag::cancel_for_right_press(hwnd: HWND) -> bool`, `tab_drag::right_release(hwnd: HWND) -> bool`
  - `tab_drag::keeps_key(hwnd: HWND, message: &MSG) -> bool`

- [ ] **Step 1: Write the failing tests** in `main_window.rs`'s tests module. Add these helpers first:

```rust
    /// Group `from`'s window, and a press on its tab `index` followed by a move past the drag
    /// distance.
    fn start_strip_drag(hwnd: HWND, from: GroupId, index: usize) -> HWND {
        use windows_sys::Win32::UI::WindowsAndMessaging::{WM_LBUTTONDOWN, WM_MOUSEMOVE};
        let window = super::with_group_id(hwnd, from, |state| state.hwnd).unwrap();
        let tab = super::strip_layout_of(hwnd, from)
            .unwrap()
            .tab(index)
            .unwrap()
            .center();
        unsafe {
            SendMessageW(window, WM_LBUTTONDOWN, 1, client_lparam(tab.x, tab.y));
            SendMessageW(window, WM_MOUSEMOVE, 1, client_lparam(tab.x + 30, tab.y));
        }
        window
    }

    /// Window `to`'s client point (`x`, `y`) in window `from`'s client coordinates, as an
    /// `lParam`: where the source group, which has the capture, sees the pointer.
    fn lparam_in(from: HWND, to: HWND, x: i32, y: i32) -> super::LPARAM {
        let mut point = windows_sys::Win32::Foundation::POINT { x, y };
        unsafe { windows_sys::Win32::Graphics::Gdi::MapWindowPoints(to, from, &mut point, 1) };
        client_lparam(point.x, point.y)
    }

    /// Moves the drag to window `to`'s client point and releases there; `buttons` carries
    /// `MK_CONTROL` for a copy.
    fn drop_strip_drag(from: HWND, to: HWND, x: i32, y: i32, buttons: usize) {
        use windows_sys::Win32::UI::WindowsAndMessaging::{WM_LBUTTONUP, WM_MOUSEMOVE};
        let point = lparam_in(from, to, x, y);
        unsafe {
            SendMessageW(from, WM_MOUSEMOVE, 1 | buttons, point);
            SendMessageW(from, WM_LBUTTONUP, buttons, point);
        }
    }

    fn strip_ids(hwnd: HWND, group: GroupId) -> Vec<DocumentId> {
        app_mut(hwnd).tabs.group(group).unwrap().document_ids()
    }
```

Then the tests:

```rust
    #[test]
    fn a_tab_dragged_along_its_strip_moves_there_and_stays_active() {
        // Break caught: a strip drag that does nothing, or reorders but leaves the editor on
        // another tab.
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        execute_command(window.hwnd, CommandId::New);
        execute_command(window.hwnd, CommandId::New);
        let group = app_mut(window.hwnd).tabs.active_group();
        let ids = strip_ids(window.hwnd, group);
        let source = start_strip_drag(window.hwnd, group, 0);
        assert!(app_mut(window.hwnd).tab_drag.as_ref().is_some_and(|drag| drag.started));
        let layout = super::strip_layout_of(window.hwnd, group).unwrap();
        let end = layout.tab(2).unwrap();
        drop_strip_drag(source, source, end.right - 2, end.center().y, 0);
        assert_eq!(strip_ids(window.hwnd, group), [ids[1], ids[2], ids[0]]);
        assert_eq!(app_mut(window.hwnd).tabs.active().unwrap().id, ids[0]);
        assert!(app_mut(window.hwnd).tab_drag.is_none());
        assert!(app_mut(window.hwnd).drop_overlay.is_none());
    }

    #[test]
    fn a_wobble_under_the_drag_distance_is_still_a_click() {
        // Break caught (Review Focus 1): a slightly shaky click starting a drag, so the tab
        // never activates.
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE,
        };
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        execute_command(window.hwnd, CommandId::New);
        let group = app_mut(window.hwnd).tabs.active_group();
        let first = strip_ids(window.hwnd, group)[0];
        let tab = super::strip_layout_of(window.hwnd, group)
            .unwrap()
            .tab(0)
            .unwrap()
            .center();
        let source = super::with_group_id(window.hwnd, group, |state| state.hwnd).unwrap();
        unsafe {
            SendMessageW(source, WM_LBUTTONDOWN, 1, client_lparam(tab.x, tab.y));
            SendMessageW(source, WM_MOUSEMOVE, 1, client_lparam(tab.x + 1, tab.y));
            SendMessageW(source, WM_LBUTTONUP, 0, client_lparam(tab.x + 1, tab.y));
        }
        assert_eq!(app_mut(window.hwnd).tabs.active().unwrap().id, first);
        assert!(app_mut(window.hwnd).tab_drag.is_none());
    }

    #[test]
    fn a_press_on_a_tabs_close_button_never_starts_a_drag() {
        // Break caught: a jittery click on × dragging the tab instead of closing it.
        use windows_sys::Win32::UI::WindowsAndMessaging::{WM_LBUTTONDOWN, WM_MOUSEMOVE};
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        let group = app_mut(window.hwnd).tabs.active_group();
        let close = super::strip_layout_of(window.hwnd, group)
            .unwrap()
            .close_tab(0)
            .unwrap()
            .center();
        let source = super::with_group_id(window.hwnd, group, |state| state.hwnd).unwrap();
        unsafe {
            SendMessageW(source, WM_LBUTTONDOWN, 1, client_lparam(close.x, close.y));
            SendMessageW(source, WM_MOUSEMOVE, 1, client_lparam(close.x - 40, close.y));
        }
        assert!(app_mut(window.hwnd).tab_drag.as_ref().is_none_or(|drag| !drag.started));
    }

    #[test]
    fn esc_cancels_a_tab_drag_and_takes_its_label_and_overlay() {
        // Break caught: Esc typed into the editor while a tab drag hangs on the pointer.
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE;
        use windows_sys::Win32::UI::WindowsAndMessaging::{MSG, WM_KEYDOWN};
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        execute_command(window.hwnd, CommandId::New);
        let group = app_mut(window.hwnd).tabs.active_group();
        let ids = strip_ids(window.hwnd, group);
        start_strip_drag(window.hwnd, group, 0);
        let escape = MSG {
            hwnd: editor.hwnd(),
            message: WM_KEYDOWN,
            wParam: VK_ESCAPE as usize,
            ..Default::default()
        };
        assert!(crate::window::tab_drag::keeps_key(window.hwnd, &escape));
        assert!(app_mut(window.hwnd).tab_drag.is_none());
        assert!(app_mut(window.hwnd).drop_overlay.is_none());
        assert_eq!(strip_ids(window.hwnd, group), ids);
        assert_eq!(unsafe { windows_sys::Win32::UI::Input::KeyboardAndMouse::GetCapture() }, std::ptr::null_mut());
    }

    #[test]
    fn a_lost_capture_cancels_a_tab_drag() {
        // Break caught (Review Focus 2): Alt+Tab mid-drag leaving the label on screen and the
        // next click dropping the tab somewhere.
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        execute_command(window.hwnd, CommandId::New);
        let group = app_mut(window.hwnd).tabs.active_group();
        start_strip_drag(window.hwnd, group, 0);
        unsafe { ReleaseCapture() };
        assert!(app_mut(window.hwnd).tab_drag.is_none());
        assert!(app_mut(window.hwnd).drop_overlay.is_none());
    }

    #[test]
    fn a_right_press_cancels_a_tab_drag_and_its_release_opens_no_menu() {
        // Break caught: the right release after a cancel falling through to the strip's menu.
        use windows_sys::Win32::UI::WindowsAndMessaging::{WM_RBUTTONDOWN, WM_RBUTTONUP};
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        execute_command(window.hwnd, CommandId::New);
        let group = app_mut(window.hwnd).tabs.active_group();
        let source = start_strip_drag(window.hwnd, group, 0);
        unsafe { SendMessageW(source, WM_RBUTTONDOWN, 2, client_lparam(20, 10)) };
        assert!(app_mut(window.hwnd).tab_drag.as_ref().is_some_and(|drag| drag.eat_right_up));
        unsafe { SendMessageW(source, WM_RBUTTONUP, 0, client_lparam(20, 10)) };
        assert!(app_mut(window.hwnd).tab_drag.is_none());
        assert!(crate::window::menus::take_last_popup().is_none());
    }

    #[test]
    fn the_insertion_bar_shows_over_the_strip_under_the_pointer() {
        // Break caught: no feedback until the drop, so the user cannot see where the tab lands.
        use windows_sys::Win32::UI::WindowsAndMessaging::WM_MOUSEMOVE;
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        execute_command(window.hwnd, CommandId::New);
        execute_command(window.hwnd, CommandId::New);
        let group = app_mut(window.hwnd).tabs.active_group();
        let source = start_strip_drag(window.hwnd, group, 0);
        let layout = super::strip_layout_of(window.hwnd, group).unwrap();
        let x = layout.insertion_x(2);
        unsafe { SendMessageW(source, WM_MOUSEMOVE, 1, client_lparam(x + 1, 10)) };
        let overlay = app_mut(window.hwnd).drop_overlay.expect("an insertion bar");
        let mut origin = windows_sys::Win32::Foundation::POINT { x, y: 0 };
        unsafe { windows_sys::Win32::Graphics::Gdi::ClientToScreen(source, &mut origin) };
        let rect = overlay.rect();
        assert!((rect.left - origin.x).abs() <= 2, "{rect:?} vs {origin:?}");
        assert_eq!(rect.bottom - rect.top, layout.height);
        crate::window::tab_drag::cancel(window.hwnd);
    }

    #[test]
    fn near_a_content_edge_the_half_the_new_group_takes_is_tinted() {
        // Break caught: no zone highlight near the edges, a tint over the whole group for an edge
        // (the user cannot tell a split from a move), the wrong half, or a tint where the drop
        // does nothing.
        use windows_sys::Win32::UI::WindowsAndMessaging::WM_MOUSEMOVE;
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        execute_command(window.hwnd, CommandId::New);
        let group = app_mut(window.hwnd).tabs.active_group();
        let area = super::with_group_id(window.hwnd, group, |state| state.content).unwrap();
        let screen = |source: HWND, rect: RECT| {
            let mut corners = [
                windows_sys::Win32::Foundation::POINT { x: rect.left, y: rect.top },
                windows_sys::Win32::Foundation::POINT { x: rect.right, y: rect.bottom },
            ];
            unsafe {
                windows_sys::Win32::Graphics::Gdi::MapWindowPoints(
                    source,
                    std::ptr::null_mut(),
                    corners.as_mut_ptr(),
                    2,
                )
            };
            (corners[0].x, corners[0].y, corners[1].x, corners[1].y)
        };
        let tint = || {
            app_mut(window.hwnd).drop_overlay.map(|overlay| {
                let rect = overlay.rect();
                (rect.left, rect.top, rect.right, rect.bottom)
            })
        };
        let (width, height) = (area.right - area.left, area.bottom - area.top);
        let source = start_strip_drag(window.hwnd, group, 0);

        // The right edge: the right half.
        let point = client_lparam(area.right - 5, (area.top + area.bottom) / 2);
        unsafe { SendMessageW(source, WM_MOUSEMOVE, 1, point) };
        assert_eq!(tint(), Some(screen(source, RECT { left: area.right - width / 2, ..area })));
        // The bottom edge: the bottom half; the zone follows the pointer.
        let point = client_lparam((area.left + area.right) / 2, area.bottom - 5);
        unsafe { SendMessageW(source, WM_MOUSEMOVE, 1, point) };
        assert_eq!(tint(), Some(screen(source, RECT { top: area.bottom - height / 2, ..area })));
        crate::window::tab_drag::cancel(window.hwnd);
        assert!(app_mut(window.hwnd).drop_overlay.is_none());

        // A lone tab over its own edge would do nothing: no tint.
        execute_command(window.hwnd, CommandId::CloseTab);
        let source = start_strip_drag(window.hwnd, group, 0);
        let point = client_lparam(area.right - 5, (area.top + area.bottom) / 2);
        unsafe { SendMessageW(source, WM_MOUSEMOVE, 1, point) };
        assert!(tint().is_none());
        crate::window::tab_drag::cancel(window.hwnd);
    }
```

If `menus::take_last_popup` doesn't exist, assert what the existing right-click tests assert. Look for the helper they use to record a shown menu (`grep -n "show_tab_strip_menu" src/window/menus.rs`) and use the same one.

- [ ] **Step 2: Run them to confirm they fail.**

Run: `cargo test --lib -- tab_drag a_tab_dragged_along a_wobble a_press_on_a_tabs_close esc_cancels_a_tab a_lost_capture_cancels a_right_press_cancels_a_tab the_insertion_bar near_a_content_edge --test-threads=1`
Expected: FAIL to compile (`tab_drag`, `App.tab_drag` and `App.drop_overlay` don't exist yet).

- [ ] **Step 3: Implement.**

`src/app.rs`: after `last_sash_click`:

```rust
    /// A tab pressed on a strip, and dragged once past the drag distance (split editors spec §6).
    pub(crate) tab_drag: Option<crate::window::tab_drag::TabDrag>,
    /// The drop overlay of the drag under way: a tab drag or an Open Editors row drag.
    pub(crate) drop_overlay: Option<crate::window::drop_overlay::DropOverlay>,
```

Also initialise both to `None` in `App::new` next to `sash_drag: None`.

`editor_group.rs`: add `pub(crate) content: windows_sys::Win32::Foundation::RECT,` to `GroupWindow`, with the doc comment `/// The content area (below the strip, band and find bar), in group-client coordinates, as last laid out.`. Initialise it with `Default::default()`.

`main_window.rs`:

1. In `layout_group`, right after `let area = RECT { … };`, add `with_group_id(hwnd, id, |state| state.content = area);`.
2. Add `pub(crate)` to `update_strip_pointer`, `strip_target`, `invalidate_group_strip` and `refresh_tabs`.
3. Add, next to `group_id_of`:

```rust
/// The group whose window contains screen point `point`, with that window.
pub(crate) fn group_at(hwnd: HWND, point: windows_sys::Win32::Foundation::POINT) -> Option<(GroupId, HWND)> {
    let windows = unsafe { app_ptr(hwnd) }.map(|app| {
        unsafe { app.as_ref() }
            .groups
            .iter()
            .map(|group| (group.id, group.hwnd))
            .collect::<Vec<_>>()
    })?;
    windows.into_iter().find(|(_, window)| {
        let mut rect = RECT::default();
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::IsWindowVisible(*window) != 0
                && GetWindowRect(*window, &mut rect) != 0
        } && point.x >= rect.left
            && point.x < rect.right
            && point.y >= rect.top
            && point.y < rect.bottom
    })
}
```

4. Add `place_view` above `move_active_view`, and make `move_active_view` end with `place_view(hwnd, source, id, target, None, false);` in place of its lines from `remember_view(hwnd, source);` to `focus_content(hwnd);`:

```rust
/// Puts document `id`'s view from group `from` into group `to` at strip `index` (`None`: the
/// end): moved, or with `copy` a second view at the same position. A group that already shows
/// `id` activates that view, and a move still removes the source view (spec §6.2). The source
/// group closes when that was its last tab; the focus goes to `to`.
pub(crate) fn place_view(
    hwnd: HWND,
    from: GroupId,
    id: DocumentId,
    to: GroupId,
    index: Option<usize>,
    copy: bool,
) -> bool {
    remember_view(hwnd, from);
    let placed = unsafe { app_ptr(hwnd) }.is_some_and(|mut app| {
        let tabs = &mut unsafe { app.as_mut() }.tabs;
        if copy {
            let state = tabs.view_state_in(from, id);
            tabs.add_view_at(to, id, state, index)
        } else {
            tabs.move_view_at(from, id, to, index)
        }
    });
    if !placed {
        return false;
    }
    activate_group(hwnd, to);
    show_group_view(hwnd, from);
    show_group_view(hwnd, to);
    remove_empty_group(hwnd, from);
    layout_editor_and_find_bar(hwnd);
    refresh_tabs(hwnd);
    focus_content(hwnd);
    true
}
```

5. In `group_strip_message`:
   - `WM_MOUSEMOVE`: straight after the `drag_tab_thumb` check, add `if crate::window::tab_drag::mouse_move(hwnd, group, x, y, wparam) { return Some(0); }`.
   - `WM_LBUTTONDOWN | WM_LBUTTONDBLCLK`: after the pointer press, add `if let StripTarget::Tab(index) = target { crate::window::tab_drag::arm(hwnd, id, group, index, x, y); }`.
   - `WM_LBUTTONUP`: before the `update_strip_pointer` release, add `if crate::window::tab_drag::release(hwnd, group, x, y, wparam) { update_strip_pointer(hwnd, id, |pointer| pointer.press(None)); return Some(0); }`.
   - A new first arm, `WM_RBUTTONDOWN if crate::window::tab_drag::cancel_for_right_press(hwnd) => Some(0),`.
   - Put `WM_RBUTTONUP if crate::window::tab_drag::right_release(hwnd) => Some(0),` before the two existing `WM_RBUTTONUP` arms.
   - `WM_CAPTURECHANGED`: add `if lparam as HWND != group { crate::window::tab_drag::cancel(hwnd); }`. `lParam` is the window gaining the capture; `tab_drag` takes its state before releasing, so its own `ReleaseCapture` finds nothing to cancel.

   `group_proc` sends `WM_RBUTTONDOWN` to `press_group_window` first; leave that as it is.
6. In `translate_accelerator`, first thing after the liveness check: `if crate::window::tab_drag::keeps_key(hwnd, message) { return true; }`.
7. In `destroy_group(hwnd, id)`, first: `if unsafe { app_ptr(hwnd) }.is_some_and(|app| unsafe { app.as_ref() }.tab_drag.as_ref().is_some_and(|drag| drag.source.group == id)) { crate::window::tab_drag::cancel(hwnd); }`.

`src/window/tab_drag.rs`:

```rust
//! Dragging a tab (split editors spec §6): a press on a tab arms a drag; movement past the drag
//! distance starts it, with the capture on the group window, the tab's label following the
//! pointer and the drop overlay over where it would land. `group_drop` decides; this module
//! resolves the pointer, shows the feedback and carries the drop out. Open Editors row drags,
//! which the notebook panel runs, use `target_at`, `show_feedback` and `apply` too.

use crate::document::DocumentId;
use crate::window::drag_label::DragLabel;
use crate::window::drop_overlay::{BAR_ALPHA, DropOverlay, TINT_ALPHA};
use crate::window::group_drop::{self, Action, Source, Target, Zone};
use crate::window::main_window::{self, app_ptr};
use crate::window::split_tree::{Direction, GroupId};
use crate::window::titlebar::{Point, Rect, scale};
use windows_sys::Win32::Foundation::{HWND, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{ClientToScreen, MapWindowPoints, ScreenToClient};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetCapture, GetKeyState, ReleaseCapture, SetCapture, VK_CONTROL, VK_ESCAPE,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, IDC_ARROW, IDC_NO, LoadCursorW, MK_CONTROL, MK_LBUTTON, MSG,
    SM_CXDRAG, SM_CYDRAG, SetCursor, WM_CHAR, WM_KEYDOWN, WM_KEYUP, WM_SYSCHAR, WM_SYSKEYDOWN,
    WM_SYSKEYUP,
};

/// A tab drag: armed by the press, `started` once past the drag distance.
#[derive(Clone, Copy)]
pub(crate) struct TabDrag {
    pub(crate) source: Source,
    /// The group window the press went down in; it has the capture once the drag starts.
    pub(crate) window: HWND,
    /// Where the press went down, in that window's client coordinates.
    pub(crate) origin: (i32, i32),
    pub(crate) started: bool,
    /// What a release where the pointer last was would do.
    pub(crate) action: Option<Action>,
    /// The last pointer position, on the screen, so Ctrl can re-target without a move.
    pub(crate) pointer: POINT,
    pub(crate) label: Option<DragLabel>,
    /// A right press cancelled the drag; the capture stays until its release (amendment 8).
    pub(crate) eat_right_up: bool,
}

impl std::fmt::Debug for TabDrag {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TabDrag")
            .field("source", &self.source)
            .field("started", &self.started)
            .field("action", &self.action)
            .field("eat_right_up", &self.eat_right_up)
            .finish_non_exhaustive()
    }
}

fn with_drag<R>(hwnd: HWND, f: impl FnOnce(&mut Option<TabDrag>) -> R) -> Option<R> {
    let mut app = unsafe { app_ptr(hwnd) }?;
    Some(f(&mut unsafe { app.as_mut() }.tab_drag))
}

/// The dragged view as it is now: its index and its group's tab count. `None` once it has
/// closed or left `group`.
pub(crate) fn source_of(hwnd: HWND, group: GroupId, id: DocumentId) -> Option<Source> {
    let app = unsafe { app_ptr(hwnd) }?;
    let tabs = unsafe { app.as_ref() }.tabs.group(group)?;
    let ids = tabs.document_ids();
    Some(Source {
        group,
        document: id,
        index: ids.iter().position(|document| *document == id)?,
        group_len: ids.len(),
    })
}

/// What screen point `point` is over: a group's strip insertion point, or a zone of its content.
/// `None` over the band, a find bar, a caption button or anything outside the groups.
pub(crate) fn target_at(hwnd: HWND, point: POINT) -> Option<Target> {
    let (group, window) = main_window::group_at(hwnd, point)?;
    let mut local = point;
    unsafe { ScreenToClient(window, &mut local) };
    let layout = main_window::strip_layout_of(hwnd, group)?;
    if local.y >= 0 && local.y < layout.height {
        return (local.x < layout.bounds().right).then(|| Target::Strip {
            group,
            index: layout.insertion_index(local.x),
        });
    }
    let content = main_window::with_group_id(hwnd, group, |state| state.content)?;
    let content = Rect::new(content.left, content.top, content.right, content.bottom);
    content
        .contains(Point::new(local.x, local.y))
        .then(|| Target::Content {
            group,
            zone: group_drop::zone(content, Point::new(local.x, local.y)),
        })
}

/// The screen rectangle and alpha the overlay shows for a drop of `action` on `target`.
fn feedback(hwnd: HWND, target: Target, action: Action) -> Option<(RECT, u8)> {
    let (group, local) = match (target, action) {
        (Target::Strip { group, index }, Action::Reorder { .. } | Action::Place { .. }) => {
            let layout = main_window::strip_layout_of(hwnd, group)?;
            let window = main_window::with_group_id(hwnd, group, |state| state.hwnd)?;
            let dpi = unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForWindow(window) }.max(96);
            let x = layout.insertion_x(index);
            let half = scale(2, dpi) / 2;
            (
                group,
                RECT {
                    left: x - half,
                    top: 0,
                    right: x - half + scale(2, dpi).max(1),
                    bottom: layout.height,
                },
            )
        }
        (Target::Content { group, .. }, _) => {
            let content = main_window::with_group_id(hwnd, group, |state| state.content)?;
            let rect = match action {
                Action::Split { direction, .. } => half_towards(content, direction),
                _ => content,
            };
            (group, rect)
        }
        (Target::Strip { .. }, Action::Split { .. }) => return None,
    };
    let window = main_window::with_group_id(hwnd, group, |state| state.hwnd)?;
    let mut corners = [
        POINT {
            x: local.left,
            y: local.top,
        },
        POINT {
            x: local.right,
            y: local.bottom,
        },
    ];
    unsafe { MapWindowPoints(window, std::ptr::null_mut(), corners.as_mut_ptr(), 2) };
    let alpha = if matches!(target, Target::Strip { .. }) {
        BAR_ALPHA
    } else {
        TINT_ALPHA
    };
    Some((
        RECT {
            left: corners[0].x,
            top: corners[0].y,
            right: corners[1].x,
            bottom: corners[1].y,
        },
        alpha,
    ))
}

/// The half of `rect` towards `direction`: where the new group of an edge drop goes.
fn half_towards(rect: RECT, direction: Direction) -> RECT {
    let (width, height) = (rect.right - rect.left, rect.bottom - rect.top);
    match direction {
        Direction::Left => RECT {
            right: rect.left + width / 2,
            ..rect
        },
        Direction::Right => RECT {
            left: rect.right - width / 2,
            ..rect
        },
        Direction::Up => RECT {
            bottom: rect.top + height / 2,
            ..rect
        },
        Direction::Down => RECT {
            top: rect.bottom - height / 2,
            ..rect
        },
    }
}

/// Shows the overlay for `action` on `target`, or hides it when the drop would do nothing; sets
/// the arrow or no-drop cursor to match. Call it with nothing of the App borrowed.
pub(crate) fn show_feedback(hwnd: HWND, target: Option<Target>, action: Option<Action>) {
    let wanted = target.zip(action).and_then(|(target, action)| feedback(hwnd, target, action));
    let Some((rect, alpha)) = wanted else {
        hide_feedback(hwnd);
        set_cursor(false);
        return;
    };
    set_cursor(true);
    let palette = main_window::current_palette(hwnd);
    let color = if alpha == BAR_ALPHA {
        palette.editor_foreground
    } else {
        palette.selection_background
    };
    let existing = unsafe { app_ptr(hwnd) }.and_then(|app| unsafe { app.as_ref() }.drop_overlay);
    match existing {
        Some(overlay) => overlay.place(rect, color, alpha),
        None => {
            if let Some(overlay) = DropOverlay::show(hwnd, rect, color, alpha)
                && let Some(mut app) = unsafe { app_ptr(hwnd) }
            {
                unsafe { app.as_mut() }.drop_overlay = Some(overlay);
            }
        }
    }
}

/// Destroys the overlay, if one shows. Call it with nothing of the App borrowed.
pub(crate) fn hide_feedback(hwnd: HWND) {
    let overlay = unsafe { app_ptr(hwnd) }
        .and_then(|mut app| unsafe { app.as_mut() }.drop_overlay.take());
    if let Some(overlay) = overlay {
        overlay.destroy();
    }
}

fn set_cursor(accepted: bool) {
    let cursor = if accepted { IDC_ARROW } else { IDC_NO };
    unsafe { SetCursor(LoadCursorW(std::ptr::null_mut(), cursor)) };
}

/// A left press on tab `index` of `group`'s strip arms a drag of its view.
pub(crate) fn arm(hwnd: HWND, group: GroupId, window: HWND, index: usize, x: i32, y: i32) {
    let document = unsafe { app_ptr(hwnd) }.and_then(|app| {
        unsafe { app.as_ref() }
            .tabs
            .group(group)?
            .document_ids()
            .get(index)
            .copied()
    });
    let Some(source) = document.and_then(|id| source_of(hwnd, group, id)) else {
        return;
    };
    with_drag(hwnd, |drag| {
        *drag = Some(TabDrag {
            source,
            window,
            origin: (x, y),
            started: false,
            action: None,
            pointer: POINT { x, y },
            label: None,
            eat_right_up: false,
        })
    });
}

/// `WM_MOUSEMOVE` on `window`: starts an armed drag past the drag distance, then follows the
/// pointer. True while a started drag has the move, so the strip's hover code leaves it alone.
pub(crate) fn mouse_move(hwnd: HWND, window: HWND, x: i32, y: i32, buttons: WPARAM) -> bool {
    let Some(drag) = with_drag(hwnd, |drag| *drag).flatten() else {
        return false;
    };
    if drag.window != window || drag.eat_right_up {
        return drag.started;
    }
    if buttons & MK_LBUTTON as usize == 0 {
        // The release went elsewhere: a menu, a dialog, another window.
        cancel(hwnd);
        return drag.started;
    }
    if !drag.started {
        let (cx, cy) = unsafe { (GetSystemMetrics(SM_CXDRAG), GetSystemMetrics(SM_CYDRAG)) };
        if !crate::window::tree_drag::past_threshold(drag.origin, (x, y), cx, cy) {
            return false;
        }
        start(hwnd, window, x, y);
    }
    let mut screen = POINT { x, y };
    unsafe { ClientToScreen(window, &mut screen) };
    if let Some(label) = with_drag(hwnd, |drag| drag.as_ref().and_then(|drag| drag.label)).flatten() {
        label.move_to(screen);
    }
    retarget(hwnd, screen, buttons & MK_CONTROL as usize != 0);
    true
}

/// The drag goes past the drag distance: the capture, the strip's press cleared, the label.
fn start(hwnd: HWND, window: HWND, x: i32, y: i32) {
    let Some(source) = with_drag(hwnd, |drag| {
        let drag = drag.as_mut()?;
        drag.started = true;
        Some(drag.source)
    })
    .flatten() else {
        return;
    };
    main_window::update_strip_pointer(hwnd, source.group, |pointer| {
        pointer.press(None).hover(None)
    });
    unsafe { SetCapture(window) };
    let named = unsafe { app_ptr(hwnd) }.and_then(|app| {
        let document = unsafe { app.as_ref() }.tabs.document(source.document)?;
        let item = crate::library::tree::TreeItem::Note(
            document
                .path
                .as_deref()
                .map_or(crate::window::file_icons::NoteKind::Text, crate::window::file_icons::note_kind),
        );
        Some((item, document.title()))
    });
    let Some((item, name)) = named else {
        return;
    };
    let Some(image) = crate::window::notebook_view::tab_label_image(hwnd, window, item, &name) else {
        return;
    };
    let mut screen = POINT { x, y };
    unsafe { ClientToScreen(window, &mut screen) };
    let dpi = unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForWindow(window) }.max(96);
    let label = DragLabel::show(hwnd, &image, &name, screen, dpi);
    let orphan = with_drag(hwnd, |drag| match drag.as_mut() {
        Some(drag) => {
            drag.label = label;
            None
        }
        None => label,
    })
    .flatten();
    if let Some(orphan) = orphan {
        orphan.destroy();
    }
}

/// Resolves the pointer at screen `point` and shows what a release there would do.
fn retarget(hwnd: HWND, point: POINT, copy: bool) {
    let Some(drag) = with_drag(hwnd, |drag| *drag).flatten() else {
        return;
    };
    let target = target_at(hwnd, point);
    let source = source_of(hwnd, drag.source.group, drag.source.document);
    let action = source
        .zip(target)
        .and_then(|(source, target)| group_drop::decide(source, target, copy));
    with_drag(hwnd, |drag| {
        if let Some(drag) = drag.as_mut() {
            drag.pointer = point;
            drag.action = action;
        }
    });
    show_feedback(hwnd, target, action);
}

/// `WM_LBUTTONUP` on `window`: a started drag drops where the button went up. An armed one was
/// a click and is dropped here, leaving the release to the strip. True when a drag was under way.
pub(crate) fn release(hwnd: HWND, window: HWND, x: i32, y: i32, buttons: WPARAM) -> bool {
    let Some(drag) = with_drag(hwnd, Option::take).flatten() else {
        return false;
    };
    if !drag.started || drag.window != window {
        return false;
    }
    let mut screen = POINT { x, y };
    unsafe { ClientToScreen(window, &mut screen) };
    let target = target_at(hwnd, screen);
    let action = source_of(hwnd, drag.source.group, drag.source.document)
        .zip(target)
        .and_then(|(source, target)| {
            group_drop::decide(source, target, buttons & MK_CONTROL as usize != 0)
                .map(|action| (source, action))
        });
    end_feedback(hwnd, drag);
    if let Some((source, action)) = action {
        apply(hwnd, source, action);
    }
    true
}

/// Takes the label, the overlay and the capture down after `drag` was taken out of the App.
fn end_feedback(hwnd: HWND, drag: TabDrag) {
    if let Some(label) = drag.label {
        label.destroy();
    }
    hide_feedback(hwnd);
    set_cursor(true);
    if unsafe { GetCapture() } == drag.window {
        unsafe { ReleaseCapture() };
    }
}

/// Ends a drag without dropping: Esc, a lost capture, its group closing. An armed drag just
/// goes. True when a drag was under way.
pub(crate) fn cancel(hwnd: HWND) -> bool {
    let Some(drag) = with_drag(hwnd, Option::take).flatten() else {
        return false;
    };
    if drag.started {
        end_feedback(hwnd, drag);
    }
    drag.started
}

/// A right press cancels a started drag but keeps the capture until its own release reaches the
/// strip, which `right_release` swallows (plan amendment 8). True when a drag was under way.
pub(crate) fn cancel_for_right_press(hwnd: HWND) -> bool {
    let Some(drag) = with_drag(hwnd, |slot| {
        let drag = slot.as_mut().filter(|drag| drag.started && !drag.eat_right_up)?;
        drag.eat_right_up = true;
        let label = drag.label.take();
        Some(label)
    })
    .flatten() else {
        return false;
    };
    if let Some(label) = drag {
        label.destroy();
    }
    hide_feedback(hwnd);
    set_cursor(true);
    true
}

/// `WM_RBUTTONUP` after `cancel_for_right_press`: the capture goes, and the release does
/// nothing else. True when it was that release.
pub(crate) fn right_release(hwnd: HWND) -> bool {
    let Some(drag) = with_drag(hwnd, |slot| slot.take_if(|drag| drag.eat_right_up)).flatten() else {
        return false;
    };
    if unsafe { GetCapture() } == drag.window {
        unsafe { ReleaseCapture() };
    }
    true
}

/// Keys during a started drag (plan amendment 7): Esc cancels, Ctrl re-targets (move and copy
/// swap), and nothing else reaches the editor until the drag ends. True when the key was taken.
pub(crate) fn keeps_key(hwnd: HWND, message: &MSG) -> bool {
    if !matches!(
        message.message,
        WM_KEYDOWN | WM_KEYUP | WM_CHAR | WM_SYSKEYDOWN | WM_SYSKEYUP | WM_SYSCHAR
    ) {
        return false;
    }
    let Some(drag) = with_drag(hwnd, |drag| *drag)
        .flatten()
        .filter(|drag| drag.started && !drag.eat_right_up)
    else {
        return false;
    };
    let key = message.wParam as u16;
    if message.message == WM_KEYDOWN && key == VK_ESCAPE {
        cancel(hwnd);
    } else if key == VK_CONTROL {
        let copy = unsafe { GetKeyState(i32::from(VK_CONTROL)) } < 0;
        retarget(hwnd, drag.pointer, copy);
    }
    true
}

/// Carries out `action` for `source`, if its view is still where the drag found it (a tab can
/// close mid-drag).
pub(crate) fn apply(hwnd: HWND, source: Source, action: Action) {
    let Some(current) = source_of(hwnd, source.group, source.document) else {
        return;
    };
    match action {
        Action::Reorder { group, to, .. } => {
            let reordered = unsafe { app_ptr(hwnd) }.is_some_and(|mut app| {
                unsafe { app.as_mut() }
                    .tabs
                    .reorder(group, current.index, to.min(current.group_len - 1))
            });
            if reordered {
                main_window::focus_view(hwnd, group, source.document);
                main_window::refresh_tabs(hwnd);
                main_window::focus_content(hwnd);
            }
        }
        Action::Place { group, index, copy } => {
            main_window::place_view(hwnd, source.group, source.document, group, index, copy);
        }
        Action::Split {
            group,
            direction,
            copy,
        } => {
            if let Some(new) = main_window::split_group(hwnd, group, direction) {
                main_window::place_view(hwnd, source.group, source.document, new, None, copy);
            }
        }
    }
}
```

Adjust imports to what the compiler asks for. `Zone` may be unused here; drop it if clippy says so. `main_window::app_ptr` must be reachable from `tab_drag`: if it isn't `pub(crate)` in a module path `tab_drag` can see, use `crate::window::main_window::app_ptr`, as `preview_buttons.rs` does. `main_window` is a private `mod` in `window/mod.rs`, so sibling modules reach it as `super::main_window::…`. Use that form throughout.

In `group_drop.rs`, remove the `#![allow(dead_code)]` Task 1 added.

- [ ] **Step 4: Run the tests** and the strip's neighbours.

Run: `cargo test --lib -- tab_drag a_tab_dragged_along a_wobble a_press_on_a_tabs_close esc_cancels_a_tab a_lost_capture_cancels a_right_press_cancels_a_tab the_insertion_bar strip tab_click move_tab --test-threads=1`
Expected: PASS.

- [ ] **Step 5: Clippy and commit.**

Run: `cargo clippy --all-targets --all-features -- -D warnings`
Expected: no warnings.

```bash
git add src/window/tab_drag.rs src/window/mod.rs src/app.rs src/window/editor_group.rs src/window/main_window.rs src/window/group_drop.rs src/window/drop_overlay.rs src/window/notebook_view.rs
git commit -m "feat(split-editors): drag a tab along its strip to reorder it"
```

---

## Task 5: Drops on other groups and edge splits

Afterwards a tab dropped on another group's strip or content moves there, or with Ctrl gets a second view there. A tab dropped on a content edge splits that group. The already-open rule, the last-tab rule and the no-room notice hold. `tab_drag::apply` already routes the actions (Task 4); this task proves them end to end and fixes what the tests turn up.

**Files:**
- Modify: `src/window/tab_drag.rs`, `src/window/main_window.rs` (only if the tests below fail for a reason not covered in Task 4)
- Test: `src/window/main_window.rs` tests module

**Interfaces:**
- Consumes: Task 4's `start_strip_drag`, `drop_strip_drag`, `strip_ids`, `lparam_in`, `tab_drag::*` and `main_window::place_view`; the existing `split_group` and `NO_ROOM_TO_SPLIT`; the test helpers `notices(hwnd)` and `execute_command`.
- Produces: nothing new.

- [ ] **Step 1: Write the failing tests** in `main_window.rs`'s tests module:

```rust
    /// Two groups side by side, the second showing the first's document; the second active.
    fn two_groups(hwnd: HWND) -> (GroupId, GroupId) {
        let first = app_mut(hwnd).tabs.active_group();
        execute_command(hwnd, CommandId::SplitRight);
        (first, app_mut(hwnd).tabs.active_group())
    }

    fn group_window(hwnd: HWND, id: GroupId) -> HWND {
        super::with_group_id(hwnd, id, |state| state.hwnd).unwrap()
    }

    fn content(hwnd: HWND, id: GroupId) -> RECT {
        super::with_group_id(hwnd, id, |state| state.content).unwrap()
    }

    #[test]
    fn a_tab_dropped_on_another_groups_strip_moves_there_at_that_point() {
        // Break caught: a cross-group drop appending, keeping the source view, or leaving the
        // focus in the source group.
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        let (first, second) = two_groups(window.hwnd);
        execute_command(window.hwnd, CommandId::New);
        let moving = app_mut(window.hwnd).tabs.active().unwrap().id;
        assert!(super::activate_group(window.hwnd, first));
        execute_command(window.hwnd, CommandId::New);
        let dragged = app_mut(window.hwnd).tabs.active().unwrap().id;
        let index = strip_ids(window.hwnd, first).iter().position(|id| *id == dragged).unwrap();
        let source = start_strip_drag(window.hwnd, first, index);
        let target = group_window(window.hwnd, second);
        let layout = super::strip_layout_of(window.hwnd, second).unwrap();
        drop_strip_drag(source, target, layout.insertion_x(1) + 1, 10, 0);
        assert_eq!(strip_ids(window.hwnd, second)[1], dragged);
        assert!(strip_ids(window.hwnd, second).contains(&moving));
        assert!(!strip_ids(window.hwnd, first).contains(&dragged));
        assert_eq!(app_mut(window.hwnd).tabs.active_group(), second);
        assert_eq!(
            unsafe { windows_sys::Win32::UI::Input::KeyboardAndMouse::GetFocus() },
            super::group_editor(window.hwnd, second).unwrap().hwnd()
        );
    }

    #[test]
    fn a_ctrl_drop_on_another_group_adds_a_view_and_keeps_the_source() {
        // Break caught: Ctrl ignored, so a copy drag takes the tab away from where it was.
        use windows_sys::Win32::UI::WindowsAndMessaging::MK_CONTROL;
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        let (first, second) = two_groups(window.hwnd);
        assert!(super::activate_group(window.hwnd, first));
        execute_command(window.hwnd, CommandId::New);
        let dragged = app_mut(window.hwnd).tabs.active().unwrap().id;
        let index = strip_ids(window.hwnd, first).iter().position(|id| *id == dragged).unwrap();
        let source = start_strip_drag(window.hwnd, first, index);
        let target = group_window(window.hwnd, second);
        let middle = content(window.hwnd, second);
        drop_strip_drag(
            source,
            target,
            (middle.left + middle.right) / 2,
            (middle.top + middle.bottom) / 2,
            MK_CONTROL as usize,
        );
        assert!(strip_ids(window.hwnd, first).contains(&dragged));
        assert_eq!(strip_ids(window.hwnd, second).last(), Some(&dragged));
        assert_eq!(app_mut(window.hwnd).tabs.views_of(dragged).len(), 2);
    }

    #[test]
    fn a_drop_where_the_document_is_already_open_activates_that_view_and_still_moves() {
        // Break caught (spec §6.2): a second view of one document in one group, or the source
        // view left behind by a move.
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        let (first, second) = two_groups(window.hwnd);
        let shared = strip_ids(window.hwnd, second)[0];
        execute_command(window.hwnd, CommandId::New);
        assert!(super::activate_group(window.hwnd, first));
        execute_command(window.hwnd, CommandId::New);
        let source = start_strip_drag(window.hwnd, first, 0);
        let target = group_window(window.hwnd, second);
        let middle = content(window.hwnd, second);
        drop_strip_drag(source, target, (middle.left + middle.right) / 2, (middle.top + middle.bottom) / 2, 0);
        assert_eq!(strip_ids(window.hwnd, second).iter().filter(|id| **id == shared).count(), 1);
        assert_eq!(app_mut(window.hwnd).tabs.group(second).unwrap().active_document(), Some(shared));
        assert!(!strip_ids(window.hwnd, first).contains(&shared));
    }

    #[test]
    fn dragging_a_groups_last_tab_to_another_group_closes_the_source_group() {
        // Break caught (Review Focus 4): an empty group left behind after its last tab moved.
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        let (first, second) = two_groups(window.hwnd);
        execute_command(window.hwnd, CommandId::New);
        let dragged = app_mut(window.hwnd).tabs.active().unwrap().id;
        let index = strip_ids(window.hwnd, second).iter().position(|id| *id == dragged).unwrap();
        let other = strip_ids(window.hwnd, second)[1 - index];
        // Leave `dragged` alone in the second group.
        super::focus_view(window.hwnd, second, other);
        super::close_document_tab(window.hwnd, other);
        let source = start_strip_drag(window.hwnd, second, 0);
        let target = group_window(window.hwnd, first);
        let middle = content(window.hwnd, first);
        drop_strip_drag(source, target, (middle.left + middle.right) / 2, (middle.top + middle.bottom) / 2, 0);
        assert_eq!(super::group_order(window.hwnd), [first]);
        assert!(strip_ids(window.hwnd, first).contains(&dragged));
    }

    #[test]
    fn a_lone_tab_dropped_on_its_own_edge_or_middle_does_nothing() {
        // Break caught (Review Focus 4): a one-tab group splitting itself and closing, which
        // shuffles the layout for nothing.
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        let group = app_mut(window.hwnd).tabs.active_group();
        let ids = strip_ids(window.hwnd, group);
        let area = content(window.hwnd, group);
        let own = group_window(window.hwnd, group);
        let source = start_strip_drag(window.hwnd, group, 0);
        drop_strip_drag(source, own, area.right - 5, (area.top + area.bottom) / 2, 0);
        assert_eq!(super::group_order(window.hwnd), [group]);
        let source = start_strip_drag(window.hwnd, group, 0);
        drop_strip_drag(source, own, (area.left + area.right) / 2, (area.top + area.bottom) / 2, 0);
        assert_eq!(strip_ids(window.hwnd, group), ids);
    }

    #[test]
    fn a_tab_dropped_on_an_edge_splits_that_way_and_moves_into_the_new_group() {
        // Break caught: the split going the wrong way, or the view copied instead of moved.
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        execute_command(window.hwnd, CommandId::New);
        let group = app_mut(window.hwnd).tabs.active_group();
        let dragged = strip_ids(window.hwnd, group)[0];
        let area = content(window.hwnd, group);
        let own = group_window(window.hwnd, group);
        let source = start_strip_drag(window.hwnd, group, 0);
        drop_strip_drag(source, own, (area.left + area.right) / 2, area.bottom - 5, 0);
        let order = super::group_order(window.hwnd);
        assert_eq!(order.len(), 2);
        let new = order[1];
        assert_eq!(strip_ids(window.hwnd, new), [dragged]);
        assert!(!strip_ids(window.hwnd, group).contains(&dragged));
        let layout = super::tree_layout(window.hwnd).unwrap();
        assert!(layout.rect_of(new).unwrap().top > layout.rect_of(group).unwrap().top, "below");
    }

    #[test]
    fn an_edge_drop_without_room_says_so_and_leaves_the_tab_where_it_was() {
        // Break caught (Review Focus 5): the view removed from its group before the split was
        // refused, losing the tab.
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        execute_command(window.hwnd, CommandId::New);
        let group = app_mut(window.hwnd).tabs.active_group();
        let ids = strip_ids(window.hwnd, group);
        let area = content(window.hwnd, group);
        let own = group_window(window.hwnd, group);
        let source = start_strip_drag(window.hwnd, group, 0);
        // Shrink the window below two minimum-width groups mid-drag.
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::SetWindowPos(
                window.hwnd,
                std::ptr::null_mut(),
                0,
                0,
                300,
                400,
                windows_sys::Win32::UI::WindowsAndMessaging::SWP_NOMOVE
                    | windows_sys::Win32::UI::WindowsAndMessaging::SWP_NOZORDER,
            );
        }
        let area = super::with_group_id(window.hwnd, group, |state| state.content).unwrap_or(area);
        drop_strip_drag(source, own, area.right - 3, (area.top + area.bottom) / 2, 0);
        assert_eq!(super::group_order(window.hwnd), [group]);
        assert_eq!(strip_ids(window.hwnd, group), ids);
        assert!(notices(window.hwnd).iter().any(|notice| notice.contains(super::NO_ROOM_TO_SPLIT)));
    }

    #[test]
    fn a_tab_closed_mid_drag_drops_nothing() {
        // Break caught (Review Focus 2): a stale id moving some other tab, or a panic.
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        let (first, second) = two_groups(window.hwnd);
        assert!(super::activate_group(window.hwnd, first));
        execute_command(window.hwnd, CommandId::New);
        let dragged = app_mut(window.hwnd).tabs.active().unwrap().id;
        let index = strip_ids(window.hwnd, first).iter().position(|id| *id == dragged).unwrap();
        let source = start_strip_drag(window.hwnd, first, index);
        super::close_document_without_prompt(window.hwnd, dragged);
        let before = strip_ids(window.hwnd, second);
        let target = group_window(window.hwnd, second);
        let middle = content(window.hwnd, second);
        drop_strip_drag(source, target, (middle.left + middle.right) / 2, (middle.top + middle.bottom) / 2, 0);
        assert_eq!(strip_ids(window.hwnd, second), before);
        assert!(app_mut(window.hwnd).drop_overlay.is_none());
    }

    #[test]
    fn a_dirty_document_moves_between_groups_without_a_prompt_and_stays_dirty() {
        // Break caught: a move implemented as close plus open, asking to save.
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        let (first, second) = two_groups(window.hwnd);
        assert!(super::activate_group(window.hwnd, first));
        execute_command(window.hwnd, CommandId::New);
        editor.set_text("unsaved").unwrap();
        let dragged = app_mut(window.hwnd).tabs.active().unwrap().id;
        assert!(app_mut(window.hwnd).tabs.set_dirty(dragged, true));
        let index = strip_ids(window.hwnd, first).iter().position(|id| *id == dragged).unwrap();
        let source = start_strip_drag(window.hwnd, first, index);
        let target = group_window(window.hwnd, second);
        let middle = content(window.hwnd, second);
        drop_strip_drag(source, target, (middle.left + middle.right) / 2, (middle.top + middle.bottom) / 2, 0);
        assert!(crate::window::modal::take_last_confirm().is_none());
        assert!(app_mut(window.hwnd).tabs.document(dragged).unwrap().dirty);
        assert!(strip_ids(window.hwnd, second).contains(&dragged));
    }
```

Use the helper names the tests module already has for these, and adapt the calls to their real signatures:
- `close_document_tab(hwnd, id)`: `grep -n "fn close_document_tab" src/window/main_window.rs`;
- `notices(hwnd)`;
- `modal::take_last_confirm()`.

In the no-room test, give the shrink step a real layout pass if the window's `WM_SIZE` doesn't lay out synchronously in tests: call `super::layout_editor_and_find_bar(window.hwnd)` after `SetWindowPos`.

- [ ] **Step 2: Run them.**

Run: `cargo test --lib -- a_tab_dropped_on a_ctrl_drop a_drop_where_the_document dragging_a_groups_last_tab a_lone_tab_dropped an_edge_drop_without_room a_tab_closed_mid_drag a_dirty_document_moves --test-threads=1`
Expected: they should pass on Task 4's code. Any that fail show a gap. Fix it in `tab_drag.rs` or `place_view` with superpowers:systematic-debugging, and ledger the cause. If every one passes on the first run, say so in the ledger ("Task 5 tests green on Task 4's routing"): this task's tests pin behaviour that Task 4 built.

- [ ] **Step 3: Run the group tests** to make sure `move_active_view`'s refactor onto `place_view` broke nothing.

Run: `cargo test --lib -- group split move_tab --test-threads=1`
Expected: PASS.

- [ ] **Step 4: Clippy and commit.**

Run: `cargo clippy --all-targets --all-features -- -D warnings`
Expected: no warnings.

```bash
git add src/window/main_window.rs src/window/tab_drag.rs
git commit -m "test(split-editors): tab drops between groups, copies, edge splits and the last-tab rules"
```

---

## Task 6: A strip tab dropped on a notebook folder copies the file

Afterwards a tab dragged from a strip over the sidebar's notebook tree highlights a folder that takes its file, and a drop there copies it (spec §6.1). This is today's `copy_tab_into` behaviour, which Open Editors rows already have.

**Files:**
- Modify: `src/window/tree_drag.rs` (`DragSource::GroupTab`, `source_accepts`)
- Modify: `src/window/notebook_view.rs` (`is_external`, `drag_label_image`, `drag_release`, and new `strip_tab_over`, `strip_tab_leave` and `strip_tab_drop` next to `external_over` ~2868)
- Modify: `src/window/tab_drag.rs` (`retarget`, `release` and `end_feedback` consult the sidebar when no group is under the pointer)
- Test: `src/window/main_window.rs` tests module

**Interfaces:**
- Consumes: `notebook_view::{with_view, Drag, DragSource, is_external, end_drag_timer, DRAG_TIMER}`, `copy_host::copy_tab_into(hwnd, id, path, folder)`.
- Produces:
  - `DragSource::GroupTab { id: DocumentId, path: PathBuf }`: a strip tab dragged over the tree. It is an external-style drag: no capture and no label of the view's own.
  - `notebook_view::strip_tab_over(hwnd: HWND, point: POINT, id: DocumentId, path: &Path) -> bool`
  - `notebook_view::strip_tab_leave(hwnd: HWND)`
  - `notebook_view::strip_tab_drop(hwnd: HWND, point: POINT, id: DocumentId, path: &Path) -> bool`

- [ ] **Step 1: Write the failing test** in `main_window.rs`'s tests module, next to `open_editors_drag_onto_a_folder_copies_the_file_and_leaves_the_tab_on_it`:

```rust
    #[test]
    fn a_strip_tab_dropped_on_a_notebook_folder_copies_its_file() {
        // Break caught: strip drags ignoring the tree the Open Editors rows already copy into
        // (split editors spec §6.1).
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("strip-drag-folder");
        std::fs::create_dir_all(scratch.folder().join("work")).unwrap();
        scratch.note(r"work\b.md", "b");
        let outside = scratch.root.join("draft.txt");
        std::fs::write(&outside, "draft").unwrap();
        let (window, _editor) = notebook_window(&scratch);
        super::open_path(window.hwnd, &outside).unwrap();
        let group = app_mut(window.hwnd).tabs.active_group();
        let index = strip_ids(window.hwnd, group)
            .iter()
            .position(|id| app_mut(window.hwnd).tabs.document(*id).unwrap().path.as_deref() == Some(outside.as_path()))
            .unwrap();
        let source = start_strip_drag(window.hwnd, group, index);
        let panel = sidebar_windows(window.hwnd).1;
        let work = row_lparam(window.hwnd, &RowKind::Folder("work".into()));
        let (x, y) = ((work & 0xffff) as i16 as i32, ((work >> 16) & 0xffff) as i16 as i32);
        drop_strip_drag(source, panel, x, y, 0);
        crate::window::copy_host::wait_for_copies(window.hwnd);
        assert_eq!(
            std::fs::read_to_string(scratch.folder().join(r"work\draft.txt")).unwrap(),
            "draft"
        );
        assert!(strip_ids(window.hwnd, group).len() >= 1, "the tab stays");
        assert!(notebook_view(window.hwnd).drag.is_none());
    }
```

- [ ] **Step 2: Run it to confirm it fails.**

Run: `cargo test --lib -- a_strip_tab_dropped_on_a_notebook_folder --test-threads=1`
Expected: FAIL. The file isn't copied, because no sidebar target is resolved.

- [ ] **Step 3: Implement.**

`tree_drag.rs`: add the variant, and extend `source_accepts`:

```rust
    /// A tab dragged from an editor group's strip over the tree: a drop copies its file (split
    /// editors spec §6.1). The strip drag owns the capture and the label.
    GroupTab { id: DocumentId, path: PathBuf },
```

```rust
        DragSource::Tab { path, .. } | DragSource::GroupTab { path, .. } => {
            tree_copy::any_accepted(std::slice::from_ref(path), root, folder)
        }
```

(If Task 7 has already made `Tab`'s path an `Option`, keep `Tab` in its own arm as Task 7 shows.)

`notebook_view.rs`:
- `is_external`: `matches!(source, DragSource::Files(_) | DragSource::GroupTab { .. })`.
- `drag_label_image`: add `DragSource::GroupTab { .. } => return None,`.
- `drag_release`'s match: add `DragSource::GroupTab { .. } => {}`.
- Add, after `external_drop`:

```rust
/// Screen point `point` in the panel's client coordinates, when it is over the panel.
fn panel_point_of(hwnd: HWND, point: POINT) -> Option<(i32, i32)> {
    let panel = with_view(hwnd, |view| view.panel)?;
    if unsafe { IsWindowVisible(panel) } == 0 {
        return None;
    }
    let mut local = point;
    unsafe { ScreenToClient(panel, &mut local) };
    let mut client = RECT::default();
    unsafe { GetClientRect(panel, &mut client) };
    contains(client, local.x, local.y).then_some((local.x, local.y))
}

/// A strip tab dragged to screen point `point` (split editors spec §6.1): over a folder that
/// takes `path`'s file the band shows and true is returned; over Open Editors, or anywhere a
/// copy would do nothing, nothing shows. Kept as a started `DragSource::GroupTab` drag with no
/// capture and no label, as an Explorer drag is.
pub(crate) fn strip_tab_over(hwnd: HWND, point: POINT, id: DocumentId, path: &Path) -> bool {
    let Some((x, y)) = panel_point_of(hwnd, point) else {
        strip_tab_leave(hwnd);
        return false;
    };
    if with_view(hwnd, |view| view.opens_at(x, y)).unwrap_or(true) {
        strip_tab_leave(hwnd);
        return false;
    }
    let started = with_view(hwnd, |view| {
        if !view
            .drag
            .as_ref()
            .is_some_and(|drag| matches!(drag.source, DragSource::GroupTab { .. }))
        {
            let source = DragSource::GroupTab {
                id,
                path: path.to_path_buf(),
            };
            view.drag = Drag::armed(source, x, y).map(|mut drag| {
                drag.started = true;
                drag
            });
            return true;
        }
        false
    })
    .unwrap_or(false);
    if started && let Some(panel) = with_view(hwnd, |view| view.panel) {
        unsafe { SetTimer(panel, DRAG_TIMER, tree_drag::TICK.as_millis() as u32, None) };
    }
    with_view(hwnd, |view| view.drag_to(x, y, Instant::now())).unwrap_or(false)
}

/// The strip tab left the panel, or its drag ended: the band and the timer go.
pub(crate) fn strip_tab_leave(hwnd: HWND) {
    let panel = with_view(hwnd, |view| {
        let ours = view
            .drag
            .as_ref()
            .is_some_and(|drag| matches!(drag.source, DragSource::GroupTab { .. }));
        if ours {
            view.drag = None;
            view.invalidate();
        }
        ours.then_some(view.panel)
    })
    .flatten();
    if let Some(panel) = panel {
        end_drag_timer(panel);
    }
}

/// A strip tab dropped at screen point `point`: copies its file into the folder under it, when
/// that folder takes it. True when a copy started.
pub(crate) fn strip_tab_drop(hwnd: HWND, point: POINT, id: DocumentId, path: &Path) -> bool {
    let accepted = strip_tab_over(hwnd, point, id, path);
    let folder = accepted
        .then(|| with_view(hwnd, |view| view.drag.as_ref().and_then(|drag| drag.target.clone())))
        .flatten()
        .flatten();
    strip_tab_leave(hwnd);
    let Some(folder) = folder else {
        return false;
    };
    super::copy_host::copy_tab_into(hwnd, id, path, &folder);
    true
}
```

`tab_drag.rs`:
- In `retarget`, after `let target = target_at(hwnd, point);`, when `target.is_none()`, ask the sidebar. The drag's document path comes from `tabs.document(id)?.path.clone()`.

```rust
    let over_tree = target.is_none()
        && tab_path(hwnd, drag.source.document).is_some_and(|path| {
            super::notebook_view::strip_tab_over(hwnd, point, drag.source.document, &path)
        });
    if target.is_some() || !over_tree {
        super::notebook_view::strip_tab_leave(hwnd);
    }
```

  Then, when `over_tree` is true, call `hide_feedback(hwnd); set_cursor(true); return;` after storing the pointer and `action = None`. The tree draws its own band.
- In `release`, when `target` is `None`, first try `super::notebook_view::strip_tab_drop(hwnd, screen, drag.source.document, &path)`, with `path` from `tab_path`. In `end_feedback`, always call `super::notebook_view::strip_tab_leave(hwnd)`.
- Add:

```rust
/// Document `id`'s file, while it has one.
fn tab_path(hwnd: HWND, id: DocumentId) -> Option<std::path::PathBuf> {
    let app = unsafe { app_ptr(hwnd) }?;
    unsafe { app.as_ref() }.tabs.document(id)?.path.clone()
}
```

- [ ] **Step 4: Run the test and the tree drag's neighbours.**

Run: `cargo test --lib -- a_strip_tab_dropped_on_a_notebook_folder tree_drag open_editors_drag panel_drop --test-threads=1`
Expected: PASS.

- [ ] **Step 5: Clippy and commit.**

Run: `cargo clippy --all-targets --all-features -- -D warnings`
Expected: no warnings.

```bash
git add src/window/tree_drag.rs src/window/notebook_view.rs src/window/tab_drag.rs src/window/main_window.rs
git commit -m "feat(split-editors): a strip tab dropped on a notebook folder copies its file"
```

---

## Task 7: Open Editors rows drag onto groups

Afterwards an Open Editors row dragged out of the sidebar over a group's strip or content shows the same overlay. A release there moves the view (with Ctrl, copies it) by the same rules as a tab drag. Untitled rows drag too, and only onto groups: plan amendment 5.

**Files:**
- Modify: `src/window/tree_drag.rs` (`DragSource::Tab` becomes `Tab { id: DocumentId, group: GroupId, name: String, path: Option<PathBuf> }`; `source_accepts`)
- Modify: `src/window/notebook_view.rs`:
  - arming in the Open Editors press handler (~3070);
  - `drag_label_image` (~1321);
  - `drag_move` (~2700), `drag_release` (~2738) and `take_started_drag`/`cancel_drag` (~2767);
  - the `WM_LBUTTONUP` call site passes `wparam` to `drag_release`.
- Test: `src/window/main_window.rs` tests module (and update `open_editors_an_untitled_row_does_not_start_a_drag`)

**Interfaces:**
- Consumes: Task 4's `tab_drag::{target_at, source_of, show_feedback, hide_feedback, apply}` and `group_drop::decide`.
- Produces: `drag_release(hwnd: HWND, x: i32, y: i32, buttons: WPARAM) -> bool`, replacing today's three-argument form.

- [ ] **Step 1: Write the failing tests.** Replace `open_editors_an_untitled_row_does_not_start_a_drag` with:

```rust
    #[test]
    fn open_editors_an_untitled_row_drags_but_no_folder_takes_it() {
        // Break caught: an untitled row that cannot reach another group, or one the tree tries
        // to copy with no file behind it (plan amendment 5).
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("editors-drag-untitled");
        std::fs::create_dir_all(scratch.folder().join("work")).unwrap();
        let (window, _editor) = notebook_window(&scratch);
        let panel = sidebar_windows(window.hwnd).1;
        start_tab_drag(window.hwnd, panel, 0);
        assert!(notebook_view(window.hwnd).drag.as_ref().is_some_and(|drag| drag.started));
        let work = row_lparam(window.hwnd, &RowKind::Folder("work".into()));
        drag_over(panel, work);
        assert_eq!(notebook_view(window.hwnd).drag.as_ref().unwrap().target, None);
        drop_at(panel, work);
    }
```

Then add:

```rust
    #[test]
    fn open_editors_a_row_dropped_on_another_groups_content_moves_there() {
        // Break caught: row drags stopping at the sidebar's edge (split editors spec §6.2).
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("editors-drag-group");
        let outside = scratch.root.join("draft.txt");
        std::fs::write(&outside, "draft").unwrap();
        let (window, _editor) = notebook_window(&scratch);
        super::open_path(window.hwnd, &outside).unwrap();
        let first = app_mut(window.hwnd).tabs.active_group();
        let dragged = app_mut(window.hwnd).tabs.active().unwrap().id;
        execute_command(window.hwnd, CommandId::SplitRight);
        let second = app_mut(window.hwnd).tabs.active_group();
        execute_command(window.hwnd, CommandId::New);
        // Close the split's view of `dragged`, so a move is visible.
        super::focus_view(window.hwnd, second, dragged);
        execute_command(window.hwnd, CommandId::CloseTab);
        assert!(super::activate_group(window.hwnd, first));
        let panel = sidebar_windows(window.hwnd).1;
        let row = open_editors_row_of(window.hwnd, dragged);
        start_tab_drag(window.hwnd, panel, row);
        let target = super::with_group_id(window.hwnd, second, |state| state.hwnd).unwrap();
        let middle = super::with_group_id(window.hwnd, second, |state| state.content).unwrap();
        let point = lparam_in(
            panel,
            target,
            (middle.left + middle.right) / 2,
            (middle.top + middle.bottom) / 2,
        );
        drag_over(panel, point);
        assert!(app_mut(window.hwnd).drop_overlay.is_some());
        drop_at(panel, point);
        assert!(app_mut(window.hwnd).tabs.group(second).unwrap().contains(dragged));
        assert!(!app_mut(window.hwnd).tabs.group(first).is_some_and(|group| group.contains(dragged)));
        assert!(app_mut(window.hwnd).drop_overlay.is_none());
    }

    #[test]
    fn open_editors_a_row_drag_cancelled_over_a_group_leaves_no_overlay() {
        // Break caught: Esc mid-drag leaving the tint over the editor.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("editors-drag-cancel");
        let (window, _editor) = notebook_window(&scratch);
        execute_command(window.hwnd, CommandId::SplitRight);
        let second = app_mut(window.hwnd).tabs.active_group();
        let panel = sidebar_windows(window.hwnd).1;
        start_tab_drag(window.hwnd, panel, 0);
        let target = super::with_group_id(window.hwnd, second, |state| state.hwnd).unwrap();
        let middle = super::with_group_id(window.hwnd, second, |state| state.content).unwrap();
        drag_over(panel, lparam_in(panel, target, middle.left + 40, (middle.top + middle.bottom) / 2));
        assert!(crate::window::notebook_view::cancel_drag(window.hwnd));
        assert!(app_mut(window.hwnd).drop_overlay.is_none());
    }
```

`open_editors_row_of(hwnd, id) -> usize` is a new test helper. It returns the Open Editors row index (as `start_tab_drag` takes it) of document `id`'s first view, from `crate::window::open_editors::snapshot(hwnd)`: count only `EditorEntry::View` rows if `editor_rect_at` indexes views, or every entry if it indexes all rows. Check `editor_rect_at` and match it.

- [ ] **Step 2: Run them to confirm they fail.**

Run: `cargo test --lib -- open_editors_ --test-threads=1`
Expected: the untitled test fails because no drag arms, and the group test fails because nothing moves.

- [ ] **Step 3: Implement.**

`tree_drag.rs`:

```rust
    /// An Open Editors tab, from group `group`. Its name and file are taken when the drag
    /// armed, so the drag outlives its tab closing: a drop on a folder copies the file, one on a
    /// group moves or copies the view (split editors spec §6.2). Untitled tabs have no file.
    Tab {
        id: DocumentId,
        group: crate::window::split_tree::GroupId,
        name: String,
        path: Option<PathBuf>,
    },
```

```rust
        DragSource::Tab { path: Some(path), .. } | DragSource::GroupTab { path, .. } => {
            tree_copy::any_accepted(std::slice::from_ref(path), root, folder)
        }
        DragSource::Tab { path: None, .. } => false,
```

Fix `tree_drag.rs`'s own tests that build `DragSource::Tab { id, path }`: give them `group: GroupId(1), name: "a.md".into(), path: Some(path)`.

`notebook_view.rs`:
- The press handler: always arm, and drop the `if let Some(path)`:

```rust
                arm_drag(
                    hwnd,
                    DragSource::Tab {
                        id: row.id,
                        group: row.group,
                        name: row.name.clone(),
                        path: row.path.clone(),
                    },
                    x,
                    y,
                );
```

  Update its comment: "The name and path are taken now, so the drag outlives its tab closing (open editors spec §4.3). An untitled tab drags onto groups only."
- `drag_label_image`: `DragSource::Tab { path, name, .. } => (TreeItem::Note(path.as_deref().map_or(NoteKind::Text, note_kind)), name.clone()),`.
- `drag_release`: the `Tab` arm copies only `Some(path)`:

```rust
            DragSource::Tab {
                id, path: Some(path), ..
            } => {
                super::copy_host::copy_tab_into(hwnd, *id, path, &folder);
            }
            DragSource::Tab { path: None, .. } | DragSource::GroupTab { .. } => {}
```

- Add a helper that resolves a row drag against the groups:

```rust
/// A started Open Editors drag of `source` at panel point `x`, `y`, off the panel: the group
/// drop there, if any, and its feedback (split editors spec §6.2). `None` for any other drag or
/// point, which also hides the overlay.
fn group_drop_at(
    hwnd: HWND,
    panel: HWND,
    source: &DragSource,
    x: i32,
    y: i32,
    copy: bool,
) -> Option<(super::group_drop::Source, super::group_drop::Action)> {
    let DragSource::Tab { id, group, .. } = source else {
        super::tab_drag::hide_feedback(hwnd);
        return None;
    };
    let screen = screen_point(panel, x, y);
    let target = super::tab_drag::target_at(hwnd, screen);
    let found = super::tab_drag::source_of(hwnd, *group, *id).zip(target).and_then(|(from, target)| {
        super::group_drop::decide(from, target, copy).map(|action| (from, action))
    });
    super::tab_drag::show_feedback(hwnd, target, found.map(|(_, action)| action));
    found
}
```

- In `drag_move`, after `let accepted = with_view(…drag_to…)`: when the drag is started and not accepted, try the groups. The source comes from `with_view(hwnd, |view| view.drag.as_ref().map(|drag| drag.source.clone())).flatten()`. Then: `let over_group = !accepted && group_drop_at(hwnd, panel, &source, x, y, buttons & MK_CONTROL as usize != 0).is_some();` and `set_drag_cursor(accepted || over_group);`. When `accepted`, call `super::tab_drag::hide_feedback(hwnd)`. `show_feedback` sets the cursor itself, so make sure `set_drag_cursor` runs last.
- `drag_release(hwnd, x, y, buttons)`: when the drag was started and `drag.target` is `None`, compute `group_drop_at(hwnd, panel, &drag.source, x, y, buttons & MK_CONTROL as usize != 0)` before `end_drag_input`. After the label ends, call `super::tab_drag::hide_feedback(hwnd)`, then `super::tab_drag::apply(hwnd, from, action)` when it was `Some`.
- `cancel_drag` and `cancel_drag_for_right_press`: add `super::tab_drag::hide_feedback(hwnd);` after `end_drag_label(hwnd);`.
- Pass `wparam` from the `WM_LBUTTONUP` arm into `drag_release`.

- [ ] **Step 4: Run the tests.**

Run: `cargo test --lib -- open_editors_ tree_drag panel_drop notebook --test-threads=1`
Expected: PASS.

- [ ] **Step 5: Clippy and commit.**

Run: `cargo clippy --all-targets --all-features -- -D warnings`
Expected: no warnings.

```bash
git add src/window/tree_drag.rs src/window/notebook_view.rs src/window/main_window.rs
git commit -m "feat(split-editors): Open Editors rows drag onto groups, untitled ones included"
```

---

## Task 8: Explorer drops open in the group under the pointer

Afterwards a file dropped from Explorer opens in the group it was dropped on (spec §6.2): on that group's editor, strip, preview or image view. A drop on none opens in the active group. This was PR 2's deferred "Explorer drops only reach group 1".

**Files:**
- Modify: `src/window/library_host.rs`:
  - `accept_editor_file_drops` (~1080) wraps every group's editor and records that it did;
  - `wrap_editor_drop_target` gains the group window and posts it as `wparam`;
  - `editor_files_dropped` takes `wparam`.
- Modify: `src/app.rs` (new `App.file_drops_accepted: bool`, `false` in `App::new`)
- Modify: `src/window/main_window.rs`:
  - `create_group` (~1276) wraps a group made after the chrome;
  - `WM_DROPFILES` (~294) resolves the group from the drop point;
  - the `WM_FASTPAD_FILES_DROPPED` dispatch (~625) passes `wparam`.
- Modify: `src/platform/win32.rs` (test helper `test_hdrop_at`)
- Test: `src/window/main_window.rs` tests module

**Interfaces:**
- Consumes: Task 4's `main_window::group_at`; `editor::file_drop::test_support::drag_and_drop(hwnd, paths) -> [u32; 3]`; `platform::win32::dropped_paths`.
- Produces:
  - `library_host::wrap_group_drop_target(hwnd: HWND, group: HWND, editor: HWND)`
  - `library_host::editor_files_dropped(hwnd: HWND, wparam: WPARAM, lparam: LPARAM)`
  - `#[cfg(all(test, windows))] platform::win32::test_hdrop_at(paths: &[&Path], point: POINT) -> HGLOBAL`

- [ ] **Step 1: Write the failing tests** in `main_window.rs`'s tests module, next to `files_dropped_on_the_editor_reach_the_drop_handler`:

```rust
    #[test]
    fn files_dropped_on_a_second_groups_editor_open_in_that_group() {
        // Break caught: only group 1's editor taking Explorer drops, so a drop on the right-hand
        // editor opens on the left (split editors spec §6.2).
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("editor-drop-group");
        let note = scratch.note("dropped.md", "dropped");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        let first = app_mut(window.hwnd).tabs.active_group();
        execute_command(window.hwnd, CommandId::SplitRight);
        let second = app_mut(window.hwnd).tabs.active_group();
        assert!(super::activate_group(window.hwnd, first));
        crate::window::library_host::accept_editor_file_drops(window.hwnd);
        let target = super::group_editor(window.hwnd, second).unwrap();
        crate::editor::file_drop::test_support::drag_and_drop(target.hwnd(), &[&note]);
        pump_until(window.hwnd, || {
            app_mut(window.hwnd)
                .tabs
                .group_documents(second)
                .iter()
                .any(|document| document.path.as_deref() == Some(note.as_path()))
        });
        assert_eq!(app_mut(window.hwnd).tabs.active_group(), second);
    }

    #[test]
    fn a_group_split_off_after_the_chrome_takes_explorer_drops() {
        // Break caught: the wrapper installed once in BUILD_CHROME, so every later group's
        // editor refuses files.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("editor-drop-late");
        let note = scratch.note("late.md", "late");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        crate::window::library_host::accept_editor_file_drops(window.hwnd);
        execute_command(window.hwnd, CommandId::SplitRight);
        let second = app_mut(window.hwnd).tabs.active_group();
        let target = super::group_editor(window.hwnd, second).unwrap();
        let effects = crate::editor::file_drop::test_support::drag_and_drop(target.hwnd(), &[&note]);
        assert_eq!(effects, [windows_sys::Win32::System::Ole::DROPEFFECT_COPY; 3]);
        pump_until(window.hwnd, || {
            app_mut(window.hwnd)
                .tabs
                .group_documents(second)
                .iter()
                .any(|document| document.path.as_deref() == Some(note.as_path()))
        });
    }

    #[test]
    fn files_dropped_on_a_groups_strip_open_in_that_group() {
        // Break caught: WM_DROPFILES (a drop on a strip, a preview or an image) always opening
        // in the active group.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("strip-drop");
        let note = scratch.note("strip.md", "strip");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        let first = app_mut(window.hwnd).tabs.active_group();
        execute_command(window.hwnd, CommandId::SplitRight);
        let second = app_mut(window.hwnd).tabs.active_group();
        assert!(super::activate_group(window.hwnd, first));
        let strip = super::with_group_id(window.hwnd, second, |state| state.hwnd).unwrap();
        let mut point = windows_sys::Win32::Foundation::POINT { x: 20, y: 10 };
        unsafe { windows_sys::Win32::Graphics::Gdi::MapWindowPoints(strip, window.hwnd, &mut point, 1) };
        let drop = crate::platform::win32::test_hdrop_at(&[&note], point);
        unsafe { SendMessageW(window.hwnd, super::WM_DROPFILES, drop as usize, 0) };
        assert!(
            app_mut(window.hwnd)
                .tabs
                .group_documents(second)
                .iter()
                .any(|document| document.path.as_deref() == Some(note.as_path()))
        );
    }
```

- [ ] **Step 2: Run them to confirm they fail.**

Run: `cargo test --lib -- files_dropped_on a_group_split_off_after_the_chrome --test-threads=1`
Expected: FAIL. `test_hdrop_at` doesn't exist yet, and the first two tests open the file in group 1 or not at all.

- [ ] **Step 3: Implement.**

`src/platform/win32.rs`: rename the body of `test_hdrop` into `test_hdrop_at(paths, point)`, setting `pt: point` in the `DROPFILES`. Make `test_hdrop(paths)` call `test_hdrop_at(paths, POINT { x: 0, y: 0 })`.

`src/app.rs`: add

```rust
    /// `BUILD_CHROME` wrapped the group editors' drop targets: a group made from now on gets its
    /// wrapper when it is created.
    pub(crate) file_drops_accepted: bool,
```

`library_host.rs`:

```rust
/// Scintilla's own OLE drop target refuses files and wins over `WM_DROPFILES`, so every group's
/// editor gets a wrapper that posts dropped files here as `WM_FASTPAD_FILES_DROPPED`, with its
/// group window (split editors spec §6.2). The sidebar panel's own drop target is registered
/// here too. Runs in `BUILD_CHROME`; `create_group` wraps groups made later.
pub(crate) fn accept_editor_file_drops(hwnd: HWND) {
    let editors = unsafe { app_ptr(hwnd) }.map(|mut app| {
        let app = unsafe { app.as_mut() };
        app.file_drops_accepted = true;
        app.groups
            .iter()
            .map(|group| (group.hwnd, group.editor.hwnd()))
            .collect::<Vec<_>>()
    });
    for (group, editor) in editors.unwrap_or_default() {
        wrap_group_drop_target(hwnd, group, editor);
    }
    super::side_panel::accept_file_drops(hwnd);
}

/// Wraps `editor`'s drop target so its files open in group window `group`.
pub(crate) fn wrap_group_drop_target(hwnd: HWND, group: HWND, editor: HWND) {
    let (target, group) = (hwnd as isize, group as usize);
    // Text drag-and-drop still works without the wrapper; only file drops on the editor are lost.
    let _ = crate::editor::file_drop::accept_file_drops(editor, move |paths| {
        let payload = Box::into_raw(Box::new(paths));
        if unsafe {
            PostMessageW(
                target as HWND,
                crate::window::WM_FASTPAD_FILES_DROPPED,
                group,
                payload as isize,
            )
        } == 0
        {
            drop(unsafe { Box::from_raw(payload) });
        }
    });
}

/// `WM_FASTPAD_FILES_DROPPED`: frees the posted paths and opens them in the group of window
/// `wparam`, else the active one. A drop that lands while a modal dialog runs is ignored, as
/// `WM_DROPFILES` is for a disabled window.
pub(crate) fn editor_files_dropped(hwnd: HWND, wparam: WPARAM, lparam: LPARAM) {
    if lparam == 0 {
        return;
    }
    let paths = *unsafe { Box::from_raw(lparam as *mut Vec<PathBuf>) };
    if super::modal::modal_active(hwnd) {
        return;
    }
    if let Some(group) = super::main_window::group_id_of(hwnd, wparam as HWND) {
        super::main_window::activate_group(hwnd, group);
    }
    files_dropped(hwnd, paths);
}
```

Delete the old `wrap_editor_drop_target`.

`main_window.rs`:
- Dispatch: `crate::window::library_host::editor_files_dropped(hwnd, wparam, lparam);`.
- `WM_DROPFILES`:

```rust
        WM_DROPFILES => {
            let drop = wparam as windows_sys::Win32::UI::Shell::HDROP;
            let paths = crate::platform::win32::dropped_paths(drop);
            let mut point = windows_sys::Win32::Foundation::POINT { x: 0, y: 0 };
            let at = unsafe { windows_sys::Win32::UI::Shell::DragQueryPoint(drop, &mut point) };
            unsafe { windows_sys::Win32::UI::Shell::DragFinish(drop) };
            // The group under the drop opens it: a strip, a preview or an image view (plan
            // amendment 6).
            if at != 0 {
                unsafe { ClientToScreen(hwnd, &mut point) };
                if let Some((group, _)) = group_at(hwnd, point) {
                    activate_group(hwnd, group);
                }
            }
            crate::window::library_host::files_dropped(hwnd, paths);
            0
        }
```

  `DragQueryPoint` returns nonzero for a drop in the client area. The test builds its `DROPFILES` with `fNC = 0`, so it counts as a client-area drop.
- `create_group`: after `app.groups.push(…)`, read `let wrap = app.file_drops_accepted;` before the borrow ends. After it, with nothing borrowed: `if wrap { crate::window::library_host::wrap_group_drop_target(hwnd, window, editor_hwnd); }`. Capture `editor_hwnd = editor.hwnd()` before `editor` moves into `GroupWindow::new`.

- [ ] **Step 4: Run the tests.**

Run: `cargo test --lib -- files_dropped a_group_split_off_after_the_chrome panel_drop dropped_paths --test-threads=1`
Expected: PASS.

- [ ] **Step 5: Clippy and commit.**

Run: `cargo clippy --all-targets --all-features -- -D warnings`
Expected: no warnings.

```bash
git add src/window/library_host.rs src/app.rs src/window/main_window.rs src/platform/win32.rs
git commit -m "feat(split-editors): Explorer drops open in the group under the pointer"
```

---

## Task 9: Spec amendments and the full suite

**Files:**
- Modify: `docs/superpowers/specs/2026-09-28-split-editors-design.md` (§6, §6.1, §10 item 3)

- [ ] **Step 1: Amend the spec.** In §10, under item 3, add "PR 3 plan-time amendments (docs/superpowers/plans/2026-09-29-split-editors-3-dnd.md)" with this plan's 9 amendments, one line each. Then edit the sections they touch so they no longer contradict it:
  - §6.1's corner sentence: "nearer" means in pixels, and a tie goes to the top or bottom edge;
  - §6.1's table: add a row "Its own strip, with Ctrl | Reorders, as without Ctrl";
  - §6.2's Explorer line: "…in the group under the pointer: the editor's own group, or for a drop on a strip, a preview or an image view, the group under the drop point";
  - §3.2 (the preview flag): "a view arriving in a group that already has a preview tab becomes a normal tab".

- [ ] **Step 2: Run the full suite once.** Back up `%LOCALAPPDATA%\FastPad` first, and make sure the display is awake.

Run: `cargo test --all-targets --no-fail-fast -- --test-threads=1`
Expected: every test passes. If the run is killed for lack of memory, don't re-run it on your own. Say so, and ask the user to run it.

- [ ] **Step 3: Commit.**

```bash
git add docs/superpowers/specs/2026-09-28-split-editors-design.md
git commit -m "docs(split-editors): PR 3 plan-time amendments"
```

- [ ] **Step 4: Manual checks before the PR** (listed in the PR body, done by the user):
  - [ ] Drag a tab along its strip, across to another group's strip, onto another group's middle, and onto each of the four edges. The overlay matches where the tab lands each time.
  - [ ] The same with Ctrl held, and pressing and releasing Ctrl mid-drag: the source tab stays for a copy.
  - [ ] Esc and a right-click mid-drag cancel it, with no menu.
  - [ ] Drag an Open Editors row, a saved file and an untitled one, onto each group.
  - [ ] Drag a strip tab onto a notebook folder: the file is copied.
  - [ ] Drop a file from Explorer on each group's editor, strip and Markdown preview.
  - [ ] High contrast: the overlay and the insertion bar are visible.
  - [ ] 150–200% DPI: the label, the insertion bar and the edge zones line up.
  - [ ] Drag while the menu band is open (Alt): the band closes, or the drag still works.
