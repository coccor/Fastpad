# External Changes (PR 1a) Implementation Plan

> **For agentic workers:** Steps use checkbox (`- [ ]`) syntax for tracking. Each part ends with its own commit.

**Goal:** FastPad notices changes made to files outside it, as they happen. A clean tab reloads as one undoable edit. A dirty tab takes in text appended on disk, and enters a conflict state for any other change. Notes changed outside FastPad are marked in the tree.

**Spec:** `docs/superpowers/specs/2026-10-16-second-brain-phase-1-design.md` §5 (and §3, §13, §14 for 1a)

**Architecture:**
- Pure decisions (how text changed, the minimal replacement, what a tab check concludes) live in a new window-free module, `src/library/external.rs`, with unit tests.
- Tab handling lives in `src/window/library_host/external.rs`. It applies a check result to any tab, visible or hidden: visible tabs through their group editor, hidden ones through `with_background_document`.
- The watcher thread lives in `src/library/watch.rs`, the Win32 part, and is owned by `LibraryHost` like `CopyWorker`: started after a notebook's first scan, and stopped and joined on switch, close, notes mode off and `WM_DESTROY`. It posts one boxed `WM_FASTPAD_FILES_CHANGED` per 250 ms batch.
- Changed-note marks: the scan worker diffs the new scan against the cached one, the local file persists `changed=` lines, and the tree carries a `changed` flag per note.

**Tech stack:** Rust 2024, `windows-sys` 0.61.2 (existing features only: `Win32_Storage_FileSystem` and `Win32_System_IO` cover `ReadDirectoryChangesW` and `GetOverlappedResult`), Scintilla 5.6.6.

## Global constraints

- No new dependencies and no new `windows-sys` features.
- Never edit `src/editor/scintilla_constants.rs` by hand: add names to `tools/generate-scintilla-constants.ps1` and regenerate.
- Nothing new runs before first paint or first input. The watcher starts in `install` after the first scan of a notebook.
- A reload is **one undo step**. Ctrl+Z after it brings back the previous text and makes the tab dirty.
- FastPad's own writes never look like outside changes: saves update `disk_stamp` and the text base synchronously; replace-across-notes and copy register their target paths as **own writes in flight** before their workers start.
- Compile gate per part: `cargo clippy --all-targets --all-features -- -D warnings`. Run the targeted tests per part; run the full suite and every integration target once, at the end (Part 8).
- Window tests need `native/out/x64` and `-- --test-threads=1`.

## Deliberate deviations from the spec

- **Hidden tabs reload at once** (spec §5.2 says they are marked stale and reload when shown). Hidden tabs keep live Scintilla documents, and `with_background_document` already edits them, so one code path serves every tab and no "stale" state is needed. The spec gets an implementation note in Part 8.
- **Tab checks on activation cover every tab with a path**, not only visible ones. It is one stat per tab and it removes the "check when shown" trigger for outside-notebook tabs. Tab activation still checks that tab, for the case where the window never lost focus.

## Review focus

1. **Undo after reload:** Ctrl+Z restores the exact previous text and makes the tab dirty. Part 3 test `undo_after_reload_restores_the_previous_text`.
2. **Own writes:** a save, autosave, replace across notes or capture never reloads, conflicts or marks its own file. Part 6 test `own_writes_are_not_outside_changes`.
3. **Append merge keeps the user's edits:** an append to a dirty tab inserts at the end, and the next save writes both. Part 1 tests `append_is_detected_only_for_a_pure_append`; Part 3 test `append_to_a_dirty_tab_merges_without_conflict`.
4. **Hidden tabs:** a change to a tab nobody is showing reloads it correctly, its preview and markdown state are refreshed, and the dirty flag follows. Part 3 test `hidden_tab_reloads_and_stays_clean`.
5. **Watcher lifetime:** no thread outlives its notebook; switching notebooks quickly never leaks or double-posts. Part 6 test `watcher_stops_on_notebook_switch_and_close`.

---

## Part 0: Constants

- [ ] Add `SCI_SETREADONLY` and `SCI_GETREADONLY` to `tools/generate-scintilla-constants.ps1`, regenerate, check that the diff only adds those two.
- [ ] Commit: `chore: add Scintilla read-only constants`.

## Part 1: Pure change logic (`src/library/external.rs`)

**Produces:**
- `TextBase { len: usize, hash: u64 }`, `TextBase::of(text: &str) -> Self`. `len` is in bytes of the UTF-8 text. `hash` is FNV-1a 64 (no dependency, stable across runs).
- `enum DiskChange { Same, Appended { at: usize }, Other }`. `classify(base: Option<TextBase>, disk: &str) -> DiskChange`: `Same` when the length and hash match; `Appended { at: base.len }` when `disk.len() > base.len`, `disk.is_char_boundary(base.len)` and the prefix hashes to `base.hash`; otherwise `Other`. With no base the result is `Other`.
- `Replacement { start: usize, old_end: usize, new_text: String }` and `minimal_replacement(old: &str, new: &str) -> Option<Replacement>`: the longest common prefix and suffix (not overlapping), both cut back to char boundaries; `None` when the texts are equal.
- `enum TabVerdict { Unchanged, Reload, AppendMerge { at }, Conflict(ConflictKind), Deleted, Restored }` and `ConflictKind { Changed, Deleted }`. `verdict(dirty: bool, known: Option<DiskStamp>, now: Option<DiskStamp>, change: impl FnOnce() -> DiskChange, policy: ReloadPolicy) -> TabVerdict` implements spec §5.2's table. The disk is read lazily, only when the stamps differ. `ReloadPolicy { Auto, Ask }`.
- `const AUTO_RELOAD_LIMIT: u64 = 8 * 1024 * 1024`. Above it, a clean tab gets `Conflict(Changed)`.

**Tests (written first):**
- `minimal_replacement`: identical, empty to text, text to empty, insertion in the middle, a change at each edge, multi-byte characters at the boundaries (`é`, an emoji), a repeated pattern whose prefix and suffix would overlap.
- `classify`: a pure append; append plus a middle edit gives `Other`; truncation gives `Other`; the same length with different text gives `Other`; a base ending inside a multi-byte character of the disk text gives `Other`; no base gives `Other`.
- `verdict`: every row of spec §5.2, plus `Ask` policy and the 8 MB limit.

- [ ] Write the module and tests; `pub mod external;` in `src/library/mod.rs`.
- [ ] `cargo test --lib library::external`.
- [ ] Commit: `feat(library): decide how a file changed on disk`.

## Part 2: Document state

**Modify:** `src/document.rs`, `src/window/tabs.rs`, the open, save and reload paths.

- Add to `Document`: `text_base: Option<TextBase>`, `conflict: Option<ConflictKind>`, `deleted_on_disk: bool`. Include them in `PartialEq`.
- `Document::title()`: a `deleted_on_disk` tab shows `"<name> (deleted)"` before the dirty marker.
- Set `text_base` wherever `disk_stamp` is set from a load or save:
  - `opening.rs` `open_path_placed` (from `loaded.text`);
  - `autosave.rs` `document_saved` (from the editor's text, through `with_document_text`, so it costs no copy);
  - `reload_from_disk`;
  - `background_documents.rs` `reload_clean_document` (replace and copy reloads).
- Clear `conflict` and `deleted_on_disk` on a successful save and on reload.
- **Test** (`document.rs`): the title with `(deleted)`, with and without the dirty marker.
- [ ] Commit: `feat: remember each tab's text base and disk state`.

## Part 3: Tab checks and edit-style reload (`src/window/library_host/external.rs`)

**Produces:**
- `check_tab(hwnd, id) -> TabVerdict`: stats the file, reads it with `loader::load` only when the stamps differ, refuses NUL text like `reload_from_disk` does, and applies the verdict.
- `check_all_tabs(hwnd)` and `check_paths(hwnd, &[PathBuf])`.
- `apply_reload(hwnd, id, loaded, stamp)`:
  - compute `minimal_replacement` against the tab's current text;
  - in the editor showing it (`editor_showing`), or else inside `with_background_document`, do `begin_undo_action`, `replace_target`, `end_undo_action`, `set_save_point`;
  - for a hidden document: `tabs.note_background_edit(id)`, `tabs.set_dirty(id, false)`, `markdown_host::forget`;
  - for a visible one: `preview_host::document_reloaded`;
  - set `encoding`, `disk_stamp`, `text_base`; set `autosave_paused = false`; clear `conflict` and `deleted_on_disk`;
  - `remove_saved_document_snapshots` if it was the active tab;
  - set the status hint "Reloaded: changed outside FastPad" (Part 4).
- `apply_append(hwnd, id, loaded, at, stamp)`: inserts `loaded.text[at..]` at the end of the buffer as one undo step, keeps the tab dirty, updates `disk_stamp` and `text_base`, and sets the status hint "Added text from outside FastPad".
- `apply_conflict(hwnd, id, kind)`: sets `conflict` and `autosave_paused`, refreshes the title strip, and posts one notice per conflict (no repeat while the conflict stands).
- `apply_deleted` and `apply_restored`: toggle `deleted_on_disk`, refresh the views.

**Wiring:**
- `wndproc.rs` `WM_ACTIVATEAPP` (activate): `check_all_tabs`, next to `image_host::check_disk`.
- `document_tabs.rs` `activate_document_in`, after `show_group_view`: `check_tab` for the newly shown tab.
- `autosave_active`: a tab with `conflict` set never autosaves. A changed stamp now goes through `check_tab` instead of pausing directly, so a pure append merges and a clean tab reloads.
- `reload_from_disk` (`NoteReloadFromDisk`) uses `apply_reload` (undoable). `keep_mine` clears `conflict`.
- Setting `reload_changed_files = auto | ask` in `config/persisted.rs`, default `auto`, accepting the usual spellings, plus its Settings row.

**Tests:**
- Unit tests in `config/persisted.rs` for the setting.
- Window tests in a new `tests/windows/external.rs` (with a `[[test]]` entry):
  - `clean_tab_reloads_on_activation`;
  - `undo_after_reload_restores_the_previous_text`;
  - `append_to_a_dirty_tab_merges_without_conflict`;
  - `middle_edit_to_a_dirty_tab_conflicts_and_autosave_pauses`;
  - `deleted_file_marks_the_tab_and_save_recreates_it`;
  - `hidden_tab_reloads_and_stays_clean`;
  - `ask_policy_turns_a_reload_into_a_conflict`.
- [ ] Commit: `feat: reload tabs changed outside FastPad as an undoable edit`.

## Part 4: Status hint and conflict mark

- **Status hint:** `App` gains `status_hint: Option<(String, Instant)>`. `current_status_bar` (`chrome.rs`) shows it on the left when no notification is pending, the same way it shows `preview.status_hint()`. `STATUS_HINT_TIMER_ID` clears it after 4 s and repaints.
- **Conflict mark in tabs:** `tab_snapshot` and `StripPaint` gain `conflicts: Vec<bool>`. `group_strip::draw` draws a warning glyph (`\u{26A0}` in `palette.warning_foreground`, or the system text color in high contrast) before the title of a tab in conflict. `view_tabs` appends ", changed on disk" to the accessible title.
- **Open Editors:** conflict rows draw the dot in `warning_foreground` and add ", changed on disk" to the name.
- **Tests:** a unit test for the accessible title; a `group_strip` layout test that the glyph doesn't move the close button.
- [ ] Commit: `feat: show conflicts in tabs and a transient status hint`.

## Part 5: Compare with disk

- Add `CommandId::NoteCompareWithDisk` (the next free number), its keymap id `note.compareWithDisk`, its palette entry "Note: Compare with disk", its `TEXT_COMMANDS` entry and its dispatch, updating the array lengths and counts in their tests.
- Behavior: for the active tab with a path, load the disk version, then:
  - `split_group(hwnd, active, Direction::Right)`, and on no room use the active group;
  - create a document with **no path**, `untitled_label = "<name> (on disk)"` and the same language, populated with `populate_clean`, then `SCI_SETREADONLY 1`;
  - activate it in the new group.

  The pathless document is never autosaved, and session restore skips it: it is marked `compare: true` on `Document`, and `session.rs` and the recovery snapshots skip documents with that flag. Closing it never prompts.
- **Tests:** a window test, `compare_with_disk_opens_a_read_only_split`. It checks that typing doesn't change the text and that closing it doesn't prompt.
- [ ] Commit: `feat: compare a conflicting tab with the file on disk`.

## Part 6: The notebook watcher

**Create:** `src/library/watch.rs` for the thread and the Win32 calls, and `src/window/library_host/watch.rs` for the host.

**`library::watch`:**
- `Watcher::start(folder: PathBuf, target: isize, message: u32, generation: u64) -> io::Result<Watcher>`.
  - It opens the folder with `FILE_LIST_DIRECTORY`, `FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED`, and full sharing.
  - It spawns a thread with a 64 KB buffer that loops: `ReadDirectoryChangesW` (recursive, filter `FILE_NAME | DIR_NAME | LAST_WRITE | SIZE`), then `WaitForMultipleObjects([io_event, stop_event])`, then `GetOverlappedResult`, then it parses `FILE_NOTIFY_INFORMATION` records.
  - It collects for 250 ms after the last event (a timed wait on the same handles), then posts `Box<FileChanges { generation, paths: Vec<PathBuf>, names_changed: bool, overflow: bool }>` and frees the box if the post fails.
- `skip_by_name(relative: &Path) -> bool` drops paths under a component the scan skips (`scan::skip_directory`), except `.fastpad\library.ini` and `.fastpad\notebook.ini`.
- A zero-byte completion means overflow: it posts `overflow: true`.
- A failure of `ReadDirectoryChangesW` ends the thread after posting `failed: true` once.
- `Drop` sets the stop event, cancels the I/O (`CancelIoEx`), and joins.
- **Unit tests:** `skip_by_name`; parsing a hand-built `FILE_NOTIFY_INFORMATION` buffer with two records, including a name with non-ASCII characters; and a real-folder test that writes a file and receives one batch within 2 s.

**Host:**
- `LibraryHost.watcher: Option<Watcher>`. It starts in `load.rs` `install` when `first_time` is true and notes mode is on, and is dropped:
  - in `open_checked_folder`, `close_notebook` and `notes_mode_changed(false)`;
  - in `WM_DESTROY` (`library_host::stop_watcher`), next to `copy_host::stop`.
- New `WM_FASTPAD_FILES_CHANGED = WM_APP + 20`, handled in `wndproc.rs` with the boxed handlers. The generation must match `host.generation`, as with `LIBRARY_READY`.
  - `failed`: drop the watcher and keep activation rescans (`watcher_failed = true`, which gates nothing else).
  - `overflow`: `request_rescan` and `check_all_tabs`.
  - Otherwise: drop the paths in `host.own_writes`, then `check_paths` for open tabs, then mark changed notes (Part 7), then `request_rescan` when `names_changed`.
  - Add the message to the boxed-message list in the `messages.rs` test.
- **Own writes in flight:** `LibraryHost.own_writes: HashSet<PathBuf>` (lowercase keys).
  - `text_search_host::spawn_write` inserts its targets before spawning and removes them in `replace_written`; `copy_host` does the same around its copies.
  - A path is also removed 2 s after its writer's result lands (`OWN_WRITE_GRACE`), so the watcher's late event for it is still ignored. This uses a timer, not a sleep.
- With a watcher running, `activation_changed` no longer rescans after 5 s, but `check_all_tabs` still runs.
- **Window tests** (in `tests/windows/external.rs`):
  - `outside_write_reloads_an_open_note_within_a_second` (no activation change);
  - `git_writes_cause_no_rescan` (writes under `.git\`; checks that the scan generation is unchanged);
  - `own_writes_are_not_outside_changes` (a save, an autosave and a replace across notes, each followed by 1 s of watching with no reload, conflict or mark);
  - `watcher_stops_on_notebook_switch_and_close`.
- [ ] Commit: `feat: watch the open notebook for outside changes`.

## Part 7: Changed-note marks

**Library (pure):**
- `LocalState` gains `changed: Vec<PathBuf>` (relative), encoded as `changed=<path>` lines and parsed as such, and included in `Conveniences` so a change gets written by the UI.
- `library::load` diffs the scan against the previous cache, `local.files`, before reconcile replaces it: an entry is changed when its path is in the cache with a different size or mtime, or new when the cache was non-empty and the path isn't in it. Paths in `touched` are skipped. The result goes into `LibraryState.changed_on_scan`.
- A note renamed outside FastPad, from `relocated` and file-ID matches, is marked under its new path. On a first scan (empty cache) nothing is marked.
- `LibraryState::mark_changed(path)`, `clear_changed(path)` and `clear_all_changed()` keep `local.changed` and the tree in step. Removing or renaming a note drops or moves its mark.
- `NoteTree` `Note` gains `changed: bool`, `TreeRow` gains `changed: bool`, and there is `NoteTree::set_changed(path, bool)`. `NoteTree::build` takes the changed set alongside pins. Folder rows gain `holds_changed: bool`, computed in `rows()` for collapsed folders.
- **Unit tests:** local file round trip with `changed=`; the scan diff (changed size, changed mtime, a new file, a first scan, a touched path, a rename); the tree flags and `holds_changed`.

**Window:**
- `FILES_CHANGED` marks notes whose stat differs from their `NoteEntry` (after own writes are filtered out) and updates the entry's size and mtime. `install` applies `changed_on_scan`.
- Opening a note in a tab clears its mark (`open_path_placed`, `activate_document_in`). So does `check_tab` reloading a visible tab.
- Painting (`notebook_view/paint.rs`): a changed note draws a `\u{25CF}` dot after its name, in `palette.accent`, measured so the name's ellipsis leaves room for it. A collapsed folder with `holds_changed` draws the dot in `muted_foreground`. The pin box is unaffected.
- Accessibility: `sidebar_accessibility::tree_item` appends ", changed outside FastPad" to the name.
- Commands (each with a `CommandId`, keymap id, palette entry and dispatch; no default keys):
  - `NotesShowChanged`, "Notes: Show changed notes": toggles a filter in `NotebookView` that keeps marked notes and their ancestor folders. The filter is part of `RebuildKey`, and the header shows "Changed notes · Show all" while it is on.
  - `NotesClearChanged`, "Notes: Clear changed marks".
- **Window tests:** `outside_write_marks_the_note_until_opened`; `marks_survive_a_restart`; `show_changed_notes_filters_the_tree`.
- [ ] Commit: `feat: mark notes changed outside FastPad`.

## Part 8: Finish

- [ ] README: a "Changes made outside FastPad" subsection under Notes, the `reload_changed_files` row in the settings table, and the new palette commands.
- [ ] Spec: an implementation-notes section (§17) recording the two deviations above and anything else decided while building.
- [ ] Full suite: `cargo test --all-features -- --test-threads=1`, then each window integration target.
- [ ] Manual check in the running app (`run` skill):
  - edit an open note from a PowerShell `Add-Content` and `Set-Content` while FastPad shows it;
  - delete it, restore it;
  - make a conflict and use Compare with disk;
  - run `git commit` in the notebook.
- [ ] `fastpad-bench`: warm TTI and first paint stay within noise with a notebook open and the watcher running.
- [ ] Commit: `docs: external changes README and implementation notes`.
