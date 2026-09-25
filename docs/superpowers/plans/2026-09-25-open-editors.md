# Open Editors and copying into the notebook Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The Notebook panel gets a VS Code-style Open Editors section above the notebook, which becomes a collapsible root row. Dragging an Open Editors row, or files from Windows Explorer, onto the tree copies them into the folder under the pointer.

**Architecture:**
- Three new pure modules decide things without a window:
  - `window::notebook_layout`: where the title band, the Open Editors header and rows, the root row and the body go.
  - `window::panel_cursor`: how the keyboard moves one selection through the whole panel.
  - `window::tree_copy`: what a drop copies where, what it refuses, what clashes, and the notice wording.
- `window::open_editors` builds one row per tab and paints it.
- `notebook_view` loses its header band and the tree's unsaved rows, and paints, hit-tests and handles the new sections.
- The tree drag's source becomes `DragSource { Row, Tab, Files }`:
  - a `Row` still moves;
  - a `Tab` (an Open Editors row) and `Files` (an Explorer drag) copy.
- `window::copy_host` runs a copy:
  - it plans on the UI thread;
  - it asks about each clash and recycles on OK;
  - it copies on one queued worker thread (`platform::files::copy_tree`);
  - it posts the result back for indexing, selection, reloads and notices.
- `window::panel_drop` is an OLE `IDropTarget` on the panel. It shares the COM plumbing moved out of `editor::file_drop` into `platform::ole_drop`.

**Tech Stack:** Rust 2024, windows-sys 0.61 (raw Win32, OLE, Shell), the GDI owner-drawn sidebar.

**Spec:** `docs/superpowers/specs/2026-09-25-open-editors-design.md`

## Global Constraints

- **Latency:**
  - Nothing new runs before first paint or first input.
  - The panel's OLE drop target is registered in `BUILD_CHROME` (after first paint), or when notes mode is turned on later.
  - Open Editors rows are rebuilt from the tab list in memory: O(tabs), with no disk access.
  - All copying runs on the copy worker.
  - The plan, the clash prompts and the Recycle Bin step of an answered prompt run on the UI thread, as Delete does. The plan does one `exists` check per top-level item.
- **An Explorer drop never waits on the UI thread.** `Drop` posts `WM_FASTPAD_PANEL_DROPPED` and returns. The prompts and the copy start from that message.
- **Never overwrite without asking.** Files are copied with `CopyFileExW` and `COPY_FILE_FAIL_IF_EXISTS`, and folders are created with `create_dir` (which fails if the folder exists). A replace always recycles first: `platform::files::recycle` or `recycle_folder`.
- **Never recycle the source.** A destination that is the source, or holds the source, is refused before any prompt (Review Focus 1 and 2).
- **App-borrow rule:** while a `&mut` from `with_view`, `with_state`, `with_host` or `app_ptr` is held, never call any of these:
  - `SetFocus`, `SetCapture`, `ReleaseCapture`, `SetTimer`, `KillTimer`, `SetCursor`;
  - `CreateWindowExW`, `UpdateWindow`, `SetWindowTextW` on a child, `GetWindowTextW`;
  - `SendMessageW` to another window, `RegisterDragDrop`, `modal::*`, or a document swap.
  - Note that `ReleaseCapture` sends `WM_CAPTURECHANGED` to the panel synchronously.
- **Tests never touch the real profile** (`%LOCALAPPDATA%\FastPad`) **or the real Documents folder.** Window tests use `LibraryScratch` under `%TEMP%`, and disk tests use their own `scratch` folder under `%TEMP%`.
- **Test runs:**
  - Compile with `cargo clippy --all-targets -- -D warnings` and run only the named tests while working.
  - Window tests need `-- --test-threads=1`.
  - The full suite (`cargo test -- --test-threads=1`) runs only at the final review.
  - If the user's FastPad is running, use a separate `CARGO_TARGET_DIR`: a running FastPad locks `target/debug/fastpad.exe`, and the e2e tests fail.
- **No backward compatibility.** No migrations, no compat readers.
- **Commits:** no attribution lines, and run `cargo fmt` before each commit.
- **Never modify** `assets/fastpad-icon.svg`, `assets/fastpad.ico`, `src/preview/images.rs` or `src/preview/svg.rs`.
- **Never search the whole disk.** Crate sources are in `C:\Users\korn3\.cargo\registry\src\`.
- **Sizes at 96 DPI** (spec §3.1), scaled with `panel::scale`:
  - the title band is `side_panel::HEADER_HEIGHT_96` (38);
  - section header rows, Open Editors rows and tree rows are 26;
  - at most 9 Open Editors rows are visible;
  - root row buttons are 22 square;
  - the close box is 24 wide at the row's right.
- **Wording, verbatim from spec §4.4 and §4.6:**
  - `<name> already exists in <folder>. Replace it?` For the root, `<folder>` is `library_host::notebook_name(root)`.
  - `<name> was copied but isn't shown: the notebook lists text notes only.` With several: `<n> files were copied but aren't shown: <a>, <b>, …` (at most 3 names, then `…`).
  - `Copied the saved version of <name>. Your unsaved changes are still in its tab.`
  - `<name> could not be copied: <reason>.` For a folder: `<name> could not be copied: <reason>. <n> files were copied before the failure.`
  - `<name> was not copied: it is already there.` and `<name> was not copied: a folder can't be copied into itself.`
  - A new refusal, which the spec implies (§4.2 and Review Focus 2): `<name> was not copied: it would replace the folder it is in.`
  - `<name> was not copied: it could not be moved to the Recycle Bin.` (spec §4.4, "the notice says so").
- **Accessible names** (spec §7):
  - Open Editors rows are `<name>, open editor`, plus `, modified` when dirty.
  - The Open Editors header is `Open editors, <n>`, and the root row is the notebook's name. Both are outline items with an expanded or collapsed state.
- **Settings:** `open_editors_expanded=true|false`, default `true`. The per-notebook local file gets the line `root=collapsed`, written only while the root is collapsed.
- **New message:** `WM_FASTPAD_COPY_DONE = WM_APP + 18` and `WM_FASTPAD_PANEL_DROPPED = WM_APP + 19`. Both carry a `Box` the receiver frees, and a failed post is freed on the sender.

## Review Focus

1. **A copy onto itself.** A tab whose file is `notes\work\a.md` dragged onto `work` (or onto a note in `work`) must be refused before any prompt. Otherwise "Replace?" → OK recycles the source and the copy then fails, which loses the file. Task 7's `a_copy_onto_itself_is_refused_before_any_prompt` and Task 9's `open_editors_drag_onto_its_own_folder_copies_nothing_and_asks_nothing` pin this.
2. **A destination that holds the source.** Dropping `notes\work\work` (a folder named like its parent) onto the root plans `notes\work`, which exists. That folder holds the source, so replacing it would recycle the source. It must be refused with "it would replace the folder it is in." Task 7's `a_destination_that_holds_the_source_is_refused` pins this.
3. **A Recycle Bin step that fails** (a network share, the bin disabled, the shell warning answered No) must copy nothing for that item and say so. The rest of the drop carries on. Task 8's `copy_host_a_failed_recycle_skips_that_item_and_says_so` pins this.
4. **An Explorer drop must not block Explorer.** `Drop` returns before any prompt or copy: it only posts. Task 10's `panel_drop_returns_before_asking_and_the_posted_drop_asks` pins this.
5. **A tab switch or a close while an Open Editors row is being dragged.** The source tab can go away mid-drag. The drop must copy the file it pressed (`path` captured at arm time), and a file gone from disk gives the failure notice. Nothing may panic on a stale `DocumentId`. Task 9's `open_editors_drag_survives_its_tab_closing_mid_drag` pins this.

---

## File structure

- **Create `src/window/notebook_layout.rs`:** `PanelLayout`, `panel_layout`, `RootParts`, `root_parts`, `section_chevron` and their unit tests (Task 2).
- **Create `src/window/open_editors.rs`:** `EditorRow`, `editor_rows`, `unsaved_label` (moved from `notebook_view`), `accessible_name`, `tooltip`, `close_rect`, `OpenEditors`, `draw_editor_row`, `snapshot` and their tests (Task 3).
- **Create `src/window/panel_cursor.rs`:** `Cursor`, `Shape`, `step` and their tests (Task 6).
- **Create `src/window/tree_copy.rs`:** `Refusal`, `Planned`, `plan`, `any_accepted` and the notice functions, with tests (Task 7).
- **Create `src/window/copy_host.rs`:**
  - the entry points `copy_into`, `copy_tab_into` and `panel_dropped`;
  - the prompts and the Recycle Bin step, `CopyWorker`, `CopyJob`, `CopyDone` and `copy_done` (Task 8).
- **Create `src/window/panel_drop.rs`:** the panel's `IDropTarget`: `register`, `revoke`, and the test driver (Task 10).
- **Create `src/platform/ole_drop.rs`:** the COM vtables and `CF_HDROP` helpers, moved from `editor::file_drop` (Task 10).
- **Modify `src/platform/files.rs`:** `copy_file_no_replace` and `copy_tree` (Task 7).
- **Modify `src/window/mod.rs`:** register the new modules and re-export the two messages. **Modify `src/window/messages.rs`:** the two message constants (Tasks 8 and 10).
- **Modify `src/config/persisted.rs` and `src/config/defaults.rs`:** `open_editors_expanded` (Task 1).
- **Modify `src/library/local.rs` and `src/library/mod.rs`:** `root_collapsed` (Task 1).
- **Modify `src/library/tree.rs`:** remove `Unsaved` and `UnsavedEntry` (Task 4).
- **Modify `src/window/library_host.rs`:**
  - `root_expanded` and `set_root_expanded` (Task 1);
  - `refresh_label` (Tasks 4 and 5);
  - `confirmed` becomes `pub(crate)`, and `LibraryHost.copy_worker` is added (Task 8);
  - `accept_editor_file_drops` also registers the panel target (Task 10).
- **Modify `src/window/notebook_view.rs`:** the bulk of Tasks 4, 5, 6, 9 and 10.
- **Modify `src/window/tree_drag.rs`:** `DragSource` (Task 9).
- **Modify `src/window/side_panel.rs`:**
  - the title band as caption, and middle-button routing (Task 5);
  - revoking the drop target in `destroy_windows`, and registering it in `notes_mode_changed` (Task 10).
- **Modify `src/window/sidebar_accessibility.rs`:** `section_item`, and the `, unsaved` suffix goes (Tasks 4 and 6).
- **Modify `src/window/inline_name.rs`:** `insert_draft` at the root (Task 4).
- **Modify `src/window/main_window.rs`:**
  - `set_open_editors_expanded` and `close_document_tab` (Tasks 1 and 5);
  - `editors_changed` calls (Task 5);
  - the new messages (Tasks 8 and 10);
  - the window tests.
- **Modify `src/editor/file_drop.rs`:** it uses `platform::ole_drop`, and its `test_support` gains `drag_and_drop_at` (Task 10).
- **Modify the docs:** the `README.md` Notebook section, §9 of the tree drag spec, and this spec's §10 decisions (Task 11).

---

### Task 1: Settings and the per-notebook root state

**Files:**
- Modify: `src/config/persisted.rs` (`Settings`, `SettingsDelta`, `apply_delta`, `apply_line`, the `parse` doc comment, tests)
- Modify: `src/config/defaults.rs` (the default)
- Modify: `src/library/local.rs` (`LocalState`, `Conveniences`, `encode`, `parse`, tests)
- Modify: `src/library/mod.rs:424-426` (`merge_rescan`)
- Modify: `src/window/library_host.rs` (next to `set_expanded`, ~line 555)
- Modify: `src/window/main_window.rs` (next to `set_file_icons`, ~line 2741; and `change_setting`'s `sidebar_only`)

**Interfaces:**
- Produces:
  - `Settings::open_editors_expanded: bool`
  - `main_window::open_editors_expanded(hwnd: HWND) -> bool`
  - `main_window::set_open_editors_expanded(hwnd: HWND, expanded: bool)`, which saves the setting and invalidates the panel
  - `LocalState::root_collapsed: bool`
  - `library_host::root_expanded(hwnd: HWND) -> bool`, which is `true` without a loaded notebook
  - `library_host::set_root_expanded(hwnd: HWND, expanded: bool)`, which bumps `expansion_revision` and writes the local file when the value changed

- [ ] **Step 1: Write the failing tests.**

In `src/config/persisted.rs` `mod tests`:

```rust
    #[test]
    fn open_editors_expanded_parses_as_a_bool_and_defaults_to_true() {
        // Break caught: the collapsed Open Editors section forgotten on restart, or a typo
        // collapsing it silently (open editors spec §3.3).
        assert_eq!(parse("open_editors_expanded=false").open_editors_expanded, Some(false));
        assert_eq!(parse("open_editors_expanded=On").open_editors_expanded, Some(true));
        let delta = parse("open_editors_expanded=maybe");
        assert_eq!(delta.open_editors_expanded, None);
        assert_eq!(delta.warnings.len(), 1);
        let mut settings = crate::config::defaults::default_settings();
        assert!(settings.open_editors_expanded);
        settings.apply_delta(&parse("open_editors_expanded=false"));
        assert!(!settings.open_editors_expanded);
    }
```

(If `defaults.rs` names its constructor differently, use that name: it is the function that builds the `Settings` literal at `defaults.rs:39`.)

In `src/library/local.rs` `mod tests`:

```rust
    #[test]
    fn a_collapsed_root_round_trips_and_an_expanded_one_writes_nothing() {
        // Break caught: the notebook's root row reopening expanded after a restart, or every
        // local file gaining a root= line (open editors spec §3.3).
        let folder = PathBuf::from(r"C:\notes");
        let mut state = LocalState::new(folder.clone());
        assert!(!state.encode().contains("root="));
        state.root_collapsed = true;
        let encoded = state.encode();
        assert!(encoded.contains("root=collapsed\r\n"));
        assert!(LocalState::parse(&encoded, &folder).unwrap().root_collapsed);
        let before = state.conveniences();
        state.root_collapsed = false;
        assert_ne!(state.conveniences(), before, "a change is a change to write");
    }
```

In `src/library/mod.rs` `mod tests`, after `a_rescan_keeps_changes_made_while_it_ran` (`:1015`), which builds its states the same way:

```rust
    #[test]
    fn a_rescan_keeps_the_root_collapsed_state_the_ui_set_while_it_ran() {
        // Break caught: a rescan reopening the root row the user just collapsed.
        let scratch = Scratch::new("rescan-root");
        std::fs::write(scratch.folder().join("a.md"), "a").unwrap();
        let mut previous = load(&scratch.folder(), &scratch.local(), 100).unwrap();
        let fresh = load(&scratch.folder(), &scratch.local(), 101).unwrap();
        previous.local.root_collapsed = true;
        let merged = merge_rescan(previous, fresh);
        assert!(merged.local.root_collapsed);
    }
```

- [ ] **Step 2: Run them and see them fail to compile.**

Run `cargo clippy --all-targets -- -D warnings`. The expected errors are "no field `open_editors_expanded`" and "no field `root_collapsed`".

- [ ] **Step 3: Implement.**

1. `persisted.rs`:
   - Add `/// Whether the Notebook view's Open Editors section is expanded.` and `pub open_editors_expanded: bool,` to `Settings`, after `file_icons`.
   - Add `pub open_editors_expanded: Option<bool>,` to `SettingsDelta`, before `warnings`.
   - In `apply_delta`, add `if let Some(expanded) = delta.open_editors_expanded { self.open_editors_expanded = expanded; }`.
   - In `apply_line`, before the `_ =>` arm, add:

     ```rust
             "open_editors_expanded" => match parse_bool(value) {
                 Some(expanded) => delta.open_editors_expanded = Some(expanded),
                 None => warn(delta, line_number, key, value),
             },
     ```

   - Add `open_editors_expanded` to the list of recognized keys in `parse`'s doc comment.
2. `defaults.rs`:
   - Add `pub const DEFAULT_OPEN_EDITORS_EXPANDED: bool = true;`.
   - Add `open_editors_expanded: DEFAULT_OPEN_EDITORS_EXPANDED,` to the `Settings` literal.
3. `local.rs`:
   - Add `/// The notebook's root row is collapsed in the Notebook view (open editors spec §3.3).` and `pub root_collapsed: bool,` to both `LocalState` and `Conveniences`.
   - Copy it in `conveniences()`, and set it to `false` in `new`.
   - In `encode`, after the `expanded` lines, add `if self.root_collapsed { output.push_str("root=collapsed\r\n"); }`.
   - In `parse`, add `"root" => state.root_collapsed = value == "collapsed",`.
   - Add `root_collapsed: false` to any other `LocalState { .. }` or `Conveniences { .. }` literal the compiler finds.
4. `library/mod.rs` `merge_rescan`: after `fresh.local.expanded = previous_local.expanded;`, add `fresh.local.root_collapsed = previous_local.root_collapsed;`.
5. `library_host.rs`, after `expansion_revision`:

```rust
/// Whether the open notebook's root row is expanded (open editors spec §3.3). True while no
/// notebook state is loaded, so the loading and failed states show under it.
pub(crate) fn root_expanded(hwnd: HWND) -> bool {
    with_state(hwnd, |state| !state.local.root_collapsed).unwrap_or(true)
}

/// Expands or collapses the notebook's root row and remembers it in the per-PC file, the way
/// `set_expanded` does for a folder.
pub(crate) fn set_root_expanded(hwnd: HWND, expanded: bool) {
    let changed = with_state(hwnd, |state| {
        let changed = state.local.root_collapsed == expanded;
        state.local.root_collapsed = !expanded;
        changed
    })
    .unwrap_or(false);
    if changed {
        host(hwnd, |host| {
            host.expansion_revision = host.expansion_revision.wrapping_add(1);
        });
        save_local_soon(hwnd);
    }
}
```

6. `main_window.rs`, after `set_file_icons`:

```rust
/// Whether the Notebook view's Open Editors section is expanded (open editors spec §3.3).
pub(crate) fn open_editors_expanded(hwnd: HWND) -> bool {
    unsafe { app_ptr(hwnd) }.is_none_or(|app| unsafe { app.as_ref() }.settings.open_editors_expanded)
}

/// Collapses or expands the Open Editors section and saves it. Only the panel repaints.
pub(crate) fn set_open_editors_expanded(hwnd: HWND, expanded: bool) {
    change_setting(hwnd, |settings| {
        (settings.open_editors_expanded != expanded).then(|| {
            settings.open_editors_expanded = expanded;
            ("open_editors_expanded", expanded.to_string())
        })
    });
    if let Some((_, panel)) = crate::window::side_panel::windows(hwnd) {
        unsafe { InvalidateRect(panel, std::ptr::null(), 0) };
    }
}
```

   In `change_setting`, add `"open_editors_expanded"` to the `sidebar_only` `matches!`.

- [ ] **Step 4: Run the three tests.**

Run `cargo test open_editors_expanded_parses a_collapsed_root_round_trips a_rescan_keeps_the_root_collapsed`. Expected: PASS.

- [ ] **Step 5: Commit.**

```bash
cargo fmt
git add src/config src/library src/window/library_host.rs src/window/main_window.rs
git commit -m "feat(notebook): remember the Open Editors section and the notebook root row collapsed"
```

---

### Task 2: The panel layout (pure)

**Files:**
- Create: `src/window/notebook_layout.rs`
- Modify: `src/window/mod.rs` (add `pub(crate) mod notebook_layout;` in alphabetical order among the `pub(crate) mod` lines)

**Interfaces:**
- Consumes: `crate::window::panel::scale`, `crate::window::side_panel::HEADER_HEIGHT_96`, `crate::window::notebook_view::HeaderButton` (already `pub(crate)`).
- Produces:
  - `pub(crate) const ROW_HEIGHT: i32 = 26;` and `pub(crate) const MAX_EDITOR_ROWS: usize = 9;`
  - `pub(crate) struct PanelLayout { pub title: RECT, pub editors_header: RECT, pub editors_list: RECT, pub root: RECT, pub body: RECT }`, which is `Clone + Copy`. Every rectangle has the panel's full width. `editors_list` is empty (`bottom == top`) when the section is collapsed or has no tabs.
  - `pub(crate) fn panel_layout(client: RECT, dpi: u32, editors: usize, editors_expanded: bool) -> PanelLayout`
  - `pub(crate) fn section_chevron(row: RECT, dpi: u32) -> RECT`: a section header's chevron box.
  - `pub(crate) struct RootParts { pub chevron: RECT, pub name: RECT, pub buttons: [(HeaderButton, RECT); 4] }`
  - `pub(crate) fn root_parts(row: RECT, dpi: u32) -> RootParts`: the buttons run left to right as Favorite, NewNote, NewFolder, More, against the right edge. The name stops before the star.

- [ ] **Step 1: Write the module with its tests.** The tests go in `mod tests`, and the bodies start as `todo!()`.

```rust
//! Where the Notebook view's parts go (open editors spec §3.1): the title band in the window's
//! title strip, the Open Editors header and its rows, the notebook's root row with its buttons,
//! and the body below, which holds the tree or the view's state. Pure: rectangles only.

use crate::window::notebook_view::HeaderButton;
use crate::window::panel::scale;
use crate::window::side_panel::HEADER_HEIGHT_96;
use windows_sys::Win32::Foundation::RECT;

/// Every row's height at 96 DPI: section headers, Open Editors rows and tree rows alike.
pub(crate) const ROW_HEIGHT: i32 = 26;
/// Open Editors rows visible before the section scrolls on its own (spec §3.1).
pub(crate) const MAX_EDITOR_ROWS: usize = 9;
const ROOT_BUTTON: i32 = 22;
const CHEVRON_LEFT: i32 = 4;
const CHEVRON: i32 = 16;
const RIGHT_PAD: i32 = 6;

#[derive(Clone, Copy)]
pub(crate) struct PanelLayout {
    /// The view's name, in the window's title strip: all caption.
    pub title: RECT,
    pub editors_header: RECT,
    /// The Open Editors rows; empty when collapsed or with no tabs.
    pub editors_list: RECT,
    /// The notebook's root row.
    pub root: RECT,
    /// Everything under the root row: the tree, or the view's state.
    pub body: RECT,
}

/// The panel's bands for `client` at `dpi`, with `editors` tabs, stacked from the top and cut at
/// the panel's bottom edge, so a short panel never turns a band inside out.
pub(crate) fn panel_layout(
    client: RECT,
    dpi: u32,
    editors: usize,
    editors_expanded: bool,
) -> PanelLayout {
    let row = scale(ROW_HEIGHT, dpi);
    let cut = |y: i32| y.min(client.bottom);
    let title_bottom = cut(client.top + scale(HEADER_HEIGHT_96, dpi));
    let header_bottom = cut(title_bottom + row);
    let shown = if editors_expanded {
        editors.min(MAX_EDITOR_ROWS)
    } else {
        0
    };
    let list_bottom = cut(header_bottom + row * shown as i32);
    let root_bottom = cut(list_bottom + row);
    let band = |top: i32, bottom: i32| RECT {
        left: client.left,
        top,
        right: client.right,
        bottom,
    };
    PanelLayout {
        title: band(client.top, title_bottom),
        editors_header: band(title_bottom, header_bottom),
        editors_list: band(header_bottom, list_bottom),
        root: band(list_bottom, root_bottom),
        body: band(root_bottom, client.bottom),
    }
}

/// A section header row's chevron.
pub(crate) fn section_chevron(row: RECT, dpi: u32) -> RECT {
    let left = (row.left + scale(CHEVRON_LEFT, dpi)).min(row.right);
    RECT {
        left,
        top: row.top,
        right: (left + scale(CHEVRON, dpi)).min(row.right),
        bottom: row.bottom,
    }
}

#[derive(Clone, Copy)]
pub(crate) struct RootParts {
    pub chevron: RECT,
    pub name: RECT,
    /// Left to right: star, New note, New folder, "…".
    pub buttons: [(HeaderButton, RECT); 4],
}

/// The root row's chevron, the notebook's name after it, and its four buttons at the right.
pub(crate) fn root_parts(row: RECT, dpi: u32) -> RootParts {
    let chevron = section_chevron(row, dpi);
    let size = scale(ROOT_BUTTON, dpi);
    let top = row.top + (row.bottom - row.top - size) / 2;
    let right = row.right - scale(RIGHT_PAD, dpi);
    let slot = |from_right: i32| RECT {
        left: (right - (from_right + 1) * size).max(chevron.right),
        top,
        right: (right - from_right * size).max(chevron.right),
        bottom: top + size,
    };
    let buttons = [
        (HeaderButton::Favorite, slot(3)),
        (HeaderButton::NewNote, slot(2)),
        (HeaderButton::NewFolder, slot(1)),
        (HeaderButton::More, slot(0)),
    ];
    let name = RECT {
        left: chevron.right,
        top: row.top,
        right: buttons[0].1.left.max(chevron.right),
        bottom: row.bottom,
    };
    RootParts {
        chevron,
        name,
        buttons,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edges(rect: RECT) -> (i32, i32, i32, i32) {
        (rect.left, rect.top, rect.right, rect.bottom)
    }

    const CLIENT: RECT = RECT {
        left: 0,
        top: 0,
        right: 260,
        bottom: 600,
    };

    #[test]
    fn the_bands_stack_title_editors_root_then_body() {
        // Break caught: the Open Editors rows painted over the root row, the title band not the
        // strip's 38 px (so the window can't be dragged from it), or a body that starts above
        // the root row.
        let layout = panel_layout(CLIENT, 96, 3, true);
        assert_eq!(edges(layout.title), (0, 0, 260, 38));
        assert_eq!(edges(layout.editors_header), (0, 38, 260, 64));
        assert_eq!(edges(layout.editors_list), (0, 64, 260, 64 + 3 * 26));
        assert_eq!(edges(layout.root), (0, 142, 260, 168));
        assert_eq!(edges(layout.body), (0, 168, 260, 600));
    }

    #[test]
    fn a_collapsed_or_empty_section_has_no_rows_and_twenty_tabs_show_nine() {
        // Break caught: a collapsed section still taking room, or a long tab list pushing the
        // notebook off the panel (spec §3.1).
        let collapsed = panel_layout(CLIENT, 96, 5, false);
        assert_eq!(collapsed.editors_list.top, collapsed.editors_list.bottom);
        assert_eq!(collapsed.root.top, 64);
        let empty = panel_layout(CLIENT, 96, 0, true);
        assert_eq!(empty.root.top, 64);
        let many = panel_layout(CLIENT, 96, 20, true);
        assert_eq!(many.editors_list.bottom - many.editors_list.top, 9 * 26);
        assert_eq!(panel_layout(CLIENT, 192, 1, true).title.bottom, 76);
    }

    #[test]
    fn a_short_panel_cuts_every_band_at_its_bottom() {
        let short = RECT {
            bottom: 80,
            ..CLIENT
        };
        let layout = panel_layout(short, 96, 9, true);
        for band in [
            layout.title,
            layout.editors_header,
            layout.editors_list,
            layout.root,
            layout.body,
        ] {
            assert!(band.top <= band.bottom && band.bottom <= 80);
        }
    }

    #[test]
    fn the_root_buttons_sit_right_to_left_and_the_name_stops_before_them() {
        // Break caught: the notebook name drawn under the star, or buttons that don't follow the
        // panel's right edge.
        let row = RECT {
            left: 0,
            top: 142,
            right: 260,
            bottom: 168,
        };
        let parts = root_parts(row, 96);
        let [(a, star), (b, new), (c, folder), (d, more)] = parts.buttons;
        assert_eq!(
            (a, b, c, d),
            (
                HeaderButton::Favorite,
                HeaderButton::NewNote,
                HeaderButton::NewFolder,
                HeaderButton::More
            )
        );
        assert_eq!(edges(more), (232, 144, 254, 166));
        assert_eq!((star.right, new.right, folder.right), (new.left, folder.left, more.left));
        assert_eq!(parts.name.right, star.left);
        assert_eq!(parts.name.left, parts.chevron.right);
        assert_eq!(edges(parts.chevron), (4, 142, 20, 168));
        let narrow = root_parts(RECT { right: 40, ..row }, 96);
        assert!(narrow.name.left <= narrow.name.right);
    }
}
```

- [ ] **Step 2: Run the tests with the `todo!()` bodies and check they fail.** Run `cargo test --lib notebook_layout`. Expected: FAIL (panics at `todo!`).
- [ ] **Step 3: Put the implementations above in place of the `todo!()`s.**
- [ ] **Step 4: Run the tests and check they pass.** Run `cargo test --lib notebook_layout`. Expected: 4 passed.
- [ ] **Step 5: Commit.** Run `cargo fmt`, then `git add src/window/notebook_layout.rs src/window/mod.rs`, then `git commit -m "feat(notebook): lay out the title band, Open Editors, the root row and the body"`.

---

### Task 3: Open Editors rows (model and painting)

**Files:**
- Create: `src/window/open_editors.rs`
- Modify: `src/window/mod.rs` (`pub(crate) mod open_editors;`)
- Modify: `src/window/notebook_view.rs`. `draw_item_icon` becomes `pub(crate)`. Leave `unsaved_label` in place for now: Task 4 removes it.

**Interfaces:**
- Consumes:
  - `crate::document::{Document, DocumentId}`
  - `notebook_view::draw_item_icon(dc, item, rect, px, muted, palette, icons, images, set, light_theme)`
  - `crate::window::file_icons::note_kind`, `crate::window::icon_sets::TreeItem`
  - `side_panel::{UiFonts, draw_text}`, `row_list::{RowLook, RowListState, row_foreground}`
  - `notebook_layout::ROW_HEIGHT`
- Produces:
  - `pub(crate) struct EditorRow { pub id: DocumentId, pub name: String, pub path: Option<PathBuf>, pub dirty: bool, pub active: bool }`, which is `Clone, Debug, Eq, PartialEq`.
  - `pub(crate) fn unsaved_label(document: &Document) -> String`, with the same body as `notebook_view::unsaved_label`.
  - `pub(crate) fn editor_rows<'a>(documents: impl Iterator<Item = &'a Document>, active: Option<DocumentId>) -> Vec<EditorRow>`
  - `pub(crate) fn snapshot(hwnd: HWND) -> Vec<EditorRow>`, which borrows the App on its own.
  - `pub(crate) fn accessible_name(row: &EditorRow) -> String`
  - `pub(crate) fn tooltip(row: &EditorRow) -> String`
  - `pub(crate) fn close_rect(row: RECT, dpi: u32) -> RECT`
  - `pub(crate) fn tree_item(row: &EditorRow) -> TreeItem`: the note type from the extension, or `note_kind(None)` for an untitled tab.
  - `pub(crate) struct OpenEditors { pub rows: Vec<EditorRow>, pub list: RowListState, pub hover_close: bool }`, with:
    - `OpenEditors::new(row_height: i32) -> Self`;
    - `OpenEditors::set_rows(&mut self, rows: Vec<EditorRow>) -> bool`, which is true when anything changed. It sets `list.count`, and `list.selected` becomes the active row's index;
    - `OpenEditors::active_index(&self) -> Option<usize>`.
  - `pub(crate) fn draw_editor_row(dc: HDC, row: &EditorRow, rect: RECT, look: RowLook, paint: &ViewPaint, images: &mut IconImages, close_hot: bool)`

- [ ] **Step 1: Write the module with its tests** (the bodies start as `todo!()`).

```rust
//! The Notebook view's Open Editors section (open editors spec §3.2): one row per tab in
//! tab-strip order, with its type icon and name, a dot while it has unsaved changes and a close
//! box on hover. Built from the tab list in memory: no disk.

use crate::document::{Document, DocumentId};
use crate::window::file_icons::note_kind;
use crate::window::icon_sets::TreeItem;
use crate::window::icon_sets::images::IconImages;
use crate::window::main_window::app_ptr;
use crate::window::notebook_view::draw_item_icon;
use crate::window::panel::scale;
use crate::window::row_list::{RowListState, RowLook, row_foreground};
use crate::window::side_panel::{ViewPaint, draw_text};
use std::path::PathBuf;
use windows_sys::Win32::Foundation::{HWND, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    DT_CENTER, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, HDC,
};

// Sizes at 96 DPI.
const LEFT_PAD: i32 = 24;
const GLYPH_BOX: i32 = 16;
const GAP: i32 = 6;
const CLOSE_BOX: i32 = 24;
/// Segoe MDL2 Assets' Cancel, the tab strip's close glyph.
const GLYPH_CLOSE: &str = "\u{E711}";
const DIRTY_DOT: &str = "\u{25CF}";
const LINE: u32 = DT_SINGLELINE | DT_VCENTER | DT_LEFT | DT_END_ELLIPSIS | DT_NOPREFIX;
const CENTERED: u32 = DT_SINGLELINE | DT_VCENTER | DT_CENTER | DT_NOPREFIX;

/// One tab as the section shows it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct EditorRow {
    pub id: DocumentId,
    /// The file name with its extension, or an untitled tab's label.
    pub name: String,
    pub path: Option<PathBuf>,
    pub dirty: bool,
    pub active: bool,
}

/// An untitled tab's name: its tab label, else its first line, else "Untitled".
pub(crate) fn unsaved_label(document: &Document) -> String {
    document
        .untitled_label
        .clone()
        .or_else(|| document.first_line_label.clone())
        .unwrap_or_else(|| "Untitled".to_owned())
}

/// One row per document, in the order given (the strip's).
pub(crate) fn editor_rows<'a>(
    documents: impl Iterator<Item = &'a Document>,
    active: Option<DocumentId>,
) -> Vec<EditorRow> {
    documents
        .map(|document| EditorRow {
            id: document.id,
            name: match &document.path {
                Some(path) => path.file_name().map_or_else(
                    || path.display().to_string(),
                    |name| name.to_string_lossy().into_owned(),
                ),
                None => unsaved_label(document),
            },
            path: document.path.clone(),
            dirty: document.dirty,
            active: Some(document.id) == active,
        })
        .collect()
}

/// The rows for the window's tabs now. Borrows the App on its own, never nested.
pub(crate) fn snapshot(hwnd: HWND) -> Vec<EditorRow> {
    unsafe { app_ptr(hwnd) }
        .map(|app| {
            let tabs = &unsafe { app.as_ref() }.tabs;
            editor_rows(tabs.documents(), tabs.active().map(|active| active.id))
        })
        .unwrap_or_default()
}

/// What screen readers hear for a row (spec §7).
pub(crate) fn accessible_name(row: &EditorRow) -> String {
    let mut name = format!("{}, open editor", row.name);
    if row.dirty {
        name.push_str(", modified");
    }
    name
}

/// A row's tooltip: the file's full path, or an untitled tab's label.
pub(crate) fn tooltip(row: &EditorRow) -> String {
    row.path
        .as_ref()
        .map_or_else(|| row.name.clone(), |path| path.display().to_string())
}

/// The close box at a row's right edge.
pub(crate) fn close_rect(row: RECT, dpi: u32) -> RECT {
    RECT {
        left: (row.right - scale(CLOSE_BOX, dpi)).max(row.left),
        ..row
    }
}

/// The icon a row (and its drag label) shows.
pub(crate) fn tree_item(row: &EditorRow) -> TreeItem {
    let extension = row
        .path
        .as_ref()
        .and_then(|path| path.extension())
        .map(|extension| extension.to_string_lossy());
    TreeItem::Note(note_kind(extension.as_deref()))
}

/// The section's rows and its own list state (scroll, hover, the active row as selected).
#[derive(Debug)]
pub(crate) struct OpenEditors {
    pub rows: Vec<EditorRow>,
    pub list: RowListState,
    /// The pointer is over the hovered row's close box.
    pub hover_close: bool,
}

impl OpenEditors {
    pub(crate) fn new(row_height: i32) -> Self {
        Self {
            rows: Vec::new(),
            list: RowListState::new(row_height),
            hover_close: false,
        }
    }

    /// Takes `rows`; true when anything shown changed. The list's selection is the active row.
    pub(crate) fn set_rows(&mut self, rows: Vec<EditorRow>) -> bool {
        if rows == self.rows {
            return false;
        }
        self.rows = rows;
        self.list.set_count(self.rows.len());
        self.list.selected = self.active_index();
        true
    }

    pub(crate) fn active_index(&self) -> Option<usize> {
        self.rows.iter().position(|row| row.active)
    }
}

/// Paints one row: the icon, the name, and at the right the dot of a dirty tab or, on hover or
/// selection, a clean tab's close box.
pub(crate) fn draw_editor_row(
    dc: HDC,
    row: &EditorRow,
    rect: RECT,
    look: RowLook,
    paint: &ViewPaint,
    images: &mut IconImages,
    close_hot: bool,
) {
    let (palette, dpi) = (&paint.palette, paint.dpi);
    let foreground = row_foreground(look, palette);
    let muted = if look.selected && look.focused {
        foreground
    } else {
        palette.muted_foreground
    };
    let px = scale(GLYPH_BOX, dpi);
    let icon = RECT {
        left: (rect.left + scale(LEFT_PAD, dpi)).min(rect.right),
        top: rect.top,
        right: (rect.left + scale(LEFT_PAD, dpi) + px).min(rect.right),
        bottom: rect.bottom,
    };
    draw_item_icon(
        dc,
        tree_item(row),
        icon,
        px,
        muted,
        palette,
        &paint.icons,
        images,
        paint.icon_set,
        paint.light_theme,
    );
    let close = close_rect(rect, dpi);
    let name = RECT {
        left: (icon.right + scale(GAP, dpi)).min(close.left),
        right: close.left,
        ..rect
    };
    unsafe { draw_text(dc, &row.name, name, paint.fonts.text, foreground, LINE) };
    if row.dirty {
        unsafe { draw_text(dc, DIRTY_DOT, close, paint.fonts.text, muted, CENTERED) };
    } else if look.hover || look.selected {
        let color = if close_hot { foreground } else { muted };
        unsafe { draw_text(dc, GLYPH_CLOSE, close, paint.fonts.glyph, color, CENTERED) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn document(id: u64, path: Option<&str>, dirty: bool) -> Document {
        let mut document = Document::test_fixture(DocumentId(id), dirty);
        document.path = path.map(PathBuf::from);
        document
    }

    #[test]
    fn a_row_per_tab_in_order_named_by_file_or_label() {
        // Break caught: rows out of strip order, a full path as the name, a blank name for an
        // untitled tab, or the wrong row marked active.
        let mut untitled = document(3, None, false);
        untitled.first_line_label = Some("Groceries".to_owned());
        let docs = [
            document(1, Some(r"C:\n\todo.md"), false),
            document(2, Some(r"D:\x\draft.txt"), true),
            untitled,
            document(4, None, false),
        ];
        let rows = editor_rows(docs.iter(), Some(DocumentId(2)));
        let names: Vec<_> = rows.iter().map(|row| row.name.as_str()).collect();
        assert_eq!(names, ["todo.md", "draft.txt", "Groceries", "Untitled"]);
        assert_eq!(
            rows.iter().map(|row| (row.dirty, row.active)).collect::<Vec<_>>(),
            [(false, false), (true, true), (false, false), (false, false)]
        );
        assert_eq!(rows[1].path.as_deref(), Some(std::path::Path::new(r"D:\x\draft.txt")));
    }

    #[test]
    fn names_tips_and_the_close_box() {
        let row = EditorRow {
            id: DocumentId(1),
            name: "draft.txt".into(),
            path: Some(PathBuf::from(r"D:\x\draft.txt")),
            dirty: true,
            active: false,
        };
        assert_eq!(accessible_name(&row), "draft.txt, open editor, modified");
        assert_eq!(tooltip(&row), r"D:\x\draft.txt");
        let clean = EditorRow { dirty: false, path: None, ..row };
        assert_eq!(accessible_name(&clean), "draft.txt, open editor");
        assert_eq!(tooltip(&clean), "draft.txt");
        let rect = RECT { left: 0, top: 10, right: 200, bottom: 36 };
        let close = close_rect(rect, 96);
        assert_eq!((close.left, close.right, close.top), (176, 200, 10));
    }

    #[test]
    fn set_rows_reports_changes_and_selects_the_active_row() {
        let docs = [document(1, Some(r"C:\a.md"), false), document(2, Some(r"C:\b.md"), false)];
        let mut editors = OpenEditors::new(26);
        assert!(editors.set_rows(editor_rows(docs.iter(), Some(DocumentId(2)))));
        assert_eq!((editors.list.count, editors.list.selected), (2, Some(1)));
        assert!(!editors.set_rows(editor_rows(docs.iter(), Some(DocumentId(2)))));
        assert!(editors.set_rows(editor_rows(docs.iter(), Some(DocumentId(1)))));
        assert_eq!(editors.list.selected, Some(0));
    }
}
```

`Document::test_fixture(id, dirty)` already exists (`notebook_view` tests use it). Its `path` is `None`.

- [ ] **Step 2: Run the tests and check they fail.** Run `cargo test --lib open_editors`. Expected: FAIL at `todo!`.
- [ ] **Step 3: Put the implementations in place.** Make `notebook_view::draw_item_icon` `pub(crate)`.
- [ ] **Step 4: Run the tests and check they pass.** Run `cargo test --lib open_editors`. Expected: 3 passed.
- [ ] **Step 5: Commit** with the message `"feat(notebook): Open Editors rows from the tab list"`.

---

### Task 4: Untitled tabs leave the tree

**Files:**
- Modify: `src/library/tree.rs`:
  - remove `UnsavedEntry`, `RowKind::Unsaved` and the `unsaved` parameter of `NoteTree::rows`;
  - update the `row_index` match (`:800`) and the debug marker (`:870`);
  - update the tests (`:983-997`, `:1296`, `:1327`).
- Modify: `src/window/notebook_view.rs`:
  - change `flatten`;
  - remove `unsaved` from `RebuildKey`, and update `rebuild_key` and `snapshot`;
  - remove `unsaved_entries`, `unsaved_label` and `unsaved_label_changed`;
  - remove the `RowKind::Unsaved` arms in `draw_tree_row`, `drag_item`, `row_tip`, `open_context_menu` and `activate`;
  - `active_target` returns `None` for an untitled tab;
  - `Mode::Empty`'s doc comment becomes "Loaded, with no notes.";
  - update the tests.
- Modify: `src/window/tree_drag.rs`: `source_path` and `drop_folder` lose their `Unsaved` arms, and the tests' `rows()` fixture drops its unsaved row.
- Modify: `src/window/inline_name.rs:222-229`: the root draft goes at index 0. Update the tests at `:1461-1491`.
- Modify: `src/window/sidebar_accessibility.rs:264-287`: drop `, unsaved`, and update the test at `:1201-1262`.
- Modify: `src/window/library_host.rs:1173-1178`: use `open_editors::unsaved_label`, and drop the `unsaved_label_changed` call. Task 5 adds `editors_changed` there.
- Modify: `src/window/side_panel.rs:68`: the comment "Unsaved rows and notices inside the list" becomes "Notices inside the list".
- Modify: `src/window/main_window.rs` tests: every test that relied on an unsaved tree row (see Step 3).

**Interfaces:**
- Produces:
  - `NoteTree::rows(&self, expanded: &dyn Fn(&Path) -> bool) -> Vec<TreeRow>`
  - `notebook_view::flatten(tree: &NoteTree, expanded: &[PathBuf]) -> Vec<TreeRow>`
  - `RowKind` is `{ Folder(PathBuf), Note(PathBuf), Draft }`.

- [ ] **Step 1: Write the failing window test.** Put it in `main_window.rs` tests, replacing `an_empty_notebook_says_so_until_an_untitled_tab_appears_as_an_unsaved_row` (`:12262`), which asserts the old behaviour:

```rust
    #[test]
    fn an_empty_notebook_stays_empty_with_untitled_tabs_open() {
        // Break caught: untitled tabs still listed in the tree as well as in Open Editors, or an
        // empty notebook's state hidden by them (open editors spec §3.4).
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("empty-untitled");
        let (window, _editor) = notebook_window(&scratch);
        execute_command(window.hwnd, CommandId::New);
        crate::window::notebook_view::rebuild(window.hwnd);
        let view = notebook_view(window.hwnd);
        assert_eq!(view.mode, crate::window::notebook_view::Mode::Empty);
        assert!(view.rows.is_empty());
    }
```

(If the new-tab command is named differently in `CommandId`, use the command Ctrl+N runs: `commands.rs` maps it.)

- [ ] **Step 2: Run it and check it fails.** Run `cargo test --lib an_empty_notebook_stays_empty_with_untitled_tabs_open -- --test-threads=1`. Expected: FAIL, because the mode is `Tree` with one unsaved row.

- [ ] **Step 3: Implement the removal.** The compiler lists every place; the rules are:

1. `tree.rs`:
   - Delete `UnsavedEntry` and `RowKind::Unsaved`.
   - `rows` takes only `expanded`. Drop the `rows.extend(unsaved…)` line and the `unsaved.len() +` in the capacity.
   - Remove the `(RowKind::Unsaved(a), RowKind::Unsaved(b)) => a == b` arm.
   - Remove the `?` marker arm and change the marker doc comment to "`/` a folder, `*` a pinned note".
   - Delete `unsaved_entries_come_first_at_the_root`.
   - In the `row_index` tests, remove the `Unsaved` asserts.
   - Every `tree.rows(&x, &[])` / `tree.rows(&x, &unsaved)` becomes `tree.rows(&x)`.
2. `notebook_view.rs`:
   - `flatten(tree, expanded)` calls `tree.rows(&|path| …)`.
   - `RebuildKey` drops `unsaved`. `rebuild_key` drops the tab read, and `snapshot` calls `flatten(&state.tree, &state.local.expanded)`.
   - Delete `unsaved_entries`, `unsaved_label` and `unsaved_label_changed`, along with the test `unsaved_rows_come_from_untitled_tabs_labelled_by_their_first_line`, which moves to `open_editors` in spirit: Task 3's first test.
   - In `draw_tree_row`, delete the `RowKind::Unsaved(_)` arm. The name font is always `fonts.text`.
   - In `drag_item`, the `None` arm is `RowKind::Draft => None`.
   - In `row_tip`, the font is always `fonts.text`.
   - In `open_context_menu` and `activate`, delete the `Unsaved` arms.
   - `active_target` becomes:

     ```rust
     fn active_target(hwnd: HWND) -> Option<RowKind> {
         let root = super::library_host::folder(hwnd)?;
         let path = unsafe { app_ptr(hwnd) }
             .and_then(|app| unsafe { app.as_ref() }.tabs.active()?.path.clone())?;
         crate::library::is_inside(&root, &path)
             .then(|| RowKind::Note(crate::library::record_path(&root, &path)))
     }
     ```

   - The tests' `flatten(&tree, &expanded, &[])` calls become `flatten(&tree, &expanded)`.
   - Remove now-unused imports (`UnsavedEntry`, `Document` if unused).
3. `tree_drag.rs`:
   - `source_path`'s `None` arm is `RowKind::Draft => None`.
   - `drop_folder`'s row arm becomes `Some(RowKind::Folder(path)) => Some(path.clone()), Some(RowKind::Note(path)) => Some(parent_of(path)), None => Some(PathBuf::new()), Some(RowKind::Draft) => None`.
   - The tests' `rows()` fixture drops `row(RowKind::Unsaved(7), 0, false)`, so every index in the tests goes down by one. Update each literal index and the doc comment `/// work (expanded) { inner (collapsed), b.md }, a.md`.
   - Remove the `Unsaved` asserts in `only_notes_and_folders_arm_a_drag`, in the drop-folder test ("an unsaved row: the root") and in `accepts`.
4. `inline_name.rs::insert_draft`: `None => (0, 0)`. Update the doc comment: "or first at the root". Update the tests at `:1461` and `:1487`: drop the `Unsaved` fixture rows and rename `the_draft_row_is_the_first_child_of_an_expanded_folder_or_below_the_unsaved_rows` to `…_or_first_at_the_root`. Remove `RowKind::Unsaved(_)` from the `:756` match.
5. `sidebar_accessibility.rs::tree_item`: delete the `, unsaved` block, and fix the doc comment to say "then \", pinned\"". In the test, delete the `unsaved` item and its asserts.
6. `library_host.rs:1173`: `Some((id, crate::window::open_editors::unsaved_label(document)))`. At `:1175-1179`, keep `refresh_tab_view` and delete the `unsaved_label_changed` call and its comment. The binding `(id, label)` becomes `(_, _)` until Task 5, or reduce `changed` to a `bool`: `.map(|_| ())`.
7. `main_window.rs` tests. Search `Unsaved` and `unsaved row`, and fix each:
   - `:12281`: covered by Step 1's replacement test.
   - `:12366` (a test that clicks an unsaved row): delete that part. Task 5's Open Editors click test covers switching to an untitled tab.
   - `:12609-12642` (context menu "Close tab" on an unsaved row): delete that menu case and its break-caught clause.
   - `:16373-16376`, `:17758`, `:17937`, `:17962`, `:17990`: comments and index offsets that counted the editor's own untitled tab as row 0. Drop the `+1` offset or the filter, and delete the comment.

- [ ] **Step 4: Run the changed tests.**

Run `cargo clippy --all-targets -- -D warnings`, then:

```bash
cargo test --lib tree:: tree_drag inline_name::tests sidebar_accessibility notebook_view::tests -- --test-threads=1
cargo test --lib an_empty_notebook_stays_empty_with_untitled_tabs_open -- --test-threads=1
```

Also run each `main_window` test named in Step 3.7. Expected: all pass.

- [ ] **Step 5: Commit** with the message `"refactor(notebook): untitled tabs leave the tree; Open Editors will list them"`.

---

### Task 5: The panel's sections: painting, hit test and mouse

**Files:**
- Modify: `src/window/notebook_view.rs` (most of the task)
- Modify: `src/window/side_panel.rs`:
  - in `header_is_caption`, the Notebook arm becomes `true`;
  - route `WM_MBUTTONDOWN` and `WM_MBUTTONUP` to the view;
  - `paint_view` syncs the editors before painting.
- Modify: `src/window/main_window.rs`:
  - `close_document_tab`;
  - `editors_changed` calls in `invalidate_title_strip` and in the savepoint handlers;
  - update the test at `:8558-8570`;
  - add the window tests.
- Modify: `src/window/library_host.rs:1175` (call `editors_changed`)

**Interfaces:**
- Consumes: Tasks 1–3 (`notebook_layout::*`, `open_editors::*`, `library_host::{root_expanded, set_root_expanded}`, `main_window::{open_editors_expanded, set_open_editors_expanded}`).
- Produces:
  - `NotebookView.editors: OpenEditors`;
  - `notebook_view::editors_changed(hwnd: HWND)`: syncs the rows from the tabs and repaints when they changed. It's cheap, and fine to call often;
  - `NotebookView::layout(&self, client: RECT, dpi: u32) -> PanelLayout`;
  - `NotebookView::tree_shown(&self) -> bool`, which is `mode == Tree && root_expanded`;
  - `main_window::close_document_tab(hwnd: HWND, id: DocumentId)`, which closes like a middle-click on that tab;
  - `#[cfg(test)] NotebookView::editor_rect_at(&self, index: usize) -> Option<RECT>`, `editors_header_rect()` and `root_rect()`, for the window tests.
- Removes: `header_layout`, `HeaderLayout`, `body_rect`, `HEADER_HEIGHT`, `HEADER_BUTTON`, `Hit::Title`, `TOOL_TITLE`'s old rectangle (it becomes the root name's), `paint_header` and `header_hit`.

- [ ] **Step 1: Write the failing window tests** (in `main_window.rs` tests, after the tree drag tests):

```rust
    /// The centre of `rect` as a panel `lParam`.
    fn centre(rect: RECT) -> super::LPARAM {
        client_lparam((rect.left + rect.right) / 2, (rect.top + rect.bottom) / 2)
    }

    #[test]
    fn open_editors_lists_the_tabs_and_follows_opening_closing_and_saving() {
        // Break caught: a tab opened or closed without its row following, a dirty tab without
        // its dot, or the active tab's row not the selected one (open editors spec §3.2).
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("editors-follow");
        let a = scratch.note("a.md", "a");
        let (window, editor) = notebook_window(&scratch);
        super::open_path(window.hwnd, &a).unwrap();
        let outside = scratch.root.join("outside.txt");
        std::fs::write(&outside, "x").unwrap();
        super::open_path(window.hwnd, &outside).unwrap();
        let names = |hwnd| {
            notebook_view(hwnd)
                .editors
                .rows
                .iter()
                .map(|row| (row.name.clone(), row.dirty, row.active))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            names(window.hwnd),
            [("a.md".into(), false, false), ("outside.txt".into(), false, true)]
        );
        editor.set_text("changed").unwrap();
        assert!(names(window.hwnd)[1].1, "the dirty dot follows the edit");
        execute_command(window.hwnd, CommandId::CloseTab);
        crate::window::modal::answer_next_confirm(false);
        assert_eq!(names(window.hwnd).len(), 1);
    }

    #[test]
    fn open_editors_click_switches_close_box_and_middle_click_close() {
        // Break caught: a click that opens nothing, the close box closing the wrong tab, or a
        // middle-click ignored in the panel (open editors spec §3.2).
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP,
        };
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("editors-click");
        let a = scratch.note("a.md", "a");
        let b = scratch.note("b.md", "b");
        let c = scratch.note("c.md", "c");
        let (window, _editor) = notebook_window(&scratch);
        for path in [&a, &b, &c] {
            super::open_path(window.hwnd, path).unwrap();
        }
        let panel = sidebar_windows(window.hwnd).1;
        let row0 = notebook_view(window.hwnd).editor_rect_at(0).unwrap();
        mouse(panel, WM_LBUTTONDOWN, 1, centre(row0));
        mouse(panel, WM_LBUTTONUP, 0, centre(row0));
        assert_eq!(app_mut(window.hwnd).tabs.active().unwrap().path.as_deref(), Some(a.as_path()));

        let row1 = notebook_view(window.hwnd).editor_rect_at(1).unwrap();
        let close = crate::window::open_editors::close_rect(row1, 96);
        mouse(panel, WM_LBUTTONDOWN, 1, centre(close));
        mouse(panel, WM_LBUTTONUP, 0, centre(close));
        assert_eq!(tab_paths(window.hwnd), [Some(a.clone()), Some(c.clone())]);

        let row1 = notebook_view(window.hwnd).editor_rect_at(1).unwrap();
        mouse(panel, WM_MBUTTONDOWN, 4, centre(row1));
        mouse(panel, WM_MBUTTONUP, 0, centre(row1));
        assert_eq!(tab_paths(window.hwnd), [Some(a)]);
    }

    #[test]
    fn open_editors_and_the_root_collapse_and_stay_so() {
        // Break caught: the chevrons doing nothing, the tree still hit-tested while the root is
        // collapsed, or either state lost (open editors spec §3.3).
        use windows_sys::Win32::UI::WindowsAndMessaging::{WM_LBUTTONDOWN, WM_LBUTTONUP};
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("editors-collapse");
        scratch.note("a.md", "a");
        let (window, _editor) = notebook_window(&scratch);
        let panel = sidebar_windows(window.hwnd).1;
        let header = notebook_view(window.hwnd).editors_header_rect();
        mouse(panel, WM_LBUTTONDOWN, 1, centre(header));
        mouse(panel, WM_LBUTTONUP, 0, centre(header));
        assert!(!super::open_editors_expanded(window.hwnd));
        let root = notebook_view(window.hwnd).root_rect();
        let chevron = crate::window::notebook_layout::root_parts(root, 96).chevron;
        mouse(panel, WM_LBUTTONDOWN, 1, centre(chevron));
        mouse(panel, WM_LBUTTONUP, 0, centre(chevron));
        assert!(!crate::window::library_host::root_expanded(window.hwnd));
        assert!(!notebook_view(window.hwnd).tree_shown());
        let local = crate::library::local::local_file(&scratch.data(), &scratch.folder());
        assert!(std::fs::read_to_string(local).unwrap().contains("root=collapsed"));
    }
```

Also fix the existing test at `main_window.rs:8558`. It asserted that the header's title was client area. It now asserts that the whole title band is caption (`HTTRANSPARENT` from the panel's `WM_NCHITTEST`) at the old title point. Keep its other asserts.

- [ ] **Step 2: Run the tests and check they fail.** Run `cargo test --lib open_editors_ -- --test-threads=1`. Expected: compile errors (`editors`, `editor_rect_at` …).

- [ ] **Step 3: Implement.**

1. **Fields and layout.**
   - Add these fields to `NotebookView`, and initialise them in `new`:
     - `pub(crate) editors: OpenEditors` (`OpenEditors::new(scale(ROW_HEIGHT, dpi))`)
     - `editors_expanded: bool` (`true`)
     - `root_expanded: bool` (`true`)
   - Replace `header_layout`, `body_rect` and their test with:

```rust
    /// The panel's bands for the view as it is (open editors spec §3.1).
    pub(crate) fn layout(&self, client: RECT, dpi: u32) -> PanelLayout {
        notebook_layout::panel_layout(client, dpi, self.editors.rows.len(), self.editors_expanded)
    }

    /// The tree is painted and hit: loaded rows under an expanded root.
    pub(crate) fn tree_shown(&self) -> bool {
        self.mode == Mode::Tree && self.root_expanded
    }
```

   - `list_area` becomes:

```rust
    pub(crate) fn list_area(&self, client: RECT, dpi: u32) -> RECT {
        let body = self.layout(client, dpi).body;
        let empty = RECT { bottom: body.top, ..body };
        match self.mode {
            Mode::Tree if self.root_expanded => body,
            Mode::NoNotebook => state_layout(body, dpi).list,
            _ => empty,
        }
    }
```

   - Every other `body_rect(area, dpi)` becomes `self.layout(area, dpi).body`.
   - `buttons()` and `more_menu` read `notebook_layout::root_parts(self.layout(client, dpi).root, dpi).buttons` instead of `header_layout(..).buttons`. The buttons exist only while `self.mode != Mode::NoNotebook`.
   - The states under the root (Loading, Empty, Failed) paint only while `self.root_expanded`. NoNotebook always paints, since its root row isn't collapsible.

2. **Syncing.**
   - In `apply`, set `self.root_expanded = snapshot.root_expanded` and `self.editors_expanded = snapshot.editors_expanded`. Add both fields to `Snapshot`. `snapshot()` reads `library_host::root_expanded(hwnd)` (true without a notebook) and `main_window::open_editors_expanded(hwnd)`.
   - Also add `root_expanded` to `RebuildKey`, so collapsing is a rebuild. `expansion_revision` already changes with it, so `RebuildKey` needs nothing else.
   - Add:

```rust
/// The tabs changed in some way the Open Editors rows show (a tab opened, closed, switched,
/// renamed, saved, made dirty or clean): the rows follow, and the panel repaints only if they
/// changed. Cheap: the tab list in memory, no rebuild of the tree.
pub(crate) fn editors_changed(hwnd: HWND) {
    let rows = super::open_editors::snapshot(hwnd);
    with_view(hwnd, |view| {
        if view.editors.set_rows(rows) {
            let area = view.client();
            let height = height(view.layout(area, view.dpi()).editors_list);
            if let Some(active) = view.editors.active_index() {
                view.editors.list.ensure_visible(active, height);
            }
            view.invalidate();
        }
    });
}
```

   - Call `editors_changed(hwnd)` from these places:
     - `active_tab_changed` (first line), which covers every `refresh_tabs`;
     - `library_host::refresh_label` (where `unsaved_label_changed` was);
     - `main_window::invalidate_title_strip`, after its invalidate;
     - the Scintilla savepoint notifications (search `SCN_SAVEPOINTLEFT` and `SCN_SAVEPOINTREACHED` in `main_window.rs`), after they set `dirty`;
     - `side_panel::refresh_now`, before `rebuild`.

3. **Hit test.** Replace `Hit` with:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Hit {
    /// The Open Editors header row: toggles the section.
    EditorsHeader,
    /// An Open Editors row; `close` on a clean tab's close box.
    Editor { index: usize, close: bool },
    /// The root row's chevron or name: toggles the tree.
    Root,
    /// A root row button.
    Header(HeaderButton),
    StateButton,
    SecondButton,
    Thumb(i32),
    Row { index: usize, part: RowPart },
    Empty,
}
```

   `hit_test` starts with:

```rust
        let area = self.client();
        let dpi = self.dpi();
        let layout = self.layout(area, dpi);
        if y < layout.title.bottom {
            return Hit::Empty;
        }
        if contains(layout.editors_header, x, y) {
            return Hit::EditorsHeader;
        }
        if contains(layout.editors_list, x, y) {
            let Some(index) = self.editors.list.row_at(y - layout.editors_list.top) else {
                return Hit::Empty;
            };
            let row = self.editors_row_rect(layout.editors_list, index);
            let clean = self.editors.rows.get(index).is_some_and(|row| !row.dirty);
            let close = clean
                && row.is_some_and(|row| contains(super::open_editors::close_rect(row, dpi), x, y));
            return Hit::Editor { index, close };
        }
        if contains(layout.root, x, y) {
            if self.mode != Mode::NoNotebook {
                let parts = notebook_layout::root_parts(layout.root, dpi);
                for (button, rect) in parts.buttons {
                    if contains(rect, x, y) {
                        return Hit::Header(button);
                    }
                }
                return Hit::Root;
            }
            return Hit::Empty;
        }
        let body = layout.body;
```

   The rest of `hit_test` stays as it is (the body's states and the tree), except that the tree branch applies only when `self.tree_shown()`. Add the helper:

```rust
    fn editors_row_rect(&self, list: RECT, index: usize) -> Option<RECT> {
        let top = self.editors.list.row_top(index)?;
        Some(RECT {
            left: list.left,
            top: list.top + top,
            right: list.right,
            bottom: list.top + top + self.editors.list.row_height,
        })
    }
```

   Add the `#[cfg(test)]` accessors `editor_rect_at(index)`, `editors_header_rect()` and `root_rect()`, which read `self.layout(self.client(), self.dpi())`.

4. **Painting.** Replace `paint_header` with `paint_sections`, and call it first in `paint`:

```rust
    fn paint_sections(&mut self, paint: &ViewPaint, layout: PanelLayout) {
        let (dc, palette, fonts, dpi) = (paint.hdc, &paint.palette, paint.fonts, paint.dpi);
        let bold = |text: &str, rect: RECT, color: u32| unsafe {
            draw_text(dc, text, rect, fonts.bold, color, LINE)
        };
        let title = RECT { left: layout.title.left + scale(12, dpi), ..layout.title };
        bold("NOTEBOOK", title, palette.muted_foreground);
        // Open Editors header: chevron, label and count.
        let chevron = notebook_layout::section_chevron(layout.editors_header, dpi);
        let glyph = if self.editors_expanded { GLYPH_CHEVRON_DOWN } else { GLYPH_CHEVRON_RIGHT };
        unsafe { draw_text(dc, glyph, chevron, fonts.glyph, palette.muted_foreground, CENTERED) };
        let label = RECT { left: chevron.right, ..layout.editors_header };
        let count = self.editors.rows.len();
        bold(&format!("OPEN EDITORS  {count}"), label, palette.muted_foreground);
        // The rows.
        let editors = &self.editors;
        let images = &mut self.images;
        let hover_close = editors.hover_close;
        row_list::paint(dc, layout.editors_list, &editors.list, palette, paint.focused, &mut |dc, index, rect, look| {
            if let Some(row) = editors.rows.get(index) {
                super::open_editors::draw_editor_row(dc, row, rect, look, paint, images, hover_close && look.hover);
            }
        });
        // The root row: chevron, name, and its buttons (not without a notebook).
        let parts = notebook_layout::root_parts(layout.root, dpi);
        if self.mode == Mode::NoNotebook {
            bold("NO NOTEBOOK", RECT { left: parts.chevron.right, ..layout.root }, palette.muted_foreground);
        } else {
            let glyph = if self.root_expanded { GLYPH_CHEVRON_DOWN } else { GLYPH_CHEVRON_RIGHT };
            unsafe { draw_text(dc, glyph, parts.chevron, fonts.glyph, palette.muted_foreground, CENTERED) };
            bold(&self.name.to_uppercase(), parts.name, palette.muted_foreground);
            for (button, rect) in parts.buttons {
                let hot = self.hover == Some(Hit::Header(button));
                if hot {
                    unsafe { fill(dc, rect, palette.hover_background) };
                }
                let glyph = match button {
                    HeaderButton::Favorite if self.favorite => GLYPH_STAR_FILLED,
                    HeaderButton::Favorite => GLYPH_STAR,
                    HeaderButton::NewNote => GLYPH_ADD,
                    HeaderButton::NewFolder => GLYPH_NEW_FOLDER,
                    HeaderButton::More => GLYPH_MORE,
                };
                let color = if hot { palette.hover_foreground } else { palette.muted_foreground };
                unsafe { draw_text(dc, glyph, rect, fonts.glyph, color, CENTERED) };
            }
        }
    }
```

   - In `paint`, set `self.editors.list.row_height = scale(ROW_HEIGHT, dpi);` next to the tree's, compute `let layout = self.layout(area, dpi);`, call `self.paint_sections(paint, layout)`, and use `layout.body` in place of `body_rect(area, dpi)`.
   - The `Mode::Tree` arm paints only `if self.root_expanded`.
   - In `side_panel::paint_view`, call `crate::window::notebook_view::editors_changed(main)` before `notebook_view::paint`, so a paint never shows stale rows. `editors_changed` only repaints when they changed, so this doesn't loop: an `InvalidateRect` during `WM_PAINT` of the same window just marks it dirty again after `EndPaint`. With identical rows it does nothing.

5. **Mouse.** In `left_down`, add these arms before `Hit::Header`:

```rust
        Hit::EditorsHeader => {
            let expanded = with_view(hwnd, |view| view.editors_expanded).unwrap_or(true);
            super::main_window::set_open_editors_expanded(hwnd, !expanded);
            rebuild(hwnd);
        }
        Hit::Editor { index, close } => {
            let Some(row) = with_view(hwnd, |view| view.editors.rows.get(index).cloned()).flatten() else {
                return;
            };
            if close {
                super::main_window::close_document_tab(hwnd, row.id);
            } else {
                super::main_window::activate_document_by_id(hwnd, row.id);
            }
        }
        Hit::Root => {
            let expanded = super::library_host::root_expanded(hwnd);
            super::library_host::set_root_expanded(hwnd, !expanded);
            rebuild(hwnd);
        }
```

   - Delete the `Hit::Title` arm.
   - In `mouse_move`:
     - set `editors.list` hover and `editors.hover_close` from `Hit::Editor`;
     - clear the tree's `list.hover` when the hit isn't a tree row, and clear the editors' hover when it isn't an editor row;
     - `hot` also includes `Hit::EditorsHeader | Hit::Root`.
   - In `WM_MOUSELEAVE`, also clear `editors.list.hover` and `editors.hover_close`.
   - `WM_MOUSEWHEEL`: if the pointer (`GetCursorPos` → `ScreenToClient`) is over `layout.editors_list`, scroll `editors.list` with `height(layout.editors_list)`. Otherwise scroll the tree, as now.
   - In `handle`, add:

```rust
        WM_MBUTTONDOWN => {
            let (x, y) = point_of(lparam);
            let pressed = with_view(hwnd, |view| match view.hit_test(x, y) {
                Hit::Editor { index, .. } => view.editors.rows.get(index).map(|row| row.id),
                _ => None,
            })
            .flatten();
            with_view(hwnd, |view| view.middle_press = pressed);
            Some(0)
        }
        WM_MBUTTONUP => {
            let (x, y) = point_of(lparam);
            let pressed = with_view(hwnd, |view| view.middle_press.take()).flatten();
            let released = with_view(hwnd, |view| match view.hit_test(x, y) {
                Hit::Editor { index, .. } => view.editors.rows.get(index).map(|row| row.id),
                _ => None,
            })
            .flatten();
            if let Some(id) = pressed.filter(|id| Some(*id) == released) {
                super::main_window::close_document_tab(hwnd, id);
            }
            Some(0)
        }
```

     This adds a `middle_press: Option<DocumentId>` field (initially `None`). In `side_panel::panel_proc`, add `WM_MBUTTONDOWN | WM_MBUTTONUP` to the list that goes to `route`, and to the `with_accessible_events` list.
   - In `main_window.rs`, next to `close_tab_at`:

```rust
/// Closes tab `id` as a middle-click on it does (open editors spec §3.2), if it is still open.
pub(crate) fn close_document_tab(hwnd: HWND, id: DocumentId) {
    let index = unsafe { app_ptr(hwnd) }
        .and_then(|app| unsafe { app.as_ref() }.tabs.documents().position(|document| document.id == id));
    if let Some(index) = index {
        close_tab_at(hwnd, index);
    }
}
```

6. **Tooltips.** `tooltip_tools` uses these rectangles:
   - `TOOL_TITLE` is the root name's rectangle (`root_parts(layout.root, dpi).name`), with the notebook path, and empty without a notebook.
   - The button tools use `root_parts(..).buttons`.
   - `TOOL_ROW` is the hovered editor row with `open_editors::tooltip(row)` when `editors.list.hover` is set. Otherwise it's the tree row tip, as before.

7. **Caption.**
   - `side_panel::header_is_caption`: the `PanelView::Notebook` arm is `true`.
   - `panel_hit_test` still limits caption to the top `HEADER_HEIGHT_96`.
   - Delete `notebook_view::header_hit`.

8. **Drag hover** (the tree drag keeps working in the new layout): `drag_hover` returns these values:
   - `Hover::Outside` above `layout.root.top` (the title band and Open Editors);
   - `Hover::Header` on the root row;
   - `Hover::Below` in the body when the tree isn't shown (a collapsed root);
   - otherwise what it returns today.

   `tree_drag::drop_folder` already maps `Header` and `Below` to the root.

- [ ] **Step 4: Run the tests.**

```bash
cargo clippy --all-targets -- -D warnings
cargo test --lib open_editors_ tree_drag_ notebook_view -- --test-threads=1
```

Also run the fixed caption test from Step 1. Expected: all pass.

- [ ] **Step 5: Commit** with the message `"feat(notebook): the Open Editors section and the notebook as a collapsible root row"`.

---

### Task 6: One keyboard selection through the panel, and screen readers

**Files:**
- Create: `src/window/panel_cursor.rs` (+ `mod.rs`)
- Modify: `src/window/notebook_view.rs`:
  - the `cursor` field;
  - `key_down`, `left` and `right`;
  - `focused_note`, `focused_folder` and `selected_folder`;
  - the `AccessibleView` impl;
  - painting the focus outline.
- Modify: `src/window/sidebar_accessibility.rs` (`section_item`)
- Modify: `src/window/main_window.rs` (tests)

**Interfaces:**
- Produces:
  - `pub(crate) enum Cursor { EditorsHeader, Editor(usize), Root, Tree }`, which is `Clone + Copy + Debug + Eq + PartialEq`
  - `pub(crate) struct Shape { pub editors: usize, pub root: bool, pub tree: usize }`: `editors` counts the visible rows (0 when collapsed), `root` is true when a notebook is open, and `tree` counts the rows while the tree is shown, else 0
  - `pub(crate) fn step(cursor: Cursor, tree_selected: Option<usize>, key: ListKey, shape: Shape) -> Option<(Cursor, Option<usize>)>`: `None` for `PageUp` and `PageDown`, which the section's own list handles. `Some((Cursor::Tree, Some(index)))` when the move lands in the tree.
  - `sidebar_accessibility::section_item(name: &str, expanded: bool, selected: bool, focused: bool, rect: RECT) -> AccessibleItem`
  - `NotebookView.cursor: Cursor`, initially `Cursor::Tree`

- [ ] **Step 1: Write `panel_cursor` with its tests.**

```rust
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
    let root = shape.editors + 1;
    match cursor {
        Cursor::EditorsHeader => 0,
        Cursor::Editor(index) => 1 + index.min(shape.editors.saturating_sub(1)),
        Cursor::Root if shape.root => root,
        Cursor::Root => shape.editors,
        Cursor::Tree if shape.tree > 0 => {
            root + usize::from(shape.root) - 1 + 1 + tree_selected.unwrap_or(0).min(shape.tree - 1)
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

    const SHAPE: Shape = Shape { editors: 2, root: true, tree: 3 };

    #[test]
    fn up_and_down_cross_from_open_editors_through_the_root_into_the_tree() {
        // Break caught: Down stuck at the last tab, the root row skipped, or Up from the first
        // tree row jumping to the top (open editors spec §3.5).
        let down = |cursor, tree| step(cursor, tree, ListKey::Down, SHAPE).unwrap();
        assert_eq!(down(Cursor::EditorsHeader, None), (Cursor::Editor(0), None));
        assert_eq!(down(Cursor::Editor(1), None), (Cursor::Root, None));
        assert_eq!(down(Cursor::Root, None), (Cursor::Tree, Some(0)));
        assert_eq!(down(Cursor::Tree, Some(2)), (Cursor::Tree, Some(2)), "stays on the last");
        let up = |cursor, tree| step(cursor, tree, ListKey::Up, SHAPE).unwrap();
        assert_eq!(up(Cursor::Tree, Some(0)), (Cursor::Root, None));
        assert_eq!(up(Cursor::Root, None), (Cursor::Editor(1), None));
        assert_eq!(up(Cursor::EditorsHeader, None), (Cursor::EditorsHeader, None));
    }

    #[test]
    fn home_end_and_collapsed_sections() {
        assert_eq!(step(Cursor::Root, None, ListKey::End, SHAPE), Some((Cursor::Tree, Some(2))));
        assert_eq!(step(Cursor::Tree, Some(1), ListKey::Home, SHAPE), Some((Cursor::EditorsHeader, None)));
        assert_eq!(step(Cursor::Tree, Some(1), ListKey::PageDown, SHAPE), None);
        let collapsed = Shape { editors: 0, root: true, tree: 0 };
        assert_eq!(step(Cursor::EditorsHeader, None, ListKey::Down, collapsed), Some((Cursor::Root, None)));
        assert_eq!(step(Cursor::Root, None, ListKey::Down, collapsed), Some((Cursor::Root, None)));
        let no_notebook = Shape { editors: 1, root: false, tree: 0 };
        assert_eq!(step(Cursor::Editor(0), None, ListKey::Down, no_notebook), Some((Cursor::Editor(0), None)));
    }
}
```

Simplify `index_of`'s `Cursor::Tree` arm when you implement it. The expression is `shape.editors + usize::from(shape.root) + 1 + selected`. The tests pin the result.

- [ ] **Step 2: Run the tests and check they fail, implement, then run them and check they pass.** Use `cargo test --lib panel_cursor`. The first run fails; after implementing, expect 2 passed.

- [ ] **Step 3: Write the failing window tests.**

```rust
    #[test]
    fn the_arrow_keys_cross_from_open_editors_into_the_tree_and_del_on_a_tab_row_deletes_nothing() {
        // Break caught: the keyboard stuck in the tree, Enter on a tab row doing nothing, or Del
        // on a tab row deleting the tree's selected note (open editors spec §3.5).
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_DELETE, VK_DOWN, VK_HOME, VK_RETURN};
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("editors-keys");
        let a = scratch.note("a.md", "a");
        let b = scratch.note("b.md", "b");
        let (window, _editor) = notebook_window(&scratch);
        super::open_path(window.hwnd, &a).unwrap();
        super::open_path(window.hwnd, &b).unwrap();
        let key = |vk: u16| crate::window::notebook_view::key_down(window.hwnd, vk);
        key(VK_HOME);
        assert_eq!(notebook_view(window.hwnd).cursor, crate::window::panel_cursor::Cursor::EditorsHeader);
        key(VK_DOWN);
        key(VK_DELETE);
        assert!(a.exists() && b.exists(), "Del on a tab row deletes nothing");
        key(VK_RETURN);
        assert_eq!(app_mut(window.hwnd).tabs.active().unwrap().path.as_deref(), Some(a.as_path()));
        for _ in 0..3 {
            key(VK_DOWN);
        }
        assert_eq!(notebook_view(window.hwnd).cursor, crate::window::panel_cursor::Cursor::Tree);
        assert_eq!(selected_kind(window.hwnd), Some(RowKind::Note("a.md".into())));
    }

    #[test]
    fn screen_readers_see_the_sections_and_the_tab_rows() {
        // Break caught: Open Editors rows invisible to screen readers, or headers without their
        // expanded state (open editors spec §7).
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("editors-msaa");
        let a = scratch.note("a.md", "a");
        let (window, editor) = notebook_window(&scratch);
        super::open_path(window.hwnd, &a).unwrap();
        editor.set_text("changed").unwrap();
        let panel = sidebar_windows(window.hwnd).1;
        let count = crate::window::side_panel::accessible_item_count(panel);
        let names: Vec<String> = (0..count)
            .filter_map(|index| crate::window::side_panel::accessible_item(panel, index))
            .map(|item| item.name)
            .collect();
        assert!(names.contains(&"Open editors, 1".to_owned()), "{names:?}");
        assert!(names.contains(&"a.md, open editor, modified".to_owned()), "{names:?}");
        assert!(names.iter().any(|name| name == &crate::window::library_host::notebook_name(&scratch.folder())));
    }
```

- [ ] **Step 4: Implement.**
   1. Add the `cursor: Cursor` field, initialised to `Cursor::Tree`. Add `fn shape(&self) -> Shape`:

```rust
    fn shape(&self) -> panel_cursor::Shape {
        panel_cursor::Shape {
            editors: if self.editors_expanded { self.editors.rows.len() } else { 0 },
            root: self.mode != Mode::NoNotebook,
            tree: if self.tree_shown() { self.rows.len() } else { 0 },
        }
    }
```

   2. `key_down`: a list key goes through `panel_cursor::step`:

```rust
    if let Some(list_key) = ListKey::from_virtual_key(u32::from(key)) {
        with_view(hwnd, |view| {
            let shape = view.shape();
            match panel_cursor::step(view.cursor, view.list.selected, list_key, shape) {
                Some((cursor, tree)) => {
                    view.cursor = cursor;
                    if let Some(index) = tree {
                        view.select(index);
                    }
                    if let panel_cursor::Cursor::Editor(index) = cursor {
                        let height = height(view.layout(view.client(), view.dpi()).editors_list);
                        view.editors.list.ensure_visible(index, height);
                    }
                }
                None if view.cursor == panel_cursor::Cursor::Tree => {
                    let height = view.list_height();
                    view.list.move_selection(list_key, height);
                }
                None => {}
            }
            view.invalidate();
        });
        return true;
    }
```

      The NoNotebook state's RECENT list keeps today's behaviour: while `mode == NoNotebook` and the cursor is `Tree`, list keys move `list` as before, because `shape.tree` is 0 there and the old `move_selection` path applies. Implement that as an early branch.

      After a list key, if the cursor isn't `Tree`, the rest of `key_down` dispatches on the cursor:
      - `VK_RETURN` on `Editor(index)` activates that tab, then `focus_content`.
      - `VK_RETURN` on `EditorsHeader` toggles the setting; on `Root` it toggles `set_root_expanded`. Both rebuild.
      - `VK_LEFT` and `VK_RIGHT` on a header collapse or expand it. On `Editor(_)` they do nothing.
      - `VK_F2` and `VK_DELETE` do nothing unless the cursor is `Tree`.
      - `VK_LEFT` on a top-level tree row (no parent and not an expanded folder) moves the cursor to `Root`.

   3. A click sets the cursor:
      - `Hit::Row` sets `Cursor::Tree`;
      - `Hit::Editor { index, .. }` sets `Cursor::Editor(index)`;
      - `Hit::EditorsHeader` sets `Cursor::EditorsHeader`;
      - `Hit::Root` and `Hit::Header(_)` set `Cursor::Root`.
      
      `active_tab_changed`'s tree selection leaves the cursor alone.

   4. `focused_note`, `focused_folder` and `selected_folder` also require `view.cursor == Cursor::Tree`. `selected_folder` returns `None` otherwise, which means the root.

   5. Paint a 1 px (scaled) outline in `palette.selection_background` round the cursor's header row or `Editor` row while `paint.focused` and the cursor isn't `Tree`. Use four `fill` calls, as `paint_band` does. The tree keeps its own selection look.

   6. Add `sidebar_accessibility::section_item`:

```rust
/// A section header in a sidebar view (the Notebook view's Open Editors and notebook rows): an
/// outline item that is expanded or collapsed, selectable with the keyboard.
pub(crate) fn section_item(
    name: &str,
    expanded: bool,
    selected: bool,
    focused: bool,
    rect: RECT,
) -> AccessibleItem {
    let mut state = row_state(selected, focused, true);
    state |= if expanded { STATE_EXPANDED } else { STATE_COLLAPSED };
    AccessibleItem {
        name: name.to_owned(),
        role: ROLE_SYSTEM_OUTLINEITEM,
        state,
        rect,
        value: "0".to_owned(),
        window: std::ptr::null_mut(),
    }
}
```

   7. The `AccessibleView` impl gets a new child order:
      1. buttons (the root row's, then the state's);
      2. `Open editors, <n>`;
      3. the editor rows;
      4. the root row (not in the NoNotebook state);
      5. RECENT;
      6. the tree rows, shown only while `tree_shown()`.
      
      Write one private helper that every method uses:

```rust
    /// The children after the push buttons, in order: the Open Editors header, its rows, the
    /// root row, then the RECENT notebooks or the tree rows.
    fn accessible_parts(&self) -> (usize, usize, usize) {
        let editors = if self.editors_expanded { self.editors.rows.len() } else { 0 };
        let root = usize::from(self.mode != Mode::NoNotebook);
        (1, editors, root)
    }
```

      With `b = buttons.len()` and `(h, e, r) = accessible_parts()`:
      - index `b` is the header: `section_item(&format!("Open editors, {}", self.editors.rows.len()), self.editors_expanded, self.cursor == Cursor::EditorsHeader, focused, layout.editors_header)`;
      - `b+1 .. b+1+e` are `list_item(&open_editors::accessible_name(row), self.cursor == Cursor::Editor(i), focused, rect, visible)`, using `row_rect(layout.editors_list, &self.editors.list, i)`;
      - `b+1+e` (if `r == 1`) is `section_item(&self.name, self.root_expanded, self.cursor == Cursor::Root, focused, layout.root)`;
      - after it come RECENT, then the tree rows (only while `tree_shown()`), offset by `b + h + e + r`.
      
      Then:
      - `accessible_current` maps the cursor back to that index.
      - `accessible_select` sets the cursor, and for tree rows also selects in the tree list.
      - `accessible_hit` tests the header, the editor list and the root row rectangles before the list.
      - `accessible_identity` uses these:
        - `identity_of(&"open-editors")` for the header;
        - `identity_of(&("editor", row.id.0))` for an editor row;
        - `identity_of(&"notebook-root")` for the root row;
        - the existing identities for the rest.
      - `accessible_count` is `b + h + e + r + recent + rows`.
      
      `order` is also bumped by `editors_changed` when the rows' IDs change, so a reorder is announced.

- [ ] **Step 5: Run the tests.** Run `cargo test --lib panel_cursor the_arrow_keys_cross screen_readers_see_the_sections sidebar_accessibility notebook_view -- --test-threads=1`. Expected: all pass. The existing accessibility window tests that count children (search `accessible_item_count` in `main_window.rs` tests) need their expected counts and indices shifted by the new header, editor and root children. Update each one's numbers. Don't change what they assert about tree rows.

- [ ] **Step 6: Commit** with the message `"feat(notebook): one keyboard selection through Open Editors and the tree, and their screen reader children"`.

---

### Task 7: The copy plan and the disk copy

**Files:**
- Create: `src/window/tree_copy.rs` (+ `mod.rs`)
- Modify: `src/platform/files.rs`

**Interfaces:**
- Consumes: `crate::library::{at_or_under, model::same_path}`, `library_host::notebook_name`.
- Produces:
  - `platform::files::copy_file_no_replace(from: &Path, to: &Path) -> Result<()>`, which is `CopyFileExW` with `COPY_FILE_FAIL_IF_EXISTS`;
  - `platform::files::copy_tree(from: &Path, to: &Path, cancel: &AtomicBool) -> Copied`, where `pub struct Copied { pub files: usize, pub error: Option<(PathBuf, crate::FastPadError)> }`. It copies a file, or a folder recursively with an explicit stack. `to` must not exist. It stops at the first error, and a set `cancel` stops it after the file in hand.
  - `tree_copy::Refusal { SamePlace, IntoItself, HoldsSource }`
  - `tree_copy::Planned { pub source: PathBuf, pub destination: PathBuf, pub outcome: Outcome }` with `pub enum Outcome { Copy, Clash, Refused(Refusal) }`
  - `tree_copy::plan(sources: &[PathBuf], root: &Path, folder: &Path, exists: &dyn Fn(&Path) -> bool) -> Vec<Planned>`. `sources` are absolute, `folder` is relative to `root` (empty is the root), and `destination` is absolute.
  - `tree_copy::any_accepted(sources: &[PathBuf], root: &Path, folder: &Path) -> bool`: at least one source that isn't refused. This is memory only; the `exists` check isn't needed to refuse.
  - `tree_copy::item_name(path: &Path) -> String`
  - notice functions: `replace_question(name, folder_label)`, `hidden_notice(names: &[String])`, `dirty_notice(name)`, `failed_notice(name, reason: &str, copied_before: Option<usize>)`, `refused_notice(name, Refusal)`, `recycle_failed_notice(name)`

- [ ] **Step 1: Write the tests.** `tree_copy.rs` `mod tests`:

```rust
    use super::*;
    use std::path::{Path, PathBuf};

    const ROOT: &str = r"C:\notes";

    fn plan_of(sources: &[&str], folder: &str, existing: &[&str]) -> Vec<(String, Outcome)> {
        let sources: Vec<PathBuf> = sources.iter().map(PathBuf::from).collect();
        let existing: Vec<PathBuf> = existing.iter().map(PathBuf::from).collect();
        let exists = |path: &Path| existing.iter().any(|known| same_path(known, path));
        plan(&sources, Path::new(ROOT), Path::new(folder), &exists)
            .into_iter()
            .map(|planned| (planned.destination.display().to_string(), planned.outcome))
            .collect()
    }

    #[test]
    fn a_file_lands_under_its_own_name_in_the_target_folder() {
        assert_eq!(
            plan_of(&[r"D:\x\draft.txt"], "work", &[]),
            [(r"C:\notes\work\draft.txt".into(), Outcome::Copy)]
        );
        assert_eq!(
            plan_of(&[r"D:\x\draft.txt"], "", &[]),
            [(r"C:\notes\draft.txt".into(), Outcome::Copy)],
            "empty is the root"
        );
    }

    #[test]
    fn a_copy_onto_itself_is_refused_before_any_prompt() {
        // Break caught (Review Focus 1): "Replace?" offered for a note onto its own folder, so
        // OK would recycle the source and lose it.
        assert_eq!(
            plan_of(&[r"C:\notes\work\a.md"], "work", &[r"C:\notes\work\a.md"]),
            [(r"C:\notes\work\a.md".into(), Outcome::Refused(Refusal::SamePlace))]
        );
        assert_eq!(
            plan_of(&[r"C:\NOTES\Work\A.md"], "work", &[r"C:\notes\work\a.md"])[0].1,
            Outcome::Refused(Refusal::SamePlace),
            "letter case doesn't matter"
        );
    }

    #[test]
    fn a_folder_into_itself_or_below_is_refused() {
        assert_eq!(
            plan_of(&[r"C:\notes\work"], r"work\inner", &[])[0].1,
            Outcome::Refused(Refusal::IntoItself)
        );
        assert_eq!(plan_of(&[r"C:\"], "work", &[])[0].1, Outcome::Refused(Refusal::IntoItself));
    }

    #[test]
    fn a_destination_that_holds_the_source_is_refused() {
        // Break caught (Review Focus 2): notes\work\work dropped on the root would replace
        // notes\work, recycling the source with it.
        assert_eq!(
            plan_of(&[r"C:\notes\work\work"], "", &[r"C:\notes\work"])[0].1,
            Outcome::Refused(Refusal::HoldsSource)
        );
    }

    #[test]
    fn a_taken_name_is_a_clash_and_the_rest_carry_on() {
        assert_eq!(
            plan_of(&[r"D:\a.md", r"D:\b.md"], "", &[r"C:\notes\a.md"]),
            [
                (r"C:\notes\a.md".into(), Outcome::Clash),
                (r"C:\notes\b.md".into(), Outcome::Copy)
            ]
        );
        assert!(any_accepted(&[PathBuf::from(r"C:\notes\a.md"), PathBuf::from(r"D:\b.md")], Path::new(ROOT), Path::new("")));
        assert!(!any_accepted(&[PathBuf::from(r"C:\notes\a.md")], Path::new(ROOT), Path::new("")));
    }

    #[test]
    fn the_notices_read_as_the_spec_words_them() {
        assert_eq!(replace_question("a.md", "work"), "a.md already exists in work. Replace it?");
        assert_eq!(hidden_notice(&["photo.png".into()]), "photo.png was copied but isn't shown: the notebook lists text notes only.");
        assert_eq!(
            hidden_notice(&["a.png".into(), "b.png".into(), "c.png".into(), "d.png".into()]),
            "4 files were copied but aren't shown: a.png, b.png, c.png, …"
        );
        assert_eq!(dirty_notice("draft.txt"), "Copied the saved version of draft.txt. Your unsaved changes are still in its tab.");
        assert_eq!(failed_notice("a.md", "Access is denied", None), "a.md could not be copied: Access is denied.");
        assert_eq!(
            failed_notice("pics", "The disk is full", Some(3)),
            "pics could not be copied: The disk is full. 3 files were copied before the failure."
        );
        assert_eq!(refused_notice("a.md", Refusal::SamePlace), "a.md was not copied: it is already there.");
        assert_eq!(refused_notice("work", Refusal::IntoItself), "work was not copied: a folder can't be copied into itself.");
        assert_eq!(refused_notice("work", Refusal::HoldsSource), "work was not copied: it would replace the folder it is in.");
        assert_eq!(recycle_failed_notice("a.md"), "a.md was not copied: it could not be moved to the Recycle Bin.");
    }
```

`platform/files.rs` tests:

```rust
    #[test]
    fn copying_a_file_never_replaces_and_a_folder_copies_whole() {
        // Break caught: a copy over an existing note, a nested folder copied flat or partly, or
        // non-note files left behind (open editors spec §4.5).
        let dir = scratch("copy");
        std::fs::write(dir.join("a.md"), "a").unwrap();
        std::fs::write(dir.join("taken.md"), "keep").unwrap();
        assert!(copy_file_no_replace(&dir.join("a.md"), &dir.join("taken.md")).is_err());
        assert_eq!(std::fs::read_to_string(dir.join("taken.md")).unwrap(), "keep");
        copy_file_no_replace(&dir.join("a.md"), &dir.join("b.md")).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("b.md")).unwrap(), "a");

        let from = dir.join("pics");
        std::fs::create_dir_all(from.join(r"deep\er")).unwrap();
        std::fs::write(from.join("x.png"), [1u8, 2]).unwrap();
        std::fs::write(from.join(r"deep\er\y.md"), "y").unwrap();
        std::fs::create_dir_all(from.join("empty")).unwrap();
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let copied = copy_tree(&from, &dir.join("copy"), &cancel);
        assert!(copied.error.is_none());
        assert_eq!(copied.files, 2);
        assert_eq!(std::fs::read(dir.join(r"copy\x.png")).unwrap(), [1, 2]);
        assert_eq!(std::fs::read_to_string(dir.join(r"copy\deep\er\y.md")).unwrap(), "y");
        assert!(dir.join(r"copy\empty").is_dir());
        let again = copy_tree(&from, &dir.join("copy"), &cancel);
        assert!(again.error.is_some() && again.files == 0, "an existing folder is never merged into");
        let single = copy_tree(&dir.join("a.md"), &dir.join("c.md"), &cancel);
        assert_eq!((single.files, single.error.is_none()), (1, true));
        let _ = std::fs::remove_dir_all(&dir);
    }
```

- [ ] **Step 2: Run the tests and check they fail.** Run `cargo test --lib tree_copy copying_a_file_never_replaces`. Expected: compile errors.

- [ ] **Step 3: Implement.**

`platform/files.rs`: add `COPY_FILE_FAIL_IF_EXISTS` and `CopyFileExW` to the `Storage::FileSystem` import, then add:

```rust
/// `CopyFileExW` with `COPY_FILE_FAIL_IF_EXISTS`: fails if `to` already exists.
pub fn copy_file_no_replace(from: &Path, to: &Path) -> Result<()> {
    let (from_wide, to_wide) = (wide(from), wide(to));
    let copied = unsafe {
        CopyFileExW(
            from_wide.as_ptr(),
            to_wide.as_ptr(),
            None,
            std::ptr::null(),
            std::ptr::null_mut(),
            COPY_FILE_FAIL_IF_EXISTS,
        )
    };
    if copied == 0 {
        return Err(last_error());
    }
    Ok(())
}

/// What `copy_tree` did: the files copied, and the first error with the path it happened at.
#[derive(Debug)]
pub struct Copied {
    pub files: usize,
    pub error: Option<(std::path::PathBuf, crate::FastPadError)>,
}

/// Copies the file or folder `from` to `to`, which must not exist: a folder is created with
/// everything in it, never merged into one that is there. Stops at the first error, and after
/// the file in hand once `cancel` is set. An explicit stack, so a deep folder can't overflow.
pub fn copy_tree(from: &Path, to: &Path, cancel: &std::sync::atomic::AtomicBool) -> Copied {
    use std::sync::atomic::Ordering;
    let mut copied = Copied { files: 0, error: None };
    let fail = |copied: &mut Copied, path: &Path, error: crate::FastPadError| {
        copied.error = Some((path.to_path_buf(), error));
    };
    if !from.is_dir() {
        match copy_file_no_replace(from, to) {
            Ok(()) => copied.files = 1,
            Err(error) => fail(&mut copied, from, error),
        }
        return copied;
    }
    let mut stack = vec![(from.to_path_buf(), to.to_path_buf())];
    while let Some((source, target)) = stack.pop() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        if let Err(error) = std::fs::create_dir(&target) {
            fail(&mut copied, &target, error.into());
            break;
        }
        let entries = match std::fs::read_dir(&source) {
            Ok(entries) => entries,
            Err(error) => {
                fail(&mut copied, &source, error.into());
                break;
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    fail(&mut copied, &source, error.into());
                    return copied;
                }
            };
            let (inner, outer) = (entry.path(), target.join(entry.file_name()));
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                stack.push((inner, outer));
            } else {
                if cancel.load(Ordering::Relaxed) {
                    return copied;
                }
                match copy_file_no_replace(&inner, &outer) {
                    Ok(()) => copied.files += 1,
                    Err(error) => {
                        fail(&mut copied, &inner, error);
                        return copied;
                    }
                }
            }
        }
    }
    copied
}
```

(If `FastPadError` has no `From<std::io::Error>`, map with the constructor the crate uses for I/O errors: search `impl From<std::io::Error> for FastPadError` in `src/error.rs`.)

`tree_copy.rs`:

```rust
//! Copying files into the notebook tree (open editors spec §4): where each dropped item lands,
//! which ones are refused, which clash with a name already there, and the words the prompts and
//! notices use. Pure but for the one `exists` check per item the caller passes in.

use crate::library::{at_or_under, model::same_path};
use std::path::{Path, PathBuf};

/// Why an item is not copied (spec §4.2).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Refusal {
    /// The destination is the item itself.
    SamePlace,
    /// A folder into itself or a folder inside it.
    IntoItself,
    /// The destination holds the item: replacing it would recycle the item too.
    HoldsSource,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Outcome {
    Copy,
    /// Something already has the destination's name: ask before replacing it.
    Clash,
    Refused(Refusal),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Planned {
    pub source: PathBuf,
    pub destination: PathBuf,
    pub outcome: Outcome,
}

/// The refusal for `source` landing at `destination` inside the folder `target`, if any.
fn refusal(source: &Path, target: &Path, destination: &Path) -> Option<Refusal> {
    if same_path(source, destination) {
        Some(Refusal::SamePlace)
    } else if at_or_under(target, source) {
        Some(Refusal::IntoItself)
    } else if at_or_under(source, destination) {
        Some(Refusal::HoldsSource)
    } else {
        None
    }
}

/// Where each of `sources` lands in `folder` (relative to `root`; empty is the root), refused,
/// clashing or free. `exists` is asked once per item that isn't refused.
pub(crate) fn plan(
    sources: &[PathBuf],
    root: &Path,
    folder: &Path,
    exists: &dyn Fn(&Path) -> bool,
) -> Vec<Planned> {
    let target = root.join(folder);
    sources
        .iter()
        .filter_map(|source| {
            let destination = target.join(source.file_name()?);
            let outcome = match refusal(source, &target, &destination) {
                Some(refusal) => Outcome::Refused(refusal),
                None if exists(&destination) => Outcome::Clash,
                None => Outcome::Copy,
            };
            Some(Planned { source: source.clone(), destination, outcome })
        })
        .collect()
}

/// Whether dropping `sources` into `folder` copies anything: the drag's target test, in memory.
pub(crate) fn any_accepted(sources: &[PathBuf], root: &Path, folder: &Path) -> bool {
    let target = root.join(folder);
    sources.iter().any(|source| {
        source
            .file_name()
            .is_some_and(|name| refusal(source, &target, &target.join(name)).is_none())
    })
}

pub(crate) fn item_name(path: &Path) -> String {
    path.file_name()
        .map_or_else(|| path.display().to_string(), |name| name.to_string_lossy().into_owned())
}

pub(crate) fn replace_question(name: &str, folder: &str) -> String {
    format!("{name} already exists in {folder}. Replace it?")
}

pub(crate) fn hidden_notice(names: &[String]) -> String {
    match names {
        [one] => format!("{one} was copied but isn't shown: the notebook lists text notes only."),
        _ => {
            let mut listed = names.iter().take(3).cloned().collect::<Vec<_>>().join(", ");
            if names.len() > 3 {
                listed.push_str(", …");
            }
            format!("{} files were copied but aren't shown: {listed}", names.len())
        }
    }
}

pub(crate) fn dirty_notice(name: &str) -> String {
    format!("Copied the saved version of {name}. Your unsaved changes are still in its tab.")
}

pub(crate) fn failed_notice(name: &str, reason: &str, copied_before: Option<usize>) -> String {
    let reason = reason.trim_end_matches(['.', ' ', '\r', '\n']);
    match copied_before {
        Some(files) => format!(
            "{name} could not be copied: {reason}. {files} files were copied before the failure."
        ),
        None => format!("{name} could not be copied: {reason}."),
    }
}

pub(crate) fn refused_notice(name: &str, refusal: Refusal) -> String {
    let why = match refusal {
        Refusal::SamePlace => "it is already there",
        Refusal::IntoItself => "a folder can't be copied into itself",
        Refusal::HoldsSource => "it would replace the folder it is in",
    };
    format!("{name} was not copied: {why}.")
}

pub(crate) fn recycle_failed_notice(name: &str) -> String {
    format!("{name} was not copied: it could not be moved to the Recycle Bin.")
}
```

- [ ] **Step 4: Run the tests and check they pass.** Run `cargo test --lib tree_copy copying_a_file_never_replaces`. Expected: 7 passed.
- [ ] **Step 5: Commit** with the message `"feat(notebook): plan a copy into the tree and copy files and folders without replacing"`.

---

### Task 8: The copy host: prompts, the worker and the results

**Files:**
- Create: `src/window/copy_host.rs` (+ `mod.rs`)
- Modify: `src/window/messages.rs` (`WM_FASTPAD_COPY_DONE = WM_APP + 18`), `src/window/mod.rs` (re-export it)
- Modify: `src/window/main_window.rs` (dispatch the message next to `WM_FASTPAD_REPLACE_RELOADED` at `:750`; the tests)
- Modify: `src/window/library_host.rs` (`confirmed` becomes `pub(crate)`; add the `copy_worker` field to `LibraryHost` and its `Default`/constructor)

**Interfaces:**
- Consumes:
  - `tree_copy::*` and `platform::files::{copy_tree, recycle, recycle_folder}`;
  - `library_host::{folder, notebook_name, confirmed, with_state, set_expanded, request_rescan, tabs_under}`;
  - `main_window::{push_notice, reload_clean_document, TabMark, file_population_active}`, `modal::modal_active`;
  - `notebook_view::select_row`, `side_panel::{refresh, with_accessible_events}`.
- Produces:
  - `pub(crate) fn copy_into(hwnd: HWND, sources: Vec<PathBuf>, folder: &Path, dirty_tab: Option<String>)`: `folder` is relative to the notebook, and `dirty_tab` names the tab whose saved version is copied.
  - `pub(crate) fn copy_tab_into(hwnd: HWND, id: DocumentId, path: &Path, folder: &Path)`
  - `pub(crate) struct CopyWorker`, which is `Default` and ends its thread on `Drop`
  - `pub(crate) fn copy_done(hwnd: HWND, lparam: LPARAM)`
  - `#[cfg(test)] pub(crate) fn wait_for_copies(hwnd: HWND)`, which pumps until the worker's queue is empty and its results are handled

- [ ] **Step 1: Write the failing window tests.**

```rust
    #[test]
    fn copy_host_copies_files_and_folders_indexes_notes_and_says_what_is_hidden() {
        // Break caught: a copied note missing from the tree until a rescan, a copied folder not
        // listed, a non-note copied silently, or the single copied row not selected
        // (open editors spec §4.5, §4.6).
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("copy-into");
        std::fs::create_dir_all(scratch.folder().join("work")).unwrap();
        let outside = scratch.root.join("outside");
        std::fs::create_dir_all(outside.join(r"pics\deep")).unwrap();
        std::fs::write(outside.join("draft.md"), "d").unwrap();
        std::fs::write(outside.join(r"pics\deep\x.png"), [1u8]).unwrap();
        std::fs::write(outside.join("photo.png"), [1u8]).unwrap();
        let (window, _editor) = notebook_window(&scratch);

        crate::window::copy_host::copy_into(window.hwnd, vec![outside.join("draft.md")], std::path::Path::new("work"), None);
        crate::window::copy_host::wait_for_copies(window.hwnd);
        assert!(scratch.folder().join(r"work\draft.md").exists());
        assert!(outside.join("draft.md").exists(), "a copy, not a move");
        assert_eq!(selected_kind(window.hwnd), Some(RowKind::Note(r"work\draft.md".into())));

        crate::window::copy_host::copy_into(
            window.hwnd,
            vec![outside.join("pics"), outside.join("photo.png")],
            std::path::Path::new(""),
            None,
        );
        crate::window::copy_host::wait_for_copies(window.hwnd);
        assert!(scratch.folder().join(r"pics\deep\x.png").exists());
        assert!(scratch.folder().join("photo.png").exists());
        assert!(notices(window.hwnd).iter().any(|notice| notice.contains("isn't shown") || notice.contains("aren't shown")));
    }

    #[test]
    fn copy_host_a_clash_asks_ok_replaces_and_cancel_skips() {
        // Break caught: a clash replaced without asking, Cancel stopping the whole drop, or a
        // replaced clean tab left showing the old text (open editors spec §4.4, §4.7).
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("copy-clash");
        let a = scratch.note("a.md", "old a");
        scratch.note("b.md", "old b");
        let outside = scratch.root.join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("a.md"), "new a").unwrap();
        std::fs::write(outside.join("b.md"), "new b").unwrap();
        std::fs::write(outside.join("c.md"), "new c").unwrap();
        let (window, editor) = notebook_window(&scratch);
        super::open_path(window.hwnd, &a).unwrap();

        crate::window::modal::answer_next_confirm(true);
        crate::window::modal::answer_next_confirm(false);
        crate::window::copy_host::copy_into(
            window.hwnd,
            vec![outside.join("a.md"), outside.join("b.md"), outside.join("c.md")],
            std::path::Path::new(""),
            None,
        );
        crate::window::copy_host::wait_for_copies(window.hwnd);
        let folder = crate::window::library_host::notebook_name(&scratch.folder());
        assert_eq!(
            crate::window::modal::take_last_confirm().as_deref(),
            Some(format!("b.md already exists in {folder}. Replace it?").as_str())
        );
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "new a");
        assert_eq!(std::fs::read_to_string(scratch.folder().join("b.md")).unwrap(), "old b");
        assert_eq!(std::fs::read_to_string(scratch.folder().join("c.md")).unwrap(), "new c");
        assert_eq!(editor.text().unwrap(), "new a", "the clean tab reloaded");
    }

    #[test]
    fn copy_host_a_failed_recycle_skips_that_item_and_says_so() {
        // Break caught (Review Focus 3): an item copied (or half-copied) after its Recycle Bin
        // step failed, or the rest of the drop abandoned.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("copy-recycle-fails");
        scratch.note("a.md", "old a");
        let outside = scratch.root.join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("a.md"), "new a").unwrap();
        std::fs::write(outside.join("b.md"), "new b").unwrap();
        let (window, _editor) = notebook_window(&scratch);
        crate::window::copy_host::fail_next_recycle();
        crate::window::modal::answer_next_confirm(true);
        crate::window::copy_host::copy_into(
            window.hwnd,
            vec![outside.join("a.md"), outside.join("b.md")],
            std::path::Path::new(""),
            None,
        );
        crate::window::copy_host::wait_for_copies(window.hwnd);
        assert_eq!(std::fs::read_to_string(scratch.folder().join("a.md")).unwrap(), "old a");
        assert!(scratch.folder().join("b.md").exists());
        assert!(notices(window.hwnd).contains(&"a.md was not copied: it could not be moved to the Recycle Bin.".to_owned()));
    }
```

(If `modal::take_last_confirm` returns only the last question asked, the assert above checks the second prompt. That is intended, since the first was for `a.md`.)

- [ ] **Step 2: Run the tests and check they fail.** Run `cargo test --lib copy_host_ -- --test-threads=1`. Expected: compile errors.

- [ ] **Step 3: Implement `copy_host.rs`.**

```rust
//! Copying into the notebook tree (open editors spec §4.4-§4.7): the plan and the clash prompts
//! on the UI thread, each answered replace sent to the Recycle Bin there as Delete does, the
//! copies on one queued worker, and the result back on the UI thread, which indexes the new
//! notes, selects a single copied row, reloads replaced clean tabs and says what happened.

use super::library_host;
use super::main_window::{TabMark, app_ptr, push_notice};
use super::tree_copy::{self, Outcome, Planned};
use crate::document::DocumentId;
use crate::library;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use windows_sys::Win32::Foundation::{HWND, LPARAM};
use windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW;

/// One drop's copies, in order, as the worker gets them.
struct CopyJob {
    target: isize,
    items: Vec<(PathBuf, PathBuf)>,
    /// Clean tabs open on a replaced file, to read again once it is copied.
    reloads: Vec<(DocumentId, PathBuf, TabMark)>,
    /// Notices known before the copy: refusals, failed recycles, the dirty tab's.
    notices: Vec<String>,
    /// The relative folder copied into, for the selection.
    folder: PathBuf,
}

/// One item's result.
#[derive(Debug)]
struct ItemDone {
    destination: PathBuf,
    is_folder: bool,
    copied: library_copy::Result,
}

mod library_copy {
    pub(super) type Result = crate::platform::files::Copied;
}

/// `WM_FASTPAD_COPY_DONE`'s payload.
#[derive(Debug)]
pub(crate) struct CopyDone {
    items: Vec<ItemDone>,
    reloads: Vec<Reload>,
    notices: Vec<String>,
    folder: PathBuf,
}

#[derive(Debug)]
struct Reload {
    id: DocumentId,
    path: PathBuf,
    mark: TabMark,
    stamp: Option<library::DiskStamp>,
    loaded: Option<crate::file::loader::LoadedFile>,
}

/// The one copy worker a window has, started by its first copy. Its queue is a channel: a second
/// drop waits behind the first. Dropping it (the window closing) stops the worker after the file
/// in hand.
#[derive(Default)]
pub(crate) struct CopyWorker {
    sender: Option<Sender<CopyJob>>,
    cancel: Arc<AtomicBool>,
    #[cfg(test)]
    pending: Arc<std::sync::atomic::AtomicUsize>,
}

impl std::fmt::Debug for CopyWorker {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("CopyWorker").finish_non_exhaustive()
    }
}

impl Drop for CopyWorker {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

fn run_worker(jobs: Receiver<CopyJob>, cancel: Arc<AtomicBool>) {
    while let Ok(job) = jobs.recv() {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        let items = job
            .items
            .into_iter()
            .map(|(source, destination)| ItemDone {
                is_folder: source.is_dir(),
                copied: crate::platform::files::copy_tree(&source, &destination, &cancel),
                destination,
            })
            .collect();
        let reloads = job
            .reloads
            .into_iter()
            .map(|(id, path, mark)| {
                let stamp = library::disk_stamp(&path);
                let loaded = crate::file::loader::load(&path)
                    .ok()
                    .filter(|loaded| std::ffi::CString::new(loaded.text.as_str()).is_ok());
                Reload { id, path, mark, stamp, loaded }
            })
            .collect();
        let done = Box::into_raw(Box::new(CopyDone {
            items,
            reloads,
            notices: job.notices,
            folder: job.folder,
        }));
        let posted = unsafe {
            PostMessageW(job.target as HWND, crate::window::WM_FASTPAD_COPY_DONE, 0, done as isize)
        } != 0;
        if !posted {
            drop(unsafe { Box::from_raw(done) });
        }
    }
}

#[cfg(test)]
thread_local! {
    static FAIL_RECYCLE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Makes the next Recycle Bin step fail, as a network share would.
#[cfg(test)]
pub(crate) fn fail_next_recycle() {
    FAIL_RECYCLE.with(|fail| fail.set(true));
}

fn recycle(hwnd: HWND, path: &Path) -> bool {
    #[cfg(test)]
    if FAIL_RECYCLE.with(|fail| fail.replace(false)) {
        return false;
    }
    let result = if path.is_dir() {
        crate::platform::files::recycle_folder(hwnd, path)
    } else {
        crate::platform::files::recycle(hwnd, path)
    };
    result.is_ok()
}

/// Copies `sources` (absolute) into `folder` (relative to the notebook; empty is the root):
/// plans, asks about each clash, recycles each answered replace, and queues the copies.
pub(crate) fn copy_into(hwnd: HWND, sources: Vec<PathBuf>, folder: &Path, dirty_tab: Option<String>) {
    let Some(root) = library_host::folder(hwnd) else {
        return;
    };
    let planned = tree_copy::plan(&sources, &root, folder, &|path: &Path| path.exists());
    let folder_label = match folder.file_name() {
        Some(name) => name.to_string_lossy().into_owned(),
        None => library_host::notebook_name(&root),
    };
    let mut notices = Vec::new();
    let mut items = Vec::new();
    let mut replaced = Vec::new();
    for Planned { source, destination, outcome } in planned {
        let name = tree_copy::item_name(&source);
        match outcome {
            Outcome::Refused(refusal) => notices.push(tree_copy::refused_notice(&name, refusal)),
            Outcome::Clash => {
                if !library_host::confirmed(hwnd, &tree_copy::replace_question(&name, &folder_label)) {
                    continue;
                }
                if !recycle(hwnd, &destination) {
                    notices.push(tree_copy::recycle_failed_notice(&name));
                    continue;
                }
                replaced.push(destination.clone());
                items.push((source, destination));
            }
            Outcome::Copy => items.push((source, destination)),
        }
    }
    if let Some(name) = dirty_tab.filter(|_| !items.is_empty()) {
        notices.push(tree_copy::dirty_notice(&name));
    }
    if items.is_empty() {
        for notice in notices {
            push_notice(hwnd, notice);
        }
        return;
    }
    let reloads = clean_tabs_on(hwnd, &replaced);
    queue(hwnd, CopyJob { target: hwnd as isize, items, reloads, notices, folder: folder.to_path_buf() });
}

/// An Open Editors row dropped on `folder`: its file on disk is copied. A dirty tab's saved
/// version goes, and the notice says so.
pub(crate) fn copy_tab_into(hwnd: HWND, id: DocumentId, path: &Path, folder: &Path) {
    let dirty = unsafe { app_ptr(hwnd) }
        .and_then(|app| unsafe { app.as_ref() }.tabs.document(id).map(|document| document.dirty))
        .unwrap_or(false);
    let dirty_tab = dirty.then(|| tree_copy::item_name(path));
    copy_into(hwnd, vec![path.to_path_buf()], folder, dirty_tab);
}

/// Clean tabs open on `replaced` files or on files under `replaced` folders.
fn clean_tabs_on(hwnd: HWND, replaced: &[PathBuf]) -> Vec<(DocumentId, PathBuf, TabMark)> {
    unsafe { app_ptr(hwnd) }
        .map(|app| {
            unsafe { app.as_ref() }
                .tabs
                .documents()
                .filter(|document| !document.dirty)
                .filter_map(|document| {
                    let path = document.path.as_ref()?;
                    replaced.iter().any(|gone| library::at_or_under(path, gone)).then(|| {
                        let mark = TabMark {
                            generation: document.generation,
                            disk_stamp: document.disk_stamp,
                        };
                        (document.id, path.clone(), mark)
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn queue(hwnd: HWND, job: CopyJob) {
    #[cfg(test)]
    let pending = library_host::with_copy_worker(hwnd, |worker| Arc::clone(&worker.pending));
    let sender = library_host::with_copy_worker(hwnd, |worker| {
        worker
            .sender
            .get_or_insert_with(|| {
                let (sender, receiver) = channel();
                let cancel = Arc::clone(&worker.cancel);
                std::thread::spawn(move || run_worker(receiver, cancel));
                sender
            })
            .clone()
    });
    #[cfg(test)]
    if let Some(pending) = &pending {
        pending.fetch_add(1, Ordering::SeqCst);
    }
    if let Some(sender) = sender {
        let _ = sender.send(job);
    }
}

/// `WM_FASTPAD_COPY_DONE`: frees the result and applies it.
pub(crate) fn copy_done(hwnd: HWND, lparam: LPARAM) {
    if lparam == 0 {
        return;
    }
    let done = *unsafe { Box::from_raw(lparam as *mut CopyDone) };
    #[cfg(test)]
    if let Some(pending) = library_host::with_copy_worker(hwnd, |worker| Arc::clone(&worker.pending)) {
        pending.fetch_sub(1, Ordering::SeqCst);
    }
    apply(hwnd, done);
}

fn apply(hwnd: HWND, done: CopyDone) {
    let Some(root) = library_host::folder(hwnd) else {
        return;
    };
    let mut notices = done.notices;
    let mut hidden = Vec::new();
    let mut rescan = false;
    let mut listed = Vec::new();
    for item in &done.items {
        let name = tree_copy::item_name(&item.destination);
        if let Some((_, error)) = &item.copied.error {
            let before = item.is_folder.then_some(item.copied.files);
            notices.push(tree_copy::failed_notice(&name, &error.to_string(), before));
            rescan = true;
            continue;
        }
        if item.is_folder {
            rescan = true;
            listed.push(super::super::library::tree::RowKind::Folder(library::record_path(&root, &item.destination)));
            continue;
        }
        let is_note = item
            .destination
            .extension()
            .is_some_and(|extension| library::title::is_note_extension(&extension.to_string_lossy()));
        if is_note {
            library_host::with_state(hwnd, |state| state.add_note(&item.destination));
            listed.push(super::super::library::tree::RowKind::Note(library::record_path(&root, &item.destination)));
        } else {
            hidden.push(name);
        }
    }
    if !hidden.is_empty() {
        notices.push(tree_copy::hidden_notice(&hidden));
    }
    for ancestor in library::tree::ancestors(&done.folder.join("x")) {
        library_host::set_expanded(hwnd, &ancestor, true);
    }
    library_host::schedule_write(hwnd);
    if rescan {
        library_host::request_rescan(hwnd);
    }
    let single = (done.items.len() == 1).then(|| listed.pop()).flatten();
    super::side_panel::with_accessible_events(hwnd, || {
        super::side_panel::refresh(hwnd);
        if let Some(row) = &single {
            super::notebook_view::select_row(hwnd, row);
        }
    });
    // A reload swaps documents: inside a modal loop or a file population the tab is left as it
    // is, and its disk stamp pauses its autosave as for any change made outside FastPad.
    let busy = super::main_window::file_population_active(hwnd) || super::modal::modal_active(hwnd);
    if !busy {
        for reload in done.reloads {
            if let Some(loaded) = reload.loaded {
                super::main_window::reload_clean_document(hwnd, reload.id, &reload.path, reload.mark, &loaded, reload.stamp);
            }
        }
    }
    for notice in notices {
        push_notice(hwnd, notice);
    }
}

/// Pumps the window's messages until every queued copy has come back and been applied.
#[cfg(test)]
pub(crate) fn wait_for_copies(hwnd: HWND) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let pending = library_host::with_copy_worker(hwnd, |worker| worker.pending.load(Ordering::SeqCst)).unwrap_or(0);
        if pending == 0 {
            return;
        }
        assert!(std::time::Instant::now() < deadline, "the copy never came back");
        super::main_window::pump_posted_messages_for_test(hwnd);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}
```

The code above is close to final. Tidy these while implementing:
- `super::super::library::tree::RowKind` is `crate::library::tree::RowKind`: import it.
- `library::tree::ancestors(&folder.join("x"))` lists `folder` and its parents. If `ancestors` already includes the path itself, pass `folder` directly. Check its doc comment in `tree.rs`.
- The `library_copy` module alias is noise. Use `crate::platform::files::Copied` directly.
- `pump_posted_messages_for_test` is whatever `main_window`'s tests use to pump (`pump_posted_messages`). Expose it `pub(crate)` under `#[cfg(test)]`.

`library_host.rs`:
- Add `pub(crate) copy_worker: super::copy_host::CopyWorker,` to `LibraryHost`, and to whatever builds it (`Default` or `new`).
- Add:

```rust
/// Runs `f` on the window's copy worker.
pub(crate) fn with_copy_worker<R>(
    hwnd: HWND,
    f: impl FnOnce(&mut super::copy_host::CopyWorker) -> R,
) -> Option<R> {
    host(hwnd, |host| f(&mut host.copy_worker))
}
```

- `confirmed` becomes `pub(crate)`.

`messages.rs`: add `pub const WM_FASTPAD_COPY_DONE: u32 = WM_APP + 18;` with the comment "Not part of the deferred chain: the copy worker's result for one drop, as a `Box` the receiver frees. A post that fails because the window is gone is freed on the worker." Add it to the `mod.rs` re-export, and to the messages test's list of non-deferred messages (the `for message in [ … ]` at `messages.rs:209`).

`main_window.rs`, next to `:750`:

```rust
            if message == crate::window::WM_FASTPAD_COPY_DONE {
                crate::window::copy_host::copy_done(hwnd, lparam);
                return 0;
            }
```

- [ ] **Step 4: Run the tests and check they pass.** Run `cargo test --lib copy_host_ tree_copy messages -- --test-threads=1`. Expected: all pass.
- [ ] **Step 5: Commit** with the message `"feat(notebook): copy into the tree on a worker, asking before replacing"`.

---

### Task 9: Dragging an Open Editors row into the tree

**Files:**
- Modify: `src/window/tree_drag.rs` (`DragSource`, `Drag.source`, `Drag::armed`, `Drag::hover`, `accepts`, tests)
- Modify: `src/window/notebook_view.rs`:
  - `arm_drag`, and arming from `Hit::Editor`;
  - `drag_release`, `drag_label_image`, `is_dragged_row` and `rebuild`'s lost-source check;
  - `drag_to`, which passes the root.
- Modify: `src/window/tree_move.rs` (`drop_into` takes `&RowKind` as before; the caller unwraps `DragSource::Row`)
- Modify: `src/window/main_window.rs` tests (`.source` comparisons become `DragSource::Row(…)`)

**Interfaces:**
- Produces:
  - `pub(crate) enum DragSource { Row(RowKind), Tab { id: DocumentId, path: PathBuf }, Files(Vec<PathBuf>) }`, which is `Clone, Debug, Eq, PartialEq`.
    - A `Row` moves, as before. A `Tab` and `Files` copy (spec §4.3).
    - `Files` is used by Task 10.
  - `Drag::armed(source: DragSource, x: i32, y: i32) -> Option<Drag>`: a `Row` must be `draggable`, and a `Tab` or `Files` source always arms.
  - `Drag::hover(&mut self, rows: &[TreeRow], root: &Path, point: (i32, i32), hover: Hover, now: Instant) -> bool`
  - `pub(crate) fn source_accepts(source: &DragSource, root: &Path, folder: &Path) -> bool`: a `Row` uses `accepts`, a `Tab` uses `tree_copy::any_accepted(&[path], root, folder)`, and `Files` uses `any_accepted(paths, …)`.
  - `pub(crate) fn copies(source: &DragSource) -> bool`

- [ ] **Step 1: Write the failing tests.** In `tree_drag.rs` tests:

```rust
    #[test]
    fn a_tab_drag_takes_any_folder_but_its_own_files_folder() {
        // Break caught (Review Focus 1): a tab inside the notebook offered its own folder, so a
        // "copy" would ask to replace the file with itself.
        let root = Path::new(r"C:\notes");
        let inside = DragSource::Tab { id: DocumentId(1), path: PathBuf::from(r"C:\notes\work\b.md") };
        assert!(!source_accepts(&inside, root, Path::new("work")));
        assert!(source_accepts(&inside, root, Path::new("")));
        let outside = DragSource::Tab { id: DocumentId(2), path: PathBuf::from(r"D:\x\draft.txt") };
        assert!(source_accepts(&outside, root, Path::new("work")));
        assert!(copies(&outside) && !copies(&DragSource::Row(note("a.md"))));
        let mut drag = Drag::armed(outside, 1, 2).unwrap();
        assert!(drag.hover(&rows(), root, (5, 5), Hover::Row(0), Instant::now()));
        assert_eq!(drag.target, Some(PathBuf::from("work")));
    }
```

(`rows()` after Task 4 starts with the `work` folder at index 0.)

In the `main_window.rs` tests:

```rust
    /// Presses on Open Editors row `index` and moves past the drag distance.
    fn start_tab_drag(hwnd: HWND, panel: HWND, index: usize) {
        use windows_sys::Win32::UI::WindowsAndMessaging::{WM_LBUTTONDOWN, WM_MOUSEMOVE};
        let rect = notebook_view(hwnd).editor_rect_at(index).unwrap();
        let (x, y) = ((rect.left + rect.right) / 3, (rect.top + rect.bottom) / 2);
        mouse(panel, WM_LBUTTONDOWN, 1, client_lparam(x, y));
        mouse(panel, WM_MOUSEMOVE, 1, client_lparam(x, y + 40));
    }

    #[test]
    fn open_editors_drag_onto_a_folder_copies_the_file_and_leaves_the_tab_on_it() {
        // Break caught: the drop moving the file, the tab following the copy, or the copied row
        // not selected (open editors spec §4.1, §4.5).
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("editors-drag");
        std::fs::create_dir_all(scratch.folder().join("work")).unwrap();
        scratch.note(r"work\b.md", "b");
        let outside = scratch.root.join("draft.txt");
        std::fs::write(&outside, "draft").unwrap();
        let (window, _editor) = notebook_window(&scratch);
        super::open_path(window.hwnd, &outside).unwrap();
        let panel = sidebar_windows(window.hwnd).1;
        start_tab_drag(window.hwnd, panel, 0);
        let work = row_lparam(window.hwnd, &RowKind::Folder("work".into()));
        drag_over(panel, work);
        drop_at(panel, work);
        crate::window::copy_host::wait_for_copies(window.hwnd);
        assert_eq!(std::fs::read_to_string(scratch.folder().join(r"work\draft.txt")).unwrap(), "draft");
        assert!(outside.exists());
        assert_eq!(app_mut(window.hwnd).tabs.active().unwrap().path.as_deref(), Some(outside.as_path()));
        assert_eq!(
            selected_kind(window.hwnd),
            Some(RowKind::Note(r"work\draft.txt".into())),
            "a .txt is a note type, listed and selected"
        );
    }

    #[test]
    fn open_editors_drag_onto_its_own_folder_copies_nothing_and_asks_nothing() {
        // Break caught (Review Focus 1).
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("editors-drag-self");
        std::fs::create_dir_all(scratch.folder().join("work")).unwrap();
        let b = scratch.note(r"work\b.md", "b");
        let (window, _editor) = notebook_window(&scratch);
        super::open_path(window.hwnd, &b).unwrap();
        let panel = sidebar_windows(window.hwnd).1;
        start_tab_drag(window.hwnd, panel, 0);
        let row = row_lparam(window.hwnd, &RowKind::Note(r"work\b.md".into()));
        drag_over(panel, row);
        assert_eq!(notebook_view(window.hwnd).drag.as_ref().unwrap().target, None);
        drop_at(panel, row);
        assert!(crate::window::modal::take_last_confirm().is_none());
        assert_eq!(std::fs::read_to_string(&b).unwrap(), "b");
    }

    #[test]
    fn open_editors_an_untitled_row_does_not_start_a_drag() {
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("editors-drag-untitled");
        let (window, _editor) = notebook_window(&scratch);
        let panel = sidebar_windows(window.hwnd).1;
        start_tab_drag(window.hwnd, panel, 0);
        assert!(notebook_view(window.hwnd).drag.is_none());
    }

    #[test]
    fn open_editors_drag_survives_its_tab_closing_mid_drag() {
        // Break caught (Review Focus 5): a stale DocumentId panicking the drop, or the drop
        // copying nothing though the pressed file is still on disk.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("editors-drag-closed");
        std::fs::create_dir_all(scratch.folder().join("work")).unwrap();
        scratch.note(r"work\b.md", "b");
        let outside = scratch.root.join("gone.md");
        std::fs::write(&outside, "g").unwrap();
        let (window, _editor) = notebook_window(&scratch);
        super::open_path(window.hwnd, &outside).unwrap();
        let panel = sidebar_windows(window.hwnd).1;
        start_tab_drag(window.hwnd, panel, 0);
        execute_command(window.hwnd, CommandId::CloseTab);
        let work = row_lparam(window.hwnd, &RowKind::Folder("work".into()));
        drag_over(panel, work);
        drop_at(panel, work);
        crate::window::copy_host::wait_for_copies(window.hwnd);
        assert!(scratch.folder().join(r"work\gone.md").exists());
    }
```

`.txt` is one of the 14 note types (`library::title::NOTE_EXTENSIONS`), so the copy is listed and selected.

A tab drag moves `y + 40`, not `x + 30`. That keeps the pointer inside the Open Editors list, so the test also proves that a drag over Open Editors has no target. It then moves to the folder row.

- [ ] **Step 2: Run the tests and check they fail.** Run `cargo test --lib tree_drag open_editors_drag open_editors_an_untitled -- --test-threads=1`.

- [ ] **Step 3: Implement.**
   1. `tree_drag.rs`: add the `DragSource` enum, `source_accepts` and `copies`. Change `Drag.source` to `DragSource`.
      - `armed` checks `matches!(&source, DragSource::Row(kind) if draggable(kind)) || !matches!(source, DragSource::Row(_))`.
      - `hover` takes `root` and filters with `source_accepts(&self.source, root, folder)`.
      - Update the module doc comment to mention copies.
      - Existing tests: wrap every `RowKind` source in `DragSource::Row(…)`, and pass `Path::new(r"C:\notes")` as `root`.
   2. `notebook_view.rs`:
      - `arm_drag(hwnd, source: DragSource, x, y)`.
      - In `left_down`, `Hit::Row` arms `DragSource::Row(source)`.
      - In `left_down`'s new `Hit::Editor { index, close: false }` arm, after activating the tab, arm `DragSource::Tab { id: row.id, path }` when `row.path` is `Some`. The path is captured now, so it survives the tab closing (Review Focus 5).
      - `drag_to` passes `&self.root.clone().unwrap_or_default()` as `root`.
      - `drag_label_image`:
        - for a `Row`, as now;
        - for a `Tab`, the name is `tree_copy::item_name(path)` and the icon is `open_editors::tree_item` of a row with that path. Build the `TreeItem` directly: `TreeItem::Note(note_kind(extension))`;
        - for `Files`, `None`, since there's no label.
      - `is_dragged_row` takes `Option<&DragSource>` and matches only `DragSource::Row(kind)`.
      - `rebuild`'s lost-source check applies only to `DragSource::Row`. A tab or files source is never lost by a rebuild.
      - `drag_release`:

```rust
    if let Some(folder) = drag.target {
        match &drag.source {
            DragSource::Row(kind) => super::tree_move::drop_into(hwnd, kind, &folder),
            DragSource::Tab { id, path } => super::copy_host::copy_tab_into(hwnd, *id, path, &folder),
            DragSource::Files(_) => {}
        }
    }
```

   3. `main_window.rs` tests: the existing tree drag tests compare `.source` to `RowKind::…`. Wrap those in `DragSource::Row(…)`.

- [ ] **Step 4: Run the tests.** Run `cargo test --lib tree_drag open_editors_ -- --test-threads=1`. Expected: all pass. Also run all `tree_drag_` window tests with the same command filter.
- [ ] **Step 5: Commit** with the message `"feat(notebook): drag an Open Editors row onto the tree to copy its file there"`.

---

### Task 10: Explorer drops on the panel

**Files:**
- Create: `src/platform/ole_drop.rs` (+ `platform/mod.rs`)
- Create: `src/window/panel_drop.rs` (+ `window/mod.rs`)
- Modify: `src/editor/file_drop.rs`:
  - use `platform::ole_drop` for `DropTargetVtbl`, `DataObjectVtbl`, the IIDs, `hdrop_format`, `offers_files` and `dropped_files`;
  - `test_support` gains `drag_and_drop_at(hwnd, paths, screen: POINTL) -> [u32; 3]`, and `drag_and_drop` calls it with `(1, 1)`.
- Modify: `src/window/messages.rs` (`WM_FASTPAD_PANEL_DROPPED = WM_APP + 19`), `src/window/mod.rs`
- Modify: `src/window/notebook_view.rs` (`external_over`, `external_leave`, `external_drop`)
- Modify: `src/window/side_panel.rs`:
  - `accept_file_drops(hwnd)`;
  - `destroy_windows` revokes the target;
  - `notes_mode_changed` registers it.
- Modify: `src/window/library_host.rs` (`accept_editor_file_drops` also calls `side_panel::accept_file_drops`)
- Modify: `src/window/main_window.rs` (dispatch `WM_FASTPAD_PANEL_DROPPED`; the tests)

**Interfaces:**
- Produces:
  - `platform::ole_drop`, containing:
    - `pub(crate) type Unknown = *mut c_void`;
    - `pub(crate) struct DropTargetVtbl` (same fields as today) and `pub(crate) struct DataObjectVtbl`;
    - `pub(crate) const IID_IUNKNOWN`, `IID_IDROPTARGET` and `pub(crate) fn is_drop_target_iid(iid: &GUID) -> bool`;
    - `pub(crate) fn offers_files(data: Unknown) -> bool` and `pub(crate) fn dropped_files(data: Unknown) -> Vec<PathBuf>`;
    - `pub(crate) fn ensure_ole()`: `OleInitialize(null)` once per thread, in a `thread_local!` `Cell<bool>`, and never uninitialised, since the UI thread lives as long as the process.
  - `panel_drop::register(main: HWND, panel: HWND) -> crate::Result<()>`, which is idempotent, and `panel_drop::revoke(panel: HWND)`
  - `notebook_view::external_over(main: HWND, x: i32, y: i32, paths: &[PathBuf]) -> bool`, which is true to answer COPY. It keeps a `DragSource::Files` drag that is `started`, with no capture and no label, so the band, auto-expand and auto-scroll are the tree drag's.
  - `notebook_view::external_leave(main: HWND)`
  - `notebook_view::external_drop(main: HWND, x: i32, y: i32, paths: Vec<PathBuf>) -> bool`, which posts `PanelDrop` and returns whether it's accepted
  - `copy_host::panel_dropped(hwnd: HWND, lparam: LPARAM)`, which is `WM_FASTPAD_PANEL_DROPPED`'s handler: it opens, or copies into the folder
  - `side_panel::accept_file_drops(hwnd: HWND)`

- [ ] **Step 1: Write the failing window tests.**

```rust
    /// A drag from Explorer onto panel point `x`, `y`: DragEnter, DragOver, then Drop or
    /// DragLeave, as OLE runs them. The effects each answered.
    fn explorer_drop(panel: HWND, x: i32, y: i32, paths: &[&std::path::Path]) -> [u32; 3] {
        let mut point = windows_sys::Win32::Foundation::POINT { x, y };
        unsafe { windows_sys::Win32::Graphics::Gdi::ClientToScreen(panel, &mut point) };
        crate::editor::file_drop::test_support::drag_and_drop_at(
            panel,
            paths,
            windows_sys::Win32::Foundation::POINTL { x: point.x, y: point.y },
        )
    }

    #[test]
    fn panel_drop_onto_the_root_row_copies_and_onto_open_editors_opens() {
        // Break caught: Explorer drops refused on the panel, dropped on the wrong folder, or
        // Open Editors copying instead of opening (open editors spec §4.1, §4.3).
        use windows_sys::Win32::System::Ole::{DROPEFFECT_COPY, DROPEFFECT_NONE};
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("panel-drop");
        std::fs::create_dir_all(scratch.folder().join("work")).unwrap();
        let outside = scratch.root.join("x.md");
        std::fs::write(&outside, "x").unwrap();
        let (window, _editor) = notebook_window(&scratch);
        crate::window::side_panel::accept_file_drops(window.hwnd);
        let panel = sidebar_windows(window.hwnd).1;
        let root = notebook_view(window.hwnd).root_rect();
        let effects = explorer_drop(panel, root.left + 40, (root.top + root.bottom) / 2, &[&outside]);
        assert_eq!(effects, [DROPEFFECT_COPY; 3]);
        pump_until(window.hwnd, || scratch.folder().join("x.md").exists());
        crate::window::copy_host::wait_for_copies(window.hwnd);

        let header = notebook_view(window.hwnd).editors_header_rect();
        explorer_drop(panel, header.left + 40, header.top + 5, &[&outside]);
        pump_until(window.hwnd, || tab_paths(window.hwnd).contains(&Some(outside.clone())));

        let title = 10;
        assert_eq!(explorer_drop(panel, 40, title, &[&outside])[1], DROPEFFECT_NONE, "the title band takes nothing");
    }

    #[test]
    fn panel_drop_returns_before_asking_and_the_posted_drop_asks() {
        // Break caught (Review Focus 4): the clash prompt shown inside Drop, which keeps
        // Explorer's drag waiting on FastPad.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("panel-drop-post");
        scratch.note("x.md", "old");
        let outside = scratch.root.join("x.md");
        std::fs::write(&outside, "new").unwrap();
        let (window, _editor) = notebook_window(&scratch);
        crate::window::side_panel::accept_file_drops(window.hwnd);
        let panel = sidebar_windows(window.hwnd).1;
        let root = notebook_view(window.hwnd).root_rect();
        crate::window::modal::answer_next_confirm(true);
        explorer_drop(panel, root.left + 40, (root.top + root.bottom) / 2, &[&outside]);
        assert!(crate::window::modal::take_last_confirm().is_none(), "nothing asked during Drop");
        crate::window::modal::answer_next_confirm(true);
        pump_until(window.hwnd, || crate::window::modal::take_last_confirm().is_some());
        crate::window::copy_host::wait_for_copies(window.hwnd);
        assert_eq!(std::fs::read_to_string(scratch.folder().join("x.md")).unwrap(), "new");
    }
```

If `answer_next_confirm` is a queue, the first `answer_next_confirm(true)` in the second test is left over. Drop that line if so: read `modal.rs:179-215` first.

- [ ] **Step 2: Run the tests and check they fail.** Run `cargo test --lib panel_drop -- --test-threads=1`.

- [ ] **Step 3: Implement.**
   1. **`platform::ole_drop`:**
      - Move these from `editor/file_drop.rs`, verbatim, making them `pub(crate)`: `DropTargetVtbl`, `DataObjectVtbl`, `IID_IUNKNOWN`, `IID_IDROPTARGET`, `hdrop_format`, `offers_files` and `dropped_files`.
      - Add `is_drop_target_iid`, which compares like `file_drop::query_interface`'s `known`, and `ensure_ole`.
      - `file_drop.rs` imports them. Its behaviour is unchanged, and its existing tests must still pass.
   2. **`test_support::drag_and_drop_at`:** the same body as `drag_and_drop`, with the point passed in. `drag_and_drop(hwnd, paths)` becomes `drag_and_drop_at(hwnd, paths, POINTL { x: 1, y: 1 })`.
   3. **`window::panel_drop`:** a `#[repr(C)] struct PanelTarget { vtbl: &'static DropTargetVtbl, refs: Cell<u32>, main: HWND, panel: HWND, files: RefCell<Vec<PathBuf>> }`, with its own `query_interface`, `add_ref` and `release` (as in `file_drop`, without an inner target):
      - `drag_enter`: if `offers_files(data)`, store `dropped_files(data)` in `files`, then answer as `drag_over` does. Otherwise answer `DROPEFFECT_NONE`.
      - `drag_over`: convert `point` with `ScreenToClient(panel)`. Answer `DROPEFFECT_COPY` if `notebook_view::external_over(main, x, y, &files)` and the source allows copy (`*effect & DROPEFFECT_COPY`); otherwise `DROPEFFECT_NONE`.
      - `drag_leave`: clear `files` and call `notebook_view::external_leave(main)`.
      - `drop`: take `files`, convert the point, and answer COPY or NONE from `notebook_view::external_drop(main, x, y, files)`.
      - `register`:

```rust
pub(crate) fn register(main: HWND, panel: HWND) -> crate::Result<()> {
    if !crate::editor::file_drop::registered_target(panel).is_null() {
        return Ok(());
    }
    crate::platform::ole_drop::ensure_ole();
    let target = Box::into_raw(Box::new(PanelTarget {
        vtbl: &PANEL_VTBL,
        refs: Cell::new(1),
        main,
        panel,
        files: RefCell::new(Vec::new()),
    }))
    .cast::<c_void>();
    let result = unsafe { RegisterDragDrop(panel, target) };
    unsafe { release(target) };
    if result < 0 {
        return Err(crate::FastPadError::Win32(result as u32));
    }
    Ok(())
}

pub(crate) fn revoke(panel: HWND) {
    if !crate::editor::file_drop::registered_target(panel).is_null() {
        unsafe { RevokeDragDrop(panel) };
    }
}
```

   4. **`notebook_view`:**

```rust
/// What an Explorer drag at panel point `x`, `y` does (open editors spec §4.1, §4.3): over Open
/// Editors, or with no notebook, it opens (COPY, no highlight); over the tree, the root row or
/// the body, it copies into the folder under it when that folder takes one of `paths` (the
/// band shows); elsewhere nothing. The drag is kept as a started `DragSource::Files` drag with
/// no capture and no label, so the tree drag's band, auto-expand and auto-scroll apply.
pub(crate) fn external_over(hwnd: HWND, x: i32, y: i32, paths: &[PathBuf]) -> bool {
    let opens = with_view(hwnd, |view| view.opens_at(x, y)).unwrap_or(false);
    if opens {
        external_leave(hwnd);
        return true;
    }
    let started = with_view(hwnd, |view| {
        if !matches!(view.drag.as_ref().map(|drag| &drag.source), Some(DragSource::Files(_))) {
            view.drag = Drag::armed(DragSource::Files(paths.to_vec()), x, y).map(|mut drag| {
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

/// The Explorer drag left the panel or was cancelled: its band and timer go.
pub(crate) fn external_leave(hwnd: HWND) {
    let panel = with_view(hwnd, |view| {
        let external = matches!(view.drag.as_ref().map(|drag| &drag.source), Some(DragSource::Files(_)));
        if external {
            view.drag = None;
            view.invalidate();
        }
        external.then_some(view.panel)
    })
    .flatten();
    if let Some(panel) = panel {
        end_drag_timer(panel);
    }
}

/// An Explorer drop at panel point `x`, `y`: posts what to do and returns at once, so Explorer
/// never waits on a prompt (spec §6). False when nothing here takes it.
pub(crate) fn external_drop(hwnd: HWND, x: i32, y: i32, paths: Vec<PathBuf>) -> bool {
    let opens = with_view(hwnd, |view| view.opens_at(x, y)).unwrap_or(false);
    let folder = if opens {
        None
    } else {
        with_view(hwnd, |view| view.drag_to(x, y, Instant::now()))
            .filter(|&accepted| accepted)
            .and_then(|_| with_view(hwnd, |view| view.drag.as_ref().and_then(|drag| drag.target.clone())).flatten())
    };
    external_leave(hwnd);
    if !opens && folder.is_none() {
        return false;
    }
    super::copy_host::post_panel_drop(hwnd, paths, folder)
}
```

      Also add `fn opens_at(&self, x, y) -> bool`: true when `self.root.is_none()` and `(x, y)` is under the title band, or when `(x, y)` is inside `editors_header` or `editors_list`.
      
      Guard `set_drag_cursor` in `retarget_drag` and `drag_tick` with `!matches!(source, DragSource::Files(_))`: OLE owns the cursor during an Explorer drag.
   5. **`copy_host`:**

```rust
/// `WM_FASTPAD_PANEL_DROPPED`'s payload: what an Explorer drop on the panel does.
pub(crate) struct PanelDrop {
    paths: Vec<PathBuf>,
    /// `None` opens the paths, as a drop on the window does.
    folder: Option<PathBuf>,
}

pub(crate) fn post_panel_drop(hwnd: HWND, paths: Vec<PathBuf>, folder: Option<PathBuf>) -> bool {
    let payload = Box::into_raw(Box::new(PanelDrop { paths, folder }));
    let posted = unsafe {
        PostMessageW(hwnd, crate::window::WM_FASTPAD_PANEL_DROPPED, 0, payload as isize)
    } != 0;
    if !posted {
        drop(unsafe { Box::from_raw(payload) });
    }
    posted
}

/// `WM_FASTPAD_PANEL_DROPPED`: opens or copies. Ignored while a modal dialog runs, as
/// `WM_DROPFILES` is for a disabled window.
pub(crate) fn panel_dropped(hwnd: HWND, lparam: LPARAM) {
    if lparam == 0 {
        return;
    }
    let drop = *unsafe { Box::from_raw(lparam as *mut PanelDrop) };
    if super::modal::modal_active(hwnd) {
        return;
    }
    match drop.folder {
        Some(folder) => copy_into(hwnd, drop.paths, &folder, None),
        None => library_host::files_dropped(hwnd, drop.paths),
    }
}
```

   6. **Registration:**
      - `side_panel::accept_file_drops(hwnd)`: `if let Some((_, panel)) = windows(hwnd) { if let Err(error) = panel_drop::register(hwnd, panel) { push_notice(hwnd, format!("FastPad could not accept files dropped on the sidebar: {error}")) } }`.
      - `library_host::accept_editor_file_drops` calls it last.
      - `side_panel::notes_mode_changed` calls it after `sync_presence` when `enabled`.
      - `create_for_first_frame` does **not** call it (latency: first paint).
      - `destroy_windows` calls `panel_drop::revoke(sidebar.panel)` before `DestroyWindow(sidebar.panel)`.
   7. **Messages:**
      - Add `WM_FASTPAD_PANEL_DROPPED = WM_APP + 19` in `messages.rs`, with its comment, the re-export, and the non-deferred list.
      - Dispatch it in `main_window.rs` next to `WM_FASTPAD_COPY_DONE`: `crate::window::copy_host::panel_dropped(hwnd, lparam)`.

- [ ] **Step 4: Run the tests.** Run `cargo test --lib panel_drop file_drop messages -- --test-threads=1`. Expected: all pass, including `file_drop`'s two existing tests.
- [ ] **Step 5: Commit** with the message `"feat(notebook): drop files from Explorer onto the tree to copy them in"`.

---

### Task 11: Docs

**Files:**
- Modify: `README.md` (the Notebook section)
- Modify: `docs/superpowers/specs/2026-09-25-tree-drag-move-design.md` §9
- Modify: `docs/superpowers/specs/2026-09-25-open-editors-design.md` (add §10)

- [ ] **Step 1: README.** In the Notebook section, after the paragraph on dragging in the tree, add:

```markdown
**Open Editors.** The top of the Notebook view lists every open tab, the notebook's own notes and any other file alike. Click a row to switch to it; its ✕ or a middle-click closes it. The notebook itself is the collapsible row below.

**Adding a file to the notebook.** Drag its row from Open Editors onto a folder in the tree, or drag files and folders from Explorer onto the tree. They are copied there; the original stays where it is. If the name is taken, FastPad asks before replacing it, and the replaced item goes to the Recycle Bin.
```

- [ ] **Step 2: The tree drag spec §9.** Replace the first bullet ("Dropping files or folders from Explorer into a tree folder…") with: "Dropping files or folders from Explorer into a tree folder: now in the Open Editors spec (`2026-09-25-open-editors-design.md`), where it copies."

- [ ] **Step 3: The Open Editors spec.** Add a `## 10. Decisions made while implementing` section. It lists what the build decided that the spec left open, one line each:
  - **The title band stays.** It holds "NOTEBOOK" and is all caption, because it sits in the window's title strip.
  - **Replace recycles on the UI thread.** The Recycle Bin step runs right after its prompt, as Delete does.
  - **A new refusal:** "it would replace the folder it is in."
  - **The Open Editors list can't be dragged by its scroll thumb.** Wheel and keyboard only.
  - Add any other decision the review turned up.

- [ ] **Step 4: Commit** with the message `"docs: Open Editors and copying files into the notebook"`.

---

## Final review

- Run the full suite once: `cargo test -- --test-threads=1` (with a separate `CARGO_TARGET_DIR` if FastPad is running).
- Manual checks before merge:
  - Narrator on the new sections;
  - high contrast;
  - 150–200% DPI;
  - a real Explorer drag (single file, a folder, several items, a clash) onto a folder, the root row, and Open Editors;
  - an Open Editors drag of a dirty tab.
