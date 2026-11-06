# Multi-root notebooks, phase 1: implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Open several notebooks at once in one FastPad window, each one a top-level root of the Notebook tree.

**Architecture:**

- **Slots.** `LibraryHost` stops holding one notebook. It holds `notebooks: Vec<NotebookSlot>`, one slot per open notebook, each carrying its own load, watcher, index and `notebook.ini` bookkeeping.
- **Generations.** Every generation number (load, watch, index) comes from one counter shared by the whole window. A worker result therefore names exactly one slot, or none once that notebook has closed.
- **Shims.** Today's `folder(hwnd)` and `with_state(hwnd, f)` survive as temporary shims over the *default* notebook. Everything that depends on a particular file goes through `notebook_of` and `with_notebook` instead.
- **Tree.** The Notebook view builds its rows one root after another. The fixed header becomes a root row in the scrolling list, one per notebook.

**Tech Stack:** Rust 2024 on Win32 (`windows-sys`). Unit and window tests are `#[cfg(test)]` modules run by `cargo test`.

**Spec:** `docs/superpowers/specs/2026-11-05-multi-root-notebooks-design.md`

## Global Constraints

- **No new crates.** The dependency tree stays as `Cargo.toml` pins it.
- **CI must pass:**
  - `cargo fmt --all -- --check`
  - `cargo clippy --locked --target x86_64-pc-windows-msvc --all-targets --all-features -- -D warnings`
  - `cargo test --locked --target x86_64-pc-windows-msvc -- --test-threads=1`
- **Commits.** Messages never include a `Co-Authored-By` trailer.
- **Copy, verbatim from the spec:**

  | Where | Text |
  |---|---|
  | Placeholder rows | "Loading…", "Couldn't load. Retry", "No notes yet" |
  | Root `…` menu | "Close notebook", "Set as default notebook", "Add to Favorites" / "Remove from Favorites" |
  | Default marker tooltip | "Default notebook" |
  | UI Automation name of the default root | "*Name* (default)" |
  | Cross-root drop hint | "Moving between notebooks isn't supported yet" |
  | Search view header | "Searching *Name*" |
  | Quick Open placeholder | "Search notes in *Name*" |
  | Activity-bar tooltip and side-panel label, several notebooks open | "*N* notebooks" |
  | Missing notebook at startup | "*path* is not available." |

- **Nested notebooks are refused** in both directions, inside an open notebook or containing one. `notebook_of` relies on this: at most one root can own a path.
- **Wikilinks and backlinks** resolve inside the note's own notebook only.
- **Search, Quick Open and Tags** cover `target_notebook` only in this phase.
- **Quick capture** always targets the default notebook, or that notebook's `capture_target`.
- **`folders.ini` stays compatible:**
  - with no `open=` lines and no `open=none`, startup opens `folder[0]`;
  - `open=none` wins over `open=<path>` lines;
  - `default=` is ignored unless it also appears on an `open=` line.
- **Resolved spec ambiguity: changed-notes filter.** "Show changed notes" (`changed_only`) is one window-wide filter that applies to every root, as spec §3.1 places it on the host. §5.1's mention of it means only that the command works when any notebook is open.
- **Naming convention.** A per-notebook variant of an existing host function takes `id: NotebookId` as its second parameter and carries the suffix `_of`, for example `start_load_of(hwnd, id)`. The window-side names are separate from `library::capture_file::notebook_settings_of(root)`, which is unrelated and does not change.

## Review Focus

1. **The same notebook spelled differently.** Different case, a trailing backslash, or `D:\Notes\.`. It must reveal the existing root, not add a second one.
   *Test:* Task 4, `opening_the_same_notebook_spelled_differently_reveals_it`.
2. **Two roots with the same folder name** (`D:\Work\Notes` and `E:\Notes`). Root rows and labels must tell them apart, using the `display_names` hint.
   *Test:* Task 9, `two_roots_with_the_same_name_show_their_parent_hint`.
3. **Closing a notebook while its load, index or copy worker runs.** The late result is dropped, and no other root changes.
   *Test:* Task 5, `results_for_a_closed_notebook_are_dropped`.
4. **A tab whose notebook was closed and then reopened.** Autosave and pins route to the reopened root again, because `notebook_of` is evaluated on every call.
   *Test:* Task 7, `a_tab_rejoins_its_notebook_when_it_reopens`.
5. **An untitled note made in notebook B, with B closed before its first save.** The save writes into that folder as an ordinary file. It must neither crash nor silently save into the default notebook.
   *Test:* Task 7, `first_save_after_its_notebook_closed_saves_as_a_plain_file`.

---

### Task 1: `folders.ini` learns `open=` and `default=`

**Files:**
- Modify: `src/library/local.rs`, the `RecentFolders` struct at :307 and its tests at the end of the file.

**Interfaces:**
- **Produces, on `RecentFolders`:**

  ```rust
  pub open: Vec<PathBuf>
  pub default: Option<PathBuf>
  pub fn set_open(&mut self, open: &[PathBuf], default: Option<&Path>)
  pub fn startup_roots(&self) -> (Vec<PathBuf>, Option<PathBuf>)
  ```

- **Behaviour:**
  - `set_open` with an empty list sets `closed = true` and clears `default`. With a non-empty list it sets `closed = false`.
  - `startup_roots` returns the roots to open and the default notebook, following the spec §6.1 table.
  - `push` (an existing method) leaves `open` alone.

- [ ] **Step 1: Write the failing tests** in `local.rs`'s test module:

  - **`open_and_default_round_trip_in_order`**
    1. Call `set_open(&[D:\Work, E:\Home], Some(E:\Home))`.
    2. Check that `encode()` contains `"open=D:\\Work\r\nopen=E:\\Home\r\ndefault=E:\\Home\r\n"` before the `folder=` lines.
    3. Check that `parse(encode())` equals the original value.
  - **`a_file_without_open_lines_starts_with_its_most_recent_folder`**
    - Parse `"version=1\r\nfolder=D:\\Work\r\nfolder=E:\\Old\r\n"`.
    - Expect `startup_roots() == (vec![D:\Work], Some(D:\Work))`.
  - **`open_none_wins_over_open_paths`**
    - Parse `"version=1\r\nopen=none\r\nopen=D:\\Work\r\n"`.
    - Expect `startup_roots() == (vec![], None)`.
  - **`a_default_that_is_not_open_falls_back_to_the_first_root`**
    - Parse `open=D:\A`, `open=D:\B`, `default=D:\C`.
    - Expect `(vec![A, B], Some(A))`.
  - **`setting_no_open_roots_records_a_close`**
    - Call `set_open(&[], None)`.
    - Expect `closed`, and `encode()` contains `open=none` and no `default=`.

- [ ] **Step 2: Run the tests and check that they fail.**
  - Run: `cargo test --locked --target x86_64-pc-windows-msvc library::local -- --test-threads=1`
  - Expected: compile errors, because `set_open`, `startup_roots` and the new fields do not exist.

- [ ] **Step 3: Implement.**
  - **Parsing.** `Some(("open", v))` where `v != "none"` pushes onto `open`. `Some(("default", v))` sets `default`.
  - **Encoding order:** `version`, then `open=none` or the `open=` lines, then `default=`, then `folder=`, then `favorite=`.
  - **Limits.** Truncate `open` to `FOLDER_LIMIT`.
  - **Normalisation and matching.** Normalise with `super::normalize_folder`, and compare with `same_path`, as `push` does.

- [ ] **Step 4: Run the tests and check that they pass.** Same command as Step 2. All of `library::local` passes, including the existing `open=none` tests.

- [ ] **Step 5: Commit.** `git commit -am "feat(library): folders.ini remembers every open notebook and the default"`

---

### Task 2: Pure root helpers (`library::roots`)

**Files:**
- Create: `src/library/roots.rs`
- Modify: `src/library/mod.rs`, to add `pub mod roots;`.

**Interfaces:**
- **Produces:**

  ```rust
  pub enum Nesting {
      /// The candidate is inside `open[i]`.
      Inside(usize),
      /// The candidate contains `open[i]`.
      Contains(usize),
  }

  pub fn nesting(open: &[PathBuf], candidate: &Path) -> Option<Nesting>
  pub fn owner(open: &[PathBuf], path: &Path) -> Option<usize>
  pub fn missing_notice(missing: &[PathBuf]) -> Option<String>
  ```

- **`nesting`** returns `None` when `candidate` is the same folder as an open one. Callers check `same_path` first, and that case means "reveal".
- **`owner`** returns the index of the folder that contains `path`, using `is_inside`.
- **`missing_notice`** returns `None` for an empty list. For one path it returns `"D:\\Notes is not available."`. For several it returns `"D:\\A, D:\\B and D:\\C are not available."`.

- [ ] **Step 1: Write the failing tests** in `roots.rs`:

  - **`nesting_finds_a_folder_inside_or_around_an_open_one`**
    - With `open = [D:\Notes]`:

      | Candidate | Expected |
      |---|---|
      | `D:\Notes\Work` | `Some(Inside(0))` |
      | `D:\` | `Some(Contains(0))` |
      | `D:\NotesOld` | `None` (a prefix of the name is not nesting) |
      | `d:\notes\` | `None` (the same folder) |

  - **`owner_picks_the_root_that_contains_the_path`**
    - With `open = [D:\A, E:\B]`:

      | Path | Expected |
      |---|---|
      | `E:\B\x\n.md` | `Some(1)` |
      | `F:\n.md` | `None` |

  - **`missing_notice_lists_every_missing_root`** checks the three cases above.

- [ ] **Step 2: Run the tests and check that they fail.**
  - Run: `cargo test --locked --target x86_64-pc-windows-msvc library::roots -- --test-threads=1`
  - Expected: FAIL, because the module does not exist yet.

- [ ] **Step 3: Implement**, on top of `library::is_inside`, `library::normalize_folder` and `model::same_path`.

- [ ] **Step 4: Run the tests and check that they pass.** Same command as Step 2.

- [ ] **Step 5: Commit.** `git commit -am "feat(library): pure helpers for nested notebooks and root ownership"`

---

### Task 3: Notebook slots behind the single-notebook shims (no behaviour change)

This is the mechanical heart of the work. When it is done, `LibraryHost` holds a `Vec<NotebookSlot>` that never has more than one entry, and the whole existing test suite passes unchanged.

**Files:**
- Create: `src/window/library_host/slots.rs`, which holds `NotebookId`, `NotebookSlot`, `slot`, `slot_by_*` and the public accessors listed below.
- Modify:
  - `src/window/library_host.rs`: move the struct fields at :65-136 into the slot; `host`, `folder` and `with_state` stay at :202-216.
  - `src/window/library_host/load.rs`
  - `src/window/library_host/watch.rs`
  - `src/window/library_host/index.rs`
  - `src/window/library_host/local_state.rs`
  - `src/window/library_host/autosave.rs`: `install_for_test` at :285.
  - `src/window/library_host/daily.rs`: the `notebook` and `notebook_text` fields.
  - `src/window/library_host/links.rs`: `renamed_outside`.
  - `src/window/library_host/notebooks.rs`
- Test: the existing suite, plus one new test in `src/window/main_window/tests/notebook_tree.rs`.

**Interfaces:**
- **Produces, in `library_host::slots`, re-exported through `library_host`:**

  ```rust
  #[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
  pub(crate) struct NotebookId(pub(crate) u32);

  pub(crate) struct NotebookSlot { /* fields of spec §3.1 */ }

  fn slot<R>(hwnd, id: NotebookId, f: impl FnOnce(&mut NotebookSlot) -> R) -> Option<R>

  pub(crate) fn notebooks(hwnd) -> Vec<(NotebookId, PathBuf)>       // sidebar order
  pub(crate) fn default_notebook(hwnd) -> Option<NotebookId>
  pub(crate) fn notebook_folder(hwnd, id) -> Option<PathBuf>
  pub(crate) fn with_notebook<R>(hwnd, id, f: impl FnOnce(&mut LibraryState) -> R) -> Option<R>
  ```

- **New fields on `LibraryHost`:**
  - `notebooks: Vec<NotebookSlot>`
  - `default_notebook: Option<NotebookId>`
  - `next_id: u32`
  - `next_generation: u64`
- **`LibraryHost::fresh_generation(&mut self) -> u64`** returns the next value of `next_generation`. Every write to a slot's `generation`, `watch_generation` or `index_generation` uses it, so no two slots ever share a generation value.
- **Per-notebook variants**, `_of(hwnd, id, …)` versions of:

  ```
  start_load, retry_load, load_failed, install, ensure_watcher, stop_watcher, watching,
  request_index, reset_index, note_index, index_loaded, save_local, try_flush,
  reload_notebook_settings, notebook_settings, set_expanded, set_all_expanded,
  any_folder_expanded, root_expanded, set_root_expanded, expanded
  ```

  The original no-argument functions become shims that call the `_of` version on `default_notebook`.
- **Test hook.** `#[cfg(test)] pub(crate) fn generations_of(hwnd, id) -> (u64, u64, u64)` returns the slot's load, watch and index generations. Tasks 3–6 use it.
- **Lookups.** `library_ready`, `files_changed` and `index_ready` find their slot with `slot_by_load_generation`, `slot_by_watch_generation` and `slot_by_index_generation`, and drop the payload when no slot matches.
- **The temporary single-slot rule.** `open_checked_folder` still *replaces* the notebook: it closes the default slot and then adds one. Task 4 changes that.

- [ ] **Step 1: Write the failing test** `a_load_result_for_a_replaced_notebook_is_dropped` in `tests/notebook_tree.rs`:
  1. Install scratch A.
  2. Capture `test_ready_payload(old_generation, Ok(state_of_A))`, where `old_generation` is A's load generation.
  3. Open scratch B with `open_folder`, then pump until B is loaded.
  4. Deliver the captured payload to `library_ready`.
  5. Assert that `notebooks(hwnd)` is `[B]` and that B's notes are unchanged.

  `test_ready_payload` gains no new parameter, because the generation alone identifies the slot.

- [ ] **Step 2: Run the test and check that it fails.**
  - Run: `cargo test --locked --target x86_64-pc-windows-msvc a_load_result_for_a_replaced -- --test-threads=1`
  - Expected: FAIL, because `notebooks` does not exist yet.

- [ ] **Step 3: Move the per-notebook fields into `NotebookSlot`.** Re-point every `host.folder`, `host.state`, `host.generation`, `host.watcher` and `host.index*` use at the slot. Implement the `_of` variants and the shims, and switch the generations to `fresh_generation`.

- [ ] **Step 4: Update `install_for_test`.** When no slot exists, it creates one, so the existing helpers keep working. `LibraryScratch::install` and `library(hwnd)` in `src/window/main_window/tests.rs:323` and :400 read the default slot.

- [ ] **Step 5: Run the full suite.**
  - Run: `cargo test --locked --target x86_64-pc-windows-msvc -- --test-threads=1`
  - Expected: PASS, with the same count as `main` plus the new test.
  - Then run clippy and check that it passes with no warnings.

- [ ] **Step 6: Commit.** `git commit -am "refactor(library_host): per-notebook slots behind single-notebook shims"`

---

### Task 4: Opening adds a root; closing one notebook

**Files:**
- Modify:
  - `src/window/library_host/notebooks.rs`: `open_folder`, `open_checked_folder` at :59, `notebook_checked`, `close_notebook`, `save_before_leaving` at :34, and `files_dropped`.
  - `src/window/library_host/slots.rs`
  - `src/window/main_window/ipc_host.rs:121`, which needs no code change because it already calls `open_folder`. Keep it under test.
- Test: `src/window/main_window/tests/notebook_tree.rs`

**Interfaces:**
- **Consumes:** Task 2's `roots::nesting`; Task 3's slots and `_of` functions.
- **Produces:**

  ```rust
  pub(crate) fn close_notebook_of(hwnd, id) -> bool            // false = kept open
  pub(crate) fn set_default_notebook(hwnd, id)
  pub(crate) fn notebook_opened(hwnd, id)                      // side_panel hook
  pub(crate) fn reveal_notebook(hwnd, id)                      // in notebook_view; Task 9 renders it
  ```

  - `close_notebook_of` returns false when the notebook was kept open.
  - `notebook_opened` is the hook that calls `crate::window::side_panel::refresh`.
  - Until Task 9, `reveal_notebook` selects the root by scrolling to the top.
- **Notice copy:**
  - "*{F.display()}* is inside the open notebook *{name}*."
  - "*{F.display()}* contains the open notebook *{name}*."
  - `name` is the folder's display name, from `local::display_names` over the open roots.

- [ ] **Step 1: Write the failing tests:**
  - **`opening_a_second_notebook_adds_a_root_and_keeps_the_first_live`**
    - After opening A then B, `notebooks()` is `[A, B]`.
    - Both slots have loaded `state`, and both have a watcher (`watching_of`) once loaded.
    - The default notebook is A.
  - **`opening_the_same_notebook_spelled_differently_reveals_it`**
    - Open A, then open A's path upper-cased, with a trailing `\`.
    - There is still one root, and no second load starts: the load generation is unchanged.
  - **`a_notebook_inside_or_around_an_open_one_is_refused`**
    - With A open, opening `A\sub` adds no root and pushes the "is inside" notice.
    - Opening A's parent adds no root and pushes the "contains" notice.
  - **`closing_one_notebook_keeps_its_tabs_and_hands_the_default_on`**
    1. Open A and B, with a note from A open in a tab. A is the default.
    2. Call `close_notebook_of(A)`.
    3. Expect `notebooks() == [B]`, `default_notebook() == B`, and the tab still open with the same path.
  - **`closing_a_notebook_whose_pins_cannot_be_written_keeps_it_open`**
    - Hold A's `library.ini` open exclusively. Use the existing busy-file helper in `tests/note_operations.rs` if there is one; otherwise use `std::fs::OpenOptions` with `share_mode(0)`.
    - Pin a note, then call `close_notebook_of(A)`.
    - Expect it to return false, A to still be listed, and the notice "FastPad kept this notebook open because it could not save its pins".
  - **`closing_the_last_notebook_records_open_none`**: `read_folders(...).closed` is true.

- [ ] **Step 2: Run the tests and check that they fail.**
  - Run: `cargo test --locked --target x86_64-pc-windows-msvc notebook_tree -- --test-threads=1`
  - Expected: the new tests FAIL.

- [ ] **Step 3: Implement the spec §3.3 steps 1–4 in `open_checked_folder`**, in this order:
  1. Notes mode.
  2. The `same_path` check. When it matches, call `reveal_notebook`.
  3. `roots::nesting`, with its notice.
  4. Append the slot, `update_folders` (push plus `set_open`), `start_load_of`, and `notebook_opened`.

  `save_before_leaving` becomes `save_before_leaving_of(hwnd, id)`. It is no longer called on open, only on close.

- [ ] **Step 4: Implement `close_notebook_of`** following spec §3.4 steps 1–8.
  - The palette's and menu's `close_notebook(hwnd)` close `default_notebook()` until Task 8 swaps in `target_notebook`.
  - Every `open=` change goes through `update_folders`, calling `folders.set_open(&open_roots, default_root)`.

- [ ] **Step 5: Run the tests and check that they pass.**
  - Run the full suite. Expected: PASS.
  - Update any existing test that relied on *open replaces*, such as the switching tests in `tests/notebook_tree.rs`, `tests/open_editors_and_folder_commands.rs` and `tests/new_notebook.rs`, to the new "adds" behaviour. Keep their intent: the old notebook's dirty notes are no longer saved on open, but they are on close.

- [ ] **Step 6: Commit.** `git commit -am "feat(notes): opening a notebook adds a root; close one notebook at a time"`

---

### Task 5: Every slot flushes, rescans and fails on its own

**Files:**
- Modify:
  - `src/window/library_host/local_state.rs`: `schedule_write`, `flush_now`, `flush_before_close`, `flush_reporting`, `try_flush` and `activation_changed`.
  - `src/window/library_host/load.rs`: the `library_ready` error path.
  - `src/window/copy_host.rs:440`: route `CopyDone` by `root` to `notebook_of`.
- Test: `src/window/main_window/tests/external_changes.rs` and `tests/notebook_tree.rs`.

**Interfaces:**
- **Consumes:** Task 2's `roots::owner`; Task 3's `_of` functions and `generations_of`. This task defines `notebook_of` in `slots.rs`, as `roots::owner` over `notebooks()`. Task 7 migrates its callers.
- **Produces:**
  - `pub(crate) fn notebook_of(hwnd, path: &Path) -> Option<NotebookId>`.
  - One `LIBRARY_WRITE_TIMER_ID` timer, whose tick runs `try_flush_of` on every slot that has `pending` work.

- [ ] **Step 1: Write the failing tests:**
  - **`results_for_a_closed_notebook_are_dropped`**
    1. Open A and B.
    2. Take A's watch, index and load generations.
    3. Close A.
    4. Post a `FileChanges` with A's watch generation, an `IndexReady` and a `Loaded` with A's generations, and a `CopyDone` whose root is A.
    5. Assert that B's state is untouched (note count, index Arc pointer), that no root is added, and that nothing panics.
  - **`one_failed_load_shows_retry_on_that_root_only`**
    - Deliver `test_ready_payload(b_generation, Err("boom"))`.
    - Expect `load_failed_of(B)`, `!load_failed_of(A)`, and A's notes still listed.
  - **`a_busy_library_file_in_one_notebook_does_not_stop_the_other_flushing`**
    - Pin a note in A and a note in B, and hold A's `library.ini` exclusively.
    - Call `flush_now`.
    - Expect B's `library.ini` on disk to contain the pin, and A to still have `pending` work.
  - **`activation_rescans_only_notebooks_without_a_watcher`**
    - Set `watch_failed` on A's slot through a test hook.
    - Deactivate past `RESCAN_AFTER`, then activate.
    - Expect A's load generation to have advanced and B's to be unchanged.

- [ ] **Step 2: Run the tests and check that they fail.**
  - Run: `cargo test --locked --target x86_64-pc-windows-msvc -- --test-threads=1 results_for_a_closed one_failed_load a_busy_library activation_rescans`
  - Expected: FAIL.

- [ ] **Step 3: Implement.** Loop over `notebooks()` in every flush and activation path. Report each slot's failure with the existing notice text, unchanged, and carry on to the next slot.

- [ ] **Step 4: Run the full suite.** Expected: PASS.

- [ ] **Step 5: Commit.** `git commit -am "feat(notes): open notebooks flush, rescan and fail independently"`

---

### Task 6: Startup restores every notebook

**Files:**
- Modify: `src/window/library_host/load.rs`: `resolve_startup` at :14, `open_library_step` at :51, the `Startup` and `Loaded` structs in `library_host.rs:176-200`, and `remember_folder`.
- Test: the `library_host.rs` unit tests at :236 and `tests/notebook_tree.rs`.

**Interfaces:**
- **Consumes:** Task 1's `RecentFolders::startup_roots` and `set_open`; Task 2's `roots::missing_notice`.
- **Produces:**

  ```rust
  // in Startup
  remembered: Vec<PathBuf>,
  default: Option<PathBuf>,

  pub(super) fn resolve_startup(startup: Startup) -> StartupRoots

  pub(super) struct StartupRoots {
      pub(super) roots: Vec<PathBuf>,      // exist, in order, launch last
      pub(super) default: Option<PathBuf>,
      pub(super) notice: Option<String>,
  }
  ```

  - `remembered` replaces `Option<PathBuf>`, and `default` is new.
  - `resolve_startup` checks every remembered root on its own `std::thread::scope` thread.

- **What `resolve_startup` produces:**
  - **Missing roots** are left out of `roots`, but stay on their `open=` lines: the worker does **not** rewrite `open=`.
  - **The launch folder** is appended if it exists and is not already listed. It is not nested-checked here; the UI thread refuses it later, through the Task 4 path.
  - **The `Documents\FastPad` fallback** is used only when `remembered` is empty, `closed` is false, and there is no launch folder.

- **On the UI thread.** `open_library_step` creates one slot per root, and each one loads through its own worker, which is today's `spawn_load` with the slot's folder. The startup notice comes from `missing_notice`.

- [ ] **Step 1: Write the failing tests.** In `library_host.rs` tests:
  - **`startup_restores_every_open_root_in_order_and_keeps_missing_ones`**
    - Use remembered `[A, Missing, B]` with default B.
    - Expect `roots == [A, B]`, `default == Some(B)`, and `notice == Some("…Missing is not available.")`.
    - Assert that `folders.ini` still lists `Missing` under `open=`.
  - **`a_missing_default_hands_over_to_the_first_root_that_opened`**: with the default missing, `default == Some(A)`.
  - **`a_launch_folder_is_added_after_the_restored_roots`**: expect `roots == [A, B, Launch]`.
  - Keep **`a_session_that_closed_its_notebook_opens_none_and_checks_no_folder`** and update it to the new return type.

  In `tests/notebook_tree.rs`:
  - **`two_remembered_notebooks_both_load_at_startup`**
    1. Write `folders.ini` with `open=` A and B.
    2. Run `open_library_step` and pump.
    3. Expect two loaded slots, in order.

- [ ] **Step 2: Run the tests and check that they fail.**
  - Run: `cargo test --locked --target x86_64-pc-windows-msvc startup -- --test-threads=1`
  - Expected: FAIL.

- [ ] **Step 3: Implement** as specified above. `library_ready`'s `moved` logic (the assumed folder against the real one) applies per slot. The UI creates slots for the remembered roots before the worker answers, so the tree shows "Loading…" under each one. The worker's answer removes the slots whose root is missing.

- [ ] **Step 4: Run the full suite.** Expected: PASS.

- [ ] **Step 5: Commit.** `git commit -am "feat(notes): restore every open notebook at startup"`

---

### Task 7: Route per-file decisions to the file's own notebook

**Files:**
- Modify. The callers listed below move from `folder()` and `with_state()` to `notebook_of(path)` and `with_notebook(id, …)`:

  | File | What changes |
  |---|---|
  | `src/window/library_host/autosave.rs` | `folder_autosave` at :21, `autosave_target` at :32, `autosave_all` at :120, `document_saved` |
  | `src/window/library_host/pins.rs` | :97 |
  | `src/window/library_host/naming.rs` | :265 |
  | `src/window/library_host/note_actions.rs` | :49 and :170 |
  | `src/window/library_host/untitled.rs` | `new_note_in`, `save_folder_for` |
  | `src/window/main_window/saving.rs` | :65 |
  | `src/window/attachments.rs` | |
  | `src/window/library_host/links.rs` | `resolver`, `completion_items`, `follow_wiki`, `create_linked_note`, `notes_moved` |
  | `src/window/markdown_completion.rs` | |
  | `src/window/library_host/external.rs` | |
  | `src/window/editor_toolbar.rs` | `location` (the breadcrumb), at :161 |
  | `src/window/inline_name.rs` | the root and state reads, which resolve their notebook from the target row or folder (Task 9 provides row ids; until then, from the absolute folder) |

- Test: `src/window/main_window/tests/first_save_and_autosave.rs` and `tests/links_and_backlinks.rs`.

**Interfaces:**
- **Consumes:** `notebook_of`, from Task 5; `with_notebook` and `_of`, from Task 3.
- **Produces:**
  - `pub(crate) fn note_index_for(hwnd, path: &Path) -> Arc<NoteIndex>`, the index of the notebook that owns `path`, or an empty index when no notebook does.
  - The links and completion code uses it in place of `note_index(hwnd)`.

- [ ] **Step 1: Write the failing tests.** Each opens notebooks A and B, with B *not* the default:
  - **`autosave_saves_a_note_in_a_non_default_notebook`**
    - Edit a note of B and fire the autosave timer.
    - Expect it saved, and `autosave_active` to return `Saved`.
  - **`turning_autosave_off_in_one_notebook_leaves_the_other_autosaving`**
    - Set B's local autosave to false.
    - Edit notes in A and B and fire the timer.
    - Expect A's note saved and B's still dirty.
  - **`pinning_a_note_writes_to_its_own_notebook`**
    - Pin a note of B.
    - Expect B's `library.ini` to contain the pin, and A's not to.
  - **`first_save_names_a_new_note_in_the_notebook_it_was_created_in`**
    - Run new note with the context set to B (call `new_note_in(hwnd, B_folder)`), type and save.
    - Expect the file under B, and B's `state.notes` to contain it.
  - **`a_wikilink_resolves_inside_its_own_notebook_only`**
    - A and B each have `Plan.md`. From a note in B, follow `[[Plan]]`.
    - Expect B's `Plan.md` opened.
    - Completion items from a B note list B's notes only.
  - **`a_tab_rejoins_its_notebook_when_it_reopens`**
    1. Open a note of B, then close B. Editing it does not autosave, because it is now a plain file.
    2. Reopen B and edit again.
    3. Expect an autosave.
  - **`first_save_after_its_notebook_closed_saves_as_a_plain_file`**
    1. Create an untitled note in B, then close B.
    2. Save and accept the name box.
    3. Expect the file in B's folder, no slot changed, and no panic.
  - **`the_breadcrumb_shows_the_active_notes_own_notebook`**: the toolbar location's root text is B's name.

- [ ] **Step 2: Run the tests and check that they fail.**
  - Run: `cargo test --locked --target x86_64-pc-windows-msvc -- --test-threads=1 first_save_and_autosave links_and_backlinks`
  - Expected: the new tests FAIL.

- [ ] **Step 3: Migrate each caller in the list.** An untitled tab with no `save_folder` uses `default_notebook()`'s folder, as spec §5.2 requires.

- [ ] **Step 4: Check that no correctness-critical caller still uses the shims.**
  - Run: `grep -rn "library_host::folder(\|library_host::with_state(\| folder(hwnd)\| with_state(hwnd" src/window --include=*.rs`
  - Expected: the only callers left are those Task 8 or Task 9 own (target-notebook commands, the tree view, search, Quick Open and Tags). Record the remaining list in the commit message.

- [ ] **Step 5: Run the full suite.** Expected: PASS.

- [ ] **Step 6: Commit.** `git commit -am "feat(notes): autosave, pins, naming and links follow the file's own notebook"`

---

### Task 8: Target notebook, commands, capture and Move to notebook

**Files:**
- Modify:
  - `src/window/library_host/slots.rs`
  - `src/window/commands.rs`
  - `src/window/main_window/command_dispatch.rs`
  - `src/window/library_host/daily.rs`: `open_today`, `step_daily`, `new_note_from_template` and notebook settings.
  - `src/window/library_host/autosave.rs`: `toggle_folder_autosave` at :164.
  - `src/window/library_host/links.rs`: `update_renamed_links`.
  - `src/window/library_host/note_actions.rs`: `move_destinations`, `move_to_notebook` and `move_note_to`.
  - `src/window/capture_host.rs`
  - `src/window/capture_only.rs`
  - `src/window/capture_box.rs`, so the box shows the target notebook's name.
  - `src/window/text_search_host.rs`
  - `src/window/search_view/*`, for the header copy.
  - `src/window/main_window/command_palette_ui.rs`: Quick Open at :261 and :402.
  - `src/window/tags_view.rs`
- Test:
  - `tests/command_palette.rs`
  - `tests/daily_notes_and_templates.rs`
  - `tests/quick_capture.rs`
  - `tests/note_operations.rs`
  - `tests/search_view.rs`
  - `tests/index_and_tags.rs`

**Interfaces:**
- **Consumes:** Task 3 accessors and `notebook_of`.
- **Produces:**
  - `pub(crate) fn target_notebook(hwnd) -> Option<NotebookId>`, following the spec §5.1 order.
  - To support it, `notebook_view` exposes `pub(crate) fn selected_notebook(hwnd) -> Option<NotebookId>`. It returns `Some` when the Notebook view has focus or a selected row. Until Task 9, it returns the default when the selection is on the root.
  - `pub(crate) fn capture_notebook(hwnd) -> Option<NotebookId>`, which is `default_notebook`.
- **Commands and their target.** These switch from shims to `target_notebook`:
  - the `CloseNotebook` and `ToggleNotebookFavorite` palette commands;
  - `NotesEditNotebookSettings`, toggle folder autosave and update renamed links;
  - daily today, previous and next, and new from template;
  - Search, Quick Open and Tags.
- **The Move to notebook picker** lists rows in this order:
  1. the other open notebooks, in sidebar order;
  2. then recent ones that are not open;
  3. then "Browse…".
- **Moving to an open destination.** `move_note_to` into an open destination also calls `with_notebook(dest, |s| s.add_note(...))`, the same call a save into the folder makes, and requests the destination's index.

- [ ] **Step 1: Write the failing tests:**
  - **`the_target_is_the_selected_row_then_the_active_tab_then_the_default`**
    - Open A (the default) and B.
    - With the focus in the editor on a B note, expect B.
    - With an untitled tab active, expect A.
    - With the Notebook view focused and a B row selected (set `list.selected` directly until Task 9), expect B.
  - **`todays_daily_note_goes_to_the_target_notebook`**: with a B note active, `open_today` creates the file under B's daily folder.
  - **`quick_capture_appends_to_the_default_notebooks_inbox`**
    - With a B note active and A the default, a capture appends to A's `inbox.md`.
    - The capture box's notebook label reads A's name.
  - **`move_to_notebook_lists_open_notebooks_first_and_updates_an_open_destination`**
    - Open A, B and recent C.
    - The picker rows are `[B, C, Browse…]`.
    - After moving a note of A to B, B's `state.notes` contains it without a rescan.
  - **`search_covers_the_target_notebook_and_names_it`**
    - A and B both contain "needle". With a B note active, search for "needle".
    - Hits come only from B, and the header reads "Searching B".
  - **`quick_open_lists_the_target_notebook_and_names_it`**: the placeholder reads "Search notes in B", and the rows are B's notes.
  - **`the_tags_view_shows_the_target_notebooks_tags`**

- [ ] **Step 2: Run the tests and check that they fail.**
  - Run: `cargo test --locked --target x86_64-pc-windows-msvc -- --test-threads=1 command_palette daily_notes quick_capture note_operations search_view index_and_tags`
  - Expected: the new tests FAIL.

- [ ] **Step 3: Implement.**
  - Search runs `text_search::run(target_folder, target_notes, …)`.
  - Quick Open passes the target's `state.notes`.
  - Tags reads `note_index_of(target)`.
  - When the target changes, search and Tags refresh on `active_tab_changed` and on `notebook_opened`.

- [ ] **Step 4: Run the full suite.** Expected: PASS.

- [ ] **Step 5: Commit.** `git commit -am "feat(notes): commands, capture, search and move pick their notebook by context"`

---

### Task 9: Root rows in the scrolling tree

**Files:**
- Modify:
  - `src/library/tree.rs:20`, adding to `RowKind`:

    ```rust
    Root,
    Placeholder(Placeholder),

    pub enum Placeholder { Loading, Failed, Empty }
    ```

  - `src/window/notebook_view.rs`:
    - `Mode` at :91 shrinks to `NoNotebook | Tree`;
    - `Hit::Root` and `Header` carry a `NotebookId`;
    - `RebuildKey` at :360 becomes `Vec<RootKey>` plus `changed_only`;
    - `rows: Vec<ViewRow>`;
    - the single `root`, `name` and `favorite` fields are removed.
  - `src/window/notebook_view/rebuild.rs`: `snapshot`, `active_target`, `focused_note`, `focused_folder` and `selected_folder`.
  - `src/window/notebook_view/paint.rs`
  - `src/window/notebook_view/geometry.rs`
  - `src/window/notebook_view/input.rs`
  - `src/window/notebook_view/context_menu.rs`
  - `src/window/notebook_layout.rs`: delete the fixed `PanelLayout.root` rect; root buttons are laid out within the root row.
  - `src/window/panel_cursor.rs`: `Cursor::Root` carries an id.
  - `src/window/notebook_view/tests.rs`
  - `src/window/main_window/tests.rs`: `row_of` and `selected_kind` take `(NotebookId, RowKind)`. Add `row_in(hwnd, id, kind)` and keep `row_of(hwnd, kind)` as "in the first root" for old tests.
- Test: `src/window/notebook_view/tests.rs` and `src/window/main_window/tests/notebook_tree.rs`.

**Interfaces:**
- **Consumes:** Task 3 accessors; `library::local::display_names`.
- **Produces:**

  ```rust
  pub(crate) struct ViewRow { pub(crate) notebook: NotebookId, pub(crate) row: TreeRow }
  pub(crate) struct RootKey { id, loaded, failed, expansion, root_expanded, truncated }
  ```

  - `reveal_notebook(hwnd, id)` now selects that root row and scrolls it into view.
  - `selected_notebook(hwnd)` returns the selected row's notebook.
  - **Root rows:** `depth == 0`. The contents of a root are `depth + 1` relative to today's rows.
- **Truncation is per root.** The "more than 10,000 notes" row follows the last row of its root.

- [ ] **Step 1: Write the failing tests:**
  - **`rows_are_built_root_by_root_with_one_root_row_each`**: with A and B loaded, the rows read `[Root A, …A rows…, Root B, …B rows…]`, all tagged with their ids.
  - **`a_collapsed_root_hides_its_rows_and_remembers_it`**
    - Collapse B.
    - B's rows go away and `root_expanded_of(B)` is false.
    - After reinstalling B from disk, it is still collapsed.
  - **`an_unloaded_root_shows_one_placeholder_row`**
    - B loading gives `[Root B, Placeholder(Loading)]`.
    - B failed gives `Placeholder(Failed)`, whose text is "Couldn't load. Retry". Clicking it calls `retry_load_of(B)` only.
    - An empty notebook gives `Placeholder(Empty)`, "No notes yet".
  - **`the_default_root_shows_its_marker`**: the paint model, or a `root_label(hwnd, id)` helper, reports the marker on A only, and its tooltip text is "Default notebook".
  - **`the_root_menu_offers_close_default_and_favorite`**
    - On B, the menu items include "Close notebook", "Set as default notebook" and "Add to Favorites".
    - On A, the default, "Set as default notebook" is absent.
    - Choosing "Set as default notebook" makes B the default and writes `default=` to `folders.ini`.
  - **`two_roots_with_the_same_name_show_their_parent_hint`**: for `X\Notes` and `Y\Notes`, the root labels carry the hints `X` and `Y`.
  - **`root_buttons_act_on_their_own_root`**: New note on B's root row opens the name box for B's folder.
  - **`with_no_notebook_open_the_view_shows_the_open_a_notebook_panel`**: unchanged `NoNotebook` behaviour.

- [ ] **Step 2: Run the tests and check that they fail.**
  - Run: `cargo test --locked --target x86_64-pc-windows-msvc -- --test-threads=1 notebook_view notebook_tree`
  - Expected: FAIL.

- [ ] **Step 3: Implement.**
  - `snapshot` concatenates each slot's `library::tree::rows` output under its root row.
  - Paint and geometry draw the root row with its chevron, name, marker and buttons. Button hit areas are visible only when the row is hovered or selected.
  - Remove the fixed header layout.

- [ ] **Step 4: Update the existing row-model tests.** These are about 150 across `notebook_view/tests.rs`, `sidebar_accessibility/tests.rs`, `inline_name/tests.rs` and `main_window/tests/*`. A test that expected rows starting at the root's contents now skips one root row; prefer `row_of`/`row_in` over hard-coded indices.

- [ ] **Step 5: Run the full suite.** Expected: PASS.

- [ ] **Step 6: Commit.** `git commit -am "feat(notebook-view): one collapsible root row per open notebook"`

---

### Task 10: Keyboard, type-ahead and UI Automation for roots

**Files:**
- Modify:
  - `src/window/notebook_view/input.rs`
  - `src/window/notebook_view/accessibility.rs`
  - `src/window/sidebar_accessibility.rs`
- Test:
  - `src/window/notebook_view/tests.rs`
  - `src/window/sidebar_accessibility/tests.rs`
  - `tests/focus_and_accessibility.rs`

**Interfaces:**
- **Consumes:** Task 9's `ViewRow`, `RowKind::Root` and `Placeholder`.
- **Keys on a root row** follow the spec §4.4 table:
  - Left collapses, or stays when already collapsed.
  - Right expands, then moves to the first child.
  - Enter toggles.
  - F2 and Del do nothing.
  - Type-ahead matches root names.
- **Left on a depth-1 row** moves to its root row.

- [ ] **Step 1: Write the failing tests:**
  - **`arrow_keys_collapse_expand_and_enter_a_root`**: Left, Right and Enter on B's root row behave as listed above.
  - **`left_from_a_top_level_note_moves_to_its_root`**
  - **`f2_and_delete_on_a_root_do_nothing`**: no name box opens, and no Recycle Bin question appears.
  - **`type_ahead_matches_a_root_name`**: typing "b" selects Root B when B's name starts with b and no earlier row matches.
  - **`each_root_is_a_top_level_automation_item`**
    - The UIA tree's top-level items are named "A (default)" and "B".
    - B's folders are B's descendants.
    - A placeholder row is a text item.

- [ ] **Step 2: Run the tests and check that they fail.**
  - Run: `cargo test --locked --target x86_64-pc-windows-msvc -- --test-threads=1 notebook_view sidebar_accessibility focus_and_accessibility`
  - Expected: FAIL.

- [ ] **Step 3: Implement.**

- [ ] **Step 4: Run the full suite.** Expected: PASS.

- [ ] **Step 5: Commit.** `git commit -am "feat(notebook-view): keyboard and UI Automation for notebook roots"`

---

### Task 11: Drag and drop, and Open Editors' pins across roots

**Files:**
- Modify:
  - `src/window/tree_drag.rs`: `accepts`, `destination` and `Hover::Header`, which now carry the source and destination `NotebookId`.
  - `src/window/tree_move.rs:163`
  - `src/window/notebook_view/drag.rs`
  - `src/window/panel_drop.rs`
  - `src/window/copy_host.rs`, where `copy_into` takes the destination root from the row under the cursor.
  - `src/window/open_editors.rs`, where Pinned lists every slot's `pinned_notes`.
  - `src/window/drag_label.rs`, for the hint text.
- Test:
  - `tests/tree_and_open_editors_drag.rs`
  - `tests/copy_host_and_panel_drop.rs`
  - `tests/open_editors_and_folder_commands.rs`

**Interfaces:**
- **Consumes:** Task 9's `ViewRow`.
- **Produces:** `tree_drag::accepts(source: NotebookId, dest: NotebookId, …) -> Verdict`, which returns `Verdict::Rejected(Reason::OtherNotebook)` when the ids differ.
  - **`Verdict` and `Reason` are new.** Define them in `tree_drag.rs` *only* if `accepts` does not already return an enum with a reject reason. If one exists, add the `OtherNotebook` variant to it.

- [ ] **Step 1: Write the failing tests:**
  - **`dragging_a_note_onto_another_root_is_refused_with_a_hint`**
    - Drag A's note over B's root, then over a B folder.
    - The cursor is "no drop", the hint reads "Moving between notebooks isn't supported yet", and dropping changes no file.
  - **`dragging_within_a_root_still_moves`**: the existing behaviour, now running under two roots.
  - **`files_dropped_from_outside_copy_into_the_root_under_the_cursor`**: dropping an external file on B's root row copies it into B's folder, and B's state lists it.
  - **`pinned_editors_list_every_notebooks_pins_with_their_names`**
    - Pin one note in A and one in B.
    - The Pinned part lists both, in sidebar order, each with its notebook name as secondary text.
    - With only one notebook open, there is no secondary text.

- [ ] **Step 2: Run the tests and check that they fail.**
  - Run: `cargo test --locked --target x86_64-pc-windows-msvc -- --test-threads=1 tree_and_open_editors_drag copy_host_and_panel_drop open_editors`
  - Expected: FAIL.

- [ ] **Step 3: Implement.**

- [ ] **Step 4: Run the full suite.** Expected: PASS.

- [ ] **Step 5: Commit.** `git commit -am "feat(notes): drops stay within a notebook; pins from every notebook"`

---

### Task 12: Labels, Favorites, docs and final checks

**Files:**
- Modify:
  - `src/window/activity_bar.rs:412`: `notebook_label`.
  - `src/window/side_panel.rs:734` and :796.
  - `src/window/favorites_view.rs`
  - `README.md`: the "Notes and notebooks" section around :180, and the `folders.ini` description if it is listed.
- Test:
  - `tests/sidebar_layout.rs`
  - the existing tests in `src/window/favorites_view.rs`, or `tests/notebook_tree.rs` if it has none

**Interfaces:**
- **Consumes:** Task 3's `notebooks()`; Task 4's `reveal_notebook`.
- **Labels:**
  - `notebook_label(hwnd)` returns the single root's name when one notebook is open, or `format!("{n} notebooks")` when several are.
  - Favorites marks every favorite that is open.
  - Clicking a favorite that is already open calls `reveal_notebook` instead of `open_listed_notebook_in_view`.

- [ ] **Step 1: Write the failing tests:**
  - **`the_notebook_label_counts_open_notebooks`**: with one notebook it shows the name; with A and B it shows "2 notebooks".
  - **`favorites_highlight_every_open_notebook_and_reveal_on_click`**
    - Favorite A and B, both open.
    - Both are highlighted.
    - Clicking B selects B's root and starts no folder check: the `check_request` is unchanged.

- [ ] **Step 2: Run the tests and check that they fail.**
  - Run: `cargo test --locked --target x86_64-pc-windows-msvc -- --test-threads=1 sidebar_layout favorites`
  - Expected: FAIL.

- [ ] **Step 3: Implement.** Update the README:
  - opening a notebook adds it to the sidebar;
  - Close notebook and Set as default notebook are on each root's `…` menu;
  - create commands go to the notebook you're working in, otherwise the default;
  - quick capture uses the default.

- [ ] **Step 4: Run the full CI gate.** Run all three commands from Global Constraints. Expected: all pass.

- [ ] **Step 5: Smoke-test the real app.**
  1. Copy `native/out/x64` DLLs into `target/debug`.
  2. Launch it, and open two notebooks.
  3. Check that both trees, autosave in the second notebook, and restart all keep both notebooks.

- [ ] **Step 6: Commit.** `git commit -am "feat(notes): multi-notebook labels, favorites and README"`

- [ ] **Step 7: Clean up the build.** Run `cargo clean`, then `rm -rf native/src`. Keep `native/out` and `native/cache`.
