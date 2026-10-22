# Preview Text Selection Implementation Plan

Spec: `docs/superpowers/specs/2026-10-22-preview-selection-design.md`

**Goal:** select the Markdown preview's text with the mouse and copy it as plain text and HTML.

## Decisions taken while planning

1. **Runs are what `push_laid_text` and `push_code` draw.** Those two functions are the only
   places a document's text becomes a `DrawOp::Text`. Markers, alert titles, the disclosure
   triangle and image alt boxes use their own layouts and stay unselectable. Each of the two
   records a `LaidRun` and tags its op with the run's index.
2. **The model walk counts `<details>` the way layout does.** Layout skips a collapsed section's
   nested sections with `count_nested`. `text_runs` walks a collapsed section's children
   without emitting runs, so its section counter stays in step, and asks
   `open(index, attribute)` for each section's state.
3. **Ctrl+A is a key, Ctrl+C a command.** Ctrl+A has no accelerator (Scintilla handles it), so
   the preview's `WM_KEYDOWN` takes it. Ctrl+C is an accelerator for `CommandId::Copy`, whose
   dispatch now copies from the preview when a preview has the keyboard focus. The Edit menu's
   Copy goes the same way. Ctrl+Insert is handled as a key.
4. **The context menu is the host's.** The view posts `WM_FASTPAD_PREVIEW_MENU` with the screen
   point and the link under it. `preview_host` builds the menu with `menus::track_popup` and
   calls back into the view, as Live's menu does.
5. **High contrast:** the selection fill is `Highlight`. The text inside it is drawn a second
   time, clipped to the fill, with `HighlightText` as the default brush. Link text keeps its
   own colour there (a drawing effect wins over the default brush). That is accepted, since
   the link colour is the system's `HotTrack`.
6. **`range_rects` moves to `preview/dwrite.rs`** with a `hit_test_point` beside it. Both the
   preview and Live use it.

## Tasks

1. `src/preview/selection.rs`: `TextPoint`, run walk, UTF-16 slicing, word bounds, plain text,
   HTML, the CF_HTML wrapper. Unit tests.
2. Layout: `LaidRun`, `DrawOp::Text { run }`, runs recorded in reading order. A test checks
   they agree with `text_runs`.
3. Colours: `Selection`, `InactiveSelection`, `SelectionText` roles from the editor palette.
4. Rendering: `draw_ops` takes the selected range per run and fills it behind the text.
5. View: selection state, hit testing, mouse (drag, autoscroll, double and triple click, Shift+click,
   links), keys (Ctrl+A, Ctrl+Insert, Escape, Menu, Shift+F10), clearing on edits, reloads and
   toggles; `copy()` and `select_all()`.
6. Clipboard: `set_text_and_html`.
7. Host: the context menu; `CommandId::Copy` while a preview has focus.
8. Window tests, README.
