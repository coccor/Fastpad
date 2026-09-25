# Notebook Folders Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Create, rename and delete folders from the Notebook view, show empty folders in the tree, and draw a coloured icon per note type (VS Code style).

**Architecture:**
- **Pure code:** `library::scan` also records every folder it walks (`Scan::folders`, capped at `FOLDER_LIMIT`). `LibraryState` keeps them in `folders`; `NoteTree::build` gives each a row and `remove_note` no longer prunes. New `NoteTree::{insert_folder, remove_folder, rename_folder, contains_folder}` and `LibraryState::{add_folder, rename_folder, remove_folder}` keep the folder list, the notes, the records (`Relocate` / `SetDeleted` ops), the expanded list and the tree in step, entirely in memory. Each folder change is also kept as a `FolderChange` until the next rescan starts and is replayed onto that rescan's result, so a rescan that listed the folders before the change cannot undo it. A new `window::file_icons` maps an extension to a glyph, a font and a Catppuccin colour role; `palette::FileIcons` holds each theme's six colours as compiled static data.
- **Window code:** `CommandId::NoteNewFolder = 192` (header button, folder menu, palette). The name box gains `NamePurpose::NewFolder(parent)` and `RenameFolder(folder)`. `library_host` does the one disk call per command (`std::fs::create_dir`, `platform::files::rename_no_replace`, the new `platform::files::recycle_folder`), then updates the library, rebinds or closes tabs, and refreshes the sidebar. F2, Del and the folder row's menu route to the folder variants.

**Tech Stack:** Rust 2024, `windows-sys` 0.61, GDI text drawing (Segoe MDL2 Assets glyphs and the bold UI font), `SHFileOperationW`, `MoveFileExW`, MSAA names. No new crates.

**Spec:** `docs/superpowers/specs/2026-09-24-notebook-folders-design.md` (binding). Decisions the code forced go into its new §9, written in Task 6.

**Branch:** `feat/notebook-folders` (stacked on `feat/quick-open`). One PR.

## Global Constraints

- **Latency:** nothing new runs before first paint or first input. No note-file I/O (metadata included) on the UI thread except the single user-initiated disk call per folder command that spec §4.4 allows (`create_dir`, `rename_no_replace` (plus its undo on failure), `recycle_folder`), like today's note rename and delete. Scanning stays on its worker; note counts, tab lookups, name checks and path rewrites use data already in memory; a rescan's merge replays folder changes without touching the disk.
- **App-borrow rule:** while a `&mut` from `with_view`, `library_host::with_state`, `host`/`with_host` or `app_ptr(...).as_mut()` is held, never call `SetFocus`, `SetCapture`, `CreateWindowExW`, `UpdateWindow`, `SetWindowTextW` on a child, `GetWindowTextW`, `SendMessageW` to another window, `modal::*`, or a document swap. Take the values, end the borrow, then act. Every snippet below does this (`tabs_under` snapshots, `confirmed` runs with nothing borrowed, `focus_tree` runs after `close_name_box`).
- **Tests never touch the real profile** (`%LOCALAPPDATA%\FastPad`) or the real Documents: window tests use `LibraryScratch` in `main_window.rs`'s tests with `ProductionWindow::new(make_app())` (no data dir under `cfg(test)` unless a test sets a scratch one); library tests use their module's `Scratch`. Recycle-bin tests recycle only scratch folders under `%TEMP%`, exactly as `deleting_a_note_asks_then_recycles_it_and_keeps_its_record_hidden` does today (there is no recycle hook; the real `SHFileOperationW` runs on the scratch path).
- **Test runs:** window tests need `-- --test-threads=1`. Run only the targeted tests named in each task, plus `cargo clippy --all-targets -- -D warnings`. The full suite runs once, at the final review. In a worktree, copy the `native/out` DLLs first.
- **No backward compatibility:** no shims; `NoteTree::build` changes signature and every caller is updated.
- **Commits:** conventional messages, no attribution lines.
- **Searching:** never search the whole disk (no `find /`, no recursive listing of `C:\` or `D:\`). Crate sources are in `C:\Users\korn3\.cargo\registry\src\`.
- **Style:** match the surrounding code, its comment density and test naming; every new test starts with a `// Break caught:` comment.
- **Do not modify** `assets/fastpad-icon.svg`, `assets/fastpad.ico`, `src/preview/images.rs`, `src/preview/svg.rs`.
- **Table sizes (verified against the current code):** `COMMANDS` 83 → 84, `command_palette::ENTRIES` 70 → 71. `CommandId::QuickOpen = 191` is the highest today; `CommandId::NoteNewFolder = 192`, not `needs_document`, not `is_sidebar`. Accelerators unchanged.
- **Values:** `scan::FOLDER_LIMIT = 10_000`. New folder glyph `\u{E8F4}`. Icons: `md`/`markdown` `\u{E8A5}` blue; `json` label `{}` in the bold UI font, yellow; `yaml`/`yml`/`toml`/`ini`/`cfg`/`conf` `\u{E713}` peach; `csv` `\u{E80A}` green; `xml` `\u{E943}` maroon; `txt`/`text`/`log`/anything else `\u{E8A5}` overlay2; folder `\u{E8B7}` yellow. Light uses Latte's colours, Dark Mocha's, each Catppuccin theme its own flavour; high contrast uses `muted_foreground` for every icon.
- **Wording (verbatim; quotes are U+201C/U+201D as in today's note Delete confirmation):** header tooltip and accessible name `New folder`; folder menu `New folder here`, `Rename...\tF2`, `Delete...\tDel`; palette `Notebook: New folder…` (U+2026); default name `New folder`, then `New folder 2`, `New folder 3`…; suffix `in <parent>` or `in <notebook name>`; errors `A folder or file named “<name>” already exists` and `Type a folder name`; confirmations `Move the folder “<name>” to the Recycle Bin?` and `Move “<name>” and its <n> note(s) to the Recycle Bin?` (rendered `1 note` / `<n> notes`), second line `<k> open note(s) have unsaved changes, which will be lost.` (rendered `1 open note has` / `<k> open notes have`); accessible type names `Markdown`, `JSON`, `YAML`, `TOML`, `INI`, `config`, `CSV`, `XML`, `text`, `log`, placed before `, pinned` / `, unsaved`.

## Review Focus

1. **Typed folder names that are not what they look like:** `" a/b: c?. "` (invalid characters, trailing dot and spaces) creates `ab c`; `CON` creates `CON_`; `...` and spaces only are `Type a folder name`; `.git`, `node_modules` (names the scan hides) are refused; `v1.2` keeps its `.2` and is selected in full; a name taken by a note, a folder in another case, or a non-note file is refused with the box kept open. Pinned in Task 5 (`folder_names_are_cleaned_like_stems_and_an_empty_one_is_none`, `a_typed_folder_name_is_sanitized_and_empty_or_hidden_names_are_refused`, `new_folder_here_creates_it_inside_that_folder_expanded_and_refuses_a_taken_name`) and Task 6 (`a_folder_name_box_selects_the_whole_name_even_with_a_dot`).
2. **Renames that touch open tabs or only change case:** `sub` → `Sub` on NTFS (renamed on disk, tab path and expanded entry in the new case); a folder holding the active tab, a dirty tab and the preview tab (all rebound, still dirty, still preview, nothing saved); a rebind that fails midway (every rebind undone and the folder renamed back). Pinned in Task 6 (`a_case_only_folder_rename_renames_it_on_disk_and_in_tabs_and_expansion`, `renaming_a_folder_moves_its_preview_and_dirty_tabs_without_saving_them`, `a_folder_rename_an_open_tab_cannot_follow_is_undone`, `renaming_a_folder_with_an_open_note_rebinds_the_tab_and_keeps_the_notes_pin`).
3. **Deletes that remove what the UI is pointing at:** a folder holding the only open tab (dirty) and the open New folder box's parent: the warning names the unsaved note, the tab closes with no prompt, the box closes, and the selection lands on the next row. Pinned in Task 6 (`deleting_a_folder_that_holds_the_only_tab_and_the_name_box_target_warns_and_closes_both`, `deleting_a_folder_recycles_it_closes_its_tabs_and_removes_its_rows`).
4. **Rescans racing folder commands:** a rescan landing while a New folder box is open (box and typed text kept), one that finds the box's parent gone (box closes), and a rescan that listed the folders before a rename lands after it (no old row comes back); nested empty folders and their expanded state follow a rename. Pinned in Task 3 (`a_folder_changed_while_a_rescan_ran_is_not_undone_by_its_result`, `renaming_a_folder_rewrites_its_notes_records_expanded_entries_and_tree`), Task 5 (`a_new_folder_box_survives_a_rescan_but_closes_when_its_parent_goes`) and Task 6 (`a_folder_renamed_while_a_rescan_runs_keeps_its_new_name_once_the_rescan_lands`).
5. **Edges of the folder list and its paint:** folders past `FOLDER_LIMIT` are walked (their notes counted and given rows) but not listed; each type icon keeps a visible colour on the selected, inactive-selected and hovered row in every theme (the weakest pair, Latte yellow on the Light selection, is about 1.7:1). Pinned in Task 1 (`folders_past_the_folder_limit_are_walked_but_not_listed`), Task 2 (`a_folder_the_list_left_out_still_gets_a_row_from_its_notes`) and Task 4 (`file_icon_colours_stay_visible_on_selected_and_hovered_rows`).

## File Map

| File | Responsibility | Task |
|---|---|---|
| `src/library/scan.rs`, `src/library/mod.rs`, `src/library/reconcile.rs` (test fixture) | `Scan::folders`, `FOLDER_LIMIT`, `scan_limited`; `LibraryState.folders` from load and rescans | 1 |
| `src/library/tree.rs`, `src/library/mod.rs`, `src/bin/fastpad-bench.rs`, `src/window/notebook_view.rs` (one test) | `NoteTree::build(notes, folders, pinned)`, no pruning, `insert_folder`, `remove_folder`, `rename_folder`, `contains_folder`; bench regression check | 2 |
| `src/library/mod.rs`, `src/window/library_host.rs` (one line) | `FolderChange`, `LibraryState::{folder_changes, is_folder, is_listed, notes_under, add_folder, rename_folder, remove_folder}`, replay in `merge_rescan` | 3 |
| `src/window/file_icons.rs` (new), `src/window/mod.rs`, `src/window/palette.rs`, `src/window/side_panel.rs`, `src/window/main_window.rs`, `src/window/notebook_view.rs`, `src/window/sidebar_accessibility.rs` | `file_icon`, `type_name`, `FOLDER_ICON`; `FileIcons`; `ViewPaint::icons`; icons in `draw_tree_row`; type in the row's accessible name | 4 |
| `src/window/commands.rs`, `src/window/command_palette.rs`, `src/window/name_box.rs`, `src/library/title.rs`, `src/window/library_host.rs`, `src/window/notebook_view.rs`, `src/window/main_window.rs` | `CommandId::NoteNewFolder`, palette row, `NamePurpose::NewFolder`, `title::folder_name`, `new_folder`/`submit_new_folder`, folder-aware `close_stale_name_box`, header button, "New folder here" | 5 |
| `src/platform/files.rs`, `src/window/name_box.rs`, `src/window/library_host.rs`, `src/window/notebook_view.rs`, `src/window/main_window.rs`, `README.md`, the spec (§9) | `recycle_folder`, `NamePurpose::RenameFolder`, `rename_folder`/`submit_rename_folder`, `delete_folder`, F2/Del/menu/command routing for folder rows; docs | 6 |

---

### Task 1: The scan lists its folders, and `LibraryState` keeps them

**Files:**
- Modify: `src/library/scan.rs` (module doc, `Scan`, `scan`, the directory branch of the walk, tests)
- Modify: `src/library/mod.rs` (`LibraryState`, `load`, test `bare_state`, one new test)
- Modify: `src/library/reconcile.rs` (the test `Fixture::new` literal)

**Interfaces:**
- Consumes: `scan::skip_directory(&str) -> bool` (unchanged).
- Produces:
  - `pub const FOLDER_LIMIT: usize = 10_000;`
  - `pub struct Scan { pub volume: u32, pub entries: Vec<ScanEntry>, pub folders: Vec<PathBuf>, pub truncated: bool }` — `folders` relative, spelled as on disk, root not included, walk order, at most the folder limit.
  - `pub fn scan(folder: &Path, limit: usize) -> Result<Scan>` (now `scan_limited(folder, limit, FOLDER_LIMIT)`).
  - `pub fn scan_limited(folder: &Path, limit: usize, folder_limit: usize) -> Result<Scan>`.
  - `LibraryState.folders: Vec<PathBuf>` (the scan's folders; a rescan replaces it).

**Decisions this task settles:**
- A folder is recorded when the walk decides to descend into it (same test as today: not hidden/system, not a reparse point, not `skip_directory`), before it is opened. A folder that then fails to open is still listed: it exists.
- Past `FOLDER_LIMIT` a folder is still walked; its notes count toward `NOTE_LIMIT` as today.

- [ ] **Step 0: Record the bench baseline (before any change)**

Run (PowerShell):
```powershell
$bench = Join-Path $env:TEMP "fastpad-folders-bench"
Remove-Item -Recurse -Force $bench -ErrorAction SilentlyContinue
cargo run --release --bin fastpad-bench -- library-scan $bench --count 10000
Remove-Item -Recurse -Force $bench
```
Expected: exit code 0. Write down `cold_ms`, `warm_median_ms` and `tree_build_ms`; Task 2 Step 6 compares against them.

- [ ] **Step 1: Write the failing tests**

In `src/library/scan.rs`'s `mod tests`, add to `impl Scratch` (after `fn file`):

```rust
        fn dir(&self, relative: &str) {
            std::fs::create_dir_all(self.0.join(relative)).unwrap();
        }
```

Add after `fn paths`:

```rust
    fn folders(scan: &Scan) -> Vec<String> {
        let mut folders: Vec<_> = scan
            .folders
            .iter()
            .map(|folder| folder.to_string_lossy().into_owned())
            .collect();
        folders.sort();
        folders
    }

    #[test]
    fn every_walked_folder_is_listed_and_skipped_or_hidden_folders_are_not() {
        // Break caught: an empty folder missing from the tree because only notes' parents became
        // rows, or a .git, node_modules or hidden folder showing up as an empty row.
        let scratch = Scratch::new("folders");
        scratch.dir("empty");
        scratch.dir(r"outer\inner\deepest");
        scratch.file(r"notes\a.md", "a");
        scratch.dir(r".git\objects");
        scratch.dir("node_modules");
        scratch.dir("Target");
        scratch.file(r"secret\b.md", "b");
        let secret = crate::platform::wide_null(&scratch.0.join("secret").to_string_lossy());
        unsafe {
            windows_sys::Win32::Storage::FileSystem::SetFileAttributesW(
                secret.as_ptr(),
                windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_HIDDEN,
            );
        }
        let scan = scan(&scratch.0, NOTE_LIMIT).unwrap();
        assert_eq!(
            folders(&scan),
            [
                "empty",
                "notes",
                "outer",
                r"outer\inner",
                r"outer\inner\deepest"
            ]
        );
        assert_eq!(paths(&scan), [r"notes\a.md"]);
    }

    #[test]
    fn folders_past_the_folder_limit_are_walked_but_not_listed() {
        // Break caught: the folder cap also dropping the notes of the folders past it, or a
        // notebook of a million empty folders listing them all.
        let scratch = Scratch::new("folder-limit");
        for name in ["a", "b", "c"] {
            scratch.file(&format!(r"{name}\n.md"), "x");
        }
        let scan = scan_limited(&scratch.0, NOTE_LIMIT, 2).unwrap();
        assert_eq!(scan.folders.len(), 2);
        assert_eq!(paths(&scan), [r"a\n.md", r"b\n.md", r"c\n.md"]);
        assert!(!scan.truncated);
        assert_eq!(FOLDER_LIMIT, 10_000);
    }
```

In `reparse_points_are_not_followed`, after `assert_eq!(paths(&scan), ["a.md"]);` add:

```rust
        assert!(scan.folders.is_empty(), "a junction is not a folder row");
```

In `src/library/mod.rs`'s `mod tests`, add after `the_index_follows_saves_renames_and_deletes`:

```rust
    #[test]
    fn the_state_lists_the_scans_folders_and_a_rescan_replaces_them() {
        // Break caught: an empty folder known only until the first rescan, or a folder deleted in
        // Explorer kept alive by the merge.
        let scratch = Scratch::new("folder-list");
        let folder = scratch.folder();
        std::fs::create_dir_all(folder.join("empty")).unwrap();
        std::fs::create_dir_all(folder.join("sub")).unwrap();
        std::fs::write(folder.join(r"sub\a.md"), "a").unwrap();
        let sorted = |state: &LibraryState| {
            let mut folders = state.folders.clone();
            folders.sort();
            folders
        };
        let previous = load(&folder, &scratch.local(), 100).unwrap();
        assert_eq!(sorted(&previous), ["empty", "sub"].map(PathBuf::from));
        std::fs::remove_dir(folder.join("empty")).unwrap();
        std::fs::create_dir(folder.join("later")).unwrap();
        let fresh = load(&folder, &scratch.local(), 101).unwrap();
        let merged = merge_rescan(previous, fresh);
        assert_eq!(sorted(&merged), ["later", "sub"].map(PathBuf::from));
    }
```

- [ ] **Step 2: Run the tests and see them fail**

Run: `cargo test --lib library::scan`
Expected: FAIL to compile: no field `folders` on `Scan`, cannot find `scan_limited` / `FOLDER_LIMIT`.

- [ ] **Step 3: Implement**

In `src/library/scan.rs`, replace the module doc's first line pair:

```rust
//! Walks a folder for notes. Each directory is listed with `FileIdBothDirectoryInfo` queries,
//! which return names, sizes, write times and file IDs in bulk without opening any file.
```

with:

```rust
//! Walks a folder for notes, and lists every folder it walks. Each directory is listed with
//! `FileIdBothDirectoryInfo` queries, which return names, sizes, write times and file IDs in
//! bulk without opening any file.
```

Replace `pub const NOTE_LIMIT: usize = 10_000;` with:

```rust
pub const NOTE_LIMIT: usize = 10_000;
/// At most this many folders are listed in `Scan::folders`. Past it folders are still walked:
/// their notes count toward `NOTE_LIMIT`, and the tree gives each note's folder a row anyway.
pub const FOLDER_LIMIT: usize = 10_000;
```

Replace the `Scan` struct with:

```rust
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Scan {
    pub volume: u32,
    pub entries: Vec<ScanEntry>,
    /// Every folder walked, relative to the scanned folder and spelled as on disk, the root not
    /// included: at most `FOLDER_LIMIT` of them, in walk order.
    pub folders: Vec<PathBuf>,
    pub truncated: bool,
}
```

Replace the line `pub fn scan(folder: &Path, limit: usize) -> Result<Scan> {` with:

```rust
pub fn scan(folder: &Path, limit: usize) -> Result<Scan> {
    scan_limited(folder, limit, FOLDER_LIMIT)
}

/// `scan` with its own folder cap: at most `folder_limit` folders are listed.
pub fn scan_limited(folder: &Path, limit: usize, folder_limit: usize) -> Result<Scan> {
```

In the walk, replace:

```rust
                    if attributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
                        if attributes & FILE_ATTRIBUTE_REPARSE_POINT == 0 && !skip_directory(&name)
                        {
                            pending.push((path, None));
                        }
```

with:

```rust
                    if attributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
                        if attributes & FILE_ATTRIBUTE_REPARSE_POINT == 0 && !skip_directory(&name)
                        {
                            // Listed as it is walked, so an empty folder still gets a row.
                            if scan.folders.len() < folder_limit {
                                scan.folders.push(path.clone());
                            }
                            pending.push((path, None));
                        }
```

In `src/library/mod.rs`, in `pub struct LibraryState`, after `pub notes: Vec<NoteEntry>,` add:

```rust
    /// Every folder the scan walked, relative and spelled as on disk, the root not included
    /// (notebook folders spec §3.1). A rescan replaces it; FastPad's own folder commands update
    /// it in place.
    pub folders: Vec<PathBuf>,
```

In `load`, in the `Ok(LibraryState { ... })` literal, replace `        truncated: scan.truncated,` with:

```rust
        truncated: scan.truncated,
        folders: scan.folders,
```

In the test helper `bare_state`, after `            notes,` add `            folders: Vec::new(),`.

In `src/library/reconcile.rs`, in `Fixture::new`, replace:

```rust
                scan: Scan {
                    volume: VOLUME,
                    entries,
                    truncated: false,
                },
```

with:

```rust
                scan: Scan {
                    volume: VOLUME,
                    entries,
                    folders: Vec::new(),
                    truncated: false,
                },
```

- [ ] **Step 4: Run the tests and see them pass**

Run: `cargo test --lib library::scan`
Expected: all pass (6 tests).
Run: `cargo test --lib library::tests::the_state_lists_the_scans_folders_and_a_rescan_replaces_them library::reconcile`
Expected: all pass.

- [ ] **Step 5: Clippy**

Run: `cargo fmt; cargo clippy --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 6: Commit**

```bash
git add src/library/scan.rs src/library/mod.rs src/library/reconcile.rs
git commit -m "feat(library): the scan lists every folder it walks (Scan::folders, FOLDER_LIMIT 10,000) and LibraryState keeps them from load and rescans"
```

---

### Task 2: Folder rows in the tree

**Files:**
- Modify: `src/library/tree.rs` (module doc, `Folder`, `NoteTree::build`, `remove_note`, new methods and helpers, tests)
- Modify: `src/library/mod.rs` (`load`'s tree build, test helpers `bare_state` and `rebuilt_rows`, one assertion)
- Modify: `src/bin/fastpad-bench.rs` (two `NoteTree::build` calls)
- Modify: `src/window/notebook_view.rs` (the test `a_thousand_expanded_folders_of_ten_notes_flatten_within_a_frame`)

**Interfaces:**
- Consumes: `LibraryState.folders` (Task 1).
- Produces:
  - `pub fn build<'a, P>(notes: impl IntoIterator<Item = &'a P>, folders: &[PathBuf], pinned: &[PathBuf]) -> NoteTree where P: AsRef<Path> + ?Sized + 'a`
  - `pub fn insert_folder(&mut self, path: &Path)` — adds missing ancestors; an existing folder keeps its spelling.
  - `pub fn remove_folder(&mut self, path: &Path)` — the folder and everything under it.
  - `pub fn rename_folder(&mut self, old: &Path, new: &Path)` — moves the subtree with its notes and pins; `old` missing ⇒ `new` is inserted; a folder already at `new` ⇒ the two merge.
  - `pub fn contains_folder(&self, path: &Path) -> bool`
  - `remove_note` no longer removes emptied folders.

**Decisions this task settles:**
- Listed folders enter `build`'s arena before the notes, so a folder keeps the spelling on disk even when a note's path spells it in another case.
- Every walk over a subtree (`note_count`, `entries`, `drop_flat`) is iterative, like the tree's own `Drop`, so a 2,000-deep chain cannot overflow the UI thread's stack.

- [ ] **Step 1: Write the failing tests**

In `src/library/tree.rs`'s `mod tests`, replace the helper `fn build(notes: &[&str], pinned: &[&str]) -> NoteTree { ... }` with:

```rust
    fn build_with(notes: &[&str], folders: &[&str], pinned: &[&str]) -> NoteTree {
        let notes: Vec<PathBuf> = notes.iter().map(PathBuf::from).collect();
        let folders: Vec<PathBuf> = folders.iter().map(PathBuf::from).collect();
        let pinned: Vec<PathBuf> = pinned.iter().map(PathBuf::from).collect();
        NoteTree::build(&notes, &folders, &pinned)
    }

    fn build(notes: &[&str], pinned: &[&str]) -> NoteTree {
        build_with(notes, &[], pinned)
    }
```

Replace the whole test `removing_the_last_note_in_a_folder_chain_removes_the_chain` with:

```rust
    #[test]
    fn removing_the_last_note_in_a_folder_chain_keeps_the_chain() {
        // Break caught: a folder row vanishing when its last note is deleted or moved while the
        // folder is still on disk (spec §3.2).
        let mut tree = build(&[r"a\b\c\n.md", r"a\keep.md", "top.md"], &[]);
        tree.remove_note(Path::new(r"a\b\c\n.md"));
        assert_eq!(
            outline(&tree.rows(&all, &[])),
            ["a/", "  b/", "    c/", "  keep", "top"]
        );
        tree.remove_note(Path::new(r"a\keep.md"));
        assert_eq!(
            outline(&tree.rows(&all, &[])),
            ["a/", "  b/", "    c/", "top"]
        );
        tree.remove_note(Path::new("missing.md"));
        assert_eq!(tree.note_count(), 1);
    }

    #[test]
    fn listed_folders_get_rows_even_empty_and_still_sort_first() {
        // Break caught: an empty folder with no row, a listed folder sorting among the notes, or
        // a note's own spelling of its folder replacing the one on disk.
        let tree = build_with(
            &["b.md", r"notes\a.md", r"SUB\x.md"],
            &["empty", "Zeta", r"notes\inner", "Sub"],
            &[],
        );
        assert_eq!(
            outline(&tree.rows(&all, &[])),
            [
                "empty/", "notes/", "  inner/", "  a", "Sub/", "  x", "Zeta/", "b"
            ]
        );
        assert_eq!(tree.note_count(), 3);
        assert!(tree.contains_folder(Path::new(r"NOTES\Inner")));
        assert!(!tree.contains_folder(Path::new("b.md")));
        assert!(!tree.contains_folder(Path::new("")));
    }

    #[test]
    fn a_folder_the_list_left_out_still_gets_a_row_from_its_notes() {
        // Break caught: a folder past the scan's folder cap hiding the notes inside it.
        let tree = build_with(&[r"c\n.md"], &["a", "b"], &[]);
        assert_eq!(outline(&tree.rows(&all, &[])), ["a/", "b/", "c/", "  n"]);
    }

    #[test]
    fn insert_folder_adds_its_missing_ancestors_and_keeps_an_existing_spelling() {
        // Break caught: a new folder inside a chain with no parent row, a second row for a folder
        // typed in another case, or an absolute path making a row.
        let mut tree = build(&["top.md"], &[]);
        tree.insert_folder(Path::new(r"x\y\z"));
        assert_eq!(
            outline(&tree.rows(&all, &[])),
            ["x/", "  y/", "    z/", "top"]
        );
        tree.insert_folder(Path::new(r"X\Y"));
        for bad in [r"C:\abs", "", r"..\up", r"\rooted"] {
            tree.insert_folder(Path::new(bad));
        }
        assert_eq!(
            outline(&tree.rows(&all, &[])),
            ["x/", "  y/", "    z/", "top"]
        );
        assert_eq!(tree.note_count(), 1);
    }

    #[test]
    fn remove_folder_takes_its_whole_subtree_and_its_notes_count() {
        // Break caught: a deleted folder leaving its subfolders or notes behind, or a note count
        // that still includes them.
        let mut tree = build_with(
            &[r"a\one.md", r"a\b\two.md", r"a\b\c\three.md", "top.md"],
            &[r"a\b\empty"],
            &[r"a\b\two.md"],
        );
        tree.remove_folder(Path::new(r"A\B"));
        assert_eq!(outline(&tree.rows(&all, &[])), ["a/", "  one", "top"]);
        assert_eq!(tree.note_count(), 2);
        tree.remove_folder(Path::new("missing"));
        tree.remove_folder(Path::new("top.md"));
        assert_eq!(tree.note_count(), 2);
        tree.remove_folder(Path::new("a"));
        assert_eq!(outline(&tree.rows(&all, &[])), ["top"]);
        assert_eq!(tree.note_count(), 1);
    }

    #[test]
    fn rename_folder_moves_the_subtree_with_its_notes_and_pins() {
        // Break caught: a renamed folder losing its notes, their pins or its empty subfolders, a
        // case-only rename leaving the old spelling, or a replayed rename losing the folder.
        let mut tree = build_with(
            &[r"work\plan.md", r"work\sub\deep.md", "top.md"],
            &[r"work\empty\deeper"],
            &[r"work\plan.md"],
        );
        tree.rename_folder(Path::new("work"), Path::new("Archive"));
        assert_eq!(
            outline(&tree.rows(&all, &[])),
            [
                "Archive/",
                "  plan*",
                "  empty/",
                "    deeper/",
                "  sub/",
                "    deep",
                "top"
            ]
        );
        assert_eq!(tree.note_count(), 3);
        let rows = tree.rows(&all, &[]);
        assert!(
            row_index(
                &rows,
                &RowKind::Note(PathBuf::from(r"Archive\sub\deep.md"))
            )
            .is_some()
        );
        assert!(!tree.contains_folder(Path::new("work")));

        tree.rename_folder(Path::new("archive"), Path::new("ARCHIVE"));
        assert_eq!(tree.rows(&all, &[])[0].name, "ARCHIVE");
        assert_eq!(tree.note_count(), 3);

        tree.rename_folder(Path::new("gone"), Path::new("Made"));
        assert!(
            tree.contains_folder(Path::new("Made")),
            "a rescan that already saw the rename"
        );

        let mut merged = build_with(&[r"a\x.md", r"b\y.md"], &[], &[r"a\x.md"]);
        merged.rename_folder(Path::new("a"), Path::new("b"));
        assert_eq!(outline(&merged.rows(&all, &[])), ["b/", "  x*", "  y"]);
        assert_eq!(merged.note_count(), 2);
    }
```

In `incremental_updates_match_a_full_rebuild`:
- replace its `// Break caught:` comment lines with:

```rust
        // Break caught: an insert, removal, rename or pin change leaving the tree in an order, or
        // without a folder a note once made, that a fresh build of the same notes and folders
        // would not have.
```

- after `let mut model: Vec<(PathBuf, bool)> = Vec::new();` add `let mut folders: Vec<PathBuf> = Vec::new();`
- in the `0 => { ... }` arm, after `tree.insert_note(&path, pinned);` add `folders.extend(ancestors(&path));`
- in the `_ => { ... }` arm, replace

```rust
                    if let Some(entry) = model.iter_mut().find(|(existing, _)| existing == &path) {
                        entry.0 = target;
                    }
```

with

```rust
                    if let Some(entry) = model.iter_mut().find(|(existing, _)| existing == &path) {
                        folders.extend(ancestors(&target));
                        entry.0 = target;
                    }
```

- replace `                NoteTree::build(&notes, &pinned).rows(&all, &[]),` with `                NoteTree::build(&notes, &folders, &pinned).rows(&all, &[]),`.

In `a_very_deep_folder_chain_builds_flattens_and_empties_without_recursion`, replace:

```rust
        tree.remove_note(&note);
        assert!(tree.rows(&all, &[]).is_empty());
```

with:

```rust
        tree.remove_note(&note);
        assert_eq!(tree.rows(&all, &[]).len(), depth, "the emptied folders stay");
        tree.remove_folder(Path::new("d0"));
        assert!(tree.rows(&all, &[]).is_empty());
```

Replace every remaining `NoteTree::build(<a>, <b>)` call in `tree.rs`'s tests (in `ten_thousand_notes_in_one_folder_flatten_quickly`, `a_very_deep_folder_chain_...` (twice), `names_stay_right_when_removals_compact_their_store` (twice), `a_name_no_file_can_have_is_refused_without_leaving_its_folder`) with `NoteTree::build(<a>, &[], <b>)`.

In `src/library/mod.rs`'s tests, replace `rebuilt_rows` with:

```rust
    /// What a fresh build of the state's notes, folders and pins shows.
    fn rebuilt_rows(state: &LibraryState) -> Vec<tree::TreeRow> {
        let paths: Vec<PathBuf> = state.notes.iter().map(|note| note.path.clone()).collect();
        tree_rows(&tree::NoteTree::build(
            &paths,
            &state.folders,
            &pinned_paths(&state.library),
        ))
    }
```

and at the end of `the_tree_follows_the_index_and_the_pins` replace:

```rust
        assert!(
            !tree_rows(&state.tree)
                .iter()
                .any(|row| matches!(row.kind, tree::RowKind::Folder(_))),
            "an emptied folder goes"
        );
```

with:

```rust
        assert!(
            tree_rows(&state.tree)
                .iter()
                .any(|row| row.kind == tree::RowKind::Folder(PathBuf::from("sub"))),
            "an emptied folder stays while it is on disk"
        );
```

In `bare_state`, replace `            tree: tree::NoteTree::build(&paths, &[]),` with `            tree: tree::NoteTree::build(&paths, &[], &[]),`.

In `src/window/notebook_view.rs`'s test `a_thousand_expanded_folders_of_ten_notes_flatten_within_a_frame`, replace `        let tree = NoteTree::build(&notes, &[]);` with `        let tree = NoteTree::build(&notes, &[], &[]);`.

- [ ] **Step 2: Run the tests and see them fail**

Run: `cargo test --lib library::tree`
Expected: FAIL to compile: `NoteTree::build` takes 2 arguments, no method `insert_folder` / `remove_folder` / `rename_folder` / `contains_folder`.

- [ ] **Step 3: Implement**

In `src/library/tree.rs`, replace the module doc's sentence

```rust
//! exact name. A folder exists only while it holds a note at some depth. Paths are relative to
```

with

```rust
//! exact name. Every folder the scan listed has a row, empty or not, and so does every folder a
//! note is in; a folder leaves only through `remove_folder`, `rename_folder` or a rebuild
//! without it (notebook folders spec §3.2). Paths are relative to
```

Delete `Folder::is_empty` (its only caller, the pruning loop, goes below):

```rust
    fn is_empty(&self) -> bool {
        self.folders.is_empty() && self.notes.is_empty()
    }

```

Add to `impl Folder` (after `fn pinned_count`):

```rust
    /// The notes in this folder and every folder under it.
    fn note_count(&self) -> usize {
        let mut count = 0;
        let mut pending = vec![self];
        while let Some(folder) = pending.pop() {
            count += folder.notes.len();
            pending.extend(folder.folders.iter());
        }
        count
    }

    /// Every note (with its pin) and every folder under this one, as paths relative to it.
    fn entries(&self) -> Vec<(PathBuf, Option<bool>)> {
        let mut entries = Vec::new();
        let mut pending = vec![(PathBuf::new(), self)];
        while let Some((path, folder)) = pending.pop() {
            for note in &folder.notes {
                entries.push((path.join(file_name(&folder.names, note)), Some(note.pinned)));
            }
            for child in &folder.folders {
                let child_path = path.join(&child.name);
                entries.push((child_path.clone(), None));
                pending.push((child_path, child));
            }
        }
        entries
    }
```

After `fn split_path`, add:

```rust
/// The names of a plain relative folder path; `None` for anything else (absolute, rooted, `.`,
/// `..` or empty).
fn folder_parts(path: &Path) -> Option<Vec<&OsStr>> {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => parts.push(part),
            _ => return None,
        }
    }
    (!parts.is_empty()).then_some(parts)
}

/// The arena index of the folder at `parts`, adding it and any missing ancestor. `key` is
/// scratch space, reused so a whole build allocates one key buffer.
fn arena_folder(
    arena: &mut Vec<Folder>,
    parents: &mut Vec<usize>,
    by_key: &mut HashMap<String, usize>,
    key: &mut String,
    parts: &[&OsStr],
) -> usize {
    let mut current = 0;
    key.clear();
    for part in parts {
        if !key.is_empty() {
            key.push('\\');
        }
        key.push_str(&part.to_string_lossy().to_lowercase());
        current = match by_key.get(key.as_str()) {
            Some(&index) => index,
            None => {
                arena.push(Folder {
                    name: part.to_os_string(),
                    ..Folder::default()
                });
                parents.push(current);
                by_key.insert(key.clone(), arena.len() - 1);
                arena.len() - 1
            }
        };
    }
    current
}

/// Drops a folder taken out of the tree one level at a time, as `NoteTree`'s own drop does.
fn drop_flat(folder: Folder) {
    let mut pending = vec![folder];
    while let Some(mut folder) = pending.pop() {
        pending.append(&mut folder.folders);
    }
}
```

Replace `build`'s doc comment, signature and body up to (not including) the line `        // Every folder comes after its parent in the arena, so taking them from the end nests` with:

```rust
    /// The tree of `notes` and `folders`, with the notes in `pinned` pinned. All are relative to
    /// the notebook; a path that is not a plain relative path is skipped, and a second spelling
    /// of one note (another letter case) is dropped. Every listed folder gets a row, even an
    /// empty one, and so does every folder a note is in. `notes` is only read: the library
    /// builds its tree straight from its own note list.
    pub fn build<'a, P>(
        notes: impl IntoIterator<Item = &'a P>,
        folders: &[PathBuf],
        pinned: &[PathBuf],
    ) -> NoteTree
    where
        P: AsRef<Path> + ?Sized + 'a,
    {
        let pinned: HashSet<String> = pinned.iter().map(|path| path_key(path)).collect();
        // Folders live in an arena keyed by lower-case relative path, so each note finds its
        // folder in one lookup however many folders there are; they are nested once, at the end.
        let mut arena = vec![Folder::default()];
        let mut parents = vec![0_usize];
        let mut by_key: HashMap<String, usize> = HashMap::new();
        let mut count = 0;
        let mut folder_key = String::new();
        // The listed folders go in first, so each keeps the spelling the scan saw on disk.
        for path in folders {
            if let Some(parts) = folder_parts(path) {
                arena_folder(&mut arena, &mut parents, &mut by_key, &mut folder_key, &parts);
            }
        }
        for path in notes {
            let path = path.as_ref();
            let Some((parts, file_name)) = split_path(path) else {
                continue;
            };
            let is_pinned = !pinned.is_empty() && pinned.contains(&path_key(path));
            let current =
                arena_folder(&mut arena, &mut parents, &mut by_key, &mut folder_key, &parts);
            let folder = &mut arena[current];
            if let Some(note) = folder.push_name(file_name, is_pinned) {
                folder.notes.push(note);
                count += 1;
            }
        }
```

Replace `remove_note` (doc comment and body) with:

```rust
    /// Removes a note. Its folder keeps its row even when it now holds nothing: folders leave
    /// only through `remove_folder`, `rename_folder` or a rebuild without them. Unknown paths
    /// do nothing.
    pub fn remove_note(&mut self, path: &Path) {
        let Some((folders, file_name)) = split_path(path) else {
            return;
        };
        let Some(trail) = self.trail(&folders) else {
            return;
        };
        let folder = self.folder_at_mut(&trail);
        let Some(index) = folder.find_note(&file_name.to_string_lossy()) else {
            return;
        };
        folder.remove_note_at(index);
        self.count -= 1;
    }

    /// Whether the tree has a folder at `path`, ignoring case.
    pub fn contains_folder(&self, path: &Path) -> bool {
        folder_parts(path).is_some_and(|parts| self.trail(&parts).is_some())
    }

    /// Adds a folder row at `path`, with any missing ancestor, holding nothing. A folder already
    /// there in any letter case keeps its spelling; a path that is not a plain relative name
    /// does nothing.
    pub fn insert_folder(&mut self, path: &Path) {
        let Some(parts) = folder_parts(path) else {
            return;
        };
        parts
            .into_iter()
            .fold(&mut self.root, |folder, part| folder.child_or_insert(part));
    }

    /// Removes the folder at `path` and everything under it. Unknown paths do nothing.
    pub fn remove_folder(&mut self, path: &Path) {
        let Some(parts) = folder_parts(path) else {
            return;
        };
        let Some(mut trail) = self.trail(&parts) else {
            return;
        };
        let Some(index) = trail.pop() else {
            return;
        };
        let removed = self.folder_at_mut(&trail).folders.remove(index);
        self.count -= removed.note_count();
        drop_flat(removed);
    }

    /// Moves the folder at `old`, with everything under it, to `new`, keeping its notes and
    /// their pins; a change of letter case renames it in place. When `old` is not in the tree,
    /// `new` is added: a rescan's result that already saw the rename gets it replayed. When a
    /// folder is already at `new`, what `old` held merges into it.
    pub fn rename_folder(&mut self, old: &Path, new: &Path) {
        let (Some(old_parts), Some(new_parts)) = (folder_parts(old), folder_parts(new)) else {
            return;
        };
        let Some((&name, parents)) = new_parts.split_last() else {
            return;
        };
        let Some(mut trail) = self.trail(&old_parts) else {
            self.insert_folder(new);
            return;
        };
        let Some(index) = trail.pop() else {
            return;
        };
        let mut moved = self.folder_at_mut(&trail).folders.remove(index);
        let mut parent = &mut self.root;
        for part in parents {
            parent = parent.child_or_insert(part);
        }
        if parent.find_folder(&name.to_string_lossy()).is_none() {
            moved.name = name.to_os_string();
            let index = parent
                .folders
                .partition_point(|existing| folder_order(existing, &moved) == Ordering::Less);
            parent.folders.insert(index, moved);
            return;
        }
        self.count -= moved.note_count();
        let entries = moved.entries();
        drop_flat(moved);
        for (path, pinned) in entries {
            let path = new.join(path);
            match pinned {
                Some(pinned) => self.insert_note(&path, pinned),
                None => self.insert_folder(&path),
            }
        }
    }
```

In `src/library/mod.rs`'s `load`, replace:

```rust
    let tree = tree::NoteTree::build(
        notes.iter().map(|note| note.path.as_path()),
        &pinned_paths(&library),
    );
```

with:

```rust
    let tree = tree::NoteTree::build(
        notes.iter().map(|note| note.path.as_path()),
        &scan.folders,
        &pinned_paths(&library),
    );
```

In `src/bin/fastpad-bench.rs`, replace the two calls `fastpad::library::tree::NoteTree::build(&paths, &pinned)` with `fastpad::library::tree::NoteTree::build(&paths, &state.folders, &pinned)`.

- [ ] **Step 4: Run the tests and see them pass**

Run: `cargo test --lib library::tree`
Expected: all pass.
Run: `cargo test --lib library::tests window::notebook_view`
Expected: all pass.

- [ ] **Step 5: Clippy**

Run: `cargo fmt; cargo clippy --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 6: Bench**

Run the Task 1 Step 0 commands again.
Expected: `cold_ms`, `warm_median_ms` and `tree_build_ms` each at most 110% of the Step 0 baseline (spec §7). If one is over, run it twice more and compare the lowest; a real regression is fixed before committing.

- [ ] **Step 7: Commit**

```bash
git add src/library/tree.rs src/library/mod.rs src/bin/fastpad-bench.rs src/window/notebook_view.rs
git commit -m "feat(library): the note tree gives every listed folder a row, keeps emptied folders, and gains insert_folder, remove_folder and rename_folder"
```

---

### Task 3: `LibraryState` folder operations, replayed onto a rescan

**Files:**
- Modify: `src/library/mod.rs` (`FolderChange`, `at_or_under`, `reroot`, `LibraryState` field and methods, `merge_rescan`, `load`, tests)
- Modify: `src/window/library_host.rs` (`spawn_load` clears `folder_changes`)

**Interfaces:**
- Consumes: `NoteTree::{insert_folder, remove_folder, rename_folder, contains_folder}` (Task 2), `LibraryState::apply`, `LocalState::set_missing`.
- Produces:
  - `#[derive(Clone, Debug, Eq, PartialEq)] pub enum FolderChange { Added(PathBuf), Removed(PathBuf), Renamed { old: PathBuf, new: PathBuf } }`
  - `LibraryState.folder_changes: Vec<FolderChange>`
  - `pub fn is_folder(&self, relative: &Path) -> bool`
  - `pub fn is_listed(&self, relative: &Path) -> bool` — a tree folder or a listed note at that path, ignoring case.
  - `pub fn notes_under(&self, relative: &Path) -> usize`
  - `pub fn add_folder(&mut self, relative: &Path)`
  - `pub fn rename_folder(&mut self, old: &Path, new: &Path)` — `Relocate` per live record under `old`, expanded and touched entries rewritten.
  - `pub fn remove_folder(&mut self, relative: &Path, now: u64)` — `SetDeleted` + `local.set_missing(id, now)` per live record under it, expanded and touched entries dropped.

**Decisions this task settles:**
- `touched` entries under a renamed folder are rewritten (a note saved there before the rename is re-checked at its new path), and entries under a removed folder are dropped, so the merge makes no extra stat for them.
- The merge replays `folder_changes` onto the rescan's folders, notes and tree *before* `merge_notes`, in order. The replay is idempotent: a rescan that already saw the change is left as it was. Records are not replayed here (the pending `Relocate`/`SetDeleted` ops already are), and the expanded list is the live one.

- [ ] **Step 1: Write the failing tests**

In `src/library/mod.rs`'s `mod tests`, add after `the_state_lists_the_scans_folders_and_a_rescan_replaces_them`:

```rust
    fn sorted_folders(state: &LibraryState) -> Vec<PathBuf> {
        let mut folders = state.folders.clone();
        folders.sort();
        folders
    }

    fn sorted_notes(state: &LibraryState) -> Vec<PathBuf> {
        let mut notes: Vec<PathBuf> = state.notes.iter().map(|note| note.path.clone()).collect();
        notes.sort();
        notes
    }

    #[test]
    fn renaming_a_folder_rewrites_its_notes_records_expanded_entries_and_tree() {
        // Break caught: a folder rename leaving notes, pins or expanded folders at the old path,
        // so rows open files that are gone, pins vanish, or the renamed folder and its nested
        // empty folders collapse.
        let scratch = Scratch::new("folder-rename");
        let folder = scratch.folder();
        std::fs::create_dir_all(folder.join(r"work\empty\deeper")).unwrap();
        std::fs::write(folder.join(r"work\plan.md"), "p").unwrap();
        std::fs::write(folder.join("top.md"), "t").unwrap();
        let mut ids = IdSource::new(1, 2);
        let mut state = load(&folder, &scratch.local(), 100).unwrap();
        let target = state.note_ref(&mut ids, &folder.join(r"work\plan.md"));
        state
            .apply(PendingOp::SetPinned {
                note: target,
                value: true,
            })
            .unwrap();
        state.local.set_expanded(Path::new("work"), true);
        state.local.set_expanded(Path::new(r"work\empty"), true);
        state.local.set_expanded(Path::new("other"), true);
        std::fs::rename(folder.join("work"), folder.join("Archive")).unwrap();

        state.rename_folder(Path::new("work"), Path::new("Archive"));

        assert_eq!(
            sorted_notes(&state),
            [PathBuf::from(r"Archive\plan.md"), PathBuf::from("top.md")]
        );
        assert!(state.is_pinned(Path::new(r"Archive\plan.md")), "the record moved");
        assert!(!state.is_pinned(Path::new(r"work\plan.md")));
        assert_eq!(
            state.local.expanded,
            [
                PathBuf::from("Archive"),
                PathBuf::from(r"Archive\empty"),
                PathBuf::from("other")
            ]
        );
        assert_eq!(
            sorted_folders(&state),
            ["Archive", r"Archive\empty", r"Archive\empty\deeper"].map(PathBuf::from)
        );
        assert!(state.is_folder(Path::new(r"archive\EMPTY\deeper")));
        assert!(!state.is_folder(Path::new("work")));
        assert_eq!(tree_rows(&state.tree), rebuilt_rows(&state));
        assert!(state.pending.iter().any(|op| matches!(
            op,
            PendingOp::Relocate { path, .. } if path == Path::new(r"Archive\plan.md")
        )));
        assert_eq!(flush(&mut state).unwrap(), Flushed::Wrote);
        let reloaded = load(&folder, &scratch.local(), 101).unwrap();
        assert!(reloaded.is_pinned(Path::new(r"Archive\plan.md")));
    }

    #[test]
    fn removing_a_folder_drops_its_notes_marks_their_records_deleted_and_forgets_its_expansion() {
        // Break caught: a deleted folder's notes still listed (rows that open nothing), its
        // pinned note's record looking alive, or its expanded entries left in the local file.
        let scratch = Scratch::new("folder-remove");
        let folder = scratch.folder();
        std::fs::create_dir_all(folder.join(r"old\inner")).unwrap();
        std::fs::write(folder.join(r"old\a.md"), "a").unwrap();
        std::fs::write(folder.join(r"old\inner\b.md"), "b").unwrap();
        std::fs::write(folder.join("keep.md"), "k").unwrap();
        let mut ids = IdSource::new(1, 2);
        let mut state = load(&folder, &scratch.local(), 100).unwrap();
        let target = state.note_ref(&mut ids, &folder.join(r"old\a.md"));
        state
            .apply(PendingOp::SetPinned {
                note: target,
                value: true,
            })
            .unwrap();
        state.local.set_expanded(Path::new("old"), true);
        state.local.set_expanded(Path::new(r"old\inner"), true);
        assert_eq!(state.notes_under(Path::new("OLD")), 2);
        assert!(state.is_listed(Path::new("keep.md")));
        assert!(state.is_listed(Path::new(r"Old\Inner")));
        assert!(!state.is_listed(Path::new("new")));

        state.remove_folder(Path::new("old"), 500);

        assert_eq!(sorted_notes(&state), [PathBuf::from("keep.md")]);
        let record = state.library.note_by_path(Path::new(r"old\a.md")).unwrap();
        assert!(record.deleted);
        assert_eq!(state.local.missing_since(record.id), Some(500));
        assert!(!state.is_pinned(Path::new(r"old\a.md")));
        assert!(state.local.expanded.is_empty());
        assert!(state.folders.is_empty());
        assert!(!state.is_folder(Path::new(r"old\inner")));
        assert_eq!(tree_rows(&state.tree), rebuilt_rows(&state));
        assert_eq!(tree_rows(&state.tree).len(), 1);
    }

    #[test]
    fn a_folder_changed_while_a_rescan_ran_is_not_undone_by_its_result() {
        // Break caught: a rescan that listed the folders before FastPad renamed or made one
        // bringing the old row back (with its note at a path that is gone) and hiding the new
        // one until the next rescan; or a replay that breaks a rescan that already saw it.
        let scratch = Scratch::new("folder-merge");
        let folder = scratch.folder();
        std::fs::create_dir_all(folder.join(r"work\empty")).unwrap();
        std::fs::write(folder.join(r"work\plan.md"), "p").unwrap();
        std::fs::write(folder.join("top.md"), "t").unwrap();
        let mut previous = load(&folder, &scratch.local(), 100).unwrap();
        let stale = load(&folder, &scratch.local(), 101).unwrap();
        std::fs::rename(folder.join("work"), folder.join("Archive")).unwrap();
        previous.rename_folder(Path::new("work"), Path::new("Archive"));
        std::fs::create_dir(folder.join("Made")).unwrap();
        previous.add_folder(Path::new("Made"));

        let merged = merge_rescan(previous, stale);

        assert_eq!(
            sorted_folders(&merged),
            ["Archive", r"Archive\empty", "Made"].map(PathBuf::from)
        );
        assert_eq!(
            sorted_notes(&merged),
            [PathBuf::from(r"Archive\plan.md"), PathBuf::from("top.md")]
        );
        assert_eq!(tree_rows(&merged.tree), rebuilt_rows(&merged));

        let mut previous = load(&folder, &scratch.local(), 102).unwrap();
        std::fs::rename(folder.join("Archive"), folder.join("Final")).unwrap();
        previous.rename_folder(Path::new("Archive"), Path::new("Final"));
        let seen = load(&folder, &scratch.local(), 103).unwrap();

        let merged = merge_rescan(previous, seen);

        assert_eq!(
            sorted_folders(&merged),
            ["Final", r"Final\empty", "Made"].map(PathBuf::from)
        );
        assert_eq!(
            sorted_notes(&merged),
            [PathBuf::from(r"Final\plan.md"), PathBuf::from("top.md")]
        );
        assert_eq!(tree_rows(&merged.tree), rebuilt_rows(&merged));
    }
```

- [ ] **Step 2: Run the tests and see them fail**

Run: `cargo test --lib library::tests`
Expected: FAIL to compile: no method `rename_folder` / `remove_folder` / `add_folder` / `is_folder` / `is_listed` / `notes_under` on `LibraryState`.

- [ ] **Step 3: Implement**

In `src/library/mod.rs`, after the `NoteEntry` struct, add:

```rust
/// A folder change FastPad made itself (notebook folders spec §3.2). Each is kept until the next
/// rescan starts, and replayed onto that rescan's result, which may have listed the folders
/// before the change.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FolderChange {
    Added(PathBuf),
    Removed(PathBuf),
    Renamed { old: PathBuf, new: PathBuf },
}
```

In `pub struct LibraryState`, after the `folders` field, add:

```rust
    /// FastPad's own folder changes since the running rescan started, replayed onto its result.
    pub folder_changes: Vec<FolderChange>,
```

In `load`'s literal, after `        folders: scan.folders,` add `        folder_changes: Vec::new(),`. In the test `bare_state`, after `            folders: Vec::new(),` add `            folder_changes: Vec::new(),`.

After `fn sync_pins`, add:

```rust
/// Whether `path` is `folder` itself or inside it, ignoring case.
fn at_or_under(path: &Path, folder: &Path) -> bool {
    same_path(path, folder) || strip_folder(folder, path).is_some()
}

/// `path` moved from `old` to `new`, when it is `old` itself or inside it.
fn reroot(path: &Path, old: &Path, new: &Path) -> Option<PathBuf> {
    if same_path(path, old) {
        return Some(new.to_path_buf());
    }
    strip_folder(old, path).map(|rest| new.join(rest))
}
```

In `merge_rescan`, replace:

```rust
    let LibraryState {
        library: previous_library,
        metadata: previous_metadata,
        stamp: previous_stamp,
        pending: previous_pending,
        local: previous_local,
        touched,
        ..
    } = previous;
    let mut fresh = fresh;
    // The rescan built its tree from these pins; what the merge changes is applied to it below.
    let built_pins = pinned_paths(&fresh.library);
```

with:

```rust
    let LibraryState {
        library: previous_library,
        metadata: previous_metadata,
        stamp: previous_stamp,
        pending: previous_pending,
        local: previous_local,
        touched,
        folder_changes,
        ..
    } = previous;
    let mut fresh = fresh;
    // The rescan built its tree from these pins; what the merge changes is applied to it below.
    let built_pins = pinned_paths(&fresh.library);
    // FastPad's folder changes while the rescan ran, which its result may not have seen: first,
    // so the touched notes below are re-checked at their current paths.
    for change in &folder_changes {
        fresh.change_folders(change);
    }
```

In `impl LibraryState`, after `rename_note`, add:

```rust
    /// Whether the tree has a folder at `relative`, ignoring case.
    pub fn is_folder(&self, relative: &Path) -> bool {
        self.tree.contains_folder(relative)
    }

    /// Whether a folder or a listed note already has the path `relative`, ignoring case: what a
    /// new or renamed folder may not take (notebook folders spec §4.1).
    pub fn is_listed(&self, relative: &Path) -> bool {
        self.is_folder(relative)
            || self
                .notes
                .iter()
                .any(|note| same_path(&note.path, relative))
    }

    /// How many listed notes are inside the folder `relative`, at any depth.
    pub fn notes_under(&self, relative: &Path) -> usize {
        self.notes
            .iter()
            .filter(|note| at_or_under(&note.path, relative))
            .count()
    }

    /// Follows a folder FastPad just created: it is listed and has a row.
    pub fn add_folder(&mut self, relative: &Path) {
        let change = FolderChange::Added(relative.to_path_buf());
        self.change_folders(&change);
        self.folder_changes.push(change);
    }

    /// Follows a folder rename FastPad just made on disk: the records of the notes under it move
    /// (`Relocate`, so their IDs and pins survive), and so do the notes, the folder list, the
    /// expanded folders and the tree. Touches no disk.
    pub fn rename_folder(&mut self, old: &Path, new: &Path) {
        let relocations: Vec<PendingOp> = self
            .library
            .notes
            .iter()
            .filter(|record| !record.deleted && !record.path.is_absolute())
            .filter_map(|record| {
                let path = reroot(&record.path, old, new)?;
                Some(PendingOp::Relocate {
                    note: NoteRef {
                        id: record.id,
                        path: record.path.clone(),
                    },
                    path,
                })
            })
            .collect();
        for op in relocations {
            let _ = self.apply(op);
        }
        for entry in self.local.expanded.iter_mut().chain(self.touched.iter_mut()) {
            if let Some(moved) = reroot(entry, old, new) {
                *entry = moved;
            }
        }
        let change = FolderChange::Renamed {
            old: old.to_path_buf(),
            new: new.to_path_buf(),
        };
        self.change_folders(&change);
        self.folder_changes.push(change);
    }

    /// Follows a folder FastPad just sent to the Recycle Bin: its notes leave the list and the
    /// tree, their records are flagged deleted and marked missing at `now`, as a deleted note's
    /// are, and its expanded entries go. Touches no disk.
    pub fn remove_folder(&mut self, relative: &Path, now: u64) {
        let doomed: Vec<NoteRef> = self
            .library
            .notes
            .iter()
            .filter(|record| {
                !record.deleted
                    && !record.path.is_absolute()
                    && at_or_under(&record.path, relative)
            })
            .map(|record| NoteRef {
                id: record.id,
                path: record.path.clone(),
            })
            .collect();
        for note in doomed {
            let id = note.id;
            let _ = self.apply(PendingOp::SetDeleted { note, value: true });
            self.local.set_missing(id, now);
        }
        self.local
            .expanded
            .retain(|folder| !at_or_under(folder, relative));
        self.touched.retain(|path| !at_or_under(path, relative));
        let change = FolderChange::Removed(relative.to_path_buf());
        self.change_folders(&change);
        self.folder_changes.push(change);
    }

    /// The in-memory part of a folder change: the folder list, the notes and the tree. A
    /// rescan's result gets only this part replayed: its records come from the pending
    /// operations, and its expanded folders from the live state.
    fn change_folders(&mut self, change: &FolderChange) {
        match change {
            FolderChange::Added(path) => {
                if !self.folders.iter().any(|folder| same_path(folder, path)) {
                    self.folders.push(path.clone());
                }
                self.tree.insert_folder(path);
            }
            FolderChange::Removed(path) => {
                self.folders.retain(|folder| !at_or_under(folder, path));
                self.notes.retain(|note| !at_or_under(&note.path, path));
                self.tree.remove_folder(path);
            }
            FolderChange::Renamed { old, new } => {
                for folder in &mut self.folders {
                    if let Some(moved) = reroot(folder, old, new) {
                        *folder = moved;
                    }
                }
                if !self.folders.iter().any(|folder| same_path(folder, new)) {
                    self.folders.push(new.clone());
                }
                for note in &mut self.notes {
                    if let Some(moved) = reroot(&note.path, old, new) {
                        note.path = moved;
                    }
                }
                self.tree.rename_folder(old, new);
            }
        }
    }
```

In `src/window/library_host.rs`'s `spawn_load`, replace:

```rust
        if let Some(state) = host.state.as_mut() {
            state.touched.clear();
        }
```

with:

```rust
        if let Some(state) = host.state.as_mut() {
            state.touched.clear();
            state.folder_changes.clear();
        }
```

- [ ] **Step 4: Run the tests and see them pass**

Run: `cargo test --lib library::tests`
Expected: all pass.

- [ ] **Step 5: Clippy**

Run: `cargo fmt; cargo clippy --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 6: Commit**

```bash
git add src/library/mod.rs src/window/library_host.rs
git commit -m "feat(library): LibraryState add_folder, rename_folder and remove_folder keep notes, records, expanded folders and the tree in step, replayed onto a rescan that ran meanwhile"
```

---

### Task 4: File-type icons and type names

**Files:**
- Create: `src/window/file_icons.rs`
- Modify: `src/window/mod.rs` (module list), `src/window/palette.rs` (`FileIcons`, tests), `src/window/side_panel.rs` (`ViewPaint::icons`, `view_paint`), `src/window/main_window.rs` (`current_file_icons`, one test assertion), `src/window/notebook_view.rs` (imports, `draw_tree_row`, `paint`), `src/window/sidebar_accessibility.rs` (`tree_item`, its test)

**Interfaces:**
- Consumes: `catppuccin::{LATTE, FRAPPE, MACCHIATO, MOCHA}` roles `blue`, `yellow`, `peach`, `green`, `maroon`, `overlay2`; `Palette::for_theme`.
- Produces:
  - `pub(crate) enum IconFont { Glyph, Bold }`, `pub(crate) enum IconColor { Blue, Yellow, Peach, Green, Maroon, Overlay2 }`
  - `pub(crate) struct FileIcon { pub(crate) text: &'static str, pub(crate) font: IconFont, pub(crate) color: IconColor }`
  - `pub(crate) fn file_icon(extension: Option<&str>) -> FileIcon`, `pub(crate) const FOLDER_ICON: FileIcon`, `pub(crate) fn type_name(extension: Option<&str>) -> &'static str`
  - `pub struct FileIcons { pub blue: u32, pub yellow: u32, pub peach: u32, pub green: u32, pub maroon: u32, pub overlay2: u32 }` with `pub const fn neutral() -> Self`, `pub fn for_theme(Theme, bool) -> Self`, `pub fn for_cached_theme(Option<SystemTheme>, ThemePreference) -> Self`, `pub(crate) const fn color(&self, IconColor) -> u32`
  - `pub(crate) fn current_file_icons(hwnd: HWND) -> FileIcons` in `main_window`
  - `ViewPaint.icons: FileIcons`

**Decisions this task settles:**
- On a selected or hovered row the icon keeps its colour (spec §5.2); only high contrast draws it in the row's existing `muted` colour (`muted_foreground`, or the selection text colour on a focused selected row, as today), so high contrast never draws a colour pair the system did not choose.
- `file_icon` matches extensions with `eq_ignore_ascii_case` over a fixed table: no allocation per painted row.

- [ ] **Step 1: Write the failing tests**

Create `src/window/file_icons.rs` with only its tests for now:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_note_type_has_its_icon_font_and_colour_role_in_any_letter_case() {
        // Break caught: a JSON note drawn with the document glyph, a config file in the text
        // colour, or `.MD` falling back to plain text (spec §5.1).
        let cases = [
            ("md", GLYPH_DOCUMENT, IconFont::Glyph, IconColor::Blue),
            ("markdown", GLYPH_DOCUMENT, IconFont::Glyph, IconColor::Blue),
            ("json", "{}", IconFont::Bold, IconColor::Yellow),
            ("yaml", GLYPH_SETTINGS, IconFont::Glyph, IconColor::Peach),
            ("yml", GLYPH_SETTINGS, IconFont::Glyph, IconColor::Peach),
            ("toml", GLYPH_SETTINGS, IconFont::Glyph, IconColor::Peach),
            ("ini", GLYPH_SETTINGS, IconFont::Glyph, IconColor::Peach),
            ("cfg", GLYPH_SETTINGS, IconFont::Glyph, IconColor::Peach),
            ("conf", GLYPH_SETTINGS, IconFont::Glyph, IconColor::Peach),
            ("csv", GLYPH_GRID, IconFont::Glyph, IconColor::Green),
            ("xml", GLYPH_CODE, IconFont::Glyph, IconColor::Maroon),
            ("txt", GLYPH_DOCUMENT, IconFont::Glyph, IconColor::Overlay2),
            ("text", GLYPH_DOCUMENT, IconFont::Glyph, IconColor::Overlay2),
            ("log", GLYPH_DOCUMENT, IconFont::Glyph, IconColor::Overlay2),
        ];
        for (extension, text, font, color) in cases {
            let expected = FileIcon { text, font, color };
            assert_eq!(file_icon(Some(extension)), expected, "{extension}");
            assert_eq!(
                file_icon(Some(&extension.to_uppercase())),
                expected,
                "{extension}"
            );
        }
        assert_eq!(file_icon(Some("Md")), file_icon(Some("md")));
        assert_eq!(file_icon(Some("JsOn")).text, "{}");
        let text = file_icon(Some("txt"));
        assert_eq!(file_icon(None), text);
        assert_eq!(file_icon(Some("py")), text);
        assert_eq!(file_icon(Some("")), text);
        assert_eq!(
            FOLDER_ICON,
            FileIcon {
                text: "\u{E8B7}",
                font: IconFont::Glyph,
                color: IconColor::Yellow
            }
        );
        assert_eq!(
            (GLYPH_DOCUMENT, GLYPH_SETTINGS, GLYPH_GRID, GLYPH_CODE),
            ("\u{E8A5}", "\u{E713}", "\u{E80A}", "\u{E943}")
        );
    }

    #[test]
    fn every_note_extension_has_a_type_name_for_screen_readers() {
        // Break caught: a note type added to NOTE_EXTENSIONS read out as "text", or `cfg` and
        // `conf` named differently (spec §5.4).
        for extension in crate::library::title::NOTE_EXTENSIONS {
            assert!(
                KINDS.iter().any(|(known, _)| *known == extension),
                "{extension}"
            );
        }
        for (extension, name) in [
            ("md", "Markdown"),
            ("markdown", "Markdown"),
            ("json", "JSON"),
            ("yaml", "YAML"),
            ("yml", "YAML"),
            ("toml", "TOML"),
            ("ini", "INI"),
            ("cfg", "config"),
            ("conf", "config"),
            ("CSV", "CSV"),
            ("xml", "XML"),
            ("txt", "text"),
            ("text", "text"),
            ("log", "log"),
        ] {
            assert_eq!(type_name(Some(extension)), name, "{extension}");
        }
        assert_eq!(type_name(None), "text");
    }
}
```

In `src/window/mod.rs`, after `pub mod find_bar;` add `pub(crate) mod file_icons;`.

In `src/window/palette.rs`'s `mod tests`, add:

```rust
    /// WCAG relative luminance of a `COLORREF`.
    fn luminance(color: u32) -> f64 {
        let channel = |shift: u32| {
            let value = f64::from((color >> shift) & 0xFF) / 255.0;
            if value <= 0.040_45 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(0) + 0.7152 * channel(8) + 0.0722 * channel(16)
    }

    fn contrast(a: u32, b: u32) -> f64 {
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[test]
    fn file_icon_colours_come_from_the_themes_flavour_and_high_contrast_mutes_them() {
        // Break caught: the Light theme drawing Mocha's pastel icons on white, a Catppuccin theme
        // using another flavour's swatches, or coloured icons in high contrast (spec §5.2).
        use super::FileIcons;
        let flavors = [
            (Theme::Light, catppuccin::LATTE),
            (Theme::Dark, catppuccin::MOCHA),
            (Theme::CatppuccinLatte, catppuccin::LATTE),
            (Theme::CatppuccinFrappe, catppuccin::FRAPPE),
            (Theme::CatppuccinMacchiato, catppuccin::MACCHIATO),
            (Theme::CatppuccinMocha, catppuccin::MOCHA),
        ];
        for (theme, flavor) in flavors {
            let icons = FileIcons::for_theme(theme, false);
            assert_eq!(
                (
                    icons.blue,
                    icons.yellow,
                    icons.peach,
                    icons.green,
                    icons.maroon,
                    icons.overlay2
                ),
                (
                    flavor.blue,
                    flavor.yellow,
                    flavor.peach,
                    flavor.green,
                    flavor.maroon,
                    flavor.overlay2
                ),
                "{theme:?}"
            );
            let muted = Palette::for_theme(theme, true).muted_foreground;
            let system = FileIcons::for_theme(theme, true);
            assert!(
                [
                    system.blue,
                    system.yellow,
                    system.peach,
                    system.green,
                    system.maroon,
                    system.overlay2
                ]
                .iter()
                .all(|&color| color == muted)
            );
        }
        assert_eq!(FileIcons::neutral(), FileIcons::for_theme(Theme::Light, false));
    }

    #[test]
    fn file_icon_colours_stay_visible_on_selected_and_hovered_rows() {
        // Break caught: an icon colour that disappears into the selection or hover highlight,
        // where spec §5.2 keeps it. The weakest pair, Latte yellow on the Light theme's
        // selection, is about 1.7:1.
        use super::FileIcons;
        for theme in Theme::ALL {
            let palette = Palette::for_theme(theme, false);
            let icons = FileIcons::for_theme(theme, false);
            for color in [
                icons.blue,
                icons.yellow,
                icons.peach,
                icons.green,
                icons.maroon,
                icons.overlay2,
            ] {
                for background in [
                    palette.selection_background,
                    palette.inactive_selection_background,
                    palette.hover_background,
                    palette.panel_background(),
                ] {
                    let ratio = contrast(color, background);
                    assert!(
                        ratio >= 1.5,
                        "{theme:?}: {color:06x} on {background:06x} is {ratio:.2}:1"
                    );
                }
            }
        }
    }
```

In `src/window/sidebar_accessibility.rs`'s test `tree_rows_are_outline_items_with_expansion_pin_and_unsaved_in_their_names`, replace `        assert_eq!(pinned.name, "a, pinned");` with:

```rust
        assert_eq!(pinned.name, "a, Markdown, pinned", "the type, then the pin");
```

and after the `unsaved` assertions (before the test's closing brace) add:

```rust
        // Break caught: a note's type conveyed by its coloured icon alone (spec §5.4).
        let csv = tree_item(
            &row(RowKind::Note(PathBuf::from(r"Work\budget.CSV")), "budget", 1, false, false),
            false,
            false,
            ROW,
            true,
        );
        assert_eq!(csv.name, "budget, CSV");
        let markdown = tree_item(
            &row(
                RowKind::Note(PathBuf::from("meeting notes.md")),
                "meeting notes",
                0,
                false,
                false,
            ),
            false,
            false,
            ROW,
            true,
        );
        assert_eq!(markdown.name, "meeting notes, Markdown");
        assert_eq!(folder.name, "Work", "folder rows are unchanged");
```

In `src/window/main_window.rs`'s test `the_panel_exposes_the_tree_as_an_outline_with_pinned_and_folder_states`, replace `        assert_eq!(rows[0].name, "a, pinned");` with `        assert_eq!(rows[0].name, "a, Markdown, pinned");`.

- [ ] **Step 2: Run the tests and see them fail**

Run: `cargo test --lib window::file_icons window::palette window::sidebar_accessibility`
Expected: FAIL to compile: cannot find `file_icon`, `FileIcon`, `KINDS`, `FileIcons`.

- [ ] **Step 3: Implement**

Prepend to `src/window/file_icons.rs` (above the tests):

```rust
//! The Notebook view's note-type icons (notebook folders spec §5): for each note extension, a
//! glyph (or the `{}` label), the font it is drawn in and the Catppuccin colour role that
//! `palette::FileIcons` resolves per theme; and the type name screen readers hear. Pure: no
//! Win32, no disk.

/// Which of the sidebar's fonts an icon is drawn in.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum IconFont {
    /// Segoe MDL2 Assets (`UiFonts::glyph`).
    Glyph,
    /// The bold UI font (`UiFonts::bold`), for the `{}` label.
    Bold,
}

/// A Catppuccin colour role; `palette::FileIcons` holds each theme's colour for it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum IconColor {
    Blue,
    Yellow,
    Peach,
    Green,
    Maroon,
    Overlay2,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FileIcon {
    pub(crate) text: &'static str,
    pub(crate) font: IconFont,
    pub(crate) color: IconColor,
}

const GLYPH_DOCUMENT: &str = "\u{E8A5}";
const GLYPH_SETTINGS: &str = "\u{E713}";
const GLYPH_GRID: &str = "\u{E80A}";
const GLYPH_CODE: &str = "\u{E943}";
const GLYPH_FOLDER: &str = "\u{E8B7}";

const fn icon(text: &'static str, font: IconFont, color: IconColor) -> FileIcon {
    FileIcon { text, font, color }
}

/// A folder row's icon.
pub(crate) const FOLDER_ICON: FileIcon = icon(GLYPH_FOLDER, IconFont::Glyph, IconColor::Yellow);

/// The note types: each group of extensions that shares an icon and a name.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Kind {
    Markdown,
    Json,
    Yaml,
    Toml,
    Ini,
    Config,
    Csv,
    Xml,
    Text,
    Log,
}

/// Every note extension (`title::NOTE_EXTENSIONS`) and its type.
const KINDS: [(&str, Kind); 14] = [
    ("md", Kind::Markdown),
    ("markdown", Kind::Markdown),
    ("json", Kind::Json),
    ("yaml", Kind::Yaml),
    ("yml", Kind::Yaml),
    ("toml", Kind::Toml),
    ("ini", Kind::Ini),
    ("cfg", Kind::Config),
    ("conf", Kind::Config),
    ("csv", Kind::Csv),
    ("xml", Kind::Xml),
    ("txt", Kind::Text),
    ("text", Kind::Text),
    ("log", Kind::Log),
];

/// `extension`'s type, ignoring case; anything else (or no extension) is text.
fn kind(extension: Option<&str>) -> Kind {
    extension
        .and_then(|extension| {
            KINDS
                .iter()
                .find(|(known, _)| known.eq_ignore_ascii_case(extension))
        })
        .map_or(Kind::Text, |&(_, kind)| kind)
}

/// A note row's icon, from its file's extension (spec §5.1).
pub(crate) fn file_icon(extension: Option<&str>) -> FileIcon {
    match kind(extension) {
        Kind::Markdown => icon(GLYPH_DOCUMENT, IconFont::Glyph, IconColor::Blue),
        Kind::Json => icon("{}", IconFont::Bold, IconColor::Yellow),
        Kind::Yaml | Kind::Toml | Kind::Ini | Kind::Config => {
            icon(GLYPH_SETTINGS, IconFont::Glyph, IconColor::Peach)
        }
        Kind::Csv => icon(GLYPH_GRID, IconFont::Glyph, IconColor::Green),
        Kind::Xml => icon(GLYPH_CODE, IconFont::Glyph, IconColor::Maroon),
        Kind::Text | Kind::Log => icon(GLYPH_DOCUMENT, IconFont::Glyph, IconColor::Overlay2),
    }
}

/// The type a note row's accessible name carries after its name (spec §5.4).
pub(crate) fn type_name(extension: Option<&str>) -> &'static str {
    match kind(extension) {
        Kind::Markdown => "Markdown",
        Kind::Json => "JSON",
        Kind::Yaml => "YAML",
        Kind::Toml => "TOML",
        Kind::Ini => "INI",
        Kind::Config => "config",
        Kind::Csv => "CSV",
        Kind::Xml => "XML",
        Kind::Text => "text",
        Kind::Log => "log",
    }
}
```

In `src/window/palette.rs`, after `use crate::catppuccin::{self, Flavor};` add `use super::file_icons::IconColor;`, and after the `PALETTES` static add:

```rust
/// The Notebook view's file-type icon colours (notebook folders spec §5.2): six Catppuccin
/// roles, from the theme's own flavour, Latte's for Light and Mocha's for Dark. High contrast
/// uses the muted system colour for all of them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileIcons {
    pub blue: u32,
    pub yellow: u32,
    pub peach: u32,
    pub green: u32,
    pub maroon: u32,
    pub overlay2: u32,
}

const fn file_icons(flavor: &Flavor) -> FileIcons {
    FileIcons {
        blue: flavor.blue,
        yellow: flavor.yellow,
        peach: flavor.peach,
        green: flavor.green,
        maroon: flavor.maroon,
        overlay2: flavor.overlay2,
    }
}

/// Indexed by `Theme as usize`, like `PALETTES`.
static FILE_ICONS: [FileIcons; Theme::COUNT] = [
    file_icons(&catppuccin::LATTE),
    file_icons(&catppuccin::MOCHA),
    file_icons(&catppuccin::LATTE),
    file_icons(&catppuccin::FRAPPE),
    file_icons(&catppuccin::MACCHIATO),
    file_icons(&catppuccin::MOCHA),
];

impl FileIcons {
    /// The neutral first-paint palette's icons (Light's).
    pub const fn neutral() -> Self {
        file_icons(&catppuccin::LATTE)
    }

    pub fn for_theme(theme: Theme, high_contrast: bool) -> Self {
        if high_contrast {
            let muted = Palette::for_theme(theme, true).muted_foreground;
            Self {
                blue: muted,
                yellow: muted,
                peach: muted,
                green: muted,
                maroon: muted,
                overlay2: muted,
            }
        } else {
            FILE_ICONS[theme as usize]
        }
    }

    /// `None` means chrome has not been built yet: the neutral icons, with no theme queries.
    pub fn for_cached_theme(
        theme: Option<SystemTheme>,
        preference: crate::config::ThemePreference,
    ) -> Self {
        theme.map_or_else(Self::neutral, |theme| {
            Self::for_theme(theme.effective_theme(preference), theme.high_contrast)
        })
    }

    pub(crate) const fn color(&self, role: IconColor) -> u32 {
        match role {
            IconColor::Blue => self.blue,
            IconColor::Yellow => self.yellow,
            IconColor::Peach => self.peach,
            IconColor::Green => self.green,
            IconColor::Maroon => self.maroon,
            IconColor::Overlay2 => self.overlay2,
        }
    }
}
```

In `src/window/main_window.rs`, after

```rust
pub(crate) fn current_palette(hwnd: HWND) -> Palette {
    title_chrome(hwnd).0
}
```

add:

```rust
/// The Notebook view's file-type icon colours for the current theme. Call it with nothing of
/// the App borrowed.
pub(crate) fn current_file_icons(hwnd: HWND) -> crate::window::palette::FileIcons {
    unsafe { app_ptr(hwnd) }.map_or_else(crate::window::palette::FileIcons::neutral, |app| {
        let app = unsafe { app.as_ref() };
        crate::window::palette::FileIcons::for_cached_theme(app.theme, app.settings.theme)
    })
}
```

In `src/window/side_panel.rs`, in `pub(crate) struct ViewPaint`, after `pub(crate) palette: Palette,` add:

```rust
    /// The Notebook view's file-type icon colours for the theme (notebook folders spec §5.2).
    pub(crate) icons: crate::window::palette::FileIcons,
```

and in `view_paint`, after `        palette,` add `        icons: super::main_window::current_file_icons(main),`.

In `src/window/notebook_view.rs`:
- after `use crate::window::commands::CommandId;` add `use crate::window::file_icons::{FOLDER_ICON, FileIcon, IconFont, file_icon};`
- replace `use crate::window::palette::Palette;` with `use crate::window::palette::{FileIcons, Palette};`
- replace the signature and the icon part of `draw_tree_row`, from `fn draw_tree_row(` through the end of the `match &row.kind { ... }` block (the block ending with the `RowKind::Note(_) | RowKind::Unsaved(_) => { ... }` arm), with:

```rust
fn draw_tree_row(
    dc: HDC,
    row: Option<&TreeRow>,
    rect: RECT,
    look: RowLook,
    palette: &Palette,
    icons: &FileIcons,
    fonts: UiFonts,
    dpi: u32,
    pin_hot: bool,
) {
    let foreground = row_foreground(look, palette);
    let muted = if look.selected && look.focused {
        foreground
    } else {
        palette.muted_foreground
    };
    let Some(row) = row else {
        let text = RECT {
            left: rect.left + scale(LEFT_PAD, dpi),
            ..rect
        };
        unsafe { draw_text(dc, TRUNCATED_ROW, text, fonts.italic, muted, LINE) };
        return;
    };
    let parts = row_parts(rect, row.depth, dpi);
    // A type icon keeps its colour on a selected or hovered row: the colours are mid-tones that
    // read on the selection. High contrast draws every icon in the muted system pair, as before
    // (notebook folders spec §5.2).
    let draw_icon = |icon: FileIcon| {
        let color = if palette.high_contrast {
            muted
        } else {
            icons.color(icon.color)
        };
        let font = match icon.font {
            IconFont::Glyph => fonts.glyph,
            IconFont::Bold => fonts.bold,
        };
        unsafe { draw_text(dc, icon.text, parts.icon, font, color, CENTERED) };
    };
    match &row.kind {
        RowKind::Folder(_) => {
            let chevron = if row.expanded {
                GLYPH_CHEVRON_DOWN
            } else {
                GLYPH_CHEVRON_RIGHT
            };
            unsafe { draw_text(dc, chevron, parts.chevron, fonts.glyph, muted, CENTERED) };
            draw_icon(FOLDER_ICON);
        }
        RowKind::Note(path) => {
            let extension = path.extension().map(|extension| extension.to_string_lossy());
            draw_icon(file_icon(extension.as_deref()));
        }
        RowKind::Unsaved(_) => {
            unsafe { draw_text(dc, GLYPH_NOTE, parts.icon, fonts.glyph, muted, CENTERED) };
        }
    }
```

  (the rest of `draw_tree_row`, from `if matches!(row.kind, RowKind::Note(_)) {` on, is unchanged).
- in `NotebookView::paint`'s `Mode::Tree` arm, replace

```rust
                let rows = &self.rows;
                let hover_pin = self.hover_pin;
```

with

```rust
                let rows = &self.rows;
                let hover_pin = self.hover_pin;
                let icons = &paint.icons;
```

  and in the `draw_tree_row(` call replace the argument line `                            palette,` with the two lines `                            palette,` and `                            icons,`.

In `src/window/sidebar_accessibility.rs`'s `tree_item`, replace:

```rust
    let mut name = row.name.clone();
    if row.pinned {
```

with:

```rust
    let mut name = row.name.clone();
    // The type comes from the extension, which the row's name leaves out (spec §5.4).
    if let RowKind::Note(path) = &row.kind {
        let extension = path.extension().map(|extension| extension.to_string_lossy());
        name.push_str(", ");
        name.push_str(crate::window::file_icons::type_name(extension.as_deref()));
    }
    if row.pinned {
```

and update its doc comment's first line to `/// A Notebook-view row. The name carries a note's type, then ", pinned" or ", unsaved", so none`.

- [ ] **Step 4: Run the tests and see them pass**

Run: `cargo test --lib window::file_icons window::palette window::sidebar_accessibility window::notebook_view`
Expected: all pass.
Run: `cargo test --lib -- --test-threads=1 the_panel_exposes_the_tree_as_an_outline_with_pinned_and_folder_states`
Expected: 1 passed.

- [ ] **Step 5: Clippy**

Run: `cargo fmt; cargo clippy --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 6: Commit**

```bash
git add src/window/file_icons.rs src/window/mod.rs src/window/palette.rs src/window/side_panel.rs src/window/main_window.rs src/window/notebook_view.rs src/window/sidebar_accessibility.rs
git commit -m "feat(sidebar): coloured file-type icons per note extension from the theme's Catppuccin roles, and the type in each note row's accessible name"
```

---

### Task 5: New folder

**Files:**
- Modify: `src/window/commands.rs` (`NoteNewFolder`, `needs_document`, `COMMANDS`, tests)
- Modify: `src/window/command_palette.rs` (`ENTRIES`, tests)
- Modify: `src/library/title.rs` (`clean_stem`, `sanitize_stem`, `folder_name`, test)
- Modify: `src/window/name_box.rs` (`NamePurpose::NewFolder`, `document`, `folder`, `focus`)
- Modify: `src/window/library_host.rs` (imports, `new_folder`, `submit_new_folder`, helpers, `name_box_submit`, `close_stale_name_box`, `install`, `open_checked_folder`)
- Modify: `src/window/notebook_view.rs` (header button, folder menu, `select_row`, `focus_tree`, header test)
- Modify: `src/window/main_window.rs` (`execute_command_with_note` arm, palette filter, window tests)

**Interfaces:**
- Consumes: `LibraryState::{add_folder, is_listed, is_folder}` (Task 3), `scan::skip_directory`, `tree::ancestors`, `notebook_view::selected_folder`, `menus::answer_next_popup_menu` (tests).
- Produces:
  - `CommandId::NoteNewFolder = 192`
  - `pub fn folder_name(input: &str) -> Option<String>` in `library::title`
  - `NamePurpose::NewFolder(PathBuf)`; `pub(crate) fn folder(&self) -> Option<&Path>`
  - `pub(crate) fn new_folder(hwnd: HWND, parent: Option<PathBuf>)` in `library_host` (`parent` relative)
  - `HeaderButton::NewFolder`; `HeaderLayout.buttons: [(HeaderButton, RECT); 4]`
  - `pub(crate) fn select_row(hwnd: HWND, kind: &RowKind) -> bool`, `pub(crate) fn focus_tree(hwnd: HWND)` in `notebook_view`
  - `fn folder_taken_error(name: &str) -> String`, `fn hidden_folder_error(name: &str) -> String`, `fn folder_suffix(hwnd: HWND, parent: &Path) -> String`, `const NO_FOLDER_NAME: &str` in `library_host` (used again by Task 6)

**Decisions this task settles:**
- A name the scan would hide (`skip_directory`: a dot-folder, `node_modules`, `target`, `bin`, `obj`) is refused: `FastPad hides folders named “<name>”. Choose another name.` Otherwise the folder would vanish at the next rescan.
- Clashes are checked in memory (`is_listed`); any other file or hidden folder with the name makes `create_dir` fail with `AlreadyExists`, reported with the same message. The default `New folder N` counts listed folders only.
- The name is selected in full (dots included). Enter moves the focus to the tree, on the new row; Escape returns it to the editor, as the note name box does.
- The suffix names the parent's own folder name (`in inner`), or the notebook at the root.
- A folder box closes when its folder (for `NewFolder`, its parent) is not in the tree: checked by `close_stale_name_box`, which now also runs after each install (load or rescan) and after a notebook switch.

- [ ] **Step 1: Write the failing tests**

In `src/window/commands.rs`'s tests, in `quick_open_is_191_and_needs_no_document`, delete the line `        assert_eq!(CommandId::try_from(192), Err(()));`, and add:

```rust
    #[test]
    fn new_folder_is_192_and_neither_needs_a_document_nor_the_sidebar() {
        // Break caught: New folder renumbered onto another command, greyed out while no tab is
        // open, or treated as a sidebar command (notebook folders spec §6).
        assert_eq!(CommandId::NoteNewFolder as u16, 192);
        assert_eq!(CommandId::try_from(192), Ok(CommandId::NoteNewFolder));
        assert!(!CommandId::NoteNewFolder.needs_document());
        assert!(!CommandId::NoteNewFolder.is_sidebar());
        assert_eq!(CommandId::try_from(193), Err(()));
    }
```

In `src/window/command_palette.rs`'s tests, in `go_to_note_is_listed_once_with_ctrl_p` replace `        assert_eq!(ENTRIES.len(), 70);` with `        assert_eq!(ENTRIES.len(), 71);`, and add:

```rust
    #[test]
    fn new_folder_is_listed_under_notebook_without_a_shortcut() {
        // Break caught: the palette never offering New folder, or showing a shortcut it doesn't
        // have (spec §6).
        assert_eq!(labels("new folder")[0], "Notebook: New folder\u{2026}");
        assert_eq!(shortcut_text(CommandId::NoteNewFolder), None);
    }
```

In `src/library/title.rs`'s tests add:

```rust
    #[test]
    fn folder_names_are_cleaned_like_stems_and_an_empty_one_is_none() {
        // Break caught: a typed "a/b: c?" or "CON" folder that Windows refuses, "..." creating a
        // folder named "Untitled", or "v1.2" losing ".2" to extension handling (spec §4.1).
        assert_eq!(folder_name(" a/b: c?. ").as_deref(), Some("ab c"));
        assert_eq!(folder_name("Plans.  ").as_deref(), Some("Plans"));
        assert_eq!(folder_name("CON").as_deref(), Some("CON_"));
        assert_eq!(folder_name("v1.2").as_deref(), Some("v1.2"));
        assert_eq!(folder_name("..."), None);
        assert_eq!(folder_name("   "), None);
        assert_eq!(folder_name("<>"), None);
        assert_eq!(sanitize_stem("..."), "Untitled", "file stems keep their fallback");
    }
```

In `src/window/notebook_view.rs`'s tests, replace the body of `the_header_buttons_sit_right_to_left_and_the_title_stops_before_them` from `        let layout = header_layout(area, 96);` to its end with:

```rust
        let layout = header_layout(area, 96);
        let [(first, star), (second, new), (third, folder), (fourth, more)] = layout.buttons;
        assert_eq!(
            (first, second, third, fourth),
            (
                HeaderButton::Favorite,
                HeaderButton::NewNote,
                HeaderButton::NewFolder,
                HeaderButton::More
            )
        );
        assert_eq!(edges(more), (226, 5, 254, 33));
        assert_eq!(
            (star.right, new.right, folder.right),
            (new.left, folder.left, more.left),
            "New folder sits right of New note"
        );
        assert!(layout.title.right <= star.left);
        assert_eq!(layout.title.bottom, 38);
        assert_eq!(body_rect(area, 96).top, 38);
    }
```

In `src/window/main_window.rs`'s `mod tests`, append (before the module's closing `}`):

```rust
    #[test]
    fn new_folder_from_the_header_creates_it_on_disk_and_selects_its_row() {
        // Break caught: the header button opening nothing, a folder created before Enter or on
        // Escape, a suggested name that is taken, or the new folder not selected in the tree.
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetFocus, VK_ESCAPE};
        use windows_sys::Win32::UI::WindowsAndMessaging::WM_KEYDOWN;
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("new-folder-header");
        std::fs::create_dir_all(scratch.folder().join("New folder")).unwrap();
        scratch.note("top.md", "t");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        ensure_sidebar(window.hwnd);
        scratch.install(window.hwnd);
        crate::window::side_panel::show_view(
            window.hwnd,
            crate::config::SidebarView::Notebook,
            false,
        );
        crate::window::notebook_view::rebuild(window.hwnd);
        select_row(window.hwnd, &RowKind::Note("top.md".into()));
        let panel = crate::window::side_panel::windows(window.hwnd).unwrap().1;
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
        let edit = app_mut(window.hwnd).name_box.as_ref().unwrap().edit_hwnd();
        unsafe { SendMessageW(edit, WM_KEYDOWN, VK_ESCAPE as usize, 0) };
        assert!(!name_box_visible(window.hwnd));
        assert!(!scratch.folder().join("New folder 2").exists(), "Escape creates nothing");

        press();
        let name_box = app_mut(window.hwnd).name_box.as_ref().unwrap();
        assert!(name_box.is_visible());
        assert_eq!(
            name_box.purpose(),
            Some(&crate::window::name_box::NamePurpose::NewFolder(
                std::path::PathBuf::new()
            ))
        );
        assert_eq!(name_box.text(), "New folder 2", "the first free name");
        assert!(!scratch.folder().join("New folder 2").exists(), "nothing before Enter");
        crate::window::library_host::name_box_submit(window.hwnd);

        assert!(!name_box_visible(window.hwnd));
        assert!(scratch.folder().join("New folder 2").is_dir());
        assert_eq!(
            selected_kind(window.hwnd),
            Some(RowKind::Folder("New folder 2".into()))
        );
        assert_eq!(unsafe { GetFocus() }, panel, "the focus returns to the tree");
    }

    #[test]
    fn new_folder_here_creates_it_inside_that_folder_expanded_and_refuses_a_taken_name() {
        // Break caught: "New folder here" creating at the root, a clash with a note or a non-note
        // file missed (or closing the box), or the new row hidden in a collapsed folder.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("new-folder-here");
        std::fs::create_dir_all(scratch.folder().join("sub")).unwrap();
        scratch.note(r"sub\a.md", "a");
        std::fs::write(scratch.folder().join(r"sub\notes.bin"), "x").unwrap();
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        ensure_sidebar(window.hwnd);
        scratch.install(window.hwnd);
        crate::window::notebook_view::rebuild(window.hwnd);
        let index = row_of(window.hwnd, &RowKind::Folder("sub".into()));
        crate::window::menus::answer_next_popup_menu(|_| Some(CommandId::NoteNewFolder));
        crate::window::notebook_view::open_context_menu(window.hwnd, index, None);

        let name_box = app_mut(window.hwnd).name_box.as_ref().unwrap();
        assert_eq!(
            name_box.purpose(),
            Some(&crate::window::name_box::NamePurpose::NewFolder("sub".into()))
        );
        assert_eq!(name_box.text(), "New folder");
        for taken in ["A.md", "notes.bin"] {
            type_into_name_box(window.hwnd, taken);
            crate::window::library_host::name_box_submit(window.hwnd);
            let name_box = app_mut(window.hwnd).name_box.as_ref().unwrap();
            assert!(name_box.is_visible(), "{taken}");
            assert_eq!(
                name_box.error(),
                Some(format!("A folder or file named \u{201c}{taken}\u{201d} already exists").as_str())
            );
        }
        type_into_name_box(window.hwnd, "Plans");
        crate::window::library_host::name_box_submit(window.hwnd);

        assert!(!name_box_visible(window.hwnd));
        assert!(scratch.folder().join(r"sub\Plans").is_dir());
        assert!(
            crate::window::library_host::expanded(window.hwnd)
                .contains(&std::path::PathBuf::from("sub"))
        );
        assert_eq!(
            selected_kind(window.hwnd),
            Some(RowKind::Folder(r"sub\Plans".into()))
        );
    }

    #[test]
    fn a_typed_folder_name_is_sanitized_and_empty_or_hidden_names_are_refused() {
        // Break caught: a name Windows refuses failing with a path error, "..." creating
        // "Untitled", or a .git or node_modules folder that the next rescan hides (spec §4.1).
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("new-folder-names");
        scratch.note("top.md", "t");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        ensure_sidebar(window.hwnd);
        scratch.install(window.hwnd);
        let create = |typed: &str| {
            crate::window::library_host::new_folder(window.hwnd, Some(std::path::PathBuf::new()));
            type_into_name_box(window.hwnd, typed);
            crate::window::library_host::name_box_submit(window.hwnd);
        };
        let error = || {
            app_mut(window.hwnd)
                .name_box
                .as_ref()
                .unwrap()
                .error()
                .map(str::to_owned)
        };

        create(" a/b: c?. ");
        assert!(scratch.folder().join("ab c").is_dir());
        create("CON");
        assert!(scratch.folder().join("CON_").is_dir());
        assert!(!name_box_visible(window.hwnd));

        create("...");
        assert!(name_box_visible(window.hwnd));
        assert_eq!(error().as_deref(), Some("Type a folder name"));
        for hidden in [".git", "node_modules"] {
            type_into_name_box(window.hwnd, hidden);
            crate::window::library_host::name_box_submit(window.hwnd);
            assert_eq!(
                error(),
                Some(format!(
                    "FastPad hides folders named \u{201c}{hidden}\u{201d}. Choose another name."
                ))
            );
            assert!(!scratch.folder().join(hidden).exists());
        }
    }

    #[test]
    fn a_new_folder_box_survives_a_rescan_but_closes_when_its_parent_goes() {
        // Break caught: a rescan closing the box (and the typed name) for nothing, a box left
        // offering to create inside a folder deleted in Explorer, or one outliving its notebook.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("new-folder-rescan");
        std::fs::create_dir_all(scratch.folder().join("sub")).unwrap();
        scratch.note(r"sub\a.md", "a");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        ensure_sidebar(window.hwnd);
        app_mut(window.hwnd).library.data_dir = Some(scratch.data());
        scratch.install(window.hwnd);
        crate::window::library_host::new_folder(window.hwnd, Some("sub".into()));
        type_into_name_box(window.hwnd, "Typed");
        super::create_new_document(window.hwnd).unwrap();
        assert!(name_box_visible(window.hwnd), "a tab switch keeps a folder box");

        rescan_and_wait(window.hwnd);
        assert!(name_box_visible(window.hwnd));
        assert_eq!(app_mut(window.hwnd).name_box.as_ref().unwrap().text(), "Typed");

        std::fs::remove_dir_all(scratch.folder().join("sub")).unwrap();
        rescan_and_wait(window.hwnd);
        assert!(!name_box_visible(window.hwnd));

        crate::window::library_host::new_folder(window.hwnd, None);
        assert!(name_box_visible(window.hwnd));
        crate::window::library_host::close_notebook(window.hwnd);
        assert!(!name_box_visible(window.hwnd));
    }

    #[test]
    fn an_empty_folder_made_on_disk_appears_after_a_rescan_and_goes_with_it() {
        // Break caught: a folder made in Explorer staying invisible until it holds a note, or a
        // folder deleted in Explorer keeping its row (spec §3.2).
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("empty-folder-rescan");
        scratch.note("top.md", "t");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        ensure_sidebar(window.hwnd);
        app_mut(window.hwnd).library.data_dir = Some(scratch.data());
        scratch.install(window.hwnd);

        std::fs::create_dir(scratch.folder().join("Fresh")).unwrap();
        rescan_and_wait(window.hwnd);
        row_of(window.hwnd, &RowKind::Folder("Fresh".into()));

        std::fs::remove_dir(scratch.folder().join("Fresh")).unwrap();
        rescan_and_wait(window.hwnd);
        assert!(
            crate::library::tree::row_index(
                &notebook_view(window.hwnd).rows,
                &RowKind::Folder("Fresh".into())
            )
            .is_none()
        );
    }

    #[test]
    fn the_palette_offers_new_folder_only_while_a_notebook_is_open() {
        // Break caught: "Notebook: New folder…" listed with no notebook, where it can only say
        // "Open a notebook first.", or missing once one is open (spec §4.1).
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("palette-new-folder");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        let listed = || {
            execute_command(window.hwnd, CommandId::CommandPalette);
            let listed = app_mut(window.hwnd)
                .command_palette
                .as_ref()
                .unwrap()
                .shown()
                .iter()
                .any(|entry| entry.command == CommandId::NoteNewFolder);
            super::close_command_palette(window.hwnd, false);
            listed
        };
        assert!(crate::window::library_host::folder(window.hwnd).is_none());
        assert!(!listed());
        scratch.install(window.hwnd);
        assert!(listed());
    }
```

- [ ] **Step 2: Run the tests and see them fail**

Run: `cargo test --lib window::commands`
Expected: FAIL to compile: no variant `NoteNewFolder`, no `HeaderButton::NewFolder`, no `NamePurpose::NewFolder`, no `folder_name`, no `library_host::new_folder`.

- [ ] **Step 3: Implement**

**`src/window/commands.rs`:**
- after `    QuickOpen = 191,` add `    NoteNewFolder = 192,`
- in `needs_document`, replace `                | Self::QuickOpen` with `                | Self::QuickOpen` and a new line `                | Self::NoteNewFolder`
- replace `        const COMMANDS: [CommandId; 83] = [` with `        const COMMANDS: [CommandId; 84] = [`, and after `            CommandId::QuickOpen,` add `            CommandId::NoteNewFolder,`

**`src/window/command_palette.rs`:** replace `pub(crate) const ENTRIES: [PaletteEntry; 70] = [` with `pub(crate) const ENTRIES: [PaletteEntry; 71] = [`, and replace

```rust
    entry(
        "Notebook: Toggle favorite",
        CommandId::ToggleNotebookFavorite,
    ),
```

with

```rust
    entry(
        "Notebook: Toggle favorite",
        CommandId::ToggleNotebookFavorite,
    ),
    entry("Notebook: New folder\u{2026}", CommandId::NoteNewFolder),
```

**`src/library/title.rs`:** replace the whole `sanitize_stem` function (doc comment included) with:

```rust
/// A filename stem Windows accepts: no `<>:"/\|?*` or control characters, no trailing dots,
/// spaces or label ellipsis, not a reserved device name, never empty.
pub fn sanitize_stem(name: &str) -> String {
    clean_stem(name).unwrap_or_else(|| "Untitled".to_owned())
}

/// A folder name typed in the name box, cleaned the way `sanitize_stem` cleans a stem, with no
/// extension handling; `None` when nothing is left of it (notebook folders spec §4.1).
pub fn folder_name(input: &str) -> Option<String> {
    clean_stem(input)
}

/// `sanitize_stem`'s cleaning, with `None` for a name that cleans to nothing.
fn clean_stem(name: &str) -> Option<String> {
    let cleaned: String = name
        .chars()
        .filter(|c| !matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'))
        .filter(|c| !c.is_control())
        .collect();
    let trimmed = cleaned
        .trim()
        .trim_end_matches(['.', ' ', '…'])
        .trim()
        .to_owned();
    if trimmed.is_empty() {
        return None;
    }
    // Windows reads "con.txt" and "con .txt" as the device too: neutralise the part before the
    // first dot.
    let device = trimmed.split('.').next().unwrap_or("").trim_end();
    if RESERVED
        .iter()
        .any(|reserved| reserved.eq_ignore_ascii_case(device))
    {
        let (name, rest) = trimmed.split_at(device.len());
        return Some(format!("{name}_{rest}"));
    }
    Some(trimmed)
}
```

**`src/window/name_box.rs`:**
- add `use std::path::{Path, PathBuf};` after `use std::rc::Rc;`
- replace the `NamePurpose` enum and its `impl` with:

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum NamePurpose {
    FirstSave(DocumentId),
    RenameNote(DocumentId),
    /// A new folder in this folder, relative to the notebook (empty for its root).
    NewFolder(PathBuf),
}

impl NamePurpose {
    /// The tab this purpose acts on: the box closes when that tab goes away.
    pub(crate) fn document(&self) -> Option<DocumentId> {
        match self {
            Self::FirstSave(id) | Self::RenameNote(id) => Some(*id),
            Self::NewFolder(_) => None,
        }
    }

    /// For a folder box, the folder that must stay in the tree for the box to stay open: a new
    /// folder's parent (empty for the notebook root). A folder box belongs to no tab.
    pub(crate) fn folder(&self) -> Option<&Path> {
        match self {
            Self::FirstSave(_) | Self::RenameNote(_) => None,
            Self::NewFolder(parent) => Some(parent),
        }
    }
}
```

- replace `focus` with:

```rust
    /// Focuses the field with the name selected up to its extension, so typing replaces the stem.
    /// A folder name has no extension: it is selected in full.
    pub(crate) fn focus(&self) {
        let text = self.text();
        let folder = self
            .purpose
            .as_ref()
            .is_some_and(|purpose| purpose.folder().is_some());
        let end = if folder {
            -1
        } else {
            text.rfind('.')
                .map_or(-1, |dot| text[..dot].encode_utf16().count() as isize)
        };
        unsafe {
            SetFocus(self.edit);
            SendMessageW(self.edit, EM_SETSEL, 0, end);
        }
    }
```

**`src/window/library_host.rs`:**
- after `use crate::library::title;` add `use crate::library::tree::RowKind;`
- after `folder_display_name`, add:

```rust
const NEW_FOLDER: &str = "New folder";
const NO_FOLDER_NAME: &str = "Type a folder name";

/// A new or renamed folder's name is taken by a folder or file (spec §4.1).
fn folder_taken_error(name: &str) -> String {
    format!("A folder or file named \u{201c}{name}\u{201d} already exists")
}

/// A folder named like one the scan skips would vanish at the next rescan (scan §3.1).
fn hidden_folder_error(name: &str) -> String {
    format!("FastPad hides folders named \u{201c}{name}\u{201d}. Choose another name.")
}

/// "in <folder>": `parent`'s own name, or the notebook's at the root (spec §4.1, §4.2).
fn folder_suffix(hwnd: HWND, parent: &Path) -> String {
    match parent.file_name() {
        Some(name) => format!("in {}", name.to_string_lossy()),
        None => format!("in {}", folder_display_name(hwnd)),
    }
}

/// `folder` (absolute, inside the notebook `root`) relative to it; empty for the root itself.
fn relative_folder(root: &Path, folder: &Path) -> PathBuf {
    if library::model::same_path(root, folder) {
        PathBuf::new()
    } else {
        library::record_path(root, folder)
    }
}

/// The Notebook view's New folder button, "New folder here" on a folder row (`parent`, relative
/// to the notebook) and Notebook: New folder… (spec §4.1). The name box opens for a folder in
/// `parent`, else in the selected row's folder, else at the root. Nothing is created before
/// Enter.
pub(crate) fn new_folder(hwnd: HWND, parent: Option<PathBuf>) {
    if !ready_library(hwnd) {
        return;
    }
    let Some(root) = folder(hwnd) else {
        return;
    };
    let parent = parent.unwrap_or_else(|| {
        super::notebook_view::selected_folder(hwnd)
            .map(|selected| relative_folder(&root, &selected))
            .unwrap_or_default()
    });
    let purpose = NamePurpose::NewFolder(parent.clone());
    // New folder again while the box is open for the same folder keeps what was typed.
    if name_box_purpose(hwnd) == Some(purpose.clone()) {
        focus_name_box(hwnd);
        return;
    }
    let name = with_state(hwnd, |state| {
        title::free_name(NEW_FOLDER, "", |candidate| {
            state.is_listed(&parent.join(candidate))
        })
    })
    .unwrap_or_else(|| NEW_FOLDER.to_owned());
    let suffix = folder_suffix(hwnd, &parent);
    open_name_box(hwnd, purpose, &name, suffix, false);
}

/// Enter in the New folder box: creates the folder with the one disk call, then lists it,
/// expands its parent, selects its row and gives the tree the focus.
fn submit_new_folder(hwnd: HWND, parent: &Path, text: &str) {
    let Some(root) = folder(hwnd) else {
        close_name_box(hwnd);
        return;
    };
    let Some(name) = title::folder_name(text) else {
        name_box_error(hwnd, NO_FOLDER_NAME.to_owned());
        return;
    };
    if library::scan::skip_directory(&name) {
        name_box_error(hwnd, hidden_folder_error(&name));
        return;
    }
    let relative = parent.join(&name);
    if with_state(hwnd, |state| state.is_listed(&relative)).unwrap_or(false) {
        name_box_error(hwnd, folder_taken_error(&name));
        return;
    }
    if let Err(error) = std::fs::create_dir(root.join(&relative)) {
        // A file (or a folder the scan skips) may already have the name.
        let error = if error.kind() == std::io::ErrorKind::AlreadyExists {
            folder_taken_error(&name)
        } else {
            format!("FastPad could not create the folder: {error}")
        };
        name_box_error(hwnd, error);
        return;
    }
    with_state(hwnd, |state| state.add_folder(&relative));
    for ancestor in library::tree::ancestors(&relative) {
        set_expanded(hwnd, &ancestor, true);
    }
    close_name_box(hwnd);
    super::side_panel::with_accessible_events(hwnd, || {
        super::side_panel::refresh(hwnd);
        super::notebook_view::select_row(hwnd, &RowKind::Folder(relative.clone()));
    });
    super::notebook_view::focus_tree(hwnd);
}
```

- replace `close_stale_name_box` with:

```rust
/// Closes a name box whose tab is gone or no longer active, or whose first save already
/// happened. A folder box belongs to the notebook, not a tab: it closes when the notebook goes
/// (no state) or when its folder, or a new folder's parent, is no longer in the tree.
pub(crate) fn close_stale_name_box(hwnd: HWND) {
    let Some(purpose) = name_box_purpose(hwnd) else {
        return;
    };
    if let Some(folder) = purpose.folder() {
        let listed = with_state(hwnd, |state| {
            folder.as_os_str().is_empty() || state.is_folder(folder)
        })
        .unwrap_or(false);
        if !listed {
            close_name_box(hwnd);
        }
        return;
    }
    let Some(id) = purpose.document() else {
        return;
    };
    let stale = unsafe { app_ptr(hwnd) }.is_none_or(|app| {
        let tabs = &unsafe { app.as_ref() }.tabs;
        let Some(document) = tabs.document(id) else {
            return true;
        };
        tabs.active().is_none_or(|active| active.id != id)
            || (matches!(purpose, NamePurpose::FirstSave(_)) && document.path.is_some())
    });
    if stale {
        close_name_box(hwnd);
    }
}
```

- in `name_box_submit`, replace the `match purpose { ... }` with:

```rust
    match purpose {
        NamePurpose::FirstSave(id) => submit_first_save(hwnd, id, &text),
        NamePurpose::RenameNote(_) | NamePurpose::NewFolder(_) if !ready_library(hwnd) => {
            close_name_box(hwnd);
        }
        NamePurpose::RenameNote(id) => submit_rename(hwnd, id, &text),
        NamePurpose::NewFolder(parent) => submit_new_folder(hwnd, &parent, &text),
    }
```

- in `install`, replace:

```rust
    // A load or rescan may have seen outside edits: the Search view's query runs again.
    crate::window::text_search_host::notes_reloaded(hwnd);
    super::side_panel::refresh(hwnd);
```

with:

```rust
    // A load or rescan may have seen outside edits: the Search view's query runs again.
    crate::window::text_search_host::notes_reloaded(hwnd);
    // A folder box whose folder the rescan no longer finds closes.
    close_stale_name_box(hwnd);
    super::side_panel::refresh(hwnd);
```

- in `open_checked_folder`, replace:

```rust
    update_folders(hwnd, |folders| folders.push(path.clone()));
    start_load(hwnd);
```

with:

```rust
    // A folder box names a folder of the notebook that just went.
    close_stale_name_box(hwnd);
    update_folders(hwnd, |folders| folders.push(path.clone()));
    start_load(hwnd);
```

**`src/window/notebook_view.rs`:**
- after `const GLYPH_MORE: &str = "\u{E712}";` add `const GLYPH_NEW_FOLDER: &str = "\u{E8F4}";`
- after `const TOOL_MORE: usize = 5;` add `const TOOL_NEW_FOLDER: usize = 6;`
- replace the `HeaderButton` enum with:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HeaderButton {
    Favorite,
    NewNote,
    NewFolder,
    More,
}
```

- in `HeaderLayout`, replace

```rust
    /// Left to right: star, New note, "…".
    pub buttons: [(HeaderButton, RECT); 3],
```

with

```rust
    /// Left to right: star, New note, New folder, "…".
    pub buttons: [(HeaderButton, RECT); 4],
```

- in `header_layout`, replace

```rust
    let buttons = [
        (HeaderButton::Favorite, slot(2)),
        (HeaderButton::NewNote, slot(1)),
        (HeaderButton::More, slot(0)),
    ];
```

with

```rust
    let buttons = [
        (HeaderButton::Favorite, slot(3)),
        (HeaderButton::NewNote, slot(2)),
        (HeaderButton::NewFolder, slot(1)),
        (HeaderButton::More, slot(0)),
    ];
```

- in `tooltip_tools`, after `                HeaderButton::NewNote => (TOOL_NEW, "New note"),` add `                HeaderButton::NewFolder => (TOOL_NEW_FOLDER, "New folder"),`
- in `paint_header`, after `                HeaderButton::NewNote => GLYPH_ADD,` add `                HeaderButton::NewFolder => GLYPH_NEW_FOLDER,`
- in `buttons`, after `                    HeaderButton::NewNote => "New note",` add `                    HeaderButton::NewFolder => "New folder",`
- in `header_clicked`, after `        HeaderButton::NewNote => run(hwnd, CommandId::New),` add `        HeaderButton::NewFolder => run(hwnd, CommandId::NoteNewFolder),`
- in `more_menu`, replace `        let rect = header_layout(view.client(), view.dpi()).buttons[2].1;` with `        let rect = header_layout(view.client(), view.dpi()).buttons[3].1;`
- in `open_context_menu`, replace the `RowKind::Folder(relative) => { ... }` arm with:

```rust
        RowKind::Folder(relative) => {
            let path = root.join(relative);
            let entries = [
                MenuEntry::command("New note here", CommandId::New),
                MenuEntry::command("New folder here", CommandId::NoteNewFolder),
                MenuEntry::command("Reveal in Explorer", CommandId::NoteRevealInExplorer),
            ];
            match super::menus::track_popup(hwnd, &entries, point) {
                Some(CommandId::New) => super::library_host::new_note_in(hwnd, Some(path)),
                Some(CommandId::NoteNewFolder) => {
                    super::library_host::new_folder(hwnd, Some(relative.clone()));
                }
                Some(CommandId::NoteRevealInExplorer) => super::library_host::reveal(hwnd, &path),
                _ => {}
            }
        }
```

- after `selected_folder`, add:

```rust
/// Selects the row showing `kind` and scrolls it into view; false when no row shows it.
pub(crate) fn select_row(hwnd: HWND, kind: &RowKind) -> bool {
    with_view(hwnd, |view| {
        let Some(index) = tree::row_index(&view.rows, kind) else {
            return false;
        };
        view.select(index);
        true
    })
    .unwrap_or(false)
}

/// Gives the tree the keyboard focus, after a folder command closed its name box.
pub(crate) fn focus_tree(hwnd: HWND) {
    focus_panel(hwnd);
}
```

**`src/window/main_window.rs`:**
- in `execute_command_with_note`, after `        CommandId::QuickOpen => open_quick_open(hwnd),` add `        CommandId::NoteNewFolder => crate::window::library_host::new_folder(hwnd, None),`
- in the palette refresh, replace:

```rust
        let sidebar = notes_mode_enabled(hwnd);
        let subset = with_command_palette(hwnd, CommandPalette::subset).flatten();
        let entries = command_palette::filter_entries(&query, |command| {
            subset.is_none_or(|subset| subset.contains(&command))
                && (has_tabs || !command.needs_document())
                && (markdown || !command.is_markdown_preview())
                && (sidebar || !command.is_sidebar())
        });
```

with:

```rust
        let sidebar = notes_mode_enabled(hwnd);
        // New folder needs a notebook, open or loading, to put the folder in (spec §4.1).
        let notebook = crate::window::library_host::folder(hwnd).is_some();
        let subset = with_command_palette(hwnd, CommandPalette::subset).flatten();
        let entries = command_palette::filter_entries(&query, |command| {
            subset.is_none_or(|subset| subset.contains(&command))
                && (has_tabs || !command.needs_document())
                && (markdown || !command.is_markdown_preview())
                && (sidebar || !command.is_sidebar())
                && (notebook || command != CommandId::NoteNewFolder)
        });
```

- [ ] **Step 4: Run the tests and see them pass**

Run: `cargo test --lib window::commands window::command_palette library::title window::notebook_view window::name_box`
Expected: all pass.
Run: `cargo test --lib -- --test-threads=1 new_folder_from_the_header_creates_it_on_disk_and_selects_its_row new_folder_here_creates_it_inside_that_folder_expanded_and_refuses_a_taken_name a_typed_folder_name_is_sanitized_and_empty_or_hidden_names_are_refused a_new_folder_box_survives_a_rescan_but_closes_when_its_parent_goes an_empty_folder_made_on_disk_appears_after_a_rescan_and_goes_with_it the_palette_offers_new_folder_only_while_a_notebook_is_open`
Expected: 6 passed.
Run (regressions): `cargo test --lib -- --test-threads=1 the_context_menu_acts_on_its_row_not_the_active_tab escape_in_the_name_box_cancels_the_save the_name_box_closes_with_its_tab_or_when_another_tab_is_activated the_name_box_closes_once_its_tab_is_saved_another_way saving_again_while_the_box_is_open_keeps_what_was_typed a_rescan_keeps_selection_and_expansion_by_path with_notes_mode_off_there_is_no_sidebar_and_nothing_moves renaming_a_note_renames_its_file_and_keeps_its_metadata`
Expected: all pass.

- [ ] **Step 5: Clippy**

Run: `cargo fmt; cargo clippy --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 6: Commit**

```bash
git add src/window/commands.rs src/window/command_palette.rs src/library/title.rs src/window/name_box.rs src/window/library_host.rs src/window/notebook_view.rs src/window/main_window.rs
git commit -m "feat(sidebar): New folder (command 192) from the Notebook header, a folder's menu and the palette, named in the name box"
```

---

### Task 6: Rename and delete folders, docs

**Files:**
- Modify: `src/platform/files.rs` (`recycle` split, `recycle_folder`, test)
- Modify: `src/window/name_box.rs` (`NamePurpose::RenameFolder`)
- Modify: `src/window/library_host.rs` (imports, `rename_folder`, `submit_rename_folder`, `delete_folder`, `tabs_under`, `already_exists`, `delete_folder_question`, `name_box_submit`, unit test)
- Modify: `src/window/notebook_view.rs` (F2/Del, folder menu, `focused_folder`, `row_index_of`, `select_index`)
- Modify: `src/window/main_window.rs` (`execute_command_with_note` routing, window tests)
- Modify: `README.md`, `docs/superpowers/specs/2026-09-24-notebook-folders-design.md` (§9)

**Interfaces:**
- Consumes: `LibraryState::{rename_folder, remove_folder, notes_under, is_listed, is_folder}` (Task 3); `folder_taken_error`, `hidden_folder_error`, `folder_suffix`, `NO_FOLDER_NAME`, `notebook_view::{select_row, focus_tree}` (Task 5); `Tabs::rebind_path`, `main_window::close_document_without_prompt`, `confirmed`, `save_local`.
- Produces:
  - `pub fn recycle_folder(owner: HWND, path: &Path) -> Result<()>` in `platform::files`
  - `NamePurpose::RenameFolder(PathBuf)`
  - `pub(crate) fn rename_folder(hwnd: HWND, relative: &Path)`, `pub(crate) fn delete_folder(hwnd: HWND, relative: &Path)` in `library_host` (callers check `ready_library`, as for `rename_file`/`delete_file`)
  - `pub(crate) fn focused_folder(hwnd: HWND) -> Option<PathBuf>`, `pub(crate) fn row_index_of(hwnd: HWND, kind: &RowKind) -> Option<usize>`, `pub(crate) fn select_index(hwnd: HWND, index: usize)` in `notebook_view`

**Decisions this task settles:**
- `recycle` stays file-only (its test refuses a directory, note-library spec §14); `recycle_folder` checks for a directory and shares the `SHFileOperationW` call.
- A rename rebinds tabs one by one; on the first failure the ones already moved go back, then the folder is renamed back, and `Another tab already has that file open.` (today's message) shows on the box.
- `Note: Rename` / `Note: Delete` from `execute_command` act on a focused folder row when no note row is the target, so the `needs_document` guard lets them through with no tab open. From the palette they still act on the active tab: the palette records a focused note row, not a folder row.
- After a delete the selection goes to the row that took the folder's index (the next row, or the previous one at the end), set after the refresh, since closing the tabs moves it to the new active note.

- [ ] **Step 1: Write the failing tests**

In `src/platform/files.rs`'s tests add:

```rust
    #[test]
    fn recycling_a_folder_removes_it_with_everything_in_it_and_refuses_anything_else() {
        // Break caught: a folder delete refused because `recycle` takes only files, contents left
        // behind, or a file or relative path accepted as a folder.
        let dir = scratch("recycle-folder");
        let folder = dir.join("old");
        std::fs::create_dir_all(folder.join("inner")).unwrap();
        std::fs::write(folder.join(r"inner\a.md"), "a").unwrap();
        recycle_folder(std::ptr::null_mut(), &folder).unwrap();
        assert!(!folder.exists());
        assert!(dir.exists());
        let file = dir.join("file.md");
        std::fs::write(&file, "x").unwrap();
        assert!(recycle_folder(std::ptr::null_mut(), &file).is_err());
        assert!(file.exists());
        assert!(recycle_folder(std::ptr::null_mut(), std::path::Path::new("old")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
```

In `src/window/library_host.rs`'s tests add:

```rust
    #[test]
    fn the_folder_delete_question_counts_notes_and_unsaved_tabs() {
        // Break caught: an empty folder asked about as if it held notes, "1 notes", or unsaved
        // edits lost without a word (spec §4.3).
        assert_eq!(
            delete_folder_question("Old", 0, 0),
            "Move the folder \u{201c}Old\u{201d} to the Recycle Bin?"
        );
        assert_eq!(
            delete_folder_question("Old", 1, 0),
            "Move \u{201c}Old\u{201d} and its 1 note to the Recycle Bin?"
        );
        assert_eq!(
            delete_folder_question("Old", 3, 1),
            "Move \u{201c}Old\u{201d} and its 3 notes to the Recycle Bin?\n1 open note has unsaved changes, which will be lost."
        );
        assert_eq!(
            delete_folder_question("Old", 3, 2),
            "Move \u{201c}Old\u{201d} and its 3 notes to the Recycle Bin?\n2 open notes have unsaved changes, which will be lost."
        );
    }
```

In `src/window/main_window.rs`'s `mod tests`, append:

```rust
    #[test]
    fn renaming_a_folder_with_an_open_note_rebinds_the_tab_and_keeps_the_notes_pin() {
        // Break caught: a folder rename leaving its open tab on the old path (the next save
        // re-creating the old folder), dropping the note's pin, or losing the row's selection.
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_F2;
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("folder-rename");
        std::fs::create_dir_all(scratch.folder().join("sub")).unwrap();
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        ensure_sidebar(window.hwnd);
        let old = open_note(&window, &scratch, r"sub\a.md", "a");
        execute_command(window.hwnd, CommandId::NoteTogglePin);
        crate::window::notebook_view::rebuild(window.hwnd);
        select_row(window.hwnd, &RowKind::Folder("sub".into()));

        assert!(crate::window::notebook_view::key_down(window.hwnd, VK_F2));
        let name_box = app_mut(window.hwnd).name_box.as_ref().unwrap();
        assert_eq!(
            name_box.purpose(),
            Some(&crate::window::name_box::NamePurpose::RenameFolder("sub".into()))
        );
        assert_eq!(name_box.text(), "sub");
        type_into_name_box(window.hwnd, "Projects");
        crate::window::library_host::name_box_submit(window.hwnd);

        let new = scratch.folder().join(r"Projects\a.md");
        assert!(!name_box_visible(window.hwnd));
        assert!(new.exists());
        assert!(!old.exists());
        assert_eq!(
            app_mut(window.hwnd).tabs.active().unwrap().path.as_deref(),
            Some(new.as_path())
        );
        assert_eq!(
            selected_kind(window.hwnd),
            Some(RowKind::Folder("Projects".into()))
        );
        crate::window::library_host::with_state(window.hwnd, |state| {
            assert!(state.is_pinned(&new));
            assert!(!state.is_folder(std::path::Path::new("sub")));
        });
        crate::window::library_host::flush_now(window.hwnd);
        let reloaded =
            crate::library::load(&scratch.folder(), &scratch.root.join("x.ini"), 0).unwrap();
        assert!(reloaded.is_pinned(&new), "the pin was written under the new path");
    }

    #[test]
    fn renaming_a_folder_moves_its_preview_and_dirty_tabs_without_saving_them() {
        // Break caught: a rename that saves a dirty tab (touching the file's contents), turns the
        // preview into a normal tab, or leaves either on the old path (spec §4.2).
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("folder-rename-tabs");
        std::fs::create_dir_all(scratch.folder().join("sub")).unwrap();
        let a = scratch.note(r"sub\a.md", "a");
        let b = scratch.note(r"sub\b.md", "b");
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        ensure_sidebar(window.hwnd);
        scratch.install(window.hwnd);
        // Autosave would save `a` the moment `b` opens; the rename must leave it dirty.
        execute_command(window.hwnd, CommandId::ToggleFolderAutosave);
        super::open_path(window.hwnd, &a).unwrap();
        editor.set_text("a, edited").unwrap();
        super::open_note(window.hwnd, &b, super::OpenMode::Preview, false).unwrap();
        let a_id = app_mut(window.hwnd).tabs.find_stored_path(&a).unwrap();
        let b_id = app_mut(window.hwnd).tabs.find_stored_path(&b).unwrap();

        crate::window::library_host::rename_folder(window.hwnd, std::path::Path::new("sub"));
        type_into_name_box(window.hwnd, "Moved");
        crate::window::library_host::name_box_submit(window.hwnd);

        let moved = scratch.folder().join("Moved");
        let tabs = &app_mut(window.hwnd).tabs;
        assert_eq!(tabs.document(a_id).unwrap().path, Some(moved.join("a.md")));
        assert!(tabs.document(a_id).unwrap().dirty);
        assert_eq!(tabs.document(b_id).unwrap().path, Some(moved.join("b.md")));
        assert_eq!(tabs.preview_id(), Some(b_id));
        assert_eq!(
            std::fs::read_to_string(moved.join("a.md")).unwrap(),
            "a",
            "nothing was saved"
        );
    }

    #[test]
    fn a_case_only_folder_rename_renames_it_on_disk_and_in_tabs_and_expansion() {
        // Break caught: "sub" → "Sub" refused as a clash with itself, a no-op on NTFS, or the tab
        // and the expanded entry left in the old case.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("folder-rename-case");
        std::fs::create_dir_all(scratch.folder().join("sub")).unwrap();
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        ensure_sidebar(window.hwnd);
        open_note(&window, &scratch, r"sub\a.md", "a");

        crate::window::library_host::rename_folder(window.hwnd, std::path::Path::new("sub"));
        type_into_name_box(window.hwnd, "Sub");
        crate::window::library_host::name_box_submit(window.hwnd);

        let names: Vec<String> = std::fs::read_dir(scratch.folder())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(names.contains(&"Sub".to_owned()), "{names:?}");
        assert!(!name_box_visible(window.hwnd));
        assert_eq!(
            app_mut(window.hwnd).tabs.active().unwrap().path,
            Some(scratch.folder().join(r"Sub\a.md"))
        );
        assert!(
            crate::window::library_host::expanded(window.hwnd)
                .contains(&std::path::PathBuf::from("Sub"))
        );
        assert_eq!(selected_kind(window.hwnd), Some(RowKind::Folder("Sub".into())));
    }

    #[test]
    fn a_folder_rename_an_open_tab_cannot_follow_is_undone() {
        // Break caught: the folder renamed on disk while a tab stays on the old path (its next
        // save re-creating the old folder), or a half-done rename left behind (spec §4.2).
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("folder-rename-undo");
        std::fs::create_dir_all(scratch.folder().join("sub")).unwrap();
        let a = scratch.note(r"sub\a.md", "a");
        let top = scratch.note("top.md", "t");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        ensure_sidebar(window.hwnd);
        scratch.install(window.hwnd);
        super::open_path(window.hwnd, &a).unwrap();
        super::open_path(window.hwnd, &top).unwrap();
        // Another tab already names the path `a` would move to, so `a`'s tab cannot follow.
        let top_id = app_mut(window.hwnd).tabs.find_stored_path(&top).unwrap();
        app_mut(window.hwnd)
            .tabs
            .document_mut(top_id)
            .unwrap()
            .path = Some(scratch.folder().join(r"Moved\a.md"));

        crate::window::library_host::rename_folder(window.hwnd, std::path::Path::new("sub"));
        type_into_name_box(window.hwnd, "Moved");
        crate::window::library_host::name_box_submit(window.hwnd);

        let name_box = app_mut(window.hwnd).name_box.as_ref().unwrap();
        assert!(name_box.is_visible());
        assert_eq!(name_box.error(), Some("Another tab already has that file open."));
        assert!(a.exists());
        assert!(!scratch.folder().join("Moved").exists());
        let a_id = app_mut(window.hwnd).tabs.find_stored_path(&a);
        assert!(a_id.is_some(), "a's tab is back on its old path");
        crate::window::library_host::with_state(window.hwnd, |state| {
            assert!(state.is_folder(std::path::Path::new("sub")));
            assert!(!state.is_folder(std::path::Path::new("Moved")));
        });
    }

    #[test]
    fn a_folder_rename_onto_a_sibling_is_refused_and_the_same_name_changes_nothing() {
        // Break caught: a rename onto an existing sibling (in another case) merging or failing
        // oddly, or Note: Rename on a focused folder row renaming the active tab instead.
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetFocus, SetFocus};
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("folder-rename-clash");
        std::fs::create_dir_all(scratch.folder().join("sub")).unwrap();
        std::fs::create_dir_all(scratch.folder().join("Other")).unwrap();
        scratch.note(r"sub\a.md", "a");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        ensure_sidebar(window.hwnd);
        scratch.install(window.hwnd);
        crate::window::notebook_view::rebuild(window.hwnd);
        select_row(window.hwnd, &RowKind::Folder("sub".into()));
        let (_, panel) = sidebar_windows(window.hwnd);
        unsafe { SetFocus(panel) };
        assert_eq!(unsafe { GetFocus() }, panel);

        execute_command(window.hwnd, CommandId::NoteRename);
        assert_eq!(
            app_mut(window.hwnd).name_box.as_ref().unwrap().purpose(),
            Some(&crate::window::name_box::NamePurpose::RenameFolder("sub".into()))
        );
        type_into_name_box(window.hwnd, "other");
        crate::window::library_host::name_box_submit(window.hwnd);
        let name_box = app_mut(window.hwnd).name_box.as_ref().unwrap();
        assert!(name_box.is_visible());
        assert_eq!(
            name_box.error(),
            Some("A folder or file named \u{201c}other\u{201d} already exists")
        );
        assert!(scratch.folder().join(r"sub\a.md").exists());

        type_into_name_box(window.hwnd, "sub");
        crate::window::library_host::name_box_submit(window.hwnd);
        assert!(!name_box_visible(window.hwnd));
        assert!(scratch.folder().join(r"sub\a.md").exists());
    }

    #[test]
    fn a_folder_name_box_selects_the_whole_name_even_with_a_dot() {
        // Break caught: "v1.2" opening with only "v1" selected, as a file name's stem would be,
        // so typing keeps ".2" (spec §4.2).
        use windows_sys::Win32::UI::Controls::EM_GETSEL;
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("folder-rename-dot");
        std::fs::create_dir_all(scratch.folder().join("v1.2")).unwrap();
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        ensure_sidebar(window.hwnd);
        scratch.install(window.hwnd);

        crate::window::library_host::rename_folder(window.hwnd, std::path::Path::new("v1.2"));

        let edit = app_mut(window.hwnd).name_box.as_ref().unwrap().edit_hwnd();
        let (mut start, mut end) = (0_u32, 0_u32);
        unsafe {
            SendMessageW(
                edit,
                EM_GETSEL,
                &mut start as *mut u32 as usize,
                &mut end as *mut u32 as isize,
            )
        };
        assert_eq!((start, end), (0, 4));
    }

    #[test]
    fn a_folder_renamed_while_a_rescan_runs_keeps_its_new_name_once_the_rescan_lands() {
        // Break caught: a rescan that listed the folder before the rename bringing the old row
        // back, with its note at a path that is gone, and hiding the new one (spec §3.2).
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("folder-rename-rescan");
        std::fs::create_dir_all(scratch.folder().join("sub")).unwrap();
        scratch.note(r"sub\a.md", "a");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        ensure_sidebar(window.hwnd);
        app_mut(window.hwnd).library.data_dir = Some(scratch.data());
        scratch.install(window.hwnd);
        crate::window::library_host::request_rescan(window.hwnd);
        assert!(app_mut(window.hwnd).library.scanning);

        crate::window::library_host::rename_folder(window.hwnd, std::path::Path::new("sub"));
        type_into_name_box(window.hwnd, "Moved");
        crate::window::library_host::name_box_submit(window.hwnd);
        pump_until(window.hwnd, || !app_mut(window.hwnd).library.scanning);

        crate::window::library_host::with_state(window.hwnd, |state| {
            assert!(state.is_folder(std::path::Path::new("Moved")));
            assert!(!state.is_folder(std::path::Path::new("sub")));
            let notes: Vec<_> = state.notes.iter().map(|note| note.path.clone()).collect();
            assert_eq!(notes, [std::path::PathBuf::from(r"Moved\a.md")]);
        });
        row_of(window.hwnd, &RowKind::Folder("Moved".into()));
        assert!(
            crate::library::tree::row_index(
                &notebook_view(window.hwnd).rows,
                &RowKind::Folder("sub".into())
            )
            .is_none()
        );
    }

    #[test]
    fn deleting_a_folder_recycles_it_closes_its_tabs_and_removes_its_rows() {
        // Break caught: a folder delete that leaves its rows, its open tab on a file that is gone
        // or its pinned note's record looking alive; one that deletes on Cancel; or a selection
        // that jumps away from where the folder was (spec §4.3).
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("folder-delete");
        std::fs::create_dir_all(scratch.folder().join(r"sub\inner")).unwrap();
        std::fs::create_dir_all(scratch.folder().join("empty")).unwrap();
        scratch.note(r"sub\inner\b.md", "b");
        scratch.note("top.md", "t");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        ensure_sidebar(window.hwnd);
        let a = open_note(&window, &scratch, r"sub\a.md", "a");
        execute_command(window.hwnd, CommandId::NoteTogglePin);
        crate::window::notebook_view::rebuild(window.hwnd);
        let menu = |kind: &RowKind, answer: CommandId| {
            crate::window::menus::answer_next_popup_menu(move |_| Some(answer));
            let index = row_of(window.hwnd, kind);
            crate::window::notebook_view::open_context_menu(window.hwnd, index, None);
        };
        crate::window::modal::take_last_confirm();

        crate::window::answer_next_confirm(|_| false);
        menu(&RowKind::Folder("empty".into()), CommandId::NoteDelete);
        assert_eq!(
            crate::window::modal::take_last_confirm().as_deref(),
            Some("Move the folder \u{201c}empty\u{201d} to the Recycle Bin?")
        );
        assert!(scratch.folder().join("empty").exists(), "Cancel deletes nothing");

        crate::window::answer_next_confirm(|_| true);
        menu(&RowKind::Folder("sub".into()), CommandId::NoteDelete);
        assert_eq!(
            crate::window::modal::take_last_confirm().as_deref(),
            Some("Move \u{201c}sub\u{201d} and its 2 notes to the Recycle Bin?")
        );
        assert!(!scratch.folder().join("sub").exists());
        assert!(
            tab_paths(window.hwnd)
                .iter()
                .all(|path| path.as_deref() != Some(a.as_path()))
        );
        let rows = &notebook_view(window.hwnd).rows;
        for gone in ["sub", r"sub\inner"] {
            assert!(
                crate::library::tree::row_index(rows, &RowKind::Folder(gone.into())).is_none(),
                "{gone}"
            );
        }
        assert_eq!(selected_kind(window.hwnd), Some(RowKind::Note("top.md".into())));
        crate::window::library_host::with_state(window.hwnd, |state| {
            let record = state.record_for(&a).unwrap();
            assert!(record.deleted);
            assert!(state.local.missing_since(record.id).is_some());
            let notes: Vec<_> = state.notes.iter().map(|note| note.path.clone()).collect();
            assert_eq!(notes, [std::path::PathBuf::from("top.md")]);
        });
    }

    #[test]
    fn deleting_a_folder_that_holds_the_only_tab_and_the_name_box_target_warns_and_closes_both() {
        // Break caught: unsaved edits discarded without a word, the last tab left on a deleted
        // file, or a New folder box still offering to create inside a folder that is gone.
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
        crate::window::library_host::new_folder(window.hwnd, Some("sub".into()));
        assert!(name_box_visible(window.hwnd));
        crate::window::notebook_view::rebuild(window.hwnd);
        select_row(window.hwnd, &RowKind::Folder("sub".into()));
        crate::window::modal::take_last_confirm();
        crate::window::answer_next_confirm(|_| true);

        assert!(crate::window::notebook_view::key_down(window.hwnd, VK_DELETE));

        assert_eq!(
            crate::window::modal::take_last_confirm().as_deref(),
            Some(
                "Move \u{201c}sub\u{201d} and its 1 note to the Recycle Bin?\n1 open note has unsaved changes, which will be lost."
            )
        );
        assert!(!a.exists());
        assert_eq!(super::tab_count(window.hwnd), 0);
        assert!(!name_box_visible(window.hwnd));
    }
```

- [ ] **Step 2: Run the tests and see them fail**

Run: `cargo test --lib platform::files`
Expected: FAIL to compile: cannot find `recycle_folder`, `delete_folder_question`, `NamePurpose::RenameFolder`, `library_host::rename_folder`.

- [ ] **Step 3: Implement**

**`src/platform/files.rs`:** replace the whole `recycle` function (from `pub fn recycle(owner: HWND, path: &Path) -> Result<()> {` to its closing `}`; its doc comment stays) with:

```rust
pub fn recycle(owner: HWND, path: &Path) -> Result<()> {
    if !path.is_absolute() {
        return Err(crate::FastPadError::Invariant(
            "recycle requires an absolute path",
        ));
    }
    if !path.is_file() {
        return Err(crate::FastPadError::Invariant(
            "the file to delete does not exist",
        ));
    }
    shell_recycle(owner, path)
}

/// Sends one folder, with everything in it, to the Recycle Bin, the way `recycle` sends a file
/// (notebook folders spec §4.3). A relative path, or anything but a directory, is refused before
/// anything is sent to the shell.
pub fn recycle_folder(owner: HWND, path: &Path) -> Result<()> {
    if !path.is_absolute() {
        return Err(crate::FastPadError::Invariant(
            "recycle requires an absolute path",
        ));
    }
    if !path.is_dir() {
        return Err(crate::FastPadError::Invariant(
            "the folder to delete does not exist",
        ));
    }
    shell_recycle(owner, path)
}

/// `SHFileOperationW`'s delete to the Recycle Bin, shared by `recycle` and `recycle_folder`.
fn shell_recycle(owner: HWND, path: &Path) -> Result<()> {
    // SHFileOperationW takes a list ending in two NULs.
    let mut from = wide(path);
    from.push(0);
    let mut operation: SHFILEOPSTRUCTW = unsafe { std::mem::zeroed() };
    operation.hwnd = owner;
    operation.wFunc = FO_DELETE;
    operation.pFrom = from.as_ptr();
    operation.fFlags =
        (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_SILENT | FOF_NOERRORUI | FOF_WANTNUKEWARNING)
            as _;
    let status = unsafe { SHFileOperationW(&mut operation) };
    if operation.fAnyOperationsAborted != 0 {
        return Err(crate::FastPadError::Invariant("recycle aborted"));
    }
    if status != 0 {
        return Err(crate::FastPadError::Win32(status as u32));
    }
    Ok(())
}
```

**`src/window/name_box.rs`:** in `NamePurpose`, after the `NewFolder(PathBuf),` variant add:

```rust
    /// Renaming this folder, relative to the notebook.
    RenameFolder(PathBuf),
```

In `document`, replace `            Self::NewFolder(_) => None,` with `            Self::NewFolder(_) | Self::RenameFolder(_) => None,`. In `folder`, after `            Self::NewFolder(parent) => Some(parent),` add `            Self::RenameFolder(folder) => Some(folder),`, and extend its doc comment's first sentence to `... a new folder's parent (empty for the notebook root), or the folder being renamed.`

**`src/window/library_host.rs`:**
- replace `use windows_sys::Win32::Foundation::{HWND, LPARAM};` with `use windows_sys::Win32::Foundation::{ERROR_ALREADY_EXISTS, ERROR_FILE_EXISTS, HWND, LPARAM};`
- in `name_box_submit`, replace the `match purpose { ... }` with:

```rust
    match purpose {
        NamePurpose::FirstSave(id) => submit_first_save(hwnd, id, &text),
        NamePurpose::RenameNote(_) | NamePurpose::NewFolder(_) | NamePurpose::RenameFolder(_)
            if !ready_library(hwnd) =>
        {
            close_name_box(hwnd);
        }
        NamePurpose::RenameNote(id) => submit_rename(hwnd, id, &text),
        NamePurpose::NewFolder(parent) => submit_new_folder(hwnd, &parent, &text),
        NamePurpose::RenameFolder(folder) => submit_rename_folder(hwnd, &folder, &text),
    }
```

- after `submit_new_folder`, add:

```rust
/// Whether a failed `MoveFileExW` means the target name is taken.
fn already_exists(error: &crate::FastPadError) -> bool {
    matches!(
        error,
        crate::FastPadError::Win32(code) if *code == ERROR_ALREADY_EXISTS || *code == ERROR_FILE_EXISTS
    )
}

/// The open tabs whose file is inside `folder` (absolute), with their stored paths and whether
/// each has unsaved edits. Read from the tabs' stored paths: no disk access.
fn tabs_under(hwnd: HWND, folder: &Path) -> Vec<(crate::document::DocumentId, PathBuf, bool)> {
    unsafe { app_ptr(hwnd) }
        .map(|app| {
            unsafe { app.as_ref() }
                .tabs
                .documents()
                .filter_map(|document| {
                    let path = document.path.as_ref()?;
                    library::is_inside(folder, path)
                        .then(|| (document.id, path.clone(), document.dirty))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// F2 or Rename… on a folder row (spec §4.2): the name box, prefilled with the folder's name.
pub(crate) fn rename_folder(hwnd: HWND, relative: &Path) {
    let Some(name) = relative
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
    else {
        return;
    };
    let parent = relative.parent().map(Path::to_path_buf).unwrap_or_default();
    let suffix = folder_suffix(hwnd, &parent);
    open_name_box(
        hwnd,
        NamePurpose::RenameFolder(relative.to_path_buf()),
        &name,
        suffix,
        false,
    );
}

/// Enter in the folder rename box: renames the folder with the one disk call, never onto
/// another name, then rebinds the open tabs under it and follows it in the library. A tab that
/// cannot follow undoes the whole rename.
fn submit_rename_folder(hwnd: HWND, old: &Path, text: &str) {
    let Some(root) = folder(hwnd) else {
        close_name_box(hwnd);
        return;
    };
    let Some(name) = title::folder_name(text) else {
        name_box_error(hwnd, NO_FOLDER_NAME.to_owned());
        return;
    };
    let parent = old.parent().map(Path::to_path_buf).unwrap_or_default();
    let new = parent.join(&name);
    if new.as_os_str() == old.as_os_str() {
        close_name_box(hwnd);
        return;
    }
    if library::scan::skip_directory(&name) {
        name_box_error(hwnd, hidden_folder_error(&name));
        return;
    }
    // A change of letter case only names the same folder, so it is not a clash.
    let case_only = library::model::same_path(&new, old);
    if !case_only && with_state(hwnd, |state| state.is_listed(&new)).unwrap_or(false) {
        name_box_error(hwnd, folder_taken_error(&name));
        return;
    }
    let (old_path, new_path) = (root.join(old), root.join(&new));
    if let Err(error) = crate::platform::files::rename_no_replace(&old_path, &new_path) {
        let error = if !case_only && already_exists(&error) {
            folder_taken_error(&name)
        } else {
            format!("FastPad could not rename the folder: {error}")
        };
        name_box_error(hwnd, error);
        return;
    }
    let tabs = tabs_under(hwnd, &old_path);
    let mut moved = Vec::with_capacity(tabs.len());
    let mut failed = false;
    for (id, path, _) in tabs {
        let target = new_path.join(library::record_path(&old_path, &path));
        let rebound = unsafe { app_ptr(hwnd) }.is_some_and(|mut app| {
            unsafe { app.as_mut() }
                .tabs
                .rebind_path(id, target)
                .is_ok()
        });
        if !rebound {
            failed = true;
            break;
        }
        moved.push((id, path));
    }
    if failed {
        // Undo, so the tabs and the disk agree: the tabs that moved go back, then the folder.
        for (id, path) in moved.into_iter().rev() {
            if let Some(mut app) = unsafe { app_ptr(hwnd) } {
                let _ = unsafe { app.as_mut() }.tabs.rebind_path(id, path);
            }
        }
        let _ = crate::platform::files::rename_no_replace(&new_path, &old_path);
        name_box_error(hwnd, "Another tab already has that file open.".to_owned());
        return;
    }
    with_state(hwnd, |state| state.rename_folder(old, &new));
    save_local(
        hwnd,
        LocalWrite {
            wait: false,
            force: false,
        },
    );
    schedule_write(hwnd);
    close_name_box(hwnd);
    super::main_window::invalidate_title_strip(hwnd);
    super::side_panel::with_accessible_events(hwnd, || {
        super::side_panel::refresh(hwnd);
        super::notebook_view::select_row(hwnd, &RowKind::Folder(new.clone()));
    });
    super::notebook_view::focus_tree(hwnd);
}

/// The Delete confirmation for a folder named `name` holding `notes` listed notes, `dirty` of
/// its open tabs with unsaved changes (spec §4.3).
fn delete_folder_question(name: &str, notes: usize, dirty: usize) -> String {
    let mut question = match notes {
        0 => format!("Move the folder \u{201c}{name}\u{201d} to the Recycle Bin?"),
        1 => format!("Move \u{201c}{name}\u{201d} and its 1 note to the Recycle Bin?"),
        _ => format!("Move \u{201c}{name}\u{201d} and its {notes} notes to the Recycle Bin?"),
    };
    match dirty {
        0 => {}
        1 => question.push_str("\n1 open note has unsaved changes, which will be lost."),
        _ => question.push_str(&format!(
            "\n{dirty} open notes have unsaved changes, which will be lost."
        )),
    }
    question
}

/// Del or Delete… on a folder row (spec §4.3): after a confirm, sends the folder and everything
/// in it to the Recycle Bin, closes the tabs under it without asking, and drops its notes, their
/// records flagged deleted. A failure changes nothing in the library.
pub(crate) fn delete_folder(hwnd: HWND, relative: &Path) {
    let Some(root) = folder(hwnd) else {
        return;
    };
    let absolute = root.join(relative);
    let notes = with_state(hwnd, |state| state.notes_under(relative)).unwrap_or(0);
    let tabs = tabs_under(hwnd, &absolute);
    let dirty = tabs.iter().filter(|(_, _, dirty)| *dirty).count();
    let name = relative.file_name().unwrap_or_default().to_string_lossy();
    if !confirmed(hwnd, &delete_folder_question(&name, notes, dirty)) {
        return;
    }
    let row = super::notebook_view::row_index_of(hwnd, &RowKind::Folder(relative.to_path_buf()));
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    // The shell may show its own modal warning (a permanent delete), owned by this window.
    let recycled = crate::platform::files::recycle_folder(hwnd, &absolute);
    if !identity.is_live_for(hwnd) {
        return;
    }
    if let Err(error) = recycled {
        push_notice(
            hwnd,
            format!("FastPad could not delete {}: {error}", absolute.display()),
        );
        return;
    }
    for (id, _, _) in tabs {
        super::main_window::close_document_without_prompt(hwnd, id);
    }
    with_state(hwnd, |state| state.remove_folder(relative, library::now_unix()));
    save_local(
        hwnd,
        LocalWrite {
            wait: false,
            force: false,
        },
    );
    schedule_write(hwnd);
    close_stale_name_box(hwnd);
    super::side_panel::with_accessible_events(hwnd, || {
        super::side_panel::refresh(hwnd);
        // The row that took the folder's place: the next one, or the previous at the end.
        if let Some(row) = row {
            super::notebook_view::select_index(hwnd, row);
        }
    });
}
```

**`src/window/notebook_view.rs`:**
- after `focused_note`, add:

```rust
/// The selected folder row's path, relative to the notebook, while the panel has the keyboard
/// focus: Rename and Delete act on it (notebook folders spec §4.2, §4.3).
pub(crate) fn focused_folder(hwnd: HWND) -> Option<PathBuf> {
    with_view(hwnd, |view| {
        let focused = unsafe { GetFocus() } == view.panel;
        match view.list.selected.map(|index| view.target(index)) {
            Some(Target::Row(TreeRow {
                kind: RowKind::Folder(relative),
                ..
            })) if focused => Some(relative),
            _ => None,
        }
    })
    .flatten()
}
```

- after `focus_tree`, add:

```rust
/// The index of the row showing `kind`, if one does.
pub(crate) fn row_index_of(hwnd: HWND, kind: &RowKind) -> Option<usize> {
    with_view(hwnd, |view| tree::row_index(&view.rows, kind)).flatten()
}

/// Selects row `index` (the last row when past the end) and scrolls it into view.
pub(crate) fn select_index(hwnd: HWND, index: usize) {
    with_view(hwnd, |view| {
        if view.mode == Mode::Tree {
            view.select(index);
        }
    });
}
```

- in `open_context_menu`, replace the `RowKind::Folder(relative) => { ... }` arm (Task 5's) with:

```rust
        RowKind::Folder(relative) => {
            let path = root.join(relative);
            let entries = [
                MenuEntry::command("New note here", CommandId::New),
                MenuEntry::command("New folder here", CommandId::NoteNewFolder),
                MenuEntry::Separator,
                MenuEntry::command("Rename...\tF2", CommandId::NoteRename),
                MenuEntry::command("Reveal in Explorer", CommandId::NoteRevealInExplorer),
                MenuEntry::Separator,
                MenuEntry::command("Delete...\tDel", CommandId::NoteDelete),
            ];
            match super::menus::track_popup(hwnd, &entries, point) {
                Some(CommandId::New) => super::library_host::new_note_in(hwnd, Some(path)),
                Some(CommandId::NoteNewFolder) => {
                    super::library_host::new_folder(hwnd, Some(relative.clone()));
                }
                Some(CommandId::NoteRename) if super::library_host::ready_library(hwnd) => {
                    super::library_host::rename_folder(hwnd, relative);
                }
                Some(CommandId::NoteRevealInExplorer) => super::library_host::reveal(hwnd, &path),
                Some(CommandId::NoteDelete) if super::library_host::ready_library(hwnd) => {
                    super::library_host::delete_folder(hwnd, relative);
                }
                _ => {}
            }
        }
```

- in `key_down`, replace the `VK_F2 | VK_DELETE => { ... }` arm with:

```rust
        VK_F2 | VK_DELETE => {
            let kind = with_view(hwnd, |view| match view.target(selected) {
                Target::Row(row) => Some(row.kind),
                _ => None,
            })
            .flatten();
            let Some(root) = super::library_host::folder(hwnd) else {
                return true;
            };
            match kind {
                Some(RowKind::Note(relative)) if super::library_host::ready_library(hwnd) => {
                    let path = root.join(relative);
                    if key == VK_F2 {
                        super::library_host::rename_file(hwnd, &path);
                    } else {
                        super::library_host::delete_file(hwnd, &path);
                    }
                }
                Some(RowKind::Folder(relative)) if super::library_host::ready_library(hwnd) => {
                    if key == VK_F2 {
                        super::library_host::rename_folder(hwnd, &relative);
                    } else {
                        super::library_host::delete_folder(hwnd, &relative);
                    }
                }
                _ => {}
            }
            true
        }
```

**`src/window/main_window.rs`** (`execute_command_with_note`):
- replace:

```rust
    // A focused (or recorded) note lets a note-scoped command through even with no tab open.
    if command.needs_document() && tab_count(hwnd) == 0 && tree_note.is_none() {
        return;
    }
```

with:

```rust
    // Rename and Delete on a focused folder row act on the folder (notebook folders spec §4.2).
    let tree_folder = (matches!(command, CommandId::NoteRename | CommandId::NoteDelete)
        && tree_note.is_none())
    .then(|| crate::window::notebook_view::focused_folder(hwnd))
    .flatten();
    // A focused (or recorded) note or folder lets a note-scoped command through even with no
    // tab open.
    if command.needs_document()
        && tab_count(hwnd) == 0
        && tree_note.is_none()
        && tree_folder.is_none()
    {
        return;
    }
```

- replace the `CommandId::NoteRename` and `CommandId::NoteDelete` arms with:

```rust
        CommandId::NoteRename => {
            if crate::window::library_host::ready_library(hwnd) {
                match (&tree_note, &tree_folder) {
                    (Some(path), _) => crate::window::library_host::rename_file(hwnd, path),
                    (None, Some(folder)) => {
                        crate::window::library_host::rename_folder(hwnd, folder);
                    }
                    (None, None) => crate::window::library_host::rename_note(hwnd),
                }
            }
        }
        CommandId::NoteDelete => {
            if crate::window::library_host::ready_library(hwnd) {
                match (&tree_note, &tree_folder) {
                    (Some(path), _) => crate::window::library_host::delete_file(hwnd, path),
                    (None, Some(folder)) => {
                        crate::window::library_host::delete_folder(hwnd, folder);
                    }
                    (None, None) => crate::window::library_host::delete_note(hwnd),
                }
            }
        }
```

**`README.md`:** in "### Notes and notebooks", after the `- **Ctrl+P** opens a note …` bullet, add:

```markdown
- **New folder** in the Notebook view's header, or **New folder here** on a folder's menu,
  names a folder in the same inline box. **F2** renames a folder and **Del** sends it, with its
  notes, to the Recycle Bin. Empty folders show in the tree, and each note has a coloured icon
  for its type.
```

**Spec:** append to `docs/superpowers/specs/2026-09-24-notebook-folders-design.md`:

```markdown

## 9. Implementation notes

- **Recycling a folder** uses a new `platform::files::recycle_folder`: `recycle` takes only a file (a directory is refused, note-library spec §14), so the folder variant checks for a directory and shares its `SHFileOperationW` call.
- **Wording:** the confirmations and the name-taken error use typographic quotes (“ ”), like the note Delete confirmation. Counts read naturally: `1 note` / `2 notes`, `1 open note has` / `2 open notes have`.
- **Names the scan hides are refused:** a dot-folder, `node_modules`, `target`, `bin` or `obj` would vanish at the next rescan (§3.1), so New folder and Rename refuse them with `FastPad hides folders named “<name>”. Choose another name.`
- **Name clashes** are checked in memory against the tree's folders and the listed notes. Any other file, or a folder the scan skips, with that name makes `create_dir` or `MoveFileExW` refuse, and the same message shows. `New folder N` counts listed folders only.
- **Rescans racing folder commands:** each folder change is also kept as a `FolderChange` until the next rescan starts, and replayed onto that rescan's folders, notes and tree when it is merged, before the touched notes are re-checked. A rescan that listed the folders before the change cannot bring the old row back. The replay is idempotent and touches no disk; records come from the pending `Relocate`/`SetDeleted` operations, and expanded folders from the live state.
- **`touched` entries** under a renamed folder are rewritten, and those under a deleted folder dropped, so a merge makes no stat for them.
- **A folder name box closes** when its folder (or a new folder's parent) is no longer in the tree, checked after every load or rescan, after a notebook switch and after a folder delete. The check reads the tree, so a folder past `FOLDER_LIMIT` that holds notes still counts.
- **Tab rebinds** go through `Tabs::rebind_path`, whose collision check canonicalizes the open tabs' paths, as a note rename's does. A failed rebind undoes the ones before it, renames the folder back and shows today's `Another tab already has that file open.`
- **Focus:** Enter in a folder box moves the focus to the tree, on the new or renamed row; Escape returns it to the editor, as for the note name box. The folder name is selected in full, dots included (`v1.2`).
- **The suffix** names the parent's own name (`in inner`, not `in outer\inner`), or the notebook at the root.
- **The header** reads, left to right: the star, New note, New folder, "…".
- **The empty state** ("No notes in <notebook> yet.") shows only when the notebook has no notes and no folders.
- **Rename and Delete from the palette** still act on the active tab's note: the palette records a focused note row when it opens, not a folder row. F2, Del and the folder menu act on folders, and so do the commands run with a folder row focused.
- **After a delete** the selection goes to the row that took the folder's index, set after the refresh, because closing the folder's tabs moves it to the new active note first.
- **Icon contrast:** the weakest pair is Latte yellow on the Light theme's selection, about 1.7:1. A test keeps every icon colour at 1.5:1 or more on the selection, inactive selection, hover and panel backgrounds of every theme. In high contrast an icon takes the row's muted colour, which on a focused selected row is the selection text colour, as the other glyphs do.
- **Palette placement:** `Notebook: New folder…` follows `Notebook: Toggle favorite` and is listed only while a notebook is open (or loading).
```

- [ ] **Step 4: Run the tests and see them pass**

Run: `cargo test --lib platform::files window::library_host window::name_box window::notebook_view`
Expected: all pass.
Run: `cargo test --lib -- --test-threads=1 renaming_a_folder_with_an_open_note_rebinds_the_tab_and_keeps_the_notes_pin renaming_a_folder_moves_its_preview_and_dirty_tabs_without_saving_them a_case_only_folder_rename_renames_it_on_disk_and_in_tabs_and_expansion a_folder_rename_an_open_tab_cannot_follow_is_undone a_folder_rename_onto_a_sibling_is_refused_and_the_same_name_changes_nothing a_folder_name_box_selects_the_whole_name_even_with_a_dot a_folder_renamed_while_a_rescan_runs_keeps_its_new_name_once_the_rescan_lands deleting_a_folder_recycles_it_closes_its_tabs_and_removes_its_rows deleting_a_folder_that_holds_the_only_tab_and_the_name_box_target_warns_and_closes_both`
Expected: 9 passed.
Run (regressions): `cargo test --lib -- --test-threads=1 f2_on_a_note_row_that_is_not_open_opens_it_and_the_rename_box deleting_a_note_asks_then_recycles_it_and_keeps_its_record_hidden palette_commands_act_on_the_row_focused_when_the_palette_opened the_context_menu_acts_on_its_row_not_the_active_tab recycle_rejects_a_directory`
Expected: all pass.

- [ ] **Step 5: Clippy**

Run: `cargo fmt; cargo clippy --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 6: Commit**

```bash
git add src/platform/files.rs src/window/name_box.rs src/window/library_host.rs src/window/notebook_view.rs src/window/main_window.rs README.md docs/superpowers/specs/2026-09-24-notebook-folders-design.md
git commit -m "feat(sidebar): rename (F2) and delete (Del) folders, rebinding or closing their open tabs; README and the spec's implementation notes"
```

---

## Final review

- [ ] Run the full suite once: `cargo test -- --test-threads=1`. Expected: all pass.
- [ ] Run `cargo clippy --all-targets -- -D warnings`. Expected: no warnings.
- [ ] Re-run the bench from Task 1, Step 0. Expected: `cold_ms`, `warm_median_ms` and `tree_build_ms` within 110% of the Step 0 baseline.
- [ ] Before any live check of the real exe, back up `%LOCALAPPDATA%\FastPad\fastpad.ini` and restore it afterwards.
