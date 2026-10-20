# Live Markdown Editor, Phase 4: Integration — Implementation Plan

> **For agentic workers:** Steps use checkbox (`- [ ]`) syntax for tracking. Work task by task; each task ends compiling, with its named tests green, and is committed on its own.

**Goal:** Live works with the rest of FastPad as the source view does:
- **Find:** find next, find previous, replace and go to line move Live to the match and reveal its block, from every entry point, including Enter in the find bar.
- **Split:** with Live on the left, scrolling either side scrolls the other, past headings, tables and images, without echo loops.
- **Commands:** every command is checked in Live. It either works, or it switches to the source view with a notice before it runs.
- **Context menu and drag-and-drop:** Live has the source view's right-click menu, accepts dropped text, and moves or copies a dragged selection.
- **Images:** a note saved for the first time picks up its folder for relative images at once.
- **Accessibility:** the Live window answers `WM_GETOBJECT` with an MSAA editable-text object.
- **Perf:** the Live typing scenarios run in CI.

After this phase, PR #72 is ready to merge.

**Spec:** `docs/superpowers/specs/2026-10-18-markdown-live-editor-design.md`: §7.2 (caret and selection follow Scintilla), §7.3 (commands in Live), §8 (mouse), §9 (accessibility), §10 (performance), §11 (find, Split and IME tests) and §12 phase 4. Builds on phases 1–3 and keeps their deviations.

**Tech stack:** as phase 3. Also used:
- `window::accessibility` (`AccessibleVtable`, `allocate_bstr`, `RawVariant`, `LresultFromObject`), as `preview/accessible.rs` uses it;
- `menus::track_popup` for the context menu;
- `platform::ole_drop` and `editor::file_drop::accept_file_drops_on` for drops.

## Where things stand

These findings come from the survey made for this plan:

1. **Find already reaches Live** *(corrected in Task 1)*.
   - The survey reported a gap. Enter in the find bar, its buttons and go to line all run outside `execute_command`, and it assumed a hidden Scintilla sends no `SCN_UPDATEUI`.
   - That last part is wrong. A hidden Scintilla queues `SCN_UPDATEUI` as idle work: it posts a message, which runs before the next paint. `wndproc.rs` polls Live from it, so Live follows find, replace, go to line and link jumps.
   - Task 1 confirmed this. Its tests pass without any extra poll.
2. **Find draws no highlights in either view.** The source view shows the current match as its selection, and nothing else. The only indicator FastPad fills is the link underline.
3. **Split sync works one way.**
   - **Live → preview works.** `live_host::poll(scrolled)` sets the hidden editor's top line and calls `preview_host::editor_scrolled`.
   - **Preview → Live is missing.** `follow_preview` scrolls only the hidden Scintilla, and `LiveView::set_top_line` has no production caller.
   - **Echo guard:** `follow_preview` sets `scroll_origin = Preview` and waits for the echo `SCN_UPDATEUI`. Per finding 1, a hidden editor does send it, from idle work, so the guard is probably cleared. Task 2 checks this with a test rather than assuming it.
4. **Commands.**
   - **Multiple selections** (Ctrl+D, select all occurrences, add cursor above/below, Alt+Shift+arrows) run first. `poll` then sees more than one selection and switches to source with a notice.
   - **A one-row rectangular selection** reports one selection, so it stays in Live.
   - **Fold commands** fold the hidden editor, and Live ignores the folds.
   - **Word wrap, show whitespace and highlight current line** change only the source view.
   - **Wikilink completion** is off in Live (phase 1 deviation 4).
5. **No context menu.**
   - The source view uses Scintilla's built-in popup: Undo, Redo, Cut, Copy, Paste, Delete, Select All.
   - Live handles no `WM_CONTEXTMENU`, so a right-click does nothing.
6. **Drag-and-drop.**
   - Live registers no drop target. A dropped file probably falls through to the group window's target. Dropped text is refused.
   - A press inside the selection always starts a new selection, so a selection cannot be dragged.
7. **Image folder.** It is set only in `live_host::show_text`. A Save As reaches it indirectly, through `WM_FASTPAD_APPLY_LANGUAGE` → `apply_language` → `sync_visibility`. That path skips the update when `apply_language` fails.
8. **Accessibility.** Live answers only `WM_GETTEXT` / `WM_GETTEXTLENGTH`, and keeps the system caret in place. `WM_GETOBJECT` falls to `DefWindowProcW`.
9. **Perf.**
   - `src/perf` holds only the startup diagnostic protocol. `fastpad-bench` measures startup only.
   - The seven `typing_in_live` tests in `tests/windows/markdown_live.rs` are `#[ignore]`d, and CI never runs them.
10. **Present.** Each frame is drawn in the dirty band, but `EndDraw` on the `ID2D1HwndRenderTarget` (with `RETAIN_CONTENTS`) presents the whole target.

## Decisions this plan makes

1. **Live follows the selection wherever it changes.** It already does (finding 1), so Task 1 only adds tests that keep it so. A match then shows as revealed source, selected, in the middle of its block's formatting. With focus in the find bar, Live draws the match in the inactive selection colour, which it already does.
2. **Find highlights only the current match, as the source view does.** The spec's "find highlights the match" is met by the selection on revealed source. Highlighting every match is a feature of neither view, and it stays out of scope (a later change would add it to both).
3. **Preview → Live sync.**
   - `follow_preview` checks `live_host::view_shown(hwnd, id)`. When Live is shown, it calls a new `LiveView::follow_line(line)`, which:
     - sets the top line from the preview's document line, with no fold mapping (Live shows every line);
     - invalidates the view;
     - does **not** post `WM_FASTPAD_LIVE_CHANGED`. That keeps the scroll from echoing back to the preview.
   - It then sets the hidden editor's first visible line, so a toggle back to source keeps the place. It never sets `scroll_origin`.
   - Live → preview stays as it is. Its echo, the preview scrolling programmatically, is already absorbed by the preview's own guard.
   - A test proves no loop: scroll either side, pump messages, and both top lines stay put.
4. **Commands that need Scintilla's own UI switch to source before they run.**
   - `live_host::needs_source(CommandId) -> bool` is a pure match that names:
     - `AddNextOccurrence`, `SelectAllOccurrences`, `AddCursorAbove` and `AddCursorBelow`;
     - `ToggleCodeFolding`, `FoldAll` and `UnfoldAll`.
   - `execute_command_with_note` checks it first. In Live, it switches the group to source with the notice "*Command* needs the source view. Switched to the source view.", then runs the command on the visible editor.
   - The existing multi-selection check in `poll` stays as the safety net for keys forwarded straight to Scintilla (Alt+Shift+arrows). It is widened to `selection_count > 1 || rectangular` through a new `Editor::is_rectangular_selection` (`SCI_GETSELECTIONMODE`).
   - **View toggles:**
     - **Word wrap and show whitespace:** Live always wraps and never shows whitespace, since it is rendered text. The commands still change the source view, and their menu checks stay.
     - **Highlight current line:** this does drive Live's active-block fill. It is the same "where am I" cue, and `LiveColors.active_block` already comes from the current-line colour.
   - **Wikilink completion** stays off in Live, as phase 1 decided. Switching views on `[[` would be worse than typing the link in full. This is recorded as a deviation.
   - **The audit is a test.** For every command in `EDITING_COMMANDS` and `TEXT_COMMANDS`, run it in Live on a fixture note, then check one of these:
     - for commands in `needs_source`: the view is source and the notice was posted;
     - for every other command: Live is still shown, Live's selection equals the editor's, and the text equals what the same command gives in the source view.
5. **Context menu: Scintilla's items, plus link items.**
   - Right-click outside the selection moves the caret there first, as Scintilla does. Right-click inside the selection keeps it.
   - Shift+F10 and the Menu key open the menu at the caret.
   - **Items:**
     - Undo, Redo;
     - Cut, Copy, Paste, Delete;
     - Select All, each greyed as Scintilla greys it.
   - **Over a link or wikilink**, it adds "Open link" (the Ctrl+click path) and "Copy link address" (the destination as written, or the wikilink's target).
   - **Plumbing:** the menu is built with `menus::track_popup`. New `CommandId`s are added only for the two link items, and they act on the link stored when the menu opened.
6. **Drag-and-drop.**
   - **Drop in:** Live registers its own OLE target, which wraps `accept_file_drops_on`'s target:
     - Files go to the existing open-file callback.
     - Text (`CF_UNICODETEXT`) is inserted at the hit-tested drop point as one undo action. The drop point is shown as a caret while dragging.
   - **Drag within Live:**
     - A press inside a non-empty selection, then a move past the system drag threshold, starts a text drag.
     - Release moves the text as one undo action. Ctrl copies it instead.
     - Releasing inside the selection, or pressing Esc, cancels the drag and places the caret, as a click does.
     - This runs on Live's own mouse capture, not OLE.
   - **Out of scope:** dragging out of Live to another app, which needs an `IDataObject` / `IDropSource` FastPad does not have. This is recorded as a deviation.
7. **The document folder follows saves.** `library_host::document_saved` and the Save As path call `live_host::document_moved(hwnd)`. That re-reads `group_document` for each group showing the document and calls `set_document_dir`, so the update no longer depends on `apply_language` succeeding.
8. **MSAA object.**
   - `src/live/view/accessible.rs` follows `preview/accessible.rs`. It provides one object with no children:
     - `ROLE_SYSTEM_TEXT`;
     - name "Live Markdown";
     - value: the document text (`get_accValue`, from the editor, as `WM_GETTEXT` reads it);
     - state `FOCUSABLE`, plus `FOCUSED` with focus;
     - location: the window rectangle;
     - `accDoDefaultAction` focuses the view.
   - `put_accValue` returns `E_NOTIMPL`. Edits go through typing.
   - **Window events:**
     - `NotifyWinEvent(EVENT_OBJECT_VALUECHANGE)` after a flush that changed text, at most once per frame;
     - `EVENT_OBJECT_FOCUS` on `WM_SETFOCUS`.
   - The system caret keeps its `EVENT_OBJECT_LOCATIONCHANGE`, which Windows sends.
   - UIA `TextPattern` stays the spec's follow-up.
9. **Perf in CI.**
   - The spec's "a Live typing scenario added to `src/perf`" does not fit: `src/perf` is the startup protocol.
   - Instead, a new `live-perf` CI job builds the release test binary and runs `cargo test --release --test markdown_live typing_in_live -- --ignored --test-threads=1`.
   - The budget is scaled by `FASTPAD_PERF_SCALE` (default `1.0`). CI sets `2.5`, because hosted runners are slower and noisier than the reference machine; a fixed 4 ms there would flake.
   - Each test prints its Live and source p95s. The job appends them to `$GITHUB_STEP_SUMMARY`, so regressions show as a trend.
   - A Split-sync scenario joins them: `typing_in_live_with_split_preview_stays_in_budget`.
10. **Partial present is measured, not built.**
    - A true dirty-rect present needs a DXGI flip-model swap chain and `ID2D1DeviceContext`, which means new `windows` features and a second render path. Typing is already 2.0–2.8 ms against a 4 ms budget.
    - This phase adds an `EndDraw` timing to `take_timings` and records the present's share in the spec note.
    - If the present is more than half the keystroke, the swap chain becomes a follow-up after the merge.

## Global constraints

- **No new crates.**
  - The only `windows-sys` feature change allowed is one the MSAA object needs. `Win32_UI_Accessibility` and `Win32_System_Ole` are already on, so none is expected.
  - No `windows` crate feature changes.
- **Lazy graphics:** `assert_no_preview_imports()` and `launching_with_a_markdown_file_loads_no_preview_graphics_library` keep passing. The accessible object, the context menu and the drop target are created with the view, never before.
- **Revealing never edits.** The only edits Live makes on its own are:
  - the checkbox click (phase 3);
  - a text drop;
  - a drag move or copy.

  Each is one undo action.
- **Leaving Live is always announced:** every switch to source posts a notice and keeps text, caret, selection and undo.
- **Typing latency:** Live p95 ≤ 4 ms on the phase 3 scenarios and on the Split scenario.
- **Tests:** compile with `cargo clippy --all-targets` during tasks, and run only the tests named in each step. The full suite runs once, in Task 9.
  - Window tests run with `--test-threads=1`.
  - Perf tests always run with the `typing_in_live` filter.
  - `tabs::window_close_reviews_dirty_tabs_in_order_and_cancel_aborts_shutdown` fails on `main` as well; it is not this phase's to fix.
- Any run of the real app is wrapped in a backup and restore of `%APPDATA%\FastPad\fastpad.ini`.
- Work continues on `feat/markdown-live-editor` (PR #72). Commit messages carry no `Co-Authored-By` trailer. The PR is merged only on the user's word, after Task 9.

## Deviations from the spec (decided while planning)

1. **Only the current match is highlighted** (Decision 2), as in the source view.
2. **Wikilink completion stays off in Live** (Decision 4). The spec would switch to source for the popup.
3. **Word wrap and whitespace toggles do not apply to Live** (Decision 4).
4. **Text can be dragged within Live but not out of it** (Decision 6).
5. **The perf scenario is a CI job over the `typing_in_live` tests,** with a scaled budget, not an `src/perf` scenario (Decision 9).
6. **No partial present** (Decision 10). Phase 1's note named it phase 4 work; it moves to a measured follow-up.

## Review focus

1. **Live never falls behind the editor.** Every selection-moving entry point polls. *(Task 1: `find_from_the_find_bar_moves_live_to_the_match`, `go_to_line_moves_live`, the grep audit listed in the commit message.)*
2. **No scroll echo loops.** No stale guard swallows a later sync. *(Task 2: `scrolling_the_split_preview_scrolls_live`, `split_sync_settles_after_scrolling_either_side`.)*
3. **No command silently does nothing in Live.** *(Task 3: `every_text_command_works_in_live_or_switches_to_source`.)*
4. **Drops and drags are single undo actions,** and a cancelled drag edits nothing. *(Task 5.)*
5. **The MSAA object never outlives the view,** and no call crashes after the window is destroyed. *(Task 7: `the_accessible_object_survives_closing_live`.)*

---

## File structure

| File | Status | Responsibility |
|---|---|---|
| `src/window/main_window/find.rs`, `.../go_to_line` callers | modify | poll Live after moving the selection |
| `src/window/preview_host.rs` | modify | preview → Live sync; no stale guard |
| `src/live/view.rs` | modify | `follow_line`, inactive selection, drag state, accessible object, value-change events |
| `src/window/live_host.rs` | modify | `needs_source`, rectangular check, `document_moved`, current-line toggle, context menu commands |
| `src/window/main_window/command_dispatch.rs` | modify | switch-to-source before source-only commands; link menu commands |
| `src/editor/scintilla/*` | modify | `is_rectangular_selection` |
| `src/window/commands.rs` | modify | `LiveOpenLink`, `LiveCopyLinkAddress` |
| `src/live/view/menu.rs` | create | context menu entries and hit |
| `src/live/view/mouse.rs` | modify | right-click caret, drag-to-move |
| `src/live/view/drop.rs` | create | OLE drop target for text and files |
| `src/live/view/accessible.rs` | create | MSAA object |
| `src/live/view/window_proc.rs` | modify | `WM_CONTEXTMENU`, `WM_GETOBJECT`, drop registration |
| `src/window/library_host/autosave.rs`, `src/window/main_window/saving.rs` | modify | `document_moved` on save |
| `tests/windows/markdown_live.rs` | modify | find, Split, command audit, menu, drop, MSAA, perf tests |
| `.github/workflows/ci.yml` | modify | `live-perf` job |
| `README.md`, the spec | modify | phase 4 note; Live section complete |

---

### Task 1: Live follows find, replace and go to line

**Files:** `tests/windows/markdown_live.rs`, `src/window/mod.rs` (test re-exports), and a comment in `src/window/live_host.rs`.

- [x] Audit the call sites that move the hidden editor's selection outside `execute_command`: `select_match`, `replace_current`, `replace_all_matches`, `go_to_line`, the links view, heading jumps and daily notes. Each one's selection change reaches Live through the deferred `SCN_UPDATEUI` (finding 1), so no poll is added.
- [x] Tests:
  - `find_from_the_find_bar_moves_live_to_the_match`: Enter in the query field, then F3.
  - `find_previous_and_replace_follow_in_live`
  - `replace_all_keeps_live_in_step`
  - `go_to_line_moves_live`

  Each checks that the match is drawn on screen, that its block is revealed, and that focus stays in the find field.
- [x] Commit `test(live): Live follows find, replace and go to line`.

### Task 2: Split sync from the preview to Live

**Files:** `src/window/preview_host.rs`, `src/live/view.rs`, `src/window/live_host.rs`.

- [ ] Add `LiveView::follow_line(&self, line: usize)`. It runs `prepare`, sets `scroll_y` to `layout.line_top(line)` clamped, and invalidates the view. It posts nothing. `set_top_line` becomes this function if it has no other caller.
- [ ] `live_host::follow_preview(hwnd, id, line) -> bool` returns `false` when the group has no shown Live view.
- [ ] `preview_host::follow_preview` changes:
  - It calls `live_host::follow_preview` first. When that returns `true`, it sets the hidden editor's first visible line without setting `scroll_origin`, and returns.
  - The source path is unchanged.
- [ ] Tests:
  - `scrolling_the_split_preview_scrolls_live`: a note with headings, a table and a tall image. Scrolling the preview to each heading puts that heading's line at Live's top, within one line.
  - `split_sync_settles_after_scrolling_either_side`: scroll Live, pump, scroll the preview, pump, scroll Live again. Each sync happens (`sync_count` increments). Afterwards, neither top line moves on further pumping.
  - `toggling_to_source_after_a_preview_scroll_keeps_the_place`
  - The existing `scrolling_live_scrolls_the_hidden_editor_and_the_split_preview` keeps passing.
- [ ] Commit `feat(live): scroll Live from the Split preview`.

### Task 3: Command audit

**Files:** `src/window/live_host.rs`, `src/window/main_window/command_dispatch.rs`, `src/editor/scintilla/*`.

- [ ] Add `live_host::needs_source(CommandId) -> bool` (Decision 4), with a unit test that names each command it covers.
- [ ] Before `run_command`, `execute_command_with_note` checks two things: whether the active group shows Live, and whether `needs_source(command)`. When both hold, it calls `live_host::leave_for(hwnd, command)`. That function:
  - turns Live off for the group;
  - posts the notice with the command's menu label;
  - shows the source view with the caret and the top line kept.
- [ ] Add `Editor::is_rectangular_selection()` (`SCI_GETSELECTIONMODE != SC_SEL_STREAM`). `poll`'s switch condition becomes `selection_count > 1 || rectangular`, with a notice for each case.
- [ ] `ToggleHighlightCurrentLine` drives the active-block fill. `appearance()` reads the setting, and `refresh_appearance` runs after the toggle.
- [ ] Tests:
  - `fold_all_switches_to_source_with_a_notice`
  - `a_one_row_rectangular_selection_switches_to_source`
  - `turning_off_current_line_highlight_drops_the_block_fill`
  - `every_text_command_works_in_live_or_switches_to_source`: a table-driven test over `EDITING_COMMANDS` and `TEXT_COMMANDS`, as Decision 4 describes. Commands that open a dialog are answered through the existing test hooks, or listed as skipped with a reason in the test.
- [ ] Commit `feat(live): command audit; source-only commands switch with a notice`.

### Task 4: Context menu

**Files:** `src/live/view/menu.rs` (new), `src/live/view/mouse.rs`, `src/live/view/window_proc.rs`, `src/window/commands.rs`, `src/window/main_window/command_dispatch.rs`.

- [ ] **Right button down:** if the press is outside the selection, it places the caret as a click does, without Shift. Inside the selection, nothing changes.
- [ ] **`WM_CONTEXTMENU`:** the menu shows at the cursor, or at the caret's rectangle for a keyboard invocation (`lparam == -1`).
  - **Building the menu:** `menu::entries(state)` returns the menu: Undo, Redo, separator, Cut, Copy, Paste, Delete, separator, Select All.
  - **Greying:** each entry is greyed from the editor (`can_undo`, `can_redo`, the selection being non-empty, `can_paste`).
  - **Over a link:** a link hit adds a separator and two items, Open link and Copy link address.
  - **Running the command:** the chosen `CommandId` goes through `execute_command`.
- [ ] **Delete:** `Delete` uses the existing clear-selection command; if none exists, add `EditDelete` to the menu only.
- [ ] Add `LiveOpenLink` and `LiveCopyLinkAddress`. They act on the link `LiveView` stored when the menu opened.
- [ ] Tests (using `POPUP_ANSWERS`):
  - `right_click_outside_the_selection_moves_the_caret`
  - `right_click_inside_the_selection_keeps_it`
  - `context_menu_copy_and_paste_work_in_live`
  - `context_menu_over_a_link_copies_its_address`
  - `shift_f10_opens_the_menu_at_the_caret`
  - `undo_is_greyed_with_nothing_to_undo`
- [ ] Commit `feat(live): context menu`.

### Task 5: Drag-and-drop

**Files:** `src/live/view/drop.rs` (new), `src/live/view/mouse.rs`, `src/live/view/window_proc.rs`.

- [ ] **Drop target:** on create, Live registers a target built on `ole_drop` plumbing, and revokes it on destroy.
  - **Files:** handed to the same callback `accept_file_drops_on` uses for the group.
  - **`CF_UNICODETEXT`:**
    - **While dragging:** `DragOver` hit-tests the point, draws a drop caret, and autoscrolls near the edges.
    - **On drop:** the text is inserted at that point as one undo action, and selected afterwards.
  - **Anything else:** `DROPEFFECT_NONE`.
- [ ] **Drag within Live:**
  - A press inside a non-empty selection records a pending drag instead of placing the caret.
  - Moving past `SM_CXDRAG` / `SM_CYDRAG` starts the drag:
    - a drop caret follows the cursor;
    - the cursor shows the move (or copy, with Ctrl) arrow.
  - **Release outside the selection:**
    - **Move:** one undo action that deletes the source and inserts it at the target. The target is adjusted when it lies after the source.
    - **Copy:** an insert.
    - The moved or copied text ends up selected.
  - **Release inside the selection without moving:** a plain click.
  - **Esc or losing capture:** cancels with no edit.
- [ ] Tests:
  - `dropping_text_inserts_it_at_the_point_in_one_undo_step`, through the test hook `file_drop` already offers for drops
  - `dropping_a_file_on_live_opens_it`
  - `dragging_a_selection_moves_it`
  - `ctrl_drag_copies`
  - `a_drag_released_inside_the_selection_places_the_caret`
  - `escape_cancels_a_drag`
- [ ] Commit `feat(live): text drag-and-drop`.

### Task 6: The image folder follows saves

**Files:** `src/window/live_host.rs`, `src/window/library_host/autosave.rs`, `src/window/main_window/saving.rs`.

- [ ] Add `live_host::document_moved(hwnd)`. For every group whose Live view shows the saved document, it reads the document's folder from `preview_host::group_document` and calls `set_document_dir`.
- [ ] Call it from `document_saved` and after a successful Save As in `save_active_to`. Also call it from the name box's first save and from rename, if those don't reach `document_saved`.
- [ ] Tests:
  - `saving_an_untitled_note_shows_its_relative_images`: an untitled note in Live with `![x](pic.png)`, where `pic.png` sits in the folder it is then saved to. After saving, it shows a thumbnail with no toggle.
  - `renaming_into_another_folder_updates_images`
- [ ] Commit `fix(live): pick up the image folder when a note is saved`.

### Task 7: MSAA object

**Files:** `src/live/view/accessible.rs` (new), `src/live/view/window_proc.rs`, `src/live/view.rs`.

- [ ] **The object:** an `IAccessible` built on `window::accessibility` (Decision 8).
  - It holds a weak handle to the view state. After the view is destroyed, every method returns `CO_E_OBJNOTCONNECTED`.
  - `WM_GETOBJECT` with `OBJID_CLIENT` returns it through `LresultFromObject`.
- [ ] **Events:**
  - `EVENT_OBJECT_VALUECHANGE` after a flush that applied edits, once per flush;
  - `EVENT_OBJECT_FOCUS` on `WM_SETFOCUS`.
- [ ] Tests (through `AccessibleObjectFromWindow`, as the preview's accessibility tests do):
  - `live_exposes_an_editable_text_object`: role text, name, a focused state while focused, and a value equal to the document text.
  - `the_accessible_value_follows_typing`
  - `the_accessible_object_survives_closing_live`: the object is held, Live is toggled off, and calls return the error rather than crashing.
- [ ] Commit `feat(live): MSAA editable-text object`.

### Task 8: Perf in CI and the present's cost

**Files:** `tests/windows/markdown_live.rs`, `src/live/view.rs`, `src/live/view/paint.rs`, `.github/workflows/ci.yml`.

- [ ] `LIVE_KEYSTROKE_BUDGET_MICROS` is multiplied by `FASTPAD_PERF_SCALE`, when that is set and parses as a float of 1.0 or more. Each perf test prints one line `live-perf <name> live_p95_us=<n> source_p95_us=<n>`.
- [ ] Add `typing_in_live_with_split_preview_stays_in_budget`: the 1 MB sample in Split with Live on the left, typing in the middle.
- [ ] Record `present` (the `EndDraw` call) in `take_timings`. The 10k-line test prints its share of the keystroke.
- [ ] CI: add a `live-perf` job on `windows-latest`. It needs `windows-integration`, sets `FASTPAD_PERF_SCALE=2.5`, runs the filtered release command, and greps the `live-perf` lines into `$GITHUB_STEP_SUMMARY`.
- [ ] Run the perf tests locally with `CARGO_TARGET_DIR=target/perf` and record the numbers for Task 9's note.
- [ ] Commit `ci(live): run the Live typing scenarios`.

### Task 9: Docs, full suite, manual check

**Files:** `README.md`, the spec.

- [ ] **README:** the Live section drops "come in the next release". It says what works in Live:
  - find;
  - Split sync both ways;
  - the context menu;
  - drag-and-drop within Live;
  - screen readers through MSAA.

  It also says which commands switch to the source view.
- [ ] **Spec:** a "Phase 4 shipped" note under §12, with this plan's deviations, the perf numbers, and the present's share.
- [ ] **Full suite:**
  - `cargo clippy --all-targets -- -D warnings`
  - `cargo test --lib`
  - `cargo test --test markdown_live -- --test-threads=1`
  - the full window suite, `-- --test-threads=1`
  - Report any failure other than the known `tabs::` one.
- [ ] **Manual check** in the real app, with `fastpad.ini` backed up and restored:
  - find through the bar and F3;
  - Split scrolling both ways over the README;
  - fold and Ctrl+D notices;
  - the context menu on text and on a link;
  - dragging text within Live and from another app;
  - Narrator reading the Live view.
- [ ] Commit `docs(live): phase 4 README and spec note`. Push, and tell the user PR #72 is ready for their merge.
