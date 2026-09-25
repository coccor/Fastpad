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
#[cfg_attr(test, allow(dead_code))]
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
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "used by the tree drag in notebook_view (next tasks)"
    )
)]
pub(crate) enum Highlight {
    /// The whole list.
    Root,
    /// A folder's row and the rows shown under it: `start..end`.
    Rows { start: usize, end: usize },
}

/// What dropping into `folder` highlights; `None` when the folder has no row now.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "used by the tree drag in notebook_view (next tasks)"
    )
)]
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
        assert_eq!(
            (drag.origin, drag.pointer, drag.target),
            ((1, 2), (1, 2), None)
        );
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
        assert_eq!(
            at(Hover::Row(3)),
            Some("work".into()),
            "a note row: its folder"
        );
        assert_eq!(
            at(Hover::Row(4)),
            Some(PathBuf::new()),
            "a top-level note: the root"
        );
        assert_eq!(
            at(Hover::Row(0)),
            Some(PathBuf::new()),
            "an unsaved row: the root"
        );
        assert_eq!(at(Hover::Row(5)), Some(PathBuf::new()), "the truncated row");
        assert_eq!(at(Hover::Below), Some(PathBuf::new()));
        assert_eq!(at(Hover::Header), Some(PathBuf::new()));
        assert_eq!(at(Hover::Outside), None);
        let draft = vec![row(RowKind::Draft, 0, false)];
        assert_eq!(drop_folder(&draft, Hover::Row(0)), None);
    }

    #[test]
    fn a_folder_refuses_its_own_items_and_a_folder_refuses_itself_and_its_insides() {
        assert!(
            !accepts(&note(r"work\b.md"), Path::new("work")),
            "its own folder"
        );
        assert!(
            !accepts(&note(r"work\b.md"), Path::new("WORK")),
            "in any letter case"
        );
        assert!(
            !accepts(&note("a.md"), Path::new("")),
            "a top-level note on the root"
        );
        assert!(accepts(&note("a.md"), Path::new("work")));
        assert!(accepts(&note(r"work\b.md"), Path::new("")));
        assert!(!accepts(&folder("work"), Path::new("work")), "itself");
        assert!(
            !accepts(&folder("work"), Path::new(r"work\inner")),
            "inside itself"
        );
        assert!(!accepts(&folder("work"), Path::new(r"Work\Inner\deep")));
        assert!(
            !accepts(&folder(r"work\inner"), Path::new("work")),
            "its own parent"
        );
        assert!(accepts(&folder(r"work\inner"), Path::new("")));
        assert!(
            accepts(&folder("work"), Path::new("workshop")),
            "a name prefix is not inside"
        );
        assert!(!accepts(&RowKind::Unsaved(7), Path::new("work")));
    }

    #[test]
    fn a_drop_keeps_the_name_in_the_new_folder() {
        assert_eq!(
            destination(&note("a.md"), Path::new("work")),
            Some(r"work\a.md".into())
        );
        assert_eq!(
            destination(&folder(r"work\inner"), Path::new("")),
            Some("inner".into())
        );
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
        assert!(
            !drag.hover(&rows, (5, 6), Hover::Row(1), now),
            "same target, no repaint"
        );
        assert!(
            drag.hover(&rows, (5, 7), Hover::Below, now),
            "the root is a.md's own folder"
        );
        assert_eq!(drag.target, None);
        assert!(!drag.hover(&rows, (5, 8), Hover::Outside, now));
    }

    #[test]
    fn a_collapsed_target_folder_expands_after_resting_700_ms_on_it() {
        let rows = rows();
        let start = Instant::now();
        let mut drag = Drag::armed(note("a.md"), 0, 0).unwrap();
        drag.hover(&rows, (0, 0), Hover::Row(2), start);
        assert_eq!(
            drag.resting.as_ref().map(|(path, _)| path.clone()),
            Some(r"work\inner".into())
        );
        // Moving within the same row keeps the first time.
        drag.hover(
            &rows,
            (1, 0),
            Hover::Row(2),
            start + Duration::from_millis(300),
        );
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
        assert_eq!(
            highlight(&last, Path::new("z")),
            Some(Highlight::Rows { start: 0, end: 2 })
        );
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
