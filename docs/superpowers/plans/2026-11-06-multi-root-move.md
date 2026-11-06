# Multi-root notebooks phase 3: moving between notebooks — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Notes and folders move between open notebooks by drag (Ctrl copies) and by Move to notebook…, with a question when links would break or images stay behind, and a checked, cancellable worker move for folders going to another drive.

**Architecture:** A pure planner (`library/cross_move.rs`) works out destinations, clashes, breaking links and images, and the question. `window/cross_move_host.rs` checks, saves dirty tabs, asks, then carries the move out: one rename on the same drive (lifted from `tree_move`), `move_file` for notes on another drive, or a `MoveWorker` job (copy, check, recycle) for a folder on another drive. Every path ends in one hand-off that updates both notebooks, tabs, pins, expanded folders and the tree.

**Tech Stack:** Rust, Win32 via windows-sys, Scintilla. Existing pieces reused: `files::copy_tree`, `files::move_file`, `files::rename_no_replace`, `modal::choose` (with `answer_next_choice`), `links::relative_target`, `links::NoteEdits` and `library_host::write_link_edits_of`, `copy_host::copy_into_of`, `note_actions::save_before_move`.

**Spec:** `docs/superpowers/specs/2026-11-06-multi-root-move-design.md`

## Global Constraints

- CI gate, all three must pass: `cargo fmt --all -- --check`; `cargo clippy --locked --target x86_64-pc-windows-msvc --all-targets --all-features -- -D warnings`; `cargo test --locked --target x86_64-pc-windows-msvc --no-fail-fast -- --test-threads=1`.
- Window tests live under `src/window/main_window/tests/` and run in `--test in_process`; each starts with `let _scintilla = load_native_scintilla();`.
- Modal dialogs in tests are answered through the existing hooks (`modal::answer_next_choice`, `answer_next_confirm`, picker test hooks); a dialog with no queued answer panics in tests.
- Wikilinks never resolve across notebooks. No link is rewritten except image links under Move with images (spec §4.2).
- User-facing strings exactly as the spec writes them (§3.2, §3.3, §5). Notebook names come from `library_host::notebook_name` / phase 2's `notebook_labels`; folder labels as "*Notebook* › *folder*" through `roots::notebook_prefixed(label, folder, true)`.
- Commits: no `Co-Authored-By` trailer, no attribution lines.
- When finished: `cargo clean`, remove `native/src` if present; keep `native/out` and `native/cache`.

## Review Focus

1. **The active tab is dirty and inside a moved folder** — it is saved first, follows to the new path, stays active, and its undo history survives. Pinned in Task 3.
2. **A clash that differs only by letter case** (`plan.md` onto a folder holding `Plan.md`) — refused as a clash, never moved over it. Pinned in Task 1 (planner) and Task 3 (window).
3. **A preview tab on a moved note** — follows and becomes a normal tab, as `move_note_to` does today. Pinned in Task 3.
4. **Some dirty tabs saved, then one refuses** (changed on disk) — nothing moves; the tabs that saved stay saved at the old path. Pinned in Task 3.
5. **Ctrl released between hover and drop** — the drop does what the label last said at release (`MK_CONTROL` of the `WM_LBUTTONUP`), and the label had updated on the key alone. Pinned in Task 5.

---

### Task 1: The planner (`library/cross_move.rs`)

**Files:**
- Create: `src/library/cross_move.rs` (register in `src/library/mod.rs`)
- Test: in-file `#[cfg(test)] mod tests`

**Interfaces:**
- Produces:
  - `pub struct Side<'a> { pub notes: &'a [PathBuf], pub index: Option<&'a NoteIndex>, pub resolver: &'a Resolver }`
  - `pub enum Item { Note(PathBuf), Folder(PathBuf) }` (relative to the source)
  - `pub struct Image { pub source: PathBuf /* relative to source notebook */, pub users_only_moving: bool }`
  - `pub struct MovePlan { pub moves: Vec<(Item, PathBuf)>, pub notes: Vec<(PathBuf, PathBuf)>, pub clashes: Vec<String>, pub breaking: Option<usize>, pub images: Vec<Image> }`
  - `pub fn plan(items: &[Item], dest_folder: &Path, source: &Side, dest: Option<&Side>, dest_exists: &dyn Fn(&Path) -> bool, source_exists: &dyn Fn(&Path) -> bool) -> MovePlan` — `dest_exists` takes a path relative to the destination notebook; `source_exists` one relative to the source (to resolve images on disk).
  - `pub enum Answer { Move, MoveWithImages, MoveWithoutImages, Cancel }`
  - `pub struct Question { pub text: String, pub buttons: Vec<(&'static str, Answer)> }`
  - `pub fn question(plan: &MovePlan, item: &str, source_name: &str, dest_name: &str) -> Option<Question>`
  - `pub fn item_label(items: &[Item]) -> String` — file or folder name for one item, "*N* items" otherwise.
  - `pub fn image_destination(folder: &Path, name: &OsStr, taken: &dyn Fn(&Path) -> bool) -> PathBuf`
  - `pub fn image_link_edits(note_index_facts: &NoteFacts, note_new: &Path, note_old: &Path, moved: &[(PathBuf, PathBuf)]) -> Vec<(Range<usize>, String)>` — for each image link of the note (old location) whose resolved target is in `moved` (old, new; relative to source / destination), the new target via `links::relative_target(note_new, new, original)`; wiki embeds get the new file name, or its relative path when the name alone would be ambiguous.

- [ ] **Step 1: Write the failing tests** (`#[cfg(test)] mod tests`, building `NoteIndex`/`Resolver` from small fixtures the way `links.rs` tests do):
  - `a_note_and_a_folder_map_into_the_destination_folder`: items `a.md`, folder `work` (holding `work\b.md`, `work\deep\c.md`) into `projects` → `moves` `[(Note a.md, projects\a.md), (Folder work, projects\work)]`; `notes` contains `(work\deep\c.md, projects\work\deep\c.md)`.
  - `any_clash_refuses_and_case_counts`: destination already has `projects\A.MD` → `clashes == ["a.md"]`.
  - `each_breaking_kind_counts_and_links_inside_the_set_do_not`: stay→move counts 1; move→stay counts 1; move whose `[[Plan]]` resolves to `projects\Plan.md` in a destination that has a different `Plan.md` counts 1; `work\b.md` → `work\deep\c.md` (both moving) counts 0; an image link counts 0. Total `breaking == Some(3)`.
  - `an_unbuilt_source_index_gives_none`: `source.index == None` → `breaking == None`, `images` empty.
  - `images_used_only_by_moving_notes_move_and_shared_ones_copy`: `a.md` embeds `attachments\x.png` and `attachments\y.png`; a staying note embeds `y.png` → `[Image{x, true}, Image{y, false}]`. A `![[z.png]]` wiki embed resolves too.
  - `images_inside_a_moving_folder_are_left_out`: `work\b.md` embeds `work\img\p.png` → `images` empty.
  - `the_question_wording_and_buttons`: one test per §3.2 row, exact strings, singular ("1 link", "1 image") and plural; Links only → `[("Move", Move), ("Cancel", Cancel)]`; images → `[("Move with images", …), ("Move without images", …), ("Cancel", …)]`; nothing → `None`; index not built → the indexing text with Move / Cancel.
  - `image_names_avoid_taken_ones`: `x.png` taken, `x (2).png` taken → `x (3).png`.
  - `image_link_edits_change_only_targets`: `![a](../attachments/x.png)` in `work\b.md` moved to `p\b.md`, image to `assets\x.png` → edit range covers exactly the target, new text `../assets/x.png`.
- [ ] **Step 2: Run** `cargo test --locked --target x86_64-pc-windows-msvc --lib cross_move -- --test-threads=1` — expect compile failure (module missing).
- [ ] **Step 3: Implement** in `src/library/cross_move.rs`. Breaking links: build the set of moving relative paths; walk `source.index` notes' `facts.links` (non-image), resolve with `source.resolver`; count per the three rules. For the third rule, resolve each moving note's wikilinks with `dest.resolver` at the note's new path, built over `dest.notes` plus the moving notes at their new paths; it counts when the result differs from the moved target's new path.
- [ ] **Step 4: Run** the same command — expect PASS.
- [ ] **Step 5: Commit** `feat(notes): plan a move between notebooks`.

### Task 2: Platform helpers (same volume, copy progress, verification)

**Files:**
- Modify: `src/platform/files.rs`
- Test: in-file tests (temp dirs, as the existing `copy_tree` tests do)

**Interfaces:**
- Produces:
  - `pub fn same_volume(a: &Path, b: &Path) -> bool` — `GetVolumePathNameW` of both, compared ignoring case; `false` when either fails. Under `cfg(test)`, `force_other_volume_for_test(on: bool)` (thread-local) makes it return `false`.
  - `pub fn copy_tree_reporting(from, to, cancel, progress: &mut dyn FnMut(usize)) -> Copied` — `copy_tree` becomes a wrapper passing a no-op; `progress` gets the running file count after each file.
  - `pub struct Snapshot(Vec<(PathBuf /* relative */, u64 /* size */, u64 /* mtime */)>)`; `pub fn snapshot(root: &Path) -> std::io::Result<Snapshot>`; `pub fn verify_copy(source: &Path, copy: &Path, before: &Snapshot) -> Result<(), Mismatch>` with `pub enum Mismatch { Missing(PathBuf), Size(PathBuf), Changed(PathBuf) }` (`Changed` = the source's size or mtime differs from `before`).
  - `pub fn count_files(root: &Path) -> usize` for the progress total.
- [ ] **Step 1: Failing tests:** `same_volume_is_true_for_two_folders_of_one_temp_dir_and_false_when_forced`; `copy_tree_reporting_counts_each_file` (3 files → progress saw `[1, 2, 3]`); `verify_copy_accepts_a_faithful_copy`; `verify_copy_names_a_missing_file`, `…_a_size_mismatch`, `…_a_source_changed_after_the_snapshot` (rewrite one source file with different length after `snapshot`).
- [ ] **Step 2: Run** `cargo test --locked --target x86_64-pc-windows-msvc --lib platform::files -- --test-threads=1` — FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** — PASS.
- [ ] **Step 5: Commit** `feat(platform): same-volume check, copy progress and copy verification`.

### Task 3: Same-drive move between notebooks (`window/cross_move_host.rs`)

**Files:**
- Create: `src/window/cross_move_host.rs` (register in `src/window/mod.rs` or wherever `tree_move` is declared)
- Modify: `src/window/tree_move.rs` — lift `move_note`/`move_folder`'s rename-and-follow into `pub(crate) fn rename_item(hwnd, old_abs: &Path, new_abs: &Path, is_folder: bool) -> Result<Moved, MoveError>` (no library update); `move_note`/`move_folder` call it, then do their in-notebook library update as now.
- Modify: `src/window/library_host/*` as needed for `pub(crate)` access to `PendingOp::Drop` / `SetPinned` application through `apply_op_of`, `remove_note`, `add_note`, pinned and expanded state.
- Test: `src/window/main_window/tests/cross_move.rs` (new; add to the tests module list)

**Interfaces:**
- Consumes: Task 1 (`plan`, `question`, `item_label`), Task 2 (`same_volume`).
- Produces:
  - `pub(crate) enum Destination { Open { id: NotebookId, folder: PathBuf }, Closed { root: PathBuf } }`
  - `pub(crate) fn move_items(hwnd: HWND, from: NotebookId, items: Vec<RowKind>, dest: Destination)`
  - `pub(crate) fn busy(hwnd: HWND, id: NotebookId) -> Option<String>` (the moving item's name; always `None` until Task 6)
  - private `fn hand_off(hwnd, from, dest, plan_notes: &[(PathBuf, PathBuf)], moved_items: &[(Item, PathBuf)], pins: Vec<PathBuf>, expanded: Vec<PathBuf>)` used by every path.
- Question: `modal::choose(hwnd, &q.text, &labels)` → index into `q.buttons`. In this task `MoveWithImages` behaves as `MoveWithoutImages` (Task 4 fills it in).

- [ ] **Step 1: Failing tests** (`two_notebooks(...)` fixture: reuse phase 2's `searchable_notebooks` test helpers that open two `LibraryScratch` notebooks; both on one temp volume):
  - `a_note_moves_into_a_folder_of_another_notebook`: disk moved; source `is_listed` false; destination `is_listed` true; its open tab's stored path is the new path; pin carried (`is_pinned` in the destination); the destination row selected; notice `Moved a.md to B › projects.`.
  - `a_folder_moves_with_its_notes_and_expanded_subfolder`: notice `Moved work (2 notes) to B.`; `work\deep` expanded in B.
  - `the_dirty_active_tab_inside_a_moved_folder_is_saved_follows_and_stays_active` (Review Focus 1): its text on disk at the new path; still active; `SCI_CANUNDO` true.
  - `a_preview_tab_follows_and_becomes_a_normal_tab` (Review Focus 3).
  - `a_case_only_clash_refuses_and_nothing_moves` (Review Focus 2): notice `a.md already exists in B › projects. Nothing was moved.`
  - `a_changed_on_disk_tab_stops_the_move_after_others_saved` (Review Focus 4): two dirty tabs in the folder, the second changed on disk; nothing moved; the first is saved at the old path; the existing `save_before_move` notice.
  - `breaking_links_ask_and_cancel_changes_nothing` and `…_move_proceeds`: `answer_next_choice` returns the Cancel / Move index; `take_last_choice()` text equals `Move a.md to B? 1 link in A will stop working.`
  - `a_tab_that_cannot_follow_undoes_the_move`: another tab already has the target path open → `Couldn't move a.md: another tab already has that file open.`, file back at source.
  - `a_late_closed_destination_is_refused`: `Destination::Open` with an id no longer open → nothing moves.
- [ ] **Step 2: Run** `cargo test --locked --target x86_64-pc-windows-msvc --test in_process cross_move -- --test-threads=1` — FAIL.
- [ ] **Step 3: Implement** `move_items` per spec §2.2 steps 1–5. Notes on another drive go through `files::move_file` here already (with its copied-but-not-removed notice). A folder on another drive is Task 6's: until then it is refused with the interim notice `Couldn't move work: moving folders to another drive isn't ready yet.` and a test pinning it (`a_folder_to_another_drive_is_refused_for_now`), both of which Task 6 deletes. Hand-off per spec §4.1. Expanded state: read `expanded` under the moved folder from the source's local state, then `set_expanded_of` re-rooted in the destination.
- [ ] **Step 4: Run** — PASS. Also run `--test in_process tree_move` and `note_operations` (the `rename_item` lift must not change them).
- [ ] **Step 5: Commit** `feat(notes): move notes and folders between notebooks on one drive`.

### Task 4: Move with images

**Files:**
- Modify: `src/window/cross_move_host.rs`
- Test: `src/window/main_window/tests/cross_move.rs`

**Interfaces:**
- Consumes: `cross_move::{image_destination, image_link_edits}`, `capture::attachments_folder` (destination's `notebook.ini` `attachments_folder`, its Obsidian setting), `library_host::write_link_edits_of(hwnd, dest_id, &[NoteEdits])`, `files::move_file`, `files::copy_file_no_replace`.
- Produces: `fn bring_images(hwnd, from, dest_id, images: &[Image], notes: &[(PathBuf, PathBuf)]) -> usize /* failed */`, called by `hand_off` when the answer was `MoveWithImages`.

- [ ] **Step 1: Failing tests:**
  - `move_with_images_moves_a_sole_image_copies_a_shared_one_and_rewrites_the_link`: after the move, `B\attachments\x.png` exists and `A\attachments\x.png` doesn't; `y.png` exists in both; the moved note's text on disk has the new targets; its clean open tab shows them.
  - `move_without_images_touches_no_image`.
  - `a_taken_image_name_gets_a_number_and_the_link_follows`: `B\attachments\x.png` already exists → `x (2).png`, link rewritten to it.
  - `an_image_that_fails_keeps_its_link_and_the_notice_says_so`: image source deleted between plan and move (delete it in the `answer_next_choice` closure) → notice ends `1 image couldn't be brought along.`; link unchanged.
- [ ] **Step 2: Run** — FAIL.
- [ ] **Step 3: Implement.** Own writes for every image path before writing. `NoteEdits` built from the moved file's current size and mtime on disk.
- [ ] **Step 4: Run** — PASS.
- [ ] **Step 5: Commit** `feat(notes): bring a moved note's images along`.

### Task 5: The tree drag across notebooks, and Ctrl to copy

**Files:**
- Modify: `src/window/tree_drag.rs` (remove `Reason::OtherNotebook`, `accepts`' notebook check becomes a `cross: bool` that skips `accepts_here`'s own-folder rule — any folder of another notebook accepts a note or folder), `src/window/drag_label.rs` (remove `OTHER_NOTEBOOK_HINT`), `src/window/notebook_view.rs` (new field `drop_notebook: Option<NotebookId>`: the notebook under the pointer for a row drag; `drag_notebook` stays the source), `src/window/notebook_view/geometry.rs` (`drag_to` targets folders of `drop_notebook` for row drags), `src/window/notebook_view/paint.rs` (the hint line shows the action), `src/window/notebook_view/drag.rs` (release routes; Ctrl key updates).
- Test: `tree_drag.rs` unit tests; `src/window/main_window/tests/tree_and_open_editors_drag.rs`.

**Interfaces:**
- Consumes: Task 3 `move_items`, `copy_host::copy_into_of`.
- Produces: `pub(crate) fn drop_action(from: NotebookId, dest: NotebookId, copy: bool) -> DropAction` with `pub(crate) enum DropAction { MoveHere, MoveAcross, Copy }`; `pub(crate) fn action_hint(action: DropAction, dest_label: &str) -> Option<String>` → `None` for `MoveHere`, `Some("Move to B › projects")`, `Some("Copy to B › projects")` (inside one notebook a Ctrl copy shows "Copy to *folder*" with the same prefix rule).
- Ruling (spec §3.1 says "the drag label"): the action text is drawn where phase 1 drew the refusal hint (`paint.rs` hint line), not in the floating label window, which keeps showing the item's name.

- [ ] **Step 1: Failing tests:**
  - Unit: replace `a_folder_of_another_notebook_refuses_with_a_reason_and_a_hint` with `a_folder_of_another_notebook_takes_a_row_even_at_the_same_relative_path` (a note at `a.md` onto B's root accepts); `drop_action` table; `action_hint` strings.
  - Window: `dropping_a_note_on_another_notebooks_folder_moves_it` (simulated press, moves, release as the existing drag tests do); `ctrl_drop_across_notebooks_copies`; `ctrl_drop_inside_one_notebook_copies`; `the_hint_reads_move_then_copy_when_ctrl_goes_down_without_a_mouse_move` (send `WM_KEYDOWN VK_CONTROL` to the panel during the drag; `drag_hint()` changes, cursor is the copy cursor); `releasing_ctrl_before_the_drop_moves` (Review Focus 5); rewrite the phase 1 test at `tree_and_open_editors_drag.rs:1110` to assert the move.
- [ ] **Step 2: Run** `--lib tree_drag` and `--test in_process tree_and_open_editors_drag` — FAIL.
- [ ] **Step 3: Implement.** Copy state = `MK_CONTROL` from mouse messages, and `GetKeyState(VK_CONTROL)` on `WM_KEYDOWN`/`WM_KEYUP` of `VK_CONTROL` while a drag has the capture (re-run `drag_to` at the last pointer and repaint).
- [ ] **Step 4: Run** — PASS.
- [ ] **Step 5: Commit** `feat(notes): drag notes and folders between notebooks; Ctrl copies`.

### Task 6: Folders to another drive (`MoveWorker`)

**Files:**
- Create: `src/window/move_worker.rs`
- Modify: `src/window/cross_move_host.rs` (folder branch, `busy`, outcome handling, close hooks), `src/window/mod.rs` (`WM_FASTPAD_MOVE_DONE`, `WM_FASTPAD_MOVE_PROGRESS`), the main window message loop dispatch, `src/window/notebook_view/{paint,accessibility,input}.rs` (progress text and Cancel button on the row), rename/delete/drag/Move-to entry points (refuse while `busy`), autosave and save paths (hold writes for tabs under a moving folder), window close and notebook close (the §3.3 prompts through `modal::choose`).
- Test: `src/window/main_window/tests/cross_move_drive.rs` (new)

**Interfaces:**
- Consumes: Task 2 (`copy_tree_reporting`, `snapshot`, `verify_copy`, `count_files`, `force_other_volume_for_test`), Task 3 `hand_off`.
- Produces:
  - `pub(crate) struct MoveJob { target: isize, from: NotebookId, dest: NotebookId, source: PathBuf, destination: PathBuf, total: usize }`
  - `pub(crate) enum MoveOutcome { Moved { files: usize }, Cancelled, Failed(String), ChangedDuring, CopiedNotRemoved }` and `pub(crate) struct MoveDone { job id fields…, outcome }`
  - `MoveWorker` on the App like `CopyWorker` (queue, cancel flag, `Drop` cancels, removes partial copy, joins); `#[cfg(test)] wait_for_moves(hwnd)`, `#[cfg(test)] fail_next_copy()`, `#[cfg(test)] pause_before_check()` / `resume()` to run assertions mid-move.
  - `pub(crate) fn cancel_move(hwnd, from: NotebookId)`; `pub(crate) fn moving_tab(hwnd, path: &Path) -> bool` (save paths consult it).
  - The recycle runs on the UI thread on `MoveDone` (the worker posts "copied and checked"), through `copy_host`'s `recycle` (test hook `fail_next_recycle`).

- [ ] **Step 1: Failing tests** (`force_other_volume_for_test(true)`):
  - `a_folder_to_another_drive_copies_checks_recycles_and_hands_off` (notice `Moved work (2 notes) to B.`).
  - `the_row_shows_progress_and_refuses_drag_rename_delete_and_a_second_move` (with `pause_before_check`): row text `Moving to B… 2 of 2`; accessible name `work, moving to B, 2 of 2`; a second `move_items` → `Wait for the move of work to finish.`.
  - `cancel_removes_the_partial_copy_and_keeps_the_original` → `Move of work cancelled. Nothing was moved.`
  - `a_failed_copy_removes_the_partial_copy` (`fail_next_copy`) → `Couldn't move work: … Nothing was moved.`
  - `a_file_changed_during_the_move_fails_the_check` → `work changed while it was being moved. Nothing was moved.`
  - `a_failed_recycle_leaves_both_copies_listed` → `work was copied to B, but the original in A couldn't be removed.`
  - `closing_the_destination_cancels`; `closing_the_source_asks_first` (choice text `A move to B is still running. Cancel it and close?`); `closing_the_window_asks_first`; `a_late_done_for_a_closed_notebook_is_dropped`.
  - `a_tab_edited_during_the_move_saves_to_the_new_path` and `…_to_the_old_path_after_cancel`.
  - Delete Task 3's placeholder-refusal test.
- [ ] **Step 2: Run** `--test in_process cross_move_drive` — FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** — PASS; rerun `cross_move`, `tabs`, `note_operations`.
- [ ] **Step 5: Commit** `feat(notes): move folders to a notebook on another drive, checked and cancellable`.

### Task 7: Move to notebook… for folders, with a folder step; README

**Files:**
- Modify: `src/window/library_host/note_actions.rs` (`move_to_notebook` takes a `RowKind`-like target: note path or folder; `move_note_to` deleted, its callers go to `move_items`), `src/window/command_palette.rs` (`PickerKind::MoveToFolder`), the picker-choice handler (step 2), `src/window/notebook_view/context_menu.rs` (Move to notebook… on folder rows), `src/window/main_window/command_dispatch.rs` (`Ctrl+Shift+M`: selected tree row, then the active tab's note), `README.md`.
- Test: `src/window/main_window/tests/note_operations.rs`, `notebook_tree.rs` (rewrite the existing `move_to_notebook` tests).

**Interfaces:**
- Consumes: Task 3 `move_items`, `Destination`.
- Produces: `pub(crate) fn move_to_notebook(hwnd: HWND, item: MoveTarget)` with `pub(crate) enum MoveTarget { Note(PathBuf /* absolute */), Folder { notebook: NotebookId, folder: PathBuf /* relative */ } }`; `fn folder_choices(hwnd, id) -> Vec<(String, PathBuf)>` → first `("B (root)", "")`, then every folder by its relative path as written (`work\deep`), in natural order.

- [ ] **Step 1: Failing tests:** `move_to_notebook_takes_a_folder_row`; `the_folder_step_lists_the_root_then_folders_in_natural_order`; `a_recent_destination_skips_the_folder_step_and_lands_in_its_root`; `ctrl_shift_m_takes_the_selected_row_then_the_active_tab`; rewrite `move_to_notebook_lists_open_notebooks_first_and_updates_an_open_destination` (pin now moves) and the three `notebook_tree.rs` callers; keep the copied-but-not-removed notice test passing through `move_items`.
- [ ] **Step 2: Run** `--test in_process note_operations notebook_tree` — FAIL.
- [ ] **Step 3: Implement.** README: the Notebook section says notes and folders drag between notebooks (Ctrl copies), the question about links and images, and Move to notebook… for folders with the folder step.
- [ ] **Step 4: Run** — PASS. Then the full CI gate.
- [ ] **Step 5: Commit** `feat(notes): Move to notebook takes folders and a destination folder; README`.
