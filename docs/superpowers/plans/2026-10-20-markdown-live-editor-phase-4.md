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

**Files:** `src/window/preview_host.rs`, `src/window/live_host.rs`.

- [x] `live_host::follow_preview(hwnd, id, line) -> bool` puts the preview's document line at the top of the group's shown Live view through the existing `LiveView::set_top_line`. That function posts nothing, so the scroll never syncs back. It needed no new `follow_line`.
- [x] `preview_host::follow_preview` calls it first, then keeps its source path. The hidden editor still scrolls, which keeps the place for a switch back to source. Its deferred `SCN_UPDATEUI` clears the `ScrollOrigin::Preview` guard, as a shown editor's does, so no guard is left behind (finding 3).
- [x] Tests:
  - `scrolling_the_split_preview_scrolls_live`: headings and tables; three scrolls; Live's top line equals the preview's, and so does the hidden editor's.
  - `split_sync_settles_after_scrolling_either_side`
  - `toggling_to_source_after_a_preview_scroll_keeps_the_place`

  All three failed before the change. The existing Live → preview test still passes.
- [x] Commit `feat(live): scroll Live from the Split preview`.

### Task 3: Command audit

**Files:**
- `src/window/live_host.rs`
- `src/window/main_window/command_dispatch.rs`
- `src/editor/scintilla/live_support.rs`, `src/editor/scintilla_constants.rs`
- `src/live/view/paint.rs`

- [x] **Source-only commands.** `live_host::source_only(CommandId) -> Option<&'static str>` names them, with the name the notice uses (Decision 4). `execute_command_with_note` calls `live_host::before_command` before `run_command`. In Live, that switches the group to source with "*Name* needs the source view. Switched to the source view.", so the command acts on the shown editor. Ctrl+D now gives that notice instead of the generic one.
- [x] **Column selections.** Add `Editor::is_rectangular_selection()` (`SCI_SELECTIONISRECTANGLE`). `poll` switches on `selection_count > 1` as before, and on a rectangle with "Live Markdown cannot show a column selection." Both go through one `leave` helper.
- [x] **Highlight current line.** The setting drives the active-block fill: off, the fill takes the background colour, and the painter skips it.
- [x] Tests:
  - `fold_all_switches_to_source_with_a_notice`
  - `a_one_row_rectangular_selection_switches_to_source`
  - `turning_off_current_line_highlight_drops_the_block_fill`
  - `source_only_names_the_commands_that_need_scintillas_ui`
  - `every_text_command_works_in_live_or_switches_to_source`: 35 commands from `TEXT_COMMANDS` and `EDITING_COMMANDS`.
    - Each is run with "bold" selected on a small note, once in Live and once in source.
    - Source-only commands switch with their notice. Every other command keeps Live shown with one selection and its caret's block revealed. In both cases the text matches the source view's.
    - Skipped, with the reason in the test: Save and Save As (dialog), the Live toggle, the disk-conflict commands, and language changes away from Markdown, which leave Live with their own notice.
- [x] Commit `feat(live): command audit; source-only commands switch with a notice`.

### Task 4: Context menu

**Files:**
- `src/live/view/mouse.rs`, `src/live/view/keys.rs`, `src/live/view/window_proc.rs`
- `src/window/messages.rs`
- `src/window/main_window/wndproc.rs`
- `src/window/live_host.rs`
- `src/window/menus.rs`

- [x] **Right press** (`mouse::right_press`): outside the selection, the caret moves to the pointer; inside it, the selection stays.
- [x] **`WM_CONTEXTMENU`** (`mouse::context_menu`): the menu opens at the pointer, or under the caret for a keyboard invocation (`lparam == -1`). Live sends `WM_FASTPAD_LIVE_MENU` (`WM_APP + 0x5F`) to the main window with a `MenuRequest`: the screen point, the position, and the link there.
  - The Menu key opens the menu on its release, and is no longer forwarded to the hidden editor.
  - Shift+F10 is a system key, so `DefWindowProcW` sends `WM_CONTEXTMENU`. This is checked by hand in Task 9.
- [x] **The menu** (`live_host::context_menu`):
  - Items: Undo and Redo (greyed from `can_undo` / `can_redo`); Cut, Copy and Delete (greyed without a selection); Paste; Select All.
  - Over a link, two more: Open link and Copy link address.
  - **No new command ids** (deviation from the plan): the menu-only items reuse ids as local entries, as the sidebar menus do (Delete is `DeleteLines`, Select All `SelectAllOccurrences`, Open link `NotesFollowLink`, Copy link address `MarkdownLink`). They are handled in `context_menu` and never dispatched.
  - Open link takes the Ctrl+click paths. Copy link address puts the destination as written on the clipboard.
- [x] **`menus.rs`:**
  - `MenuEntry::Greyed`, with `MenuEntry::enabled(bool)`, appends an entry `MF_GRAYED`.
  - A test-only record of the last popup, `last_popup_entries`, returns each label and whether it can run.
- [x] Tests:
  - `right_click_outside_the_selection_moves_the_caret`
  - `right_click_inside_the_selection_keeps_it`
  - `context_menu_copy_paste_delete_and_select_all_work_in_live`
  - `context_menu_over_a_link_copies_its_address`
  - `the_keyboard_menu_opens_at_the_caret_with_undo_greyed` (`WM_CONTEXTMENU` with `-1`, and the Menu key)
- [x] Commit `feat(live): context menu`.

### Task 5: Drag-and-drop

**Files:**
- `src/live/view/drop.rs` (new), `src/live/view/mouse.rs`, `src/live/view/keys.rs`, `src/live/view/paint.rs`, `src/live/view.rs`
- `src/platform/ole_drop.rs`
- `src/window/library_host/notebooks.rs`
- `src/window/live_host.rs`

- [x] **Drop target:** `drop.rs` registers an OLE target on the Live window when the window is created, and revokes it on `WM_NCDESTROY`. It takes `CF_UNICODETEXT`, read through the new `ole_drop::offers_text` / `dropped_text`.
  - While a drag is over the view, a steady drop caret (`ViewState::drop_at`) shows where the text would land.
  - On drop, the text is inserted at that point in the document's line endings, as one undo step, and selected.
  - The effect is copy, so another app's text is never taken from it unasked. Shift moves when the source allows it.
  - Autoscroll near the edges is not done.
- [x] **Files:** `ensure_view` calls the new `library_host::wrap_live_drop_target`, which wraps Live's target with the group's file handlers (the strips' kind: open, split, Ctrl to attach at the caret). A drag offering files never reaches the text target.
- [x] **Drag within Live:**
  - A press strictly inside a non-empty selection is pending until it moves past `SM_CXDRAG` / `SM_CYDRAG`. Moving past that starts a drag, with the drop caret following the pointer.
  - **Release away from the source:** moves the text, or copies it with Ctrl, as one undo step. The text is selected in its new place.
  - **Release with no move, or on the source:** places the caret, as a click.
  - **Esc, or losing the capture:** cancels with no edit.
  - The cursor is not changed during the drag.
- [x] Tests:
  - `dropping_text_inserts_it_at_the_point_in_one_undo_step`, through `LiveView::drop_text`, a fake `IDataObject` run through the registered target as OLE runs it.
  - `dropping_a_file_on_live_opens_it`
  - `dragging_a_selection_moves_it` (forward and back)
  - `ctrl_drag_copies`
  - `a_press_inside_the_selection_without_moving_places_the_caret`
  - `escape_cancels_a_drag`

  The drag tests move the real cursor with the simulated pointer. While the view holds the capture, Windows synthesizes a `WM_MOUSEMOVE` at the real cursor, and that made one run flaky.
- [x] Commit `feat(live): text drag-and-drop`.

### Task 6: The image folder follows saves

**Files:**
- `src/window/live_host.rs`
- `src/window/main_window/saving.rs`
- `src/window/library_host/local_state.rs`, `src/window/library_host/naming.rs`

- [x] `live_host::document_moved(hwnd)` re-reads each shown Live view's document folder (`preview_host::group_document`) and calls `set_document_dir`. That call does nothing when the folder is unchanged.
- [x] **Callers:**
  - every successful save (`save_active_to`, so Save and Save As);
  - `rebind_open_tab`, the one place an open tab follows a renamed or moved note (tree renames, moves, relocations the rescan finds);
  - the name box's rename.

  The update no longer depends on `WM_FASTPAD_APPLY_LANGUAGE`, which skips `sync_visibility` when Lexilla fails to apply.
- [x] Test: `saving_an_untitled_note_shows_its_relative_images`. An untitled note in Live shows its image's alt text. After a Save As next to the image, the thumbnail draws without a toggle. After a Save As into a folder without the image, the alt text is back.
  - It failed before the change.
  - The rename and move paths have no window test, since that needs a notebook. They are one call at the shared rebind point, and Task 9's manual check covers them.
- [x] Commit `fix(live): pick up the image folder when a note is saved or moved`.

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
