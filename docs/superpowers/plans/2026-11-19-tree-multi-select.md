# Notebook Tree Multi-Select Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Select several notes and folders of one notebook in the Notebook tree and move, copy or delete them together.

**Architecture:** A pure `TreeSelection` (keyed by `RowKind`, one notebook) lives on `NotebookView` beside the existing cursor (`list.selected`). An empty selection means "the cursor row alone", so every one-row path is untouched. Group operations loop over the existing one-item code (split into a no-prompt "perform" part returning its failure text), with one confirm up front and one summary notice after. Cross-notebook group moves hand the whole list to the existing all-or-nothing `cross_move_host::move_items`.

**Tech Stack:** Rust, windows-sys Win32, existing FastPad test harness (`ProductionWindow`, `LibraryScratch`, `modal::answer_next_confirm`, `mouse`/`centre`).

**Spec:** `docs/superpowers/specs/2026-11-19-tree-multi-select-design.md`

## Global Constraints

- One notebook per selection; Root, Draft and Placeholder rows are never in a multi-selection.
- One-row click, drag, Del, F2, Enter and context menu behave exactly as today; no existing test may change.
- A tree rebuild never moves the selection onto different files: entries whose rows are gone are dropped.
- With two or more effective items the context menu offers only "Move to notebook..." and "Delete...\tDel".
- Cross-notebook group moves stay all-or-nothing through `cross_move_host::move_items`, unchanged.
- In-notebook group failures: the rest still happen; one notice after: `"{failed} of {total} items were not moved:"` / `"... were not deleted:"` followed by one line per failure (the text the one-item path would have shown).
- Group delete confirm: `"Move {n} items to the Recycle Bin?"`; when dirty tabs are affected: `"Move {n} items to the Recycle Bin and discard unsaved changes in {d} open tab(s)?"` (`tab` when d == 1). Button: `"Delete"`.
- Drag label for a group: `"{n} items"`.
- Commit messages carry no Co-Authored-By trailer.
- After finishing: `cargo clean` and remove `native/src`; keep `native/out` and `native/cache`.

## Review Focus

1. A file watcher rebuild while a group is selected (a selected note deleted in Explorer) — the selection drops only that item, painting and Del still work. Test in Task 2.
2. A folder and a note inside it both selected, then dropped elsewhere — the note moves once, with its folder, and no "is gone" notice appears. Test in Task 5.
3. A group dropped onto a folder that is itself in the group (or inside one) — refused with the no-drop cursor, nothing moves. Test in Task 6.
4. A group delete where one item has a dirty open tab — one confirm mentions it, the tab closes without a save prompt, nothing is written. Test in Task 7.
5. Ctrl+click on a row of a second notebook while a group is selected in the first — the selection restarts in the second notebook; Del then deletes only that row. Test in Task 3.

---

### Task 0: Branch

- [ ] **Step 1:** `git switch -c feat/tree-multi-select main` (the current branch `feat/svg-default-view` is unrelated). Leave the untracked `docs/superpowers/specs/2026-11-17-fastdraw-look-tabs-design.md` alone.
- [ ] **Step 2:** Commit the spec and this plan: `git add docs/superpowers/specs/2026-11-19-tree-multi-select-design.md docs/superpowers/plans/2026-11-19-tree-multi-select.md && git commit -m "docs: notebook tree multi-select design and plan"`.

### Task 1: Pure selection model

**Files:**
- Create: `src/window/notebook_view/selection.rs` (declare `mod selection;` in `src/window/notebook_view.rs`)
- Test: same file, `#[cfg(test)] mod tests`

**Interfaces:**
- Produces:
  - `pub(crate) struct TreeSelection { notebook: Option<NotebookId>, items: Vec<RowKind>, anchor: Option<RowKind> }` with `Default`.
  - `pub(crate) fn selectable(kind: &RowKind) -> bool` — `Note` or `Folder` only.
  - `fn is_multi(&self) -> bool` — two or more items.
  - `fn contains(&self, id: NotebookId, kind: &RowKind) -> bool`
  - `fn notebook(&self) -> Option<NotebookId>`
  - `fn clear(&mut self)`
  - `fn toggle(&mut self, id: NotebookId, kind: &RowKind, cursor: Option<(NotebookId, RowKind)>)` — on a first toggle with an empty selection, the cursor row (if selectable and in `id`) joins first; another notebook restarts with just `kind`; sets anchor to `kind`.
  - `fn select_range(&mut self, rows: &[(NotebookId, RowKind)], id: NotebookId, to: usize, cursor: Option<usize>)` — anchor = existing anchor, else the cursor row, else `to`; selects every selectable row of `id` between anchor and `to` inclusive (replacing items); anchor unchanged. Rows of other notebooks in between are skipped.
  - `fn select_all(&mut self, rows: &[(NotebookId, RowKind)], id: NotebookId)`
  - `fn retain_present(&mut self, rows: &[(NotebookId, RowKind)], known: impl Fn(&RowKind) -> bool)` — drops items neither shown in `rows` nor `known` (known = still listed in the library, i.e. under a collapsed folder); drops the anchor likewise; becomes empty when one item is left.
  - `fn effective(&self) -> Vec<RowKind>` — items in tree order minus any whose ancestor folder is also an item (use `crate::library::tree::ancestors`).

- [ ] **Step 1: Write failing tests**

```rust
#[test] fn a_first_ctrl_toggle_takes_the_cursor_row_too()
// toggle(nb, Note("b.md"), Some((nb, Note("a.md")))) → items == [a.md, b.md], is_multi()
#[test] fn toggling_a_selected_row_takes_it_out_and_one_left_is_no_selection()
// then toggle(nb, Note("b.md"), _) → !is_multi(), items empty
#[test] fn a_toggle_in_another_notebook_restarts_there()
#[test] fn a_range_selects_the_rows_between_anchor_and_target_of_that_notebook_only()
// rows: [Root, Folder(x), Note(x\a), Note(b), Root(other)…]; anchor Folder(x), to=Note(b) → [x, x\a, b]
#[test] fn roots_drafts_and_placeholders_are_never_selected()
#[test] fn retain_drops_items_that_are_gone_but_keeps_hidden_ones()
#[test] fn effective_drops_items_inside_a_selected_folder()
// items [Folder(x), Note(x\a), Note(b)] → [Folder(x), Note(b)]
```

- [ ] **Step 2:** Run `cargo test --lib notebook_view::selection` — expect FAIL (module empty).
- [ ] **Step 3:** Implement `TreeSelection` as above.
- [ ] **Step 4:** Run `cargo test --lib notebook_view::selection` — expect PASS.
- [ ] **Step 5:** Commit `feat(tree): pure multi-selection model`.

### Task 2: Selection on the view — paint, rebuild, accessibility

**Files:**
- Modify: `src/window/notebook_view.rs` (field `pub(crate) multi: TreeSelection`, init in `new`; rebuild at ~line 806–855 calls `retain_present`)
- Modify: `src/window/row_list.rs:300` `paint` gains `also_selected: &dyn Fn(usize) -> bool`, ORed into `RowLook.selected`; the six other callers pass `&|_| false`
- Modify: `src/window/notebook_view/paint.rs` (tree list paint call passes `&|index| self.multi_contains_row(index)`)
- Modify: `src/window/notebook_view/accessibility.rs:216` (tree item `selected` = cursor row or in selection; `focused` only for the cursor row — pass `focused && Some(index) == list.selected`)
- Test: `src/window/notebook_view/tests.rs`

**Interfaces:**
- Consumes: Task 1 `TreeSelection`.
- Produces: `NotebookView::multi_contains_row(&self, index: usize) -> bool`; `NotebookView::row_keys(&self) -> Vec<(NotebookId, RowKind)>` (the rows as keys, for Task 1's slice parameters); `pub(crate) fn notebook_view::selected_items(hwnd) -> Option<(NotebookId, Vec<RowKind>)>` — the effective group when `is_multi()`, else `None`.

- [ ] **Step 1: Write failing tests**

```rust
#[test] fn selected_rows_report_selected_and_only_the_cursor_row_focused()
// two notes in the selection, cursor on one: both accessible items have STATE_SELECTED; only cursor has STATE_FOCUSED
#[test] fn a_rebuild_drops_a_selected_note_deleted_outside_fastpad()
// select a.md + b.md + c.md, delete b.md on disk, rescan + rebuild → selected_items == [a.md, c.md]
```

- [ ] **Step 2:** Run `cargo test --lib notebook_view::tests::selected_rows` and `::a_rebuild_drops` — FAIL.
- [ ] **Step 3:** Implement the field, paint parameter, rebuild retain (known = `library_host::with_notebook(.., |s| s.is_listed(p) || s.is_folder(p))`), accessibility, `selected_items`.
- [ ] **Step 4:** Run `cargo test --lib notebook_view` and `cargo test --lib row_list` — PASS, no other test changed.
- [ ] **Step 5:** Commit `feat(tree): paint and report a multi-selection`.

### Task 3: Mouse — Ctrl/Shift click, deferred press, right-click, Escape

**Files:**
- Modify: `src/window/notebook_view/input.rs` (`handle` passes `wparam` to `left_down(hwnd, x, y, buttons: WPARAM)`; `Hit::Row` branch at ~line 434; `WM_LBUTTONUP` at line 88; `WM_RBUTTONDOWN` at ~line 121)
- Modify: `src/window/notebook_view.rs` (field `pending_collapse: Option<usize>` — the row pressed inside a multi-selection whose click waits for the release)
- Modify: `src/window/side_panel.rs:1080` (Escape: `if notebook_view::clear_selection(main) { 0 } else { return_focus_to_editor }`)
- Test: `src/window/main_window/tests/tree_multi_select.rs` (new; add `mod tree_multi_select;` in `src/window/main_window/tests.rs`)

**Interfaces:**
- Consumes: Task 2 `multi`, `selected_items`, `row_keys`.
- Produces: `pub(crate) fn notebook_view::clear_selection(hwnd) -> bool` (true if a multi-selection was cleared); test helper `click_row(hwnd, kind: &RowKind, buttons: usize)` in the new test file (WM_LBUTTONDOWN + WM_LBUTTONUP at `centre(row_rect_at(row_of(kind)))` with `MK_LBUTTON | buttons`).

Behaviour (spec "Interaction details"):
- Plain press on a row not in the multi-selection: `clear()` then today's code.
- Plain press on a row in a multi-selection: select the cursor there, set `pending_collapse`, do **not** call `row_clicked`; arm a drag of the group (Task 6 changes the source; until then arm `DragSource::Row`). On `WM_LBUTTONUP` with no started drag and `pending_collapse` set: `clear()` then `row_clicked(index, part, false)`.
- Ctrl+press (`MK_CONTROL`): unselected row → `toggle` at press, cursor there, no activation, arm a drag. Selected row → toggle-out deferred to release (store in `pending_collapse`-like `pending_toggle: Option<usize>`); a started drag discards it.
- Shift+press: `select_range` to the row, cursor there, no activation, arm a drag.
- Right press on a row in the selection keeps it; on any other row `clear()` (today's code).

- [ ] **Step 1: Write failing tests**

```rust
#[test] fn ctrl_click_builds_a_selection_and_opens_nothing()
// click a.md; ctrl-click c.md → selected_items == [a.md, c.md]; no preview tab for c.md
#[test] fn shift_click_selects_the_range_from_the_cursor()
#[test] fn a_plain_click_inside_the_selection_collapses_it_on_release_and_opens_the_note()
#[test] fn a_plain_click_outside_clears_the_selection()
#[test] fn ctrl_click_in_another_notebook_restarts_the_selection_there()  // Review Focus 5 (add Del → only that row recycled)
#[test] fn escape_clears_the_selection_first_and_keeps_focus_then_returns_to_the_editor()
#[test] fn a_right_click_inside_the_selection_keeps_it()
```

- [ ] **Step 2:** Run `cargo test --lib main_window::tests::tree_multi_select` — FAIL.
- [ ] **Step 3:** Implement as described.
- [ ] **Step 4:** Run `cargo test --lib main_window::tests::tree_multi_select` and `cargo test --lib notebook_view` — PASS.
- [ ] **Step 5:** Commit `feat(tree): select several rows with Ctrl and Shift clicks`.

### Task 4: Keyboard

**Files:**
- Modify: `src/window/notebook_view/input.rs:742` — `key_down(hwnd, key)` becomes a wrapper reading `GetKeyState(VK_SHIFT/VK_CONTROL) < 0` into `Modifiers { shift: bool, ctrl: bool }` and calling `pub(crate) fn key_down_with(hwnd, key: u16, modifiers: Modifiers) -> bool`.
- Test: `src/window/main_window/tests/tree_multi_select.rs`

**Interfaces:**
- Produces: `pub(crate) struct Modifiers { pub shift: bool, pub ctrl: bool }` in `notebook_view/input.rs`, re-exported from `notebook_view`.

Behaviour, only when `cursor == Cursor::Tree`:
- List keys with Shift: move the cursor as today, then `select_range` to the new cursor row.
- List keys without Shift: `clear()` first, then today's code.
- Ctrl+Space (`VK_SPACE`): `toggle` the cursor row. Ctrl+A (`'A'`): `select_all` of the cursor row's notebook.
- Del with `selected_items` → Task 7's `delete_items`; F2 and Enter act on the cursor row only.

- [ ] **Step 1: Write failing tests**

```rust
#[test] fn shift_down_extends_and_a_plain_down_clears()
#[test] fn ctrl_space_toggles_and_ctrl_a_selects_the_notebooks_notes_and_folders()
#[test] fn f2_on_a_selection_renames_the_cursor_row_only()
```

- [ ] **Step 2:** Run — FAIL. **Step 3:** Implement. **Step 4:** Run `cargo test --lib main_window::tests::tree_multi_select` and `cargo test --lib main_window::tests::notebook_tree` — PASS.
- [ ] **Step 5:** Commit `feat(tree): extend and toggle the selection from the keyboard`.

### Task 5: Group move inside one notebook

**Files:**
- Modify: `src/window/tree_move.rs:190` — split `drop_into_of` into `pub(crate) fn try_drop_into_of(hwnd, id, source: &RowKind, folder: &Path) -> Result<RowKind, Option<String>>` (Ok = the row at its new path; `Err(Some(notice))` = what `drop_into_of` pushes today, verbatim; `Err(None)` = silently refused, e.g. `accepts_here` false). It does the move, expansion, links and stuck notice, but not `refresh`/`select_row`. `drop_into_of` keeps its behaviour by calling it then refreshing, selecting and pushing the notice.
- Add: `pub(crate) fn drop_many_into_of(hwnd, id, items: &[RowKind], folder: &Path)` — loops `try_drop_into_of`; one `side_panel::with_accessible_events` refresh at the end; clears the selection; selects the last moved row; pushes one summary notice (Global Constraints wording) when any `Err(Some)`.
- Test: `src/window/main_window/tests/tree_multi_select.rs`

- [ ] **Step 1: Write failing tests**

```rust
#[test] fn a_group_moves_into_a_folder_and_the_cursor_lands_on_the_last_one()
#[test] fn a_taken_name_skips_that_item_moves_the_rest_and_says_so_once()
// notices == ["1 of 3 items were not moved:\n<taken notice for b.md>"]
#[test] fn a_folder_and_a_note_inside_it_move_once_with_the_folder()  // Review Focus 2: effective() then drop; notices empty
```

- [ ] **Step 2:** Run — FAIL. **Step 3:** Implement. **Step 4:** Run `cargo test --lib main_window::tests::tree_multi_select` and `cargo test --lib main_window::tests::folder_rename_and_tree_move` — PASS (the one-item tests unchanged).
- [ ] **Step 5:** Commit `feat(tree): move a group of rows into a folder`.

### Task 6: Dragging a group

**Files:**
- Modify: `src/window/tree_drag.rs` — `DragSource::Rows(Vec<RowKind>)`; `Drag::armed` arms it; `source_accepts` for `Rows` = every item `accepts_here` (same notebook) or any folder (cross); `pub(crate) fn rows_accept(items: &[RowKind], folder: &Path) -> bool`.
- Modify: `src/window/notebook_view/input.rs` (Task 3's group press arms `DragSource::Rows(effective)` when `is_multi()`)
- Modify: `src/window/notebook_view/drag.rs` — `arm_drag` row_path stays `None` for `Rows` but a group with any item blocked by `cross_move_host::blocks` does not arm; `drag_release` → `drop_rows(hwnd, from, dest, items, folder, copy)`: MoveHere → Task 5 `drop_many_into_of`; MoveAcross → `cross_move_host::move_items(hwnd, from, items, Destination::Open { id: dest, folder })`; Copy → `copy_host::copy_into_of(hwnd, dest, absolute paths, &folder, None)`. `group_drop_at`/`open_drop_at` return `None` for `Rows`.
- Modify: `src/window/notebook_view/paint.rs:549` `drag_label_image` — `Rows(items)` → first item's icon, name `format!("{} items", items.len())`.
- Test: `src/window/tree_drag.rs` tests and `src/window/main_window/tests/tree_multi_select.rs`

- [ ] **Step 1: Write failing tests**

```rust
// tree_drag.rs
#[test] fn a_group_is_refused_by_a_folder_inside_it_and_by_its_own_folder()  // Review Focus 3
// main_window tests (drive with mouse(panel, WM_LBUTTONDOWN/WM_MOUSEMOVE/WM_LBUTTONUP), as copy_host_and_panel_drop.rs does)
#[test] fn dragging_a_selection_onto_a_folder_moves_all_of_it()
#[test] fn ctrl_dragging_a_selection_copies_it()
#[test] fn dropping_a_selection_on_another_notebook_moves_it_all_or_nothing()
```

- [ ] **Step 2:** Run — FAIL. **Step 3:** Implement. **Step 4:** Run `cargo test --lib tree_drag`, `cargo test --lib main_window::tests::tree_multi_select`, `cargo test --lib main_window::tests::tree_and_open_editors_drag`, `cargo test --lib main_window::tests::cross_move` — PASS.
- [ ] **Step 5:** Commit `feat(tree): drag a selection to move or copy it`.

### Task 7: Group delete

**Files:**
- Modify: `src/window/library_host/note_actions.rs:328` — split `delete_file` into the confirm and `fn perform_delete_note(hwnd, path: &Path) -> Result<(), String>` (everything after the confirm; the error string is today's notice text).
- Modify: `src/window/library_host/folders.rs:191` — same split: `fn perform_delete_folder(hwnd, notebook, relative: &Path) -> Result<(), String>`, without the final refresh/select (the one-item wrapper keeps that).
- Add in `note_actions.rs`: `pub(crate) fn delete_items(hwnd, notebook: NotebookId, items: &[RowKind])` — `ready_library_of` check; `refuse_item_if_moving` on any → stop; counts dirty tabs (`tabs_under` for folders, `find_stored_path` for notes); one `confirmed` with the Global Constraints question; `row_in_place_of` the first item; performs each; one refresh; clears the selection; selects the place row; one summary notice on failures.
- Modify: `src/window/notebook_view/input.rs` Del (Task 4) and `context_menu.rs` Delete call `delete_items` when `selected_items` is `Some`.
- Test: `src/window/main_window/tests/tree_multi_select.rs`

- [ ] **Step 1: Write failing tests** (answer with `crate::window::modal::answer_next_confirm`)

```rust
#[test] fn del_on_a_selection_asks_once_and_recycles_every_item()
// asserts the question == "Move 3 items to the Recycle Bin?"
#[test] fn a_group_delete_with_a_dirty_tab_says_so_and_closes_it_unsaved()  // Review Focus 4
#[test] fn declining_the_group_delete_changes_nothing()
```

- [ ] **Step 2:** Run — FAIL. **Step 3:** Implement. **Step 4:** Run `cargo test --lib main_window::tests::tree_multi_select` and `cargo test --lib main_window::tests::note_operations` — PASS.
- [ ] **Step 5:** Commit `feat(tree): delete a selection in one go`.

### Task 8: Context menu and Move to notebook for a group

**Files:**
- Modify: `src/window/notebook_view/context_menu.rs:30` — when `selected_items` is `Some((id, items))` and the row is in it: entries `[Move to notebook..., Separator, Delete...\tDel]`; Delete → `delete_items`; Move → `library_host::move_items_to_notebook(hwnd, id, items)`.
- Modify: `src/window/library_host/note_actions.rs:81` — `show_move_picker` and `host.shown_move` hold `Vec<PathBuf>` instead of one path; `move_item_to_notebook` becomes `move_items_to(hwnd, items: &[PathBuf], destination)`, ending in one `cross_move_host::move_items` call with every item. Add `pub(crate) fn move_items_to_notebook(hwnd, id: NotebookId, items: Vec<RowKind>)`. The one-item entry points pass a one-element vec.
- Test: `src/window/main_window/tests/tree_multi_select.rs` (use `menus::answer_next_popup_menu`)

- [ ] **Step 1: Write failing tests**

```rust
#[test] fn the_menu_on_a_selection_offers_only_move_and_delete()
#[test] fn move_to_notebook_on_a_selection_moves_all_items_with_one_picker()
```

- [ ] **Step 2:** Run — FAIL. **Step 3:** Implement. **Step 4:** Run `cargo test --lib main_window::tests::tree_multi_select`, `cargo test --lib main_window::tests::cross_move`, `cargo test --lib main_window::tests::note_operations` — PASS.
- [ ] **Step 5:** Commit `feat(tree): move a selection to another notebook`.

### Task 9: Full check and clean-up

- [ ] **Step 1:** `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` — all PASS.
- [ ] **Step 2:** Copy `native/out/x64` DLLs into `target/debug`, run the app, and check by hand: Ctrl/Shift select, drag a group into a folder, Ctrl-drag copy, Del on a group.
- [ ] **Step 3:** `cargo clean`; remove `native/src`; keep `native/out` and `native/cache`.
