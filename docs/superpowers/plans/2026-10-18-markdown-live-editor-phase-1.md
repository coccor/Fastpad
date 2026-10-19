# Live Markdown Editor, Phase 1: Core and Plain View — Implementation Plan

> **For agentic workers:** Steps use checkbox (`- [ ]`) syntax for tracking. Work task by task; each task ends compiling, with its named tests green, and is committed on its own.

**Goal:** A Markdown tab can be switched (Ctrl+Alt+V) to a new FastPad-drawn view, the **Live view**, that shows the document as wrapped plain text in the prose font and is fully editable: caret, selection, keyboard, mouse, clipboard, undo, IME, scrolling and zoom. There is no Markdown rendering yet. This phase proves the text engine and the typing latency that phases 2–4 build on.

**Spec:** `docs/superpowers/specs/2026-10-18-markdown-live-editor-design.md`, §2, §4, §7.2, §8, §10 and §12 phase 1.

**Tech stack:** Rust 2024; Direct2D + DirectWrite through the `windows` 0.62.2 crate, reused from `src/preview` (`Graphics`, `create_hwnd_target`, `resolve_family`, `text_format`) and loaded lazily; Scintilla 5.6.6 stays the text model; `windows-sys` for the window, caret and IMM.

## Architecture

The Live view is a new child window of the editor group (class `FastPadLive`). While it is shown, the group's Scintilla is **hidden but alive**: it keeps the document, selection, undo history and save point, and it still sends every `SCN_*` notification, so everything that listens to it today keeps working.

The view owns only what Scintilla cannot do for it: **drawing, vertical geometry and hit testing**.

| Input | Where it goes |
|---|---|
| Printable characters (`WM_CHAR`) | Forwarded with `SendMessageW` to the group's Scintilla. Its subclass applies the C0 filter and swallow logic; Scintilla inserts at its selection with its own undo coalescing. |
| Left/Right, Ctrl+Left/Right, Backspace, Delete, Ctrl+Backspace/Delete, Enter, Tab, Shift+Tab, Escape, Ctrl+Home/End (with or without Shift) | Forwarded as `WM_KEYDOWN` to Scintilla. That runs `GroupHooks::key_down` (list continuation, table cells, paste) exactly as in the source view, then Scintilla's own key handling. |
| Up/Down, PgUp/PgDn, Home/End (with or without Shift) | Handled by the Live view: these depend on wrapped rows, which only the Live layout knows. The result is written back with `Editor::set_selection(anchor..caret)`. |
| Mouse | Hit-tested by the Live view, written back as a Scintilla selection. |
| Commands (save, undo, clipboard, find, line moves…) | Unchanged. They act on the group's `Editor` through command dispatch. |

Every change, whoever made it, comes back as `SCN_MODIFIED` and `SCN_UPDATEUI`. `handle_editor_notification` forwards them to the group's Live view, which invalidates the touched lines, follows Scintilla's selection, keeps the caret visible and repaints.

Code is split so that everything that can be pure is pure:

- **Pure, unit tested** (`src/live/`): `text.rs` (`LiveText` trait + in-memory implementation), `offsets.rs` (UTF-8 ↔ UTF-16 per line), `lines.rs` (per-line cache that follows edits), `motion.rs` (vertical and row motion over a `LineGeometry` trait), `scroll.rs` (caret-visible and anchor maths).
- **DirectWrite** (`src/live/layout.rs`): per-line `IDWriteTextLayout`s, row metrics, hit tests.
- **Win32** (`src/live/view.rs`, `view/window_proc.rs`, `view/paint.rs`, `view/ime.rs`): the window.
- **Host** (`src/window/live_host.rs`): per-group Live state, the toggle, the setting, show/hide, layout, notification wiring.

## Global constraints

- **No new crates.** The `windows` crate features stay as they are (the preview's Direct2D/DirectWrite set is enough). `windows-sys` gains `Win32_UI_Input_Ime` for IMM calls; `windows-sys` features are not audited, but say so in the PR.
- **Lazy graphics.** `d2d1.dll`, `dwrite.dll` and `windowscodecs.dll` load only when the first Live view or preview is created, through `preview_host::shared_graphics`. `assert_no_preview_imports()` and `launching_with_a_markdown_file_loads_no_preview_graphics_library` must keep passing. Never call the `windows` crate's `D2D1CreateFactory` / `DWriteCreateFactory`.
- **IMM is imported statically.** `imm32.dll` is already loaded into every GUI process by `user32.dll`, so this adds no load. Task 10 confirms with the startup benchmark that first paint is unchanged.
- **Scintilla constants** are added only through `$RequiredNames` in `tools/generate-scintilla-constants.ps1` and regenerating. `src/editor/scintilla_constants.rs` is never edited by hand.
- **No `SendMessageW` to the Scintilla HWND outside `src/editor/`**, except the two forwards (`WM_CHAR`, `WM_KEYDOWN`) and `WM_GETTEXT`/`WM_GETTEXTLENGTH`, which go through new `Editor::forward_char` / `forward_key` / `forward_get_text` methods.
- **New `CommandId`:** `MarkdownToggleLive = 282`. Retired ids (133, 134, 163, 166–173) are never reused. String id `markdown.toggleLive`, never renamed.
- **Size limit:** `LIVE_MAX_BYTES = 4 * 1024 * 1024`.
- **Typing latency:** Live p95 ≤ `baseline + baseline / 10 + 100` µs, where baseline is the same test typing into Scintilla (the preview's rule). Replaced in Task 12 by an absolute 4 ms p95; see deviation 11.
- **No backward-compatibility code.**
- Compile with `cargo clippy --all-targets` during tasks and run only the tests named in the step. The full suite runs once, in Task 12.
- Window tests run with `--test-threads=1`. In a worktree, copy `native/out` DLLs in first.
- Any run of the real app is wrapped in a backup and restore of `%APPDATA%\FastPad\fastpad.ini`.
- Work on branch `feat/markdown-live-editor`; `main` is protected; merge by PR with the CI checks green. Commit messages carry no `Co-Authored-By` trailer.

## Deviations from the spec (decided while planning)

1. **Live state is per (group, document).** The spec says "per tab", and a tab is a document in a group. `LiveHost` on `GroupWindow` keeps the set of documents shown Live in that group. The preview mode stays per group, as today.
2. **Edits go through Scintilla's own input path, not a `TextSource::replace` API.** Forwarding `WM_CHAR` / `WM_KEYDOWN` gives identical typing, undo grouping, overtype, `SCN_CHARADDED` and writing-helper behaviour for free. `LiveText` is read-and-select only in phase 1; programmatic edits (checkbox toggles in phase 3) will use `Editor::replace_target`.
3. **Horizontal caret motion is Scintilla's.** Left/Right and word motion step by Scintilla's character and word rules. In phase 1 every line is plain source, so source and screen positions agree. Phase 2 revisits this once rendered lines hide markup (the caret line is always revealed, so it may still hold).
4. **Wikilink completion is off in Live.** `[[` completion shows Scintilla's autocomplete popup at the hidden window's caret. `markdown_host::char_added` skips it while the group's Live view is shown. Phase 4 adds a Live popup.
5. **More than one selection turns Live off.** If Scintilla reports more than one selection (Ctrl+D, Alt+click and similar commands), the tab returns to the source view with a notice.
6. **Split scroll sync, best effort.** When the Live view scrolls, it sets Scintilla's first visible line to its top source line. Scintilla's `SC_UPDATE_V_SCROLL` then drives the existing preview sync. Exact sync is phase 4.
7. **`markdown_live_default` is set only in `fastpad.ini`.** It gets no Settings dialog row in this phase.
8. **No context menu, no text drag-and-drop in Live.** File drops (attachments) work through the existing `file_drop` handler.
9. **Selection changes are polled, not notified** (found in Task 9). A hidden Scintilla sends `SCN_UPDATEUI` only when it paints or does idle work, so a selection-only change while Live is shown is never reported. The Live view posts `WM_FASTPAD_LIVE_CHANGED` after its own input, every command ends with `live_host::poll_active`, and `poll` compares the selection with the last one seen before running what `SC_UPDATE_SELECTION` would have (status bar, table tracking, scroll to caret, the one-selection check). Live scrolling likewise drives the Split preview through `preview_host::editor_scrolled` directly.
10. **IMM is loaded lazily, not imported** (Task 10). `imm32.dll` is resolved with `GetProcAddress` on the first composition, as Direct2D is; `windows-sys`'s `Win32_UI_Input_Ime` supplies only the types. So nothing enters the import table and the startup benchmark check is not needed. Ctrl+click on a wikilink goes through `WM_FASTPAD_LIVE_LINK` (sent, returning whether a link was followed) to `markdown_host::follow_link_at`.
11. **Typing latency has an absolute budget** (Task 12, decided with the user). The Live view first waited for vsync in `EndDraw` (17 ms a keystroke); it now presents immediately and repaints only the invalidated rows (`D2D1_PRESENT_OPTIONS_IMMEDIATELY | RETAIN_CONTENTS`, clipped to the update rectangle). A keystroke then costs about 2.0 ms p95 against 1.0–1.4 ms in the source view, the remaining gap being the hardware target presenting the whole window. The perf tests assert Live p95 ≤ 4 ms (a quarter frame) and print the source time; a present limited to the changed rectangle (a DC render target bound to the update rectangle, or a flip-model swap chain with dirty rectangles) is phase 4 work.

## Review focus

1. **Focus checks that assume the Scintilla HWND.** Task 7 lists them all. A missed one shows up as Ctrl+B toggling the sidebar in Live, editing shortcuts refused, or tables never auto-formatting. *(Tests: Task 7 `ctrl_b_in_live_bolds_not_toggles_sidebar`, `alt_up_moves_lines_in_live`.)*
2. **The hidden Scintilla must stay hidden.** `preview_host::sync_visibility` and `tab_strip::refresh_tabs` both call `ShowWindow` on the editor today. *(Test: Task 6 `switching_tabs_and_preview_modes_keeps_scintilla_hidden_in_live`.)*
3. **UTF-16 ↔ UTF-8 mapping** with surrogate pairs, combining marks and CRLF. *(Tests: Task 2.)*
4. **Line cache after edits** that add or remove lines, including undo of a multi-line paste. *(Tests: Task 3 `cache_follows_edits_like_a_rebuild`, Task 9 `undo_of_a_multi_line_paste_redraws_every_line`.)*
5. **Toggling never edits or dirties** the document, and undo works across a toggle. *(Test: Task 6.)*

---

## File structure

| File | Status | Responsibility |
|---|---|---|
| `tools/generate-scintilla-constants.ps1` | modify | add `SCI_WORDSTARTPOSITION`, `SCI_WORDENDPOSITION`, `SCI_GETSELECTIONS` (if missing from the list), `SCI_GETLINEENDPOSITION` (already present; check) |
| `src/editor/scintilla_constants.rs` | regenerate | — |
| `src/editor/scintilla/live_support.rs` | create | `Editor` methods Live needs: `anchor_and_caret`, `selection_count`, `line_end`, `word_range_at`, `hooks`, `forward_char`, `forward_key`, `forward_get_text` |
| `src/editor/scintilla.rs` | modify | `mod live_support;`, expose hooks |
| `src/live/mod.rs` | create | module root, `LIVE_MAX_BYTES` |
| `src/live/text.rs` | create | `LiveText` trait, `EditorText` (over `Editor`), `MemoryText` (tests) |
| `src/live/offsets.rs` | create | `OffsetMap`: UTF-8 ↔ UTF-16 for one line |
| `src/live/lines.rs` | create | `LineCache<T>`: per-line slots that follow `preview::incremental::Edit`s |
| `src/live/motion.rs` | create | `LineGeometry` trait; up/down/page/home/end over rows |
| `src/live/scroll.rs` | create | scroll-to-caret and anchor maths over `HeightIndex` |
| `src/live/layout.rs` | create | `LineLayout` (DirectWrite layout + `OffsetMap` + rows), `LiveLayout` (cache + `HeightIndex`), `LiveFonts` |
| `src/live/view.rs` | create | `LiveView` handle, `ViewState`, create/destroy, public API used by the host |
| `src/live/view/window_proc.rs` | create | message handling |
| `src/live/view/paint.rs` | create | Direct2D painting: background, text, selection, caret |
| `src/live/view/ime.rs` | create | composition and candidate window placement |
| `src/lib.rs` | modify | `pub mod live;` |
| `src/window/live_host.rs` | create | `LiveHost`, toggle, setting, show/hide, layout, notifications, `forget` |
| `src/window/editor_group.rs` | modify | `GroupWindow.live: LiveHost` |
| `src/window/main_window/group_layout.rs` | modify | `layout_group` sizes the Live view; `destroy_group` drops it |
| `src/window/preview_host.rs` | modify | `sync_visibility` asks `live_host` which text view to show |
| `src/window/tab_strip.rs` | modify | `refresh_tabs` ditto |
| `src/window/main_window/focus.rs` | modify | `content_focus_target` prefers the Live view |
| `src/window/main_window/menu_keys.rs`, `src/window/markdown_host.rs`, `src/window/image_host.rs`, `src/window/main_window/split_groups.rs` | modify | focus checks accept the Live view (Task 7) |
| `src/window/main_window/wndproc.rs` | modify | forward `SCN_MODIFIED` / `SCN_UPDATEUI` / `SCN_ZOOM` to `live_host` |
| `src/window/commands.rs`, `keymap.rs`, `menus.rs`, `command_palette.rs`, `shortcuts_model.rs`, `main_window/command_dispatch.rs`, `main_window/menu_keys.rs` | modify | the toggle command |
| `src/config/persisted.rs`, `src/config/defaults.rs` | modify | `markdown_live_default` |
| `Cargo.toml` | modify | `windows-sys` feature `Win32_UI_Input_Ime`; `[[test]] markdown_live` |
| `tests/windows/markdown_live.rs` | create | in-process integration and perf tests |
| `README.md` | modify | Live Markdown (preview) section |

---

### Task 1: `Editor` support for Live

**Files:** `tools/generate-scintilla-constants.ps1`, `src/editor/scintilla_constants.rs` (regenerated), `src/editor/scintilla/live_support.rs`, `src/editor/scintilla.rs`, `src/editor/scintilla/tests.rs`.

- [ ] Add to `$RequiredNames` whatever of these is missing: `SCI_WORDSTARTPOSITION`, `SCI_WORDENDPOSITION`, `SCI_GETSELECTIONS`, `SCI_GETANCHOR`, `SCI_GETLINEENDPOSITION`. Regenerate.
- [ ] Create `live_support.rs` with:
  ```rust
  impl Editor {
      /// The main selection as (anchor, caret), keeping its direction.
      pub fn anchor_and_caret(&self) -> Result<(usize, usize)>;
      /// How many selections Scintilla holds (1 for a plain caret).
      pub fn selection_count(&self) -> Result<usize>;
      /// Byte position of the end of `line`, before its line ending.
      pub fn line_end(&self, line: usize) -> Result<usize>;   // make the private one in line_ops.rs pub(crate) and reuse it
      /// The word around `position` by Scintilla's word rules (SCI_WORDSTART/ENDPOSITION, onlyWordCharacters = true).
      pub fn word_range_at(&self, position: usize) -> Result<Range<usize>>;
      /// The hooks installed with `set_hooks`.
      pub(crate) fn hooks(&self) -> Option<Rc<dyn EditorHooks>>;
      /// Sends `WM_CHAR` / `WM_KEYDOWN` to the Scintilla window, through FastPad's subclass.
      pub(crate) fn forward_char(&self, wparam: usize, lparam: isize) -> isize;
      pub(crate) fn forward_key(&self, message: u32, wparam: usize, lparam: isize) -> isize; // WM_KEYDOWN / WM_KEYUP / WM_SYSKEYDOWN
      /// Answers `WM_GETTEXT` / `WM_GETTEXTLENGTH` with Scintilla's own answer.
      pub(crate) fn forward_get_text(&self, message: u32, wparam: usize, lparam: isize) -> isize;
  }
  ```
  The forwards are `SendMessageW(endpoint.hwnd, …)` and return 0 once the endpoint is destroyed. Non-Windows stubs return `Err` / 0, like the other files.
- [ ] Tests (fake harness): `anchor_and_caret_reads_anchor_then_current_pos`, `selection_count_reads_getselections`, `word_range_at_asks_for_word_characters_only`. Real DLL: `forward_char_inserts_through_the_subclass` (a forwarded `\x02` is dropped by the C0 filter, `a` is inserted), `forward_key_runs_the_hooks_first` (a hook taking Enter swallows it; reuse the pattern of `a_hook_that_takes_enter_also_swallows_its_char`).
- [ ] `cargo test --lib editor::scintilla::tests` → green. Commit `feat(editor): Editor support for the Live view`.

### Task 2: `live::offsets` — UTF-8 ↔ UTF-16 for one line (pure)

**Files:** `src/lib.rs`, `src/live/mod.rs`, `src/live/offsets.rs`.

- [ ] `pub mod live;` in `lib.rs`; `live/mod.rs` declares the modules and `pub const LIVE_MAX_BYTES: usize = 4 * 1024 * 1024;`.
- [ ] `OffsetMap` is built from a line's text (no line ending) and stores, per UTF-16 unit boundary, the matching byte offset. Phase 2 adds hidden ranges, so build it from an iterator of `(byte_range, shown: bool)` pieces now, with phase 1 always passing one shown piece:
  ```rust
  pub struct OffsetMap { /* utf16 -> byte table, plus the layout string */ }
  impl OffsetMap {
      pub fn plain(line: &str) -> Self;
      pub fn layout_text(&self) -> &[u16];          // the string handed to DirectWrite
      pub fn to_utf16(&self, byte: usize) -> u32;   // byte inside the line; mid-char bytes round down
      pub fn to_byte(&self, utf16: u32) -> usize;   // a low surrogate rounds down to its pair's start
      pub fn byte_len(&self) -> usize;
  }
  ```
- [ ] Tests: `ascii_maps_one_to_one`, `two_and_three_byte_chars_map_to_one_unit`, `astral_chars_map_to_a_surrogate_pair`, `mid_char_bytes_round_down`, `low_surrogate_rounds_down`, `end_of_line_maps_to_the_end`, `empty_line`.
- [ ] `cargo test --lib live::offsets`. Commit `feat(live): map line bytes to UTF-16 positions`.

### Task 3: `live::lines` — per-line cache that follows edits (pure)

**Files:** `src/live/lines.rs`.

- [ ] `LineCache<T>` holds `Vec<Option<T>>`, one slot per document line.
  ```rust
  impl<T> LineCache<T> {
      pub fn reset(&mut self, line_count: usize);
      pub fn get(&self, line: usize) -> Option<&T>;
      pub fn set(&mut self, line: usize, value: T);
      pub fn invalidate(&mut self, lines: Range<usize>);
      /// An edit at `first_line` that added (`lines_delta > 0`) or removed lines: the first line
      /// is invalidated, removed slots are dropped, added slots start empty.
      pub fn apply(&mut self, first_line: usize, lines_delta: isize) -> Range<usize>; // returns the lines to re-lay
      pub fn len(&self) -> usize;
  }
  ```
  The host converts each `SCN_MODIFIED` into `(line_from_position(position), lines_added)`. A deletion's first line is the line of `position` before the deletion, which is what `SCN_MODIFIED` reports.
- [ ] Tests: `insert_within_a_line_invalidates_only_that_line`, `inserting_lines_shifts_later_slots_down`, `removing_lines_drops_their_slots`, `cache_follows_edits_like_a_rebuild` (property-style: a scripted sequence of random inserts/deletes on a `String`; after each, every `Some` slot must equal the line text cached when it was filled, compared against a fresh split of the string).
- [ ] `cargo test --lib live::lines`. Commit `feat(live): line cache that follows edits`.

### Task 4: `live::text`, `live::motion`, `live::scroll` (pure)

**Files:** `src/live/text.rs`, `src/live/motion.rs`, `src/live/scroll.rs`.

- [ ] `text.rs`:
  ```rust
  pub trait LiveText {
      fn len(&self) -> usize;
      fn line_count(&self) -> usize;
      fn line_of(&self, byte: usize) -> usize;
      fn line_start(&self, line: usize) -> usize;
      fn line_end(&self, line: usize) -> usize;              // before the line ending
      fn line(&self, line: usize) -> Cow<'_, str>;           // without the line ending, lossy UTF-8
      fn selection(&self) -> (usize, usize);                 // (anchor, caret)
      fn set_selection(&self, anchor: usize, caret: usize);
      fn word_at(&self, byte: usize) -> Range<usize>;
  }
  pub struct EditorText<'a>(pub &'a Editor);   // errors become 0 / empty: a destroyed editor has nothing to show
  #[cfg(test)] pub struct MemoryText { text: String, selection: Cell<(usize, usize)> }
  ```
  `EditorText::line` uses `range_bytes(line_start..line_end)` (zero copy, valid until the next edit; the layout copies it into UTF-16 immediately).
- [ ] `motion.rs` works in source bytes over a geometry the layout provides:
  ```rust
  pub struct Row { pub line: usize, pub start: usize, pub end: usize, pub top: f32, pub height: f32 } // bytes are document positions
  pub trait LineGeometry {
      fn row_at(&mut self, position: usize) -> Row;            // the wrapped row holding `position` (end-of-row positions stick to the row they end)
      fn row_above(&mut self, row: &Row) -> Option<Row>;
      fn row_below(&mut self, row: &Row) -> Option<Row>;
      fn x_of(&mut self, position: usize) -> f32;
      fn position_at_x(&mut self, row: &Row, x: f32) -> usize;
      fn page_height(&self) -> f32;
  }
  pub enum Move { Up, Down, PageUp, PageDown, RowStart, RowEnd }
  pub struct Motion { pub caret: usize, pub goal_x: Option<f32> }
  pub fn apply(geometry: &mut impl LineGeometry, caret: usize, goal_x: Option<f32>, movement: Move) -> Motion;
  ```
  Rules: Up/Down keep `goal_x` (set from the caret's x on the first vertical move, cleared by any other change of caret, which the view does); Up on the first row goes to 0, Down on the last row goes to the document end; PageUp/PageDown move by `page_height` rows' worth of y; RowStart on a wrapped row goes to the row start, and on the first row toggles between the first non-blank and the line start (Scintilla's VC Home behaviour); RowEnd goes to the row end.
- [ ] `scroll.rs`:
  ```rust
  /// The scroll offset that keeps `[top, bottom)` visible in a viewport of `height` at `scroll`, with `margin` px.
  pub fn reveal(scroll: f32, height: f32, top: f32, bottom: f32, margin: f32) -> f32;
  pub fn clamp(scroll: f32, height: f32, total: f32) -> f32;
  ```
  Anchoring across height changes reuses `preview::heights::HeightIndex::{anchor, scroll_for_anchor}`.
- [ ] Tests with a fake geometry (fixed-width "font", rows wrap every N chars): `down_keeps_the_goal_column_across_a_short_row`, `up_from_the_first_row_goes_to_the_start`, `down_from_the_last_row_goes_to_the_end`, `page_down_moves_a_screenful`, `home_toggles_between_indent_and_line_start`, `home_on_a_wrapped_row_goes_to_the_row_start`, `end_goes_to_the_row_end_not_the_line_end`; `reveal_scrolls_up_and_down_only_as_needed`, `clamp_keeps_the_last_row_at_the_bottom`; `MemoryText` line maths with CRLF and a trailing newline.
- [ ] `cargo test --lib live::`. Commit `feat(live): text access, vertical motion and scroll maths`.

### Task 5: `live::layout` — DirectWrite line layouts

**Files:** `src/live/layout.rs`.

- [ ] `LiveFonts { family: ResolvedFamily, size: f32 /* DIPs */, line_height: f32 }` built from `preview_font`, `preview_font_size` and `preview_line_height` (the same values the preview uses, through `PreviewFonts::from_settings`), scaled by the editor zoom: `size * (font_size + zoom) / font_size`, clamped to the zoom range Scintilla allows.
- [ ] `LineLayout { layout: IDWriteTextLayout, offsets: OffsetMap, rows: Vec<(u32 /*utf16 start*/, f32 /*top*/, f32 /*height*/)>, height: f32 }`, built with `CreateTextLayout(offsets.layout_text(), format, width, f32::MAX)`, word wrap on, uniform line spacing at `line_height`. Rows come from `GetLineMetrics`. An empty line still has one row of `line_height`.
- [ ] `LiveLayout` holds `LineCache<LineLayout>`, a `HeightIndex` (estimates = `line_height` × an estimate of wrapped rows from the byte length and an average char width), the width, fonts and a generation counter. Methods:
  - `ensure(&mut self, text: &impl LiveText, lines: Range<usize>)` lays out missing lines and `set_measured`s their heights;
  - `line_top(line) -> f32`, `line_at_y(y) -> usize`, `total_height()`;
  - `caret_rect(text, position) -> RectF` (`HitTestTextPosition`);
  - `position_at(text, x, y) -> usize` (`HitTestPoint`, mapped back through `OffsetMap::to_byte`, trailing hit → next position);
  - `selection_rects(text, range, visible: Range<usize>) -> Vec<RectF>` (`HitTestTextRange` per line, plus a small end-of-line box for each selected line ending);
  - `edit(first_line, lines_delta)` → `LineCache::apply` + `HeightIndex::splice`;
  - `set_width` / `set_fonts` → drop all layouts, reset estimates, keep the scroll anchor.
  - `impl LineGeometry for (LiveLayout, &impl LiveText)` for `motion::apply`.
- [ ] Tests (need DirectWrite; follow `preview/layout/tests.rs`'s pattern of loading `Graphics` in-process): `a_long_line_wraps_into_rows`, `empty_lines_have_one_row`, `hit_test_round_trips_every_position_on_a_wrapped_line`, `astral_chars_hit_test_to_their_start`, `selection_across_lines_has_one_rect_per_row`, `editing_a_line_keeps_the_others_laid_out`.
- [ ] `cargo test --lib live::layout`. Commit `feat(live): DirectWrite line layouts with hit testing`.

### Task 6: The Live window and `live_host` (show, hide, toggle plumbing)

**Files:** `src/live/view.rs`, `src/live/view/window_proc.rs`, `src/live/view/paint.rs`, `src/window/live_host.rs`, `src/window/editor_group.rs`, `src/window/main_window/group_layout.rs`, `src/window/preview_host.rs`, `src/window/tab_strip.rs`, `src/window/main_window/focus.rs`, `Cargo.toml`, `tests/windows/markdown_live.rs`.

- [ ] `LiveView` copies the preview view's pattern (`src/preview/view.rs:507-554`, `window_proc.rs`): class `FastPadLive`, `WS_CHILD | WS_CLIPSIBLINGS | WS_TABSTOP | WS_VSCROLL`, created hidden, `Box<ViewState>` in `GWLP_USERDATA`, freed on `WM_NCDESTROY`, `WM_ERASEBKGND => 1`, `WM_GETDLGCODE => DLGC_WANTALLKEYS`, `WM_SETFOCUS` → `post_content_focus(hwnd)`. `LiveView { hwnd }` is `Copy`.
- [ ] `ViewState { editor: Editor, graphics: Rc<Graphics>, target: Option<ID2D1HwndRenderTarget>, colors, fonts, layout: LiveLayout, scroll_y: f32, goal_x: Option<f32>, caret_visible: bool, focused: bool, dpi: u32 }`. The editor is the group's own `Editor` clone.
- [ ] Paint (`paint.rs`): `BeginDraw`, clear to the editor background, draw the visible lines (`ensure` them first) at `x = padding`, `y = line_top - scroll_y`, then selection rects (selection colour from the theme `Palette`, 40% when unfocused, like Scintilla's inactive selection), then the caret (1 px × row height, caret colour) when focused and blinking on. `D2DERR_RECREATE_TARGET` drops the target and repaints. Colours: text and background from `Palette::for_theme(theme, hc)` (same source as Scintilla's base colours), not the preview's GitHub colours, so Live and source look like the same editor.
- [ ] `LiveView` API for the host: `create(parent, editor, graphics, colors, fonts) -> Result<Self>`, `hwnd()`, `destroy()`, `reload()` (reset the layout from the document), `edited(first_line, lines_delta)`, `selection_changed()`, `set_appearance(colors, fonts)`, `zoom_changed()`, `scroll_to_caret()`, `top_line() -> usize`, `set_top_line(line)`.
- [ ] `live_host.rs`:
  ```rust
  #[derive(Default)]
  pub(crate) struct LiveHost {
      documents: HashMap<DocumentId, bool>, // the choice made for each document in this group; absent = follow the default
      view: Option<LiveView>,
      shown_for: Option<DocumentId>,
  }
  pub(crate) fn is_live(hwnd: HWND, group: GroupId) -> bool;               // active doc is Markdown and chosen Live
  pub(crate) fn text_view_hwnd(hwnd: HWND, group: GroupId) -> Option<HWND>; // the Live view if live, else the editor
  pub(crate) fn sync(hwnd: HWND, group: GroupId);                           // create/show/hide the view and hide/show Scintilla
  pub(crate) fn set_live(hwnd: HWND, group: GroupId, on: bool);
  pub(crate) fn forget(hwnd: HWND, document: DocumentId);                   // every group
  ```
  `sync` is the **only** place that decides whether the group's Scintilla is visible when the tab shows text. `preview_host::sync_visibility` (`:700-704`) and `tab_strip::refresh_tabs` (`:535-554`) call `live_host::sync` instead of `ShowWindow(editor, …)`, passing whether text is shown at all (Full preview / image tab → neither view shown).
  Turning on: check `LIVE_MAX_BYTES` (notice and stay off when over), get `shared_graphics`, create the view if needed, carry over the caret (already in Scintilla) and the top line (`doc_line_from_visible(first_visible_line())`), `ShowWindow(editor, SW_HIDE)`, show the view, move focus to it if focus was on the editor. Turning off: the reverse, `set_first_visible_line(visible_from_doc_line(view.top_line()))`, focus to the editor. Neither edits text.
- [ ] `GroupWindow` gains `pub(crate) live: LiveHost`. `layout_group` (`group_layout.rs:485-496`) moves the Live view to `rects.editor` too. `destroy_group` destroys it. `focus.rs::content_focus_target` becomes `image → full preview → live_host::text_view_hwnd → editor`.
- [ ] Wire `markdown_host::forget` call sites to also call `live_host::forget` (`document_tabs.rs:411,445`, `language_tools.rs:78`, `opening.rs:230,333`, `background_documents.rs:118,208,310`), or better make `markdown_host::forget` call it. A language change away from Markdown calls `live_host::sync` so the view hides.
- [ ] For testing before the command exists, expose `live_host::set_live` under `#[cfg(test)]`-reachable `pub(crate)` (the in-process test crate `include!`s `lib.rs`).
- [ ] `Cargo.toml`: `[[test]] name = "markdown_live", path = "tests/windows/markdown_live.rs"`. The test file copies `TestMain` / `make_markdown` / pumping helpers from `tests/windows/markdown_preview.rs` (move the shared ones to `tests/windows/support/in_process.rs` if the copy exceeds ~80 lines).
- [ ] Tests: `live_view_replaces_scintilla_and_back` (Scintilla hidden, Live visible and focused; off → reversed), `toggling_does_not_dirty_or_add_undo` (`can_undo` unchanged, save point kept), `switching_tabs_and_preview_modes_keeps_scintilla_hidden_in_live` (Live tab → text tab shows Scintilla → back shows Live; Split keeps Live left; Full hides both, back to Split shows Live), `a_note_over_the_size_limit_stays_in_source_with_a_notice`, `closing_the_tab_forgets_its_live_choice`, `live_paints_without_error` (`UpdateWindow`, then `GetPixel` in the text area is not uniformly background for a non-empty doc).
- [ ] `cargo test --test markdown_live -- --test-threads=1`, plus `--test markdown_preview` (visibility changes). Commit `feat(live): Live view window and per-tab hosting`.

### Task 7: Focus checks accept the Live view

**Files:** `src/window/main_window/menu_keys.rs`, `src/window/markdown_host.rs`, `src/window/tab_strip.rs`, `src/window/image_host.rs`, `src/window/main_window/split_groups.rs`, `src/window/preview_host.rs`.

Add `live_host::is_text_view(hwnd, window: HWND) -> Option<GroupId>` (the window is a group's Scintilla or its shown Live view) and use it at each place that compares against the Scintilla HWND:

- [ ] `menu_keys.rs:332-361` `markdown_key` (Ctrl+B/I/`/K reach `markdown_host::format`).
- [ ] `menu_keys.rs:308-328` `editing_key_off_editor` and `is_group_editor` (`:364`).
- [ ] `menu_keys.rs:262-301` `start_tab_for_typing`.
- [ ] `markdown_host.rs:491-501` `typed_at_caret` (`group_of_editor(hwnd, GetFocus())` must map the Live view to its group, or table auto-format never fires).
- [ ] `tab_strip.rs:542,549` focus fix-up.
- [ ] `image_host.rs:128-137,156-162` "focus in content" and focus restore.
- [ ] `split_groups.rs:365`, `preview_host.rs:422,509-520,691-696`.
- [ ] `markdown_host::char_added` returns early when the group is Live (deviation 4).
- [ ] Tests in `markdown_live.rs`: `ctrl_b_in_live_bolds_not_toggles_sidebar`, `alt_up_moves_lines_in_live`, `typing_in_a_table_then_leaving_formats_it_in_live` (this one needs Task 8's typing; mark it and write it in Task 8 if needed), `escape_from_full_preview_returns_focus_to_live`.
- [ ] `cargo test --test markdown_live -- --test-threads=1`. Commit `fix(live): editor focus checks accept the Live view`.

### Task 8: Keyboard and text input

**Files:** `src/live/view/window_proc.rs`, `src/live/view.rs`.

- [ ] `WM_CHAR` → `editor.forward_char(wparam, lparam)`. `WM_KEYDOWN` / `WM_SYSKEYDOWN`:
  - Up/Down/PgUp/PgDn/Home/End (no Ctrl; Shift extends): `motion::apply` over the layout, then `set_selection(anchor or caret, new caret)`, keep `goal_x` for Up/Down/PgUp/PgDn, clear it otherwise.
  - Everything else listed in the Architecture table → `editor.forward_key(message, wparam, lparam)`, clear `goal_x`.
  - Other keys → `DefWindowProcW` (accelerators were already translated in the message loop).
  - `WM_KEYUP` is forwarded too, so Scintilla's key state stays consistent.
- [ ] After any forward or motion: the view does **not** repaint by itself; the `SCN_*` that Scintilla sends drive it (Task 9). Exception: pure caret motion that Scintilla reports only as `SCN_UPDATEUI` — also handled in Task 9.
- [ ] Caret blink: `SetTimer` with `GetCaretBlinkTime()`, reset on every input; hidden when unfocused; no timer when the blink time is `INFINITE`.
- [ ] `WM_GETTEXT` / `WM_GETTEXTLENGTH` → `editor.forward_get_text` (screen readers, `support/win32.rs::scintilla_text`).
- [ ] Tests (send real `WM_KEYDOWN` + `WM_CHAR` to the Live HWND, read text through it): `typing_in_live_matches_typing_in_source` (same script into a Live and a source tab; same text, same undo steps), `enter_continues_a_list_in_live`, `tab_nests_a_list_item_in_live`, `backspace_and_ctrl_backspace`, `shift_down_extends_over_a_wrapped_row` (narrow the window so a line wraps; Shift+Down ends on the next row, not the next line), `home_end_follow_rows`, `up_down_keep_the_goal_column`, `typing_in_a_table_then_leaving_formats_it_in_live`.
- [ ] Commit `feat(live): keyboard and text input`.

### Task 9: Following Scintilla — notifications, scrolling, zoom

**Files:** `src/window/main_window/wndproc.rs`, `src/window/live_host.rs`, `src/live/view.rs`, `src/live/view/window_proc.rs`.

- [ ] In `handle_editor_notification` (`wndproc.rs:672-801`), for every group showing the document **with a shown Live view**:
  - `SCN_MODIFIED` with text change → `live_host::edited(hwnd, group, &notification)` → `view.edited(line_from_position(position), lines_added)` → invalidate the rect from that line's top to the bottom of the viewport when heights changed, else just the line.
  - `SCN_UPDATEUI` with `SC_UPDATE_SELECTION` or `SC_UPDATE_CONTENT` → `view.selection_changed()` → if `selection_count() > 1` turn Live off with the notice "Live Markdown works with one selection. Switched to source view." (deviation 5); else reset blink, scroll to caret, invalidate old and new caret/selection rects, update the system caret (Task 10).
  - `SCN_ZOOM` → `view.zoom_changed()` (rebuild fonts, keep anchor).
  - Note the existing filter at `:742-752` records changes only from the reporting group; Live views of **other** groups showing the same document must be notified too (iterate `groups_showing`, as `record_edit` does).
- [ ] Theme, font and settings changes: `settings_apply::apply_editor_settings` and the theme-change path call `live_host::refresh_appearance(hwnd)` for every group (mirror `preview_host::refresh_appearance`).
- [ ] Scrolling: `WM_VSCROLL` (line, page, thumb track), `WM_MOUSEWHEEL` (system wheel lines × line height), `SetScrollInfo` with pixel units, page = client height. Ctrl+wheel → `editor.zoom_in/out`. After any scroll, `editor.set_first_visible_line(visible_from_doc_line(top_line))` (deviation 6), guarded so the resulting `SCN_UPDATEUI` does not scroll Live back.
- [ ] `WM_SIZE` → `layout.set_width` (keep anchor), resize target; `WM_DPICHANGED_AFTERPARENT` → new DPI, fonts, target.
- [ ] Tests: `undo_and_redo_redraw_the_changed_lines`, `undo_of_a_multi_line_paste_redraws_every_line` (compare `layout` line texts against the document after each step via a `#[cfg(test)] view.debug_lines()`), `find_next_moves_the_caret_and_scrolls_it_into_view`, `ctrl_d_switches_to_source_with_a_notice`, `an_external_reload_while_live_shows_the_new_text`, `a_second_group_showing_the_note_in_live_follows_edits`, `ctrl_wheel_zooms_live_and_source_together`, `scrolling_live_scrolls_the_split_preview`.
- [ ] Commit `feat(live): follow Scintilla edits, selection, scroll and zoom`.

### Task 10: Mouse, system caret and IME

**Files:** `src/live/view/window_proc.rs`, `src/live/view/ime.rs`, `Cargo.toml`.

- [ ] Mouse: `WM_LBUTTONDOWN` → `SetFocus`, `SetCapture`, `position_at(x, y + scroll_y)`; Shift extends; `WM_LBUTTONDBLCLK` → `word_range_at`; triple click (second down within `GetDoubleClickTime` and the double-click rectangle after a double click) → the source line including its line ending; `WM_MOUSEMOVE` with capture → extend, keeping the double-click word / triple-click line unit; autoscroll with a 30 ms timer while the pointer is above or below the client area; `WM_LBUTTONUP` / `WM_CAPTURECHANGED` end the drag. Ctrl+click → `editor.hooks().ctrl_click(x, y)` is **not** usable (it hit-tests Scintilla); instead add `markdown_host::follow_link_at(hwnd, position)` and call it with the hit position (wikilinks follow as in source view). `WM_SETCURSOR` → I-beam in the text area.
- [ ] System caret: `CreateCaret(hwnd, null, 1, row_height)` on `WM_SETFOCUS`, `SetCaretPos` whenever the caret moves or scrolls, `DestroyCaret` on `WM_KILLFOCUS`. It is never shown (`ShowCaret` not called); the drawn caret is what the user sees.
- [ ] IME (`ime.rs`, `windows-sys` feature `Win32_UI_Input_Ime`): on `WM_IME_STARTCOMPOSITION` and on every caret move while composing, `ImmGetContext` → `ImmSetCompositionFontW` (prose font at the current size) → `ImmSetCompositionWindow(CFS_POINT at the caret)` → `ImmSetCandidateWindow(CFS_EXCLUDE around the caret row)` → `ImmReleaseContext`; then `DefWindowProcW`. The result string arrives as `WM_IME_CHAR` → `DefWindowProcW` → `WM_CHAR`s → forwarded (Task 8). Scintilla's own IME code never runs, because it never gets `WM_IME_*`.
- [ ] Tests: `click_places_the_caret_on_a_wrapped_row`, `shift_click_extends`, `double_click_selects_a_word`, `triple_click_selects_the_line`, `drag_below_the_view_autoscrolls`, `ctrl_click_follows_a_wikilink`, `system_caret_tracks_the_caret` (`GetCaretPos` after typing), `ime_result_inserts_one_undo_step` (post `WM_IME_CHAR`s for "日本" to the view; text inserted, one `undo` removes it).
- [ ] Startup check: `cargo run --release --bin fastpad-bench -- startup` (or the documented benchmark command in `benchmarks/README.md`) before and after this task; first paint unchanged within noise. Record the numbers in the PR.
- [ ] Commit `feat(live): mouse, system caret and IME`.

### Task 11: The toggle command and the default setting

**Files:** `src/window/commands.rs`, `src/window/keymap.rs`, `src/window/menus.rs`, `src/window/command_palette.rs`, `src/window/shortcuts_model.rs`, `src/window/main_window/command_dispatch.rs`, `src/window/main_window/menu_keys.rs`, `src/config/persisted.rs`, `src/config/defaults.rs`, `src/window/live_host.rs`, and the count tests.

- [ ] `CommandId::MarkdownToggleLive = 282`. Add it to `COMMANDS` (171 → 172), `TEXT_COMMANDS` if it needs a text tab (it does: it is meaningless on an image tab), and the `needs_document` handling. Change the `try_from(282).is_err()` test to 283.
- [ ] `COMMAND_IDS`: `(MarkdownToggleLive, "markdown.toggleLive")`. `DEFAULT_BINDINGS`: `(key(C | A, ch(b'V')), MarkdownToggleLive)`, global scope (so it can show its notice on non-Markdown tabs). Bump the binding array length and the menu accelerator counts (`menus.rs:869,878`, 84 → 85).
- [ ] View menu: "Live Markdown\tCtrl+Alt+V" after "Close Markdown preview", checked when the active group is Live, greyed when the tab is not Markdown. Bring back a `set_markdown_live(menu, enabled, checked)` helper (see `git show b99b099 -- src/window/menus.rs`) and call it from `open_menu` (`menu_keys.rs:123-131`).
- [ ] Palette: entry "Markdown: Toggle Live view" (`ENTRIES` 149 → 150, `command_palette/tests.rs:93`), shown only for Markdown tabs (the `is_markdown_edit` filter in `command_palette_ui.rs:535-544`; update `main_window/tests/command_palette.rs:30`'s count filter).
- [ ] Dispatch: `MarkdownToggleLive => live_host::toggle(hwnd)`. On a non-Markdown tab, push the notice "Live Markdown is available for Markdown documents. Choose View > Markdown to treat this tab as Markdown."
- [ ] `markdown_live_default: bool` (default `false`) in `Settings`, `SettingsDelta`, the parser and the recognised-keys comment in `persisted.rs`, and `DEFAULT_MARKDOWN_LIVE_DEFAULT` in `defaults.rs`. `LiveHost::documents` absent → use it. `change_setting`'s re-apply list does not need it (read at sync time).
- [ ] Tests: `ctrl_alt_v_toggles_live_through_the_accelerator` (real `TranslateAcceleratorW` path, as the existing shortcut tests), `ctrl_alt_v_on_a_text_tab_shows_the_notice`, `view_menu_checks_live`, `the_default_setting_opens_markdown_in_live`, `the_default_setting_skips_files_over_the_limit_silently`, plus the updated counts (`keymap`, `menus`, `command_palette`, `shortcuts_model` title tests).
- [ ] Commit `feat(live): Ctrl+Alt+V toggles the Live view, markdown_live_default`.

### Task 12: Typing latency, README, full verification

**Files:** `tests/windows/markdown_live.rs`, `README.md`, the spec.

- [ ] Perf test `typing_in_live_costs_the_same_as_typing_in_source`, `#[ignore]`d with the same command as the preview's: 1 MB `sample_markdown`, caret at 500 000, 200 timed `WM_CHAR` + `UpdateWindow` of the **visible** text view, p95; baseline in source, then in Live; assert `live <= baseline + baseline / 10 + 100`. Also run it on a 10k-line note. Record the numbers in the PR. If it fails, profile (likely suspects: re-laying more than the edited line, full-viewport invalidation on every keystroke, `range_bytes` copies) before touching the budget.
- [ ] README: a short "Live Markdown (preview)" section: Ctrl+Alt+V, what works, that rendering arrives in later releases, `markdown_live_default`.
- [ ] Spec: add "Phase 1 shipped" with the deviations above in a note under §12.
- [ ] Full suite: `cargo clippy --all-targets -- -D warnings`, `cargo test`, `cargo test --test '*' -- --test-threads=1`, and the ignored perf tests for `markdown_live` and `markdown_preview`.
- [ ] Manual check in the real app (back up and restore `fastpad.ini`): a long note with CJK, emoji and RTL text; type, select, undo, IME (Microsoft Japanese IME), split with Live left, zoom, dark theme, 100% and 150% DPI, switching tabs and toggling repeatedly.
- [ ] Commit `docs(live): phase 1 README and spec note`, open the PR.

## Done when

- Ctrl+Alt+V on a Markdown tab shows the Live view; the note can be written, selected, edited, undone, searched, saved and reloaded entirely in it, with IME, and the text is byte-for-byte what the source view would produce.
- Toggling never changes or dirties the document.
- Typing latency in Live is within budget on 1 MB and 10k-line notes, and startup is unchanged.
- The full suite is green on CI.
