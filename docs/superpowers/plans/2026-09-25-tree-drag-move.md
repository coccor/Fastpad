# Tree drag-to-move Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Drag a note or folder in the Notebook tree onto another folder, or the notebook root, and it moves there on disk. Its tabs, pins and expanded folders follow it.

**Architecture:**
- A pure module, `window::tree_drag`, decides everything about a drag without a window: the drag's state, what is under the pointer, which folder a drop goes into, refusals, auto-scroll speed, auto-expand timing and the highlight range.
- A move module, `window::tree_move`, holds the one no-replace rename and the tabs, state and save folders following it. The move is extracted from `inline_name`'s rename commits, which then call it, and it also serves the drop.
- `notebook_view` wires the gesture:
  - it arms a drag on a row press;
  - it captures the mouse past the system drag distance;
  - it runs a 50 ms timer during the drag, sets the cursor, cancels, drops, and paints the target band.

**Tech Stack:** Rust 2024, windows-sys 0.61 (raw Win32), the GDI owner-drawn sidebar.

**Spec:** `docs/superpowers/specs/2026-09-25-tree-drag-move-design.md`

## Global Constraints

- **Latency:** nothing new runs before first paint or first input. The drag state is `None` until a press arms it, and the drag timer exists only while a drag is under way.
- **One disk call per drop,** on the UI thread: `platform::files::rename_no_replace`. A drag's hit test, target checks, highlight and cursor use only memory. A vanished source may trigger `library_host::request_rescan`, which runs on the scan worker.
- **Never overwrite.** Every rename is `rename_no_replace` (`MoveFileExW` with flags 0).
- **App-borrow rule:** while a `&mut` from `with_view`, `with_state`, `with_host` or `app_ptr` is held, never call `SetFocus`, `SetCapture`, `ReleaseCapture`, `SetTimer`, `KillTimer`, `SetCursor`, `CreateWindowExW`, `UpdateWindow`, `SetWindowTextW` on a child, `GetWindowTextW`, `SendMessageW` to another window, `modal::*` or a document swap. `ReleaseCapture` sends `WM_CAPTURECHANGED` to the panel synchronously.
- **Tests never touch the real profile** (`%LOCALAPPDATA%\FastPad`) **or the real Documents folder.** Window tests use `LibraryScratch` under `%TEMP%`.
- **Test runs:**
  - Compile with `cargo clippy --all-targets -- -D warnings` and run only the named tests while working.
  - Window tests need `-- --test-threads=1`.
  - The full suite (`cargo test -- --test-threads=1`) runs only at the final review.
- **No backward compatibility.**
- **Commits:** no attribution lines, and each commit is followed by `cargo fmt`.
- **Never modify** `assets/fastpad-icon.svg`, `assets/fastpad.ico`, `src/preview/images.rs` or `src/preview/svg.rs`.
- **Never search the whole disk.** Crate sources are in `C:\Users\korn3\.cargo\registry\src\`. `MK_LBUTTON` is in `windows_sys::Win32::System::SystemServices`, typed `u32`.
- **Notice wording**, verbatim from spec §5:
  - `<name> already exists in <folder>. Nothing was moved.` For the root, `<folder>` is `library_host::notebook_name(root)`.
  - `<name> no longer exists.`
  - `Couldn't move <name>: <system message>`
  - `Couldn't move <name>: another tab already has that file open.`
- **Timings** (spec §3.3):
  - auto-expand after 700 ms;
  - the drag timer ticks every 50 ms;
  - the auto-scroll zone is one row high at each edge: 1 line per tick in its outer half, 2 in its inner half, 3 past the list's edge.
- **Highlight** (spec §3.2):
  - Outside high contrast, the band is filled with `palette.inactive_selection_background` before the rows paint.
  - In high contrast, the band gets an outline of `palette.selection_background`, `scale(1, dpi).max(1)` thick, drawn after the rows paint.
  - The dragged row's name is drawn in `palette.muted_foreground`.

## Review Focus

1. **A press on a folder toggles it (today's click) before the drag starts,** so the rows below shift. The drag must follow its source by `RowKind`, not by index, and the drop target is read after the toggle. Task 3's `tree_drag_a_folder_pressed_then_dragged_to_empty_space_moves_to_the_root` pins this.
2. **Released outside the panel.** The mouse is captured, so the release arrives with coordinates outside the client area: nothing moves. Task 3's `tree_drag_refused_targets_and_a_release_outside_move_nothing` pins this.
3. **A button released where the panel never sees it,** before the drag started (over another app, or eaten by a menu). The next move without `MK_LBUTTON` disarms: no capture, no drag. Task 3's `tree_drag_a_short_move_or_a_missed_release_stays_a_click` pins this.
4. **A name taken in another letter case,** such as `A.md` in `work` when dropping `a.md`, is a clash on NTFS. The in-memory check must catch it. Task 2's `tree_move_a_taken_name_is_refused_in_memory_and_on_disk` pins this.
5. **A target folder that vanishes mid-drag** (a rescan). The drop must not rename into a path that isn't there. The rebuild clears a target whose row is gone. Task 3's `tree_drag_a_rebuild_or_view_switch_mid_drag_cancels_or_retargets` pins this.

---

## File structure

- **Create `src/window/tree_drag.rs`:** the pure drag logic and its unit tests (Task 1).
- **Create `src/window/tree_move.rs`:** `MoveError`, `Moved`, `move_note`, `move_folder`, `drop_into` and the notice wording (Task 2).
- **Modify `src/window/mod.rs`:** register both modules.
- **Modify `src/window/library_host.rs`:** add `not_found` next to `already_exists` (Task 2).
- **Modify `src/window/inline_name.rs`:** `commit_rename_note` and `commit_rename_folder` call `tree_move` (Task 2).
- **Modify `src/window/notebook_view.rs`:** the drag field, the mouse, timer and capture handling, the rebuild check, the cursor (Task 3), and the painting (Task 4).
- **Modify `src/window/side_panel.rs`:**
  - route `WM_TIMER` to the view;
  - make Esc cancel a drag;
  - cancel a drag on a view switch or when the sidebar hides (Task 3).
- **Modify `src/window/main_window.rs`** (tests module only): the window tests (Tasks 2 and 3).
- **Modify the docs:** `docs/superpowers/specs/2026-09-24-notebook-folders-design.md` §8, `README.md`, and the tree drag spec with a new §10 (Task 4).

---

### Task 1: Drag logic (pure)

**Files:**
- Create: `src/window/tree_drag.rs`
- Modify: `src/window/mod.rs` (add `pub(crate) mod tree_drag;` after `pub(crate) mod tooltip;`)

**Interfaces:**
- Consumes: `crate::library::tree::{self, RowKind, TreeRow}`, `crate::library::model::same_path`, `crate::library::at_or_under(path, folder)` (component-wise, ignores case, works on relative paths).
- Produces (later tasks rely on these exact names):
  - `pub(crate) const EXPAND_DELAY: Duration` (700 ms) and `pub(crate) const TICK: Duration` (50 ms).
  - `pub(crate) enum Hover { Row(usize), Below, Header, Outside }`
  - `pub(crate) struct Drag { source: RowKind, origin: (i32, i32), started: bool, pointer: (i32, i32), target: Option<PathBuf>, resting: Option<(PathBuf, Instant)> }`. All fields are `pub(crate)`.
  - `Drag::armed(source: RowKind, x: i32, y: i32) -> Option<Drag>`
  - `Drag::hover(&mut self, rows: &[TreeRow], point: (i32, i32), hover: Hover, now: Instant) -> bool`, which returns true when the target changed.
  - `pub(crate) fn draggable(kind: &RowKind) -> bool`
  - `pub(crate) fn past_threshold(origin: (i32, i32), point: (i32, i32), cx: i32, cy: i32) -> bool`
  - `pub(crate) fn source_path(kind: &RowKind) -> Option<&Path>`
  - `pub(crate) fn drop_folder(rows: &[TreeRow], hover: Hover) -> Option<PathBuf>`
  - `pub(crate) fn accepts(source: &RowKind, folder: &Path) -> bool`
  - `pub(crate) fn destination(source: &RowKind, folder: &Path) -> Option<PathBuf>`
  - `pub(crate) enum Highlight { Root, Rows { start: usize, end: usize } }`
  - `pub(crate) fn highlight(rows: &[TreeRow], folder: &Path) -> Option<Highlight>`
  - `pub(crate) fn scroll_step(y: i32, top: i32, bottom: i32, row_height: i32) -> i32`
  - `pub(crate) fn expand_due(resting: Option<&(PathBuf, Instant)>, now: Instant) -> Option<PathBuf>`
  - Folder paths are relative to the notebook. An empty `PathBuf` is the root.

- [ ] **Step 1: Write the module with its tests.** The tests go first in the file's `mod tests`, and the functions are stubbed with `todo!()`.

```rust
//! Dragging a note or folder in the Notebook tree onto another folder (tree drag spec §3): the
//! drag's state, what the pointer is over, which folder a drop there goes into, whether that
//! folder takes the dragged item, how fast an edge scrolls and when a resting folder expands.
//! Pure: no window, no disk. Folder paths are relative to the notebook; empty is the root.

use crate::library::model::same_path;
use crate::library::tree::{self, RowKind, TreeRow};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// How long the pointer rests on a collapsed folder row before it expands (spec §3.3).
pub(crate) const EXPAND_DELAY: Duration = Duration::from_millis(700);
/// The drag timer's period: one auto-scroll step and one auto-expand check (spec §3.3).
pub(crate) const TICK: Duration = Duration::from_millis(50);

/// What a drag's pointer is over (spec §3.2).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Hover {
    /// A list row: a tree row, or the "truncated" row just past the last one.
    Row(usize),
    /// The list below its last row.
    Below,
    /// The view's header, above the list.
    Header,
    /// Outside the panel.
    Outside,
}

/// A drag armed by a press on a row, and under way once `started`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Drag {
    pub(crate) source: RowKind,
    /// Where the press landed, in panel coordinates.
    pub(crate) origin: (i32, i32),
    /// The pointer went past the system drag distance: the panel has the capture.
    pub(crate) started: bool,
    /// The last pointer position, for the timer's scroll and re-targeting.
    pub(crate) pointer: (i32, i32),
    /// The folder a release now moves the item into, when that folder takes it.
    pub(crate) target: Option<PathBuf>,
    /// The collapsed folder row the pointer rests on, and since when (spec §3.3).
    pub(crate) resting: Option<(PathBuf, Instant)>,
}

impl Drag {
    /// A drag of `source` armed by a press at `x`, `y`; `None` for a row that cannot be dragged.
    pub(crate) fn armed(source: RowKind, x: i32, y: i32) -> Option<Self> {
        draggable(&source).then_some(Self {
            source,
            origin: (x, y),
            started: false,
            pointer: (x, y),
            target: None,
            resting: None,
        })
    }

    /// The pointer moved to `point`, over `hover`: the target and the resting folder follow.
    /// True when the target changed, so the highlight repaints.
    pub(crate) fn hover(
        &mut self,
        rows: &[TreeRow],
        point: (i32, i32),
        hover: Hover,
        now: Instant,
    ) -> bool {
        self.pointer = point;
        let target = drop_folder(rows, hover).filter(|folder| accepts(&self.source, folder));
        self.resting = rest_on(self.resting.take(), rows, hover, target.as_deref(), now);
        let changed = target != self.target;
        self.target = target;
        changed
    }
}

/// Notes and folders can be dragged; unsaved rows and the draft row cannot (spec §3.1).
pub(crate) fn draggable(kind: &RowKind) -> bool {
    matches!(kind, RowKind::Note(_) | RowKind::Folder(_))
}

/// Whether the pointer moved more than the system drag distance (`SM_CXDRAG`, `SM_CYDRAG`: the
/// pixels on either side of the press) from `origin`.
pub(crate) fn past_threshold(origin: (i32, i32), point: (i32, i32), cx: i32, cy: i32) -> bool {
    (point.0 - origin.0).abs() > cx || (point.1 - origin.1).abs() > cy
}

/// A note's or folder's path, relative to the notebook.
pub(crate) fn source_path(kind: &RowKind) -> Option<&Path> {
    match kind {
        RowKind::Note(path) | RowKind::Folder(path) => Some(path),
        RowKind::Unsaved(_) | RowKind::Draft => None,
    }
}

fn parent_of(path: &Path) -> PathBuf {
    path.parent().map(Path::to_path_buf).unwrap_or_default()
}

/// The folder a drop over `hover` goes into (spec §3.2): a folder row's folder, a note row's
/// folder, the root for an unsaved row, the truncated row, the space below the rows and the
/// header; `None` outside the panel and on the draft row.
pub(crate) fn drop_folder(rows: &[TreeRow], hover: Hover) -> Option<PathBuf> {
    match hover {
        Hover::Row(index) => match rows.get(index).map(|row| &row.kind) {
            Some(RowKind::Folder(path)) => Some(path.clone()),
            Some(RowKind::Note(path)) => Some(parent_of(path)),
            Some(RowKind::Unsaved(_)) | None => Some(PathBuf::new()),
            Some(RowKind::Draft) => None,
        },
        Hover::Below | Hover::Header => Some(PathBuf::new()),
        Hover::Outside => None,
    }
}

/// Whether `folder` takes `source` (spec §3.2): not its own folder, where nothing would change,
/// and for a folder, not itself or a folder inside it. Ignores letter case, as NTFS does.
pub(crate) fn accepts(source: &RowKind, folder: &Path) -> bool {
    let Some(path) = source_path(source) else {
        return false;
    };
    if same_path(&parent_of(path), folder) {
        return false;
    }
    !(matches!(source, RowKind::Folder(_)) && crate::library::at_or_under(folder, path))
}

/// Where `source` lands when dropped into `folder`: the same name in that folder.
pub(crate) fn destination(source: &RowKind, folder: &Path) -> Option<PathBuf> {
    Some(folder.join(source_path(source)?.file_name()?))
}

/// The rows a drop target highlights (spec §3.2).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Highlight {
    /// The whole list.
    Root,
    /// A folder's row and the rows shown under it: `start..end`.
    Rows { start: usize, end: usize },
}

/// What dropping into `folder` highlights; `None` when the folder has no row now.
pub(crate) fn highlight(rows: &[TreeRow], folder: &Path) -> Option<Highlight> {
    if folder.as_os_str().is_empty() {
        return Some(Highlight::Root);
    }
    let start = tree::row_index(rows, &RowKind::Folder(folder.to_path_buf()))?;
    let depth = rows[start].depth;
    let end = rows[start + 1..]
        .iter()
        .position(|row| row.depth <= depth)
        .map_or(rows.len(), |offset| start + 1 + offset);
    Some(Highlight::Rows { start, end })
}

/// Lines to scroll per tick with the pointer at `y`, for a list from `top` to `bottom` (spec
/// §3.3): negative in the top zone, positive in the bottom zone, 0 between. Each zone is one row
/// high: 1 line in its outer half, 2 in its inner half, 3 past the list's edge.
pub(crate) fn scroll_step(y: i32, top: i32, bottom: i32, row_height: i32) -> i32 {
    let zone = row_height.max(1);
    let speed = |depth: i32| {
        if depth > zone {
            3
        } else if depth > zone / 2 {
            2
        } else {
            1
        }
    };
    if y < top + zone {
        -speed(top + zone - y)
    } else if y >= bottom - zone {
        speed(y - (bottom - zone) + 1)
    } else {
        0
    }
}

/// The collapsed folder the pointer rests on, keeping its first time while it stays the same
/// row. Only a folder that takes the drag rests: expanding a refused one would help nothing.
fn rest_on(
    resting: Option<(PathBuf, Instant)>,
    rows: &[TreeRow],
    hover: Hover,
    target: Option<&Path>,
    now: Instant,
) -> Option<(PathBuf, Instant)> {
    let Hover::Row(index) = hover else {
        return None;
    };
    let row = rows.get(index)?;
    let RowKind::Folder(path) = &row.kind else {
        return None;
    };
    if row.expanded || !target.is_some_and(|target| same_path(target, path)) {
        return None;
    }
    match resting {
        Some((kept, since)) if same_path(&kept, path) => Some((kept, since)),
        _ => Some((path.clone(), now)),
    }
}

/// The resting folder to expand at `now`, once it has rested `EXPAND_DELAY`.
pub(crate) fn expand_due(resting: Option<&(PathBuf, Instant)>, now: Instant) -> Option<PathBuf> {
    resting
        .filter(|(_, since)| now.saturating_duration_since(*since) >= EXPAND_DELAY)
        .map(|(path, _)| path.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(kind: RowKind, depth: u16, expanded: bool) -> TreeRow {
        TreeRow {
            name: String::new(),
            kind,
            depth,
            pinned: false,
            expanded,
        }
    }

    fn note(path: &str) -> RowKind {
        RowKind::Note(path.into())
    }

    fn folder(path: &str) -> RowKind {
        RowKind::Folder(path.into())
    }

    /// untitled, work (expanded) { inner (collapsed), b.md }, a.md
    fn rows() -> Vec<TreeRow> {
        vec![
            row(RowKind::Unsaved(7), 0, false),
            row(folder("work"), 0, true),
            row(folder(r"work\inner"), 1, false),
            row(note(r"work\b.md"), 1, false),
            row(note("a.md"), 0, false),
        ]
    }

    #[test]
    fn only_notes_and_folders_arm_a_drag() {
        assert!(Drag::armed(note("a.md"), 1, 2).is_some());
        assert!(Drag::armed(folder("work"), 1, 2).is_some());
        assert!(Drag::armed(RowKind::Unsaved(7), 1, 2).is_none());
        assert!(Drag::armed(RowKind::Draft, 1, 2).is_none());
        let drag = Drag::armed(note("a.md"), 1, 2).unwrap();
        assert!(!drag.started);
        assert_eq!((drag.origin, drag.pointer, drag.target), ((1, 2), (1, 2), None));
    }

    #[test]
    fn the_drag_starts_only_past_the_system_distance_on_either_axis() {
        assert!(!past_threshold((10, 10), (14, 6), 4, 4));
        assert!(past_threshold((10, 10), (15, 10), 4, 4));
        assert!(past_threshold((10, 10), (10, 5), 4, 4));
    }

    #[test]
    fn each_place_under_the_pointer_names_its_drop_folder() {
        let rows = rows();
        let at = |hover| drop_folder(&rows, hover);
        assert_eq!(at(Hover::Row(1)), Some("work".into()), "a folder row");
        assert_eq!(at(Hover::Row(3)), Some("work".into()), "a note row: its folder");
        assert_eq!(at(Hover::Row(4)), Some(PathBuf::new()), "a top-level note: the root");
        assert_eq!(at(Hover::Row(0)), Some(PathBuf::new()), "an unsaved row: the root");
        assert_eq!(at(Hover::Row(5)), Some(PathBuf::new()), "the truncated row");
        assert_eq!(at(Hover::Below), Some(PathBuf::new()));
        assert_eq!(at(Hover::Header), Some(PathBuf::new()));
        assert_eq!(at(Hover::Outside), None);
        let draft = vec![row(RowKind::Draft, 0, false)];
        assert_eq!(drop_folder(&draft, Hover::Row(0)), None);
    }

    #[test]
    fn a_folder_refuses_its_own_items_and_a_folder_refuses_itself_and_its_insides() {
        assert!(!accepts(&note(r"work\b.md"), Path::new("work")), "its own folder");
        assert!(!accepts(&note(r"work\b.md"), Path::new("WORK")), "in any letter case");
        assert!(!accepts(&note("a.md"), Path::new("")), "a top-level note on the root");
        assert!(accepts(&note("a.md"), Path::new("work")));
        assert!(accepts(&note(r"work\b.md"), Path::new("")));
        assert!(!accepts(&folder("work"), Path::new("work")), "itself");
        assert!(!accepts(&folder("work"), Path::new(r"work\inner")), "inside itself");
        assert!(!accepts(&folder("work"), Path::new(r"Work\Inner\deep")));
        assert!(!accepts(&folder(r"work\inner"), Path::new("work")), "its own parent");
        assert!(accepts(&folder(r"work\inner"), Path::new("")));
        assert!(accepts(&folder("work"), Path::new("workshop")), "a name prefix is not inside");
        assert!(!accepts(&RowKind::Unsaved(7), Path::new("work")));
    }

    #[test]
    fn a_drop_keeps_the_name_in_the_new_folder() {
        assert_eq!(destination(&note("a.md"), Path::new("work")), Some(r"work\a.md".into()));
        assert_eq!(destination(&folder(r"work\inner"), Path::new("")), Some("inner".into()));
        assert_eq!(destination(&RowKind::Draft, Path::new("")), None);
    }

    #[test]
    fn hovering_sets_the_target_only_where_the_folder_takes_the_item() {
        let rows = rows();
        let now = Instant::now();
        let mut drag = Drag::armed(note("a.md"), 0, 0).unwrap();
        assert!(drag.hover(&rows, (5, 5), Hover::Row(3), now));
        assert_eq!(drag.target, Some("work".into()));
        assert_eq!(drag.pointer, (5, 5));
        assert!(!drag.hover(&rows, (5, 6), Hover::Row(1), now), "same target, no repaint");
        assert!(drag.hover(&rows, (5, 7), Hover::Below, now), "the root is a.md's own folder");
        assert_eq!(drag.target, None);
        assert!(!drag.hover(&rows, (5, 8), Hover::Outside, now));
    }

    #[test]
    fn a_collapsed_target_folder_expands_after_resting_700_ms_on_it() {
        let rows = rows();
        let start = Instant::now();
        let mut drag = Drag::armed(note("a.md"), 0, 0).unwrap();
        drag.hover(&rows, (0, 0), Hover::Row(2), start);
        assert_eq!(drag.resting.as_ref().map(|(path, _)| path.clone()), Some(r"work\inner".into()));
        // Moving within the same row keeps the first time.
        drag.hover(&rows, (1, 0), Hover::Row(2), start + Duration::from_millis(300));
        let at = |ms| expand_due(drag.resting.as_ref(), start + Duration::from_millis(ms));
        assert_eq!(at(699), None);
        assert_eq!(at(700), Some(r"work\inner".into()));
        // An expanded folder, a note row or a refused folder does not rest.
        let mut drag = Drag::armed(note("a.md"), 0, 0).unwrap();
        drag.hover(&rows, (0, 0), Hover::Row(1), start);
        assert_eq!(drag.resting, None, "work is expanded");
        let mut drag = Drag::armed(folder("work"), 0, 0).unwrap();
        drag.hover(&rows, (0, 0), Hover::Row(2), start);
        assert_eq!(drag.resting, None, "work refuses to go inside itself");
    }

    #[test]
    fn a_folder_highlights_its_row_and_the_rows_under_it_and_the_root_the_whole_list() {
        let rows = rows();
        assert_eq!(highlight(&rows, Path::new("")), Some(Highlight::Root));
        assert_eq!(
            highlight(&rows, Path::new("work")),
            Some(Highlight::Rows { start: 1, end: 4 })
        );
        assert_eq!(
            highlight(&rows, Path::new(r"work\inner")),
            Some(Highlight::Rows { start: 2, end: 3 })
        );
        assert_eq!(highlight(&rows, Path::new("gone")), None);
        let last = vec![row(folder("z"), 0, true), row(note(r"z\n.md"), 1, false)];
        assert_eq!(highlight(&last, Path::new("z")), Some(Highlight::Rows { start: 0, end: 2 }));
    }

    #[test]
    fn the_edges_scroll_faster_the_closer_the_pointer_is() {
        // A list from 100 to 400 with 26 px rows: zones 100..126 and 374..400.
        let step = |y| scroll_step(y, 100, 400, 26);
        assert_eq!(step(250), 0);
        assert_eq!(step(126), 0);
        assert_eq!(step(120), -1, "outer half of the top zone");
        assert_eq!(step(105), -2, "inner half");
        assert_eq!(step(90), -3, "above the list");
        assert_eq!(step(373), 0);
        assert_eq!(step(380), 1);
        assert_eq!(step(395), 2);
        assert_eq!(step(420), 3, "below the list");
    }
}
```

Note: "outer half" means the half of the zone nearer the list's middle. It gets 1 line per tick. The half nearer the edge gets 2, matching `speed(depth)`, where `depth` counts from the zone's inner boundary. The spec says "faster the closer the pointer is to the edge", and these tests are the binding numbers.

- [ ] **Step 2: Run the tests before implementing.** Replace each function body with `todo!()`. Keep the types, `Drag::armed` and `Drag::hover` as written, because `hover` only calls the stubs.

Run: `cargo test --lib tree_drag`
Expected: the tests panic with "not yet implemented".

- [ ] **Step 3: Restore the bodies above.**

- [ ] **Step 4: Run the tests again.**

Run: `cargo test --lib tree_drag` and `cargo clippy --all-targets -- -D warnings`
Expected: 9 passed, and clippy is clean. `dead_code` may flag functions that only later tasks call. If so, add `#[cfg_attr(not(test), expect(dead_code, reason = "used by the tree drag in notebook_view (next tasks)"))]` on each flagged item. Task 3 removes these attributes.

- [ ] **Step 5: Commit.**

```bash
cargo fmt
git add src/window/tree_drag.rs src/window/mod.rs
git commit -m "feat(sidebar): the tree drag's targets, refusals, edge scroll and resting folders"
```

---

### Task 2: The move, shared by renames and drops

**Files:**
- Create: `src/window/tree_move.rs`
- Modify: `src/window/mod.rs` (add `pub(crate) mod tree_move;` after `tree_drag`)
- Modify: `src/window/library_host.rs` (add `not_found` after `already_exists`, at ~line 1401)
- Modify: `src/window/inline_name.rs` (`commit_rename_note` at ~1167 and `commit_rename_folder` at ~1258)
- Test: `src/window/main_window.rs` tests module. Add the new tests after `a_folder_rename_onto_a_sibling_is_refused_and_the_same_name_changes_nothing` (~18145).

**Interfaces:**
- Consumes:
  - from Task 1: `tree_drag::{accepts, destination, source_path}`;
  - from `library_host`: `folder`, `with_state`, `rebind_open_tab`, `rename_note_back`, `rename_folder_back`, `tabs_under`, `reroot_save_folders`, `schedule_write`, `save_local_soon`, `set_expanded`, `already_exists`, `request_rescan`, `notebook_name`, `rename_undo_failed_notice`;
  - `main_window::{app_ptr, push_notice, invalidate_title_strip}`;
  - `side_panel::{refresh, with_accessible_events}`;
  - `notebook_view::select_row`.
- Produces:
  - `pub(crate) enum MoveError { Taken, Missing(crate::FastPadError), Failed(crate::FastPadError), TabCantFollow }`
  - `pub(crate) struct Moved { pub(crate) rebound: bool, pub(crate) stuck: Vec<PathBuf> }`
  - `pub(crate) fn move_note(hwnd: HWND, root: &Path, old: &Path, new: &Path) -> Result<Moved, MoveError>`. `old` and `new` are relative to `root`.
  - `pub(crate) fn move_folder(hwnd: HWND, root: &Path, old: &Path, new: &Path) -> Result<Moved, MoveError>`
  - `pub(crate) fn drop_into(hwnd: HWND, source: &RowKind, folder: &Path)`
  - `pub(crate) fn taken_notice(name: &str, folder: &str) -> String`
  - `library_host::not_found(error: &crate::FastPadError) -> bool`

**Refactor rule:** the inline rename behaviour must not change. Every existing rename test must pass unchanged. The state calls keep their current argument forms:
- `state.rename_note` takes **absolute** paths;
- `state.rename_folder` takes paths **relative** to the notebook.

- [ ] **Step 1: Write the failing window tests.** Put them in `main_window.rs`'s tests module. They use the existing helpers: `LibraryScratch`, `ProductionWindow`, `make_app`, `install_test_editor`, `ensure_sidebar`, `notebook_window`, `open_note`, `super::open_path`, `execute_command`, `app_mut`, `notices`, `selected_kind` and `load_native_scintilla`.

```rust
    #[test]
    fn tree_move_a_note_moves_into_a_folder_with_its_dirty_tab_and_pin() {
        // Break caught: a drop that saves the dirty tab, leaves it on the old path, drops the pin,
        // or leaves the target folder collapsed and the row unselected (tree drag spec §4).
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("drag-move-note");
        std::fs::create_dir_all(scratch.folder().join("work")).unwrap();
        scratch.note(r"work\b.md", "b");
        let a = scratch.note("a.md", "a");
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        ensure_sidebar(window.hwnd);
        scratch.install(window.hwnd);
        execute_command(window.hwnd, CommandId::ToggleFolderAutosave);
        super::open_path(window.hwnd, &a).unwrap();
        editor.set_text("a, edited").unwrap();
        crate::window::library_host::toggle_pin(window.hwnd, &a);
        crate::window::notebook_view::rebuild(window.hwnd);

        crate::window::tree_move::drop_into(
            window.hwnd,
            &RowKind::Note("a.md".into()),
            std::path::Path::new("work"),
        );

        let moved = scratch.folder().join(r"work\a.md");
        assert!(moved.exists() && !a.exists());
        assert_eq!(std::fs::read_to_string(&moved).unwrap(), "a", "nothing was saved");
        let active = app_mut(window.hwnd).tabs.active().unwrap();
        assert_eq!(active.path.as_deref(), Some(moved.as_path()));
        assert!(active.dirty);
        crate::window::library_host::with_state(window.hwnd, |state| {
            assert!(state.is_pinned(&moved));
        });
        assert!(
            crate::window::library_host::expanded(window.hwnd)
                .contains(&std::path::PathBuf::from("work"))
        );
        assert_eq!(selected_kind(window.hwnd), Some(RowKind::Note(r"work\a.md".into())));
        assert!(notices(window.hwnd).is_empty(), "{:?}", notices(window.hwnd));
    }

    #[test]
    fn tree_move_a_folder_moves_to_the_root_with_its_tabs_and_expanded_folders() {
        // Break caught: tabs under the moved folder left on old paths, or its expanded state and
        // its own expanded subfolder lost (tree drag spec §4).
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("drag-move-folder");
        std::fs::create_dir_all(scratch.folder().join(r"work\inner\deep")).unwrap();
        let (window, _editor) = notebook_window(&scratch);
        let c = open_note(&window, &scratch, r"work\inner\c.md", "c");
        for folder in ["work", r"work\inner", r"work\inner\deep"] {
            crate::window::library_host::set_expanded(window.hwnd, std::path::Path::new(folder), true);
        }
        crate::window::notebook_view::rebuild(window.hwnd);

        crate::window::tree_move::drop_into(
            window.hwnd,
            &RowKind::Folder(r"work\inner".into()),
            std::path::Path::new(""),
        );

        let moved = scratch.folder().join(r"inner\c.md");
        assert!(moved.exists() && !c.exists());
        assert_eq!(
            app_mut(window.hwnd).tabs.active().unwrap().path.as_deref(),
            Some(moved.as_path())
        );
        let expanded = crate::window::library_host::expanded(window.hwnd);
        assert!(expanded.contains(&std::path::PathBuf::from("inner")), "{expanded:?}");
        assert!(expanded.contains(&std::path::PathBuf::from(r"inner\deep")), "{expanded:?}");
        assert!(!expanded.contains(&std::path::PathBuf::from(r"work\inner")), "{expanded:?}");
        assert_eq!(selected_kind(window.hwnd), Some(RowKind::Folder("inner".into())));
    }

    #[test]
    fn tree_move_a_taken_name_is_refused_in_memory_and_on_disk() {
        // Break caught: a drop onto a listed name in another letter case (a clash on NTFS), or
        // onto a file the tree has not seen yet, overwriting or half-moving (tree drag spec §5).
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("drag-move-taken");
        std::fs::create_dir_all(scratch.folder().join("work")).unwrap();
        scratch.note(r"work\A.md", "work A");
        let a = scratch.note("a.md", "root a");
        let z = scratch.note("z.md", "root z");
        let (window, _editor) = notebook_window(&scratch);
        // Made after the scan: only the disk knows it.
        std::fs::write(scratch.folder().join(r"work\z.md"), "unseen").unwrap();

        let drop = |name: &str, folder: &str| {
            crate::window::tree_move::drop_into(
                window.hwnd,
                &RowKind::Note(name.into()),
                std::path::Path::new(folder),
            )
        };
        drop("a.md", "work");
        drop("z.md", "work");
        // work\A.md onto the root, where a.md is: the notice names the notebook.
        drop(r"work\A.md", "");
        let root_name = crate::window::library_host::notebook_name(&scratch.folder());

        assert_eq!(std::fs::read_to_string(&a).unwrap(), "root a");
        assert_eq!(std::fs::read_to_string(&z).unwrap(), "root z");
        assert_eq!(
            std::fs::read_to_string(scratch.folder().join(r"work\z.md")).unwrap(),
            "unseen"
        );
        assert_eq!(
            std::fs::read_to_string(scratch.folder().join(r"work\A.md")).unwrap(),
            "work A"
        );
        assert_eq!(
            notices(window.hwnd),
            vec![
                "a.md already exists in work. Nothing was moved.".to_owned(),
                "z.md already exists in work. Nothing was moved.".to_owned(),
                format!("A.md already exists in {root_name}. Nothing was moved."),
            ]
        );
    }

    #[test]
    fn tree_move_a_vanished_or_locked_source_says_so_and_moves_nothing() {
        // Break caught: a missing file reported as a generic failure (or as a clash), or a
        // sharing violation swallowed (tree drag spec §5).
        use std::os::windows::fs::OpenOptionsExt;
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("drag-move-gone");
        std::fs::create_dir_all(scratch.folder().join("work")).unwrap();
        let gone = scratch.note("gone.md", "g");
        let locked = scratch.note("locked.md", "l");
        let (window, _editor) = notebook_window(&scratch);
        std::fs::remove_file(&gone).unwrap();
        let _lock = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&locked)
            .unwrap();

        for name in ["gone.md", "locked.md"] {
            crate::window::tree_move::drop_into(
                window.hwnd,
                &RowKind::Note(name.into()),
                std::path::Path::new("work"),
            );
        }

        assert!(!scratch.folder().join(r"work\gone.md").exists());
        assert!(locked.exists() && !scratch.folder().join(r"work\locked.md").exists());
        let notices = notices(window.hwnd);
        assert_eq!(notices[0], "gone.md no longer exists.");
        assert!(notices[1].starts_with("Couldn't move locked.md: "), "{notices:?}");
        assert_eq!(notices.len(), 2, "{notices:?}");
    }

    #[test]
    fn tree_move_a_tab_that_cannot_follow_undoes_the_move_or_names_the_stuck_tab() {
        // Break caught: a move that leaves a tab on a path that no longer exists without saying
        // so, or keeps the move when it could be undone (tree drag spec §5).
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("drag-move-tab");
        std::fs::create_dir_all(scratch.folder().join("work")).unwrap();
        let a = scratch.note("a.md", "a");
        let top = scratch.note("top.md", "t");
        let (window, _editor) = notebook_window(&scratch);
        super::open_path(window.hwnd, &a).unwrap();
        super::open_path(window.hwnd, &top).unwrap();
        // Another tab already names the path `a` would move to, so `a`'s tab cannot follow.
        let top_id = app_mut(window.hwnd).tabs.find_stored_path(&top).unwrap();
        app_mut(window.hwnd).tabs.document_mut(top_id).unwrap().path =
            Some(scratch.folder().join(r"work\a.md"));
        let drop = || {
            crate::window::tree_move::drop_into(
                window.hwnd,
                &RowKind::Note("a.md".into()),
                std::path::Path::new("work"),
            )
        };

        drop();
        assert!(a.exists(), "moved back");
        assert_eq!(
            notices(window.hwnd),
            vec!["Couldn't move a.md: another tab already has that file open.".to_owned()]
        );

        crate::window::library_host::fail_next_note_rename_back();
        drop();
        assert!(!a.exists() && scratch.folder().join(r"work\a.md").exists());
        let expected = crate::window::library_host::rename_undo_failed_notice(
            "a.md",
            r"work\a.md",
            std::slice::from_ref(&a),
        );
        assert_eq!(notices(window.hwnd).last(), Some(&expected));
    }
```

- [ ] **Step 2: Run the tests before implementing.**

Run: `cargo test --lib tree_move -- --test-threads=1`
Expected: a compile error, because `crate::window::tree_move` doesn't exist.

- [ ] **Step 3: Add `not_found` to `library_host.rs`.** Put it after `already_exists`, and extend the `windows_sys::Win32::Foundation` import with `ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND`.

```rust
/// Whether a failed `MoveFileExW` means the source is gone.
pub(crate) fn not_found(error: &crate::FastPadError) -> bool {
    matches!(
        error,
        crate::FastPadError::Win32(code) if *code == ERROR_FILE_NOT_FOUND || *code == ERROR_PATH_NOT_FOUND
    )
}
```

- [ ] **Step 4: Write `src/window/tree_move.rs`.**

```rust
//! Moving a note or folder within the notebook (tree drag spec §4): the one no-replace rename,
//! then the open tabs, the library's records, pins and expanded folders, and the tabs' save
//! folders follow it. A tab that cannot follow undoes the move. The inline renames (inline
//! naming spec §5.2) are the same moves with a new name in the same folder.

use super::main_window::{app_ptr, push_notice};
use super::{library_host, notebook_view, side_panel, tree_drag};
use crate::library::{self, tree, tree::RowKind};
use crate::window::library_host::with_state;
use std::path::{Path, PathBuf};
use windows_sys::Win32::Foundation::HWND;

/// Why a move did not happen. Nothing on disk changed.
#[derive(Debug)]
pub(crate) enum MoveError {
    /// Something already has the new name (a change of letter case alone is not a clash).
    Taken,
    /// The source is gone from disk.
    Missing(crate::FastPadError),
    /// Windows refused the move for another reason.
    Failed(crate::FastPadError),
    /// An open tab could not follow, and the move was undone.
    TabCantFollow,
}

/// A move that happened.
#[derive(Debug, Default)]
pub(crate) struct Moved {
    /// A note's open tab followed it.
    pub(crate) rebound: bool,
    /// The old paths of tabs that could not follow when the undo failed too: the move stands.
    pub(crate) stuck: Vec<PathBuf>,
}

fn disk_error(error: crate::FastPadError, case_only: bool) -> MoveError {
    if !case_only && library_host::already_exists(&error) {
        MoveError::Taken
    } else if library_host::not_found(&error) {
        MoveError::Missing(error)
    } else {
        MoveError::Failed(error)
    }
}

/// Moves the note `old` to `new`, both relative to the notebook `root`, never onto another
/// file. Its open tab follows; if it cannot, the note goes back, and if that fails too, the move
/// stands and `stuck` names the tab.
pub(crate) fn move_note(
    hwnd: HWND,
    root: &Path,
    old: &Path,
    new: &Path,
) -> Result<Moved, MoveError> {
    let (old_path, new_path) = (root.join(old), root.join(new));
    let case_only = library::model::same_path(&new_path, &old_path);
    crate::platform::files::rename_no_replace(&old_path, &new_path)
        .map_err(|error| disk_error(error, case_only))?;
    let mut stuck = Vec::new();
    let rebound = match library_host::rebind_open_tab(hwnd, &old_path, new_path.clone()) {
        Ok(rebound) => rebound,
        Err(()) => {
            // Undo, so the tab and the disk agree.
            if library_host::rename_note_back(&new_path, &old_path).is_ok() {
                return Err(MoveError::TabCantFollow);
            }
            // The disk is the truth: the move stands, and the tab left on the old path is named.
            stuck.push(old_path.clone());
            false
        }
    };
    with_state(hwnd, |state| state.rename_note(&old_path, &new_path));
    library_host::schedule_write(hwnd);
    Ok(Moved { rebound, stuck })
}

/// Moves the folder `old` to `new`, both relative to the notebook `root`, never onto another
/// name. The open tabs under it follow; one that cannot undoes the whole move, and if the undo
/// fails, the move stands and `stuck` names the tabs left on their old paths.
pub(crate) fn move_folder(
    hwnd: HWND,
    root: &Path,
    old: &Path,
    new: &Path,
) -> Result<Moved, MoveError> {
    // Defence in depth: an empty or escaping path would move the notebook root or a folder
    // outside it.
    if !tree::is_plain_relative_folder(old) || !tree::is_plain_relative_folder(new) {
        return Err(MoveError::Failed(crate::FastPadError::Invariant(
            "not a folder inside the notebook",
        )));
    }
    let case_only = library::model::same_path(new, old);
    let (old_path, new_path) = (root.join(old), root.join(new));
    crate::platform::files::rename_no_replace(&old_path, &new_path)
        .map_err(|error| disk_error(error, case_only))?;
    let tabs = library_host::tabs_under(hwnd, &old_path);
    let mut moved = Vec::with_capacity(tabs.len());
    let mut stuck = Vec::new();
    for (id, path, _) in tabs {
        let target = new_path.join(library::record_path(&old_path, &path));
        let rebound = unsafe { app_ptr(hwnd) }
            .is_some_and(|mut app| unsafe { app.as_mut() }.tabs.rebind_path(id, target).is_ok());
        if rebound {
            moved.push((id, path));
        } else {
            stuck.push(path);
        }
    }
    if !stuck.is_empty() && library_host::rename_folder_back(&new_path, &old_path).is_ok() {
        // Undone, so the tabs and the disk agree: the folder went back, then the tabs that moved.
        for (id, path) in moved.into_iter().rev() {
            if let Some(mut app) = unsafe { app_ptr(hwnd) } {
                let _ = unsafe { app.as_mut() }.tabs.rebind_path(id, path);
            }
        }
        return Err(MoveError::TabCantFollow);
    }
    library_host::reroot_save_folders(hwnd, &old_path, &new_path);
    with_state(hwnd, |state| state.rename_folder(old, new));
    library_host::save_local_soon(hwnd);
    library_host::schedule_write(hwnd);
    Ok(Moved {
        rebound: false,
        stuck,
    })
}

/// The notice for a drop onto a name `folder` already has (tree drag spec §5).
pub(crate) fn taken_notice(name: &str, folder: &str) -> String {
    format!("{name} already exists in {folder}. Nothing was moved.")
}

/// A drop's target folder as its notice names it: the notebook's name for the root.
fn folder_label(root: &Path, folder: &Path) -> String {
    match folder.file_name() {
        Some(name) => name.to_string_lossy().into_owned(),
        None => library_host::notebook_name(root),
    }
}

/// The source is gone: say so, and let a rescan catch the tree up.
fn gone(hwnd: HWND, name: &str) {
    push_notice(hwnd, format!("{name} no longer exists."));
    library_host::request_rescan(hwnd);
}

/// A drop in the tree (tree drag spec §3.4, §4, §5): moves `source` into `folder` (relative to
/// the notebook; empty for the root), keeping its name. A folder that refuses it does nothing; a
/// taken name, a vanished source, a refusal from Windows or a tab that cannot follow moves
/// nothing and says so. After a move the target folder and the folders above it expand, and
/// the moved row is selected. The focus stays where it is.
pub(crate) fn drop_into(hwnd: HWND, source: &RowKind, folder: &Path) {
    let Some(root) = library_host::folder(hwnd) else {
        return;
    };
    let (Some(old), Some(new)) = (
        tree_drag::source_path(source),
        tree_drag::destination(source, folder),
    ) else {
        return;
    };
    if !tree_drag::accepts(source, folder) {
        return;
    }
    let name = old
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let is_folder = matches!(source, RowKind::Folder(_));
    let (listed, taken) = with_state(hwnd, |state| {
        let listed = if is_folder {
            state.is_folder(old)
        } else {
            state.is_listed(old)
        };
        (listed, state.is_listed(&new))
    })
    .unwrap_or((false, false));
    if !listed {
        gone(hwnd, &name);
        return;
    }
    if taken {
        push_notice(hwnd, taken_notice(&name, &folder_label(&root, folder)));
        return;
    }
    let result = if is_folder {
        move_folder(hwnd, &root, old, &new)
    } else {
        move_note(hwnd, &root, old, &new)
    };
    let moved = match result {
        Ok(moved) => moved,
        Err(MoveError::Taken) => {
            push_notice(hwnd, taken_notice(&name, &folder_label(&root, folder)));
            return;
        }
        Err(MoveError::Missing(_)) => {
            gone(hwnd, &name);
            return;
        }
        Err(MoveError::Failed(error)) => {
            push_notice(hwnd, format!("Couldn't move {name}: {error}"));
            return;
        }
        Err(MoveError::TabCantFollow) => {
            push_notice(
                hwnd,
                format!("Couldn't move {name}: another tab already has that file open."),
            );
            return;
        }
    };
    for ancestor in tree::ancestors(&new) {
        library_host::set_expanded(hwnd, &ancestor, true);
    }
    super::main_window::invalidate_title_strip(hwnd);
    let row = if is_folder {
        RowKind::Folder(new.clone())
    } else {
        RowKind::Note(new.clone())
    };
    side_panel::with_accessible_events(hwnd, || {
        side_panel::refresh(hwnd);
        notebook_view::select_row(hwnd, &row);
    });
    if !moved.stuck.is_empty() {
        push_notice(
            hwnd,
            library_host::rename_undo_failed_notice(
                &name,
                &new.to_string_lossy(),
                &moved.stuck,
            ),
        );
    }
}
```

If `invalidate_title_strip` isn't reachable as `super::main_window::invalidate_title_strip` from `window::tree_move` (`main_window` is a private module of `window`, and siblings can reach its `pub(crate)` items), call it the way `inline_name` does.

- [ ] **Step 5: Make `inline_name`'s rename commits use the moves.** Replace the bodies of `commit_rename_note` and `commit_rename_folder`. The doc comments above them stay. Add `use crate::window::tree_move::{self, MoveError};`, and drop any imports that become unused.

```rust
fn commit_rename_note(hwnd: HWND, how: How, relative: &Path, text: &str) {
    let Some(root) = library_host::folder(hwnd) else {
        cancel(hwnd);
        return;
    };
    let current = relative
        .extension()
        .map(|extension| extension.to_string_lossy().into_owned());
    let Some(name) = title::renamed_note_name(text, current.as_deref()) else {
        cancelled(hwnd, how);
        return;
    };
    let new = relative.with_file_name(&name);
    if new == relative {
        cancelled(hwnd, how);
        return;
    }
    let moved = match tree_move::move_note(hwnd, &root, relative, &new) {
        Ok(moved) => moved,
        Err(error) => {
            let message = match error {
                MoveError::Taken => {
                    let (stem, extension) = title::split_rename(text, current.as_deref());
                    let parent = root
                        .join(relative)
                        .parent()
                        .map(Path::to_path_buf)
                        .unwrap_or_default();
                    library_host::name_taken_error(&parent, &stem, &extension.unwrap_or_default())
                }
                MoveError::Missing(error) | MoveError::Failed(error) => {
                    format!("FastPad could not rename the file: {error}")
                }
                MoveError::TabCantFollow => "Another tab already has that file open.".to_owned(),
            };
            fail(hwnd, how, message);
            return;
        }
    };
    end(hwnd);
    let row = library::record_path(&root, &root.join(&new));
    side_panel::with_accessible_events(hwnd, || {
        side_panel::refresh(hwnd);
        notebook_view::select_row(hwnd, &RowKind::Note(row.clone()));
    });
    if how == How::Enter {
        notebook_view::focus_tree(hwnd);
    }
    if !moved.stuck.is_empty() {
        let old_name = relative.file_name().unwrap_or_default().to_string_lossy();
        push_notice(
            hwnd,
            library_host::rename_undo_failed_notice(&old_name, &name, &moved.stuck),
        );
    }
    if moved.rebound {
        // The extension may have changed, and with it the language.
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(
                hwnd,
                crate::window::WM_FASTPAD_APPLY_LANGUAGE,
                0,
                0,
            );
        }
    }
}

fn commit_rename_folder(hwnd: HWND, how: How, old: &Path, text: &str) {
    let Some(root) = library_host::folder(hwnd) else {
        cancel(hwnd);
        return;
    };
    // Defence in depth: an empty or escaping path would rename the notebook root or a folder
    // outside it.
    if !tree::is_plain_relative_folder(old) {
        cancel(hwnd);
        return;
    }
    let Some(name) = title::folder_name(text) else {
        cancelled(hwnd, how);
        return;
    };
    let parent = old.parent().map(Path::to_path_buf).unwrap_or_default();
    let new = parent.join(&name);
    if new.as_os_str() == old.as_os_str() || !tree::is_plain_relative_folder(&new) {
        cancelled(hwnd, how);
        return;
    }
    let moved = match tree_move::move_folder(hwnd, &root, old, &new) {
        Ok(moved) => moved,
        Err(error) => {
            let message = match error {
                MoveError::Taken => library_host::folder_taken_error(&name),
                MoveError::Missing(error) | MoveError::Failed(error) => {
                    format!("FastPad could not rename the folder: {error}")
                }
                MoveError::TabCantFollow => "Another tab already has that file open.".to_owned(),
            };
            fail(hwnd, how, message);
            return;
        }
    };
    end(hwnd);
    super::main_window::invalidate_title_strip(hwnd);
    side_panel::with_accessible_events(hwnd, || {
        side_panel::refresh(hwnd);
        notebook_view::select_row(hwnd, &RowKind::Folder(new.clone()));
    });
    if how == How::Enter {
        notebook_view::focus_tree(hwnd);
    }
    if !moved.stuck.is_empty() {
        let old_name = old.file_name().unwrap_or_default().to_string_lossy();
        push_notice(
            hwnd,
            library_host::rename_undo_failed_notice(&old_name, &name, &moved.stuck),
        );
    }
}
```

Check against the old bodies. The note rename's case-only test compared the absolute paths: `move_note` compares `new_path` with `old_path`, which gives the same result. `new == relative` is the same check as the old `new == old` on the joined paths. `record_path(&root, &root.join(&new))` is the same row the old code selected.

- [ ] **Step 6: Run the new tests and the existing rename tests.**

Run: `cargo test --lib tree_move -- --test-threads=1`
Expected: 5 passed.

Run: `cargo test --lib rename -- --test-threads=1`
Expected: every existing rename test passes, including `a_note_rename_whose_tab_cannot_follow_and_cannot_be_undone_stands`, `a_folder_rename_an_open_tab_cannot_follow_is_undone` and `renaming_a_note_changing_only_letter_case_works`.

Run: `cargo clippy --all-targets -- -D warnings`
Expected: clean. Drop Task 1's `expect(dead_code)` on the items this task now uses.

- [ ] **Step 7: Commit.**

```bash
cargo fmt
git add src/window/tree_move.rs src/window/mod.rs src/window/library_host.rs src/window/inline_name.rs src/window/main_window.rs src/window/tree_drag.rs
git commit -m "feat(sidebar): moving a note or folder to another folder, shared with the inline renames"
```

---

### Task 3: The drag gesture in the tree

**Files:**
- Modify: `src/window/notebook_view.rs`:
  - the struct (~383) and `new` (~595);
  - `handle` (~1811);
  - `left_down` (~2015);
  - `rebuild` (~1471);
  - new free functions after `mouse_move`.
- Modify: `src/window/side_panel.rs`:
  - `panel_proc`'s Esc arm (~1001) and the route list (~1027);
  - the cancels at ~405 (sidebar hides) and ~499 (`show_view_now`).
- Test: `src/window/main_window.rs` tests module, after Task 2's tests.

**Interfaces:**
- Consumes: Task 1's `tree_drag::{Drag, Hover, TICK, past_threshold, scroll_step, expand_due}` and Task 2's `tree_move::drop_into`.
- Produces:
  - `NotebookView.drag: Option<tree_drag::Drag>`, `pub(crate)`, read by tests and by Task 4's paint;
  - `pub(crate) const DRAG_TIMER: usize = 0x4452;`
  - `pub(crate) fn cancel_drag(hwnd: HWND) -> bool`
  - `pub(crate) fn drag_tick(hwnd: HWND, now: Instant)`

**Behaviour** (spec §3; these rulings are binding):
- **The press still does today's click first** (spec §3.1): select, then `row_clicked`, which opens a note's preview or toggles a folder. The drag is armed after that, only for `RowPart::Body`, with the row's `RowKind` read **before** `row_clicked`, because the rows may shift.
- **No drag is armed** when `inline_name::is_open` is true after the press. A double-click's second press never arms one.
- **Armed but not started:** a `WM_MOUSEMOVE` without `MK_LBUTTON` disarms. Moves within the threshold fall through to today's hover code.
- **Starting the drag:**
  - it sets `started`;
  - it clears the list hover, `hover` and `hover_pin`;
  - then, with nothing borrowed, it calls `SetCapture(panel)` and `SetTimer(panel, DRAG_TIMER, 50, None)`.
- **Every move while started** updates the target through `view.drag_to` and sets the cursor: `IDC_ARROW` when accepted, `IDC_NO` otherwise.
- **`WM_LBUTTONUP` while started:**
  1. re-target at the release point;
  2. take the drag;
  3. with nothing borrowed, `KillTimer`, `ReleaseCapture` (only if `GetCapture() == panel`) and the arrow cursor;
  4. then `tree_move::drop_into` if there is a target.
  A release while armed but not started is a plain click: disarm, then run today's code.
- **`cancel_drag`** takes any drag, armed or started. For a started one it invalidates, then with nothing borrowed calls `KillTimer`, `ReleaseCapture` if the panel holds the capture, and the arrow cursor. It returns true only for a started drag.
- **`WM_CAPTURECHANGED`** calls `cancel_drag`. After our own `ReleaseCapture` the drag is already taken, so this does nothing.
- **`WM_RBUTTONDOWN` while started:** cancel, set `eat_right_up = true`, and return `Some(0)` without selecting. `WM_RBUTTONUP` with `eat_right_up` set clears it and returns `Some(0)`, so no menu opens. Otherwise it returns `None`, as today.
- **Esc:** in `side_panel::panel_proc`, before the arm that returns focus to the editor, add `WM_KEYDOWN if wparam as u16 == VK_ESCAPE && crate::window::notebook_view::cancel_drag(main) => 0,`. The focus stays in the tree.
- **`WM_TIMER` with `wparam == DRAG_TIMER`** calls `drag_tick(hwnd, Instant::now())`. `side_panel`'s route list gains `WM_TIMER`.
- **`rebuild`:** after `apply`, inside the same borrow:
  - if the drag's source has no row, the drag is lost;
  - else, if the drag's target is a non-empty folder with no `RowKind::Folder` row, set `target = None`.
  After the borrow, a lost drag calls `cancel_drag`.
- **`side_panel`:** call `crate::window::notebook_view::cancel_drag(hwnd);` right after each of the two `crate::window::inline_name::cancel(hwnd);` calls, when the sidebar hides and when `show_view_now` switches away from the Notebook view.
- **`left_down`** first clears any stale armed drag: `with_view(hwnd, |view| view.drag = None);`.

- [ ] **Step 1: Write the failing window tests.** Put them in the `main_window.rs` tests module, and add these helpers first in that module:

```rust
    fn mouse(panel: HWND, message: u32, buttons: usize, lparam: super::LPARAM) {
        unsafe { SendMessageW(panel, message, buttons, lparam) };
    }

    /// Presses on `from` and moves past the drag distance, still holding the button.
    fn start_drag(hwnd: HWND, panel: HWND, from: &RowKind) {
        use windows_sys::Win32::UI::WindowsAndMessaging::{WM_LBUTTONDOWN, WM_MOUSEMOVE};
        let lparam = row_lparam(hwnd, from);
        mouse(panel, WM_LBUTTONDOWN, 1, lparam);
        let (x, y) = ((lparam & 0xffff) as i32, (lparam >> 16) as i32);
        mouse(panel, WM_MOUSEMOVE, 1, client_lparam(x + 30, y));
    }

    fn drag_over(panel: HWND, lparam: super::LPARAM) {
        mouse(panel, windows_sys::Win32::UI::WindowsAndMessaging::WM_MOUSEMOVE, 1, lparam);
    }

    fn drop_at(panel: HWND, lparam: super::LPARAM) {
        mouse(panel, windows_sys::Win32::UI::WindowsAndMessaging::WM_LBUTTONUP, 0, lparam);
    }

    /// A point in the list below its last row.
    fn below_rows(hwnd: HWND, panel: HWND) -> super::LPARAM {
        let mut client = windows_sys::Win32::Foundation::RECT::default();
        unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect(panel, &mut client) };
        let last = notebook_view(hwnd).rows.len() - 1;
        let bottom = notebook_view(hwnd).row_rect_at(last).unwrap().bottom;
        assert!(bottom + 40 < client.bottom - 40, "the panel is tall enough to test with");
        client_lparam(client.right / 2, bottom + 40)
    }

    fn drag_cursor_is(cursor: windows_sys::core::PCWSTR) -> bool {
        use windows_sys::Win32::UI::WindowsAndMessaging::{GetCursor, LoadCursorW};
        unsafe { GetCursor() == LoadCursorW(std::ptr::null_mut(), cursor) }
    }
```

Then add the tests:

```rust
    #[test]
    fn tree_drag_a_short_move_or_a_missed_release_stays_a_click() {
        // Break caught: a click turned into a drag by a jitter, or a drag started after its
        // release went to another window (tree drag spec §3.1).
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetCapture;
        use windows_sys::Win32::UI::WindowsAndMessaging::{WM_LBUTTONDOWN, WM_MOUSEMOVE};
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("drag-click");
        std::fs::create_dir_all(scratch.folder().join("work")).unwrap();
        scratch.note(r"work\b.md", "b");
        let a = scratch.note("a.md", "a");
        let (window, _editor) = notebook_window(&scratch);
        let panel = sidebar_windows(window.hwnd).1;
        let lparam = row_lparam(window.hwnd, &RowKind::Note("a.md".into()));
        let (x, y) = ((lparam & 0xffff) as i32, (lparam >> 16) as i32);

        mouse(panel, WM_LBUTTONDOWN, 1, lparam);
        mouse(panel, WM_MOUSEMOVE, 1, client_lparam(x + 1, y + 1));
        assert!(unsafe { GetCapture() }.is_null());
        drop_at(panel, client_lparam(x + 1, y + 1));
        assert!(notebook_view(window.hwnd).drag.is_none());
        assert_eq!(
            app_mut(window.hwnd).tabs.active().unwrap().path.as_deref(),
            Some(a.as_path()),
            "the press still opened the note"
        );

        mouse(panel, WM_LBUTTONDOWN, 1, lparam);
        // The release went elsewhere: the next move comes without the button.
        mouse(panel, WM_MOUSEMOVE, 0, row_lparam(window.hwnd, &RowKind::Folder("work".into())));
        assert!(notebook_view(window.hwnd).drag.is_none());
        assert!(unsafe { GetCapture() }.is_null());
        assert!(a.exists());
    }

    #[test]
    fn tree_drag_a_note_dropped_on_a_folder_moves_into_it_and_is_selected() {
        // Break caught: the drag not capturing, the drop not moving, the timer or capture left
        // behind, or the moved row not selected (tree drag spec §3.1, §3.4).
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetCapture;
        use windows_sys::Win32::UI::WindowsAndMessaging::{IDC_ARROW, KillTimer};
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("drag-note");
        std::fs::create_dir_all(scratch.folder().join("work")).unwrap();
        scratch.note(r"work\b.md", "b");
        let a = scratch.note("a.md", "a");
        let (window, _editor) = notebook_window(&scratch);
        let panel = sidebar_windows(window.hwnd).1;

        start_drag(window.hwnd, panel, &RowKind::Note("a.md".into()));
        assert_eq!(unsafe { GetCapture() }, panel);
        assert!(notebook_view(window.hwnd).drag.as_ref().unwrap().started);
        let work = row_lparam(window.hwnd, &RowKind::Folder("work".into()));
        drag_over(panel, work);
        assert_eq!(
            notebook_view(window.hwnd).drag.as_ref().unwrap().target,
            Some("work".into())
        );
        assert!(drag_cursor_is(IDC_ARROW));
        drop_at(panel, work);

        let moved = scratch.folder().join(r"work\a.md");
        assert!(moved.exists() && !a.exists());
        assert!(unsafe { GetCapture() }.is_null());
        assert_eq!(unsafe { KillTimer(panel, crate::window::notebook_view::DRAG_TIMER) }, 0);
        assert!(notebook_view(window.hwnd).drag.is_none());
        assert_eq!(selected_kind(window.hwnd), Some(RowKind::Note(r"work\a.md".into())));
        assert_eq!(
            app_mut(window.hwnd).tabs.active().unwrap().path.as_deref(),
            Some(moved.as_path()),
            "the preview the press opened followed"
        );
    }

    #[test]
    fn tree_drag_a_folder_pressed_then_dragged_to_empty_space_moves_to_the_root() {
        // Break caught: the press's folder toggle shifting the rows so the drag follows the
        // wrong row, or empty space not meaning the root (tree drag spec §3.1, §3.2).
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("drag-folder");
        std::fs::create_dir_all(scratch.folder().join(r"work\inner")).unwrap();
        scratch.note(r"work\inner\c.md", "c");
        scratch.note("a.md", "a");
        let (window, _editor) = notebook_window(&scratch);
        crate::window::library_host::set_expanded(window.hwnd, std::path::Path::new("work"), true);
        crate::window::notebook_view::rebuild(window.hwnd);
        let panel = sidebar_windows(window.hwnd).1;

        // The press toggles `inner` open, adding c.md's row under it.
        start_drag(window.hwnd, panel, &RowKind::Folder(r"work\inner".into()));
        assert_eq!(
            notebook_view(window.hwnd).drag.as_ref().unwrap().source,
            RowKind::Folder(r"work\inner".into())
        );
        let below = below_rows(window.hwnd, panel);
        drag_over(panel, below);
        drop_at(panel, below);

        assert!(scratch.folder().join(r"inner\c.md").exists());
        assert!(!scratch.folder().join(r"work\inner").exists());
        assert_eq!(selected_kind(window.hwnd), Some(RowKind::Folder("inner".into())));
    }

    #[test]
    fn tree_drag_refused_targets_and_a_release_outside_move_nothing() {
        // Break caught: a folder dropped into its own subfolder, a note "moved" into its own
        // folder, the refusal cursor missing, or a release over the editor moving anyway
        // (tree drag spec §3.2).
        use windows_sys::Win32::UI::WindowsAndMessaging::IDC_NO;
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("drag-refused");
        std::fs::create_dir_all(scratch.folder().join(r"work\inner")).unwrap();
        scratch.note(r"work\inner\c.md", "c");
        scratch.note("a.md", "a");
        let (window, _editor) = notebook_window(&scratch);
        for folder in ["work", r"work\inner"] {
            crate::window::library_host::set_expanded(window.hwnd, std::path::Path::new(folder), true);
        }
        crate::window::notebook_view::rebuild(window.hwnd);
        let panel = sidebar_windows(window.hwnd).1;

        // `work` collapses on the press; it is expanded again so `inner` is there to hover.
        start_drag(window.hwnd, panel, &RowKind::Folder("work".into()));
        crate::window::library_host::set_expanded(window.hwnd, std::path::Path::new("work"), true);
        crate::window::notebook_view::rebuild(window.hwnd);
        let inner = row_lparam(window.hwnd, &RowKind::Folder(r"work\inner".into()));
        drag_over(panel, inner);
        assert_eq!(notebook_view(window.hwnd).drag.as_ref().unwrap().target, None);
        assert!(drag_cursor_is(IDC_NO));
        drop_at(panel, inner);
        assert!(scratch.folder().join(r"work\inner\c.md").exists());

        start_drag(window.hwnd, panel, &RowKind::Note("a.md".into()));
        let below = below_rows(window.hwnd, panel);
        drag_over(panel, below);
        assert!(drag_cursor_is(IDC_NO), "the root is a.md's own folder");
        drop_at(panel, below);

        start_drag(window.hwnd, panel, &RowKind::Note("a.md".into()));
        let work = row_lparam(window.hwnd, &RowKind::Folder("work".into()));
        drag_over(panel, work);
        drop_at(panel, client_lparam(-50, (work >> 16) as i32));
        assert!(scratch.folder().join("a.md").exists());
        assert!(!scratch.folder().join(r"work\a.md").exists());
        assert!(notices(window.hwnd).is_empty(), "{:?}", notices(window.hwnd));
    }

    #[test]
    fn tree_drag_esc_a_right_press_and_a_lost_capture_cancel() {
        // Break caught: Esc sending the focus to the editor mid-drag, a right press opening the
        // menu or leaving the drag on, or a task switch leaving a drag that drops later
        // (tree drag spec §3.3).
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
            GetCapture, GetFocus, ReleaseCapture, SetCapture, VK_ESCAPE,
        };
        use windows_sys::Win32::UI::WindowsAndMessaging::{WM_KEYDOWN, WM_RBUTTONDOWN};
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("drag-cancel");
        std::fs::create_dir_all(scratch.folder().join("work")).unwrap();
        scratch.note(r"work\b.md", "b");
        let a = scratch.note("a.md", "a");
        let (window, _editor) = notebook_window(&scratch);
        let panel = sidebar_windows(window.hwnd).1;
        let work = || row_lparam(window.hwnd, &RowKind::Folder("work".into()));

        start_drag(window.hwnd, panel, &RowKind::Note("a.md".into()));
        drag_over(panel, work());
        unsafe { SendMessageW(panel, WM_KEYDOWN, VK_ESCAPE as usize, 0) };
        assert!(notebook_view(window.hwnd).drag.is_none());
        assert!(unsafe { GetCapture() }.is_null());
        assert_eq!(unsafe { GetFocus() }, panel, "the focus stays in the tree");
        drop_at(panel, work());
        assert!(a.exists());

        start_drag(window.hwnd, panel, &RowKind::Note("a.md".into()));
        drag_over(panel, work());
        mouse(panel, WM_RBUTTONDOWN, 2, work());
        assert!(notebook_view(window.hwnd).drag.is_none());
        assert!(unsafe { GetCapture() }.is_null());
        drop_at(panel, work());
        assert!(a.exists());

        start_drag(window.hwnd, panel, &RowKind::Note("a.md".into()));
        drag_over(panel, work());
        unsafe { SetCapture(window.hwnd) };
        assert!(notebook_view(window.hwnd).drag.is_none());
        unsafe { ReleaseCapture() };
        drop_at(panel, work());
        assert!(a.exists());
        assert!(!scratch.folder().join(r"work\a.md").exists());
    }

    #[test]
    fn tree_drag_the_timer_expands_a_resting_folder_and_scrolls_near_the_bottom() {
        // Break caught: a hovered collapsed folder never opening, the list not scrolling at its
        // edge, or no timer while dragging (tree drag spec §3.3).
        use windows_sys::Win32::UI::WindowsAndMessaging::KillTimer;
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("drag-timer");
        std::fs::create_dir_all(scratch.folder().join("work")).unwrap();
        scratch.note(r"work\b.md", "b");
        for index in 0..80 {
            scratch.note(&format!("n{index:02}.md"), "n");
        }
        let (window, _editor) = notebook_window(&scratch);
        let panel = sidebar_windows(window.hwnd).1;
        crate::window::library_host::set_expanded(window.hwnd, std::path::Path::new("work"), false);
        crate::window::notebook_view::rebuild(window.hwnd);

        start_drag(window.hwnd, panel, &RowKind::Note("n00.md".into()));
        let start = std::time::Instant::now();
        drag_over(panel, row_lparam(window.hwnd, &RowKind::Folder("work".into())));
        crate::window::notebook_view::drag_tick(window.hwnd, start);
        assert!(
            !crate::window::library_host::expanded(window.hwnd)
                .contains(&std::path::PathBuf::from("work"))
        );
        crate::window::notebook_view::drag_tick(
            window.hwnd,
            start + std::time::Duration::from_millis(800),
        );
        assert!(
            crate::window::library_host::expanded(window.hwnd)
                .contains(&std::path::PathBuf::from("work"))
        );

        let mut client = windows_sys::Win32::Foundation::RECT::default();
        unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect(panel, &mut client) };
        drag_over(panel, client_lparam(client.right / 2, client.bottom - 2));
        let top = notebook_view(window.hwnd).list.top;
        crate::window::notebook_view::drag_tick(window.hwnd, start);
        assert!(notebook_view(window.hwnd).list.top > top, "scrolled down");
        assert_ne!(
            unsafe { KillTimer(panel, crate::window::notebook_view::DRAG_TIMER) },
            0,
            "the drag's timer runs"
        );
        crate::window::notebook_view::cancel_drag(window.hwnd);
    }

    #[test]
    fn tree_drag_a_rebuild_or_view_switch_mid_drag_cancels_or_retargets() {
        // Break caught: a drag of a row a rescan removed staying on, a drop into a folder that
        // vanished, or a drag surviving another view (tree drag spec §3.3).
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetCapture;
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("drag-rebuild");
        std::fs::create_dir_all(scratch.folder().join("work")).unwrap();
        std::fs::create_dir_all(scratch.folder().join("other")).unwrap();
        let a = scratch.note("a.md", "a");
        scratch.note("b.md", "b");
        let (window, _editor) = notebook_window(&scratch);
        let panel = sidebar_windows(window.hwnd).1;

        start_drag(window.hwnd, panel, &RowKind::Note("b.md".into()));
        drag_over(panel, row_lparam(window.hwnd, &RowKind::Folder("work".into())));
        crate::window::library_host::with_state(window.hwnd, |state| {
            state.remove_folder_for_test(std::path::Path::new("work"));
        });
        crate::window::notebook_view::rebuild(window.hwnd);
        let drag = notebook_view(window.hwnd).drag.clone().unwrap();
        assert_eq!(drag.target, None, "the target folder's row is gone");

        crate::window::library_host::with_state(window.hwnd, |state| state.remove_note(&a));
        crate::window::notebook_view::rebuild(window.hwnd);
        assert!(notebook_view(window.hwnd).drag.is_some(), "b.md is still there");
        crate::window::library_host::with_state(window.hwnd, |state| {
            state.remove_note(&scratch.folder().join("b.md"))
        });
        crate::window::notebook_view::rebuild(window.hwnd);
        assert!(notebook_view(window.hwnd).drag.is_none());
        assert!(unsafe { GetCapture() }.is_null());

        crate::window::library_host::with_state(window.hwnd, |state| {
            let _ = state.add_note(&a);
        });
        crate::window::notebook_view::rebuild(window.hwnd);
        start_drag(window.hwnd, panel, &RowKind::Note("a.md".into()));
        crate::window::side_panel::show_view(window.hwnd, crate::config::SidebarView::Search, false);
        assert!(unsafe { GetCapture() }.is_null());
        crate::window::side_panel::show_view(window.hwnd, crate::config::SidebarView::Notebook, false);
        assert!(notebook_view(window.hwnd).drag.is_none());
    }
```

Two test helpers need checking:
- **`remove_folder_for_test`:** if `LibraryState` has no way to drop a folder row in memory, look for an existing one first: grep `fn remove_folder\|fn forget_folder` in `src/library/mod.rs`. If there is none, add `#[cfg(test)] pub fn remove_folder_for_test(&mut self, relative: &Path) { self.tree.remove_folder(relative); }` to `LibraryState`, next to `is_folder`.
- **`add_note` and `remove_note`** are existing `pub` methods of `LibraryState` (`src/library/mod.rs:507`, `:614`), taking absolute paths.

Report what you used.

- [ ] **Step 2: Run the tests before implementing.**

Run: `cargo test --lib tree_drag_ -- --test-threads=1`
Expected: a compile error (`drag`, `DRAG_TIMER`, `drag_tick` and `cancel_drag` don't exist).

- [ ] **Step 3: Add the view state and the gesture code to `notebook_view.rs`.**

Imports to add:
- `use crate::window::tree_drag::{self, Drag, Hover};`
- `GetCapture` from `KeyboardAndMouse`;
- from `WindowsAndMessaging`: `GetSystemMetrics`, `IDC_ARROW`, `IDC_NO`, `KillTimer`, `LoadCursorW`, `SM_CXDRAG`, `SM_CYDRAG`, `SetCursor`, `SetTimer`, `WM_RBUTTONUP`, `WM_TIMER`;
- `use windows_sys::Win32::System::SystemServices::MK_LBUTTON;`.

Fields: add these to `NotebookView`, after `thumb_grab`, and initialise them in `new` as `drag: None, eat_right_up: false`:

```rust
    /// A drag of a row, armed by a press and under way past the drag distance (tree drag spec
    /// §3).
    pub(crate) drag: Option<Drag>,
    /// The right press that cancelled a drag: its release opens no menu.
    eat_right_up: bool,
```

Constant, placed near the other constants:

```rust
/// The panel's timer while a drag is under way (tree drag spec §3.3).
pub(crate) const DRAG_TIMER: usize = 0x4452;
```

Methods: add these to `impl NotebookView`, next to `hit_test`:

```rust
    /// What a drag at panel point `x`, `y` is over (tree drag spec §3.2). The scroll thumb
    /// counts as the row under it.
    fn drag_hover(&self, x: i32, y: i32) -> Hover {
        let area = self.client();
        if self.mode != Mode::Tree || !contains(area, x, y) {
            return Hover::Outside;
        }
        let list = self.list_rect(area);
        if y < list.top {
            return Hover::Header;
        }
        self.list
            .row_at(y - list.top)
            .map_or(Hover::Below, Hover::Row)
    }

    /// The drag moved to `x`, `y`: the target follows, and the highlight repaints when it
    /// changed. Whether a release there moves the item.
    fn drag_to(&mut self, x: i32, y: i32, now: Instant) -> bool {
        let hover = self.drag_hover(x, y);
        let Some(drag) = self.drag.as_mut() else {
            return false;
        };
        let changed = drag.hover(&self.rows, (x, y), hover, now);
        let accepted = drag.target.is_some();
        if changed {
            self.invalidate();
        }
        accepted
    }
```

Free functions, after `mouse_move`:

```rust
/// The cursor a drag shows: the arrow over a folder that takes the item, "no" elsewhere
/// (tree drag spec §3.2). Called with nothing of the App borrowed.
fn set_drag_cursor(accepted: bool) {
    let cursor = if accepted { IDC_ARROW } else { IDC_NO };
    unsafe { SetCursor(LoadCursorW(std::ptr::null_mut(), cursor)) };
}

/// Ends a drag's timer, capture and cursor, with nothing of the App borrowed: ReleaseCapture
/// sends WM_CAPTURECHANGED here.
fn end_drag_input(panel: HWND) {
    unsafe {
        KillTimer(panel, DRAG_TIMER);
        if GetCapture() == panel {
            ReleaseCapture();
        }
    }
    set_drag_cursor(true);
}

/// A press on a row's body arms a drag of `source` (tree drag spec §3.1), unless an inline
/// edit is still open.
fn arm_drag(hwnd: HWND, source: RowKind, x: i32, y: i32) {
    if super::inline_name::is_open(hwnd) {
        return;
    }
    with_view(hwnd, |view| view.drag = Drag::armed(source, x, y));
}

/// `WM_MOUSEMOVE` with a drag armed or under way (tree drag spec §3.1, §3.2). False leaves the
/// move to the hover code: no drag, or one that has not started.
fn drag_move(hwnd: HWND, x: i32, y: i32, buttons: WPARAM) -> bool {
    let Some((started, origin, panel)) = with_view(hwnd, |view| {
        view.drag
            .as_ref()
            .map(|drag| (drag.started, drag.origin, view.panel))
    })
    .flatten() else {
        return false;
    };
    if buttons & MK_LBUTTON as usize == 0 {
        // The release went elsewhere: a menu, a dialog, another window.
        if started {
            cancel_drag(hwnd);
        } else {
            with_view(hwnd, |view| view.drag = None);
        }
        return started;
    }
    if !started {
        let (cx, cy) = unsafe { (GetSystemMetrics(SM_CXDRAG), GetSystemMetrics(SM_CYDRAG)) };
        if !tree_drag::past_threshold(origin, (x, y), cx, cy) {
            return false;
        }
        with_view(hwnd, |view| {
            if let Some(drag) = view.drag.as_mut() {
                drag.started = true;
            }
            view.list.hover = None;
            view.hover = None;
            view.hover_pin = false;
            view.invalidate();
        });
        unsafe {
            SetCapture(panel);
            SetTimer(panel, DRAG_TIMER, tree_drag::TICK.as_millis() as u32, None);
        }
    }
    let accepted = with_view(hwnd, |view| view.drag_to(x, y, Instant::now())).unwrap_or(false);
    set_drag_cursor(accepted);
    true
}

/// `WM_LBUTTONUP`: a drag under way drops where the button went up (tree drag spec §3.4). An
/// armed drag was a click. True when a drag was under way.
fn drag_release(hwnd: HWND, x: i32, y: i32) -> bool {
    let Some((drag, panel)) = with_view(hwnd, |view| {
        if view.drag.as_ref().is_some_and(|drag| drag.started) {
            view.drag_to(x, y, Instant::now());
            view.invalidate();
        }
        (view.drag.take(), view.panel)
    }) else {
        return false;
    };
    let Some(drag) = drag.filter(|drag| drag.started) else {
        return false;
    };
    end_drag_input(panel);
    if let Some(folder) = drag.target {
        super::tree_move::drop_into(hwnd, &drag.source, &folder);
    }
    true
}

/// Ends a drag without moving anything (tree drag spec §3.3): Esc, a right press, a lost
/// capture, another view, the sidebar hiding, or the dragged row gone. An armed drag just goes.
/// True when a drag was under way.
pub(crate) fn cancel_drag(hwnd: HWND) -> bool {
    let Some((drag, panel)) = with_view(hwnd, |view| {
        let drag = view.drag.take();
        if drag.as_ref().is_some_and(|drag| drag.started) {
            view.invalidate();
        }
        (drag, view.panel)
    }) else {
        return false;
    };
    if !drag.is_some_and(|drag| drag.started) {
        return false;
    }
    end_drag_input(panel);
    true
}

/// The drag timer (tree drag spec §3.3): near the list's top or bottom edge the list scrolls,
/// and a collapsed folder the pointer has rested on long enough expands. `now` comes in so the
/// tests need not wait.
pub(crate) fn drag_tick(hwnd: HWND, now: Instant) {
    let Some((scrolled, expand, pointer)) = with_view(hwnd, |view| {
        let drag = view.drag.as_ref().filter(|drag| drag.started)?;
        let pointer = drag.pointer;
        let expand = tree_drag::expand_due(drag.resting.as_ref(), now);
        let list = view.list_rect(view.client());
        let lines = tree_drag::scroll_step(pointer.1, list.top, list.bottom, view.list.row_height);
        let scrolled = lines != 0 && view.list.scroll_lines(lines, height(list));
        if scrolled {
            view.invalidate();
        }
        Some((scrolled, expand, pointer))
    })
    .flatten() else {
        return;
    };
    if let Some(folder) = &expand {
        with_view(hwnd, |view| {
            if let Some(drag) = view.drag.as_mut() {
                drag.resting = None;
            }
        });
        set_folder_expanded(hwnd, folder, true);
    }
    if scrolled || expand.is_some() {
        let accepted =
            with_view(hwnd, |view| view.drag_to(pointer.0, pointer.1, now)).unwrap_or(false);
        set_drag_cursor(accepted);
    }
}
```

`handle` changes:

```rust
        WM_MOUSEMOVE => {
            let (x, y) = point_of(lparam);
            if !drag_move(hwnd, x, y, wparam) {
                mouse_move(hwnd, x, y);
            }
            Some(0)
        }
        // ...
        WM_LBUTTONUP => {
            let (x, y) = point_of(lparam);
            if drag_release(hwnd, x, y) {
                return Some(0);
            }
            // Released after the borrow ends: ReleaseCapture sends WM_CAPTURECHANGED here.
            if with_view(hwnd, |view| view.thumb_grab.take().is_some()).unwrap_or(false) {
                unsafe {
                    ReleaseCapture();
                }
            }
            Some(0)
        }
        WM_CAPTURECHANGED => {
            with_view(hwnd, |view| view.thumb_grab = None);
            // Capture taken away mid-drag (a task switch, a dialog): nothing moves.
            cancel_drag(hwnd);
            Some(0)
        }
        WM_RBUTTONDOWN => {
            // A right press cancels a drag and does nothing else (tree drag spec §3.3).
            if cancel_drag(hwnd) {
                with_view(hwnd, |view| view.eat_right_up = true);
                return Some(0);
            }
            // (the existing body follows unchanged)
        }
        WM_RBUTTONUP => {
            // The release of a right press that cancelled a drag opens no menu.
            let eaten =
                with_view(hwnd, |view| std::mem::take(&mut view.eat_right_up)).unwrap_or(false);
            eaten.then_some(0)
        }
        WM_TIMER if wparam == DRAG_TIMER => {
            drag_tick(hwnd, Instant::now());
            Some(0)
        }
```

`left_down` changes:

```rust
fn left_down(hwnd: HWND, x: i32, y: i32) {
    // A drag armed by an earlier press whose release never came here.
    with_view(hwnd, |view| view.drag = None);
    let hit = hit_after_commit(hwnd, x, y);
    // ... unchanged until the Row arm:
        Hit::Row { index, part } => {
            with_view(hwnd, |view| view.select(index));
            // Read before the click acts: opening a note or toggling a folder can move rows.
            let source = (part == RowPart::Body)
                .then(|| with_view(hwnd, |view| view.rows.get(index).map(|row| row.kind.clone())))
                .flatten()
                .flatten();
            row_clicked(hwnd, index, part, false);
            if let Some(source) = source {
                arm_drag(hwnd, source, x, y);
            }
        }
```

`double_click` calls `left_down` for non-row hits. That's fine. Its row path calls `row_clicked` directly, so a double-click never arms a drag.

`rebuild` changes:

```rust
pub(crate) fn rebuild(hwnd: HWND) {
    let snapshot = snapshot(hwnd);
    let names = crate::library::local::display_names(&snapshot.recent);
    let lost = with_view(hwnd, |view| {
        view.apply(snapshot, names);
        view.invalidate();
        // A drag whose row went ends; a target folder that went is found again at the next
        // move (tree drag spec §3.3).
        let rows = &view.rows;
        view.drag.as_mut().is_some_and(|drag| {
            if tree::row_index(rows, &drag.source).is_none() {
                return true;
            }
            if drag.target.as_ref().is_some_and(|folder| {
                !folder.as_os_str().is_empty()
                    && tree::row_index(rows, &RowKind::Folder(folder.clone())).is_none()
            }) {
                drag.target = None;
            }
            false
        })
    })
    .unwrap_or(false);
    if lost {
        cancel_drag(hwnd);
    }
    // The field follows its row, or goes with an edit the rebuild ended (inline naming spec §5.4).
    super::inline_name::place(hwnd);
}
```

- [ ] **Step 4: Change `side_panel.rs`.**
  - Import `WM_TIMER` from `windows_sys::Win32::UI::WindowsAndMessaging`.
  - Add this arm immediately before the `WM_KEYDOWN if wparam as u16 == VK_ESCAPE => { return_focus_to_editor(main); 0 }` arm:

```rust
        // Esc during a tree drag cancels it; the focus stays in the tree (tree drag spec §3.3).
        WM_KEYDOWN
            if wparam as u16 == VK_ESCAPE && crate::window::notebook_view::cancel_drag(main) =>
        {
            0
        }
```

  - Add `| WM_TIMER` to the arm that calls `route(main, panel, message, wparam, lparam)` for mouse messages.
  - Add `crate::window::notebook_view::cancel_drag(hwnd);` right after `crate::window::inline_name::cancel(hwnd);` at both call sites: the sidebar hiding (~405) and `show_view_now` switching away from Notebook (~499).

- [ ] **Step 5: Run the tests.**

Run: `cargo test --lib tree_drag_ -- --test-threads=1`
Expected: 7 passed.

Run: `cargo test --lib notebook_view -- --test-threads=1` and `cargo test --lib inline -- --test-threads=1`
Expected: every test passes. These cover today's clicks, the scroll thumb and the inline edits.

Run: `cargo clippy --all-targets -- -D warnings`
Expected: clean. Remove any `expect(dead_code)` left over from Task 1.

The cursor check uses `GetCursor`. If it's unreliable in the test process (for example, if it always returns the class cursor), keep the other assertions and replace the cursor assertion with a check on the returned target. Report that as `DONE_WITH_CONCERNS`, with the observed value.

- [ ] **Step 6: Commit.**

```bash
cargo fmt
git add src/window/notebook_view.rs src/window/side_panel.rs src/window/main_window.rs src/window/tree_drag.rs src/library/mod.rs
git commit -m "feat(sidebar): drag a note or folder in the Notebook tree to move it"
```

---

### Task 4: Drop highlight, dimmed row, and docs

**Files:**
- Modify: `src/window/notebook_view.rs`:
  - `draw_tree_row` (~446) gains `dimmed: bool` as its last parameter;
  - the `Mode::Tree` paint (~1206);
  - new `band_rect` and `paint_band`;
  - the existing pixel tests' calls pass `false`.
- Modify: `docs/superpowers/specs/2026-09-24-notebook-folders-design.md` §8
- Modify: `README.md` (the notebook bullets, ~126–130)
- Modify: `docs/superpowers/specs/2026-09-25-tree-drag-move-design.md` (append §10)

**Interfaces:**
- Consumes: Task 1's `tree_drag::{highlight, Highlight}` and Task 3's `NotebookView.drag`.
- Produces:
  - `pub(crate) fn band_rect(list: RECT, state: &RowListState, highlight: tree_drag::Highlight) -> Option<RECT>`
  - `fn paint_band(dc: HDC, band: RECT, palette: &Palette, dpi: u32, before_rows: bool)`

- [ ] **Step 1: Write the failing tests.** Put them in `notebook_view.rs`'s `mod tests`, using the existing `row` helper and `TestTarget`.

```rust
    #[test]
    fn the_drop_band_covers_the_folders_rows_in_view_or_the_whole_list() {
        use crate::window::tree_drag::Highlight;
        let list_rect = RECT { left: 0, top: 100, right: 200, bottom: 230 };
        let mut state = RowListState::new(26);
        state.set_count(20);
        state.top = 3;
        let band = |highlight| band_rect(list_rect, &state, highlight);
        assert_eq!(band(Highlight::Root).map(edges), Some((0, 100, 200, 230)));
        assert_eq!(
            band(Highlight::Rows { start: 4, end: 6 }).map(edges),
            Some((0, 126, 200, 178))
        );
        assert_eq!(
            band(Highlight::Rows { start: 0, end: 5 }).map(edges),
            Some((0, 100, 200, 152)),
            "clipped at the top of the view"
        );
        assert_eq!(
            band(Highlight::Rows { start: 6, end: 20 }).map(edges),
            Some((0, 178, 200, 230)),
            "clipped at the bottom of the list"
        );
        assert_eq!(band(Highlight::Rows { start: 0, end: 2 }), None, "above the view");
    }

    #[test]
    fn the_drop_band_fills_outside_high_contrast_and_outlines_in_it() {
        use crate::window::icon_sets::images::TestTarget;
        let target = TestTarget::new(60, 40);
        let whole = RECT { left: 0, top: 0, right: 60, bottom: 40 };
        let band = RECT { left: 10, top: 10, right: 50, bottom: 30 };
        let inside = RECT { left: 20, top: 15, right: 21, bottom: 16 };
        let edge = RECT { left: 10, top: 20, right: 11, bottom: 21 };
        let reference = |color: u32| {
            let target = TestTarget::new(1, 1);
            unsafe { fill(target.dc, RECT { left: 0, top: 0, right: 1, bottom: 1 }, color) };
            target.area(RECT { left: 0, top: 0, right: 1, bottom: 1 })[0]
        };
        let palette = Palette::neutral();
        unsafe { fill(target.dc, whole, palette.editor_background) };
        paint_band(target.dc, band, &palette, 96, true);
        paint_band(target.dc, band, &palette, 96, false);
        assert_eq!(target.area(inside)[0], reference(palette.inactive_selection_background));

        let contrast = Palette { high_contrast: true, ..palette };
        unsafe { fill(target.dc, whole, contrast.editor_background) };
        paint_band(target.dc, band, &contrast, 96, true);
        assert_eq!(target.area(inside)[0], reference(contrast.editor_background), "no blend");
        paint_band(target.dc, band, &contrast, 96, false);
        assert_eq!(target.area(edge)[0], reference(contrast.selection_background));
        assert_eq!(target.area(inside)[0], reference(contrast.editor_background));
    }

    #[test]
    fn the_dragged_row_draws_its_name_dimmed() {
        use crate::window::titlebar::create_ui_font;
        use windows_sys::Win32::Graphics::Gdi::{DeleteObject, FW_NORMAL};
        use crate::window::icon_sets::images::TestTarget;
        let fonts = UiFonts {
            text: create_ui_font(12, "Segoe UI", FW_NORMAL as i32, false),
            ..UiFonts::default()
        };
        let rect = RECT { left: 0, top: 0, right: 200, bottom: 22 };
        let name = row_parts(rect, 0, 96).name;
        let look = RowLook { selected: false, hover: false, focused: false };
        let palette = Palette::neutral();
        let note = TreeRow { name: "dragged".into(), ..row(RowKind::Note("a.md".into()), 0) };
        let mut images = IconImages::new();
        let mut draw = |dimmed: bool| {
            let target = TestTarget::new(200, 22);
            unsafe { fill(target.dc, rect, palette.editor_background) };
            draw_tree_row(
                target.dc, Some(&note), rect, look, &palette, &FileIcons::neutral(), fonts, 96,
                false, None, &mut images, FileIconSet::Minimal, true, dimmed,
            );
            target.area(name)
        };
        assert_ne!(draw(true), draw(false));
        unsafe { DeleteObject(fonts.text) };
    }
```

If `Palette::neutral()`'s `muted_foreground` equals its `editor_foreground`, the dimmed test can't tell the two apart. In that case, build the palette with `Palette { muted_foreground: <a distinct value>, ..Palette::neutral() }`.

- [ ] **Step 2: Run the tests before implementing.**

Run: `cargo test --lib notebook_view::tests::the_d`
Expected: a compile error (`band_rect`, `paint_band`, and the extra `draw_tree_row` argument are missing).

- [ ] **Step 3: Implement.**

In `draw_tree_row`, add the last parameter `dimmed: bool`, documented in the doc comment as "the row being dragged: its name draws muted (tree drag spec §3.2)". Then replace the final name draw:

```rust
    let color = if dimmed { palette.muted_foreground } else { foreground };
    unsafe { draw_text(dc, &row.name, parts.name, font, color, LINE) };
```

Add these free functions after `draw_recent_row`:

```rust
/// Where `highlight` shows in the tree's `list` area (tree drag spec §3.2): the whole list for
/// the root, else the part of the folder's rows in view; `None` when none of them is.
pub(crate) fn band_rect(
    list: RECT,
    state: &RowListState,
    highlight: tree_drag::Highlight,
) -> Option<RECT> {
    match highlight {
        tree_drag::Highlight::Root => Some(list),
        tree_drag::Highlight::Rows { start, end } => {
            let visible = state.visible_rows(height(list));
            let first = start.max(state.top);
            let last = end.min(state.top + visible);
            (first < last).then(|| RECT {
                left: list.left,
                top: list.top + (first - state.top) as i32 * state.row_height,
                right: list.right,
                bottom: (list.top + (last - state.top) as i32 * state.row_height)
                    .min(list.bottom),
            })
        }
    }
}

/// The drop target's band (tree drag spec §3.2). Called before the rows paint
/// (`before_rows`), it fills the band with the softer selection colour; after, in high
/// contrast only, it outlines the band in the system highlight, since a blend is not allowed
/// there.
fn paint_band(dc: HDC, band: RECT, palette: &Palette, dpi: u32, before_rows: bool) {
    if before_rows && !palette.high_contrast {
        unsafe { fill(dc, band, palette.inactive_selection_background) };
    } else if !before_rows && palette.high_contrast {
        let t = scale(1, dpi).max(1);
        let color = palette.selection_background;
        for edge in [
            RECT { bottom: band.top + t, ..band },
            RECT { top: band.bottom - t, ..band },
            RECT { right: band.left + t, ..band },
            RECT { left: band.right - t, ..band },
        ] {
            unsafe { fill(dc, edge, color) };
        }
    }
}
```

Import `tree_drag` in `notebook_view.rs` if Task 3 didn't. In the `Mode::Tree` paint, work out the band and the dragged row before `row_list::paint`:

```rust
                let drag = self.drag.as_ref().filter(|drag| drag.started);
                let dragged = drag.map(|drag| drag.source.clone());
                let band = drag
                    .and_then(|drag| drag.target.as_deref())
                    .and_then(|folder| tree_drag::highlight(rows, folder))
                    .and_then(|highlight| band_rect(list, &self.list, highlight));
                if let Some(band) = band {
                    paint_band(dc, band, palette, dpi, true);
                }
```

Pass the dimmed flag into the row closure's `draw_tree_row` call:

```rust
                            dragged.as_ref() == rows.get(index).map(|row| &row.kind),
```

After `row_list::paint`, before the inline field paints:

```rust
                if let Some(band) = band {
                    paint_band(dc, band, palette, dpi, false);
                }
```

Update the existing calls to `draw_tree_row` in the tests module to pass `false` as the new last argument. Resolve borrow conflicts between `self.drag`, `rows` and `self.list` by reading them into locals before the closure, as the existing code does for `rows` and `images`.

- [ ] **Step 4: Update the docs.**
  - **The notebook folders spec §8:** replace `- Moving notes or folders: drag and drop, or a "Move to folder" command.` with `- A "Move to folder" command. Dragging in the tree moves notes and folders (tree drag spec, \`2026-09-25-tree-drag-move-design.md\`).`
  - **`README.md`:** after the bullet that ends "each note has a coloured icon for its type.", add:

    ```
    - Drag a note or a folder onto another folder in the tree to move it there, or onto empty
      space to move it to the notebook's top level. Open tabs and pins follow it, nothing is
      ever overwritten, and Esc cancels the drag.
    ```

  - **The tree drag spec:** append this section:

    ```markdown
    ## 10. Decisions made while implementing

    - **The press still clicks.** A press on a row selects it and opens the note or toggles the folder, as before; the drag is armed after that. The dragged item is followed by its path, since toggling a folder moves the rows below it.
    - **No capture until the drag starts.** A release the tree never sees (over another window before the drag distance) is noticed at the next mouse move without the button, which disarms.
    - **Highlight colours:** the band uses the theme's inactive-selection colour; in high contrast, a 1 px (scaled) outline in the system highlight colour.
    - **Shared move code:** the inline renames and the drop use the same moves (`window::tree_move`), so a rename and a move fail, undo and report stuck tabs the same way. A move whose undo failed names the new path in the existing "could not undo renaming" notice.
    - **A right press that cancels a drag** swallows its release, so no context menu opens.
    ```

- [ ] **Step 5: Run the tests.**

Run: `cargo test --lib notebook_view -- --test-threads=1` and `cargo test --lib tree_drag -- --test-threads=1`
Expected: all pass.

Run: `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check`
Expected: clean.

- [ ] **Step 6: Commit.**

```bash
cargo fmt
git add src/window/notebook_view.rs README.md docs/superpowers/specs/2026-09-24-notebook-folders-design.md docs/superpowers/specs/2026-09-25-tree-drag-move-design.md
git commit -m "feat(sidebar): the drop target's rows highlight and the dragged row dims; docs"
```
