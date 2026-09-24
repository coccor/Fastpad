# Inline Naming Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Name new notes, new folders and renames in a real `EDIT` field inside the Notebook tree, at the row where the item will be, as VS Code's explorer does; New note creates the file first.

**Architecture:**
- **Pure code:** `library::title` gains `new_note_name` and `renamed_note_name` (the spec's splits, `None` for a name that cleans to nothing). The new `window::inline_name` module starts with the pure half: `Purpose` (what is being named), `typed_name`, the live `check` against the lowercased names of the rows beside it (`sibling_names`), `rename_selection`, `draft_icon`, `accessible_name`, `insert_draft`, `word_start` (Ctrl+Backspace) and the placement maths (`field_layout`, `message_rect`). `RowKind::Draft` is the draft row's kind; the tree never builds one.
- **Window code:** `inline_name::InlineName` lives in `NotebookView`. It owns the field (made on first use, subclassed for Enter, Esc, Tab, Ctrl+A, Ctrl+Backspace and focus loss) and the one open `Edit`. Every rebuild runs `InlineName::fit` inside `NotebookView::apply`: it puts the draft row back as the first child of its folder, finds the renamed row, recomputes the sibling names and re-runs the check, or cancels the edit when its folder, row or notebook is gone. `inline_name::place` moves the field over the row (after rebuilds, scrolls, selection changes and layout), clipped to the list. The panel paints the frame and the problem. Each commit makes the one disk call (`create_new` file, `create_dir`, `rename_no_replace`), then updates the library, rebinds tabs and refreshes. Focus leaving the field for a FastPad window posts `WM_FASTPAD_INLINE_NAME_LEFT`, which commits; focus leaving FastPad arms a refocus that `WM_SETFOCUS` on the frame honours. `CommandId::NoteNew = 193` drives New note. The name bar keeps only `FirstSave` and `RenameNote` (for a file with no tree row).

**Tech Stack:** Rust 2024, `windows-sys` 0.61 (Win32 `EDIT`, `SetWindowSubclass`, GDI, `IAccPropServices` dynamic annotation through a hand-rolled COM vtable, `NotifyWinEvent`). No new crates.

**Spec:** `docs/superpowers/specs/2026-09-24-inline-naming-design.md` (binding). Decisions the code forced go into its new §11, written in Task 6.

**Branch:** `feat/inline-naming` (stacked on `fix/sidebar-rough-edges`). One PR.

## Global Constraints

- **Latency:** nothing new runs before first paint or first input. The field, its brush and its accessible annotation are made on the first edit (`ensure_field`, `InlineName::brush`), never at sidebar creation. Live checks read only the rows in memory (`sibling_names` over `NotebookView::rows`, once per rebuild; a `HashSet` lookup per keystroke). The only note-file I/O on the UI thread is the single user-started call per commit (`OpenOptions::create_new`, `std::fs::create_dir`, `platform::files::rename_no_replace` plus its undo on failure), with what already follows today's first save and rename in the same commit (`LibraryState::add_note`'s stamp, `rebind_open_tab`'s stamp, `open_note` reading the new empty file, and `name_taken_error`'s free-name probe only on a clash).
- **App-borrow rule:** while a `&mut` from `notebook_view::with_view`, `library_host::with_state`, `host`/`with_host` or `app_ptr(...).as_mut()` is held, never call `SetFocus`, `SetCapture`, `CreateWindowExW`, `UpdateWindow`, `SetWindowTextW` on a child, `GetWindowTextW`, `SendMessageW` to another window, `modal::*`, or a document swap. `MoveWindow`/`ShowWindow` of the field and `annotate` also run with nothing borrowed. Every snippet below takes what it needs inside the borrow and acts after it (`place`, `changed`, `start`, `announce`).
- **Tests never touch the real profile** (`%LOCALAPPDATA%\FastPad`) or the real Documents: window tests use `LibraryScratch` in `main_window.rs`'s tests with `ProductionWindow::new(make_app())`; `library.data_dir` is set to `scratch.data()` only in tests that rescan, as today.
- **Test runs:** window tests need `-- --test-threads=1`. Each task runs `cargo fmt` (the snippets are hand-formatted; let rustfmt have the last word), then `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and only the tests it names. The full suite runs once, in Task 6. In a worktree, copy the `native/out` DLLs first.
- **Dead code while the module grows:** `src/window/inline_name.rs` starts with `#![cfg_attr(not(test), expect(dead_code, reason = "wired up task by task by the inline naming plan"))]`. Task 4, the last task that wires an item up, removes it (the expectation becomes unfulfilled there). A `#[cfg(test)]` helper that clippy reports dead in the source-linked `save_file` target gets `#[allow(dead_code, reason = "read by the lib window tests, not by the source-linked integration targets")]`, as `fail_next_folder_rename_back` has.
- **No backward compatibility:** `NamePurpose::NewFolder` and `NamePurpose::RenameFolder` go, with `library_host::{new_folder, submit_new_folder, rename_folder, submit_rename_folder, rename_file, folder_suffix, NEW_FOLDER, NO_FOLDER_NAME}`; every caller is updated. No shims.
- **Commits:** conventional messages (`feat(sidebar): …`, `refactor(library): …`, `docs(spec): …`), no attribution lines.
- **Searching:** never search the whole disk (no `find /`, no recursive listing of `C:\` or `D:\`). Crate sources are in `C:\Users\korn3\.cargo\registry\src\`.
- **Style:** match the surrounding code, its comment density and test naming; spec references read "inline naming spec §x" in doc comments; every new test starts with a `// Break caught:` comment.
- **Table sizes (verified against the current code):** `CommandId::NoteNewFolder = 192` is the highest today; `CommandId::NoteNew = 193`, not `needs_document`, not `is_sidebar`. `COMMANDS` 84 → 85, `command_palette::ENTRIES` 71 → 72. Accelerators unchanged. `WM_FASTPAD_INLINE_NAME_LEFT = WM_APP + 17` (1–16 and 0x31, 0x40–0x61 are taken).
- **Wording (verbatim; `…` is U+2026, quotes U+201C/U+201D as today):**
  - palette row `Notebook: New note…`, placed right before `Notebook: New folder…`; folder menu `New note here` (unchanged label); header tooltip and accessible name `New note` (unchanged);
  - accessible names `New note name, in <folder>`, `New folder name, in <folder>` (`<notebook name>` at the root), `Rename <file name>`;
  - live check: `<name> already exists here.`; hidden folder: today's `FastPad hides folders named “<name>”. Choose another name.`;
  - disk clash for a note: today's `<name> already exists. Try <free name>.` (`name_taken_error`); for a folder: today's `A folder or file named “<name>” already exists`;
  - failures: `FastPad could not create <name>: <error>`, `FastPad could not create the folder: <error>`, `FastPad could not rename the file: <error>`, `FastPad could not rename the folder: <error>`, `Another tab already has that file open.`, `FastPad could not show the name field: <error>`.
- **Sizes (96 DPI, scaled with `panel::scale`):** the frame starts 3 px before the row's name and ends at the pin box; it is inset 2 px from the row's top and bottom; the text starts 3 px inside the frame; the problem box has 4 px padding. The frame is `selection_background`, or `error_foreground` while a problem shows; the field and the problem box fill with `editor_background`; the problem text is `error_foreground`.

## Review Focus

1. **A click landing on a row the commit moves:** with a draft row open at the root above `a.md` and `b.md`, a click on `b.md` cancels the empty draft (the rows shift up) and must still select and preview `b.md`, not the row that slid under the pointer; a click on the row being renamed commits and does nothing more. Pinned in Task 5 (`a_click_on_a_row_below_a_draft_commits_first_and_acts_on_that_row`).
2. **Names that clean to nothing, don't change, or change only case:** new note `...`, `.json`, spaces only (cancel, no `Untitled.md`); folder `...` (cancel, no "Type a folder name"); rename `a.md` → `a.md` (cancel, no message); `plan.md` → `Plan.md` and folder `sub` → `Sub` (renamed, not a clash with itself). Pinned in Task 1 (`a_new_note_name_defaults_to_markdown_keeps_a_note_extension_and_is_none_when_empty`, `typed_names_cancel_when_empty_or_unchanged_and_a_case_change_is_a_rename`), Task 2 (`a_typed_folder_name_is_sanitized_hidden_names_are_refused_and_an_empty_one_cancels`) and Task 4 (`a_case_only_rename_in_the_tree_renames_the_note_and_the_folder`).
3. **The tree changing under the field:** a rescan keeps the typed text and the draft row; a rescan that now lists the typed name raises the problem without a keystroke; a rescan that removes the draft's folder, a folder delete, or closing the notebook cancels. Pinned in Task 2 (`a_new_folder_draft_survives_a_rescan_but_goes_with_its_folder_or_notebook`, `deleting_a_folder_that_holds_the_only_tab_and_the_draft_target_warns_and_cancels_the_draft`) and Task 3 (`a_rescan_that_lists_the_typed_name_shows_the_problem_without_a_keystroke`).
4. **The disk disagreeing with memory at Enter:** a file the tree doesn't list takes the name (`fresh.md` written after the scan), and the draft's folder deleted in Explorer before Enter. The message shows under the field and the field stays; nothing is created elsewhere. Pinned in Task 3 (`a_new_note_clashing_with_an_unlisted_file_or_a_vanished_folder_says_so_after_enter`).
5. **Focus leaving FastPad versus moving inside it:** `SetFocus(NULL)` (what deactivation does to the field) commits nothing and the frame's next `WM_SETFOCUS` gives the field its focus back; focus moving to the editor commits, and with a taken name closes the field with a notice; a modal prompt holds the commit until it ends. Pinned in Task 5 (`losing_focus_to_another_app_keeps_the_field_and_reactivation_refocuses_it`, `focus_moving_to_the_editor_commits_and_a_taken_name_closes_with_a_notice`, `a_commit_on_focus_loss_waits_for_a_modal_prompt_to_end`).

## File Map

| File | Responsibility | Task |
|---|---|---|
| `src/library/title.rs` | `new_note_name`, `renamed_note_name`, `rename_parts` (shared with `split_rename`) | 1 |
| `src/library/tree.rs` | `RowKind::Draft`, `row_index` arm | 1 |
| `src/window/inline_name.rs` (new), `src/window/mod.rs` | pure half (Task 1); `InlineName`, field, hook, `start`, `place`, `commit`, New folder (Task 2); New note (Task 3); renames, reveal (Task 4); focus-leave, refocus (Task 5) | 1–5 |
| `src/window/notebook_view.rs` | `Draft` arms (Task 1); `inline` field, `apply` fit, paint, MSAA mapping, `reveal_edit`, `with_view` pub, place calls (Task 2); header/menu New note (Task 3); F2/menu rename (Task 4); commit-before-click (Task 5) | 1–5 |
| `src/platform/annotation.rs` (new), `src/platform/mod.rs` | `annotate(hwnd, name, description)` via `IAccPropServices::SetHwndPropStr` | 2 |
| `src/window/side_panel.rs` | `EN_CHANGE` and `WM_CTLCOLOREDIT` routing, `place` after layout (Task 2); cancel on view switch and notes mode off (Task 5) | 2, 5 |
| `src/window/library_host.rs` | helper visibility, `save_local_soon`; folder submit code moves out (Tasks 2, 4) | 1, 2, 4 |
| `src/window/name_box.rs` | `NamePurpose` loses `NewFolder` (2) and `RenameFolder` (4) | 2, 4 |
| `src/window/commands.rs`, `src/window/command_palette.rs` | `CommandId::NoteNew`, palette row | 3 |
| `src/window/main_window.rs` | dispatch (Tasks 2–4), palette filter (3), `WM_SETFOCUS` refocus, the posted message, Ctrl+Z kept by the field (5); window tests (all) | 2–5 |
| `src/window/messages.rs` | `WM_FASTPAD_INLINE_NAME_LEFT` | 5 |
| `README.md`, the spec (§11) | naming in the tree; implementation decisions | 6 |

---

### Task 1: Pure name logic

**Files:**
- Modify: `src/library/title.rs` (after `split_typed_name`; `split_rename`; tests)
- Modify: `src/library/tree.rs` (`RowKind`, `row_index`, test `outline`, one new test)
- Create: `src/window/inline_name.rs`
- Modify: `src/window/mod.rs` (module list)
- Modify: `src/window/notebook_view.rs` (three `RowKind::Draft` arms)
- Modify: `src/window/library_host.rs` (`hidden_folder_error` becomes `pub(crate)`)

**Interfaces:**
- Consumes: `title::{clean_stem (private), file_name, folder_name, is_note_extension, sanitize_stem}`, `library::scan::skip_directory(&str) -> bool`, `library_host::hidden_folder_error(&str) -> String`, `notebook_view::row_parts(RECT, u16, u32) -> RowParts`, `file_icons::{file_icon, FOLDER_ICON, FileIcon}`, `panel::scale`.
- Produces:
  - `pub fn title::new_note_name(input: &str) -> Option<String>`
  - `pub fn title::renamed_note_name(input: &str, current: Option<&str>) -> Option<String>`
  - `RowKind::Draft` (unit variant; `row_index` matches it)
  - `pub(crate) enum inline_name::Purpose { NewNote(PathBuf), NewFolder(PathBuf), RenameNote(PathBuf), RenameFolder(PathBuf) }` (all paths relative to the notebook; a new item's path is its folder, empty for the root) with `parent(&self) -> &Path`, `draft_parent(&self) -> Option<&Path>`, `own_row(&self) -> Option<RowKind>`, `is_folder(&self) -> bool`, `current_name(&self) -> Option<String>`
  - `pub(crate) fn typed_name(&Purpose, &str) -> Option<String>`
  - `pub(crate) fn check(&Purpose, &str, &HashSet<String>) -> Option<String>`
  - `pub(crate) fn taken_message(&str) -> String`
  - `pub(crate) fn rename_selection(&str, folder: bool) -> (usize, usize)` (UTF-16 units)
  - `pub(crate) fn draft_icon(&Purpose, &str) -> FileIcon`
  - `pub(crate) fn accessible_name(&Purpose, notebook: &str) -> String`
  - `pub(crate) fn parent_row(&[TreeRow], &Path) -> Option<Option<usize>>`
  - `pub(crate) fn sibling_names(&[TreeRow], parent: Option<usize>, own: Option<usize>) -> HashSet<String>`
  - `pub(crate) fn insert_draft(&mut Vec<TreeRow>, parent: Option<usize>) -> Option<usize>`
  - `pub(crate) fn word_start(&[u16], caret: usize) -> usize`
  - `#[derive(Clone, Copy)] pub(crate) struct FieldLayout { pub(crate) frame: RECT, pub(crate) edit: RECT }`
  - `pub(crate) fn field_layout(row: RECT, list: RECT, depth: u16, dpi: u32, text_height: i32) -> Option<FieldLayout>`
  - `pub(crate) fn message_rect(frame: RECT, list: RECT, height: i32) -> RECT`

- [ ] **Step 1: Write the failing title tests**

Add to `mod tests` in `src/library/title.rs`:

```rust
    #[test]
    fn a_new_note_name_defaults_to_markdown_keeps_a_note_extension_and_is_none_when_empty() {
        // Break caught: `todo` saved without an extension, `data.json` turned into Markdown,
        // `v1.2 plan` cut at its dot, a device name Windows refuses, or an empty field creating
        // "Untitled.md" (inline naming spec §4.1).
        assert_eq!(new_note_name("todo").as_deref(), Some("todo.md"));
        assert_eq!(new_note_name("data.json").as_deref(), Some("data.json"));
        assert_eq!(new_note_name("v1.2 plan").as_deref(), Some("v1.2 plan.md"));
        assert_eq!(new_note_name("CON").as_deref(), Some("CON_.md"));
        assert_eq!(new_note_name(" a/b: c? ").as_deref(), Some("ab c.md"));
        for empty in ["", "   ", "...", ".json", "??"] {
            assert_eq!(new_note_name(empty), None, "{empty:?}");
        }
    }

    #[test]
    fn a_renamed_note_name_keeps_its_kind_and_is_none_when_the_stem_cleans_to_nothing() {
        // Break caught: a rename to an empty name renaming the note "Untitled.md", or the
        // inline rename splitting differently from the name bar's `split_rename`.
        assert_eq!(
            renamed_note_name("plan.txt", Some("md")).as_deref(),
            Some("plan.txt")
        );
        assert_eq!(
            renamed_note_name("script.py", Some("py")).as_deref(),
            Some("script.py")
        );
        assert_eq!(renamed_note_name("Plan", Some("md")).as_deref(), Some("Plan.md"));
        assert_eq!(renamed_note_name("tool", None).as_deref(), Some("tool"));
        for empty in ["", "  ", ".md", "..."] {
            assert_eq!(renamed_note_name(empty, Some("md")), None, "{empty:?}");
        }
        assert_eq!(
            split_rename("plan.txt", Some("md")),
            ("plan".to_owned(), Some("txt".to_owned()))
        );
    }
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test --lib title::tests`
Expected: FAIL to compile, "cannot find function `new_note_name`".

- [ ] **Step 3: Implement the title functions**

In `src/library/title.rs`, after `split_typed_name`, add `new_note_name`; replace `split_rename` with the `rename_parts` pair and add `renamed_note_name`:

```rust
/// A new note's file name from what was typed in the Notebook tree (inline naming spec §4.1):
/// `split_typed_name(input, "md")`, so `todo` is `todo.md` and `data.json` stays JSON, but
/// `None` when the stem cleans to nothing, where `split_typed_name` would say "Untitled".
pub fn new_note_name(input: &str) -> Option<String> {
    let input = input.trim();
    let (stem, extension) = match input.rsplit_once('.') {
        Some((stem, extension)) if is_note_extension(extension) => (stem, extension),
        _ => (input, "md"),
    };
    Some(file_name(&clean_stem(stem)?, extension))
}

/// The stem and extension `split_rename` takes from `input`, before the stem is cleaned.
fn rename_parts<'a>(input: &'a str, current: Option<&'a str>) -> (&'a str, Option<&'a str>) {
    let input = input.trim();
    if let Some(current) = current.filter(|current| !current.is_empty())
        && let Some((stem, extension)) = input.rsplit_once('.')
        && extension.eq_ignore_ascii_case(current)
    {
        return (stem, Some(extension));
    }
    if let Some((stem, extension)) = input.rsplit_once('.')
        && is_note_extension(extension)
    {
        return (stem, Some(extension));
    }
    (input, current)
}

/// Splits a name typed to rename a file whose extension is `current` (`None`: it has none).
/// The file keeps its kind unless the user types another one:
/// - a name ending in `.<current>` (any case) splits there, so `script.py` stays `script.py`;
/// - otherwise a typed note extension is taken, so `plan.txt` renames `plan.md` to a `.txt`;
/// - otherwise the current extension is kept, and an extensionless file stays extensionless.
pub fn split_rename(input: &str, current: Option<&str>) -> (String, Option<String>) {
    let (stem, extension) = rename_parts(input, current);
    (sanitize_stem(stem), extension.map(str::to_owned))
}

/// A renamed note's file name, split as `split_rename` splits it (inline naming spec §4.3), or
/// `None` when the stem cleans to nothing, so the rename cancels.
pub fn renamed_note_name(input: &str, current: Option<&str>) -> Option<String> {
    let (stem, extension) = rename_parts(input, current);
    Some(file_name(&clean_stem(stem)?, extension.unwrap_or("")))
}
```

- [ ] **Step 4: Run the title tests**

Run: `cargo test --lib title::tests`
Expected: PASS (all of `title::tests`, the old ones included).

- [ ] **Step 5: Add `RowKind::Draft`**

In `src/library/tree.rs`:

```rust
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum RowKind {
    Unsaved(u64),
    Folder(PathBuf),
    Note(PathBuf),
    /// The Notebook view's row for a note or folder being named in the tree (inline naming spec
    /// §3.1). The view puts it in; `rows` never builds one.
    Draft,
}
```

In `row_index`, add the arm before `_ => false`:

```rust
        (RowKind::Draft, RowKind::Draft) => true,
```

In the test helper `outline`, add `(RowKind::Draft, _) => "+",` after the `Unsaved` arm, and add this test to `tree`'s tests:

```rust
    #[test]
    fn a_draft_row_is_found_by_its_kind() {
        // Break caught: the view losing its draft row after a rebuild because `row_index`
        // never matches it.
        let rows = vec![TreeRow {
            kind: RowKind::Draft,
            depth: 0,
            name: String::new(),
            pinned: false,
            expanded: false,
        }];
        assert_eq!(row_index(&rows, &RowKind::Draft), Some(0));
        assert_eq!(row_index(&rows, &RowKind::Unsaved(0)), None);
    }
```

In `src/window/notebook_view.rs` add the arms the compiler asks for:
- `draw_tree_row`'s `match &row.kind`: `RowKind::Draft => {}` (Task 2 draws its icon);
- `open_context_menu`'s `match &row.kind`: `RowKind::Draft => {}`;
- `activate`'s `match row.kind`: `RowKind::Draft => {}`.

In `src/window/library_host.rs` make `hidden_folder_error` `pub(crate)`.

- [ ] **Step 6: Write the pure `inline_name` module with its failing tests**

Add `pub(crate) mod inline_name;` to `src/window/mod.rs` (after `pub(crate) mod file_icons;`, alphabetical). Create `src/window/inline_name.rs`:

```rust
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
            let current = path.extension().map(|extension| extension.to_string_lossy());
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
        assert_eq!(draft_icon(&Purpose::NewFolder(PathBuf::new()), "x.json"), FOLDER_ICON);
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
        assert_eq!(sibling_names(&rows, Some(1), Some(2)), names(&["deep", "b.md"]));
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
        assert!(field_layout(row_at(12), list, 1, 96, 16).is_none(), "under the header");
        assert!(field_layout(row_at(400), list, 1, 96, 16).is_none(), "below the list");
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
```

- [ ] **Step 7: Run the new tests**

Run: `cargo test --lib inline_name::tests tree::tests`
Expected: PASS. (If `the_field_covers_the_name_up_to_the_pin_and_stays_inside_the_list` fails on `layout.frame.left > parts.icon.right - 1`, the frame outset must stay at 3 px while `GAP` is 6 px: fix `FRAME_OUTSET`, not the test.)

- [ ] **Step 8: Check and commit**

Run: `cargo fmt`, `cargo fmt --check`, then `cargo clippy --all-targets -- -D warnings`
Expected: clean.

```bash
git add src/library/title.rs src/library/tree.rs src/window/inline_name.rs src/window/mod.rs src/window/notebook_view.rs src/window/library_host.rs
git commit -m "feat(sidebar): the pure name logic for naming in the Notebook tree"
```

---

### Task 2: The inline field, the draft row and New folder

**Files:**
- Modify: `src/window/inline_name.rs` (imports; everything below the pure half)
- Create: `src/platform/annotation.rs`; Modify: `src/platform/mod.rs`
- Modify: `src/window/notebook_view.rs` (`NotebookView`, `new`, `apply`, `paint`, `draw_tree_row`, AccessibleView impl, `with_view`, `rebuild`, `active_tab_changed`, `select_row`, `select_index`, `handle` wheel, `mouse_move`, header/menu New folder, a test accessor)
- Modify: `src/window/side_panel.rs` (`panel_proc` `EN_CHANGE` and `WM_CTLCOLOREDIT`; `layout`)
- Modify: `src/window/library_host.rs` (remove `NEW_FOLDER`, `NO_FOLDER_NAME`, `new_folder`, `submit_new_folder`; visibility; `save_local_soon`; `name_box_submit`)
- Modify: `src/window/name_box.rs` (drop `NamePurpose::NewFolder`)
- Modify: `src/window/main_window.rs` (`NoteNewFolder` dispatch; test helpers; migrated tests)

**Interfaces:**
- Consumes (Task 1): everything in `inline_name`'s pure half; `RowKind::Draft`.
- Consumes (existing): `library_host::{ready_library, folder, notebook_name, set_expanded, close_name_box, with_state, relative_folder, folder_taken_error, hidden_folder_error}`, `main_window::{close_find_bar, push_notice, ui_fonts}`, `side_panel::{windows, current_view, show_view, refresh, with_accessible_events}`, `sidebar_accessibility::raise`, `panel::{create_child, text_height}`.
- Produces:
  - `pub fn platform::annotation::annotate(hwnd: HWND, name: &str, description: &str) -> crate::Result<()>`
  - `pub(crate) struct inline_name::InlineName` with `new()`, `end(&mut self) -> bool`, `fit(&mut self, &mut Vec<TreeRow>) -> bool`, `wants_draft(&self) -> bool`, `row(&self) -> Option<usize>`, `draft_at(&self) -> Option<usize>`, `draft_icon(&self) -> FileIcon`, `problem(&self) -> Option<&str>`, `text_height(&self) -> i32`, `set_colors(&mut self, Palette)`
  - `#[derive(Clone, Copy, Debug, Eq, PartialEq)] pub(crate) enum inline_name::How { Enter, FocusLeft }`
  - `pub(crate) fn inline_name::{new_folder(HWND, Option<PathBuf>), commit(HWND, How), cancel(HWND), is_open(HWND) -> bool, owns(HWND, HWND) -> bool, changed(HWND), control_color(HWND, HDC) -> HBRUSH, place(HWND)}`
  - private `start(HWND, Purpose)`, `end(HWND) -> bool`, `fail(HWND, How, String)`, `cancelled(HWND, How)`, `commit_new_folder(HWND, How, &Path, &str)`
  - `#[cfg(test)] pub(crate) fn inline_name::{field_hwnd(HWND) -> Option<HWND>, purpose(HWND) -> Option<Purpose>, problem(HWND) -> Option<String>}`
  - `pub(crate) NotebookView::inline: InlineName`; `pub(crate) fn NotebookView::{inline_layout(&self) -> Option<FieldLayout>, reveal_edit(&mut self)}`; `#[cfg(test)] pub(crate) fn NotebookView::row_rect_at(&self, usize) -> Option<RECT>`; `pub(crate) fn notebook_view::with_view`
  - `pub(crate) fn library_host::save_local_soon(HWND)`; `pub(crate)` on `library_host::{relative_folder, folder_taken_error}`
  - test helpers in `main_window.rs` tests: `notebook_window`, `inline_field`, `inline_open`, `type_into_field`, `field_key`, `field_text`, `field_selection`, `draft_row`, `row_lparam`

- [ ] **Step 1: Add the window test helpers and the failing tests**

In `src/window/main_window.rs`'s tests, right after `fn select_row` (≈ line 11697), add:

```rust
    /// A window with a sidebar showing `scratch`'s notebook in the Notebook view.
    fn notebook_window(scratch: &LibraryScratch) -> (ProductionWindow, crate::editor::Editor) {
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        ensure_sidebar(window.hwnd);
        scratch.install(window.hwnd);
        crate::window::side_panel::show_view(
            window.hwnd,
            crate::config::SidebarView::Notebook,
            false,
        );
        crate::window::notebook_view::rebuild(window.hwnd);
        (window, editor)
    }

    fn inline_field(hwnd: HWND) -> HWND {
        crate::window::inline_name::field_hwnd(hwnd).expect("the name field was made")
    }

    fn inline_open(hwnd: HWND) -> bool {
        crate::window::inline_name::is_open(hwnd)
    }

    /// Types `text` into the name field as a paste would: the Edit sends EN_CHANGE to the panel.
    fn type_into_field(hwnd: HWND, text: &str) {
        let wide = crate::platform::wide_null(text);
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::SetWindowTextW(
                inline_field(hwnd),
                wide.as_ptr(),
            )
        };
    }

    fn field_key(hwnd: HWND, key: u16) {
        unsafe {
            SendMessageW(
                inline_field(hwnd),
                windows_sys::Win32::UI::WindowsAndMessaging::WM_KEYDOWN,
                usize::from(key),
                0,
            )
        };
    }

    fn field_text(hwnd: HWND) -> String {
        let mut buffer = [0u16; 260];
        let copied = unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::GetWindowTextW(
                inline_field(hwnd),
                buffer.as_mut_ptr(),
                buffer.len() as i32,
            )
        };
        String::from_utf16_lossy(&buffer[..copied.max(0) as usize])
    }

    fn field_selection(hwnd: HWND) -> (u32, u32) {
        let (mut start, mut end) = (0_u32, 0_u32);
        unsafe {
            SendMessageW(
                inline_field(hwnd),
                windows_sys::Win32::UI::Controls::EM_GETSEL,
                &mut start as *mut u32 as usize,
                &mut end as *mut u32 as isize,
            )
        };
        (start, end)
    }

    /// The draft row's index and depth, while one shows.
    fn draft_row(hwnd: HWND) -> Option<(usize, u16)> {
        let rows = &notebook_view(hwnd).rows;
        rows.iter()
            .position(|row| row.kind == RowKind::Draft)
            .map(|index| (index, rows[index].depth))
    }

    /// The middle of the row showing `kind`, as a panel mouse message's `lParam`.
    fn row_lparam(hwnd: HWND, kind: &RowKind) -> super::LPARAM {
        let index = row_of(hwnd, kind);
        let rect = notebook_view(hwnd).row_rect_at(index).unwrap();
        client_lparam((rect.left + rect.right) / 2, (rect.top + rect.bottom) / 2)
    }
```

Replace `new_folder_from_the_header_creates_it_on_disk_and_selects_its_row`, `new_folder_here_creates_it_inside_that_folder_expanded_and_refuses_a_taken_name`, `a_typed_folder_name_is_sanitized_and_empty_or_hidden_names_are_refused` and `a_new_folder_box_survives_a_rescan_but_closes_when_its_parent_goes` with:

```rust
    #[test]
    fn new_folder_from_the_header_names_it_in_an_empty_draft_row_and_selects_the_new_row() {
        // Break caught: the header button opening the name bar, a "New folder" prefill, a
        // folder made before Enter or on Escape, the draft left behind, or the new folder not
        // selected with the focus in the tree (inline naming spec §3.2, §5.2).
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetFocus, VK_ESCAPE, VK_RETURN};
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("inline-new-folder");
        scratch.note("top.md", "t");
        let (window, _editor) = notebook_window(&scratch);
        select_row(window.hwnd, &RowKind::Note("top.md".into()));
        let panel = sidebar_windows(window.hwnd).1;
        let count = crate::window::side_panel::accessible_item_count(panel);
        assert!(
            (0..count)
                .filter_map(|index| crate::window::side_panel::accessible_item(panel, index))
                .any(|item| item.name == "New folder"),
            "the header button has its accessible name"
        );
        let press = || {
            crate::window::notebook_view::header_clicked(
                window.hwnd,
                crate::window::notebook_view::HeaderButton::NewFolder,
            );
        };

        press();
        assert_eq!(draft_row(window.hwnd), Some((0, 0)), "first at the root");
        assert_eq!(
            crate::window::inline_name::purpose(window.hwnd),
            Some(crate::window::inline_name::Purpose::NewFolder(
                std::path::PathBuf::new()
            ))
        );
        assert_eq!(field_text(window.hwnd), "", "the field starts empty");
        assert_eq!(unsafe { GetFocus() }, inline_field(window.hwnd));
        assert!(!app_mut(window.hwnd)
            .name_box
            .as_ref()
            .is_some_and(|name_box| name_box.is_visible()));
        type_into_field(window.hwnd, "Plans");
        field_key(window.hwnd, VK_ESCAPE);
        assert!(!inline_open(window.hwnd));
        assert_eq!(draft_row(window.hwnd), None, "Escape takes the draft row away");
        assert!(!scratch.folder().join("Plans").exists(), "Escape creates nothing");
        assert_eq!(unsafe { GetFocus() }, panel, "Escape returns to the tree");

        press();
        type_into_field(window.hwnd, "Plans");
        assert!(!scratch.folder().join("Plans").exists(), "nothing before Enter");
        field_key(window.hwnd, VK_RETURN);

        assert!(!inline_open(window.hwnd));
        assert!(scratch.folder().join("Plans").is_dir());
        assert_eq!(draft_row(window.hwnd), None);
        assert_eq!(
            selected_kind(window.hwnd),
            Some(RowKind::Folder("Plans".into()))
        );
        assert_eq!(unsafe { GetFocus() }, panel, "the focus stays in the tree");
    }

    #[test]
    fn new_folder_here_drafts_inside_that_folder_and_a_taken_name_keeps_the_field_open() {
        // Break caught: "New folder here" drafting at the root, the folder left collapsed, a
        // name taken by a listed note missed while typing, Enter accepted over the message, or
        // a clash with an unlisted file closing the field (spec §4.4, §5.2).
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_RETURN;
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("inline-new-folder-here");
        std::fs::create_dir_all(scratch.folder().join("sub")).unwrap();
        scratch.note(r"sub\a.md", "a");
        std::fs::write(scratch.folder().join(r"sub\notes.bin"), "x").unwrap();
        let (window, _editor) = notebook_window(&scratch);
        crate::window::library_host::set_expanded(
            window.hwnd,
            std::path::Path::new("sub"),
            false,
        );
        crate::window::notebook_view::rebuild(window.hwnd);
        let index = row_of(window.hwnd, &RowKind::Folder("sub".into()));
        crate::window::menus::answer_next_popup_menu(|_| Some(CommandId::NoteNewFolder));
        crate::window::notebook_view::open_context_menu(window.hwnd, index, None);

        assert_eq!(draft_row(window.hwnd), Some((index + 1, 1)), "its first child");
        assert!(
            crate::window::library_host::expanded(window.hwnd)
                .contains(&std::path::PathBuf::from("sub"))
        );
        type_into_field(window.hwnd, "A.md");
        assert_eq!(
            crate::window::inline_name::problem(window.hwnd).as_deref(),
            Some("A.md already exists here.")
        );
        field_key(window.hwnd, VK_RETURN);
        assert!(inline_open(window.hwnd), "Enter is refused while a problem shows");
        assert!(!scratch.folder().join(r"sub\A.md").is_dir());

        type_into_field(window.hwnd, "notes.bin");
        assert_eq!(crate::window::inline_name::problem(window.hwnd), None, "not listed");
        field_key(window.hwnd, VK_RETURN);
        assert!(inline_open(window.hwnd));
        assert_eq!(
            crate::window::inline_name::problem(window.hwnd).as_deref(),
            Some("A folder or file named \u{201c}notes.bin\u{201d} already exists")
        );

        type_into_field(window.hwnd, "Plans");
        assert_eq!(crate::window::inline_name::problem(window.hwnd), None);
        field_key(window.hwnd, VK_RETURN);
        assert!(!inline_open(window.hwnd));
        assert!(scratch.folder().join(r"sub\Plans").is_dir());
        assert_eq!(
            selected_kind(window.hwnd),
            Some(RowKind::Folder(r"sub\Plans".into()))
        );
    }

    #[test]
    fn a_typed_folder_name_is_sanitized_hidden_names_are_refused_and_an_empty_one_cancels() {
        // Break caught: a name Windows refuses failing with a path error, "..." showing a
        // message instead of cancelling, or a .git or node_modules folder that the next rescan
        // hides (spec §4.2).
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_RETURN;
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("inline-folder-names");
        scratch.note("top.md", "t");
        let (window, _editor) = notebook_window(&scratch);
        let create = |typed: &str| {
            crate::window::inline_name::new_folder(window.hwnd, Some(std::path::PathBuf::new()));
            type_into_field(window.hwnd, typed);
            field_key(window.hwnd, VK_RETURN);
        };

        create(" a/b: c?. ");
        assert!(scratch.folder().join("ab c").is_dir());
        create("CON");
        assert!(scratch.folder().join("CON_").is_dir());
        assert!(!inline_open(window.hwnd));

        create("...");
        assert!(!inline_open(window.hwnd), "nothing left of the name cancels");
        assert_eq!(draft_row(window.hwnd), None);

        for hidden in [".git", "node_modules"] {
            create(hidden);
            assert!(inline_open(window.hwnd), "{hidden}");
            assert_eq!(
                crate::window::inline_name::problem(window.hwnd),
                Some(format!(
                    "FastPad hides folders named \u{201c}{hidden}\u{201d}. Choose another name."
                ))
            );
            assert!(!scratch.folder().join(hidden).exists());
            crate::window::inline_name::cancel(window.hwnd);
        }
    }

    #[test]
    fn a_new_folder_draft_survives_a_rescan_but_goes_with_its_folder_or_notebook() {
        // Break caught: a rescan dropping the draft row and what was typed, a draft left in a
        // folder deleted in Explorer, or one outliving its notebook (spec §5.4).
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("inline-folder-rescan");
        std::fs::create_dir_all(scratch.folder().join("sub")).unwrap();
        scratch.note(r"sub\a.md", "a");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        ensure_sidebar(window.hwnd);
        app_mut(window.hwnd).library.data_dir = Some(scratch.data());
        scratch.install(window.hwnd);
        crate::window::notebook_view::rebuild(window.hwnd);
        crate::window::inline_name::new_folder(window.hwnd, Some("sub".into()));
        type_into_field(window.hwnd, "Typed");

        rescan_and_wait(window.hwnd);
        assert!(inline_open(window.hwnd));
        assert_eq!(field_text(window.hwnd), "Typed");
        let sub = row_of(window.hwnd, &RowKind::Folder("sub".into()));
        assert_eq!(draft_row(window.hwnd), Some((sub + 1, 1)));

        std::fs::remove_dir_all(scratch.folder().join("sub")).unwrap();
        rescan_and_wait(window.hwnd);
        assert!(!inline_open(window.hwnd));
        assert_eq!(draft_row(window.hwnd), None);

        crate::window::inline_name::new_folder(window.hwnd, None);
        assert!(inline_open(window.hwnd));
        crate::window::library_host::close_notebook(window.hwnd);
        assert!(!inline_open(window.hwnd));
    }

    #[test]
    fn the_field_sits_over_its_row_scrolls_with_it_and_is_left_out_of_the_tree_for_screen_readers(
    ) {
        // Break caught: the field drawn away from its row or over the header, left behind
        // when the list scrolls, losing the typing when scrolled out of view, or the draft row
        // read out as an empty tree item (spec §5.4, §6).
        use windows_sys::Win32::Foundation::POINT;
        use windows_sys::Win32::Graphics::Gdi::MapWindowPoints;
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetFocus;
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GWL_STYLE, GetWindowLongPtrW, GetWindowRect, WM_MOUSEWHEEL, WS_VISIBLE,
        };
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("inline-placement");
        for index in 0..80 {
            scratch.note(&format!("n{index:02}.md"), "x");
        }
        let (window, _editor) = notebook_window(&scratch);
        let panel = sidebar_windows(window.hwnd).1;
        let names = || {
            let count = crate::window::side_panel::accessible_item_count(panel);
            (0..count)
                .filter_map(|index| crate::window::side_panel::accessible_item(panel, index))
                .map(|item| item.name)
                .collect::<Vec<_>>()
        };
        let before = names();
        // The test window is never shown, so the field's own style says whether it shows.
        let shown = |field: HWND| {
            (unsafe { GetWindowLongPtrW(field, GWL_STYLE) } as u32) & WS_VISIBLE != 0
        };

        crate::window::inline_name::new_folder(window.hwnd, None);
        let field = inline_field(window.hwnd);
        assert_eq!(names(), before, "the draft row is no MSAA item");
        assert!(shown(field));
        let row = notebook_view(window.hwnd).row_rect_at(0).unwrap();
        let mut rect = RECT::default();
        unsafe {
            GetWindowRect(field, &mut rect);
            MapWindowPoints(
                std::ptr::null_mut(),
                panel,
                &mut rect as *mut RECT as *mut POINT,
                2,
            );
        }
        assert!(
            rect.top >= row.top && rect.bottom <= row.bottom,
            "{}..{} in {}..{}",
            rect.top,
            rect.bottom,
            row.top,
            row.bottom
        );

        let down = ((-(120_i16 * 20)) as u16 as usize) << 16;
        unsafe { SendMessageW(panel, WM_MOUSEWHEEL, down, 0) };
        assert!(notebook_view(window.hwnd).list.top > 0, "the list scrolled");
        assert!(!shown(field), "out of view, hidden");
        assert_eq!(unsafe { GetFocus() }, field, "and still editing");
        type_into_field(window.hwnd, "Kept");
        let up = ((120_i16 * 20) as u16 as usize) << 16;
        unsafe { SendMessageW(panel, WM_MOUSEWHEEL, up, 0) };
        assert!(shown(field), "back in view");
        assert_eq!(field_text(window.hwnd), "Kept");
    }

    #[test]
    fn the_name_field_is_named_for_screen_readers_and_its_problem_is_its_description() {
        // Break caught: a field a screen reader announces as a bare "edit", or a problem it
        // never hears (spec §6).
        use crate::window::accessibility::{
            AccessibleVtable, IID_IACCESSIBLE, RawVariant, VariantValue,
        };
        use crate::window::sidebar_accessibility::take_raised;
        use windows_sys::Win32::Foundation::{SysFreeString, SysStringLen};
        use windows_sys::Win32::UI::Accessibility::AccessibleObjectFromWindow;
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            EVENT_OBJECT_DESCRIPTIONCHANGE, OBJID_CLIENT,
        };
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("inline-accessible");
        std::fs::create_dir_all(scratch.folder().join(r"sub\Taken")).unwrap();
        let (window, _editor) = notebook_window(&scratch);
        crate::window::inline_name::new_folder(window.hwnd, Some("sub".into()));
        let field = inline_field(window.hwnd);
        let read = |description: bool| -> String {
            let com = unsafe {
                windows_sys::Win32::System::Com::CoInitializeEx(
                    std::ptr::null(),
                    windows_sys::Win32::System::Com::COINIT_APARTMENTTHREADED as u32,
                )
            };
            let mut object = std::ptr::null_mut();
            let result = unsafe {
                AccessibleObjectFromWindow(field, OBJID_CLIENT as u32, &IID_IACCESSIBLE, &mut object)
            };
            assert!(result >= 0 && !object.is_null(), "{result:#x}");
            let vtable = unsafe { &**(object as *const *const AccessibleVtable) };
            let get = if description {
                vtable.get_acc_description
            } else {
                vtable.get_acc_name
            };
            let mut text = std::ptr::null_mut();
            unsafe { get(object, RawVariant::integer(0), &mut text) };
            let value = if text.is_null() {
                String::new()
            } else {
                let units = unsafe { std::slice::from_raw_parts(text, SysStringLen(text) as usize) };
                let value = String::from_utf16_lossy(units);
                unsafe { SysFreeString(text) };
                value
            };
            unsafe { (vtable.release)(object) };
            if com >= 0 {
                unsafe { windows_sys::Win32::System::Com::CoUninitialize() };
            }
            value
        };
        assert_eq!(read(false), "New folder name, in sub");

        take_raised();
        type_into_field(window.hwnd, "taken");
        assert!(
            take_raised().contains(&(field as usize, EVENT_OBJECT_DESCRIPTIONCHANGE, 0)),
            "the problem is announced"
        );
        assert_eq!(read(true), "taken already exists here.");
    }

    #[test]
    fn ctrl_a_selects_the_name_and_ctrl_backspace_deletes_a_word_in_the_field() {
        // Break caught: Ctrl+A doing nothing in the field, or Ctrl+Backspace typing a box
        // character instead of deleting the word before the caret (spec §5.1).
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
            GetKeyboardState, SetKeyboardState, VK_BACK, VK_CONTROL,
        };
        use windows_sys::Win32::UI::WindowsAndMessaging::WM_CHAR;
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("inline-keys");
        scratch.note("top.md", "t");
        let (window, _editor) = notebook_window(&scratch);
        crate::window::inline_name::new_folder(window.hwnd, None);
        type_into_field(window.hwnd, "my note.md");
        let field = inline_field(window.hwnd);
        unsafe { SendMessageW(field, windows_sys::Win32::UI::Controls::EM_SETSEL, 10, 10) };
        let mut keys = [0u8; 256];
        unsafe { GetKeyboardState(keys.as_mut_ptr()) };
        let original = keys;
        keys[VK_CONTROL as usize] = 0x80;
        unsafe { SetKeyboardState(keys.as_ptr()) };

        field_key(window.hwnd, VK_BACK);
        unsafe { SendMessageW(field, WM_CHAR, 0x7f, 0) };
        let after_backspace = field_text(window.hwnd);
        field_key(window.hwnd, u16::from(b'A'));
        unsafe { SendMessageW(field, WM_CHAR, 0x01, 0) };
        let selection = field_selection(window.hwnd);
        unsafe { SetKeyboardState(original.as_ptr()) };

        assert_eq!(after_backspace, "my note.");
        assert_eq!(selection, (0, 8));
        assert_eq!(field_text(window.hwnd), "my note.", "no control characters typed");
    }
```

Replace `deleting_a_folder_that_holds_the_only_tab_and_the_name_box_target_warns_and_closes_both` with:

```rust
    #[test]
    fn deleting_a_folder_that_holds_the_only_tab_and_the_draft_target_warns_and_cancels_the_draft(
    ) {
        // Break caught: unsaved edits discarded without a word, the last tab left on a deleted
        // file, or a draft still offering to create inside a folder that is gone (spec §5.4).
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_DELETE;
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("folder-delete-only-tab");
        std::fs::create_dir_all(scratch.folder().join("sub")).unwrap();
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        ensure_sidebar(window.hwnd);
        let a = open_note(&window, &scratch, r"sub\a.md", "a");
        execute_command(window.hwnd, CommandId::ToggleFolderAutosave);
        editor.set_text("unsaved").unwrap();
        crate::window::notebook_view::rebuild(window.hwnd);
        crate::window::inline_name::new_folder(window.hwnd, Some("sub".into()));
        assert!(inline_open(window.hwnd));
        select_row(window.hwnd, &RowKind::Folder("sub".into()));
        crate::window::modal::take_last_confirm();
        crate::window::answer_next_confirm(|_| true);

        assert!(crate::window::notebook_view::key_down(
            window.hwnd,
            VK_DELETE
        ));

        assert_eq!(
            crate::window::modal::take_last_confirm().as_deref(),
            Some(
                "Move \u{201c}sub\u{201d} and its 1 note to the Recycle Bin?\n1 open note has unsaved changes, which will be lost."
            )
        );
        assert!(!a.exists());
        assert_eq!(super::tab_count(window.hwnd), 0);
        assert!(!inline_open(window.hwnd));
        assert_eq!(draft_row(window.hwnd), None);
    }
```

In `folder_commands_on_an_empty_or_escaping_path_touch_no_disk`, delete the line `submit(NamePurpose::NewFolder("..".into()), "Outside");` and add before the asserts:

```rust
        crate::window::inline_name::new_folder(window.hwnd, Some("..".into()));
        assert!(!inline_open(window.hwnd), "no draft outside the notebook");
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test --lib -- --test-threads=1 new_folder_from_the_header_names_it new_folder_here_drafts a_typed_folder_name_is_sanitized_hidden a_new_folder_draft_survives the_field_sits_over_its_row the_name_field_is_named ctrl_a_selects_the_name deleting_a_folder_that_holds_the_only_tab_and_the_draft folder_commands_on_an_empty`
Expected: FAIL to compile, "cannot find function `new_folder` in module `inline_name`".

- [ ] **Step 3: Write `platform::annotation`**

Create `src/platform/annotation.rs` and add `pub mod annotation;` to `src/platform/mod.rs` (first in the list):

```rust
//! Dynamic annotation through `IAccPropServices` (oleacc): a native control's accessible name
//! and description, kept for its window by the system's own proxy, so screen readers hear them
//! from the control itself (inline naming spec §6).

use crate::{FastPadError, Result};
use std::ffi::c_void;
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    CoUninitialize,
};
use windows_sys::Win32::UI::Accessibility::{
    CAccPropServices, PROPID_ACC_DESCRIPTION, PROPID_ACC_NAME,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{CHILDID_SELF, OBJID_CLIENT};
use windows_sys::core::{GUID, HRESULT};

const IID_IACC_PROP_SERVICES: GUID = GUID::from_u128(0x6e26e776_04f0_495d_80e4_3330352e3169);
/// COM is already initialized on this thread in the other apartment model: usable, not ours
/// to uninitialize.
const RPC_E_CHANGED_MODE: HRESULT = 0x8001_0106_u32 as HRESULT;

// The SDK's IUnknown -> IAccPropServices order, up to the one method used.
#[repr(C)]
struct AccPropServicesVtable {
    query_interface: usize,
    add_ref: usize,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    set_prop_value: usize,
    set_prop_server: usize,
    clear_props: usize,
    set_hwnd_prop: usize,
    set_hwnd_prop_str:
        unsafe extern "system" fn(*mut c_void, HWND, u32, u32, GUID, *const u16) -> HRESULT,
}

fn check(status: HRESULT) -> Result<()> {
    if status < 0 {
        Err(FastPadError::Win32(status as u32))
    } else {
        Ok(())
    }
}

/// Gives `hwnd` (a native control) the accessible `name` and `description`. The annotation
/// stays with the window until it is destroyed; calling again replaces it.
pub fn annotate(hwnd: HWND, name: &str, description: &str) -> Result<()> {
    let initialized =
        unsafe { CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32) };
    if initialized < 0 && initialized != RPC_E_CHANGED_MODE {
        return Err(FastPadError::Win32(initialized as u32));
    }
    let result = set_strings(hwnd, name, description);
    if initialized >= 0 {
        // S_FALSE too: every successful CoInitializeEx is balanced.
        unsafe { CoUninitialize() };
    }
    result
}

fn set_strings(hwnd: HWND, name: &str, description: &str) -> Result<()> {
    let mut services: *mut c_void = std::ptr::null_mut();
    check(unsafe {
        CoCreateInstance(
            &CAccPropServices,
            std::ptr::null_mut(),
            CLSCTX_INPROC_SERVER,
            &IID_IACC_PROP_SERVICES,
            &mut services,
        )
    })?;
    if services.is_null() {
        return Err(FastPadError::Invariant("IAccPropServices was not created"));
    }
    let vtable = unsafe { &**(services as *const *const AccPropServicesVtable) };
    let set = |property: GUID, text: &str| {
        let wide = crate::platform::wide_null(text);
        check(unsafe {
            (vtable.set_hwnd_prop_str)(
                services,
                hwnd,
                OBJID_CLIENT as u32,
                CHILDID_SELF,
                property,
                wide.as_ptr(),
            )
        })
    };
    let result = set(PROPID_ACC_NAME, name).and_then(|()| set(PROPID_ACC_DESCRIPTION, description));
    unsafe { (vtable.release)(services) };
    result
}
```

- [ ] **Step 4: Write the window half of `inline_name`**

Replace the `use` block of `src/window/inline_name.rs` (keep the module doc and the `expect` attribute; change the doc's last sentence to "The pure half comes first; the field, the edit it shows and the commits follow.") with:

```rust
use super::main_window::push_notice;
use super::notebook_view::{self, NotebookView, with_view};
use super::side_panel;
use crate::config::SidebarView;
use crate::library::title;
use crate::library::tree::{self, RowKind, TreeRow};
use crate::platform::{last_error, wide_null};
use crate::window::file_icons::{FOLDER_ICON, FileIcon, file_icon};
use crate::window::library_host::{self, with_state};
use crate::window::palette::Palette;
use crate::window::panel::{create_child, scale};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    CreateSolidBrush, DeleteObject, HBRUSH, HDC, InvalidateRect, SetBkColor, SetTextColor,
};
use windows_sys::Win32::UI::Controls::{EM_GETSEL, EM_REPLACESEL, EM_SETSEL};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetFocus, GetKeyState, SetFocus, VK_BACK, VK_CONTROL, VK_ESCAPE, VK_MENU, VK_RETURN, VK_TAB,
};
use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DestroyWindow, ES_AUTOHSCROLL, EVENT_OBJECT_DESCRIPTIONCHANGE, GetParent,
    GetWindowTextLengthW, GetWindowTextW, MoveWindow, SW_HIDE, SW_SHOWNA, SendMessageW,
    SetWindowTextW, ShowWindow, WM_CHAR, WM_KEYDOWN, WM_NCDESTROY, WM_SETFONT, WS_CHILD,
};
```

Add after `const TEXT_INSET`:

```rust
const FIELD_HOOK_ID: usize = 0x4650_494E;
```

Append below `message_rect` (before `mod tests`):

```rust
/// How an edit is being committed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum How {
    /// Enter in the field: a problem keeps the field open with its message (spec §5.2).
    Enter,
    /// Focus left the field for another FastPad window, a click in the tree, or another edit
    /// starting (spec §3.4, §5.3): a problem closes the field with a notice, and nothing moves
    /// the focus from where it went.
    FocusLeft,
}

/// The edit the field shows.
#[derive(Debug)]
struct Edit {
    purpose: Purpose,
    /// The field's text as `EN_CHANGE` last reported it.
    text: String,
    /// The lowercased names listed beside the edited name (`sibling_names`), from the rows last
    /// built.
    siblings: HashSet<String>,
    /// The message under the field: the live check's, or a failed Enter's.
    problem: Option<String>,
    /// The field's accessible name (spec §6).
    accessible: String,
    /// `problem` changed since screen readers last heard it.
    announce: bool,
    /// Focus left FastPad while the field had it: it goes back when the window is active again
    /// (spec §5.3).
    refocus: bool,
}

impl Edit {
    fn recheck(&mut self) {
        let problem = check(&self.purpose, &self.text, &self.siblings);
        self.show(problem);
    }

    fn show(&mut self, problem: Option<String>) {
        if problem != self.problem {
            self.problem = problem;
            self.announce = true;
        }
    }
}

/// The Notebook view's inline name field and the one edit it shows, owned by `NotebookView`.
#[derive(Debug)]
pub(crate) struct InlineName {
    /// The field, made on the first edit (spec §7), a child of the side panel.
    field: Option<HWND>,
    /// The field could not be made; it is not tried again.
    failed: bool,
    edit: Option<Edit>,
    /// The edited row's index in the view's rows, from the last `fit`.
    row: Option<usize>,
    /// `row` is the draft row.
    draft: bool,
    /// The field font's text height, measured by `place`.
    text_height: i32,
    colors: Palette,
    /// The field's background for `WM_CTLCOLOREDIT`, made on its first use.
    brush: HBRUSH,
}

impl InlineName {
    pub(crate) fn new() -> Self {
        Self {
            field: None,
            failed: false,
            edit: None,
            row: None,
            draft: false,
            text_height: 0,
            colors: Palette::neutral(),
            brush: std::ptr::null_mut(),
        }
    }

    fn begin(&mut self, purpose: Purpose, text: String, accessible: String) {
        self.edit = Some(Edit {
            purpose,
            text,
            siblings: HashSet::new(),
            problem: None,
            accessible,
            announce: false,
            refocus: false,
        });
        self.row = None;
        self.draft = false;
    }

    /// Ends the open edit, if any; `place` hides the field afterwards, with nothing borrowed.
    /// Returns whether there was one.
    pub(crate) fn end(&mut self) -> bool {
        self.row = None;
        self.draft = false;
        self.edit.take().is_some()
    }

    /// A New note or New folder edit is open: its draft row needs the tree.
    pub(crate) fn wants_draft(&self) -> bool {
        self.edit
            .as_ref()
            .is_some_and(|edit| edit.purpose.draft_parent().is_some())
    }

    /// Fits the edit to freshly built `rows` (spec §5.4): the draft row goes back in as the
    /// first child of its folder, or the renamed row is found; the sibling names and the live
    /// check follow the new rows. False, changing nothing, when the folder or the row is gone:
    /// the caller ends the edit. True with no edit open.
    pub(crate) fn fit(&mut self, rows: &mut Vec<TreeRow>) -> bool {
        self.row = None;
        self.draft = false;
        let Some(edit) = self.edit.as_mut() else {
            return true;
        };
        let Some(parent) = parent_row(rows, edit.purpose.parent()) else {
            return false;
        };
        let row = match edit.purpose.own_row() {
            Some(own) => tree::row_index(rows, &own),
            None => insert_draft(rows, parent),
        };
        let Some(row) = row else {
            return false;
        };
        edit.siblings = sibling_names(rows, parent, Some(row));
        edit.recheck();
        self.draft = edit.purpose.draft_parent().is_some();
        self.row = Some(row);
        true
    }

    pub(crate) fn row(&self) -> Option<usize> {
        self.row
    }

    pub(crate) fn draft_at(&self) -> Option<usize> {
        self.row.filter(|_| self.draft)
    }

    pub(crate) fn draft_icon(&self) -> FileIcon {
        self.edit.as_ref().map_or(file_icon(Some("md")), |edit| {
            draft_icon(&edit.purpose, &edit.text)
        })
    }

    pub(crate) fn problem(&self) -> Option<&str> {
        self.edit.as_ref()?.problem.as_deref()
    }

    pub(crate) fn text_height(&self) -> i32 {
        self.text_height
    }

    /// Recolors for a theme change (the panel's paint calls it).
    pub(crate) fn set_colors(&mut self, colors: Palette) {
        if colors == self.colors {
            return;
        }
        self.colors = colors;
        if !self.brush.is_null() {
            unsafe { DeleteObject(self.brush) };
            self.brush = std::ptr::null_mut();
        }
    }

    fn brush(&mut self) -> HBRUSH {
        if self.brush.is_null() {
            self.brush = unsafe { CreateSolidBrush(self.colors.editor_background) };
        }
        self.brush
    }
}

impl Drop for InlineName {
    fn drop(&mut self) {
        if !self.brush.is_null() {
            unsafe { DeleteObject(self.brush) };
        }
    }
}

fn with_inline<R>(hwnd: HWND, f: impl FnOnce(&mut InlineName) -> R) -> Option<R> {
    with_view(hwnd, |view| f(&mut view.inline))
}

fn field_of(hwnd: HWND) -> Option<HWND> {
    with_inline(hwnd, |inline| inline.field).flatten()
}

/// Whether an edit is open.
pub(crate) fn is_open(hwnd: HWND) -> bool {
    with_inline(hwnd, |inline| inline.edit.is_some()).unwrap_or(false)
}

/// Whether `control` is the field.
pub(crate) fn owns(hwnd: HWND, control: HWND) -> bool {
    !control.is_null() && with_inline(hwnd, |inline| inline.field == Some(control)).unwrap_or(false)
}

fn field_text(field: HWND) -> String {
    unsafe {
        let length = GetWindowTextLengthW(field);
        if length <= 0 {
            return String::new();
        }
        let mut buffer = vec![0u16; length as usize + 1];
        let copied = GetWindowTextW(field, buffer.as_mut_ptr(), buffer.len() as i32);
        buffer.truncate(copied.max(0) as usize);
        String::from_utf16_lossy(&buffer)
    }
}

fn set_field_text(field: HWND, text: &str) {
    let wide = wide_null(text);
    unsafe {
        SetWindowTextW(field, wide.as_ptr());
    }
}

/// The field, made now if the view has none yet. A failure is reported once.
fn ensure_field(hwnd: HWND) -> Option<HWND> {
    let (panel, field, failed) =
        with_view(hwnd, |view| (view.panel, view.inline.field, view.inline.failed))?;
    if field.is_some() || failed {
        return field;
    }
    // Made with nothing of the App borrowed: creating the Edit sends messages to the panel.
    match create_field(panel) {
        Ok(field) => {
            if with_inline(hwnd, |inline| inline.field = Some(field)).is_none() {
                unsafe { DestroyWindow(field) };
                return None;
            }
            Some(field)
        }
        Err(error) => {
            with_inline(hwnd, |inline| inline.failed = true);
            push_notice(hwnd, format!("FastPad could not show the name field: {error}"));
            None
        }
    }
}

/// A hidden single-line `Edit` inside `panel`, subclassed by `field_proc`.
fn create_field(panel: HWND) -> crate::Result<HWND> {
    let field = create_child(panel, &wide_null("Edit"), WS_CHILD | ES_AUTOHSCROLL as u32)?;
    if unsafe { SetWindowSubclass(field, Some(field_proc), FIELD_HOOK_ID, 0) } == 0 {
        let error = last_error();
        unsafe {
            DestroyWindow(field);
        }
        return Err(error);
    }
    Ok(field)
}

fn key_down(key: u16) -> bool {
    unsafe { GetKeyState(i32::from(key)) } < 0
}

/// The field's keys (spec §5.1): Enter commits, Esc cancels, Tab does nothing, Ctrl+A selects
/// all and Ctrl+Backspace deletes the word before the caret. Everything else is the Edit's own.
unsafe extern "system" fn field_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    subclass_id: usize,
    _ref_data: usize,
) -> LRESULT {
    if message == WM_NCDESTROY {
        unsafe {
            RemoveWindowSubclass(hwnd, Some(field_proc), subclass_id);
            return DefSubclassProc(hwnd, message, wparam, lparam);
        }
    }
    let main = unsafe { GetParent(GetParent(hwnd)) };
    // A single-line Edit beeps at Enter, Escape and Tab, and types a box for Ctrl+A and
    // Ctrl+Backspace: all are handled on key down.
    if message == WM_CHAR && matches!(wparam as u16, 0x0d | 0x1b | 0x09 | 0x01 | 0x7f) {
        return 0;
    }
    if message == WM_KEYDOWN {
        let ctrl = key_down(VK_CONTROL) && !key_down(VK_MENU);
        match wparam as u16 {
            VK_RETURN => {
                commit(main, How::Enter);
                return 0;
            }
            VK_ESCAPE => {
                cancel(main);
                notebook_view::focus_tree(main);
                return 0;
            }
            VK_TAB => return 0,
            key if ctrl && key == u16::from(b'A') => {
                unsafe { SendMessageW(hwnd, EM_SETSEL, 0, -1) };
                return 0;
            }
            VK_BACK if ctrl => {
                delete_word_before(hwnd);
                return 0;
            }
            _ => {}
        }
    }
    unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
}

/// Ctrl+Backspace: deletes the selection, or back to `word_start`, as one undoable change.
fn delete_word_before(field: HWND) {
    let (mut start, mut end) = (0_u32, 0_u32);
    unsafe {
        SendMessageW(
            field,
            EM_GETSEL,
            &mut start as *mut u32 as WPARAM,
            &mut end as *mut u32 as LPARAM,
        );
    }
    if start == end {
        let text: Vec<u16> = field_text(field).encode_utf16().collect();
        start = word_start(&text, end as usize) as u32;
        unsafe { SendMessageW(field, EM_SETSEL, start as WPARAM, end as LPARAM) };
    }
    let empty = [0u16];
    unsafe { SendMessageW(field, EM_REPLACESEL, 1, empty.as_ptr() as LPARAM) };
}

/// Starts an edit for `purpose` (spec §3): the one already open commits first (§3.4), the find
/// bar and the name bar close, the Notebook view shows, a draft's folder expands, and the field
/// takes the keyboard focus over its row. Nothing touches the disk.
fn start(hwnd: HWND, purpose: Purpose) {
    if !library_host::ready_library(hwnd) || side_panel::windows(hwnd).is_none() {
        return;
    }
    commit(hwnd, How::FocusLeft);
    super::main_window::close_find_bar(hwnd);
    library_host::close_name_box(hwnd);
    if side_panel::current_view(hwnd) != SidebarView::Notebook {
        side_panel::show_view(hwnd, SidebarView::Notebook, false);
    }
    if let Some(parent) = purpose.draft_parent() {
        let mut folders = tree::ancestors(parent);
        if !parent.as_os_str().is_empty() {
            folders.push(parent.to_path_buf());
        }
        for folder in folders {
            library_host::set_expanded(hwnd, &folder, true);
        }
    }
    let Some(field) = ensure_field(hwnd) else {
        return;
    };
    let text = purpose.current_name().unwrap_or_default();
    let (select_from, select_to) = rename_selection(&text, purpose.is_folder());
    let notebook = library_host::folder(hwnd)
        .map(|root| library_host::notebook_name(&root))
        .unwrap_or_default();
    let accessible = accessible_name(&purpose, &notebook);
    with_inline(hwnd, |inline| inline.begin(purpose, text.clone(), accessible.clone()));
    side_panel::with_accessible_events(hwnd, || notebook_view::rebuild(hwnd));
    // The rebuild found no folder or row for it.
    if !is_open(hwnd) {
        return;
    }
    with_view(hwnd, NotebookView::reveal_edit);
    // Filled and focused with nothing of the App borrowed: the Edit sends EN_CHANGE to the
    // panel, and SetFocus sends focus messages.
    set_field_text(field, &text);
    let _ = crate::platform::annotation::annotate(field, &accessible, "");
    place(hwnd);
    unsafe {
        SetFocus(field);
        SendMessageW(field, EM_SETSEL, select_from, select_to as LPARAM);
    }
}

/// The folder a new item goes in, relative to the notebook (spec §3.1): `parent`, else the
/// folder of the selected row, else the root. `None` for a path that is not a plain relative
/// folder, so nothing is expanded or drafted outside the notebook.
fn target_folder(hwnd: HWND, parent: Option<PathBuf>) -> Option<PathBuf> {
    let parent = parent.unwrap_or_else(|| {
        let root = library_host::folder(hwnd);
        notebook_view::selected_folder(hwnd)
            .zip(root)
            .map(|(selected, root)| library_host::relative_folder(&root, &selected))
            .unwrap_or_default()
    });
    (parent.as_os_str().is_empty() || tree::is_plain_relative_folder(&parent)).then_some(parent)
}

/// The header's New folder, "New folder here" (`parent`) and Notebook: New folder… (spec §3.2).
pub(crate) fn new_folder(hwnd: HWND, parent: Option<PathBuf>) {
    if let Some(parent) = target_folder(hwnd, parent) {
        start(hwnd, Purpose::NewFolder(parent));
    }
}

/// `EN_CHANGE` from the field: the live check runs on what is typed now, against the rows in
/// memory (spec §4.4). No disk access.
pub(crate) fn changed(hwnd: HWND) {
    let Some(field) = field_of(hwnd) else {
        return;
    };
    // Read with nothing of the App borrowed.
    let text = field_text(field);
    let panel = with_view(hwnd, |view| {
        let edit = view.inline.edit.as_mut()?;
        edit.text = text;
        edit.recheck();
        Some(view.panel)
    })
    .flatten();
    if let Some(panel) = panel {
        unsafe { InvalidateRect(panel, std::ptr::null(), 0) };
    }
    announce(hwnd);
}

/// Tells screen readers about a problem that appeared, changed or went (spec §6): the field's
/// description, and `EVENT_OBJECT_DESCRIPTIONCHANGE`.
fn announce(hwnd: HWND) {
    let Some((field, name, description)) = with_inline(hwnd, |inline| {
        let field = inline.field?;
        let edit = inline.edit.as_mut()?;
        std::mem::take(&mut edit.announce).then(|| {
            (
                field,
                edit.accessible.clone(),
                edit.problem.clone().unwrap_or_default(),
            )
        })
    })
    .flatten() else {
        return;
    };
    let _ = crate::platform::annotation::annotate(field, &name, &description);
    super::sidebar_accessibility::raise(field, &[(EVENT_OBJECT_DESCRIPTIONCHANGE, 0)]);
}

/// `WM_CTLCOLOREDIT` for the field.
pub(crate) fn control_color(hwnd: HWND, dc: HDC) -> HBRUSH {
    with_inline(hwnd, |inline| {
        unsafe {
            SetTextColor(dc, inline.colors.editor_foreground);
            SetBkColor(dc, inline.colors.editor_background);
        }
        inline.brush()
    })
    .unwrap_or(std::ptr::null_mut())
}

/// Moves the field over its row's name, clipped to the list (spec §5.4), or hides it: when no
/// edit is open, when it has no row, or when the row is out of view. A field scrolled out of
/// view keeps the focus and goes on editing; one whose edit ended hands the focus to the tree.
/// Runs after every rebuild, scroll and layout, with nothing of the App borrowed.
pub(crate) fn place(hwnd: HWND) {
    let Some((field, editing)) = with_inline(hwnd, |inline| {
        inline.field.map(|field| (field, inline.edit.is_some()))
    })
    .flatten() else {
        return;
    };
    let font = super::main_window::ui_fonts(hwnd).text;
    let text_height = crate::window::panel::text_height(field, font);
    let layout = with_view(hwnd, |view| {
        view.inline.text_height = text_height;
        view.inline_layout()
    })
    .flatten();
    unsafe {
        match layout {
            Some(layout) => {
                if !font.is_null() {
                    SendMessageW(field, WM_SETFONT, font as WPARAM, 0);
                }
                let edit = layout.edit;
                MoveWindow(
                    field,
                    edit.left,
                    edit.top,
                    edit.right - edit.left,
                    edit.bottom - edit.top,
                    1,
                );
                ShowWindow(field, SW_SHOWNA);
            }
            None => {
                if !editing && GetFocus() == field {
                    SetFocus(GetParent(field));
                }
                ShowWindow(field, SW_HIDE);
            }
        }
    }
    announce(hwnd);
}

/// Ends the edit with no disk access; returns whether one was open.
fn end(hwnd: HWND) -> bool {
    with_inline(hwnd, InlineName::end).unwrap_or(false)
}

/// Cancels the open edit, if any: the draft row goes and the field hides (spec §5.1, §5.4).
pub(crate) fn cancel(hwnd: HWND) {
    if end(hwnd) {
        side_panel::with_accessible_events(hwnd, || notebook_view::rebuild(hwnd));
    }
}

/// An empty or unchanged name: the edit cancels without a message (spec §5.2). Enter returns
/// the focus to the tree, as Esc does.
fn cancelled(hwnd: HWND, how: How) {
    cancel(hwnd);
    if how == How::Enter {
        notebook_view::focus_tree(hwnd);
    }
}

/// A commit that cannot go ahead (spec §5.2, §5.3): after Enter the message shows under the
/// field, which stays; after focus left, the field closes and the message is a notice.
fn fail(hwnd: HWND, how: How, message: String) {
    match how {
        How::Enter => {
            let panel = with_view(hwnd, |view| {
                if let Some(edit) = view.inline.edit.as_mut() {
                    edit.show(Some(message));
                }
                view.panel
            });
            if let Some(panel) = panel {
                unsafe { InvalidateRect(panel, std::ptr::null(), 0) };
            }
            announce(hwnd);
        }
        How::FocusLeft => {
            cancel(hwnd);
            push_notice(hwnd, message);
        }
    }
}

/// Commits the open edit, if any (spec §5.2): refused while a problem shows, else the one disk
/// call its purpose makes.
pub(crate) fn commit(hwnd: HWND, how: How) {
    let Some((purpose, problem, field)) = with_inline(hwnd, |inline| {
        let edit = inline.edit.as_ref()?;
        Some((edit.purpose.clone(), edit.problem.clone(), inline.field?))
    })
    .flatten() else {
        return;
    };
    if let Some(problem) = problem {
        fail(hwnd, how, problem);
        return;
    }
    // Read with nothing of the App borrowed.
    let text = field_text(field);
    match purpose {
        Purpose::NewFolder(parent) => commit_new_folder(hwnd, how, &parent, &text),
        Purpose::NewNote(_) | Purpose::RenameNote(_) | Purpose::RenameFolder(_) => {
            cancelled(hwnd, how);
        }
    }
}

/// New folder (spec §5.2): the folder is made with the one disk call, listed, and its row
/// selected. After Enter the focus stays in the tree.
fn commit_new_folder(hwnd: HWND, how: How, parent: &Path, text: &str) {
    let Some(root) = library_host::folder(hwnd) else {
        cancel(hwnd);
        return;
    };
    let Some(name) = title::folder_name(text) else {
        cancelled(hwnd, how);
        return;
    };
    let relative = parent.join(&name);
    // Defence in depth: the folder made must be a plain relative one inside the notebook.
    if !tree::is_plain_relative_folder(&relative) {
        cancel(hwnd);
        return;
    }
    if let Err(error) = std::fs::create_dir(root.join(&relative)) {
        // A file, or a folder the tree doesn't list, may already have the name.
        let error = if error.kind() == std::io::ErrorKind::AlreadyExists {
            library_host::folder_taken_error(&name)
        } else {
            format!("FastPad could not create the folder: {error}")
        };
        fail(hwnd, how, error);
        return;
    }
    with_state(hwnd, |state| state.add_folder(&relative));
    for ancestor in tree::ancestors(&relative) {
        library_host::set_expanded(hwnd, &ancestor, true);
    }
    end(hwnd);
    side_panel::with_accessible_events(hwnd, || {
        side_panel::refresh(hwnd);
        notebook_view::select_row(hwnd, &RowKind::Folder(relative.clone()));
    });
    if how == How::Enter {
        notebook_view::focus_tree(hwnd);
    }
}

#[cfg(test)]
pub(crate) fn field_hwnd(hwnd: HWND) -> Option<HWND> {
    field_of(hwnd)
}

#[cfg(test)]
pub(crate) fn purpose(hwnd: HWND) -> Option<Purpose> {
    with_inline(hwnd, |inline| inline.edit.as_ref().map(|edit| edit.purpose.clone())).flatten()
}

#[cfg(test)]
pub(crate) fn problem(hwnd: HWND) -> Option<String> {
    with_inline(hwnd, |inline| inline.problem().map(str::to_owned)).flatten()
}
```

Note: `RECT` stays imported for the pure half; `FOLDER_ICON`, `scale` likewise.

- [ ] **Step 5: Wire the view**

In `src/window/notebook_view.rs`:

1. Imports: add `inset` to `use crate::window::panel::{fill, scale};`, add `DT_CALCRECT, DrawTextW` to the `Gdi` import, and `use crate::window::inline_name::{FieldLayout, InlineName};`.
2. Make `fn with_view` `pub(crate) fn with_view`.
3. `NotebookView` gains, after `order`:

```rust
    /// The inline name field and its edit (inline naming spec §3).
    pub(crate) inline: InlineName,
```

and `new` sets `inline: InlineName::new(),`.

4. At the top of `apply`, change the parameter to `mut snapshot: Snapshot` and insert before `let reset = …`:

```rust
        // A draft row needs the tree, even in a notebook with nothing listed yet (inline naming
        // spec §3.1).
        if snapshot.mode == Mode::Empty && self.inline.wants_draft() {
            snapshot.mode = Mode::Tree;
        }
        // The edit ends with its notebook, when the tree goes, or when its folder or row is no
        // longer listed (spec §5.4); `fit` puts the draft row back in.
        if snapshot.root != self.root
            || snapshot.mode != Mode::Tree
            || !self.inline.fit(&mut snapshot.rows)
        {
            self.inline.end();
        }
```

5. Add to the `impl NotebookView` block that holds `list_area` (after `row_rect`):

```rust
    /// The inline field's layout in `area` (the panel's client rectangle) at `dpi`: over the
    /// edited row's name, clipped to the list. `None` while nothing is edited or the row is out
    /// of view.
    fn inline_layout_in(&self, area: RECT, dpi: u32) -> Option<FieldLayout> {
        if self.mode != Mode::Tree {
            return None;
        }
        let index = self.inline.row()?;
        let list = self.list_area(area, dpi);
        let top = self.list.row_top(index)?;
        let row = RECT {
            left: list.left,
            top: list.top + top,
            right: list.right,
            bottom: list.top + top + self.list.row_height,
        };
        crate::window::inline_name::field_layout(
            row,
            list,
            self.rows.get(index)?.depth,
            dpi,
            self.inline.text_height(),
        )
    }

    /// `inline_layout_in` for the panel as it is now (`inline_name::place`).
    pub(crate) fn inline_layout(&self) -> Option<FieldLayout> {
        self.inline_layout_in(self.client(), self.dpi())
    }

    /// Brings the edited row into view: a draft row scrolled to, a renamed row selected too
    /// (inline naming spec §3.4).
    pub(crate) fn reveal_edit(&mut self) {
        let Some(index) = self.inline.row() else {
            return;
        };
        let height = self.list_height();
        if self.inline.draft_at().is_some() {
            self.list.ensure_visible(index, height);
        } else {
            self.list.select(index, height);
        }
        self.invalidate();
    }

    /// Row `index`'s rectangle in panel coordinates, for tests that click a row.
    #[cfg(test)]
    pub(crate) fn row_rect_at(&self, index: usize) -> Option<RECT> {
        self.row_rect(self.list_rect(self.client()), index)
    }

    /// The inline field's frame, in the accent colour or the error colour while a problem
    /// shows, and the problem under the field, or above it without room below (inline naming
    /// spec §4.4).
    fn paint_inline(
        &self,
        dc: HDC,
        layout: FieldLayout,
        list: RECT,
        palette: &Palette,
        fonts: UiFonts,
        dpi: u32,
    ) {
        let problem = self.inline.problem();
        let outline = if problem.is_some() {
            palette.error_foreground
        } else {
            palette.selection_background
        };
        unsafe {
            fill(dc, layout.frame, outline);
            fill(dc, inset(layout.frame, 1), palette.editor_background);
        }
        let Some(problem) = problem else {
            return;
        };
        let pad = scale(4, dpi);
        let text: Vec<u16> = problem.encode_utf16().collect();
        let mut measured = RECT {
            left: 0,
            top: 0,
            right: (layout.frame.right - layout.frame.left - 2 * pad).max(1),
            bottom: 0,
        };
        unsafe {
            let previous = SelectObject(dc, fonts.text);
            DrawTextW(
                dc,
                text.as_ptr(),
                text.len() as i32,
                &mut measured,
                DT_CALCRECT | DT_WORDBREAK | DT_NOPREFIX,
            );
            SelectObject(dc, previous);
        }
        let rect =
            crate::window::inline_name::message_rect(layout.frame, list, measured.bottom + 2 * pad);
        let inner = RECT {
            left: rect.left + pad,
            top: rect.top + pad,
            right: rect.right - pad,
            bottom: rect.bottom - pad,
        };
        unsafe {
            fill(dc, rect, palette.error_foreground);
            fill(dc, inset(rect, 1), palette.editor_background);
            draw_text(
                dc,
                problem,
                inner,
                fonts.text,
                palette.error_foreground,
                DT_WORDBREAK | DT_NOPREFIX,
            );
        }
    }
```

6. `paint`'s `Mode::Tree` branch becomes:

```rust
            Mode::Tree => {
                // `row_list::paint` draws the rows in view and the scroll thumb.
                let list = self.list_rect(area);
                self.inline.set_colors(*palette);
                let rows = &self.rows;
                let hover_pin = self.hover_pin;
                let icons = &paint.icons;
                // The edited row leaves its name to the field; a draft row shows the icon for
                // what is typed so far (inline naming spec §3.1, §3.3).
                let edited = self.inline.row().map(|index| (index, self.inline.draft_icon()));
                row_list::paint(
                    dc,
                    list,
                    &self.list,
                    palette,
                    focused,
                    &mut |dc, index, rect, look| {
                        let editing = edited
                            .and_then(|(edited, icon)| (edited == index).then_some(icon));
                        draw_tree_row(
                            dc,
                            rows.get(index),
                            rect,
                            look,
                            palette,
                            icons,
                            fonts,
                            dpi,
                            hover_pin && look.hover,
                            editing,
                        );
                    },
                );
                if let Some(layout) = self.inline_layout_in(area, dpi) {
                    self.paint_inline(dc, layout, list, palette, fonts, dpi);
                }
            }
```

7. `draw_tree_row` gains a last parameter `editing: Option<FileIcon>` (doc: "`editing`: the row is under the inline field, which draws its name; a draft row shows this icon"). Its `RowKind::Draft => {}` arm becomes:

```rust
        RowKind::Draft => {
            if let Some(icon) = editing {
                draw_icon(icon);
            }
        }
```

and right before `let font = if matches!(row.kind, RowKind::Unsaved(_))` add:

```rust
    // The field covers an edited row's name (inline naming spec §3.3).
    if editing.is_some() {
        return;
    }
```

8. The AccessibleView impl leaves the draft row out (spec §6). Add to the `impl NotebookView` block that holds `rows()`:

```rust
    /// The tree rows screen readers see: all but the draft row (inline naming spec §6).
    fn accessible_rows(&self) -> usize {
        self.rows.len() - usize::from(self.inline.draft_at().is_some())
    }

    /// The row that accessible tree row `index` stands for.
    fn row_of_accessible(&self, index: usize) -> usize {
        match self.inline.draft_at() {
            Some(draft) if index >= draft => index + 1,
            _ => index,
        }
    }

    /// Row `index`'s accessible tree row; `None` for the draft row.
    fn accessible_of_row(&self, index: usize) -> Option<usize> {
        match self.inline.draft_at() {
            Some(draft) if index == draft => None,
            Some(draft) if index > draft => Some(index - 1),
            _ => Some(index),
        }
    }
```

and replace the AccessibleView impl's six methods with:

```rust
    /// Push buttons first, then the RECENT notebooks (no-notebook state), then the tree rows,
    /// the draft row left out.
    fn accessible_count(&self, client: RECT, dpi: u32) -> usize {
        self.buttons(client, dpi).len() + self.recent.len() + self.accessible_rows()
    }

    fn accessible_item(
        &self,
        index: usize,
        client: RECT,
        dpi: u32,
        focused: bool,
    ) -> Option<crate::window::sidebar_accessibility::AccessibleItem> {
        use crate::window::sidebar_accessibility::{button_item, list_item, row_rect, tree_item};
        let buttons = self.buttons(client, dpi);
        if let Some((name, rect)) = buttons.get(index) {
            return Some(button_item(name, false, false, *rect));
        }
        let index = index - buttons.len();
        if index < self.recent.len() {
            // The no-notebook list holds the RECENT rows, indexed by list position.
            let (rect, visible) = row_rect(self.list_area(client, dpi), self.list(), index);
            return Some(list_item(
                &self.recent_name(index),
                self.list().selected == Some(index),
                focused,
                rect,
                visible,
            ));
        }
        let index = self.row_of_accessible(index - self.recent.len());
        let row = self.rows().get(index)?;
        let (rect, visible) = row_rect(self.list_area(client, dpi), self.list(), index);
        Some(tree_item(
            row,
            self.list().selected == Some(index),
            focused,
            rect,
            visible,
        ))
    }

    fn accessible_hit(&self, point: POINT, client: RECT, dpi: u32) -> Option<usize> {
        let inside = |rect: &RECT| {
            point.x >= rect.left
                && point.x < rect.right
                && point.y >= rect.top
                && point.y < rect.bottom
        };
        let buttons = self.buttons(client, dpi);
        if let Some(index) = buttons.iter().position(|(_, rect)| inside(rect)) {
            return Some(index);
        }
        let area = self.list_area(client, dpi);
        if !inside(&area) {
            return None;
        }
        let row = self.list().row_at(point.y - area.top)?;
        if self.mode == Mode::NoNotebook {
            (row < self.recent.len()).then_some(buttons.len() + row)
        } else {
            (row < self.rows().len())
                .then(|| self.accessible_of_row(row))
                .flatten()
                .map(|row| buttons.len() + self.recent.len() + row)
        }
    }

    fn accessible_current(&self, client: RECT, dpi: u32) -> Option<usize> {
        let buttons = self.buttons(client, dpi).len();
        let selected = self.list().selected?;
        // In the no-notebook state the list's selection is a RECENT row, not a tree row.
        if self.mode == Mode::NoNotebook {
            (selected < self.recent.len()).then_some(buttons + selected)
        } else {
            (selected < self.rows().len())
                .then(|| self.accessible_of_row(selected))
                .flatten()
                .map(|row| buttons + self.recent.len() + row)
        }
    }

    fn accessible_select(&mut self, index: usize, client: RECT, dpi: u32) {
        let buttons = self.buttons(client, dpi).len();
        let (offset, rows) = if self.mode == Mode::NoNotebook {
            (buttons, self.recent.len())
        } else {
            (buttons + self.recent.len(), self.accessible_rows())
        };
        let Some(row) = index.checked_sub(offset).filter(|&row| row < rows) else {
            return;
        };
        let row = if self.mode == Mode::NoNotebook {
            row
        } else {
            self.row_of_accessible(row)
        };
        let area = self.list_area(client, dpi);
        self.list_mut().select(row, area.bottom - area.top);
    }

    fn accessible_identity(&self, index: usize, client: RECT, dpi: u32) -> Option<u64> {
        use crate::window::sidebar_accessibility::identity_of;
        let index = index.checked_sub(self.buttons(client, dpi).len())?;
        if let Some(folder) = self.recent.get(index) {
            return Some(identity_of(folder));
        }
        let row = self
            .rows()
            .get(self.row_of_accessible(index - self.recent.len()))?;
        Some(identity_of(&row.kind))
    }
```

(`accessible_generation` is unchanged.)

9. The field follows its row. `rebuild`, `active_tab_changed` and `select_index` end with `super::inline_name::place(hwnd);` after their `with_view`, and `select_row` becomes:

```rust
/// Selects the row showing `kind` and scrolls it into view; false when no row shows it.
pub(crate) fn select_row(hwnd: HWND, kind: &RowKind) -> bool {
    let selected = with_view(hwnd, |view| {
        let Some(index) = tree::row_index(&view.rows, kind) else {
            return false;
        };
        view.select(index);
        true
    })
    .unwrap_or(false);
    // The field moves with its row (inline naming spec §5.4).
    super::inline_name::place(hwnd);
    selected
}
```

`rebuild`'s new last line carries the comment `// The field follows its row, or goes with an edit the rebuild ended (inline naming spec §5.4).` In `handle`'s `WM_MOUSEWHEEL` arm keep whether it scrolled and place after the borrow:

```rust
        WM_MOUSEWHEEL => {
            let delta = i32::from((wparam >> 16) as u16 as i16);
            let lines = row_list::wheel_lines();
            let scrolled = with_view(hwnd, |view| {
                let height = view.list_height();
                let scrolled = view.list.wheel(delta, lines, height);
                if scrolled {
                    view.invalidate();
                }
                scrolled
            })
            .unwrap_or(false);
            // The field moves with its row (inline naming spec §5.4).
            if scrolled {
                super::inline_name::place(hwnd);
            }
            Some(0)
        }
```

`mouse_move` becomes:

```rust
fn mouse_move(hwnd: HWND, x: i32, y: i32) {
    // Read before the view is borrowed: `ui_fonts` borrows the App itself.
    let fonts = super::main_window::ui_fonts(hwnd);
    let (tools, scrolled) = with_view(hwnd, |view| {
        if let Some(grab) = view.thumb_grab {
            let list = view.list_rect(view.client());
            let scrolled = view.list.drag_thumb(grab, y - list.top, height(list));
            if scrolled {
                view.invalidate();
            }
            return (None, scrolled);
        }
        view.track_leave();
        let hit = view.hit_test(x, y);
        let (row, pin) = match hit {
            Hit::Row { index, part } => (Some(index), part == RowPart::Pin),
            _ => (None, false),
        };
        let hot =
            matches!(hit, Hit::Header(_) | Hit::StateButton | Hit::SecondButton).then_some(hit);
        let row_changed = view.list.set_hover(row);
        if row_changed || view.hover_pin != pin || view.hover != hot {
            view.hover_pin = pin;
            view.hover = hot;
            view.invalidate();
            return (Some(view.tooltip_tools(fonts)), false);
        }
        (None, false)
    })
    .unwrap_or((None, false));
    if let Some(tools) = tools {
        apply_tooltips(hwnd, &tools);
    }
    // The field moves with its row while the thumb is dragged (inline naming spec §5.4).
    if scrolled {
        super::inline_name::place(hwnd);
    }
}
```

10. The header's New folder still runs `CommandId::NoteNewFolder`. In `open_context_menu`'s folder arm: `Some(CommandId::NoteNewFolder) => super::inline_name::new_folder(hwnd, Some(relative.clone())),`.

In `src/window/side_panel.rs`:
- `panel_proc`'s `EN_CHANGE` arm:

```rust
        // The inline name field's text changed (its live check runs), the search box's (the
        // search runs again), or the replace field's (its text is kept).
        WM_COMMAND if lparam != 0 && ((wparam >> 16) & 0xffff) as u32 == EN_CHANGE => {
            if crate::window::inline_name::owns(main, lparam as HWND) {
                crate::window::inline_name::changed(main);
            } else {
                crate::window::search_view::edit_changed(main, lparam as HWND);
            }
            0
        }
        WM_CTLCOLOREDIT if crate::window::inline_name::owns(main, lparam as HWND) => {
            crate::window::inline_name::control_color(main, wparam as HDC) as LRESULT
        }
```

  (the existing `WM_CTLCOLOREDIT =>` search arm stays after it);
- `layout` ends with `crate::window::inline_name::place(hwnd);` after `search_view::layout`.

In `src/window/main_window.rs` `execute_command_with_note`: `CommandId::NoteNewFolder => crate::window::inline_name::new_folder(hwnd, None),`.

- [ ] **Step 6: Retire the name bar's New folder**

In `src/window/library_host.rs`: delete `NEW_FOLDER`, `new_folder` and `submit_new_folder` (`NO_FOLDER_NAME` and `folder_suffix` stay for the name bar's folder rename until Task 4); make `relative_folder` and `folder_taken_error` `pub(crate)`; add after `save_local`:

```rust
/// `save_local` on a writer thread, for a change the user just made (a folder rename).
pub(crate) fn save_local_soon(hwnd: HWND) {
    save_local(
        hwnd,
        LocalWrite {
            wait: false,
            force: false,
        },
    );
}
```

`name_box_submit` becomes:

```rust
    match purpose {
        NamePurpose::FirstSave(id) => submit_first_save(hwnd, id, &text),
        NamePurpose::RenameNote(_) | NamePurpose::RenameFolder(_) if !ready_library(hwnd) => {
            close_name_box(hwnd);
        }
        NamePurpose::RenameNote(id) => submit_rename(hwnd, id, &text),
        NamePurpose::RenameFolder(folder) => submit_rename_folder(hwnd, &folder, &text),
    }
```

In `src/window/name_box.rs` delete the `NewFolder` variant and its arms (`document()`: `Self::RenameFolder(_) => None`; `folder()`: `Self::FirstSave(_) | Self::RenameNote(_) => None, Self::RenameFolder(folder) => Some(folder)`); its doc: "For a folder rename box, the folder being renamed, which must stay in the tree for the box to stay open. A folder box belongs to no tab."

- [ ] **Step 7: Run the tests**

Run: `cargo test --lib -- --test-threads=1 new_folder_from_the_header_names_it new_folder_here_drafts a_typed_folder_name_is_sanitized_hidden a_new_folder_draft_survives the_field_sits_over_its_row the_name_field_is_named ctrl_a_selects_the_name deleting_a_folder_that_holds_the_only_tab_and_the_draft folder_commands_on_an_empty deleting_a_folder_recycles_it a_rescan_keeps_selection_and_expansion_by_path`
Expected: PASS. Then `cargo test --lib notebook_view inline_name sidebar_accessibility` — PASS.

- [ ] **Step 8: Check and commit**

Run: `cargo fmt`, `cargo fmt --check`, then `cargo clippy --all-targets -- -D warnings`
Expected: clean (the module-level `expect(dead_code)` is still fulfilled by the unused rename items).

```bash
git add src/platform/annotation.rs src/platform/mod.rs src/window/inline_name.rs src/window/notebook_view.rs src/window/side_panel.rs src/window/library_host.rs src/window/name_box.rs src/window/main_window.rs
git commit -m "feat(sidebar): New folder names its folder in a draft row in the tree"
```

---

### Task 3: New note creates the file from the name typed in the tree

**Files:**
- Modify: `src/window/commands.rs` (`NoteNew = 193`, `needs_document`, `COMMANDS`, a test)
- Modify: `src/window/command_palette.rs` (`ENTRIES` 72, the row, tests)
- Modify: `src/window/inline_name.rs` (`new_note`, `commit_new_note`, `commit`'s arm)
- Modify: `src/window/notebook_view.rs` (header "+", "New note here", doc of `open_context_menu`)
- Modify: `src/window/main_window.rs` (dispatch, palette filter, tests)
- Modify: `src/window/library_host.rs` (`name_taken_error` `pub(crate)`; `new_note_in`'s doc)

**Interfaces:**
- Consumes (Task 2): `start`, `target_folder`, `fail`, `cancelled`, `cancel`, `end`, `How`; test helpers.
- Consumes (existing): `library_host::name_taken_error(&Path, &str, &str) -> String`, `LibraryState::add_note(&Path) -> bool`, `main_window::{open_note, OpenMode, report_open_failure}`.
- Produces: `CommandId::NoteNew` (193); `pub(crate) fn inline_name::new_note(HWND, Option<PathBuf>)`; private `commit_new_note(HWND, How, &Path, &str)`.

- [ ] **Step 1: Write the failing tests**

In `src/window/commands.rs` tests, after `new_folder_is_192_and_neither_needs_a_document_nor_the_sidebar`:

```rust
    #[test]
    fn new_note_is_193_and_neither_needs_a_document_nor_the_sidebar() {
        // Break caught: a renumbered command breaking the menus, or New note hidden while no
        // tab is open, when it is most wanted (inline naming spec §8).
        assert_eq!(CommandId::NoteNew as u16, 193);
        assert_eq!(CommandId::try_from(193), Ok(CommandId::NoteNew));
        assert!(!CommandId::NoteNew.needs_document());
        assert!(!CommandId::NoteNew.is_sidebar());
    }
```

In `src/window/command_palette.rs` tests, change `assert_eq!(ENTRIES.len(), 71);` to `72` and add:

```rust
    #[test]
    fn new_note_is_listed_right_before_new_folder_without_a_shortcut() {
        // Break caught: the palette never offering New note, listing it away from New folder,
        // or showing Ctrl+N (which opens an untitled tab) beside it (inline naming spec §8).
        assert_eq!(labels("notebook: new note")[0], "Notebook: New note\u{2026}");
        assert_eq!(shortcut_text(CommandId::NoteNew), None);
        let position = |command| ENTRIES.iter().position(|entry| entry.command == command);
        assert_eq!(
            position(CommandId::NoteNew).map(|index| index + 1),
            position(CommandId::NoteNewFolder)
        );
    }
```

In `src/window/main_window.rs` tests, replace `the_palette_offers_new_folder_only_while_a_notebook_is_open` with:

```rust
    #[test]
    fn the_palette_offers_new_note_and_new_folder_only_while_a_notebook_is_open() {
        // Break caught: "Notebook: New note…" or "Notebook: New folder…" listed with no
        // notebook, where they can only say "Open a notebook first." (inline naming spec §3.1).
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("palette-new-note");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        let listed = |command: CommandId| {
            execute_command(window.hwnd, CommandId::CommandPalette);
            let listed = app_mut(window.hwnd)
                .command_palette
                .as_ref()
                .unwrap()
                .shown()
                .iter()
                .any(|entry| entry.command == command);
            super::close_command_palette(window.hwnd, false);
            listed
        };
        assert!(crate::window::library_host::folder(window.hwnd).is_none());
        assert!(!listed(CommandId::NoteNew));
        assert!(!listed(CommandId::NoteNewFolder));
        scratch.install(window.hwnd);
        assert!(listed(CommandId::NoteNew));
        assert!(listed(CommandId::NoteNewFolder));
    }
```

In `the_command_palette_filters_as_typed_and_runs_the_selection_on_enter`, change the filter line to `.filter(|entry| !matches!(entry.command, CommandId::NoteNew | CommandId::NoteNewFolder))` and its comment to "and New note and New folder only while a notebook is open".

In `the_context_menu_acts_on_its_row_not_the_active_tab`, replace the block from `menu(&RowKind::Folder("sub".into()), CommandId::New);` through `let untitled = untitled.id;` with:

```rust
        menu(&RowKind::Folder("sub".into()), CommandId::NoteNew);
        assert_eq!(
            crate::window::inline_name::purpose(window.hwnd),
            Some(crate::window::inline_name::Purpose::NewNote("sub".into()))
        );
        crate::window::inline_name::cancel(window.hwnd);
        execute_command(window.hwnd, CommandId::New);
        let untitled = app_mut(window.hwnd).tabs.active().unwrap().id;
```

Add the new tests after `the_palette_offers_new_note_and_new_folder_only_while_a_notebook_is_open`:

```rust
    #[test]
    fn plus_new_note_here_and_the_palette_each_draft_a_note_in_the_right_folder() {
        // Break caught: "+" or "New note here" opening an untitled tab instead, a draft in the
        // wrong folder or at the wrong depth, a field not focused, or the palette ignoring the
        // selected row's folder (inline naming spec §3.1).
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetFocus, VK_ESCAPE, VK_RETURN};
        use windows_sys::Win32::UI::WindowsAndMessaging::{SetWindowTextW, WM_KEYDOWN};
        use crate::window::inline_name::Purpose;
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("inline-new-note-starts");
        std::fs::create_dir_all(scratch.folder().join("sub")).unwrap();
        scratch.note(r"sub\b.md", "b");
        scratch.note("top.md", "t");
        let (window, _editor) = notebook_window(&scratch);
        let tabs = super::tab_count(window.hwnd);
        select_row(window.hwnd, &RowKind::Note("top.md".into()));

        crate::window::notebook_view::header_clicked(
            window.hwnd,
            crate::window::notebook_view::HeaderButton::NewNote,
        );
        assert_eq!(
            crate::window::inline_name::purpose(window.hwnd),
            Some(Purpose::NewNote(std::path::PathBuf::new()))
        );
        assert_eq!(draft_row(window.hwnd), Some((0, 0)));
        assert_eq!(unsafe { GetFocus() }, inline_field(window.hwnd));
        assert_eq!(super::tab_count(window.hwnd), tabs, "no untitled tab");
        field_key(window.hwnd, VK_ESCAPE);
        assert_eq!(draft_row(window.hwnd), None);

        crate::window::library_host::set_expanded(
            window.hwnd,
            std::path::Path::new("sub"),
            false,
        );
        crate::window::notebook_view::rebuild(window.hwnd);
        let sub = row_of(window.hwnd, &RowKind::Folder("sub".into()));
        crate::window::menus::answer_next_popup_menu(|_| Some(CommandId::NoteNew));
        crate::window::notebook_view::open_context_menu(window.hwnd, sub, None);
        assert_eq!(
            crate::window::inline_name::purpose(window.hwnd),
            Some(Purpose::NewNote("sub".into()))
        );
        assert_eq!(draft_row(window.hwnd), Some((sub + 1, 1)), "sub expanded");
        field_key(window.hwnd, VK_ESCAPE);

        select_row(window.hwnd, &RowKind::Note(r"sub\b.md".into()));
        execute_command(window.hwnd, CommandId::CommandPalette);
        let query = app_mut(window.hwnd)
            .command_palette
            .as_ref()
            .unwrap()
            .query_hwnd();
        let typed = crate::platform::wide_null("Notebook: New note");
        unsafe { SetWindowTextW(query, typed.as_ptr()) };
        unsafe { SendMessageW(query, WM_KEYDOWN, VK_RETURN as usize, 0) };
        assert_eq!(
            crate::window::inline_name::purpose(window.hwnd),
            Some(Purpose::NewNote("sub".into())),
            "the selected note's folder"
        );
        assert_eq!(unsafe { GetFocus() }, inline_field(window.hwnd));
        assert_eq!(super::tab_count(window.hwnd), tabs);
    }

    #[test]
    fn enter_on_a_new_note_creates_the_file_and_opens_it_with_focus_in_the_editor() {
        // Break caught: the note left unsaved in an untitled tab, created with text or over a
        // file, opened as the preview, not listed in the tree, or the focus left in the tree
        // (spec §4.1, §5.2).
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetFocus, VK_RETURN};
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("inline-new-note-enter");
        scratch.note("top.md", "t");
        let (window, editor) = notebook_window(&scratch);

        crate::window::inline_name::new_note(window.hwnd, None);
        type_into_field(window.hwnd, "todo");
        field_key(window.hwnd, VK_RETURN);

        let todo = scratch.folder().join("todo.md");
        assert_eq!(std::fs::read(&todo).unwrap(), b"", "an empty file");
        assert!(!inline_open(window.hwnd));
        let active = app_mut(window.hwnd).tabs.active().unwrap();
        assert_eq!(active.path.as_deref(), Some(todo.as_path()));
        assert!(!active.preview);
        assert_eq!(unsafe { GetFocus() }, editor.hwnd());
        row_of(window.hwnd, &RowKind::Note("todo.md".into()));

        crate::window::inline_name::new_note(window.hwnd, Some(std::path::PathBuf::new()));
        type_into_field(window.hwnd, "data.json");
        field_key(window.hwnd, VK_RETURN);
        assert!(scratch.folder().join("data.json").exists(), "a typed note extension is kept");
        assert!(!scratch.folder().join("data.json.md").exists());
    }

    #[test]
    fn a_new_note_with_a_listed_name_shows_the_message_and_enter_keeps_the_field() {
        // Break caught: a clash with a listed note missed until Enter, the message shown in
        // another case than typed, or Enter going ahead (spec §4.4).
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_RETURN;
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("inline-new-note-taken");
        scratch.note("a.md", "a");
        let (window, _editor) = notebook_window(&scratch);
        crate::window::inline_name::new_note(window.hwnd, None);

        type_into_field(window.hwnd, "A");
        assert_eq!(
            crate::window::inline_name::problem(window.hwnd).as_deref(),
            Some("A.md already exists here.")
        );
        field_key(window.hwnd, VK_RETURN);
        assert!(inline_open(window.hwnd));
        assert_eq!(std::fs::read_to_string(scratch.folder().join("a.md")).unwrap(), "a");
        type_into_field(window.hwnd, "b");
        assert_eq!(crate::window::inline_name::problem(window.hwnd), None);
    }

    #[test]
    fn a_new_note_clashing_with_an_unlisted_file_or_a_vanished_folder_says_so_after_enter() {
        // Break caught: a file written after the scan overwritten, a note created somewhere
        // else when its folder was deleted in Explorer, or the field closing on the failure
        // (spec §4.4, §5.2).
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_RETURN;
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("inline-new-note-disk");
        std::fs::create_dir_all(scratch.folder().join("sub")).unwrap();
        scratch.note("top.md", "t");
        let (window, _editor) = notebook_window(&scratch);
        std::fs::write(scratch.folder().join("fresh.md"), "theirs").unwrap();

        crate::window::inline_name::new_note(window.hwnd, None);
        type_into_field(window.hwnd, "fresh");
        assert_eq!(crate::window::inline_name::problem(window.hwnd), None, "not listed");
        field_key(window.hwnd, VK_RETURN);
        assert!(inline_open(window.hwnd));
        assert_eq!(
            crate::window::inline_name::problem(window.hwnd).as_deref(),
            Some("fresh.md already exists. Try fresh 2.md.")
        );
        assert_eq!(
            std::fs::read_to_string(scratch.folder().join("fresh.md")).unwrap(),
            "theirs"
        );
        crate::window::inline_name::cancel(window.hwnd);

        crate::window::inline_name::new_note(window.hwnd, Some("sub".into()));
        std::fs::remove_dir(scratch.folder().join("sub")).unwrap();
        type_into_field(window.hwnd, "x");
        field_key(window.hwnd, VK_RETURN);
        assert!(inline_open(window.hwnd));
        let problem = crate::window::inline_name::problem(window.hwnd).unwrap();
        assert!(problem.starts_with("FastPad could not create x.md: "), "{problem}");
        assert!(!scratch.folder().join("x.md").exists());
    }

    #[test]
    fn a_rescan_that_lists_the_typed_name_shows_the_problem_without_a_keystroke() {
        // Break caught: the live check run only on keystrokes, so a note that appeared on disk
        // while the user typed its name is only caught by the disk call (spec §4.4, §5.4).
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("inline-new-note-rescan");
        scratch.note("top.md", "t");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        ensure_sidebar(window.hwnd);
        app_mut(window.hwnd).library.data_dir = Some(scratch.data());
        scratch.install(window.hwnd);
        crate::window::notebook_view::rebuild(window.hwnd);
        crate::window::inline_name::new_note(window.hwnd, None);
        type_into_field(window.hwnd, "idea");
        assert_eq!(crate::window::inline_name::problem(window.hwnd), None);

        scratch.note("idea.md", "made elsewhere");
        rescan_and_wait(window.hwnd);

        assert!(inline_open(window.hwnd));
        assert_eq!(
            crate::window::inline_name::problem(window.hwnd).as_deref(),
            Some("idea.md already exists here.")
        );
    }

    #[test]
    fn the_first_note_of_an_empty_notebook_gets_a_draft_row() {
        // Break caught: "+" in a notebook with no notes doing nothing, because the empty state
        // has no tree to put the draft row in (spec §3.1).
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE;
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("inline-new-note-empty");
        let (window, _editor) = notebook_window(&scratch);
        assert_eq!(notebook_view(window.hwnd).mode, Mode::Empty);

        crate::window::notebook_view::header_clicked(
            window.hwnd,
            crate::window::notebook_view::HeaderButton::NewNote,
        );
        assert_eq!(notebook_view(window.hwnd).mode, Mode::Tree);
        assert_eq!(draft_row(window.hwnd), Some((0, 0)));
        field_key(window.hwnd, VK_ESCAPE);
        assert_eq!(notebook_view(window.hwnd).mode, Mode::Empty);
    }
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test --lib -- --test-threads=1 new_note_is_193 new_note_is_listed_right_before plus_new_note_here enter_on_a_new_note a_new_note_with_a_listed_name a_new_note_clashing a_rescan_that_lists_the_typed_name the_first_note_of_an_empty the_palette_offers_new_note the_context_menu_acts_on_its_row the_command_palette_filters_as_typed`
Expected: FAIL to compile, "no variant named `NoteNew`".

- [ ] **Step 3: Add the command and its palette row**

In `src/window/commands.rs`: add `NoteNew = 193,` after `NoteNewFolder = 192,`; add `| Self::NoteNew` after `| Self::NoteNewFolder` in `needs_document`; `const COMMANDS: [CommandId; 85]` with `CommandId::NoteNew,` appended.

In `src/window/command_palette.rs`: `pub(crate) const ENTRIES: [PaletteEntry; 72]`, and before the New folder row:

```rust
    entry("Notebook: New note\u{2026}", CommandId::NoteNew),
```

In `src/window/main_window.rs`, the palette filter:

```rust
        // New note and New folder need a notebook, open or loading, to put the item in (inline
        // naming spec §3.1).
        let notebook = crate::window::library_host::folder(hwnd).is_some();
```

and `&& (notebook || !matches!(command, CommandId::NoteNew | CommandId::NoteNewFolder))`; the dispatch gains `CommandId::NoteNew => crate::window::inline_name::new_note(hwnd, None),` next to `NoteNewFolder`.

- [ ] **Step 4: Write New note**

In `src/window/inline_name.rs`, after `new_folder`:

```rust
/// The header's "+", "New note here" (`parent`) and Notebook: New note… (spec §3.1).
pub(crate) fn new_note(hwnd: HWND, parent: Option<PathBuf>) {
    if let Some(parent) = target_folder(hwnd, parent) {
        start(hwnd, Purpose::NewNote(parent));
    }
}
```

In `commit`, the match becomes:

```rust
    match purpose {
        Purpose::NewNote(parent) => commit_new_note(hwnd, how, &parent, &text),
        Purpose::NewFolder(parent) => commit_new_folder(hwnd, how, &parent, &text),
        Purpose::RenameNote(_) | Purpose::RenameFolder(_) => cancelled(hwnd, how),
    }
```

After `commit_new_folder` add:

```rust
/// New note (spec §4.1, §5.2): the empty file is made with the one disk call, never over an
/// existing file, listed, and opened as a normal tab. After Enter the focus goes to the editor.
fn commit_new_note(hwnd: HWND, how: How, parent: &Path, text: &str) {
    let Some(root) = library_host::folder(hwnd) else {
        cancel(hwnd);
        return;
    };
    let Some(name) = title::new_note_name(text) else {
        cancelled(hwnd, how);
        return;
    };
    // Defence in depth: the note made must be inside the notebook.
    if !parent.as_os_str().is_empty() && !tree::is_plain_relative_folder(parent) {
        cancel(hwnd);
        return;
    }
    let folder = root.join(parent);
    let path = folder.join(&name);
    let created = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map(drop);
    if let Err(error) = created {
        // A file the tree doesn't list may already have the name.
        let error = if error.kind() == std::io::ErrorKind::AlreadyExists {
            let (stem, extension) = title::split_typed_name(text, "md");
            library_host::name_taken_error(&folder, &stem, &extension)
        } else {
            format!("FastPad could not create {name}: {error}")
        };
        fail(hwnd, how, error);
        return;
    }
    with_state(hwnd, |state| state.add_note(&path));
    end(hwnd);
    side_panel::refresh(hwnd);
    let mode = super::main_window::OpenMode::Permanent;
    if let Err(error) = super::main_window::open_note(hwnd, &path, mode, how == How::Enter) {
        super::main_window::report_open_failure(hwnd, &path, &error);
    }
}
```

In `src/window/library_host.rs` make `name_taken_error` `pub(crate)`, and change `new_note_in`'s doc first sentence to "Ctrl+N, File → New tab, and the empty notebook's New note button."

In `src/window/notebook_view.rs`: `header_clicked` runs `HeaderButton::NewNote => run(hwnd, CommandId::NoteNew)`; the folder menu entry is `MenuEntry::command("New note here", CommandId::NoteNew)` and its answer `Some(CommandId::NoteNew) => super::inline_name::new_note(hwnd, Some(relative.clone())),` (remove the `CommandId::New` arm and, if `path` is then unused in that arm, keep it only for Reveal); `open_context_menu`'s doc ends "…and \"New note here\" is `CommandId::NoteNew` here."

- [ ] **Step 5: Run the tests**

Run the command from Step 2, then `cargo test --lib commands:: command_palette::`.
Expected: PASS.

- [ ] **Step 6: Check and commit**

Run: `cargo fmt`, `cargo fmt --check`, then `cargo clippy --all-targets -- -D warnings`
Expected: clean.

```bash
git add src/window/commands.rs src/window/command_palette.rs src/window/inline_name.rs src/window/notebook_view.rs src/window/main_window.rs src/window/library_host.rs
git commit -m "feat(sidebar): New note creates the file from the name typed in the tree"
```

---

### Task 4: Renames in the tree, with the row revealed

**Files:**
- Modify: `src/window/inline_name.rs` (`rename`, `rename_note_at`, `rename_active`, `reveal`, `commit_rename_note`, `commit_rename_folder`; drop the module `expect`)
- Modify: `src/window/library_host.rs` (remove `rename_folder`, `submit_rename_folder`, `folder_suffix`, `rename_file`; visibility; `rename_note`, `rebind_open_tab` docs; `close_stale_name_box`; `name_box_submit`; module doc)
- Modify: `src/window/name_box.rs` (drop `RenameFolder`, `folder()`; `document()` returns `DocumentId`; `focus()`; module doc)
- Modify: `src/window/notebook_view.rs` (F2, the menus' Rename…)
- Modify: `src/window/main_window.rs` (`NoteRename` dispatch; tests)

**Interfaces:**
- Consumes (Tasks 1–3): `start`, `fail`, `cancelled`, `cancel`, `end`, `How`, `Purpose::{RenameNote, RenameFolder}`, test helpers.
- Consumes (existing, made `pub(crate)` here): `library_host::{rebind_open_tab(HWND, &Path, PathBuf) -> Result<bool, ()>, already_exists(&FastPadError) -> bool, tabs_under(HWND, &Path) -> Vec<(DocumentId, PathBuf, bool)>, reroot_save_folders(HWND, &Path, &Path), rename_folder_back(&Path, &Path) -> crate::Result<()>, rename_undo_failed_notice(&str, &str, &[PathBuf]) -> String, save_local_soon, schedule_write, active_file, rename_note}`.
- Produces: `pub(crate) fn inline_name::{rename(HWND, &RowKind), rename_note_at(HWND, &Path), rename_active(HWND)}`; `NamePurpose::document(&self) -> DocumentId`.

- [ ] **Step 1: Write the failing tests (and migrate the folder rename tests)**

Replace `f2_on_a_note_row_that_is_not_open_opens_it_and_the_rename_box` with:

```rust
    #[test]
    fn f2_and_rename_on_a_note_row_rename_it_in_the_tree_without_opening_a_tab() {
        // Break caught: F2 opening the note as a tab first, renaming the active tab's note, a
        // prefill that selects the extension, or the renamed row losing the selection and the
        // focus (inline naming spec §3.3, §5.2).
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetFocus, VK_F2, VK_RETURN};
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("inline-rename-note");
        let a = scratch.note("a.md", "a");
        scratch.note("b.md", "b");
        let (window, _editor) = notebook_window(&scratch);
        super::open_path(window.hwnd, &a).unwrap();
        let tabs = super::tab_count(window.hwnd);
        select_row(window.hwnd, &RowKind::Note("b.md".into()));

        assert!(crate::window::notebook_view::key_down(window.hwnd, VK_F2));
        assert_eq!(
            crate::window::inline_name::purpose(window.hwnd),
            Some(crate::window::inline_name::Purpose::RenameNote("b.md".into()))
        );
        assert_eq!(field_text(window.hwnd), "b.md");
        assert_eq!(field_selection(window.hwnd), (0, 1), "the stem is selected");
        assert_eq!(super::tab_count(window.hwnd), tabs, "no tab opened");
        type_into_field(window.hwnd, "c");
        field_key(window.hwnd, VK_RETURN);

        assert!(!scratch.folder().join("b.md").exists());
        assert_eq!(std::fs::read_to_string(scratch.folder().join("c.md")).unwrap(), "b");
        assert_eq!(super::tab_count(window.hwnd), tabs);
        assert_eq!(
            app_mut(window.hwnd).tabs.active().unwrap().path.as_deref(),
            Some(a.as_path())
        );
        assert_eq!(selected_kind(window.hwnd), Some(RowKind::Note("c.md".into())));
        assert_eq!(unsafe { GetFocus() }, sidebar_windows(window.hwnd).1);

        let index = row_of(window.hwnd, &RowKind::Note("c.md".into()));
        crate::window::menus::answer_next_popup_menu(|_| Some(CommandId::NoteRename));
        crate::window::notebook_view::open_context_menu(window.hwnd, index, None);
        assert_eq!(
            crate::window::inline_name::purpose(window.hwnd),
            Some(crate::window::inline_name::Purpose::RenameNote("c.md".into()))
        );
        assert_eq!(super::tab_count(window.hwnd), tabs);
    }

    #[test]
    fn renaming_open_dirty_and_preview_notes_in_the_tree_rebinds_their_tabs() {
        // Break caught: a rename that saves a dirty tab, turns the preview into a normal tab,
        // or leaves either on the old path (spec §5.2).
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_F2, VK_RETURN};
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("inline-rename-tabs");
        let a = scratch.note("a.md", "a");
        let b = scratch.note("b.md", "b");
        let (window, editor) = notebook_window(&scratch);
        // Autosave would save `a` the moment `b` opens; the rename must leave it dirty.
        execute_command(window.hwnd, CommandId::ToggleFolderAutosave);
        super::open_path(window.hwnd, &a).unwrap();
        editor.set_text("a, edited").unwrap();
        super::open_note(window.hwnd, &b, super::OpenMode::Preview, false).unwrap();
        let a_id = app_mut(window.hwnd).tabs.find_stored_path(&a).unwrap();
        let b_id = app_mut(window.hwnd).tabs.find_stored_path(&b).unwrap();
        let rename = |from: &str, to: &str| {
            select_row(window.hwnd, &RowKind::Note(from.into()));
            assert!(crate::window::notebook_view::key_down(window.hwnd, VK_F2));
            type_into_field(window.hwnd, to);
            field_key(window.hwnd, VK_RETURN);
        };

        rename("a.md", "a2");
        rename("b.md", "b2");

        let tabs = &app_mut(window.hwnd).tabs;
        assert_eq!(
            tabs.document(a_id).unwrap().path,
            Some(scratch.folder().join("a2.md"))
        );
        assert!(tabs.document(a_id).unwrap().dirty);
        assert_eq!(
            std::fs::read_to_string(scratch.folder().join("a2.md")).unwrap(),
            "a",
            "nothing was saved"
        );
        assert_eq!(
            tabs.document(b_id).unwrap().path,
            Some(scratch.folder().join("b2.md"))
        );
        assert_eq!(tabs.preview_id(), Some(b_id), "the preview stays the preview");
        assert_eq!(super::tab_count(window.hwnd), 2);
    }

    #[test]
    fn a_case_only_rename_in_the_tree_renames_the_note_and_the_folder() {
        // Break caught: "plan.md" → "Plan.md" or "sub" → "Sub" refused as a clash with itself,
        // or a no-op on NTFS (spec §4.3).
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_RETURN;
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("inline-rename-case");
        std::fs::create_dir_all(scratch.folder().join("sub")).unwrap();
        scratch.note("plan.md", "p");
        let (window, _editor) = notebook_window(&scratch);
        let names = || {
            let mut names: Vec<String> = std::fs::read_dir(scratch.folder())
                .unwrap()
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .filter(|name| !name.starts_with('.'))
                .collect();
            names.sort();
            names
        };

        crate::window::inline_name::rename(window.hwnd, &RowKind::Note("plan.md".into()));
        type_into_field(window.hwnd, "Plan.md");
        assert_eq!(crate::window::inline_name::problem(window.hwnd), None);
        field_key(window.hwnd, VK_RETURN);
        crate::window::inline_name::rename(window.hwnd, &RowKind::Folder("sub".into()));
        type_into_field(window.hwnd, "Sub");
        field_key(window.hwnd, VK_RETURN);

        assert_eq!(names(), ["Plan.md", "Sub"]);
        assert!(!inline_open(window.hwnd));
    }

    #[test]
    fn note_rename_from_the_palette_with_the_sidebar_hidden_reveals_the_row_and_edits_it() {
        // Break caught: Note: Rename… on the active tab opening the name bar while the note has
        // a row, or editing a row nobody can see in a hidden sidebar or a collapsed folder
        // (spec §3.3).
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetFocus;
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("inline-rename-reveal");
        std::fs::create_dir_all(scratch.folder().join("sub")).unwrap();
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        ensure_sidebar(window.hwnd);
        open_note(&window, &scratch, r"sub\a.md", "a");
        crate::window::library_host::set_expanded(
            window.hwnd,
            std::path::Path::new("sub"),
            false,
        );
        crate::window::side_panel::show_view(
            window.hwnd,
            crate::config::SidebarView::Hidden,
            false,
        );

        execute_command(window.hwnd, CommandId::NoteRename);

        assert_eq!(
            crate::window::side_panel::current_view(window.hwnd),
            crate::config::SidebarView::Notebook
        );
        assert!(
            crate::window::library_host::expanded(window.hwnd)
                .contains(&std::path::PathBuf::from("sub"))
        );
        assert_eq!(
            selected_kind(window.hwnd),
            Some(RowKind::Note(r"sub\a.md".into()))
        );
        assert_eq!(
            crate::window::inline_name::purpose(window.hwnd),
            Some(crate::window::inline_name::Purpose::RenameNote(
                r"sub\a.md".into()
            ))
        );
        assert_eq!(field_text(window.hwnd), "a.md");
        assert_eq!(unsafe { GetFocus() }, inline_field(window.hwnd));
        assert!(!app_mut(window.hwnd)
            .name_box
            .as_ref()
            .is_some_and(|name_box| name_box.is_visible()));
    }

    #[test]
    fn note_rename_on_a_file_outside_the_notebook_uses_the_name_bar() {
        // Break caught: a file with no row revealing nothing and doing nothing, or the tree
        // edited for some other row (spec §3.3).
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("inline-rename-outside");
        scratch.note("a.md", "a");
        let (window, _editor) = notebook_window(&scratch);
        let outside = scratch.root.join("outside.md");
        std::fs::write(&outside, "o").unwrap();
        super::open_path(window.hwnd, &outside).unwrap();
        let id = app_mut(window.hwnd).tabs.active().unwrap().id;

        execute_command(window.hwnd, CommandId::NoteRename);

        assert!(!inline_open(window.hwnd));
        let name_box = app_mut(window.hwnd).name_box.as_ref().unwrap();
        assert!(name_box.is_visible());
        assert_eq!(
            name_box.purpose(),
            Some(&crate::window::name_box::NamePurpose::RenameNote(id))
        );
    }
```

Migrate the folder rename tests to the field. In each of `renaming_a_folder_moves_its_preview_and_dirty_tabs_without_saving_them`, `a_case_only_folder_rename_renames_it_on_disk_and_in_tabs_and_expansion`, `a_folder_rename_an_open_tab_cannot_follow_is_undone`, `a_folder_renamed_while_a_rescan_runs_keeps_its_new_name_once_the_rescan_lands`, `a_new_note_made_in_a_folder_saves_into_it_after_the_folder_is_renamed` and `a_folder_rename_that_cannot_be_undone_stands_and_names_the_tab_left_behind`:
- after the scratch is installed and before the rename, add `crate::window::notebook_view::rebuild(window.hwnd);` where the test does not already rebuild;
- replace `crate::window::library_host::rename_folder(window.hwnd, std::path::Path::new("sub"));` with `crate::window::inline_name::rename(window.hwnd, &RowKind::Folder("sub".into()));`;
- replace `type_into_name_box(window.hwnd, X);` with `type_into_field(window.hwnd, X);`;
- replace `crate::window::library_host::name_box_submit(window.hwnd);` that follows it with `field_key(window.hwnd, windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_RETURN);`;
- replace `assert!(!name_box_visible(window.hwnd));` with `assert!(!inline_open(window.hwnd));`.
- In `a_folder_rename_an_open_tab_cannot_follow_is_undone`, the refusal becomes:

```rust
        assert!(inline_open(window.hwnd));
        assert_eq!(
            crate::window::inline_name::problem(window.hwnd).as_deref(),
            Some("Another tab already has that file open.")
        );
```

- In `a_new_note_made_in_a_folder_saves_into_it_after_the_folder_is_renamed`, the first-save lines at the end keep the name bar (`execute_command(… Save)` then `name_box_submit`): only the rename part moves.

Replace `renaming_a_folder_with_an_open_note_rebinds_the_tab_and_keeps_the_notes_pin`'s block from `assert!(crate::window::notebook_view::key_down(window.hwnd, VK_F2));` through `crate::window::library_host::name_box_submit(window.hwnd);` with:

```rust
        assert!(crate::window::notebook_view::key_down(window.hwnd, VK_F2));
        assert_eq!(
            crate::window::inline_name::purpose(window.hwnd),
            Some(crate::window::inline_name::Purpose::RenameFolder("sub".into()))
        );
        assert_eq!(field_text(window.hwnd), "sub");
        type_into_field(window.hwnd, "Projects");
        field_key(window.hwnd, windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_RETURN);
```

and its `assert!(!name_box_visible(window.hwnd));` with `assert!(!inline_open(window.hwnd));`.

Replace `a_folder_rename_onto_a_sibling_is_refused_and_the_same_name_changes_nothing` with:

```rust
    #[test]
    fn a_folder_rename_onto_a_sibling_is_refused_and_the_same_name_changes_nothing() {
        // Break caught: a rename onto an existing sibling (in another case) merging or failing
        // oddly, Note: Rename on a focused folder row renaming the active tab instead, or an
        // unchanged name showing a message (spec §4.3, §4.4).
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetFocus, SetFocus, VK_RETURN};
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("folder-rename-clash");
        std::fs::create_dir_all(scratch.folder().join("sub")).unwrap();
        std::fs::create_dir_all(scratch.folder().join("Other")).unwrap();
        scratch.note(r"sub\a.md", "a");
        let (window, _editor) = notebook_window(&scratch);
        select_row(window.hwnd, &RowKind::Folder("sub".into()));
        let (_, panel) = sidebar_windows(window.hwnd);
        unsafe { SetFocus(panel) };
        assert_eq!(unsafe { GetFocus() }, panel);

        execute_command(window.hwnd, CommandId::NoteRename);
        assert_eq!(
            crate::window::inline_name::purpose(window.hwnd),
            Some(crate::window::inline_name::Purpose::RenameFolder("sub".into()))
        );
        type_into_field(window.hwnd, "other");
        assert_eq!(
            crate::window::inline_name::problem(window.hwnd).as_deref(),
            Some("other already exists here.")
        );
        field_key(window.hwnd, VK_RETURN);
        assert!(inline_open(window.hwnd));
        assert!(scratch.folder().join(r"sub\a.md").exists());

        type_into_field(window.hwnd, "sub");
        assert_eq!(crate::window::inline_name::problem(window.hwnd), None);
        field_key(window.hwnd, VK_RETURN);
        assert!(!inline_open(window.hwnd));
        assert!(scratch.folder().join(r"sub\a.md").exists());
    }
```

Replace `a_folder_name_box_selects_the_whole_name_even_with_a_dot` with:

```rust
    #[test]
    fn a_folder_rename_selects_the_whole_name_even_with_a_dot() {
        // Break caught: "v1.2" opening with only "v1" selected, as a file name's stem would be,
        // so typing keeps ".2" (spec §3.3).
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("folder-rename-dot");
        std::fs::create_dir_all(scratch.folder().join("v1.2")).unwrap();
        let (window, _editor) = notebook_window(&scratch);

        crate::window::inline_name::rename(window.hwnd, &RowKind::Folder("v1.2".into()));

        assert_eq!(field_text(window.hwnd), "v1.2");
        assert_eq!(field_selection(window.hwnd), (0, 4));
    }
```

Replace `palette_rename_with_a_folder_row_focused_opens_the_folder_rename_box` with the same test renamed `palette_rename_with_a_folder_row_focused_edits_the_folder_row`, whose assertions after the palette's Enter are:

```rust
        assert_eq!(
            crate::window::inline_name::purpose(window.hwnd),
            Some(crate::window::inline_name::Purpose::RenameFolder("sub".into()))
        );
        assert_eq!(field_text(window.hwnd), "sub");
        assert!(
            scratch.folder().join("sub").is_dir(),
            "nothing renamed before Enter"
        );
        assert!(active.exists());
```

In `folder_commands_on_an_empty_or_escaping_path_touch_no_disk`, delete `use crate::window::name_box::NamePurpose;`, the `submit` closure and its two `RenameFolder` calls, and add:

```rust
        for bad in ["", ".."] {
            crate::window::inline_name::rename(window.hwnd, &RowKind::Folder(bad.into()));
            assert!(!inline_open(window.hwnd), "{bad:?} has no row");
        }
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test --lib -- --test-threads=1 f2_and_rename_on_a_note_row renaming_open_dirty_and_preview a_case_only_rename_in_the_tree note_rename_from_the_palette_with_the_sidebar_hidden note_rename_on_a_file_outside renaming_a_folder a_case_only_folder_rename a_folder_rename a_folder_renamed_while a_new_note_made_in_a_folder palette_rename_with_a_folder_row folder_commands_on_an_empty`
Expected: FAIL to compile, "cannot find function `rename` in module `inline_name`".

- [ ] **Step 3: Write the renames**

In `src/window/inline_name.rs`, delete the module-level `#![cfg_attr(not(test), expect(dead_code, …))]` (everything is wired up after this task), add `use super::main_window::app_ptr;` (merge into the `main_window` import) and `use crate::library;`. After `new_note` add:

```rust
/// F2 or Rename… on a note or folder row (spec §3.3): the field over that row's name.
pub(crate) fn rename(hwnd: HWND, row: &RowKind) {
    let purpose = match row {
        RowKind::Note(relative) => Purpose::RenameNote(relative.clone()),
        RowKind::Folder(relative) => Purpose::RenameFolder(relative.clone()),
        RowKind::Unsaved(_) | RowKind::Draft => return,
    };
    start(hwnd, purpose);
}

/// Note: Rename… on the note at `path` (absolute; the palette's recorded row): its row,
/// revealed, else the name bar for a file the tree has no row for (spec §3.3).
pub(crate) fn rename_note_at(hwnd: HWND, path: &Path) {
    match reveal(hwnd, path) {
        Some(relative) => start(hwnd, Purpose::RenameNote(relative)),
        None => library_host::rename_note(hwnd),
    }
}

/// Note: Rename… with no row focused: the active tab's note (spec §3.3).
pub(crate) fn rename_active(hwnd: HWND) {
    if let Some(path) = library_host::active_file(hwnd) {
        rename_note_at(hwnd, &path);
    }
}

/// Shows the note at `path` in the Notebook view (opening the sidebar on it if hidden or on
/// another view), its folders expanded and its row selected and scrolled into view (spec
/// §3.3). `None`, changing nothing, when it has no row there: outside the notebook, not a note,
/// not listed yet, or no sidebar.
fn reveal(hwnd: HWND, path: &Path) -> Option<PathBuf> {
    let root = library_host::folder(hwnd)?;
    if !library::is_inside(&root, path) || side_panel::windows(hwnd).is_none() {
        return None;
    }
    let relative = library::record_path(&root, path);
    let listed = with_state(hwnd, |state| {
        state
            .notes
            .iter()
            .any(|note| library::model::same_path(&note.path, &relative))
    })
    .unwrap_or(false);
    if !listed {
        return None;
    }
    if side_panel::current_view(hwnd) != SidebarView::Notebook {
        side_panel::show_view(hwnd, SidebarView::Notebook, false);
    }
    for folder in tree::ancestors(&relative) {
        library_host::set_expanded(hwnd, &folder, true);
    }
    if notebook_view::stale(hwnd) {
        side_panel::with_accessible_events(hwnd, || notebook_view::rebuild(hwnd));
    }
    notebook_view::select_row(hwnd, &RowKind::Note(relative.clone())).then_some(relative)
}
```

`commit`'s match becomes:

```rust
    match purpose {
        Purpose::NewNote(parent) => commit_new_note(hwnd, how, &parent, &text),
        Purpose::NewFolder(parent) => commit_new_folder(hwnd, how, &parent, &text),
        Purpose::RenameNote(relative) => commit_rename_note(hwnd, how, &relative, &text),
        Purpose::RenameFolder(relative) => commit_rename_folder(hwnd, how, &relative, &text),
    }
```

After `commit_new_note` add:

```rust
/// A note rename (spec §4.3, §5.2): the one disk call, never over another file, then the tab
/// that has it open follows (dirty or preview alike) and the library follows it. A tab that
/// cannot follow undoes the rename. No tab is opened. After Enter the focus stays in the tree.
fn commit_rename_note(hwnd: HWND, how: How, relative: &Path, text: &str) {
    let Some(root) = library_host::folder(hwnd) else {
        cancel(hwnd);
        return;
    };
    let old = root.join(relative);
    let current = old
        .extension()
        .map(|extension| extension.to_string_lossy().into_owned());
    let Some(name) = title::renamed_note_name(text, current.as_deref()) else {
        cancelled(hwnd, how);
        return;
    };
    let new = old.with_file_name(&name);
    if new == old {
        cancelled(hwnd, how);
        return;
    }
    // A change of letter case only names the same file, so it is not a clash.
    let case_only = library::model::same_path(&new, &old);
    if let Err(error) = crate::platform::files::rename_no_replace(&old, &new) {
        let error = if !case_only && library_host::already_exists(&error) {
            let (stem, extension) = title::split_rename(text, current.as_deref());
            let parent = old.parent().map(Path::to_path_buf).unwrap_or_default();
            library_host::name_taken_error(&parent, &stem, &extension.unwrap_or_default())
        } else {
            format!("FastPad could not rename the file: {error}")
        };
        fail(hwnd, how, error);
        return;
    }
    let rebound = match library_host::rebind_open_tab(hwnd, &old, new.clone()) {
        Ok(rebound) => rebound,
        Err(()) => {
            // Undo, so the tab and the disk agree.
            let _ = crate::platform::files::rename_no_replace(&new, &old);
            fail(hwnd, how, "Another tab already has that file open.".to_owned());
            return;
        }
    };
    with_state(hwnd, |state| state.rename_note(&old, &new));
    library_host::schedule_write(hwnd);
    end(hwnd);
    let moved = library::record_path(&root, &new);
    side_panel::with_accessible_events(hwnd, || {
        side_panel::refresh(hwnd);
        notebook_view::select_row(hwnd, &RowKind::Note(moved.clone()));
    });
    if how == How::Enter {
        notebook_view::focus_tree(hwnd);
    }
    if rebound {
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

/// A folder rename (spec §4.3, §5.2): the one disk call, never onto another name, then the open
/// tabs under it follow and the library follows it. A tab that cannot follow undoes the whole
/// rename; if the undo fails, the rename stands and a notice names the tabs left on their old
/// paths. After Enter the focus stays in the tree.
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
    // A change of letter case only names the same folder, so it is not a clash.
    let case_only = library::model::same_path(&new, old);
    let (old_path, new_path) = (root.join(old), root.join(&new));
    if let Err(error) = crate::platform::files::rename_no_replace(&old_path, &new_path) {
        let error = if !case_only && library_host::already_exists(&error) {
            library_host::folder_taken_error(&name)
        } else {
            format!("FastPad could not rename the folder: {error}")
        };
        fail(hwnd, how, error);
        return;
    }
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
    let mut undo_failed = None;
    if !stuck.is_empty() {
        // Undo, so the tabs and the disk agree: the folder goes back, then the tabs that moved.
        if library_host::rename_folder_back(&new_path, &old_path).is_ok() {
            for (id, path) in moved.into_iter().rev() {
                if let Some(mut app) = unsafe { app_ptr(hwnd) } {
                    let _ = unsafe { app.as_mut() }.tabs.rebind_path(id, path);
                }
            }
            fail(hwnd, how, "Another tab already has that file open.".to_owned());
            return;
        }
        // The disk is the truth: the rename stands, the tabs that moved keep their new paths,
        // and the ones that could not follow are named.
        let old_name = old.file_name().unwrap_or_default().to_string_lossy();
        undo_failed = Some(library_host::rename_undo_failed_notice(
            &old_name, &name, &stuck,
        ));
    }
    library_host::reroot_save_folders(hwnd, &old_path, &new_path);
    with_state(hwnd, |state| state.rename_folder(old, &new));
    library_host::save_local_soon(hwnd);
    library_host::schedule_write(hwnd);
    end(hwnd);
    super::main_window::invalidate_title_strip(hwnd);
    side_panel::with_accessible_events(hwnd, || {
        side_panel::refresh(hwnd);
        notebook_view::select_row(hwnd, &RowKind::Folder(new.clone()));
    });
    if how == How::Enter {
        notebook_view::focus_tree(hwnd);
    }
    if let Some(notice) = undo_failed {
        push_notice(hwnd, notice);
    }
}
```

- [ ] **Step 4: Retire the name bar's folder rename and F2's tab**

In `src/window/library_host.rs`:
- delete `NO_FOLDER_NAME`, `folder_suffix`, `rename_folder`, `submit_rename_folder` and `rename_file`;
- make `already_exists`, `tabs_under`, `reroot_save_folders`, `rename_folder_back`, `rename_undo_failed_notice` and `rebind_open_tab` `pub(crate)`; in `rebind_open_tab`'s doc replace "and `submit_rename` rebinds its own tab itself (and undoes the rename when it cannot), so `Err` comes from a relocation the rescan finds" with "and the renames undo themselves on `Err` (`submit_rename` rebinds its own tab; `inline_name` calls this and undoes the rename), so an unexpected `Err` comes from a relocation the rescan finds";
- `name_box_submit`'s match: `FirstSave(id) => submit_first_save(…)`, `RenameNote(_) if !ready_library(hwnd) => close_name_box(hwnd)`, `RenameNote(id) => submit_rename(hwnd, id, &text)`;
- `close_stale_name_box` loses its folder branch; its doc becomes "Closes a name box whose tab is gone or no longer active, or whose first save already happened." and `let id = purpose.document();`;
- `rename_note`'s doc: "Note: Rename… on a file with no row in the Notebook tree (outside the open notebook, or not a note): the name bar, prefilled with the file's current name.";
- the module doc: "…debounced metadata writes, folder deletes, first-save naming, autosave, and pins."

In `src/window/name_box.rs`: delete `RenameFolder` and `folder()`; `document(&self) -> DocumentId` returns the id of either variant; `focus()` selects up to the last `.` with no folder case:

```rust
    /// Focuses the field with the name selected up to its extension, so typing replaces the stem.
    pub(crate) fn focus(&self) {
        let text = self.text();
        let end = text
            .rfind('.')
            .map_or(-1, |dot| text[..dot].encode_utf16().count() as isize);
        unsafe {
            SetFocus(self.edit);
            SendMessageW(self.edit, EM_SETSEL, 0, end);
        }
    }
```

Drop the now-unused `Path`/`PathBuf` import; the module doc: "…shown above the editor for a first save and for renaming a file the Notebook tree has no row for."

In `src/window/notebook_view.rs`:
- `key_down`'s F2/Del arm:

```rust
            match kind {
                Some(kind @ (RowKind::Note(_) | RowKind::Folder(_))) if key == VK_F2 => {
                    super::inline_name::rename(hwnd, &kind);
                }
                Some(RowKind::Note(relative)) if super::library_host::ready_library(hwnd) => {
                    super::library_host::delete_file(hwnd, &root.join(relative));
                }
                Some(RowKind::Folder(relative)) if super::library_host::ready_library(hwnd) => {
                    super::library_host::delete_folder(hwnd, &relative);
                }
                _ => {}
            }
```

- both menus: `Some(CommandId::NoteRename) => super::inline_name::rename(hwnd, &row.kind),` (the note menu's `if ready_library { rename_file }` and the folder menu's guarded arm go; `start` checks the library).

In `src/window/main_window.rs` `CommandId::NoteRename`:

```rust
        CommandId::NoteRename => {
            if crate::window::library_host::ready_library(hwnd) {
                match (&tree_note, &tree_folder) {
                    (Some(path), _) => crate::window::inline_name::rename_note_at(hwnd, path),
                    (None, Some(folder)) => crate::window::inline_name::rename(
                        hwnd,
                        &crate::library::tree::RowKind::Folder(folder.clone()),
                    ),
                    (None, None) => crate::window::inline_name::rename_active(hwnd),
                }
            }
        }
```

The name-bar rename tests that have no sidebar (`renaming_a_note_renames_its_file_and_keeps_its_metadata`, `renaming_onto_an_existing_file_is_refused`, `renaming_a_note_changing_only_letter_case_works`, `renaming_to_the_prefilled_name_keeps_a_non_note_or_extensionless_file_as_it_is`, `a_palette_rename_and_a_move_turn_the_preview_into_a_normal_tab`) keep passing unchanged: their windows never made a sidebar, so `reveal` finds no row and the name bar opens as before.

- [ ] **Step 5: Run the tests**

Run the command from Step 2, then `cargo test --lib -- --test-threads=1 renaming_a_note renaming_onto_an_existing renaming_to_the_prefilled a_palette_rename_and_a_move the_name_box first_save library_host::tests name_box::tests`.
Expected: PASS.

- [ ] **Step 6: Check and commit**

Run: `cargo fmt`, `cargo fmt --check`, then `cargo clippy --all-targets -- -D warnings`
Expected: clean, with the module `expect` gone and nothing dead.

```bash
git add src/window/inline_name.rs src/window/library_host.rs src/window/name_box.rs src/window/notebook_view.rs src/window/main_window.rs
git commit -m "feat(sidebar): rename notes and folders in the tree, revealing the row"
```

---

### Task 5: Focus leaving the field, clicks, and cancellation

**Files:**
- Modify: `src/window/messages.rs`, `src/window/mod.rs` (`WM_FASTPAD_INLINE_NAME_LEFT`)
- Modify: `src/window/inline_name.rs` (`field_proc`'s `WM_KILLFOCUS`, `focus_leaving`, `focus_left`, `refocus`; the accelerator test)
- Modify: `src/window/main_window.rs` (`WM_SETFOCUS`, the posted message, `translate_accelerator`; tests)
- Modify: `src/window/notebook_view.rs` (`hit_after_commit`, `left_down`, `WM_RBUTTONDOWN`)
- Modify: `src/window/side_panel.rs` (`show_view_now`, `sync_presence`)

**Interfaces:**
- Consumes (Tasks 2–4): `commit`, `cancel`, `is_open`, `owns`, `How`, `Edit::refocus`, test helpers.
- Consumes (existing): `modal::hold_while_modal(HWND, u32) -> bool` (window-module visible), `menus::accelerator_specs()`.
- Produces: `pub const WM_FASTPAD_INLINE_NAME_LEFT: u32 = WM_APP + 17`; `pub(crate) fn inline_name::{focus_left(HWND), refocus(HWND) -> bool}`; `fn main_window::inline_name_keeps_key(HWND, &MSG) -> bool`; `fn notebook_view::hit_after_commit(HWND, i32, i32) -> Option<Hit>`.

- [ ] **Step 1: Write the failing tests**

In `src/window/inline_name.rs` tests:

```rust
    #[test]
    fn only_ctrl_z_among_the_field_keys_is_an_accelerator() {
        // Break caught: an accelerator on Ctrl+A, Ctrl+C, Ctrl+X, Ctrl+V, Del, Home, End,
        // Ctrl+Left, Ctrl+Right or Ctrl+Backspace taking the key from the field, which keeps
        // only Ctrl+Z from the table (inline naming spec §5.1).
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
            VK_BACK, VK_DELETE, VK_END, VK_HOME, VK_LEFT, VK_RIGHT,
        };
        use windows_sys::Win32::UI::WindowsAndMessaging::{FCONTROL, FSHIFT};
        let specs = crate::window::menus::accelerator_specs();
        let bound = |modifiers: u8, key: u16| {
            specs
                .iter()
                .any(|spec| spec.modifiers == modifiers && spec.key == key)
        };
        let letter = |key: u8| u16::from(key);
        for (modifiers, key) in [
            (FCONTROL, letter(b'A')),
            (FCONTROL, letter(b'C')),
            (FCONTROL, letter(b'X')),
            (FCONTROL, letter(b'V')),
            (0, VK_DELETE),
            (0, VK_HOME),
            (0, VK_END),
            (FSHIFT, VK_HOME),
            (FSHIFT, VK_END),
            (FCONTROL, VK_LEFT),
            (FCONTROL, VK_RIGHT),
            (FCONTROL | FSHIFT, VK_LEFT),
            (FCONTROL | FSHIFT, VK_RIGHT),
            (FCONTROL, VK_BACK),
        ] {
            assert!(!bound(modifiers, key), "{modifiers:#x} {key:#x}");
        }
        assert!(bound(FCONTROL, letter(b'Z')), "Ctrl+Z is Undo: the field keeps it");
    }
```

In `src/window/main_window.rs` tests:

```rust
    #[test]
    fn focus_moving_to_the_editor_commits_and_a_taken_name_closes_with_a_notice() {
        // Break caught: a name lost when the user clicks into the editor, a click away that
        // leaves the field hanging, or a taken name closed without saying why (spec §5.3).
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus;
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("inline-focus-editor");
        let a = scratch.note("a.md", "a");
        let (window, editor) = notebook_window(&scratch);
        super::open_path(window.hwnd, &a).unwrap();

        crate::window::inline_name::new_folder(window.hwnd, None);
        type_into_field(window.hwnd, "Plans");
        unsafe { SetFocus(editor.hwnd()) };
        pump_posted_messages(window.hwnd);
        assert!(!inline_open(window.hwnd));
        assert!(scratch.folder().join("Plans").is_dir());

        crate::window::inline_name::new_folder(window.hwnd, None);
        type_into_field(window.hwnd, "plans");
        assert!(crate::window::inline_name::problem(window.hwnd).is_some());
        unsafe { SetFocus(editor.hwnd()) };
        pump_posted_messages(window.hwnd);
        assert!(!inline_open(window.hwnd));
        assert_eq!(draft_row(window.hwnd), None);
        assert!(
            notices(window.hwnd)
                .iter()
                .any(|notice| notice == "plans already exists here."),
            "{:?}",
            notices(window.hwnd)
        );
    }

    #[test]
    fn losing_focus_to_another_app_keeps_the_field_and_reactivation_refocuses_it() {
        // Break caught: Alt+Tab creating a half-typed note, or coming back to FastPad with the
        // field open but the caret in the editor (spec §5.3).
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetFocus, SetFocus};
        use windows_sys::Win32::UI::WindowsAndMessaging::WM_SETFOCUS;
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("inline-focus-app");
        scratch.note("a.md", "a");
        let (window, _editor) = notebook_window(&scratch);
        crate::window::inline_name::new_note(window.hwnd, None);
        type_into_field(window.hwnd, "half");

        // What deactivation does to the focused field: focus goes to no window of this thread.
        unsafe { SetFocus(std::ptr::null_mut()) };
        pump_posted_messages(window.hwnd);
        assert!(inline_open(window.hwnd));
        assert!(!scratch.folder().join("half.md").exists());

        unsafe { SendMessageW(window.hwnd, WM_SETFOCUS, 0, 0) };
        assert_eq!(unsafe { GetFocus() }, inline_field(window.hwnd));
        assert_eq!(field_text(window.hwnd), "half");
    }

    #[test]
    fn a_commit_on_focus_loss_waits_for_a_modal_prompt_to_end() {
        // Break caught: a note created or renamed while a modal prompt that took the focus is
        // still asking something (spec §5.3).
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus;
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("inline-focus-modal");
        scratch.note("a.md", "a");
        let (window, _editor) = notebook_window(&scratch);
        crate::window::inline_name::new_folder(window.hwnd, None);
        type_into_field(window.hwnd, "Later");

        let modal = crate::window::modal::ModalScope::enter(window.hwnd);
        // The prompt takes the focus: here the frame does, a window of this thread.
        unsafe { SetFocus(window.hwnd) };
        pump_posted_messages(window.hwnd);
        assert!(inline_open(window.hwnd));
        assert!(!scratch.folder().join("Later").exists(), "held while modal");

        // Leaving the outermost modal scope re-posts what it held.
        drop(modal);
        pump_posted_messages(window.hwnd);
        assert!(!inline_open(window.hwnd));
        assert!(scratch.folder().join("Later").is_dir());
    }

    #[test]
    fn a_click_on_a_row_below_a_draft_commits_first_and_acts_on_that_row() {
        // Break caught: the click selecting whatever row slid under the pointer once the empty
        // draft went, or the rename not committed before the click (spec §5.3).
        use windows_sys::Win32::UI::WindowsAndMessaging::{WM_LBUTTONDOWN, WM_LBUTTONUP};
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("inline-click");
        scratch.note("a.md", "a");
        scratch.note("b.md", "b");
        scratch.note("c.md", "c");
        let (window, _editor) = notebook_window(&scratch);
        let panel = sidebar_windows(window.hwnd).1;
        let click = |lparam| unsafe {
            SendMessageW(panel, WM_LBUTTONDOWN, 0, lparam);
            SendMessageW(panel, WM_LBUTTONUP, 0, lparam);
        };

        crate::window::inline_name::new_note(window.hwnd, None);
        assert_eq!(draft_row(window.hwnd), Some((0, 0)));
        click(row_lparam(window.hwnd, &RowKind::Note("b.md".into())));
        assert!(!inline_open(window.hwnd));
        assert_eq!(selected_kind(window.hwnd), Some(RowKind::Note("b.md".into())));
        assert_eq!(
            app_mut(window.hwnd).tabs.active().unwrap().path,
            Some(scratch.folder().join("b.md"))
        );

        crate::window::inline_name::rename(window.hwnd, &RowKind::Note("c.md".into()));
        type_into_field(window.hwnd, "d");
        click(row_lparam(window.hwnd, &RowKind::Note("a.md".into())));
        assert!(scratch.folder().join("d.md").exists(), "committed first");
        assert_eq!(selected_kind(window.hwnd), Some(RowKind::Note("a.md".into())));
    }

    #[test]
    fn starting_an_edit_while_one_is_open_commits_the_open_one_first() {
        // Break caught: a second edit dropping the first one's typing, or two fields at once
        // (spec §3.4).
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_F2;
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("inline-one-edit");
        scratch.note("a.md", "a");
        let (window, _editor) = notebook_window(&scratch);
        select_row(window.hwnd, &RowKind::Note("a.md".into()));
        assert!(crate::window::notebook_view::key_down(window.hwnd, VK_F2));
        type_into_field(window.hwnd, "a2");

        crate::window::notebook_view::header_clicked(
            window.hwnd,
            crate::window::notebook_view::HeaderButton::NewFolder,
        );

        assert!(scratch.folder().join("a2.md").exists());
        assert_eq!(
            crate::window::inline_name::purpose(window.hwnd),
            Some(crate::window::inline_name::Purpose::NewFolder(
                std::path::PathBuf::new()
            ))
        );
        assert_eq!(field_text(window.hwnd), "");
    }

    #[test]
    fn switching_the_sidebar_view_or_hiding_it_cancels_the_edit() {
        // Break caught: a field left typing into a view nobody can see, or Ctrl+B committing a
        // half-typed rename (spec §5.4).
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("inline-view-switch");
        scratch.note("a.md", "a");
        let (window, _editor) = notebook_window(&scratch);
        for view in [
            crate::config::SidebarView::Search,
            crate::config::SidebarView::Hidden,
        ] {
            crate::window::inline_name::rename(window.hwnd, &RowKind::Note("a.md".into()));
            type_into_field(window.hwnd, "zzz");
            crate::window::side_panel::show_view(window.hwnd, view, false);
            pump_posted_messages(window.hwnd);
            assert!(!inline_open(window.hwnd), "{view:?}");
            assert!(scratch.folder().join("a.md").exists(), "{view:?}");
            crate::window::side_panel::show_view(
                window.hwnd,
                crate::config::SidebarView::Notebook,
                false,
            );
        }
    }

    #[test]
    fn ctrl_z_in_the_field_undoes_the_field_not_the_editor() {
        // Break caught: Ctrl+Z in the name field undoing the note in the editor, because the
        // accelerator table takes the key first (spec §5.1).
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
            GetKeyboardState, SetKeyboardState, VK_CONTROL,
        };
        use windows_sys::Win32::UI::WindowsAndMessaging::{MSG, WM_CHAR, WM_KEYDOWN};
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("inline-ctrl-z");
        let a = scratch.note("a.md", "a");
        let (window, editor) = notebook_window(&scratch);
        super::open_path(window.hwnd, &a).unwrap();
        editor.set_text("typed in the editor").unwrap();
        let identity = unsafe { super::window_identity(window.hwnd).unwrap() };
        crate::window::inline_name::new_note(window.hwnd, None);
        let field = inline_field(window.hwnd);
        let typed = crate::platform::wide_null("abc");
        unsafe {
            SendMessageW(
                field,
                windows_sys::Win32::UI::Controls::EM_REPLACESEL,
                1,
                typed.as_ptr() as isize,
            )
        };
        let mut keys = [0u8; 256];
        unsafe { GetKeyboardState(keys.as_mut_ptr()) };
        let original = keys;
        keys[VK_CONTROL as usize] = 0x80;
        unsafe { SetKeyboardState(keys.as_ptr()) };
        let ctrl_z = MSG {
            hwnd: field,
            message: WM_KEYDOWN,
            wParam: usize::from(b'Z'),
            ..Default::default()
        };

        let taken = unsafe { super::translate_accelerator(window.hwnd, &identity, &ctrl_z) };
        unsafe {
            SendMessageW(field, WM_KEYDOWN, usize::from(b'Z'), 0);
            SendMessageW(field, WM_CHAR, 0x1a, 0);
        }
        unsafe { SetKeyboardState(original.as_ptr()) };

        assert!(!taken, "the field keeps Ctrl+Z");
        assert_eq!(field_text(window.hwnd), "");
        assert_eq!(editor.text().unwrap(), "typed in the editor");
    }
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test --lib -- --test-threads=1 only_ctrl_z_among focus_moving_to_the_editor losing_focus_to_another_app a_commit_on_focus_loss a_click_on_a_row_below starting_an_edit_while switching_the_sidebar_view ctrl_z_in_the_field`
Expected: FAIL to compile, "cannot find value `WM_FASTPAD_INLINE_NAME_LEFT`".

- [ ] **Step 3: Post, commit and refocus on focus changes**

In `src/window/messages.rs` after `WM_FASTPAD_REPLACE_RELOADED`:

```rust
/// Posted to the main window when the Notebook tree's inline name field lost the focus to
/// another window of this thread (inline naming spec §5.3).
pub const WM_FASTPAD_INLINE_NAME_LEFT: u32 = WM_APP + 17;
```

and add it to the `pub use messages::{…}` list in `src/window/mod.rs`.

In `src/window/inline_name.rs`: add `WM_KILLFOCUS, GetWindowThreadProcessId, PostMessageW` to the `WindowsAndMessaging` import and `use windows_sys::Win32::System::Threading::GetCurrentThreadId;`. In `field_proc`, before the final `DefSubclassProc`:

```rust
    if message == WM_KILLFOCUS {
        focus_leaving(main, wparam as HWND);
    }
```

After `announce` add:

```rust
/// The field is losing the focus to `to` (spec §5.3). Another window of FastPad commits the
/// edit, once the focus change is over; none (another app, or the window deactivating) keeps it
/// and arms `refocus`.
fn focus_leaving(hwnd: HWND, to: HWND) {
    let ours = !to.is_null()
        && unsafe { GetWindowThreadProcessId(to, std::ptr::null_mut()) }
            == unsafe { GetCurrentThreadId() };
    let open = with_inline(hwnd, |inline| {
        let edit = inline.edit.as_mut()?;
        if !ours {
            edit.refocus = true;
        }
        Some(())
    })
    .flatten()
    .is_some();
    if open && ours {
        unsafe { PostMessageW(hwnd, crate::window::WM_FASTPAD_INLINE_NAME_LEFT, 0, 0) };
    }
}

/// `WM_FASTPAD_INLINE_NAME_LEFT`: commits the open edit unless the focus is back in the field
/// (a menu closed, or another edit started meanwhile).
pub(crate) fn focus_left(hwnd: HWND) {
    let Some(field) = with_inline(hwnd, |inline| {
        inline.edit.as_ref().and(inline.field)
    })
    .flatten() else {
        return;
    };
    if unsafe { GetFocus() } != field {
        commit(hwnd, How::FocusLeft);
    }
}

/// The frame got the focus back (the window was activated again): the field takes it, if focus
/// left FastPad from it (spec §5.3). Returns whether it did.
pub(crate) fn refocus(hwnd: HWND) -> bool {
    let field = with_inline(hwnd, |inline| {
        let edit = inline.edit.as_mut()?;
        std::mem::take(&mut edit.refocus).then_some(inline.field).flatten()
    })
    .flatten();
    match field {
        Some(field) => {
            unsafe { SetFocus(field) };
            true
        }
        None => false,
    }
}
```

In `src/window/main_window.rs`:
- `WM_SETFOCUS` starts with:

```rust
            // The frame gets the focus when the window is activated again: an inline name edit
            // that was open when FastPad lost the focus takes it back (inline naming spec §5.3).
            if menu_mode(hwnd).is_none() && crate::window::inline_name::refocus(hwnd) {
                return 0;
            }
```

- in the `_ =>` arm, after the `WM_FASTPAD_NOTEBOOK_CHECKED` block:

```rust
            // Focus left the Notebook tree's name field (inline naming spec §5.3). A modal
            // prompt that took it holds the commit until it ends.
            if message == crate::window::WM_FASTPAD_INLINE_NAME_LEFT {
                if !crate::window::modal::hold_while_modal(hwnd, message) {
                    crate::window::inline_name::focus_left(hwnd);
                }
                return 0;
            }
```

- `translate_accelerator`: `if palette_keeps_key(hwnd, message) || inline_name_keeps_key(hwnd, message) { return false; }` (extend the comment: "…and Ctrl+Z in the Notebook tree's name field is the field's own undo (inline naming spec §5.1)."), and after `palette_keeps_key`:

```rust
/// Ctrl+Z (without Alt) aimed at the Notebook tree's inline name field.
fn inline_name_keeps_key(
    hwnd: HWND,
    message: &windows_sys::Win32::UI::WindowsAndMessaging::MSG,
) -> bool {
    message.message == WM_KEYDOWN
        && message.wParam == usize::from(b'Z')
        && unsafe { GetKeyState(VK_CONTROL as i32) } < 0
        && unsafe { GetKeyState(VK_MENU as i32) } >= 0
        && crate::window::inline_name::owns(hwnd, message.hwnd)
}
```

- [ ] **Step 4: Commit before a click, and cancel with the view**

In `src/window/notebook_view.rs`, before `left_down`:

```rust
/// What a press at `x`, `y` hits. An inline edit open when the press comes ends first, as if
/// focus had left it (inline naming spec §5.3), and a row hit follows the row it landed on to
/// wherever the commit moved it. `None` when there is no view, or that row went with the commit
/// (the draft row, or the row just renamed).
fn hit_after_commit(hwnd: HWND, x: i32, y: i32) -> Option<Hit> {
    let hit = with_view(hwnd, |view| view.hit_test(x, y))?;
    if !super::inline_name::is_open(hwnd) {
        return Some(hit);
    }
    let clicked = match hit {
        Hit::Row { index, .. } => {
            with_view(hwnd, |view| view.rows.get(index).map(|row| row.kind.clone())).flatten()
        }
        _ => None,
    };
    super::inline_name::commit(hwnd, super::inline_name::How::FocusLeft);
    match (hit, clicked) {
        (Hit::Row { part, .. }, Some(kind)) => {
            let index = with_view(hwnd, |view| tree::row_index(&view.rows, &kind)).flatten()?;
            Some(Hit::Row { index, part })
        }
        (Hit::Row { .. }, None) => None,
        (hit, _) => Some(hit),
    }
}
```

`left_down` becomes:

```rust
fn left_down(hwnd: HWND, x: i32, y: i32) {
    let hit = hit_after_commit(hwnd, x, y);
    focus_panel(hwnd);
    let Some(hit) = hit else {
        return;
    };
    match hit {
        // … the existing arms, unchanged …
    }
}
```

and `WM_RBUTTONDOWN`:

```rust
        WM_RBUTTONDOWN => {
            // Selects the row; DefWindowProc turns the button-up into WM_CONTEXTMENU.
            let (x, y) = point_of(lparam);
            let hit = hit_after_commit(hwnd, x, y);
            focus_panel(hwnd);
            if let Some(Hit::Row { index, .. }) = hit {
                with_view(hwnd, |view| view.select(index));
            }
            Some(0)
        }
```

In `src/window/side_panel.rs`, `show_view_now` starts with:

```rust
    // An inline name edit lives in the Notebook view: hiding the panel or showing another view
    // cancels it (inline naming spec §5.4).
    if view != SidebarView::Notebook {
        crate::window::inline_name::cancel(hwnd);
    }
```

and in `sync_presence`'s `else if !enabled && present` branch, first line: `crate::window::inline_name::cancel(hwnd);`.

- [ ] **Step 5: Run the tests**

Run the command from Step 2, then `cargo test --lib -- --test-threads=1 ctrl_w_closes_the_active_tab palette_commands_act_on_the_row new_folder_from_the_header_names_it f2_and_rename_on_a_note_row enter_on_a_new_note`.
Expected: PASS.

- [ ] **Step 6: Check and commit**

Run: `cargo fmt`, `cargo fmt --check`, then `cargo clippy --all-targets -- -D warnings`
Expected: clean.

```bash
git add src/window/messages.rs src/window/mod.rs src/window/inline_name.rs src/window/main_window.rs src/window/notebook_view.rs src/window/side_panel.rs
git commit -m "feat(sidebar): focus leaving the name field commits, leaving FastPad keeps it"
```

---

### Task 6: Docs, the spec's decisions, and the full suite

**Files:**
- Modify: `README.md` (the Notes and notebooks list)
- Modify: `docs/superpowers/specs/2026-09-24-inline-naming-design.md` (new §11)

**Interfaces:**
- Consumes: the finished feature.
- Produces: documentation only.

- [ ] **Step 1: README**

In `README.md`, replace the Ctrl+N bullet and the New folder bullet with:

```markdown
- **Ctrl+N** gives you a new note, labelled by its first line as you type. The first Ctrl+S asks
  for its name inline and saves it into the notebook. Notes in the notebook save themselves
  from then on.
- **+** in the Notebook view's header, or **New note here** on a folder's menu, names a new note
  right in the tree, where it will be: type `todo` and Enter makes `todo.md` and opens it. Type
  an extension such as `data.json` to pick another kind.
- **New folder** in the header, or **New folder here**, names a folder the same way. **F2**
  renames a note or a folder in place, without opening it, and **Del** sends it to the Recycle
  Bin. A name that is taken says so as you type. Empty folders show in the tree, and each note
  has a coloured icon for its type.
```

- [ ] **Step 2: The spec's §11**

Append to `docs/superpowers/specs/2026-09-24-inline-naming-design.md`:

```markdown
## 11. Decisions made while implementing

- **No row, no field.** `NoteRename` uses the name bar for any file the tree has no row for:
  outside the open notebook (§3.3), but also a non-note file inside it (`script.py`), a note not
  listed yet, or a window with no sidebar (notes mode off).
- **The Notebook view shows for every edit.** New note and New folder from the palette switch
  the sidebar to the Notebook view first, opening it if hidden, as `NoteRename` does (§3.3); a
  draft needs its row on screen.
- **An empty notebook** shows the tree while a draft is open, with the draft as its only row,
  and goes back to its empty state when the draft ends.
- **The empty state's New note button** stays `CommandId::New` (an untitled tab): §8 changes only
  the header's "+" and "New note here".
- **Wording.** The live check says `<name> already exists here.` for notes and folders alike. A
  clash found by the disk call keeps each kind's current wording: `<name> already exists. Try
  <free name>.` for a note, `A folder or file named “<name>” already exists` for a folder.
- **A folder name that cleans to nothing cancels**, like an empty note name (§4.1, §5.2):
  "Type a folder name" goes with the name box's New folder.
- **Focus.** A commit caused by focus leaving moves no focus: a new note opens without taking
  the focus from where it went, and a rename leaves it there too. Only Enter puts the focus in
  the editor (new note) or keeps it in the tree (folder, renames).
- **A click in the tree while editing** commits first, then acts on the row it landed on, found
  by what it shows: the rows may have moved (the draft gone, the renamed row sorted elsewhere).
  A click on the draft row or on the row just renamed does nothing more.
- **A modal prompt** that takes the focus from the field holds the commit until it ends.
- **Switching views wins.** Showing another view or hiding the sidebar cancels, even when the
  palette took the focus first and a focus-leave commit is still queued.
- **Keys.** The Edit control has no Ctrl+A or Ctrl+Backspace of its own: the field's hook
  handles both. Of the field's keys only Ctrl+Z is an accelerator; the field keeps it.
- **Screen readers.** The field's accessible name and its problem (as the description) are set
  through `IAccPropServices` dynamic annotation, so the system's own `Edit` proxy reports them.
- **A problem shown after Enter** stays until the text changes; Enter again with the same text
  is refused like any other problem.
```

- [ ] **Step 3: Full check**

Run: `cargo fmt --check`
Expected: clean.
Run: `cargo clippy --all-targets -- -D warnings`
Expected: clean.
Run: `cargo test -- --test-threads=1`
Expected: every test passes (lib, bins and the `tests/windows` integration targets).

- [ ] **Step 4: Commit**

```bash
git add README.md docs/superpowers/specs/2026-09-24-inline-naming-design.md
git commit -m "docs: naming notes and folders in the Notebook tree"
```
