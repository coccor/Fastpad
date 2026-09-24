//! Naming notes and folders in the Notebook tree (inline naming spec): an `EDIT` field over a
//! row's name for New note, New folder and both renames. This half is pure: what the typed text
//! names, the live checks against the names beside it, the rename selection, the draft row, and
//! where the field and its message go. No Win32 calls and no disk.
#![cfg_attr(
    not(test),
    expect(dead_code, reason = "wired up task by task by the inline naming plan")
)]

use crate::library::title;
use crate::library::tree::{self, RowKind, TreeRow};
use crate::window::file_icons::{FOLDER_ICON, FileIcon, file_icon};
use crate::window::panel::scale;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use windows_sys::Win32::Foundation::RECT;

// Sizes at 96 DPI; everything is scaled with `panel::scale`.
/// How far the frame starts before the row's name.
const FRAME_OUTSET: i32 = 3;
/// The frame's gap to the row's top and bottom edges.
const FRAME_INSET_Y: i32 = 2;
/// Where the text starts inside the frame.
const TEXT_INSET: i32 = 3;

/// What the field names. Paths are relative to the notebook; a new item's is the folder it
/// goes in, empty for the root.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Purpose {
    NewNote(PathBuf),
    NewFolder(PathBuf),
    RenameNote(PathBuf),
    RenameFolder(PathBuf),
}

impl Purpose {
    /// The folder the name goes in (empty for the notebook root).
    pub(crate) fn parent(&self) -> &Path {
        match self {
            Self::NewNote(parent) | Self::NewFolder(parent) => parent,
            Self::RenameNote(path) | Self::RenameFolder(path) => {
                path.parent().unwrap_or(Path::new(""))
            }
        }
    }

    /// A new item's folder: the edit shows a draft row there (spec §3.1).
    pub(crate) fn draft_parent(&self) -> Option<&Path> {
        match self {
            Self::NewNote(parent) | Self::NewFolder(parent) => Some(parent),
            Self::RenameNote(_) | Self::RenameFolder(_) => None,
        }
    }

    /// The row being renamed.
    pub(crate) fn own_row(&self) -> Option<RowKind> {
        match self {
            Self::RenameNote(path) => Some(RowKind::Note(path.clone())),
            Self::RenameFolder(path) => Some(RowKind::Folder(path.clone())),
            Self::NewNote(_) | Self::NewFolder(_) => None,
        }
    }

    pub(crate) fn is_folder(&self) -> bool {
        matches!(self, Self::NewFolder(_) | Self::RenameFolder(_))
    }

    /// A rename's current name, which the field starts with (spec §3.3).
    pub(crate) fn current_name(&self) -> Option<String> {
        match self {
            Self::RenameNote(path) | Self::RenameFolder(path) => {
                Some(path.file_name()?.to_string_lossy().into_owned())
            }
            Self::NewNote(_) | Self::NewFolder(_) => None,
        }
    }
}

/// The name `text` gives the item (spec §4), or `None` when it cancels: nothing left once
/// cleaned, or a rename to the name it already has. A change of letter case is a rename.
pub(crate) fn typed_name(purpose: &Purpose, text: &str) -> Option<String> {
    let name = match purpose {
        Purpose::NewNote(_) => title::new_note_name(text)?,
        Purpose::NewFolder(_) | Purpose::RenameFolder(_) => title::folder_name(text)?,
        Purpose::RenameNote(path) => {
            let current = path
                .extension()
                .map(|extension| extension.to_string_lossy());
            title::renamed_note_name(text, current.as_deref())?
        }
    };
    match purpose.current_name() {
        Some(current) if current == name => None,
        _ => Some(name),
    }
}

/// "<name> already exists here." (spec §4.4).
pub(crate) fn taken_message(name: &str) -> String {
    format!("{name} already exists here.")
}

/// The live check (spec §4.4): the problem the field shows for `text`, or `None`. `siblings`
/// holds the lowercased names listed beside the item, its own row left out. A name that
/// cancels has no problem.
pub(crate) fn check(purpose: &Purpose, text: &str, siblings: &HashSet<String>) -> Option<String> {
    let name = typed_name(purpose, text)?;
    if purpose.is_folder() && crate::library::scan::skip_directory(&name) {
        return Some(super::library_host::hidden_folder_error(&name));
    }
    siblings
        .contains(&name.to_lowercase())
        .then(|| taken_message(&name))
}

/// The part of `name` a rename selects, in UTF-16 units (spec §3.3): a note's name before its
/// last `.`, all of a name whose only `.` starts it, and all of a folder's.
pub(crate) fn rename_selection(name: &str, folder: bool) -> (usize, usize) {
    let all = name.encode_utf16().count();
    if folder {
        return (0, all);
    }
    match name.rfind('.') {
        Some(dot) if dot > 0 => (0, name[..dot].encode_utf16().count()),
        _ => (0, all),
    }
}

/// The draft row's icon (spec §3.1): the folder icon, or the note icon for the note extension
/// typed so far, Markdown until one is.
pub(crate) fn draft_icon(purpose: &Purpose, text: &str) -> FileIcon {
    if purpose.is_folder() {
        return FOLDER_ICON;
    }
    let extension = text
        .trim()
        .rsplit_once('.')
        .map(|(_, extension)| extension)
        .filter(|extension| title::is_note_extension(extension))
        .unwrap_or("md");
    file_icon(Some(extension))
}

/// The field's accessible name (spec §6). `notebook` names the root.
pub(crate) fn accessible_name(purpose: &Purpose, notebook: &str) -> String {
    let place = |parent: &Path| {
        parent.file_name().map_or_else(
            || notebook.to_owned(),
            |name| name.to_string_lossy().into_owned(),
        )
    };
    match purpose {
        Purpose::NewNote(parent) => format!("New note name, in {}", place(parent)),
        Purpose::NewFolder(parent) => format!("New folder name, in {}", place(parent)),
        Purpose::RenameNote(path) | Purpose::RenameFolder(path) => format!(
            "Rename {}",
            path.file_name().unwrap_or_default().to_string_lossy()
        ),
    }
}

/// The row of the folder `parent`: `Some(None)` for the notebook root, `None` when the folder
/// has no row.
pub(crate) fn parent_row(rows: &[TreeRow], parent: &Path) -> Option<Option<usize>> {
    if parent.as_os_str().is_empty() {
        return Some(None);
    }
    tree::row_index(rows, &RowKind::Folder(parent.to_path_buf())).map(Some)
}

/// The lowercased names of the notes and folders listed in the folder at row `parent` (`None`:
/// the root), leaving out row `own`: what a name typed there may not take (spec §4.4).
pub(crate) fn sibling_names(
    rows: &[TreeRow],
    parent: Option<usize>,
    own: Option<usize>,
) -> HashSet<String> {
    let (start, depth) = match parent {
        Some(index) => match rows.get(index) {
            Some(folder) => (index + 1, folder.depth.saturating_add(1)),
            None => return HashSet::new(),
        },
        None => (0, 0),
    };
    rows.iter()
        .enumerate()
        .skip(start)
        .take_while(|(_, row)| row.depth >= depth)
        .filter(|&(index, row)| {
            row.depth == depth
                && Some(index) != own
                && matches!(row.kind, RowKind::Folder(_) | RowKind::Note(_))
        })
        .map(|(_, row)| row.name.to_lowercase())
        .collect()
}

/// Puts the draft row in `rows` as the first child of the folder at row `parent`, one level
/// deeper, or at the root below the unsaved rows (spec §3.1). `None`, changing nothing, when
/// that folder is collapsed or gone. Returns the draft row's index.
pub(crate) fn insert_draft(rows: &mut Vec<TreeRow>, parent: Option<usize>) -> Option<usize> {
    let (at, depth) = match parent {
        None => (
            rows.iter()
                .take_while(|row| matches!(row.kind, RowKind::Unsaved(_)))
                .count(),
            0,
        ),
        Some(index) => {
            let folder = rows.get(index)?;
            if !folder.expanded {
                return None;
            }
            (index + 1, folder.depth.saturating_add(1))
        }
    };
    rows.insert(
        at,
        TreeRow {
            kind: RowKind::Draft,
            depth,
            name: String::new(),
            pinned: false,
            expanded: false,
        },
    );
    Some(at)
}

/// Where Ctrl+Backspace deletes back to from `caret` in `text` (UTF-16): past any spaces, then
/// past one run of letters and digits, or of other characters.
pub(crate) fn word_start(text: &[u16], caret: usize) -> usize {
    // 0 space, 1 letter or digit (a surrogate half counts as one), 2 anything else.
    let class = |unit: u16| match char::from_u32(u32::from(unit)) {
        Some(ch) if ch.is_whitespace() => 0,
        Some(ch) if !ch.is_alphanumeric() => 2,
        _ => 1,
    };
    let mut start = caret.min(text.len());
    while start > 0 && class(text[start - 1]) == 0 {
        start -= 1;
    }
    if start > 0 {
        let run = class(text[start - 1]);
        while start > 0 && class(text[start - 1]) == run {
            start -= 1;
        }
    }
    start
}

/// The field's frame and the `Edit` inside it, in panel coordinates. (`RECT` is only `Clone` and
/// `Copy`, so this is too.)
#[derive(Clone, Copy)]
pub(crate) struct FieldLayout {
    pub(crate) frame: RECT,
    pub(crate) edit: RECT,
}

/// Where the field goes for the row at `row` (depth `depth`), clipped to the `list` area so it
/// never covers the header (spec §5.4). The frame covers the name, from just before it to the
/// pin; the chevron, icon and pin stay in view (§3.3). `None` when the row is out of view, or
/// too narrow for any text.
pub(crate) fn field_layout(
    row: RECT,
    list: RECT,
    depth: u16,
    dpi: u32,
    text_height: i32,
) -> Option<FieldLayout> {
    if row.top < list.top || row.top >= list.bottom {
        return None;
    }
    let name = super::notebook_view::row_parts(row, depth, dpi).name;
    let frame = RECT {
        left: (name.left - scale(FRAME_OUTSET, dpi)).max(row.left),
        top: row.top + scale(FRAME_INSET_Y, dpi),
        right: name.right,
        bottom: (row.bottom - scale(FRAME_INSET_Y, dpi)).min(list.bottom),
    };
    let text_height = text_height.clamp(1, (frame.bottom - frame.top - 2).max(1));
    let top = frame.top + (frame.bottom - frame.top - text_height) / 2;
    let edit = RECT {
        left: frame.left + scale(TEXT_INSET, dpi),
        top,
        right: frame.right - 1,
        bottom: (top + text_height).min(frame.bottom - 1),
    };
    (edit.right > edit.left && edit.bottom > edit.top).then_some(FieldLayout { frame, edit })
}

/// Where a problem `height` tall goes (spec §4.4): under the frame, over the row beneath, or
/// above it when the list has no room below. Never above the list's top.
pub(crate) fn message_rect(frame: RECT, list: RECT, height: i32) -> RECT {
    if frame.bottom + height <= list.bottom {
        return RECT {
            top: frame.bottom,
            bottom: frame.bottom + height,
            ..frame
        };
    }
    RECT {
        top: (frame.top - height).max(list.top),
        bottom: frame.top,
        ..frame
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(kind: RowKind, name: &str, depth: u16, expanded: bool) -> TreeRow {
        TreeRow {
            kind,
            depth,
            name: name.to_owned(),
            pinned: false,
            expanded,
        }
    }

    fn names(names: &[&str]) -> HashSet<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn purpose_accessors_read_the_parent_draft_parent_and_own_row() {
        // Break caught: a new item's folder, draft-row parent or own row read wrong, which
        // would misplace the draft row or check a rename against its own name.
        let new_note = Purpose::NewNote("sub".into());
        assert_eq!(new_note.parent(), Path::new("sub"));
        assert_eq!(new_note.draft_parent(), Some(Path::new("sub")));
        assert_eq!(new_note.own_row(), None);

        let new_folder = Purpose::NewFolder(PathBuf::new());
        assert_eq!(new_folder.parent(), Path::new(""));
        assert_eq!(new_folder.draft_parent(), Some(Path::new("")));
        assert_eq!(new_folder.own_row(), None);

        let rename_note = Purpose::RenameNote(r"sub\plan.md".into());
        assert_eq!(rename_note.parent(), Path::new("sub"));
        assert_eq!(rename_note.draft_parent(), None);
        assert_eq!(
            rename_note.own_row(),
            Some(RowKind::Note(r"sub\plan.md".into()))
        );

        let rename_folder = Purpose::RenameFolder("archive".into());
        assert_eq!(rename_folder.parent(), Path::new(""));
        assert_eq!(rename_folder.draft_parent(), None);
        assert_eq!(
            rename_folder.own_row(),
            Some(RowKind::Folder("archive".into()))
        );
    }

    #[test]
    fn typed_names_cancel_when_empty_or_unchanged_and_a_case_change_is_a_rename() {
        // Break caught: an empty draft creating "Untitled.md", Enter on an unchanged rename
        // renaming onto itself, or "plan.md" to "Plan.md" treated as no change (spec §4).
        let note = Purpose::NewNote(PathBuf::new());
        assert_eq!(typed_name(&note, "todo").as_deref(), Some("todo.md"));
        assert_eq!(typed_name(&note, "  "), None);
        let folder = Purpose::NewFolder("sub".into());
        assert_eq!(typed_name(&folder, " a/b: c?. ").as_deref(), Some("ab c"));
        assert_eq!(typed_name(&folder, "..."), None);
        let rename = Purpose::RenameNote(r"sub\plan.md".into());
        assert_eq!(typed_name(&rename, "plan.md"), None);
        assert_eq!(typed_name(&rename, "Plan.md").as_deref(), Some("Plan.md"));
        assert_eq!(typed_name(&rename, "draft").as_deref(), Some("draft.md"));
        let rename_folder = Purpose::RenameFolder("v1.2".into());
        assert_eq!(typed_name(&rename_folder, "v1.2"), None);
        assert_eq!(typed_name(&rename_folder, "V1.2").as_deref(), Some("V1.2"));
    }

    #[test]
    fn the_live_check_finds_a_taken_name_ignoring_case_and_refuses_hidden_folder_names() {
        // Break caught: "TODO" slipping past a listed todo.md, a note's own name reported as
        // taken, or a ".git" folder created that the next rescan hides (spec §4.4).
        let siblings = names(&["todo.md", "archive"]);
        let note = Purpose::NewNote(PathBuf::new());
        assert_eq!(
            check(&note, "TODO", &siblings).as_deref(),
            Some("TODO.md already exists here.")
        );
        assert_eq!(check(&note, "other", &siblings), None);
        assert_eq!(check(&note, "", &siblings), None, "an empty name cancels");
        let folder = Purpose::NewFolder(PathBuf::new());
        assert_eq!(
            check(&folder, "Archive", &siblings).as_deref(),
            Some("Archive already exists here.")
        );
        assert_eq!(
            check(&folder, ".git", &siblings).as_deref(),
            Some("FastPad hides folders named \u{201c}.git\u{201d}. Choose another name.")
        );
        // A rename's own row is left out of `siblings` (`sibling_names`' `own`).
        let rename = Purpose::RenameNote("plan.md".into());
        assert_eq!(check(&rename, "PLAN.md", &names(&["b.md"])), None);
        assert_eq!(
            check(&rename, "b", &names(&["b.md"])).as_deref(),
            Some("b.md already exists here.")
        );
    }

    #[test]
    fn a_rename_selects_the_stem_of_a_note_and_all_of_a_folder() {
        // Break caught: typing over "a.md" also replacing ".md", ".gitignore" opening with
        // nothing selected, or a "v1.2" folder keeping ".2" (spec §3.3).
        assert_eq!(rename_selection("a.md", false), (0, 1));
        assert_eq!(rename_selection(".gitignore", false), (0, 10));
        assert_eq!(rename_selection("archive.tar.gz", false), (0, 11));
        assert_eq!(rename_selection("README", false), (0, 6));
        assert_eq!(rename_selection("v1.2", true), (0, 4));
        assert_eq!(rename_selection("é.md", false), (0, 1), "UTF-16 units");
    }

    #[test]
    fn the_draft_icon_follows_the_typed_note_extension() {
        // Break caught: a new JSON note drawn as Markdown, or a folder draft drawn as a note.
        let note = Purpose::NewNote(PathBuf::new());
        assert_eq!(draft_icon(&note, ""), file_icon(Some("md")));
        assert_eq!(draft_icon(&note, "data.json"), file_icon(Some("json")));
        assert_eq!(draft_icon(&note, "v1.2"), file_icon(Some("md")));
        assert_eq!(
            draft_icon(&Purpose::NewFolder(PathBuf::new()), "x.json"),
            FOLDER_ICON
        );
    }

    #[test]
    fn the_field_is_named_for_what_it_names_and_where() {
        // Break caught: a screen reader hearing a bare "edit", or the root named "" (spec §6).
        assert_eq!(
            accessible_name(&Purpose::NewNote(PathBuf::new()), "Notes"),
            "New note name, in Notes"
        );
        assert_eq!(
            accessible_name(&Purpose::NewFolder(r"a\sub".into()), "Notes"),
            "New folder name, in sub"
        );
        assert_eq!(
            accessible_name(&Purpose::RenameNote(r"a\b.md".into()), "Notes"),
            "Rename b.md"
        );
    }

    #[test]
    fn siblings_are_the_rows_directly_in_the_folder_without_the_own_row() {
        // Break caught: a name in a subfolder, an unsaved tab's label or the renamed row itself
        // counted as taken, or a sibling below a nested folder missed.
        let rows = vec![
            row(RowKind::Unsaved(1), "Untitled", 0, false),
            row(RowKind::Folder("sub".into()), "sub", 0, true),
            row(RowKind::Note(r"sub\A.md".into()), "A.md", 1, false),
            row(RowKind::Folder(r"sub\deep".into()), "deep", 1, true),
            row(RowKind::Note(r"sub\deep\x.md".into()), "x.md", 2, false),
            row(RowKind::Note(r"sub\b.md".into()), "b.md", 1, false),
            row(RowKind::Note("top.md".into()), "top.md", 0, false),
        ];
        assert_eq!(parent_row(&rows, Path::new("sub")), Some(Some(1)));
        assert_eq!(parent_row(&rows, Path::new("")), Some(None));
        assert_eq!(parent_row(&rows, Path::new("gone")), None);
        assert_eq!(
            sibling_names(&rows, Some(1), None),
            names(&["a.md", "deep", "b.md"])
        );
        assert_eq!(
            sibling_names(&rows, Some(1), Some(2)),
            names(&["deep", "b.md"])
        );
        assert_eq!(sibling_names(&rows, None, None), names(&["sub", "top.md"]));
    }

    #[test]
    fn the_draft_row_is_the_first_child_of_an_expanded_folder_or_below_the_unsaved_rows() {
        // Break caught: a draft row at the end of its folder, at the wrong depth, above the
        // unsaved rows, or inside a collapsed folder (spec §3.1).
        let mut rows = vec![
            row(RowKind::Unsaved(1), "Untitled", 0, false),
            row(RowKind::Folder("sub".into()), "sub", 0, true),
            row(RowKind::Note(r"sub\a.md".into()), "a.md", 1, false),
            row(RowKind::Folder("shut".into()), "shut", 0, false),
        ];
        assert_eq!(insert_draft(&mut rows, Some(1)), Some(2));
        assert_eq!((rows[2].kind.clone(), rows[2].depth), (RowKind::Draft, 1));
        rows.remove(2);
        assert_eq!(insert_draft(&mut rows, None), Some(1));
        assert_eq!((rows[1].kind.clone(), rows[1].depth), (RowKind::Draft, 0));
        rows.remove(1);
        assert_eq!(insert_draft(&mut rows, Some(3)), None, "collapsed");
        assert_eq!(rows.len(), 4);
    }

    #[test]
    fn ctrl_backspace_deletes_spaces_then_one_run_of_word_or_punctuation() {
        // Break caught: Ctrl+Backspace typing a box character or deleting the whole name.
        let wide = |text: &str| text.encode_utf16().collect::<Vec<_>>();
        assert_eq!(word_start(&wide("my note.md"), 10), 8);
        assert_eq!(word_start(&wide("my note.md"), 8), 7);
        assert_eq!(word_start(&wide("my note  "), 9), 3);
        assert_eq!(word_start(&wide("my"), 0), 0);
        assert_eq!(word_start(&wide("my"), 99), 0, "a caret past the end");
    }

    #[test]
    fn the_field_covers_the_name_up_to_the_pin_and_stays_inside_the_list() {
        // Break caught: the field drawn over the chevron, icon or pin, over the header when its
        // row is scrolled up, or below the list's bottom edge (spec §3.3, §5.4).
        let list = RECT {
            left: 0,
            top: 38,
            right: 240,
            bottom: 400,
        };
        let row_at = |top: i32| RECT {
            left: 0,
            top,
            right: 240,
            bottom: top + 26,
        };
        let parts = super::super::notebook_view::row_parts(row_at(60), 1, 96);
        let layout = field_layout(row_at(60), list, 1, 96, 16).unwrap();
        assert!(layout.frame.left > parts.icon.right - 1);
        assert_eq!(layout.frame.right, parts.pin.left);
        assert!(layout.edit.left > layout.frame.left && layout.edit.right < layout.frame.right);
        assert!(layout.edit.top > layout.frame.top && layout.edit.bottom < layout.frame.bottom);
        assert!(
            field_layout(row_at(12), list, 1, 96, 16).is_none(),
            "under the header"
        );
        assert!(
            field_layout(row_at(400), list, 1, 96, 16).is_none(),
            "below the list"
        );
        let cut = field_layout(row_at(390), list, 1, 96, 16).unwrap();
        assert!(cut.frame.bottom <= list.bottom && cut.edit.bottom <= list.bottom);
    }

    #[test]
    fn the_problem_goes_under_the_field_or_above_it_on_the_last_row() {
        // Break caught: a message drawn past the list's bottom, hidden under the next paint, or
        // over the header (spec §4.4).
        let list = RECT {
            left: 0,
            top: 38,
            right: 240,
            bottom: 400,
        };
        let frame = RECT {
            left: 55,
            top: 62,
            right: 216,
            bottom: 84,
        };
        let below = message_rect(frame, list, 30);
        assert_eq!((below.top, below.bottom), (84, 114));
        let last = RECT {
            top: 380,
            bottom: 398,
            ..frame
        };
        let above = message_rect(last, list, 30);
        assert_eq!((above.top, above.bottom), (350, 380));
        let tiny = RECT {
            top: 38,
            bottom: 70,
            ..list
        };
        let first = RECT {
            top: 40,
            bottom: 60,
            ..frame
        };
        assert_eq!(message_rect(first, tiny, 40).top, 38);
    }
}
