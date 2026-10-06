# Markdown Live Mode Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a Live Markdown mode that shows rendered Markdown inside the Scintilla editor, with markup revealed on the lines being edited. Also add Markdown writing helpers and key bindings that apply only in Markdown files.

**Architecture:**
- **Styling.** While Live is on, a document stops using Lexilla. FastPad styles it itself, Scintilla's "container lexer" mode, from `pulldown-cmark` source ranges. Block boundaries come from the preview's existing incremental `PreviewDocument`.
- **Hiding markup.** Markup is either hidden (an invisible style with zero width) or blanked (drawn in the background colour so it keeps its width). A painter runs after Scintilla's `WM_PAINT` and draws headings, bullets, checkboxes, quote bars, rules, table grids, fence bars and image thumbnails over the blanked text. Annotation lines reserve the extra height for large headings and images.
- **Purity.** Everything that transforms text is pure and unit tested. Window code only wires Scintilla notifications to it.

**Tech Stack:** Rust; Scintilla 5.6.6 (direct function calls); `pulldown-cmark` 0.13.4 (offset iterator); Direct2D DC render target, DirectWrite and WIC through the `windows` 0.62.2 crate, loaded lazily as the preview loads them.

**Spec:** `docs/superpowers/specs/2026-10-06-markdown-live-mode-design.md`

## Global Constraints

- No new crates. `windows` crate features stay as they are. If a task needs a new `windows` feature, add it to `$AllowedWindowsFeatures` in `tools/audit-dependencies.ps1` in the same commit.
- `d2d1.dll`, `dwrite.dll` and `windowscodecs.dll` must not be loaded until the first Live painter or preview needs them. The existing test `assert_no_preview_imports()` in `tests/windows/markdown_preview.rs` must keep passing.
- Scintilla constants are added only by extending `$RequiredNames` in `tools/generate-scintilla-constants.ps1` and regenerating. `src/editor/scintilla_constants.rs` is never edited by hand.
- Live size limit: `LIVE_MAX_BYTES = 1_048_576` (1 MB).
- Typing latency with Live on must satisfy the preview's rule: `live_p95 <= baseline + baseline / 10 + 100` microseconds.
- No backward-compatibility code: no migrations and no readers for old formats.
- New `CommandId` values start at **261**. Retired numbers (133, 134, 163, 166–173) are never reused.
- Command string IDs, once added, are never renamed.
- Window tests run with `--test-threads=1`. Before running window tests in a worktree, copy `native/out` DLLs into the worktree.
- During tasks, compile with `cargo clippy --all-targets` and run only the tests named in the step. The full suite runs once, in the final task.
- Any run of the real app is wrapped in a backup and restore of `%APPDATA%\FastPad\fastpad.ini`.
- `main` is protected. Work on branch `feat/markdown-live-mode` and merge by PR with the 4 CI checks green.

## Deviations from the spec (decided while planning)

1. **Live is per document, not per tab.** Scintilla stores styles and annotations in the document, and a document shown in two groups shares one Scintilla document. So `Document` gets the Live flag, and every group showing that document is Live. The reveal set comes from the group that last changed selection.
2. **The perf check is an ignored perf test** in `tests/windows/markdown_live.rs`, using the same pattern as `typing_with_split_open_costs_the_same_as_without_a_preview` in `tests/windows/markdown_preview.rs`. `src/perf` holds only startup milestones.
3. **Strikethrough uses a Scintilla indicator** (`INDIC_STRIKE`), because Scintilla styles have no strike attribute.
4. **Shortcuts page:**
   - A Markdown-scoped command's title ends with " (Markdown files)".
   - A Markdown-scoped key that matches a global key is not counted as a conflict, and no "overrides" message is shown.
5. **`markdown_live_default` is set only in `fastpad.ini`.** It gets no Settings dialog checkbox in v1.
6. **Footnote references and `$math$` are plain text, not dimmed.** With FastPad's parse options, pulldown-cmark reports them as ordinary text. Front matter *is* dimmed (Task 4).

## Review Focus

1. **CRLF documents.** Span offsets, reveal lines, list continuation and table padding must work with `\r\n`. Inserted lines use the document's own line ending. *(Tests: Task 3 `crlf_offsets_are_byte_exact`, Task 7 `continuation_uses_the_documents_line_ending`.)*
2. **Multi-byte UTF-8 text** such as `é` and `—`. Scintilla positions are UTF-8 byte offsets, but table padding counts characters. *(Tests: Task 3 `multibyte_text_keeps_byte_offsets`, Task 8 `padding_counts_characters_not_bytes`.)*
3. **Constructs left open while typing**, such as a fence with no closing line or a lone `**`. Incremental styling must equal a full parse at every step. *(Test: Task 4 `typing_an_open_fence_matches_a_full_parse`.)*
4. **Undo after Live actions.** Undoing a checkbox toggle or a table re-format restores the exact text. Revealing lines never adds an undo step and never marks the document modified. *(Tests: Task 13 `reveal_does_not_dirty_or_add_undo`, Task 17 `checkbox_toggle_undoes_in_one_step`.)*
5. **Tab switch, language change and theme change while Live is on.** The style table, lexer and annotations must follow the document now shown. *(Test: Task 12 `switching_tabs_restores_lexilla_and_back`.)*

---

## File Structure

| File | Status | Responsibility |
|---|---|---|
| `tools/generate-scintilla-constants.ps1` | modify | add the Scintilla names Live needs |
| `src/editor/scintilla_constants.rs` | regenerate | — |
| `src/editor/scintilla/live_ops.rs` | create | `Editor` wrappers: styling runs, style attributes, annotations, indicators, geometry |
| `src/editor/hooks.rs` | create | `EditorHooks` trait: paint, mouse, cursor, key hooks called from the editor subclass |
| `src/editor/scintilla.rs` | modify | `mod live_ops;`, hooks slot on `EditorEndpoint`, subclass proc calls the hooks |
| `src/live/mod.rs` | create | module root, `LIVE_MAX_BYTES` |
| `src/live/spans.rs` | create | **pure**: block text → spans + decorations |
| `src/live/blocks.rs` | create | **pure**: `LiveDocument` = `PreviewDocument` + per-block spans, incremental |
| `src/live/reveal.rs` | create | **pure**: selections → revealed lines |
| `src/live/styles.rs` | create | style numbers, `style_for(kind, revealed)`, style-table application |
| `src/live/styler.rs` | create | turns spans plus the reveal set into Scintilla styling runs and indicator ranges |
| `src/live/reserve.rs` | create | annotation-line reservations for headings and images |
| `src/live/painter.rs` | create | DC render target and decoration drawing |
| `src/editor/markdown_edit.rs` | create | **pure** helpers: format toggles, list continuation, nesting, table format and cell navigation |
| `src/window/live_host.rs` | create | per-document Live state, toggle, sync, notification handlers, hooks implementation |
| `src/window/commands.rs` | modify | 5 new commands, `scope()`, `is_markdown_edit()` |
| `src/window/keymap.rs` | modify | scope-aware `command_for` / `conflicts` / `first_text`; new defaults and string IDs |
| `src/window/menus.rs` | modify | accelerator table skips Markdown-scoped bindings; View item with check mark |
| `src/window/main_window/menu_keys.rs` | modify | Markdown-scoped pre-accelerator dispatch |
| `src/window/main_window/command_dispatch.rs` | modify | arms for the new commands |
| `src/window/main_window/wndproc.rs` | modify | `SCN_STYLENEEDED`, selection updates and modifications forwarded to `live_host` |
| `src/window/main_window/language_tools.rs`, `split_groups.rs`, `settings_apply.rs` | modify | `live_host::sync` after styles or settings are applied |
| `src/window/command_palette.rs`, `src/window/main_window/command_palette_ui.rs`, `src/window/shortcuts_model.rs` | modify | entries, availability, titles |
| `src/config/{defaults,persisted}.rs` | modify | `markdown_live_default` |
| `src/document.rs` | modify | `Document.live: Option<bool>` |
| `src/app.rs` | modify | `live: LiveRegistry` |
| `tests/windows/markdown_live.rs` + `Cargo.toml` `[[test]]` | create | integration and perf tests |
| `README.md` | modify | user docs |

---

### Task 1: Scintilla constants and `Editor` live operations

**Files:**
- Modify: `tools/generate-scintilla-constants.ps1` (`$RequiredNames`)
- Regenerate: `src/editor/scintilla_constants.rs`
- Create: `src/editor/scintilla/live_ops.rs`
- Modify: `src/editor/scintilla.rs:79-82` (module list)
- Test: `src/editor/scintilla/tests.rs`

**Interfaces:**
- Produces (all on `Editor`, `#[cfg(windows)]`, each with the usual `#[cfg(not(windows))]` stub returning `Err(FastPadError::Invariant("Scintilla editor is only supported on Windows"))`):
  - `pub fn set_style_visible(&self, style: u32, visible: bool) -> Result<()>`
  - `pub fn set_style_eol_filled(&self, style: u32, filled: bool) -> Result<()>`
  - `pub fn set_style_underline(&self, style: u32, underline: bool) -> Result<()>`
  - `pub fn end_styled(&self) -> Result<usize>`
  - `pub fn apply_styling(&self, start: usize, runs: &[(usize, u8)]) -> Result<()>`: `runs` are `(byte length, style)` pairs
  - `pub fn style_at(&self, position: usize) -> Result<u8>`
  - `pub fn set_annotation_lines(&self, line: usize, count: usize, style: u32) -> Result<()>`: 0 clears
  - `pub fn annotation_lines(&self, line: usize) -> Result<usize>`
  - `pub fn show_annotations(&self, visible: bool) -> Result<()>`
  - `pub fn clear_annotations(&self) -> Result<()>`
  - `pub fn text_height(&self) -> Result<i32>`: pixel height of one display line
  - `pub fn wrap_count(&self, line: usize) -> Result<usize>`
  - `pub fn lines_on_screen(&self) -> Result<usize>`
  - `pub fn point_of(&self, position: usize) -> Result<(i32, i32)>`: client x, y of the top of the line
  - `pub fn position_at(&self, x: i32, y: i32) -> Result<usize>`
  - `pub fn define_strike_indicator(&self, indicator: u32, colour: u32) -> Result<()>`
  - `pub fn set_indicator(&self, indicator: u32, range: Range<usize>, on: bool) -> Result<()>`
  - `pub fn colourise(&self, range: Range<usize>) -> Result<()>`
  - `pub fn line_start(&self, line: usize) -> Result<usize>`, `pub fn line_end(&self, line: usize) -> Result<usize>`: public versions. If `line_ops.rs` already has private ones with these names, make those `pub(crate)` and do not duplicate them.

- [ ] **Step 1: Extend the generator list**

Append to `$RequiredNames` in `tools/generate-scintilla-constants.ps1`, after `"SCK_INSERT", "SCK_DELETE", "SCK_ESCAPE"`:

```powershell
    , "SCI_STYLESETVISIBLE", "SCI_STYLESETEOLFILLED", "SCI_STYLESETUNDERLINE", "SCI_STARTSTYLING",
    "SCI_SETSTYLING", "SCI_GETENDSTYLED", "SCN_STYLENEEDED", "SCI_COLOURISE",
    "SCI_ANNOTATIONSETTEXT", "SCI_ANNOTATIONGETLINES", "SCI_ANNOTATIONSETSTYLE",
    "SCI_ANNOTATIONSETVISIBLE", "SCI_ANNOTATIONCLEARALL", "ANNOTATION_HIDDEN", "ANNOTATION_STANDARD",
    "SCI_TEXTHEIGHT", "SCI_WRAPCOUNT", "SCI_LINESONSCREEN", "SC_UPDATE_SELECTION", "SC_UPDATE_CONTENT",
    "SCI_INDICSETSTYLE", "SCI_INDICSETFORE", "SCI_SETINDICATORCURRENT", "SCI_INDICATORFILLRANGE",
    "SCI_INDICATORCLEARRANGE", "INDIC_STRIKE", "SC_MOD_BEFOREINSERT"
```

(Fold them into the array's existing comma layout. The leading comma above only shows where the list continues.)

- [ ] **Step 2: Regenerate and check**

Run: `pwsh -File tools/generate-scintilla-constants.ps1`
Then: `git diff --stat src/editor/scintilla_constants.rs`
Expected: only additions. Each new name appears once, e.g. `pub const SCN_STYLENEEDED: u32 = 2000;`.

- [ ] **Step 3: Write the failing tests** in `src/editor/scintilla/tests.rs` (append):

```rust
#[test]
fn container_styling_runs_land_on_the_right_bytes() {
    let editor = test_editor();
    editor.set_text("ab**cd**").unwrap();
    editor.set_lexer(0).unwrap();
    editor.apply_styling(0, &[(2, 0), (2, 1), (2, 3), (2, 1)]).unwrap();
    let styles: Vec<u8> = (0..8).map(|at| editor.style_at(at).unwrap()).collect();
    assert_eq!(styles, [0, 0, 1, 1, 3, 3, 1, 1]);
    assert_eq!(editor.end_styled().unwrap(), 8);
}

#[test]
fn annotation_lines_reserve_and_clear() {
    let editor = test_editor();
    editor.set_text("# Title\nbody\n").unwrap();
    editor.show_annotations(true).unwrap();
    editor.set_annotation_lines(0, 2, 40).unwrap();
    assert_eq!(editor.annotation_lines(0).unwrap(), 2);
    editor.set_annotation_lines(0, 0, 40).unwrap();
    assert_eq!(editor.annotation_lines(0).unwrap(), 0);
}

#[test]
fn geometry_round_trips_a_position() {
    let editor = test_editor();
    editor.set_text("hello\nworld\n").unwrap();
    let start = editor.line_start(1).unwrap();
    let (x, y) = editor.point_of(start).unwrap();
    assert_eq!(editor.position_at(x + 1, y + 1).unwrap(), start);
    assert!(editor.text_height().unwrap() > 0);
}
```

- [ ] **Step 4: Run them and watch them fail to compile**

Run: `cargo test --lib editor::scintilla::tests::container_styling -- --test-threads=1`
Expected: compile error, because there is no method `apply_styling`.

- [ ] **Step 5: Implement `src/editor/scintilla/live_ops.rs`**

```rust
//! `Editor`'s operations for Live Markdown (live mode spec §7): container-lexer styling runs,
//! style attributes the language tables never set, annotation lines that reserve height, the
//! strikethrough indicator, and the geometry the decoration painter needs.

use super::*;
use crate::editor::scintilla_constants::{
    ANNOTATION_HIDDEN, ANNOTATION_STANDARD, INDIC_STRIKE, SCI_ANNOTATIONCLEARALL,
    SCI_ANNOTATIONGETLINES, SCI_ANNOTATIONSETSTYLE, SCI_ANNOTATIONSETTEXT,
    SCI_ANNOTATIONSETVISIBLE, SCI_COLOURISE, SCI_GETENDSTYLED, SCI_GETLINEENDPOSITION,
    SCI_GETSTYLEAT, SCI_INDICATORCLEARRANGE, SCI_INDICATORFILLRANGE, SCI_INDICSETFORE,
    SCI_INDICSETSTYLE, SCI_LINESONSCREEN, SCI_POINTXFROMPOSITION, SCI_POINTYFROMPOSITION,
    SCI_POSITIONFROMLINE, SCI_POSITIONFROMPOINT, SCI_SETINDICATORCURRENT, SCI_SETSTYLING,
    SCI_STARTSTYLING, SCI_STYLESETEOLFILLED, SCI_STYLESETUNDERLINE,
    SCI_STYLESETVISIBLE, SCI_TEXTHEIGHT, SCI_WRAPCOUNT,
};
use std::ops::Range;

#[cfg(windows)]
impl Editor {
    fn live_send(&self, message: u32, wparam: usize, lparam: isize) -> Result<isize> {
        self.endpoint.send_direct_checked(message, wparam, lparam)
    }

    pub fn set_style_visible(&self, style: u32, visible: bool) -> Result<()> {
        self.live_send(SCI_STYLESETVISIBLE, style as usize, isize::from(visible)).map(drop)
    }

    pub fn set_style_eol_filled(&self, style: u32, filled: bool) -> Result<()> {
        self.live_send(SCI_STYLESETEOLFILLED, style as usize, isize::from(filled)).map(drop)
    }

    pub fn set_style_underline(&self, style: u32, underline: bool) -> Result<()> {
        self.live_send(SCI_STYLESETUNDERLINE, style as usize, isize::from(underline)).map(drop)
    }

    pub fn end_styled(&self) -> Result<usize> {
        Ok(self.live_send(SCI_GETENDSTYLED, 0, 0)?.max(0) as usize)
    }

    /// Styles `runs` (byte length, style) back to back from `start`.
    pub fn apply_styling(&self, start: usize, runs: &[(usize, u8)]) -> Result<()> {
        self.live_send(SCI_STARTSTYLING, start, 0)?;
        for &(length, style) in runs {
            self.live_send(SCI_SETSTYLING, length, isize::from(style))?;
        }
        Ok(())
    }

    pub fn style_at(&self, position: usize) -> Result<u8> {
        Ok(self.live_send(SCI_GETSTYLEAT, position, 0)? as u8)
    }

    /// Reserves `count` blank display lines under `line` (0 removes them).
    pub fn set_annotation_lines(&self, line: usize, count: usize, style: u32) -> Result<()> {
        if count == 0 {
            return self.live_send(SCI_ANNOTATIONSETTEXT, line, 0).map(drop);
        }
        // Scintilla shows one annotation line per text line; a space keeps each line non-empty.
        let text = vec![" "; count].join("\n");
        let text = std::ffi::CString::new(text).expect("spaces and newlines only");
        self.live_send(SCI_ANNOTATIONSETTEXT, line, text.as_ptr() as isize)?;
        self.live_send(SCI_ANNOTATIONSETSTYLE, line, style as isize).map(drop)
    }

    pub fn annotation_lines(&self, line: usize) -> Result<usize> {
        Ok(self.live_send(SCI_ANNOTATIONGETLINES, line, 0)?.max(0) as usize)
    }

    pub fn show_annotations(&self, visible: bool) -> Result<()> {
        let mode = if visible { ANNOTATION_STANDARD } else { ANNOTATION_HIDDEN };
        self.live_send(SCI_ANNOTATIONSETVISIBLE, mode as usize, 0).map(drop)
    }

    pub fn clear_annotations(&self) -> Result<()> {
        self.live_send(SCI_ANNOTATIONCLEARALL, 0, 0).map(drop)
    }

    pub fn text_height(&self) -> Result<i32> {
        Ok(self.live_send(SCI_TEXTHEIGHT, 0, 0)? as i32)
    }

    pub fn wrap_count(&self, line: usize) -> Result<usize> {
        Ok(self.live_send(SCI_WRAPCOUNT, line, 0)?.max(1) as usize)
    }

    pub fn lines_on_screen(&self) -> Result<usize> {
        Ok(self.live_send(SCI_LINESONSCREEN, 0, 0)?.max(0) as usize)
    }

    pub fn point_of(&self, position: usize) -> Result<(i32, i32)> {
        let x = self.live_send(SCI_POINTXFROMPOSITION, 0, position as isize)? as i32;
        let y = self.live_send(SCI_POINTYFROMPOSITION, 0, position as isize)? as i32;
        Ok((x, y))
    }

    pub fn position_at(&self, x: i32, y: i32) -> Result<usize> {
        Ok(self.live_send(SCI_POSITIONFROMPOINT, x as usize, y as isize)?.max(0) as usize)
    }

    pub fn define_strike_indicator(&self, indicator: u32, colour: u32) -> Result<()> {
        self.live_send(SCI_INDICSETSTYLE, indicator as usize, INDIC_STRIKE as isize)?;
        self.live_send(SCI_INDICSETFORE, indicator as usize, colour as isize).map(drop)
    }

    pub fn set_indicator(&self, indicator: u32, range: Range<usize>, on: bool) -> Result<()> {
        self.live_send(SCI_SETINDICATORCURRENT, indicator as usize, 0)?;
        let message = if on { SCI_INDICATORFILLRANGE } else { SCI_INDICATORCLEARRANGE };
        self.live_send(message, range.start, (range.end - range.start) as isize).map(drop)
    }

    pub fn colourise(&self, range: Range<usize>) -> Result<()> {
        self.live_send(SCI_COLOURISE, range.start, range.end as isize).map(drop)
    }

    pub fn line_start(&self, line: usize) -> Result<usize> {
        Ok(self.live_send(SCI_POSITIONFROMLINE, line, 0)?.max(0) as usize)
    }

    pub fn line_end(&self, line: usize) -> Result<usize> {
        Ok(self.live_send(SCI_GETLINEENDPOSITION, line, 0)?.max(0) as usize)
    }
}
```

Add `#[cfg(not(windows))]` stubs for every method, mirroring how `styling.rs` pairs them. Add `mod live_ops;` after `mod line_ops;` in `src/editor/scintilla.rs`. If `line_ops.rs` already defines private `line_start`/`line_end` on `Editor`, delete the two methods above and make the existing ones `pub` instead.

- [ ] **Step 6: Run the tests**

Run: `cargo test --lib editor::scintilla::tests -- --test-threads=1 container_styling annotation_lines geometry_round`
Expected: 3 passed.

- [ ] **Step 7: Commit**

```bash
git add tools/generate-scintilla-constants.ps1 src/editor/scintilla_constants.rs src/editor/scintilla/live_ops.rs src/editor/scintilla.rs src/editor/scintilla/tests.rs
git commit -m "feat(editor): Scintilla operations for container styling, annotations and geometry"
```

---

### Task 2: Editor hooks for paint, mouse, cursor and keys

The editor subclass knows only its HWND. Live mode and the Markdown helpers need to run code when the editor paints, is clicked, sets its cursor or gets a key. This task adds one trait-object slot to `EditorEndpoint` that the subclass proc consults.

**Files:**
- Create: `src/editor/hooks.rs`
- Modify: `src/editor/mod.rs` (`pub mod hooks;` and `pub use hooks::EditorHooks;`)
- Modify: `src/editor/scintilla.rs` (`EditorEndpoint` field at l.151; subclass proc at l.714-769; imports at l.73)
- Test: `src/editor/scintilla/tests.rs`

**Interfaces:**
- Produces:
```rust
pub trait EditorHooks {
    /// After Scintilla painted `update` (client coordinates).
    fn after_paint(&self, _hwnd: HWND, _update: RECT) {}
    /// A left-button press; true consumes it (Scintilla never sees it).
    fn mouse_down(&self, _x: i32, _y: i32, _ctrl: bool) -> bool { false }
    /// WM_SETCURSOR over the text area; true means the hook set the cursor.
    fn set_cursor(&self, _x: i32, _y: i32, _ctrl: bool) -> bool { false }
    /// A WM_KEYDOWN not taken by an accelerator; true consumes it and its WM_CHAR.
    fn key_down(&self, _vk: u16, _ctrl: bool, _shift: bool, _alt: bool) -> bool { false }
}
```
  - `Editor::set_hooks(&self, hooks: Option<Rc<dyn EditorHooks>>)`
  - `Editor::invalidate(&self)`: `InvalidateRect(hwnd, null, FALSE)`

- [ ] **Step 1: Write the failing test** (append to `src/editor/scintilla/tests.rs`):

```rust
#[test]
fn a_hook_that_takes_enter_also_swallows_its_char() {
    use crate::editor::EditorHooks;
    use std::cell::Cell;
    use std::rc::Rc;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_RETURN;
    use windows_sys::Win32::UI::WindowsAndMessaging::{SendMessageW, WM_CHAR, WM_KEYDOWN};
    struct TakeEnter(Cell<u32>);
    impl EditorHooks for TakeEnter {
        fn key_down(&self, vk: u16, _: bool, _: bool, _: bool) -> bool {
            self.0.set(self.0.get() + 1);
            vk == VK_RETURN
        }
    }
    let editor = test_editor();
    editor.set_text("a").unwrap();
    editor.set_selection(1..1).unwrap();
    let hook = Rc::new(TakeEnter(Cell::new(0)));
    editor.set_hooks(Some(hook.clone()));
    unsafe {
        SendMessageW(editor.hwnd(), WM_KEYDOWN, usize::from(VK_RETURN), 0);
        // What TranslateMessage would post after the key-down.
        SendMessageW(editor.hwnd(), WM_CHAR, 0x0D, 0);
    }
    assert_eq!(hook.0.get(), 1);
    assert_eq!(editor.text().unwrap(), "a");
    editor.set_hooks(None);
    unsafe {
        SendMessageW(editor.hwnd(), WM_KEYDOWN, usize::from(VK_RETURN), 0);
    }
    assert_ne!(editor.text().unwrap(), "a", "without a hook Enter reaches Scintilla");
}
```

- [ ] **Step 2: Run it and watch it fail**

Run: `cargo test --lib a_hook_that_takes_enter -- --test-threads=1`
Expected: compile error, because `set_hooks` is not defined.

- [ ] **Step 3: Implement**

`src/editor/hooks.rs`:

```rust
//! Callbacks the editor subclass makes into the window layer (live mode spec §7): Live Markdown
//! paints decorations after Scintilla and takes checkbox and link clicks; the Markdown helpers
//! take Enter and Tab. The editor knows nothing about documents, so this is the only way in.

use windows_sys::Win32::Foundation::{HWND, RECT};

pub trait EditorHooks {
    /// After Scintilla painted `update` (client coordinates).
    fn after_paint(&self, _hwnd: HWND, _update: RECT) {}
    /// A left-button press; true consumes it (Scintilla never sees it).
    fn mouse_down(&self, _x: i32, _y: i32, _ctrl: bool) -> bool {
        false
    }
    /// `WM_SETCURSOR` over the text area; true means the hook set the cursor.
    fn set_cursor(&self, _x: i32, _y: i32, _ctrl: bool) -> bool {
        false
    }
    /// A `WM_KEYDOWN` no accelerator took; true consumes it and the `WM_CHAR` that follows.
    fn key_down(&self, _vk: u16, _ctrl: bool, _shift: bool, _alt: bool) -> bool {
        false
    }
}
```

In `src/editor/scintilla.rs`:

1. Add two fields to `EditorEndpoint`:
```rust
    hooks: RefCell<Option<Rc<dyn crate::editor::EditorHooks>>>,
    /// The `WM_CHAR` a consumed `WM_KEYDOWN` will produce, dropped when it arrives.
    swallow_char: Cell<Option<u16>>,
```
   Initialize them to `RefCell::new(None)` and `Cell::new(None)` wherever `EditorEndpoint` is constructed.

2. Add methods on `Editor`:
```rust
    pub fn set_hooks(&self, hooks: Option<Rc<dyn crate::editor::EditorHooks>>) {
        *self.endpoint.hooks.borrow_mut() = hooks;
    }

    pub fn invalidate(&self) {
        unsafe { windows_sys::Win32::Graphics::Gdi::InvalidateRect(self.hwnd(), std::ptr::null(), 0) };
    }
```

3. In `editor_endpoint_subclass_proc`, before the existing `WM_CHAR` branch, add:
```rust
    let hooks = endpoint.hooks.borrow().clone();
    if message == WM_CHAR {
        if let Some(expected) = endpoint.swallow_char.take() {
            if expected == wparam as u16 {
                return 0;
            }
        }
    }
    if message == WM_KEYDOWN {
        if let Some(hooks) = &hooks {
            let down = |vk| unsafe { GetKeyState(i32::from(vk)) } < 0;
            let vk = wparam as u16;
            if hooks.key_down(vk, down(VK_CONTROL), down(VK_SHIFT), down(VK_MENU)) {
                // Enter and Tab produce a CR / TAB WM_CHAR after TranslateMessage.
                let produced = match vk {
                    VK_RETURN => Some(0x0D),
                    VK_TAB => Some(0x09),
                    _ => None,
                };
                endpoint.swallow_char.set(produced);
                return 0;
            }
        }
    }
    if message == WM_LBUTTONDOWN {
        if let Some(hooks) = &hooks {
            let (x, y) = (i32::from(lparam as i16), i32::from((lparam >> 16) as i16));
            let ctrl = wparam & MK_CONTROL as usize != 0;
            if hooks.mouse_down(x, y, ctrl) {
                return 0;
            }
        }
    }
    if message == WM_SETCURSOR && (lparam & 0xFFFF) as u32 == HTCLIENT {
        if let Some(hooks) = &hooks {
            let mut point = POINT::default();
            unsafe { GetCursorPos(&mut point) };
            unsafe { ScreenToClient(hwnd, &mut point) };
            let ctrl = unsafe { GetKeyState(i32::from(VK_CONTROL)) } < 0;
            if hooks.set_cursor(point.x, point.y, ctrl) {
                return 1;
            }
        }
    }
    if message == WM_PAINT {
        if let Some(hooks) = &hooks {
            let mut update = RECT::default();
            let has_update = unsafe { GetUpdateRect(hwnd, &mut update, 0) } != 0;
            let result = unsafe { DefSubclassProc(hwnd, message, wparam, lparam) };
            if has_update {
                hooks.after_paint(hwnd, update);
            }
            return result;
        }
    }
```
   - The existing `WM_LBUTTONDOWN` alt-click code stays after the hook check. A consumed click must not begin an Alt+Click.
   - Add the imports: `WM_PAINT`, `WM_SETCURSOR`, `HTCLIENT` (WindowsAndMessaging); `MK_CONTROL`, `VK_RETURN`, `VK_TAB` (KeyboardAndMouse); `GetUpdateRect`, `ScreenToClient` (Gdi); `GetCursorPos` (WindowsAndMessaging); `POINT` and `RECT` (Foundation).
   - The `WM_NCDESTROY` branch must also run `*endpoint.hooks.borrow_mut() = None;` so an `Rc` cycle cannot outlive the window.

- [ ] **Step 4: Run the test**

Run: `cargo test --lib a_hook_that_takes_enter -- --test-threads=1`
Expected: PASS.

- [ ] **Step 5: Run the existing subclass tests, so Alt+Click and Escape still work**

Run: `cargo test --lib editing_shortcuts -- --test-threads=1`
Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add src/editor/hooks.rs src/editor/mod.rs src/editor/scintilla.rs src/editor/scintilla/tests.rs
git commit -m "feat(editor): hooks for paint, mouse, cursor and keys in the editor subclass"
```

---

### Task 3: `live/spans.rs`: block text to spans and decorations (pure)

**Files:**
- Create: `src/live/mod.rs`, `src/live/spans.rs`
- Modify: `src/lib.rs` (add `pub mod live;` next to `pub mod preview;`)

**Interfaces:**
- Consumes: `crate::preview::model::{PARSE_OPTIONS, normalize_label}` (both `pub`).
- Produces (all offsets are **relative to the start of the block text**):
```rust
pub enum SpanKind { Text, Bold, Italic, BoldItalic, InlineCode, CodeBlock, Link, Quote, Dim,
    Marker, Hide, Blank, TableCell, TableHeader, TableBlank, HeadingMarker(u8), HeadingText(u8) }
pub struct Span { pub range: Range<usize>, pub kind: SpanKind }
pub enum Decoration {
    Heading { at: usize, level: u8, text: String },
    Bullet { at: usize, depth: u8 },
    Checkbox { range: Range<usize>, checked: bool },
    QuoteBar { at: usize, depth: u8 },
    Rule { at: usize },
    Fence { at: usize, language: String },
    TableRow { at: usize, pipes: Vec<usize>, header: bool },
    TableDelimiter { at: usize, pipes: Vec<usize> },
    Image { at: usize, dest: String, alt: String },
}
pub struct LinkSpan { pub range: Range<usize>, pub dest: String }
pub struct BlockSpans { pub spans: Vec<Span>, pub decorations: Vec<Decoration>,
    pub links: Vec<LinkSpan>, pub strikes: Vec<Range<usize>> }
pub fn parse_block(text: &str, refs: &dyn Fn(&str) -> Option<String>) -> BlockSpans
pub(crate) fn pipe_positions(line: &str) -> Vec<usize>
pub(crate) fn line_ranges(text: &str, range: Range<usize>) -> Vec<Range<usize>>
```
- `spans` is sorted and non-overlapping, and leaves out `Text`. Gaps are `Text`.
- `at` is the byte offset of the line start for line decorations, and of the marker for `Bullet`.
- `refs` maps a link label, as written, to its destination.

- [ ] **Step 1: Write the failing tests.** Create `src/live/spans.rs` with only the test module, and `src/live/mod.rs`:

```rust
//! Live Markdown (live mode spec): rendered Markdown inside the Scintilla editor.

pub mod spans;

/// Live is unavailable for documents larger than this (live mode spec §4).
pub const LIVE_MAX_BYTES: usize = 1_048_576;
```

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn no_refs(_: &str) -> Option<String> {
        None
    }

    /// One character per byte: the span kind covering it ('.' for Text).
    fn kinds(text: &str) -> String {
        kinds_with(text, &no_refs)
    }

    fn kinds_with(text: &str, refs: &dyn Fn(&str) -> Option<String>) -> String {
        let block = parse_block(text, refs);
        let mut out = vec!['.'; text.len()];
        for span in &block.spans {
            let code = match span.kind {
                SpanKind::Text => '.',
                SpanKind::Bold => 'b',
                SpanKind::Italic => 'i',
                SpanKind::BoldItalic => 'z',
                SpanKind::InlineCode => 'c',
                SpanKind::CodeBlock => 'C',
                SpanKind::Link => 'L',
                SpanKind::Quote => 'q',
                SpanKind::Dim => 'd',
                SpanKind::Marker => 'm',
                SpanKind::Hide => 'H',
                SpanKind::Blank => 'B',
                SpanKind::TableCell => 'x',
                SpanKind::TableHeader => 'X',
                SpanKind::TableBlank => '|',
                SpanKind::HeadingMarker(_) => '#',
                SpanKind::HeadingText(_) => 'h',
            };
            for slot in &mut out[span.range.clone()] {
                *slot = code;
            }
        }
        out.into_iter().collect()
    }

    #[test]
    fn strong_hides_its_markers() {
        assert_eq!(kinds("a **b** c"), "..HHbHH..");
    }

    #[test]
    fn nested_emphasis_is_bold_italic() {
        assert_eq!(kinds("***x***"), "HHHzHHH");
    }

    #[test]
    fn strikethrough_hides_markers_and_records_the_strike() {
        let block = parse_block("~~a~~", &no_refs);
        assert_eq!(kinds("~~a~~"), "HH.HH");
        assert_eq!(block.strikes, vec![2..3]);
    }

    #[test]
    fn inline_code_hides_backticks() {
        assert_eq!(kinds("`x`"), "HcH");
        assert_eq!(kinds("``a`b``"), "HHcccHH");
    }

    #[test]
    fn inline_link_shows_only_its_text() {
        let block = parse_block("[a](u)", &no_refs);
        assert_eq!(kinds("[a](u)"), "HLHHHH");
        assert_eq!(block.links.len(), 1);
        assert_eq!(block.links[0].range, 0..6);
        assert_eq!(block.links[0].dest, "u");
    }

    #[test]
    fn reference_links_resolve_through_refs() {
        let refs = |label: &str| (label == "r").then(|| "https://x".to_owned());
        assert_eq!(kinds_with("[a][r]", &refs), "HLHHHH");
        assert_eq!(parse_block("[a][r]", &refs).links[0].dest, "https://x");
    }

    #[test]
    fn autolink_hides_angle_brackets() {
        assert_eq!(kinds("<http://a>"), "HLLLLLLLLH");
    }

    #[test]
    fn atx_heading_splits_marker_and_text() {
        let block = parse_block("## Hi ##", &no_refs);
        assert_eq!(kinds("## Hi ##"), "###hh###");
        assert!(matches!(
            &block.decorations[..],
            [Decoration::Heading { at: 0, level: 2, text }] if text == "Hi"
        ));
    }

    #[test]
    fn setext_heading_hides_its_underline() {
        assert_eq!(kinds("Hi\n=="), "hh.##");
    }

    #[test]
    fn emphasis_inside_a_heading_is_heading_text() {
        assert_eq!(kinds("# a *b*"), "##hhhhh");
    }

    #[test]
    fn bullets_are_blanked_and_decorated() {
        let block = parse_block("- a\n- b", &no_refs);
        assert_eq!(kinds("- a\n- b"), "B...B..");
        let bullets: Vec<_> = block
            .decorations
            .iter()
            .filter_map(|d| match d {
                Decoration::Bullet { at, depth } => Some((*at, *depth)),
                _ => None,
            })
            .collect();
        assert_eq!(bullets, [(0, 1), (4, 1)]);
    }

    #[test]
    fn nested_bullets_carry_their_depth() {
        let block = parse_block("- a\n  - b", &no_refs);
        assert!(block.decorations.contains(&Decoration::Bullet { at: 6, depth: 2 }));
    }

    #[test]
    fn ordered_numbers_stay_visible_as_markers() {
        assert_eq!(kinds("1. a"), "mm..");
    }

    #[test]
    fn a_checked_task_is_a_checkbox_with_dim_struck_text() {
        let block = parse_block("- [x] done", &no_refs);
        assert_eq!(kinds("- [x] done"), "B.BBB.dddd");
        assert!(block.decorations.contains(&Decoration::Checkbox { range: 2..5, checked: true }));
        assert!(!block.decorations.iter().any(|d| matches!(d, Decoration::Bullet { .. })));
        assert_eq!(block.strikes, vec![6..10]);
    }

    #[test]
    fn quote_markers_are_blanked_with_a_bar_per_line() {
        let block = parse_block("> a\n> b", &no_refs);
        assert_eq!(kinds("> a\n> b"), "B.q.B.q");
        assert!(block.decorations.contains(&Decoration::QuoteBar { at: 0, depth: 1 }));
        assert!(block.decorations.contains(&Decoration::QuoteBar { at: 4, depth: 1 }));
    }

    #[test]
    fn nested_quote_depth_counts_markers() {
        let block = parse_block("> > a", &no_refs);
        assert!(block.decorations.contains(&Decoration::QuoteBar { at: 0, depth: 2 }));
        assert_eq!(kinds("> > a"), "B.B.q");
    }

    #[test]
    fn fenced_code_blanks_fences_and_records_the_language() {
        let text = "```rs\nx\n```";
        let block = parse_block(text, &no_refs);
        assert_eq!(kinds(text), "BBBBB.C.BBB");
        assert!(block.decorations.contains(&Decoration::Fence { at: 0, language: "rs".into() }));
    }

    #[test]
    fn an_unclosed_fence_styles_the_rest_as_code() {
        assert_eq!(kinds("```\nx"), "BBB.C");
    }

    #[test]
    fn fenced_code_inside_a_quote_keeps_the_quote_marker_blank() {
        // The space after a `>` inside the code takes the code style; only the `>` stays blank.
        assert_eq!(kinds("> ```\n> x\n> ```"), "B.BBB.BCC.B.BBB");
    }

    #[test]
    fn indented_code_is_code() {
        assert_eq!(kinds("    x"), "CCCCC");
    }

    #[test]
    fn table_pipes_and_delimiter_are_blank() {
        let text = "|a|b|\n|-|-|\n|c|d|";
        let block = parse_block(text, &no_refs);
        assert_eq!(kinds(text), "|X|X|.|||||.|x|x|");
        assert!(block.decorations.contains(&Decoration::TableRow {
            at: 0,
            pipes: vec![0, 2, 4],
            header: true
        }));
        assert!(block.decorations.contains(&Decoration::TableDelimiter { at: 6, pipes: vec![6, 8, 10] }));
    }

    #[test]
    fn escaped_and_code_pipes_are_cell_text() {
        assert_eq!(pipe_positions(r"|a\|b|"), vec![0, 5]);
        assert_eq!(pipe_positions("|`a|b`|"), vec![0, 6]);
    }

    #[test]
    fn thematic_break_is_blank_with_a_rule() {
        let block = parse_block("---", &no_refs);
        assert_eq!(kinds("---"), "BBB");
        assert_eq!(block.decorations, vec![Decoration::Rule { at: 0 }]);
    }

    #[test]
    fn image_shows_dim_alt_text_and_records_the_image() {
        let text = "![a](p.png)";
        let block = parse_block(text, &no_refs);
        assert_eq!(kinds(text), "HHdHHHHHHHH");
        assert!(block.decorations.contains(&Decoration::Image {
            at: 0,
            dest: "p.png".into(),
            alt: "a".into()
        }));
    }

    #[test]
    fn inline_html_is_dim() {
        assert_eq!(kinds("a<br>"), ".dddd");
    }

    #[test]
    fn crlf_offsets_are_byte_exact() {
        assert_eq!(kinds("**a**\r\nb"), "HHbHH...");
        assert_eq!(kinds("# a\r\n# b"), "##h..##h");
    }

    #[test]
    fn multibyte_text_keeps_byte_offsets() {
        // 'é' is two bytes.
        assert_eq!(kinds("é **b**"), "...HHbHH");
    }
}
```

- [ ] **Step 2: Run the tests and watch them fail**

Run: `cargo test --lib live::spans`
Expected: compile errors, because `parse_block`, `SpanKind` and the rest do not exist.

- [ ] **Step 3: Implement** (above the test module in `src/live/spans.rs`):

```rust
//! Block text → the styled spans and painted decorations Live Markdown shows for it (live mode
//! spec §6). Pure: offsets are bytes relative to the block text, and the caller adds the block's
//! position. Spans are flattened so markup (hidden, blanked, marker) wins over the content kind
//! underneath, which is what keeps a quote's `>` blank inside a code block.

use crate::preview::model::PARSE_OPTIONS;
use pulldown_cmark::{BrokenLink, CodeBlockKind, CowStr, Event, LinkType, Parser, Tag, TagEnd};
use std::ops::Range;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpanKind {
    Text,
    Bold,
    Italic,
    BoldItalic,
    InlineCode,
    CodeBlock,
    Link,
    Quote,
    Dim,
    Marker,
    Hide,
    Blank,
    TableCell,
    TableHeader,
    TableBlank,
    HeadingMarker(u8),
    HeadingText(u8),
}

impl SpanKind {
    /// Markup outranks the content it sits in when spans overlap.
    const fn priority(self) -> u8 {
        match self {
            Self::Hide | Self::Blank | Self::TableBlank | Self::Marker | Self::HeadingMarker(_) => 2,
            _ => 1,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Span {
    pub range: Range<usize>,
    pub kind: SpanKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Decoration {
    Heading { at: usize, level: u8, text: String },
    Bullet { at: usize, depth: u8 },
    Checkbox { range: Range<usize>, checked: bool },
    QuoteBar { at: usize, depth: u8 },
    Rule { at: usize },
    Fence { at: usize, language: String },
    TableRow { at: usize, pipes: Vec<usize>, header: bool },
    TableDelimiter { at: usize, pipes: Vec<usize> },
    Image { at: usize, dest: String, alt: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LinkSpan {
    pub range: Range<usize>,
    pub dest: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct BlockSpans {
    pub spans: Vec<Span>,
    pub decorations: Vec<Decoration>,
    pub links: Vec<LinkSpan>,
    pub strikes: Vec<Range<usize>>,
}

pub fn parse_block(text: &str, refs: &dyn Fn(&str) -> Option<String>) -> BlockSpans {
    let resolve = |link: BrokenLink<'_>| {
        refs(&link.reference).map(|dest| (CowStr::from(dest), CowStr::from("")))
    };
    let parser =
        Parser::new_with_broken_link_callback(text, PARSE_OPTIONS, Some(resolve)).into_offset_iter();
    let mut collector = Collector::new(text);
    for (event, range) in parser {
        collector.event(event, range);
    }
    collector.finish()
}

/// Byte offsets of the column separators in one table line: unescaped pipes outside code spans.
pub(crate) fn pipe_positions(line: &str) -> Vec<usize> {
    let bytes = line.as_bytes();
    let mut pipes = Vec::new();
    let mut index = 0;
    let mut code_run: Option<usize> = None;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' if code_run.is_none() => index += 2,
            b'`' => {
                let run = bytes[index..].iter().take_while(|b| **b == b'`').count();
                code_run = match code_run {
                    None => Some(run),
                    Some(open) if open == run => None,
                    other => other,
                };
                index += run;
            }
            b'|' if code_run.is_none() => {
                pipes.push(index);
                index += 1;
            }
            _ => index += 1,
        }
    }
    pipes
}

/// The lines overlapping `range`, each without its line ending.
pub(crate) fn line_ranges(text: &str, range: Range<usize>) -> Vec<Range<usize>> {
    let mut lines = Vec::new();
    let mut start = line_start(text, range.start);
    while start < range.end.max(start + 1) && start <= text.len() {
        let end = text[start..].find('\n').map_or(text.len(), |at| start + at);
        let content_end = if end > start && text.as_bytes()[end - 1] == b'\r' { end - 1 } else { end };
        lines.push(start..content_end);
        if end >= text.len() {
            break;
        }
        start = end + 1;
    }
    lines
}

fn line_start(text: &str, at: usize) -> usize {
    text[..at].rfind('\n').map_or(0, |newline| newline + 1)
}

struct OpenLink {
    whole: Range<usize>,
    dest: String,
    autolink: bool,
    text_end: usize,
}

struct OpenImage {
    whole: Range<usize>,
    dest: String,
    alt: String,
    alt_end: usize,
}

struct OpenItem {
    marker: usize,
    checked: bool,
}

struct Collector<'a> {
    text: &'a str,
    raw: Vec<Span>,
    out: BlockSpans,
    strong: u32,
    emphasis: u32,
    quote: u32,
    checked: u32,
    list_depth: u8,
    /// Inside a heading, code block, table or HTML block: inline events add no spans.
    suppress: u32,
    heading: Option<(u8, Range<usize>, String)>,
    links: Vec<OpenLink>,
    image: Option<OpenImage>,
    items: Vec<OpenItem>,
}

impl<'a> Collector<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            text,
            raw: Vec::new(),
            out: BlockSpans::default(),
            strong: 0,
            emphasis: 0,
            quote: 0,
            checked: 0,
            list_depth: 0,
            suppress: 0,
            heading: None,
            links: Vec::new(),
            image: None,
            items: Vec::new(),
        }
    }

    fn push(&mut self, range: Range<usize>, kind: SpanKind) {
        if range.start < range.end && range.end <= self.text.len() {
            self.raw.push(Span { range, kind });
        }
    }

    fn line_start(&self, at: usize) -> usize {
        line_start(self.text, at)
    }

    fn inline_kind(&self) -> SpanKind {
        if self.checked > 0 {
            return SpanKind::Dim;
        }
        if !self.links.is_empty() {
            return SpanKind::Link;
        }
        match (self.strong > 0, self.emphasis > 0) {
            (true, true) => SpanKind::BoldItalic,
            (true, false) => SpanKind::Bold,
            (false, true) => SpanKind::Italic,
            (false, false) if self.quote > 0 => SpanKind::Quote,
            (false, false) => SpanKind::Text,
        }
    }

    fn event(&mut self, event: Event<'a>, range: Range<usize>) {
        if !matches!(event, Event::Start(Tag::Link { .. }) | Event::End(TagEnd::Link)) {
            if let Some(link) = self.links.last_mut() {
                link.text_end = link.text_end.max(range.end);
            }
        }
        if !matches!(event, Event::Start(Tag::Image { .. }) | Event::End(TagEnd::Image)) {
            if let Some(image) = &mut self.image {
                image.alt_end = image.alt_end.max(range.end);
            }
        }
        match event {
            Event::Start(tag) => self.start(tag, range),
            Event::End(tag) => self.end(tag, range),
            Event::Text(text) => self.text(&text, range),
            Event::Code(_) => self.inline_code(range),
            Event::InlineHtml(_) | Event::Html(_) if self.suppress == 0 => {
                self.push(range, SpanKind::Dim)
            }
            Event::TaskListMarker(checked) => self.task(checked, range),
            Event::Rule => {
                let at = self.line_start(range.start);
                let line = line_ranges(self.text, range).remove(0);
                self.push(line, SpanKind::Blank);
                self.out.decorations.push(Decoration::Rule { at });
            }
            _ => {}
        }
    }

    fn text(&mut self, text: &str, range: Range<usize>) {
        if let Some((_, _, heading)) = &mut self.heading {
            heading.push_str(text);
            return;
        }
        if let Some(image) = &mut self.image {
            image.alt.push_str(text);
            self.push(range, SpanKind::Dim);
            return;
        }
        if self.suppress > 0 {
            return;
        }
        let kind = self.inline_kind();
        if kind != SpanKind::Text {
            self.push(range, kind);
        }
    }

    fn inline_code(&mut self, range: Range<usize>) {
        if let Some((_, _, heading)) = &mut self.heading {
            heading.push_str(self.text[range].trim_matches('`'));
            return;
        }
        if self.suppress > 0 {
            return;
        }
        let ticks = self.text[range.clone()].bytes().take_while(|b| *b == b'`').count();
        self.push(range.start..range.start + ticks, SpanKind::Hide);
        self.push(range.start + ticks..range.end - ticks, SpanKind::InlineCode);
        self.push(range.end - ticks..range.end, SpanKind::Hide);
    }

    fn wrap(&mut self, range: &Range<usize>, width: usize) {
        if self.suppress > 0 || range.end - range.start < 2 * width {
            return;
        }
        self.push(range.start..range.start + width, SpanKind::Hide);
        self.push(range.end - width..range.end, SpanKind::Hide);
    }

    fn start(&mut self, tag: Tag<'a>, range: Range<usize>) {
        match tag {
            Tag::Heading { level, .. } => {
                self.suppress += 1;
                self.heading = Some((level as u8, range, String::new()));
            }
            Tag::Strong => {
                self.wrap(&range, 2);
                self.strong += 1;
            }
            Tag::Emphasis => {
                self.wrap(&range, 1);
                self.emphasis += 1;
            }
            Tag::Strikethrough => {
                let width = if self.text[range.clone()].starts_with("~~") { 2 } else { 1 };
                self.wrap(&range, width);
                if self.suppress == 0 {
                    self.out.strikes.push(range.start + width..range.end - width);
                }
            }
            Tag::Link { link_type, dest_url, .. } => {
                let autolink = matches!(link_type, LinkType::Autolink | LinkType::Email);
                if self.suppress == 0 {
                    self.push(range.start..range.start + 1, SpanKind::Hide);
                }
                self.links.push(OpenLink {
                    whole: range.clone(),
                    dest: dest_url.to_string(),
                    autolink,
                    text_end: range.start + 1,
                });
            }
            Tag::Image { dest_url, .. } => {
                if self.suppress == 0 {
                    self.push(range.start..range.start + 2, SpanKind::Hide);
                }
                self.image = Some(OpenImage {
                    whole: range.clone(),
                    dest: dest_url.to_string(),
                    alt: String::new(),
                    alt_end: range.start + 2,
                });
            }
            Tag::List(_) => self.list_depth += 1,
            Tag::Item => self.item(range),
            Tag::BlockQuote(_) => {
                if self.quote == 0 {
                    self.quote_markers(range);
                }
                self.quote += 1;
            }
            Tag::CodeBlock(kind) => {
                self.suppress += 1;
                self.code_block(kind, range);
            }
            Tag::Table(_) => {
                self.suppress += 1;
                self.table(range);
            }
            Tag::HtmlBlock => {
                self.push(range, SpanKind::Dim);
                self.suppress += 1;
            }
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd, range: Range<usize>) {
        match tag {
            TagEnd::Heading(_) => {
                self.suppress -= 1;
                if let Some((level, whole, text)) = self.heading.take() {
                    self.heading_spans(level, whole, text);
                }
            }
            TagEnd::Strong => self.strong -= 1,
            TagEnd::Emphasis => self.emphasis -= 1,
            TagEnd::Link => {
                if let Some(link) = self.links.pop() {
                    if self.suppress == 0 {
                        let tail = if link.autolink { link.whole.end - 1 } else { link.text_end };
                        self.push(tail..link.whole.end, SpanKind::Hide);
                        if link.autolink {
                            self.push(link.whole.start + 1..link.whole.end - 1, SpanKind::Link);
                        }
                    }
                    self.out.links.push(LinkSpan { range: link.whole, dest: link.dest });
                }
            }
            TagEnd::Image => {
                if let Some(image) = self.image.take() {
                    if self.suppress == 0 {
                        self.push(image.alt_end..image.whole.end, SpanKind::Hide);
                    }
                    let at = self.line_start(image.whole.start);
                    self.out.decorations.push(Decoration::Image {
                        at,
                        dest: image.dest,
                        alt: image.alt,
                    });
                }
            }
            TagEnd::List(_) => self.list_depth -= 1,
            TagEnd::Item => {
                if self.items.pop().is_some_and(|item| item.checked) {
                    self.checked -= 1;
                }
            }
            TagEnd::BlockQuote(_) => self.quote -= 1,
            TagEnd::CodeBlock | TagEnd::Table | TagEnd::HtmlBlock => self.suppress -= 1,
            _ => {}
        }
        let _ = range;
    }

    fn item(&mut self, range: Range<usize>) {
        let bytes = self.text.as_bytes();
        let mut marker = range.start;
        while marker < range.end && matches!(bytes[marker], b' ' | b'\t') {
            marker += 1;
        }
        if marker < range.end && matches!(bytes[marker], b'-' | b'*' | b'+') {
            self.push(marker..marker + 1, SpanKind::Blank);
            self.out.decorations.push(Decoration::Bullet { at: marker, depth: self.list_depth });
        } else {
            let digits = bytes[marker..range.end].iter().take_while(|b| b.is_ascii_digit()).count();
            self.push(marker..marker + digits + 1, SpanKind::Marker);
        }
        self.items.push(OpenItem { marker, checked: false });
    }

    fn task(&mut self, checked: bool, range: Range<usize>) {
        self.push(range.clone(), SpanKind::Blank);
        if let Some(item) = self.items.last_mut() {
            let marker = item.marker;
            self.out
                .decorations
                .retain(|d| !matches!(d, Decoration::Bullet { at, .. } if *at == marker));
            if checked {
                item.checked = true;
                self.checked += 1;
                let line_end = line_ranges(self.text, range.end..range.end)[0].end;
                let text_start = (range.end + 1).min(line_end);
                if text_start < line_end {
                    self.out.strikes.push(text_start..line_end);
                }
            }
        }
        self.out.decorations.push(Decoration::Checkbox { range, checked });
    }

    fn quote_markers(&mut self, range: Range<usize>) {
        let mut previous_depth = 1;
        for line in line_ranges(self.text, range) {
            let bytes = self.text.as_bytes();
            let mut at = line.start;
            let mut depth = 0u8;
            loop {
                let mut probe = at;
                while probe < line.end && probe - at < 3 && bytes[probe] == b' ' {
                    probe += 1;
                }
                if probe < line.end && bytes[probe] == b'>' {
                    self.push(probe..probe + 1, SpanKind::Blank);
                    depth += 1;
                    at = probe + 1;
                    if at < line.end && bytes[at] == b' ' {
                        at += 1;
                    }
                } else {
                    break;
                }
            }
            // A lazy continuation line has no `>` but still belongs to the quote.
            let depth = if depth == 0 { previous_depth } else { depth };
            previous_depth = depth;
            self.out.decorations.push(Decoration::QuoteBar { at: line.start, depth });
        }
    }

    fn code_block(&mut self, kind: CodeBlockKind<'a>, range: Range<usize>) {
        let lines = line_ranges(self.text, range);
        let CodeBlockKind::Fenced(language) = kind else {
            for line in lines {
                self.push(line, SpanKind::CodeBlock);
            }
            return;
        };
        let content_start = |line: &Range<usize>| {
            let text = &self.text[line.clone()];
            // Skip a container's `>` prefix: the code starts at the first fence or code byte.
            let skip = text.find(|c: char| c != ' ' && c != '>').unwrap_or(text.len());
            line.start + skip
        };
        let is_fence = |line: &Range<usize>, start: usize| {
            let rest = self.text[start..line.end].trim_end();
            rest.len() >= 3 && (rest.bytes().all(|b| b == b'`') || rest.bytes().all(|b| b == b'~'))
        };
        let last = lines.len() - 1;
        for (index, line) in lines.iter().enumerate() {
            let start = content_start(line);
            if index == 0 {
                self.push(start..line.end, SpanKind::Blank);
                self.out.decorations.push(Decoration::Fence {
                    at: line.start,
                    language: language.to_string(),
                });
            } else if index == last && is_fence(line, start) {
                self.push(start..line.end, SpanKind::Blank);
            } else {
                self.push(line.clone(), SpanKind::CodeBlock);
            }
        }
    }

    fn table(&mut self, range: Range<usize>) {
        for (index, line) in line_ranges(self.text, range).into_iter().enumerate() {
            if index == 1 {
                self.push(line.clone(), SpanKind::TableBlank);
                let pipes = pipe_positions(&self.text[line.clone()])
                    .into_iter()
                    .map(|pipe| line.start + pipe)
                    .collect();
                self.out.decorations.push(Decoration::TableDelimiter { at: line.start, pipes });
                continue;
            }
            let kind = if index == 0 { SpanKind::TableHeader } else { SpanKind::TableCell };
            self.push(line.clone(), kind);
            let pipes: Vec<usize> = pipe_positions(&self.text[line.clone()])
                .into_iter()
                .map(|pipe| line.start + pipe)
                .collect();
            for pipe in &pipes {
                self.push(*pipe..*pipe + 1, SpanKind::TableBlank);
            }
            self.out.decorations.push(Decoration::TableRow {
                at: line.start,
                pipes,
                header: index == 0,
            });
        }
    }

    fn heading_spans(&mut self, level: u8, whole: Range<usize>, text: String) {
        let at = self.line_start(whole.start);
        let lines = line_ranges(self.text, whole.clone());
        let first = lines[0].clone();
        let source = &self.text[first.clone()];
        if source.trim_start().starts_with('#') {
            let lead = source.len() - source.trim_start().len();
            let hashes = source[lead..].bytes().take_while(|b| *b == b'#').count();
            let gap = source[lead + hashes..]
                .bytes()
                .take_while(|b| matches!(b, b' ' | b'\t'))
                .count();
            let content_start = first.start + lead + hashes + gap;
            let trimmed_end = first.start + source.trim_end().len();
            let body = &self.text[content_start..trimmed_end.max(content_start)];
            let closing = body.bytes().rev().take_while(|b| *b == b'#').count();
            let before_closing = &body[..body.len() - closing];
            let content_end = if closing > 0
                && (before_closing.is_empty() || before_closing.ends_with([' ', '\t']))
            {
                content_start + before_closing.trim_end().len()
            } else {
                trimmed_end.max(content_start)
            };
            self.push(first.start..content_start, SpanKind::HeadingMarker(level));
            self.push(content_start..content_end, SpanKind::HeadingText(level));
            self.push(content_end..first.end, SpanKind::HeadingMarker(level));
        } else {
            let last = lines.len() - 1;
            for (index, line) in lines.into_iter().enumerate() {
                let kind = if index == last {
                    SpanKind::HeadingMarker(level)
                } else {
                    SpanKind::HeadingText(level)
                };
                self.push(line, kind);
            }
        }
        self.out.decorations.push(Decoration::Heading { at, level, text });
    }

    fn finish(mut self) -> BlockSpans {
        // Flatten: per byte, the highest priority wins; equal priority, the later push wins.
        let mut slots: Vec<Option<SpanKind>> = vec![None; self.text.len()];
        for span in &self.raw {
            for slot in &mut slots[span.range.clone()] {
                if slot.is_none_or(|current| current.priority() <= span.kind.priority()) {
                    *slot = Some(span.kind);
                }
            }
        }
        let mut index = 0;
        while index < slots.len() {
            let Some(kind) = slots[index] else {
                index += 1;
                continue;
            };
            let start = index;
            while index < slots.len() && slots[index] == Some(kind) {
                index += 1;
            }
            if kind != SpanKind::Text {
                self.out.spans.push(Span { range: start..index, kind });
            }
        }
        self.out
    }
}
```

(`Option::is_none_or` is stable since Rust 1.82. If the toolchain is older, use `slot.map_or(true, |current| ...)`.)

- [ ] **Step 4: Run the tests**

Run: `cargo test --lib live::spans`
Expected: all pass.
- If a pulldown-cmark range differs from what a test assumes (for example, where an item's range starts inside a quote), print `Parser::new_ext(text, PARSE_OPTIONS).into_offset_iter()` for that input in a scratch test.
- Then fix the **collector**. Never fix the expected string. The expected strings show the rendered result the spec requires.

- [ ] **Step 5: Commit**

```bash
git add src/live/mod.rs src/live/spans.rs src/lib.rs
git commit -m "feat(live): block spans and decorations from pulldown-cmark offsets"
```

---

### Task 4: `live/blocks.rs`: incremental `LiveDocument` (pure)

**Files:**
- Create: `src/live/blocks.rs`
- Modify: `src/live/mod.rs` (`pub mod blocks;`)
- Modify: `src/preview/incremental.rs` (add the accessor `PreviewDocument::refdef_dest`)

**Interfaces:**
- Consumes:
  - `crate::preview::incremental::{PreviewDocument, Edit, Update, SourceText}`
  - `crate::preview::model::normalize_label`
  - `spans::{parse_block, BlockSpans, SpanKind}`
- Produces:
```rust
pub struct LiveDocument { /* preview: PreviewDocument, spans: Vec<BlockSpans> */ }
impl LiveDocument {
    pub fn parse(source: &str) -> Self;
    /// Applies edits (already in `source`); returns the byte range whose styling changed.
    pub fn apply(&mut self, source: &(impl SourceText + ?Sized), edits: &[Edit]) -> Range<usize>;
    /// Blocks overlapping `bytes`: (absolute block range, spans relative to its start).
    pub fn blocks_in(&self, bytes: Range<usize>) -> impl Iterator<Item = (Range<usize>, &BlockSpans)>;
    /// Absolute spans overlapping `bytes`, sorted.
    pub fn spans_in(&self, bytes: Range<usize>) -> Vec<(Range<usize>, SpanKind)>;
}
```
- Added to `PreviewDocument`: `pub fn refdef_dest(&self, label: &str) -> Option<&str>`. It compares `normalize_label(label)` against `RefDef.key`.

- [ ] **Step 1: Write the failing tests** (create `src/live/blocks.rs` with only the test module):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::preview::incremental::Edit;

    /// Applies `insert` at `at` to `text` and to `doc` the way SCN_MODIFIED reports it.
    fn type_at(text: &mut String, doc: &mut LiveDocument, at: usize, insert: &str) {
        text.insert_str(at, insert);
        let edit = Edit {
            position: at,
            removed: 0,
            inserted: insert.len(),
            lines_delta: insert.matches('\n').count() as isize,
        };
        doc.apply(text.as_str(), &[edit]);
    }

    fn delete_at(text: &mut String, doc: &mut LiveDocument, at: usize, length: usize) {
        let removed: String = text.drain(at..at + length).collect();
        let edit = Edit {
            position: at,
            removed: length,
            inserted: 0,
            lines_delta: -(removed.matches('\n').count() as isize),
        };
        doc.apply(text.as_str(), &[edit]);
    }

    fn assert_matches_full_parse(text: &str, doc: &LiveDocument) {
        let full = LiveDocument::parse(text);
        assert_eq!(doc.spans_in(0..text.len()), full.spans_in(0..text.len()), "text: {text:?}");
    }

    #[test]
    fn spans_are_absolute() {
        let doc = LiveDocument::parse("para\n\n**b**\n");
        assert_eq!(
            doc.spans_in(0..12),
            vec![(6..8, SpanKind::Hide), (8..9, SpanKind::Bold), (9..11, SpanKind::Hide)]
        );
    }

    #[test]
    fn typing_an_open_fence_matches_a_full_parse() {
        let mut text = String::from("a\n\nb\n\n**c**\n");
        let mut doc = LiveDocument::parse(&text);
        let mut at = 3;
        for piece in ["`", "`", "`", "r", "s", "\n", "x", "\n"] {
            type_at(&mut text, &mut doc, at, piece);
            at += piece.len();
            assert_matches_full_parse(&text, &doc);
        }
        type_at(&mut text, &mut doc, at, "```\n");
        assert_matches_full_parse(&text, &doc);
    }

    #[test]
    fn typing_a_lone_strong_marker_matches_a_full_parse() {
        let mut text = String::from("one\n\ntwo three\n");
        let mut doc = LiveDocument::parse(&text);
        for (at, piece) in [(5, "*"), (6, "*"), (11, "*"), (12, "*")] {
            type_at(&mut text, &mut doc, at, piece);
            assert_matches_full_parse(&text, &doc);
        }
        delete_at(&mut text, &mut doc, 5, 2);
        assert_matches_full_parse(&text, &doc);
    }

    #[test]
    fn reference_links_use_definitions_elsewhere_in_the_document() {
        let doc = LiveDocument::parse("[a][r]\n\n[r]: https://x\n");
        let (_, block) = doc.blocks_in(0..1).next().unwrap();
        assert_eq!(block.links[0].dest, "https://x");
    }

    #[test]
    fn apply_reports_the_changed_bytes() {
        let mut text = String::from("a\n\nb\n");
        let mut doc = LiveDocument::parse(&text);
        text.insert(3, '*');
        let changed = doc.apply(
            text.as_str(),
            &[Edit { position: 3, removed: 0, inserted: 1, lines_delta: 0 }],
        );
        assert!(changed.start <= 3 && changed.end >= 5, "{changed:?}");
    }
}
```

- [ ] **Step 2: Run them and watch them fail**

Run: `cargo test --lib live::blocks`
Expected: compile error, because `LiveDocument` is not defined.

- [ ] **Step 3: Add the accessor to `PreviewDocument`** in `src/preview/incremental.rs`, inside `impl PreviewDocument`:

```rust
    /// The destination of the reference definition `label` names, as a broken-link callback
    /// resolves it (live mode spec §7: Live parses one block at a time).
    pub fn refdef_dest(&self, label: &str) -> Option<&str> {
        let key = crate::preview::model::normalize_label(label);
        self.refdefs
            .iter()
            .find(|definition| definition.key == key)
            .map(|definition| definition.dest.as_str())
    }
```

- [ ] **Step 4: Implement `LiveDocument`** (above the tests):

```rust
//! The Live document model (live mode spec §7): the preview's incremental block list decides
//! which blocks an edit touched, and each block keeps its spans relative to its own start, so
//! untouched blocks never reparse even when an edit above them shifts their position.

use super::spans::{BlockSpans, SpanKind, parse_block};
use crate::preview::incremental::{Edit, PreviewDocument, SourceText, Update};
use std::ops::Range;

#[derive(Debug, Default)]
pub struct LiveDocument {
    preview: PreviewDocument,
    spans: Vec<BlockSpans>,
}

impl LiveDocument {
    pub fn parse(source: &str) -> Self {
        let preview = PreviewDocument::parse(source);
        let mut document = Self { preview, spans: Vec::new() };
        document.spans = document.parse_range(source, 0..document.preview.blocks.len());
        document
    }

    fn parse_range(&self, source: &(impl SourceText + ?Sized), blocks: Range<usize>) -> Vec<BlockSpans> {
        let refs = |label: &str| self.preview.refdef_dest(label).map(str::to_owned);
        self.preview.blocks[blocks]
            .iter()
            .map(|block| parse_block(&source.slice(block.bytes.clone()), &refs))
            .collect()
    }

    pub fn apply(&mut self, source: &(impl SourceText + ?Sized), edits: &[Edit]) -> Range<usize> {
        match self.preview.apply(source, edits) {
            Update::Unchanged => {
                let touched = edits.iter().map(|e| e.position).min().unwrap_or(0);
                touched..touched
            }
            Update::Replaced { old, new } => {
                let fresh = self.parse_range(source, new.clone());
                self.spans.splice(old, fresh);
                let blocks = &self.preview.blocks;
                let start = blocks.get(new.start).map_or(source.len(), |b| b.bytes.start);
                let end = new
                    .end
                    .checked_sub(1)
                    .and_then(|last| blocks.get(last))
                    .map_or(start, |b| b.bytes.end);
                let edit_start = edits.iter().map(|e| e.position).min().unwrap_or(start);
                start.min(edit_start)..end.max(edit_start)
            }
            Update::Full => {
                self.spans = self.parse_range(source, 0..self.preview.blocks.len());
                0..source.len()
            }
        }
    }

    pub fn blocks_in(&self, bytes: Range<usize>) -> impl Iterator<Item = (Range<usize>, &BlockSpans)> {
        let first = self.preview.blocks.partition_point(|block| block.bytes.end <= bytes.start);
        self.preview.blocks[first..]
            .iter()
            .zip(&self.spans[first..])
            .take_while(move |(block, _)| block.bytes.start < bytes.end.max(bytes.start + 1))
            .map(|(block, spans)| (block.bytes.clone(), spans))
    }

    pub fn spans_in(&self, bytes: Range<usize>) -> Vec<(Range<usize>, SpanKind)> {
        let mut out = Vec::new();
        for (block, spans) in self.blocks_in(bytes.clone()) {
            for span in &spans.spans {
                let range = block.start + span.range.start..block.start + span.range.end;
                if range.end > bytes.start && range.start < bytes.end {
                    out.push((range, span.kind));
                }
            }
        }
        out
    }
}
```

- `Update::Replaced { old, new }` holds block **indices**, per the doc comment at `incremental.rs:119`.
- `SourceText` is implemented for `str` and for `ScintillaSource` (`preview_host.rs:228`).
- Make `ScintillaSource` `pub(crate)` so `live_host` can use it (Task 12).

- [ ] **Step 5: Front matter is dimmed source** (spec §6). pulldown-cmark, with FastPad's options, reads a leading `---` … `---` block as a thematic break and a setext heading. Live shows it as one dimmed span instead.

Test (append to the module):
```rust
    #[test]
    fn front_matter_is_one_dim_span() {
        let text = "---\ntitle: x\n---\n\n**b**\n";
        let doc = LiveDocument::parse(text);
        let spans = doc.spans_in(0..text.len());
        assert_eq!(spans[0], (0..16, SpanKind::Dim));
        assert!(spans[1..].iter().all(|(range, _)| range.start >= 16), "{spans:?}");
        assert_eq!(doc.front_matter_end(), 16);
    }

    #[test]
    fn a_rule_without_a_closing_fence_is_not_front_matter() {
        let doc = LiveDocument::parse("---\ntext\n");
        assert_eq!(doc.front_matter_end(), 0);
    }
```
Implementation, added to `LiveDocument` (field `front_matter: usize`, set in `parse` and at the end of every `apply`):
```rust
/// The end of a YAML front matter block (`---` … `---` or `...` lines at the very start), or 0.
fn front_matter(source: &(impl SourceText + ?Sized)) -> usize {
    let head = source.slice(0..source.len().min(64 * 1024));
    let mut lines = head.split_inclusive('\n');
    if lines.next().map(str::trim_end) != Some("---") {
        return 0;
    }
    let mut end = 4.min(head.len());
    for line in lines {
        end += line.len();
        if matches!(line.trim_end(), "---" | "...") {
            return end - (line.len() - line.trim_end_matches(['\r', '\n']).len());
        }
    }
    0
}

pub fn front_matter_end(&self) -> usize {
    self.front_matter
}
```
In `parse`, call `front_matter(source)` directly (`str` implements `SourceText`). `spans_in` emits `(0..front_matter, SpanKind::Dim)` first, when the range starts inside it, and skips every block span that starts before `front_matter`. `blocks_in` callers that read decorations (painter, anchors) skip blocks whose start is before `front_matter_end()`. In `apply`, return a changed range starting at 0 whenever the front matter end moved, so the whole block restyles.

- [ ] **Step 6: Run the tests**

Run: `cargo test --lib live::blocks`
Expected: 7 passed.

- [ ] **Step 7: Commit**

```bash
git add src/live/blocks.rs src/live/mod.rs src/preview/incremental.rs
git commit -m "feat(live): incremental LiveDocument over the preview's block list"
```

---

### Task 5: Reveal set, style table and styling runs (pure)

**Files:**
- Create: `src/live/reveal.rs`, `src/live/styles.rs`, `src/live/styler.rs`
- Modify: `src/live/mod.rs` (`pub mod reveal; pub mod styles; pub mod styler;`)
- Modify: `src/languages/mod.rs` (add `pub(crate) fn syntax_colors(theme: Theme) -> &'static SyntaxColors { &SYNTAX_COLORS[theme as usize] }` if no accessor exists)

**Interfaces:**
- Produces:
```rust
// reveal.rs
pub fn revealed_lines(selections: &[Range<usize>], line_of: impl Fn(usize) -> usize) -> BTreeSet<usize>;
pub fn changed_lines(old: &BTreeSet<usize>, new: &BTreeSet<usize>) -> Vec<usize>;
// styles.rs
pub const TEXT: u8 = 0; HIDDEN = 1; BLANK = 2; BOLD = 3; ITALIC = 4; BOLD_ITALIC = 5;
pub const INLINE_CODE = 6; CODE_BLOCK = 7; LINK = 8; HEADING_SMALL = 9; QUOTE = 10; DIM = 11;
pub const MARKER = 12; TABLE = 13; TABLE_HEADER = 14; SOURCE_HEADING = 15; TABLE_BLANK = 16;
pub const CODE_MARKER = 17; pub const ANNOTATION: u8 = 40;
pub const STRIKE_INDICATOR: u32 = 20;
pub fn style_for(kind: SpanKind, revealed: bool) -> u8;
pub struct StyleDef { pub style: u8, pub foreground: u32, pub background: u32, pub bold: bool,
    pub italic: bool, pub underline: bool, pub visible: bool, pub eol_filled: bool, pub mono: bool }
pub fn style_table(colors: &SyntaxColors) -> Vec<StyleDef>;
// styler.rs
pub fn style_runs(spans: &[(Range<usize>, SpanKind)], range: Range<usize>,
    revealed: &[Range<usize>]) -> Vec<(usize, u8)>;
```
  - In `style_runs`, `revealed` is a sorted list of byte ranges of revealed lines, each including its line ending.
  - The returned runs exactly cover `range`.

- [ ] **Step 1: Write the failing tests**

`src/live/reveal.rs`, test module:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    // Lines of "aa\nbb\ncc\n": 0..3, 3..6, 6..9.
    fn line_of(at: usize) -> usize {
        at / 3
    }

    #[test]
    fn a_caret_reveals_its_line() {
        assert_eq!(revealed_lines(&[4..4], line_of), BTreeSet::from([1]));
    }

    #[test]
    fn a_selection_reveals_every_line_it_touches() {
        assert_eq!(revealed_lines(&[1..7], line_of), BTreeSet::from([0, 1, 2]));
    }

    #[test]
    fn a_selection_ending_at_a_line_start_does_not_reveal_that_line() {
        assert_eq!(revealed_lines(&[0..3], line_of), BTreeSet::from([0]));
    }

    #[test]
    fn multiple_carets_reveal_each_line() {
        assert_eq!(revealed_lines(&[0..0, 7..7], line_of), BTreeSet::from([0, 2]));
    }

    #[test]
    fn changed_lines_is_the_symmetric_difference() {
        let old = BTreeSet::from([1, 2]);
        let new = BTreeSet::from([2, 3]);
        assert_eq!(changed_lines(&old, &new), vec![1, 3]);
    }
}
```

`src/live/styles.rs`, test module:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::spans::SpanKind;

    #[test]
    fn rendered_and_revealed_styles() {
        assert_eq!(style_for(SpanKind::Hide, false), HIDDEN);
        assert_eq!(style_for(SpanKind::Hide, true), MARKER);
        assert_eq!(style_for(SpanKind::Blank, false), BLANK);
        assert_eq!(style_for(SpanKind::Blank, true), MARKER);
        assert_eq!(style_for(SpanKind::TableBlank, false), TABLE_BLANK);
        assert_eq!(style_for(SpanKind::TableBlank, true), TABLE);
        assert_eq!(style_for(SpanKind::HeadingText(1), false), BLANK);
        assert_eq!(style_for(SpanKind::HeadingText(4), false), HEADING_SMALL);
        assert_eq!(style_for(SpanKind::HeadingText(1), true), SOURCE_HEADING);
        assert_eq!(style_for(SpanKind::HeadingMarker(2), false), HIDDEN);
        assert_eq!(style_for(SpanKind::HeadingMarker(2), true), MARKER);
        assert_eq!(style_for(SpanKind::Bold, true), BOLD);
        assert_eq!(style_for(SpanKind::CodeBlock, false), CODE_BLOCK);
    }

    #[test]
    fn blank_styles_paint_text_in_the_background_colour() {
        let colors = crate::languages::syntax_colors(crate::platform::theme::Theme::Light);
        let table = style_table(colors);
        let get = |style| table.iter().find(|def| def.style == style).unwrap();
        assert_eq!(get(BLANK).foreground, get(BLANK).background);
        assert!(get(TABLE_BLANK).mono);
        assert_eq!(get(TABLE_BLANK).foreground, get(TABLE_BLANK).background);
        assert!(!get(HIDDEN).visible);
        assert!(get(CODE_BLOCK).eol_filled && get(CODE_BLOCK).mono);
        assert!(get(LINK).underline);
        assert_eq!(get(ANNOTATION).background, colors.background);
    }
}
```
(If `Theme` has no `Light` variant, use the first variant listed in `src/platform/theme.rs`.)

`src/live/styler.rs`, test module:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::spans::SpanKind;
    use crate::live::styles::{BOLD, HIDDEN, MARKER, TEXT};

    #[test]
    fn gaps_are_text_and_runs_cover_the_range() {
        // "a **b** c": Hide 2..4, Bold 4..5, Hide 5..7.
        let spans = [(2..4, SpanKind::Hide), (4..5, SpanKind::Bold), (5..7, SpanKind::Hide)];
        assert_eq!(
            style_runs(&spans, 0..9, &[]),
            vec![(2, TEXT), (2, HIDDEN), (1, BOLD), (2, HIDDEN), (2, TEXT)]
        );
    }

    #[test]
    fn a_revealed_line_uses_source_styles_and_splits_spans() {
        // Line 0 is 0..4 ("**b\n"), line 1 is 4..7: a span crossing the boundary splits.
        let spans = [(0..2, SpanKind::Hide), (2..6, SpanKind::Bold)];
        assert_eq!(
            style_runs(&spans, 0..7, &[0..4]),
            vec![(2, MARKER), (4, BOLD), (1, TEXT)]
        );
    }

    #[test]
    fn a_range_starting_inside_a_span_is_clipped() {
        let spans = [(0..4, SpanKind::Hide)];
        assert_eq!(style_runs(&spans, 2..6, &[]), vec![(2, HIDDEN), (2, TEXT)]);
    }
}
```
   - In the second test, `(2, MARKER), (4, BOLD)` happens because adjacent runs with the same style merge: Bold at 2..4 is revealed as BOLD, and Bold at 4..6 is rendered as BOLD.
   - Revealed Bold stays bold. Only markup changes when revealed.

- [ ] **Step 2: Run them and watch them fail**

Run: `cargo test --lib live::reveal live::styles live::styler`
Expected: compile errors.

- [ ] **Step 3: Implement**

`src/live/reveal.rs`:
```rust
//! Which lines show source (live mode spec §5): every line holding a caret or touching a
//! selection. A selection that ends at the very start of a line does not touch that line.

use std::collections::BTreeSet;
use std::ops::Range;

pub fn revealed_lines(
    selections: &[Range<usize>],
    line_of: impl Fn(usize) -> usize,
) -> BTreeSet<usize> {
    let mut lines = BTreeSet::new();
    for selection in selections {
        let (start, end) = (selection.start.min(selection.end), selection.start.max(selection.end));
        let first = line_of(start);
        let last = if end > start && line_of(end) > first && line_of(end - 1) < line_of(end) {
            line_of(end) - 1
        } else {
            line_of(end)
        };
        lines.extend(first..=last.max(first));
    }
    lines
}

pub fn changed_lines(old: &BTreeSet<usize>, new: &BTreeSet<usize>) -> Vec<usize> {
    old.symmetric_difference(new).copied().collect()
}
```

`src/live/styles.rs`:
```rust
//! Live Markdown's Scintilla style table (live mode spec §2, §6): hidden markup takes no width,
//! blanked markup keeps its width in the background colour for the painter to draw over, and a
//! revealed line swaps both for the visible marker style.

use super::spans::SpanKind;
use crate::languages::SyntaxColors;

pub const TEXT: u8 = 0;
pub const HIDDEN: u8 = 1;
pub const BLANK: u8 = 2;
pub const BOLD: u8 = 3;
pub const ITALIC: u8 = 4;
pub const BOLD_ITALIC: u8 = 5;
pub const INLINE_CODE: u8 = 6;
pub const CODE_BLOCK: u8 = 7;
pub const LINK: u8 = 8;
pub const HEADING_SMALL: u8 = 9;
pub const QUOTE: u8 = 10;
pub const DIM: u8 = 11;
pub const MARKER: u8 = 12;
pub const TABLE: u8 = 13;
pub const TABLE_HEADER: u8 = 14;
pub const SOURCE_HEADING: u8 = 15;
pub const TABLE_BLANK: u8 = 16;
pub const CODE_MARKER: u8 = 17;
/// Styles 32–39 are Scintilla's predefined styles; the annotation style sits above them.
pub const ANNOTATION: u8 = 40;
pub const STRIKE_INDICATOR: u32 = 20;

pub fn style_for(kind: SpanKind, revealed: bool) -> u8 {
    match (kind, revealed) {
        (SpanKind::Text, _) => TEXT,
        (SpanKind::Bold, _) => BOLD,
        (SpanKind::Italic, _) => ITALIC,
        (SpanKind::BoldItalic, _) => BOLD_ITALIC,
        (SpanKind::InlineCode, _) => INLINE_CODE,
        (SpanKind::CodeBlock, _) => CODE_BLOCK,
        (SpanKind::Link, _) => LINK,
        (SpanKind::Quote, _) => QUOTE,
        (SpanKind::Dim, _) => DIM,
        (SpanKind::Marker, _) => MARKER,
        (SpanKind::Hide | SpanKind::Blank | SpanKind::HeadingMarker(_), true) => MARKER,
        (SpanKind::Hide | SpanKind::HeadingMarker(_), false) => HIDDEN,
        (SpanKind::Blank, false) => BLANK,
        (SpanKind::TableCell, _) => TABLE,
        (SpanKind::TableHeader, _) => TABLE_HEADER,
        (SpanKind::TableBlank, true) => TABLE,
        (SpanKind::TableBlank, false) => TABLE_BLANK,
        (SpanKind::HeadingText(_), true) => SOURCE_HEADING,
        (SpanKind::HeadingText(level), false) if level <= 3 => BLANK,
        (SpanKind::HeadingText(_), false) => HEADING_SMALL,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StyleDef {
    pub style: u8,
    pub foreground: u32,
    pub background: u32,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub visible: bool,
    pub eol_filled: bool,
    /// Editor (monospace) font instead of the prose font.
    pub mono: bool,
}

pub fn style_table(c: &SyntaxColors) -> Vec<StyleDef> {
    let plain = |style, foreground| StyleDef {
        style,
        foreground,
        background: c.background,
        bold: false,
        italic: false,
        underline: false,
        visible: true,
        eol_filled: false,
        mono: false,
    };
    vec![
        plain(TEXT, c.text),
        StyleDef { visible: false, ..plain(HIDDEN, c.text) },
        plain(BLANK, c.background),
        StyleDef { bold: true, ..plain(BOLD, c.emphasis) },
        StyleDef { italic: true, ..plain(ITALIC, c.emphasis) },
        StyleDef { bold: true, italic: true, ..plain(BOLD_ITALIC, c.emphasis) },
        StyleDef { background: c.code_background, mono: true, ..plain(INLINE_CODE, c.code) },
        StyleDef {
            background: c.code_background,
            mono: true,
            eol_filled: true,
            ..plain(CODE_BLOCK, c.code)
        },
        StyleDef { underline: true, ..plain(LINK, c.link) },
        StyleDef { bold: true, ..plain(HEADING_SMALL, c.heading) },
        StyleDef { italic: true, ..plain(QUOTE, c.comment) },
        plain(DIM, c.comment),
        plain(MARKER, c.operator),
        StyleDef { mono: true, ..plain(TABLE, c.text) },
        StyleDef { mono: true, bold: true, ..plain(TABLE_HEADER, c.text) },
        StyleDef { bold: true, ..plain(SOURCE_HEADING, c.heading) },
        StyleDef { mono: true, ..plain(TABLE_BLANK, c.background) },
        StyleDef { mono: true, ..plain(CODE_MARKER, c.operator) },
        plain(ANNOTATION, c.background),
    ]
}
```
(If `SyntaxColors` or its fields are private to `languages`, make the struct and the fields used here `pub(crate)`.)

`src/live/styler.rs`:
```rust
//! Spans plus the reveal set → Scintilla styling runs (live mode spec §5, §7). Pure, so the
//! window layer only sends the runs.

use super::spans::SpanKind;
use super::styles::style_for;
use std::ops::Range;

pub fn style_runs(
    spans: &[(Range<usize>, SpanKind)],
    range: Range<usize>,
    revealed: &[Range<usize>],
) -> Vec<(usize, u8)> {
    let mut runs: Vec<(usize, u8)> = Vec::new();
    let mut push = |length: usize, style: u8| {
        if length == 0 {
            return;
        }
        match runs.last_mut() {
            Some((last_length, last_style)) if *last_style == style => *last_length += length,
            _ => runs.push((length, style)),
        }
    };
    // Cut points: span edges and revealed-line edges inside `range`.
    let mut cuts = vec![range.start, range.end];
    for (span, _) in spans {
        cuts.extend([span.start, span.end]);
    }
    for line in revealed {
        cuts.extend([line.start, line.end]);
    }
    cuts.retain(|cut| (range.start..=range.end).contains(cut));
    cuts.sort_unstable();
    cuts.dedup();
    let mut span_index = 0;
    for pair in cuts.windows(2) {
        let (start, end) = (pair[0], pair[1]);
        while span_index < spans.len() && spans[span_index].0.end <= start {
            span_index += 1;
        }
        let kind = spans
            .get(span_index)
            .filter(|(span, _)| span.start <= start)
            .map_or(SpanKind::Text, |(_, kind)| *kind);
        let is_revealed = revealed.iter().any(|line| line.start <= start && start < line.end);
        push(end - start, style_for(kind, is_revealed));
    }
    runs
}
```
`revealed` is short (a few lines), so the linear `any` is fine. Spans must be sorted and non-overlapping, which `LiveDocument::spans_in` guarantees.

- [ ] **Step 4: Run the tests**

Run: `cargo test --lib live::reveal live::styles live::styler`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add src/live src/languages/mod.rs
git commit -m "feat(live): reveal set, style table and styling runs"
```

---

### Task 6: Format toggles (pure)

**Files:**
- Create: `src/editor/markdown_edit.rs`
- Modify: `src/editor/mod.rs` (`pub mod markdown_edit;`)

**Interfaces:**
- Produces:
```rust
pub struct TextEdit { pub range: Range<usize>, pub text: String }
/// Edits are in pre-edit coordinates, sorted and non-overlapping; selections are post-edit.
pub struct EditPlan { pub edits: Vec<TextEdit>, pub selections: Vec<Range<usize>> }
impl EditPlan { pub fn apply_to(&self, text: &str) -> String; }
pub fn toggle_marker(text: &str, selections: &[Range<usize>], marker: &str) -> EditPlan;
pub fn insert_link(text: &str, selections: &[Range<usize>]) -> EditPlan;
```

- [ ] **Step 1: Write the failing tests** (create `src/editor/markdown_edit.rs` with only this module):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn run(text: &str, selections: &[Range<usize>], marker: &str) -> (String, Vec<Range<usize>>) {
        let plan = toggle_marker(text, selections, marker);
        (plan.apply_to(text), plan.selections)
    }

    #[test]
    fn wraps_a_selection_and_keeps_it_selected() {
        assert_eq!(run("a b c", &[2..3], "**"), ("a **b** c".into(), vec![4..5]));
    }

    #[test]
    fn unwraps_when_the_markers_are_inside_the_selection() {
        assert_eq!(run("a **b** c", &[2..7], "**"), ("a b c".into(), vec![2..3]));
    }

    #[test]
    fn unwraps_when_the_markers_surround_the_selection() {
        assert_eq!(run("a **b** c", &[4..5], "**"), ("a b c".into(), vec![2..3]));
    }

    #[test]
    fn an_empty_selection_toggles_the_word_under_the_caret() {
        assert_eq!(run("hello world", &[2..2], "**"), ("**hello** world".into(), vec![4..4]));
        assert_eq!(run("**hello** world", &[4..4], "**"), ("hello world".into(), vec![2..2]));
    }

    #[test]
    fn an_empty_selection_outside_a_word_inserts_a_pair() {
        assert_eq!(run("a  b", &[2..2], "**"), ("a **** b".into(), vec![4..4]));
    }

    #[test]
    fn italic_on_bold_text_wraps_instead_of_eating_a_bold_star() {
        assert_eq!(run("**b**", &[2..3], "*"), ("***b***".into(), vec![3..4]));
    }

    #[test]
    fn bold_off_bold_italic_leaves_italic() {
        assert_eq!(run("***b***", &[3..4], "**"), ("*b*".into(), vec![1..2]));
    }

    #[test]
    fn every_caret_of_a_multi_cursor_is_toggled() {
        assert_eq!(run("a b", &[0..0, 2..2], "**"), ("**a** **b**".into(), vec![2..2, 8..8]));
    }

    #[test]
    fn inline_code_uses_backticks() {
        assert_eq!(run("x y", &[2..3], "`"), ("x `y`".into(), vec![3..4]));
    }

    #[test]
    fn a_reversed_selection_is_handled() {
        assert_eq!(run("a b c", &[3..2], "**"), ("a **b** c".into(), vec![4..5]));
    }

    #[test]
    fn link_wraps_the_selection_and_puts_the_caret_in_the_parentheses() {
        let plan = insert_link("see docs", &[4..8]);
        assert_eq!(plan.apply_to("see docs"), "see [docs]()");
        assert_eq!(plan.selections, vec![11..11]);
    }

    #[test]
    fn link_without_a_selection_puts_the_caret_in_the_brackets() {
        let plan = insert_link("a ", &[2..2]);
        assert_eq!(plan.apply_to("a "), "a []()");
        assert_eq!(plan.selections, vec![3..3]);
    }
}
```

- [ ] **Step 2: Run them and watch them fail**

Run: `cargo test --lib editor::markdown_edit`
Expected: compile error.

- [ ] **Step 3: Implement** (above the tests):

```rust
//! Markdown writing helpers (live mode spec §8). Pure text → edit plans; the window layer
//! applies a plan as one undo step. Edits use pre-edit byte offsets; selections are post-edit.

use std::ops::Range;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextEdit {
    pub range: Range<usize>,
    pub text: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct EditPlan {
    pub edits: Vec<TextEdit>,
    pub selections: Vec<Range<usize>>,
}

impl EditPlan {
    pub fn apply_to(&self, text: &str) -> String {
        let mut out = String::with_capacity(text.len() + 16);
        let mut at = 0;
        for edit in &self.edits {
            out.push_str(&text[at..edit.range.start]);
            out.push_str(&edit.text);
            at = edit.range.end;
        }
        out.push_str(&text[at..]);
        out
    }
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn word_at(text: &str, at: usize) -> Option<Range<usize>> {
    let start = text[..at]
        .char_indices()
        .rev()
        .take_while(|(_, c)| is_word_char(*c))
        .last()
        .map_or(at, |(index, _)| index);
    let end = text[at..]
        .char_indices()
        .take_while(|(_, c)| is_word_char(*c))
        .last()
        .map_or(at, |(index, c)| at + index + c.len_utf8());
    (start < end).then_some(start..end)
}

/// Length of the run of `byte` ending at `end` (backwards) or starting at `start` (forwards).
fn run_before(text: &str, end: usize, byte: u8) -> usize {
    text.as_bytes()[..end].iter().rev().take_while(|b| **b == byte).count()
}

fn run_after(text: &str, start: usize, byte: u8) -> usize {
    text.as_bytes()[start..].iter().take_while(|b| **b == byte).count()
}

/// A marker run of `run` characters can drop a `width`-character marker: exactly that marker,
/// or the three-character bold-italic run for `*` / `**`.
fn run_matches(run: usize, width: usize) -> bool {
    run == width || (run == 3 && width < 3)
}

fn normalized(selection: &Range<usize>) -> Range<usize> {
    selection.start.min(selection.end)..selection.start.max(selection.end)
}

fn shift(position: usize, delta: isize) -> usize {
    (position as isize + delta) as usize
}

pub fn toggle_marker(text: &str, selections: &[Range<usize>], marker: &str) -> EditPlan {
    let width = marker.len();
    let byte = marker.as_bytes()[0];
    let mut order: Vec<usize> = (0..selections.len()).collect();
    order.sort_by_key(|index| normalized(&selections[*index]).start);
    let mut plan = EditPlan { edits: Vec::new(), selections: vec![0..0; selections.len()] };
    let mut delta: isize = 0;
    let insert = |plan: &mut EditPlan, at: usize, text: &str| {
        plan.edits.push(TextEdit { range: at..at, text: text.to_owned() });
    };
    let remove = |plan: &mut EditPlan, range: Range<usize>| {
        plan.edits.push(TextEdit { range, text: String::new() });
    };
    for index in order {
        let selection = normalized(&selections[index]);
        let caret = selection.is_empty().then_some(selection.start);
        let target = match caret {
            Some(at) => word_at(text, at).unwrap_or(at..at),
            None => selection.clone(),
        };
        if target.is_empty() {
            insert(&mut plan, target.start, &marker.repeat(2));
            let at = shift(target.start, delta) + width;
            plan.selections[index] = at..at;
            delta += 2 * width as isize;
            continue;
        }
        let inner = &text[target.clone()];
        let inside = inner.len() >= 2 * width
            && run_matches(run_after(inner, 0, byte), width)
            && run_matches(run_before(inner, inner.len(), byte), width);
        let around = target.start >= width
            && run_matches(run_before(text, target.start, byte), width)
            && run_matches(run_after(text, target.end, byte), width);
        if inside {
            remove(&mut plan, target.start..target.start + width);
            remove(&mut plan, target.end - width..target.end);
            let start = shift(target.start, delta);
            plan.selections[index] = match caret {
                Some(at) => {
                    let at = shift(at, delta).saturating_sub(width).max(start);
                    at..at
                }
                None => start..start + inner.len() - 2 * width,
            };
            delta -= 2 * width as isize;
        } else if around {
            remove(&mut plan, target.start - width..target.start);
            remove(&mut plan, target.end..target.end + width);
            plan.selections[index] = match caret {
                Some(at) => {
                    let at = shift(at, delta) - width;
                    at..at
                }
                None => shift(target.start, delta) - width..shift(target.end, delta) - width,
            };
            delta -= 2 * width as isize;
        } else {
            insert(&mut plan, target.start, marker);
            insert(&mut plan, target.end, marker);
            plan.selections[index] = match caret {
                Some(at) => {
                    let at = shift(at, delta) + width;
                    at..at
                }
                None => shift(target.start, delta) + width..shift(target.end, delta) + width,
            };
            delta += 2 * width as isize;
        }
    }
    plan.edits.sort_by_key(|edit| (edit.range.start, edit.range.end));
    plan
}

pub fn insert_link(text: &str, selections: &[Range<usize>]) -> EditPlan {
    let _ = text;
    let mut order: Vec<usize> = (0..selections.len()).collect();
    order.sort_by_key(|index| normalized(&selections[*index]).start);
    let mut plan = EditPlan { edits: Vec::new(), selections: vec![0..0; selections.len()] };
    let mut delta: isize = 0;
    for index in order {
        let selection = normalized(&selections[index]);
        if selection.is_empty() {
            plan.edits.push(TextEdit { range: selection.clone(), text: "[]()".into() });
            let at = shift(selection.start, delta) + 1;
            plan.selections[index] = at..at;
        } else {
            plan.edits.push(TextEdit { range: selection.start..selection.start, text: "[".into() });
            plan.edits.push(TextEdit { range: selection.end..selection.end, text: "]()".into() });
            let at = shift(selection.end, delta) + 3;
            plan.selections[index] = at..at;
        }
        delta += 4;
    }
    plan
}
```

In the word-under-caret case, the inside branch handles "**hello**" with the caret on `hello`: `word_at` returns only `hello`, and the around branch removes the markers.

- [ ] **Step 4: Run the tests**

Run: `cargo test --lib editor::markdown_edit`
Expected: 12 passed.

- [ ] **Step 5: Commit**

```bash
git add src/editor/markdown_edit.rs src/editor/mod.rs
git commit -m "feat(editor): Markdown format toggles and link insertion as edit plans"
```

---

### Task 7: List continuation and nesting (pure)

**Files:**
- Modify: `src/editor/markdown_edit.rs`

**Interfaces:**
- Produces:
```rust
/// `fallback` is the line ending for a document with none yet (the editor's EOL mode).
pub fn enter_in_list(text: &str, caret: usize, fallback: &'static str) -> Option<EditPlan>;
pub fn indent_list_item(text: &str, caret: usize, outdent: bool) -> Option<EditPlan>;
pub(crate) fn line_bounds(text: &str, at: usize) -> Range<usize>; // without the line ending
pub(crate) fn line_ending(text: &str, line_end: usize, fallback: &'static str) -> &'static str;
```

- [ ] **Step 1: Write the failing tests** (append to the test module):

```rust
    fn enter(text: &str, caret: usize) -> Option<(String, usize)> {
        enter_in_list(text, caret, "\n").map(|plan| (plan.apply_to(text), plan.selections[0].start))
    }

    fn nest(text: &str, caret: usize, outdent: bool) -> Option<(String, usize)> {
        indent_list_item(text, caret, outdent)
            .map(|plan| (plan.apply_to(text), plan.selections[0].start))
    }

    #[test]
    fn enter_continues_bullets_numbers_and_tasks() {
        assert_eq!(enter("- a", 3), Some(("- a\n- ".into(), 6)));
        assert_eq!(enter("1. a", 4), Some(("1. a\n2. ".into(), 8)));
        assert_eq!(enter("3) a", 4), Some(("3) a\n4) ".into(), 8)));
        assert_eq!(enter("- [x] a", 7), Some(("- [x] a\n- [ ] ".into(), 14)));
        assert_eq!(enter("  * a", 5), Some(("  * a\n  * ".into(), 10)));
    }

    #[test]
    fn enter_mid_item_splits_it() {
        assert_eq!(enter("- ab", 3), Some(("- a\n- b".into(), 6)));
    }

    #[test]
    fn enter_on_an_empty_item_ends_the_list() {
        assert_eq!(enter("- a\n- ", 6), Some(("- a\n".into(), 4)));
        assert_eq!(enter("- a\n-", 5), Some(("- a\n".into(), 4)));
        assert_eq!(enter("- a\n- [ ] ", 10), Some(("- a\n".into(), 4)));
    }

    #[test]
    fn enter_outside_a_list_or_inside_the_marker_is_left_alone() {
        assert_eq!(enter("text", 4), None);
        assert_eq!(enter("**b**", 5), None);
        assert_eq!(enter("- a", 1), None);
    }

    #[test]
    fn continuation_uses_the_documents_line_ending() {
        assert_eq!(enter("- a\r\nb", 3), Some(("- a\r\n- \r\nb".into(), 7)));
        let plan = enter_in_list("- a", 3, "\r\n").unwrap();
        assert_eq!(plan.apply_to("- a"), "- a\r\n- ", "a one-line document uses the fallback");
    }

    #[test]
    fn tab_nests_by_the_marker_width_and_shift_tab_un_nests() {
        assert_eq!(nest("- a", 3, false), Some(("  - a".into(), 5)));
        assert_eq!(nest("1. a", 4, false), Some(("   1. a".into(), 7)));
        assert_eq!(nest("  - a", 5, true), Some(("- a".into(), 3)));
        assert_eq!(nest("- a", 3, true), None);
        assert_eq!(nest("text", 2, false), None);
    }
```

- [ ] **Step 2: Run them and watch them fail**

Run: `cargo test --lib editor::markdown_edit`
Expected: compile error.

- [ ] **Step 3: Implement** (append above the tests):

```rust
pub(crate) fn line_bounds(text: &str, at: usize) -> Range<usize> {
    let start = text[..at].rfind('\n').map_or(0, |newline| newline + 1);
    let end = text[at..].find('\n').map_or(text.len(), |newline| at + newline);
    let end = if end > start && text.as_bytes()[end - 1] == b'\r' { end - 1 } else { end };
    start..end
}

/// The line ending after `line_end`; for the last line the document's first one, or `fallback`
/// when the document has none.
pub(crate) fn line_ending(text: &str, line_end: usize, fallback: &'static str) -> &'static str {
    let rest = &text[line_end..];
    if rest.starts_with("\r\n") {
        "\r\n"
    } else if rest.starts_with('\n') {
        "\n"
    } else {
        match text.find('\n') {
            Some(at) if at > 0 && text.as_bytes()[at - 1] == b'\r' => "\r\n",
            Some(_) => "\n",
            None => fallback,
        }
    }
}

struct ListPrefix {
    indent: usize,
    bullet: Option<u8>,
    number: Option<(u64, u8)>,
    task: bool,
    /// Bytes from the line start to the item text.
    len: usize,
    /// The marker plus its space: how far a nested item indents.
    marker_width: usize,
}

fn list_prefix(line: &str) -> Option<ListPrefix> {
    let bytes = line.as_bytes();
    let indent = bytes.iter().take_while(|b| matches!(b, b' ' | b'\t')).count();
    let mut at = indent;
    let (bullet, number) = match *bytes.get(at)? {
        marker @ (b'-' | b'*' | b'+') => {
            at += 1;
            (Some(marker), None)
        }
        digit if digit.is_ascii_digit() => {
            let digits = bytes[at..].iter().take_while(|b| b.is_ascii_digit()).count();
            if digits > 9 {
                return None;
            }
            let value = line[at..at + digits].parse().ok()?;
            at += digits;
            let delimiter = *bytes.get(at)?;
            if delimiter != b'.' && delimiter != b')' {
                return None;
            }
            at += 1;
            (None, Some((value, delimiter)))
        }
        _ => return None,
    };
    match bytes.get(at) {
        Some(b' ') => at += 1,
        None => {}
        Some(_) => return None,
    }
    let marker_width = (at - indent).max(2);
    let rest = &line[at..];
    let task = ["[ ]", "[x]", "[X]"].iter().any(|box_| {
        rest.starts_with(box_) && (rest.len() == 3 || rest.as_bytes()[3] == b' ')
    });
    if task {
        at = (at + 4).min(line.len());
    }
    Some(ListPrefix { indent, bullet, number, task, len: at, marker_width })
}

pub fn enter_in_list(text: &str, caret: usize, fallback: &'static str) -> Option<EditPlan> {
    let line = line_bounds(text, caret);
    let prefix = list_prefix(&text[line.clone()])?;
    let prefix_end = line.start + prefix.len;
    if caret < prefix_end.min(line.end) {
        return None;
    }
    if text[prefix_end.min(line.end)..line.end].trim().is_empty() {
        return Some(EditPlan {
            edits: vec![TextEdit { range: line.clone(), text: String::new() }],
            selections: vec![line.start..line.start],
        });
    }
    let marker = match (prefix.bullet, prefix.number) {
        (Some(bullet), _) => char::from(bullet).to_string(),
        (None, Some((value, delimiter))) => format!("{}{}", value + 1, char::from(delimiter)),
        (None, None) => return None,
    };
    let inserted = format!(
        "{}{}{} {}",
        line_ending(text, line.end, fallback),
        &text[line.start..line.start + prefix.indent],
        marker,
        if prefix.task { "[ ] " } else { "" }
    );
    let after = caret + inserted.len();
    Some(EditPlan {
        edits: vec![TextEdit { range: caret..caret, text: inserted }],
        selections: vec![after..after],
    })
}

pub fn indent_list_item(text: &str, caret: usize, outdent: bool) -> Option<EditPlan> {
    let line = line_bounds(text, caret);
    let prefix = list_prefix(&text[line.clone()])?;
    if !outdent {
        let pad = " ".repeat(prefix.marker_width);
        let after = caret + pad.len();
        return Some(EditPlan {
            edits: vec![TextEdit { range: line.start..line.start, text: pad }],
            selections: vec![after..after],
        });
    }
    let spaces = text[line.start..line.start + prefix.indent].bytes().take_while(|b| *b == b' ').count();
    let remove = spaces.min(prefix.marker_width);
    if remove == 0 {
        return None;
    }
    let after = caret.saturating_sub(remove).max(line.start);
    Some(EditPlan {
        edits: vec![TextEdit { range: line.start..line.start + remove, text: String::new() }],
        selections: vec![after..after],
    })
}
```

For an un-nest, `remove = min(leading spaces, own marker width)`. This undoes one `Tab` exactly, because a `Tab` added the same width. The `outdent` case for `"  - a"` removes 2, which gives `"- a"`.

- [ ] **Step 4: Run the tests**

Run: `cargo test --lib editor::markdown_edit`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add src/editor/markdown_edit.rs
git commit -m "feat(editor): Markdown list continuation and nesting"
```

---

### Task 8: Table formatting and cell navigation (pure)

**Files:**
- Modify: `src/editor/markdown_edit.rs`

**Interfaces:**
- Consumes: `crate::live::spans::pipe_positions` (Task 3).
- Produces:
```rust
pub fn table_at(text: &str, at: usize) -> Option<Range<usize>>; // whole lines, no final line ending
pub fn format_table(table: &str) -> Option<String>;            // None when already aligned
pub fn next_cell(text: &str, caret: usize, back: bool) -> Option<Range<usize>>;
```

- [ ] **Step 1: Write the failing tests** (append):

```rust
    #[test]
    fn table_at_needs_a_delimiter_row() {
        let text = "x\n\n|a|b|\n|-|-|\n|c|d|\n\ny";
        let start = text.find("|a").unwrap();
        let end = text.find("|\n\ny").unwrap() + 1;
        assert_eq!(table_at(text, start + 1), Some(start..end));
        assert_eq!(table_at("|a|b|\n|c|d|", 1), None);
        assert_eq!(table_at(text, 0), None);
    }

    #[test]
    fn format_pads_columns_with_a_minimum_width_of_three() {
        assert_eq!(
            format_table("|a|bb|\n|-|-|\n|ccc|d|").as_deref(),
            Some("| a   | bb  |\n| --- | --- |\n| ccc | d   |")
        );
    }

    #[test]
    fn an_aligned_table_is_left_alone() {
        assert_eq!(format_table("| a   | bb  |\n| --- | --- |\n| ccc | d   |"), None);
    }

    #[test]
    fn alignment_colons_are_kept_and_right_columns_pad_left() {
        assert_eq!(
            format_table("|a|b|\n|:-|-:|\n|c|d|").as_deref(),
            Some("| a   |   b |\n| :-- | --: |\n| c   |   d |")
        );
    }

    #[test]
    fn padding_counts_characters_not_bytes() {
        assert_eq!(
            format_table("|é|b|\n|-|-|").as_deref(),
            Some("| é   | b   |\n| --- | --- |")
        );
    }

    #[test]
    fn crlf_tables_stay_crlf_and_short_rows_get_empty_cells() {
        assert_eq!(
            format_table("|a|b|\r\n|-|-|\r\n|c|").as_deref(),
            Some("| a   | b   |\r\n| --- | --- |\r\n| c   |     |")
        );
    }

    #[test]
    fn escaped_pipes_stay_in_their_cell() {
        assert_eq!(
            format_table(r"|a\|b|c|
|-|-|").as_deref(),
            Some("| a\\|b | c   |\n| ---- | --- |")
        );
    }

    #[test]
    fn tab_walks_cells_skipping_the_delimiter_row() {
        let text = "| a | b |\n| - | - |\n| c | d |";
        let at = |s: &str| text.find(s).unwrap();
        assert_eq!(next_cell(text, at("a"), false), Some(at("b")..at("b") + 1));
        assert_eq!(next_cell(text, at("b"), false), Some(at("c")..at("c") + 1));
        assert_eq!(next_cell(text, at("c"), true), Some(at("b")..at("b") + 1));
        assert_eq!(next_cell(text, at("d"), false), None);
    }
```

- [ ] **Step 2: Run them and watch them fail**

Run: `cargo test --lib editor::markdown_edit`
Expected: compile error.

- [ ] **Step 3: Implement** (append above the tests):

```rust
use crate::live::spans::pipe_positions;

fn is_delimiter_row(line: &str) -> bool {
    let cells = split_cells(line);
    !cells.is_empty()
        && cells.iter().all(|(_, cell)| {
            let cell = cell.trim();
            let body = cell.trim_start_matches(':').trim_end_matches(':');
            !body.is_empty() && body.bytes().all(|b| b == b'-')
        })
}

/// A table line's cells: (range within the line, raw text), the outer pipes optional.
fn split_cells(line: &str) -> Vec<(Range<usize>, &str)> {
    let pipes = pipe_positions(line);
    if pipes.is_empty() {
        return Vec::new();
    }
    let trimmed_start = line.len() - line.trim_start().len();
    let trimmed_end = line.trim_end().len();
    let mut bounds: Vec<usize> = Vec::new();
    if pipes[0] != trimmed_start {
        bounds.push(trimmed_start);
    } else {
        bounds.push(pipes[0] + 1);
    }
    for pipe in &pipes {
        if *pipe != trimmed_start && *pipe + 1 != trimmed_end {
            bounds.push(*pipe);
            bounds.push(*pipe + 1);
        }
    }
    let last = *pipes.last().expect("non-empty");
    bounds.push(if last + 1 == trimmed_end { last } else { trimmed_end });
    bounds
        .chunks(2)
        .filter(|pair| pair.len() == 2 && pair[0] <= pair[1])
        .map(|pair| (pair[0]..pair[1], &line[pair[0]..pair[1]]))
        .collect()
}

fn table_lines(text: &str, range: Range<usize>) -> Vec<Range<usize>> {
    crate::live::spans::line_ranges(text, range)
}

pub fn table_at(text: &str, at: usize) -> Option<Range<usize>> {
    let is_row = |line: &Range<usize>| {
        let content = &text[line.clone()];
        !content.trim().is_empty() && !pipe_positions(content).is_empty()
    };
    let here = line_bounds(text, at);
    if !is_row(&here) {
        return None;
    }
    let mut first = here.clone();
    while first.start > 0 {
        let previous = line_bounds(text, first.start - 1);
        if !is_row(&previous) {
            break;
        }
        first = previous;
    }
    let mut last = here;
    loop {
        let next_start = text[last.end..].find('\n').map(|newline| last.end + newline + 1);
        let Some(next_start) = next_start.filter(|start| *start < text.len()) else { break };
        let next = line_bounds(text, next_start);
        if !is_row(&next) {
            break;
        }
        last = next;
    }
    let lines = table_lines(text, first.start..last.end);
    (lines.len() >= 2 && is_delimiter_row(&text[lines[1].clone()])).then_some(first.start..last.end)
}

#[derive(Clone, Copy)]
enum Align {
    None,
    Left,
    Right,
    Center,
}

pub fn format_table(table: &str) -> Option<String> {
    let ending = if table.contains("\r\n") { "\r\n" } else { "\n" };
    let lines: Vec<&str> = table.split('\n').map(|line| line.strip_suffix('\r').unwrap_or(line)).collect();
    let rows: Vec<Vec<String>> = lines
        .iter()
        .map(|line| split_cells(line).into_iter().map(|(_, cell)| cell.trim().to_owned()).collect())
        .collect();
    let columns = rows.iter().map(Vec::len).max()?;
    let aligns: Vec<Align> = (0..columns)
        .map(|column| {
            let cell = rows.get(1).and_then(|row| row.get(column)).map_or("", String::as_str);
            match (cell.starts_with(':'), cell.ends_with(':') && cell.len() > 1) {
                (true, true) => Align::Center,
                (true, false) => Align::Left,
                (false, true) => Align::Right,
                (false, false) => Align::None,
            }
        })
        .collect();
    let widths: Vec<usize> = (0..columns)
        .map(|column| {
            rows.iter()
                .enumerate()
                .filter(|(index, _)| *index != 1)
                .filter_map(|(_, row)| row.get(column))
                .map(|cell| cell.chars().count())
                .max()
                .unwrap_or(0)
                .max(3)
        })
        .collect();
    let rendered: Vec<String> = rows
        .iter()
        .enumerate()
        .map(|(index, row)| {
            let cells: Vec<String> = (0..columns)
                .map(|column| {
                    let width = widths[column];
                    if index == 1 {
                        return match aligns[column] {
                            Align::None => "-".repeat(width),
                            Align::Left => format!(":{}", "-".repeat(width - 1)),
                            Align::Right => format!("{}:", "-".repeat(width - 1)),
                            Align::Center => format!(":{}:", "-".repeat(width - 2)),
                        };
                    }
                    let cell = row.get(column).map_or("", String::as_str);
                    let pad = width - cell.chars().count();
                    match aligns[column] {
                        Align::Right => format!("{}{cell}", " ".repeat(pad)),
                        _ => format!("{cell}{}", " ".repeat(pad)),
                    }
                })
                .collect();
            format!("| {} |", cells.join(" | "))
        })
        .collect();
    let out = rendered.join(ending);
    (out != table).then_some(out)
}

pub fn next_cell(text: &str, caret: usize, back: bool) -> Option<Range<usize>> {
    let table = table_at(text, caret)?;
    let mut cells: Vec<(Range<usize>, Range<usize>)> = Vec::new(); // (segment, trimmed content)
    for (index, line) in table_lines(text, table).into_iter().enumerate() {
        if index == 1 {
            continue;
        }
        for (segment, raw) in split_cells(&text[line.clone()]) {
            let lead = raw.len() - raw.trim_start().len();
            let content_start = line.start + segment.start + lead;
            let content = content_start..content_start + raw.trim().len();
            cells.push((line.start + segment.start..line.start + segment.end, content));
        }
    }
    let current = cells.iter().position(|(segment, _)| segment.start <= caret && caret <= segment.end)?;
    let target = if back { current.checked_sub(1)? } else { current + 1 };
    cells.get(target).map(|(_, content)| content.clone())
}
```

`split_cells` bounds: for each inner pipe, push `pipe` (end of the previous cell) and `pipe + 1` (start of the next one), then pair them with `chunks(2)`. If a test shows a wrong cell boundary, fix `split_cells`. All three functions depend on it.

- [ ] **Step 4: Run the tests**

Run: `cargo test --lib editor::markdown_edit`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add src/editor/markdown_edit.rs
git commit -m "feat(editor): Markdown table formatting and cell navigation"
```

---

### Task 9: New commands and Markdown-scoped key bindings

**Files:**
- Modify: `src/window/commands.rs` (enum after `AddCursorBelow = 260`, `COMMANDS` array at l.454, `TEXT_COMMANDS` at l.180, new `Scope` + methods, stable-value test near l.904)
- Modify: `src/window/keymap.rs` (`COMMAND_IDS` l.179-345, `DEFAULT_BINDINGS` l.379, `command_for` l.668, `conflicts` l.676, `first_text` l.691)
- Modify: `src/window/menus.rs` (`AcceleratorTable::create` l.27; View popup l.219-228; new `set_markdown_live`; tests l.865, l.874, l.892)
- Modify: `src/window/main_window/menu_keys.rs` (`translate_accelerator` l.212; `open_menu` l.123-131)
- Modify: `src/window/command_palette.rs` (`ENTRIES` l.58), `src/window/command_palette/tests.rs` (l.93), `src/window/main_window/command_palette_ui.rs` (l.525-541), `src/window/main_window/tests/command_palette.rs` (l.28-35)
- Modify: `src/window/shortcuts_model.rs` (`title()` l.37)
- Create: `src/window/main_window/tests/markdown_keys.rs`; register `mod markdown_keys;` in `src/window/main_window/tests.rs`

**Interfaces:**
- Produces:
  - `CommandId::{MarkdownToggleLive = 261, MarkdownBold = 262, MarkdownItalic = 263, MarkdownCode = 264, MarkdownLink = 265}`
  - `pub enum Scope { Global, Markdown }` (in `commands.rs`)
  - `CommandId::scope(self) -> Scope`, `CommandId::is_markdown_edit(self) -> bool`
  - `Keymap::command_for_in(&self, stroke: KeyStroke, scope: Scope) -> Option<CommandId>`
  - `menus::set_markdown_live(menu: HMENU, enabled: bool, checked: bool)`
  - String IDs: `markdown.toggleLive`, `markdown.bold`, `markdown.italic`, `markdown.code`, `markdown.link`

- [ ] **Step 1: Write the failing tests**

In `src/window/commands.rs` tests:
```rust
    #[test]
    fn markdown_edit_commands_have_stable_values() {
        for (value, command) in [
            (261, CommandId::MarkdownToggleLive),
            (262, CommandId::MarkdownBold),
            (263, CommandId::MarkdownItalic),
            (264, CommandId::MarkdownCode),
            (265, CommandId::MarkdownLink),
        ] {
            assert_eq!(CommandId::try_from(value), Ok(command));
            assert!(command.is_markdown_edit());
        }
        assert_eq!(CommandId::MarkdownBold.scope(), Scope::Markdown);
        assert_eq!(CommandId::MarkdownToggleLive.scope(), Scope::Global);
        assert_eq!(CommandId::ToggleSidebar.scope(), Scope::Global);
    }
```
(If `try_from` returns a different error type, compare with `.ok()` and `Some(command)`.)

In `src/window/keymap.rs` tests:
```rust
    #[test]
    fn markdown_bindings_do_not_shadow_global_ones() {
        // Break caught: Ctrl+B bolding in a .txt file, or Toggle Sidebar losing its key text.
        let keymap = Keymap::defaults();
        let ctrl_b = KeyStroke::parse("Ctrl+B").unwrap();
        assert_eq!(keymap.command_for(ctrl_b), Some(CommandId::ToggleSidebar));
        assert_eq!(keymap.command_for_in(ctrl_b, Scope::Markdown), Some(CommandId::MarkdownBold));
        assert_eq!(keymap.first_text(CommandId::ToggleSidebar).as_deref(), Some("Ctrl+B"));
        assert_eq!(keymap.first_text(CommandId::MarkdownBold).as_deref(), Some("Ctrl+B"));
        assert!(keymap.conflicts(ctrl_b, CommandId::MarkdownBold).is_empty());
        assert!(keymap.conflicts(ctrl_b, CommandId::ToggleSidebar).is_empty());
    }

    #[test]
    fn live_markdown_toggles_with_ctrl_alt_v() {
        let keymap = Keymap::defaults();
        let stroke = KeyStroke::parse("Ctrl+Alt+V").unwrap();
        assert_eq!(keymap.command_for(stroke), Some(CommandId::MarkdownToggleLive));
    }

    #[test]
    fn a_user_rebinding_keeps_the_commands_scope() {
        let entries = BTreeMap::from([("key.markdown.bold".to_owned(), "Ctrl+Shift+B".to_owned())]);
        let (keymap, warnings) = Keymap::from_ini(&entries);
        assert!(warnings.is_empty(), "{warnings:?}");
        let stroke = KeyStroke::parse("Ctrl+Shift+B").unwrap();
        assert_eq!(keymap.command_for(stroke), None);
        assert_eq!(keymap.command_for_in(stroke, Scope::Markdown), Some(CommandId::MarkdownBold));
    }
```

Create `src/window/main_window/tests/markdown_keys.rs`:
```rust
//! Markdown-scoped keys (live mode spec §9), through the real accelerator path.

use super::*;
use crate::document::Language;

struct Fixture {
    editor: crate::editor::Editor,
    window: ProductionWindow,
    _scintilla: crate::platform::OwnedModule,
}

fn fixture(text: &str, language: Language) -> Fixture {
    let scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    editor.populate_clean(text).unwrap();
    app_mut(window.hwnd).tabs.set_active_language(language);
    Fixture { editor, window, _scintilla: scintilla }
}

#[test]
fn ctrl_b_is_bold_in_markdown_and_toggle_sidebar_elsewhere() {
    let f = fixture("hello", Language::Markdown);
    let key = translate_key_with(f.window.hwnd, f.editor.hwnd(), b'B', true, false, false);
    assert_eq!(key, Some(CommandId::MarkdownBold));
    let plain = fixture("hello", Language::PlainText);
    let key = translate_key_with(plain.window.hwnd, plain.editor.hwnd(), b'B', true, false, false);
    assert_eq!(key, Some(CommandId::ToggleSidebar));
}

#[test]
fn markdown_keys_off_the_editor_fall_through_to_global() {
    let f = fixture("hello", Language::Markdown);
    let key = translate_key_with(f.window.hwnd, f.window.hwnd, b'B', true, false, false);
    assert_ne!(key, Some(CommandId::MarkdownBold));
}

#[test]
fn ctrl_alt_v_toggles_live_markdown() {
    let f = fixture("hello", Language::Markdown);
    let key = translate_key_with(f.window.hwnd, f.editor.hwnd(), b'V', true, false, true);
    assert_eq!(key, Some(CommandId::MarkdownToggleLive));
}
```

- [ ] **Step 2: Run them and watch them fail**

Run: `cargo test --lib markdown_edit_commands_have_stable_values markdown_bindings_do_not_shadow live_markdown_toggles a_user_rebinding markdown_keys -- --test-threads=1`
Expected: compile errors.

- [ ] **Step 3: Implement the commands** (`src/window/commands.rs`)

Add after `AddCursorBelow = 260,`:
```rust
    // Live Markdown and the Markdown writing helpers (live mode spec §4, §8).
    MarkdownToggleLive = 261,
    MarkdownBold = 262,
    MarkdownItalic = 263,
    MarkdownCode = 264,
    MarkdownLink = 265,
```
- Append the five to the `COMMANDS` array in `TryFrom<u16>` and change its length from 150 to 155.
- Append them to `TEXT_COMMANDS` and change its length from 56 to 61. They need a text tab.
- Add:
```rust
/// Where a command's key bindings apply (live mode spec §9): Markdown bindings are tried first,
/// and only while a Markdown tab's editor has focus.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Scope {
    Global,
    Markdown,
}
```
  and, in `impl CommandId`:
```rust
    pub const fn scope(self) -> Scope {
        match self {
            Self::MarkdownBold | Self::MarkdownItalic | Self::MarkdownCode | Self::MarkdownLink => {
                Scope::Markdown
            }
            _ => Scope::Global,
        }
    }

    /// Commands offered only for Markdown tabs (palette, View menu).
    pub const fn is_markdown_edit(self) -> bool {
        matches!(
            self,
            Self::MarkdownToggleLive
                | Self::MarkdownBold
                | Self::MarkdownItalic
                | Self::MarkdownCode
                | Self::MarkdownLink
        )
    }
```

- [ ] **Step 4: Implement the keymap** (`src/window/keymap.rs`)

Append to `COMMAND_IDS`:
```rust
    (CommandId::MarkdownToggleLive, "markdown.toggleLive"),
    (CommandId::MarkdownBold, "markdown.bold"),
    (CommandId::MarkdownItalic, "markdown.italic"),
    (CommandId::MarkdownCode, "markdown.code"),
    (CommandId::MarkdownLink, "markdown.link"),
```
Append to `DEFAULT_BINDINGS` (length 82 → 87):
```rust
    (key(C | A, ch(b'V')), CommandId::MarkdownToggleLive),
    // Markdown-scoped (live mode spec §9): these never shadow a global binding.
    (key(C, ch(b'B')), CommandId::MarkdownBold),
    (key(C, ch(b'I')), CommandId::MarkdownItalic),
    (key(C, VK_OEM_3), CommandId::MarkdownCode),
    (key(C, ch(b'K')), CommandId::MarkdownLink),
```
Make lookups scope-aware:
```rust
    pub(crate) fn command_for(&self, stroke: KeyStroke) -> Option<CommandId> {
        self.command_for_in(stroke, Scope::Global)
    }

    pub(crate) fn command_for_in(&self, stroke: KeyStroke, scope: Scope) -> Option<CommandId> {
        self.bindings
            .iter()
            .find(|binding| binding.stroke == stroke && binding.command.scope() == scope)
            .map(|binding| binding.command)
    }
```
- In `conflicts(&self, stroke, except)`, add `&& binding.command.scope() == except.scope()` to its filter.
- In `first_text`, the "held by a higher binding" check must only consider bindings in the same scope as `command`. Add the same scope comparison to that check.

- [ ] **Step 5: Accelerator table, scoped dispatch, menu**

`src/window/menus.rs`, `AcceleratorTable::create`: build entries only from `keymap.bindings().iter().filter(|binding| binding.command.scope() == Scope::Global)`. Markdown-scoped keys are dispatched before the table (below).

Update the existing menu tests:
- l.865: `specs.len()` changes to whatever the test now computes. That is 87 if `specs` counts keymap bindings, or 83 if it counts table entries. Read the test's definition of `specs` and set the right one.
- l.874: `table.entries().len()` changes from 82 to 83.
- l.892 (`every_shortcut_chord_maps_to_exactly_one_command`): key the uniqueness set by `(binding.command.scope(), binding.stroke)`.
- l.969 (Ctrl+B is ToggleSidebar) and l.985 (Ctrl+K has no table entry) stay as they are and must still pass.

Add to the View popup, after `"Close Markdown pre&view"`:
```rust
                        MenuEntry::command("&Live Markdown", CommandId::MarkdownToggleLive),
```
and the helper (next to `set_markdown_preview_enabled`):
```rust
/// Grays Live Markdown off Markdown tabs and checks it while the active document is Live.
pub(crate) fn set_markdown_live(menu: HMENU, enabled: bool, checked: bool) {
    let command = CommandId::MarkdownToggleLive as u32;
    let enable = if enabled { MF_ENABLED } else { MF_GRAYED };
    let check = if checked { MF_CHECKED } else { MF_UNCHECKED };
    unsafe {
        EnableMenuItem(menu, command, MF_BYCOMMAND | enable);
        CheckMenuItem(menu, command, MF_BYCOMMAND | check);
    }
}
```
In `menu_keys.rs` `open_menu`, inside the `VIEW_MENU_INDEX` block, add:
```rust
                let markdown = active_language(hwnd) == crate::document::Language::Markdown;
                menus::set_markdown_live(menu, markdown, crate::window::live_host::is_live(hwnd));
```
`live_host::is_live` arrives in Task 12. Until then, add a stub in a new `src/window/live_host.rs` and add `pub(crate) mod live_host;` to `src/window/mod.rs`:
```rust
//! Live Markdown and the Markdown writing helpers in the window layer (live mode spec §4, §8).

use windows_sys::Win32::Foundation::HWND;

/// Whether the active document is shown in Live Markdown.
pub(crate) fn is_live(_hwnd: HWND) -> bool {
    false
}
```

In `menu_keys.rs` `translate_accelerator`, insert this as the last guard before the accelerator table is consulted, after `editing_key_off_editor`:
```rust
    if markdown_key(hwnd, message) {
        return true;
    }
```
and add:
```rust
/// A Markdown-scoped shortcut (live mode spec §9): taken only when a Markdown tab's editor has
/// focus, so the same key keeps its global command everywhere else.
fn markdown_key(hwnd: HWND, message: &MSG) -> bool {
    if message.message != WM_KEYDOWN {
        return false;
    }
    if active_language(hwnd) != crate::document::Language::Markdown {
        return false;
    }
    if unsafe { super::editor_hwnd(hwnd) } != Some(message.hwnd) {
        return false;
    }
    let down = |key| unsafe { GetKeyState(i32::from(key)) } < 0;
    let Some(stroke) = KeyStroke::from_key(
        message.wParam as u16,
        down(VK_CONTROL),
        down(VK_SHIFT),
        down(VK_MENU),
    ) else {
        return false;
    };
    let command = unsafe { app_ptr(hwnd) }.and_then(|app| {
        unsafe { app.as_ref() }.keymap.command_for_in(stroke, Scope::Markdown)
    });
    let Some(command) = command else {
        return false;
    };
    execute_command(hwnd, command);
    true
}
```
Copy the `down` helper, imports and `editor_hwnd` path exactly as `editing_key_off_editor` (l.300-320) uses them. If that function reads the key state some other way, use its way.

- [ ] **Step 6: Palette, availability, shortcuts titles**

- `src/window/command_palette.rs` `ENTRIES` (128 → 133), after the Markdown preview entries:
```rust
    entry("Markdown: Toggle live mode", CommandId::MarkdownToggleLive),
    entry("Markdown: Bold", CommandId::MarkdownBold),
    entry("Markdown: Italic", CommandId::MarkdownItalic),
    entry("Markdown: Inline code", CommandId::MarkdownCode),
    entry("Markdown: Link", CommandId::MarkdownLink),
```
  - Update `command_palette/tests.rs:93` to 133.
  - Labels start with "Markdown:" and do not contain "markdown preview", so the test at l.370 still holds.
- `command_palette_ui.rs` availability (l.525-541): add
```rust
    && (markdown_document || !command.is_markdown_edit())
```
  where `let markdown_document = active_language(hwnd) == crate::document::Language::Markdown;`. Not `buttons_visible`, which is also true for SVG.
- `main_window/tests/command_palette.rs:28-35`: the plain-text shown count must also subtract `ENTRIES.iter().filter(|e| e.command.is_markdown_edit()).count()`.
- `shortcuts_model.rs` `title()`: a Markdown-scoped command's title gets the suffix `" (Markdown files)"`.
  - If `title` returns `&'static str`, change it to `Cow<'static, str>`.
  - Update its callers. `ShortcutRow.title` becomes `String` if it is `&'static str`.
  - The test `every_command_has_a_title_and_a_row` needs no change: the palette entries supply the titles.

- [ ] **Step 7: Run the tests**

Run: `cargo clippy --all-targets`, then `cargo test --lib commands keymap menus command_palette shortcuts markdown_keys -- --test-threads=1`
Expected: all pass.

- [ ] **Step 8: Commit**

```bash
git add src/window
git commit -m "feat(keys): Markdown-scoped bindings, Live Markdown and format commands"
```

---

### Task 10: Format commands edit the text

**Files:**
- Modify: `src/editor/scintilla/live_ops.rs` (add `set_selections`, `apply_plan`, `eol`)
- Modify: `src/window/live_host.rs` (add `format`)
- Modify: `src/window/main_window/command_dispatch.rs` (arms next to the preview arm, l.439)
- Modify: `src/window/main_window/startup.rs:287` (`with_editor` becomes `pub(crate)`)
- Test: `src/window/main_window/tests/markdown_keys.rs`

**Interfaces:**
- Consumes: `markdown_edit::{toggle_marker, insert_link, EditPlan}` (Task 6).
- Produces:
  - `Editor::set_selections(&self, selections: &[Range<usize>]) -> Result<()>`
  - `Editor::apply_plan(&self, plan: &EditPlan) -> Result<()>`: one undo step
  - `Editor::eol(&self) -> Result<&'static str>`
  - `live_host::format(hwnd: HWND, command: CommandId)`

- [ ] **Step 1: Write the failing tests** (append to `markdown_keys.rs`):

```rust
/// One Undo restores `original`.
fn assert_one_undo_restores(editor: &crate::editor::Editor, original: &str) {
    editor.undo().unwrap();
    assert_eq!(editor.text().unwrap(), original);
}

#[test]
fn ctrl_b_bolds_the_word_under_the_caret_in_one_undo_step() {
    let f = fixture("hello world", Language::Markdown);
    f.editor.set_selection(2..2).unwrap();
    translate_key_with(f.window.hwnd, f.editor.hwnd(), b'B', true, false, false);
    assert_eq!(f.editor.text().unwrap(), "**hello** world");
    assert_eq!(f.editor.selection().unwrap(), 4..4);
    assert_one_undo_restores(&f.editor, "hello world");
}

#[test]
fn ctrl_i_ctrl_backtick_and_ctrl_k_wrap_the_selection() {
    let f = fixture("a b", Language::Markdown);
    f.editor.set_selection(2..3).unwrap();
    translate_key_with(f.window.hwnd, f.editor.hwnd(), b'I', true, false, false);
    assert_eq!(f.editor.text().unwrap(), "a *b*");
    f.editor.set_selection(0..1).unwrap();
    translate_key_with(f.window.hwnd, f.editor.hwnd(), 0xC0, true, false, false); // VK_OEM_3
    assert_eq!(f.editor.text().unwrap(), "`a` *b*");
    f.editor.set_selection(0..3).unwrap();
    translate_key_with(f.window.hwnd, f.editor.hwnd(), b'K', true, false, false);
    assert_eq!(f.editor.text().unwrap(), "[`a`]() *b*");
    assert_eq!(f.editor.selection().unwrap(), 6..6);
}

#[test]
fn ctrl_b_in_a_text_file_leaves_the_text_alone() {
    let f = fixture("hello", Language::PlainText);
    f.editor.set_selection(2..2).unwrap();
    translate_key_with(f.window.hwnd, f.editor.hwnd(), b'B', true, false, false);
    assert_eq!(f.editor.text().unwrap(), "hello");
}
```

- [ ] **Step 2: Run them and watch them fail**

Run: `cargo test --lib markdown_keys -- --test-threads=1`
Expected: the new tests fail, because the text is unchanged (the command has no arm yet).

- [ ] **Step 3: Implement**

`live_ops.rs`. Add `SCI_SETSELECTION`, `SCI_ADDSELECTION`, `SCI_GETEOLMODE`, `SC_EOL_CR`, `SC_EOL_LF` to its imports (they already exist in the constants) and add:
```rust
    /// Replaces the selections; each range's start is the anchor and its end the caret.
    pub fn set_selections(&self, selections: &[Range<usize>]) -> Result<()> {
        let Some((first, rest)) = selections.split_first() else {
            return Ok(());
        };
        self.live_send(SCI_SETSELECTION, first.end, first.start as isize)?;
        for selection in rest {
            self.live_send(SCI_ADDSELECTION, selection.end, selection.start as isize)?;
        }
        Ok(())
    }

    /// Applies a Markdown helper's plan as one undo step (live mode spec §8).
    pub fn apply_plan(&self, plan: &crate::editor::markdown_edit::EditPlan) -> Result<()> {
        self.begin_undo_action();
        let result = plan
            .edits
            .iter()
            .rev()
            .try_for_each(|edit| self.replace_target(edit.range.clone(), &edit.text).map(drop))
            .and_then(|()| self.set_selections(&plan.selections));
        self.end_undo_action();
        result
    }

    /// The line ending Enter inserts.
    pub fn eol(&self) -> Result<&'static str> {
        Ok(match self.live_send(SCI_GETEOLMODE, 0, 0)? as u32 {
            SC_EOL_LF => "\n",
            SC_EOL_CR => "\r",
            _ => "\r\n",
        })
    }
```
Edits are applied last-first, so the pre-edit offsets of the earlier edits stay valid.

`live_host.rs`:
```rust
use crate::document::Language;
use crate::editor::markdown_edit::{insert_link, toggle_marker};
use crate::window::commands::CommandId;
use crate::window::main_window as host_window;

/// The format commands (live mode spec §8.1): toggle a marker around each selection, or insert
/// a link, as one undo step.
pub(crate) fn format(hwnd: HWND, command: CommandId) {
    if host_window::active_language(hwnd) != Language::Markdown {
        return;
    }
    host_window::with_editor(hwnd, |editor| {
        let Ok(selections) = editor.selections() else {
            return;
        };
        let plan = editor.with_document_text(|text| match command {
            CommandId::MarkdownBold => toggle_marker(text, &selections, "**"),
            CommandId::MarkdownItalic => toggle_marker(text, &selections, "*"),
            CommandId::MarkdownCode => toggle_marker(text, &selections, "`"),
            _ => insert_link(text, &selections),
        });
        if let Ok(plan) = plan {
            let _ = editor.apply_plan(&plan);
        }
    });
}
```
- Make `active_language` (`language_tools.rs:74`) and `with_editor` (`startup.rs:287`) `pub(crate)`.
- Re-export them through `main_window` the way `push_notice` is reached (`host_window::push_notice`).
- `Editor::selections()` is `pub(crate)` already.

`command_dispatch.rs`, next to the preview arm:
```rust
        CommandId::MarkdownBold
        | CommandId::MarkdownItalic
        | CommandId::MarkdownCode
        | CommandId::MarkdownLink => crate::window::live_host::format(hwnd, command),
```

- [ ] **Step 4: Run the tests**

Run: `cargo test --lib markdown_keys -- --test-threads=1`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add src/editor/scintilla/live_ops.rs src/window
git commit -m "feat(markdown): Ctrl+B/I/\`/K format the selection in Markdown tabs"
```

---

### Task 11: Enter and Tab helpers, table auto-format on leave

**Files:**
- Modify: `src/window/live_host.rs` (`GroupHooks`, `LiveRegistry`, `DocState`, `text_changed`, `selection_changed`, `run_deferred`)
- Modify: `src/app.rs` (field `pub(crate) live: crate::window::live_host::LiveRegistry`, default-initialized)
- Modify: `src/window/messages.rs` (new `WM_FASTPAD_LIVE_DEFERRED`: the next unused `WM_APP + n`, which is `0x5B` if `0x5A` is still the highest)
- Modify: `src/window/main_window/wndproc.rs` (`WM_FASTPAD_LIVE_DEFERRED` arm; `SCN_MODIFIED` and `SCN_UPDATEUI` forwarding in `handle_editor_notification` l.634)
- Modify: `src/window/main_window/group_layout.rs:345` `configure_editor` and `src/window/main_window/startup.rs:167` `initialize_editor_with` (install hooks)
- Test: `src/window/main_window/tests/markdown_keys.rs`

**Interfaces:**
- Consumes:
  - `markdown_edit::{enter_in_list, indent_list_item, next_cell, table_at, format_table}`
  - `Editor::{apply_plan, eol, set_hooks}`
  - `EditorHooks`
- Produces:
```rust
pub(crate) struct GroupHooks { main: HWND, editor: HWND }   // impl EditorHooks
impl GroupHooks { pub(crate) fn new(main: HWND, editor: HWND) -> Self }
#[derive(Default)] pub(crate) struct LiveRegistry { docs: HashMap<DocumentId, DocState>, pending: Vec<GroupId>, posted: bool }
#[derive(Default)] pub(crate) struct DocState { dirty_table: Option<usize>, formatting: bool }
pub(crate) fn text_changed(hwnd: HWND, editor: &Editor, document: DocumentId, position: usize);
pub(crate) fn selection_changed(hwnd: HWND, group: GroupId);   // posts WM_FASTPAD_LIVE_DEFERRED once
pub(crate) fn run_deferred(hwnd: HWND);
/// The group whose editor window is `editor`, its editor and active document.
pub(crate) fn group_of_editor(hwnd: HWND, editor: HWND) -> Option<(GroupId, Editor, DocumentId, Language)>;
```

- [ ] **Step 1: Write the failing tests** (append to `markdown_keys.rs`):

```rust
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_RETURN, VK_TAB};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, MSG, PM_REMOVE, PeekMessageW, TranslateMessage, WM_KEYDOWN,
};

/// Presses `key` as the message loop delivers it: accelerators first, then TranslateMessage
/// and dispatch, then the WM_CHAR that produced, then anything posted meanwhile.
fn press(f: &Fixture, key: u16, shift: bool) {
    let identity = unsafe { super::window_identity(f.window.hwnd).unwrap() };
    let message = MSG {
        hwnd: f.editor.hwnd(),
        message: WM_KEYDOWN,
        wParam: usize::from(key),
        ..Default::default()
    };
    with_shift(shift, || unsafe {
        if !super::translate_accelerator(f.window.hwnd, &identity, &message) {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        drain_messages();
    });
}

fn with_shift(shift: bool, run: impl FnOnce()) {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetKeyboardState, SetKeyboardState, VK_SHIFT,
    };
    let mut keys = [0u8; 256];
    unsafe { GetKeyboardState(keys.as_mut_ptr()) };
    let original = keys;
    keys[VK_SHIFT as usize] = if shift { 0x80 } else { 0 };
    unsafe { SetKeyboardState(keys.as_ptr()) };
    run();
    unsafe { SetKeyboardState(original.as_ptr()) };
}

fn drain_messages() {
    let mut message = MSG::default();
    while unsafe { PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_REMOVE) } != 0 {
        unsafe {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

#[test]
fn enter_continues_a_list_through_the_real_key_path() {
    let f = fixture("- a\r\nx", Language::Markdown);
    f.editor.set_selection(3..3).unwrap();
    press(&f, VK_RETURN, false);
    assert_eq!(f.editor.text().unwrap(), "- a\r\n- \r\nx");
    assert_eq!(f.editor.selection().unwrap(), 7..7);
}

#[test]
fn enter_on_an_empty_item_ends_the_list() {
    let f = fixture("- a\r\n- ", Language::Markdown);
    f.editor.set_selection(7..7).unwrap();
    press(&f, VK_RETURN, false);
    assert_eq!(f.editor.text().unwrap(), "- a\r\n");
}

#[test]
fn enter_outside_lists_and_in_text_files_is_a_plain_newline() {
    let f = fixture("ab", Language::Markdown);
    f.editor.set_selection(1..1).unwrap();
    press(&f, VK_RETURN, false);
    assert_eq!(f.editor.text().unwrap(), "a\r\nb");
    let plain = fixture("- a", Language::PlainText);
    plain.editor.set_selection(3..3).unwrap();
    press(&plain, VK_RETURN, false);
    assert_eq!(plain.editor.text().unwrap(), "- a\r\n");
}

#[test]
fn tab_nests_a_list_item_and_shift_tab_un_nests_it() {
    let f = fixture("- a", Language::Markdown);
    f.editor.set_selection(3..3).unwrap();
    press(&f, VK_TAB, false);
    assert_eq!(f.editor.text().unwrap(), "  - a");
    press(&f, VK_TAB, true);
    assert_eq!(f.editor.text().unwrap(), "- a");
}

#[test]
fn tab_in_a_table_selects_the_next_cell() {
    let text = "| a | b |\r\n| - | - |\r\n| c | d |";
    let f = fixture(text, Language::Markdown);
    let b = text.find('b').unwrap();
    f.editor.set_selection(2..2).unwrap();
    press(&f, VK_TAB, false);
    assert_eq!(f.editor.selection().unwrap(), b..b + 1);
    assert_eq!(f.editor.text().unwrap(), text);
}

#[test]
fn a_table_is_formatted_when_the_caret_leaves_it_in_one_undo_step() {
    let f = fixture("|a|b|\r\n|-|-|\r\n|c|d|\r\n\r\nx", Language::Markdown);
    f.editor.set_selection(1..1).unwrap();
    f.editor.replace_target(1..1, "z").unwrap(); // an edit inside the table
    drain_messages();
    let edited = f.editor.text().unwrap();
    let x = edited.len() - 1;
    f.editor.set_selection(x..x).unwrap();
    drain_messages();
    assert_eq!(
        f.editor.text().unwrap(),
        "| za  | b   |\r\n| --- | --- |\r\n| c   | d   |\r\n\r\nx"
    );
    f.editor.undo().unwrap();
    assert_eq!(f.editor.text().unwrap(), edited);
}

#[test]
fn an_untouched_table_is_not_rewritten_when_the_caret_passes_through() {
    let text = "|a|b|\r\n|-|-|\r\n\r\nx";
    let f = fixture(text, Language::Markdown);
    f.editor.set_selection(1..1).unwrap();
    drain_messages();
    f.editor.set_selection(text.len()..text.len()).unwrap();
    drain_messages();
    assert_eq!(f.editor.text().unwrap(), text);
    assert!(!f.editor.can_undo().unwrap());
}
```
- The format test edits through `replace_target`, which raises `SCN_MODIFIED` exactly as typing does.
- `install_test_editor` must leave the editor wired to the main window's `WM_NOTIFY`, which it does through `initialize_editor_with`.

- [ ] **Step 2: Run them and watch them fail**

Run: `cargo test --lib markdown_keys -- --test-threads=1`
Expected: the new tests fail. Enter inserts a plain newline, and the table stays unformatted.

- [ ] **Step 3: Implement the hooks and state** in `live_host.rs`:

```rust
use crate::document::DocumentId;
use crate::editor::markdown_edit::{
    enter_in_list, format_table, indent_list_item, next_cell, table_at,
};
use crate::editor::{Editor, EditorHooks};
use crate::window::tabs::GroupId; // use the path GroupId is exported from
use std::collections::HashMap;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_RETURN, VK_TAB};
use windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW;

#[derive(Default)]
pub(crate) struct LiveRegistry {
    docs: HashMap<DocumentId, DocState>,
    /// Groups whose selection changed since the deferred message was posted.
    pending: Vec<GroupId>,
    posted: bool,
}

#[derive(Default)]
pub(crate) struct DocState {
    /// A byte inside a table the user edited; the table is formatted when the caret leaves it.
    dirty_table: Option<usize>,
    /// Set while FastPad itself rewrites a table, so that edit does not mark it dirty again.
    formatting: bool,
}

fn with_registry<R>(hwnd: HWND, run: impl FnOnce(&mut LiveRegistry) -> R) -> Option<R> {
    // SAFETY: the App pointer is used only inside `run`, which makes no Win32 call.
    unsafe { host_window::app_ptr(hwnd) }.map(|mut app| run(&mut unsafe { app.as_mut() }.live))
}

pub(crate) fn group_of_editor(
    hwnd: HWND,
    editor: HWND,
) -> Option<(GroupId, Editor, DocumentId, Language)> {
    let app = unsafe { host_window::app_ptr(hwnd)?.as_ref() };
    app.tabs.group_ids().into_iter().find_map(|group| {
        let group_editor = host_window::group_editor(hwnd, group)?;
        if group_editor.hwnd() != editor {
            return None;
        }
        let document = app.tabs.group(group)?.active_document()?;
        let language = app.tabs.document(document)?.language;
        Some((group, group_editor, document, language))
    })
}

pub(crate) struct GroupHooks {
    main: HWND,
    editor: HWND,
}

impl GroupHooks {
    pub(crate) fn new(main: HWND, editor: HWND) -> Self {
        Self { main, editor }
    }

    fn markdown_editor(&self) -> Option<Editor> {
        let (_, editor, _, language) = group_of_editor(self.main, self.editor)?;
        (language == Language::Markdown).then_some(editor)
    }
}

impl EditorHooks for GroupHooks {
    fn key_down(&self, vk: u16, ctrl: bool, shift: bool, alt: bool) -> bool {
        if ctrl || alt || !(vk == VK_RETURN || vk == VK_TAB) {
            return false;
        }
        let Some(editor) = self.markdown_editor() else {
            return false;
        };
        let Ok(selections) = editor.selections() else {
            return false;
        };
        let [selection] = selections.as_slice() else {
            return false; // multiple carets: Scintilla's own Enter / Tab
        };
        let caret = selection.end;
        let fallback = editor.eol().unwrap_or("\r\n");
        let plan = editor.with_document_text(|text| {
            if vk == VK_RETURN {
                if shift || !selection.is_empty() {
                    return None;
                }
                return enter_in_list(text, caret, fallback);
            }
            let line = |at| crate::editor::markdown_edit::line_bounds(text, at);
            if line(selection.start) != line(selection.end) {
                return None; // a multi-line selection: Scintilla indents the lines
            }
            if let Some(cell) = next_cell(text, caret, shift) {
                return Some(crate::editor::markdown_edit::EditPlan {
                    edits: Vec::new(),
                    selections: vec![cell],
                });
            }
            if selection.is_empty() {
                return indent_list_item(text, caret, shift);
            }
            None
        });
        match plan {
            Ok(Some(plan)) => editor.apply_plan(&plan).is_ok(),
            _ => false,
        }
    }
}
```
- `line_bounds` is `pub(crate)` from Task 7. `group_editor` (`group_layout.rs:240`) is `pub(crate)` already.
- Make `app_ptr` reachable as `host_window::app_ptr` if it isn't already. preview_host uses it that way.
- An empty plan (cell navigation) through `apply_plan` only sets the selection. Its undo group is empty, and Scintilla records nothing for an empty group.

Tracking and formatting tables:
```rust
/// SCN_MODIFIED for a Markdown document (live mode spec §8.3): remembers a table the user
/// edited. Reads only; never edits inside the notification.
pub(crate) fn text_changed(hwnd: HWND, editor: &Editor, document: DocumentId, position: usize) {
    let formatting = with_registry(hwnd, |registry| {
        registry.docs.get(&document).is_some_and(|state| state.formatting)
    });
    if formatting != Some(false) {
        return;
    }
    let in_table = editor.with_document_text(|text| table_at(text, position.min(text.len())).is_some());
    if in_table == Ok(true) {
        with_registry(hwnd, |registry| {
            registry.docs.entry(document).or_default().dirty_table = Some(position);
        });
    }
}

/// SCN_UPDATEUI with a selection change: the work runs from a posted message, never inside
/// Scintilla's notification.
pub(crate) fn selection_changed(hwnd: HWND, group: GroupId) {
    let post = with_registry(hwnd, |registry| {
        if !registry.pending.contains(&group) {
            registry.pending.push(group);
        }
        !std::mem::replace(&mut registry.posted, true)
    });
    if post == Some(true) {
        unsafe { PostMessageW(hwnd, crate::window::messages::WM_FASTPAD_LIVE_DEFERRED, 0, 0) };
    }
}

pub(crate) fn run_deferred(hwnd: HWND) {
    let groups = with_registry(hwnd, |registry| {
        registry.posted = false;
        std::mem::take(&mut registry.pending)
    })
    .unwrap_or_default();
    for group in groups {
        format_left_table(hwnd, group);
        // Task 13 adds: reveal(hwnd, group);
    }
}

fn format_left_table(hwnd: HWND, group: GroupId) {
    let Some(editor) = host_window::group_editor(hwnd, group) else {
        return;
    };
    let Some((_, _, document, _)) = group_of_editor(hwnd, editor.hwnd()) else {
        return;
    };
    let dirty = with_registry(hwnd, |registry| {
        registry.docs.get(&document).and_then(|state| state.dirty_table)
    });
    let Some(dirty) = dirty.flatten() else {
        return;
    };
    let Ok(caret) = editor.selection().map(|selection| selection.end) else {
        return;
    };
    let rewrite = editor.with_document_text(|text| {
        let table = table_at(text, dirty.min(text.len()))?;
        if (table.start..=table.end).contains(&caret) {
            return Some(None); // still inside: keep waiting
        }
        Some(format_table(&text[table.clone()]).map(|formatted| (table, formatted)))
    });
    match rewrite {
        Ok(Some(None)) => {}
        Ok(Some(Some((table, formatted)))) => {
            set_formatting(hwnd, document, true);
            editor.begin_undo_action();
            let _ = editor.replace_target(table, &formatted);
            editor.end_undo_action();
            set_formatting(hwnd, document, false);
            clear_dirty(hwnd, document);
        }
        _ => clear_dirty(hwnd, document),
    }
}

fn set_formatting(hwnd: HWND, document: DocumentId, formatting: bool) {
    with_registry(hwnd, |registry| registry.docs.entry(document).or_default().formatting = formatting);
}

fn clear_dirty(hwnd: HWND, document: DocumentId) {
    with_registry(hwnd, |registry| {
        if let Some(state) = registry.docs.get_mut(&document) {
            state.dirty_table = None;
        }
    });
}
```
Wire it up:
- `messages.rs`: `pub const WM_FASTPAD_LIVE_DEFERRED: u32 = WM_APP + 0x5B;`, with a doc comment ("Live Markdown work deferred out of a Scintilla notification").
- `wndproc.rs`: add the arm `WM_FASTPAD_LIVE_DEFERRED => { crate::window::live_host::run_deferred(hwnd); 0 }`.
- `handle_editor_notification`, `SCN_UPDATEUI` branch, before its `return`:
```rust
      if update.updated as u32 & SC_UPDATE_SELECTION != 0 {
          crate::window::live_host::selection_changed(hwnd, group);
      }
```
- `handle_editor_notification`, `SCN_MODIFIED` text-change branch, after `note_text_change`, when the document's language is Markdown:
```rust
      crate::window::live_host::text_changed(hwnd, &editor, document, modification.position.max(0) as usize);
```
  Read the language the way the branch already reads the document (`app.tabs.document(document)`). Copy the values out before the call; never hold an `&App` across it.
- Hooks install: in `configure_editor(hwnd, editor)` and right after the editor is created in `initialize_editor_with`, add
```rust
    editor.set_hooks(Some(std::rc::Rc::new(crate::window::live_host::GroupHooks::new(
        hwnd,
        editor.hwnd(),
    ))));
```
- `app.rs`: add the field `pub(crate) live: crate::window::live_host::LiveRegistry,`, initialized with `Default::default()` in `App::new`.

- [ ] **Step 4: Run the tests**

Run: `cargo test --lib markdown_keys editing_shortcuts -- --test-threads=1`
Expected: all pass. The editing-shortcut tests must keep passing, because hooks are now installed on every editor.

- [ ] **Step 5: Commit**

```bash
git add src/window src/app.rs
git commit -m "feat(markdown): Enter continues lists, Tab nests and walks table cells, tables format on leave"
```

---

### Task 12: Live on and off, setting, container styling

This task ships Live with styling only, with no painted decorations yet. Markers are hidden and blanked, and Lexilla is swapped out and back in.

**Files:**
- Modify: `src/config/defaults.rs`, `src/config/persisted.rs` (setting `markdown_live_default`)
- Modify: `src/document.rs` (`pub live: Option<bool>` on `Document`, `None` in every constructor)
- Modify: `src/window/live_host.rs` (`LiveState`, `toggle`, `sync`, `sync_all`, `is_live`, `style_needed`, `record_edit`)
- Modify: `src/window/preview_host.rs` (`ScintillaSource` → `pub(crate)`; extract `pub(crate) fn edit_from(notification: &ScintillaNotification) -> Edit` from `record_edit` and call it there)
- Modify: `src/window/main_window/command_dispatch.rs` (toggle arm)
- Modify: `src/window/main_window/wndproc.rs` (`SCN_STYLENEEDED` branch; `record_edit` from `SCN_MODIFIED`)
- Modify: `src/window/main_window/language_tools.rs:26` (`apply_language`), `split_groups.rs:380` (`style_group_view`), `settings_apply.rs:384` (`apply_editor_settings`): call sync
- Modify: `src/live/styles.rs` (add `apply_style_table`)
- Create: `tests/windows/markdown_live.rs`; add a `[[test]] name = "markdown_live" path = "tests/windows/markdown_live.rs"` entry to `Cargo.toml` next to `markdown_preview`

**Interfaces:**
- Consumes:
  - `LiveDocument` (Task 4)
  - `styles::{style_table, STRIKE_INDICATOR, ANNOTATION}`, `styler::style_runs` (Task 5)
  - `Editor` live ops (Task 1)
  - `LIVE_MAX_BYTES`
- Produces:
```rust
pub(crate) struct LiveState { model: LiveDocument, edits: Vec<Edit>, revealed: BTreeSet<usize> }
// DocState gains: live: Option<LiveState>
pub(crate) fn is_live(hwnd: HWND) -> bool;                       // replaces the Task 9 stub
pub(crate) fn toggle(hwnd: HWND);
pub(crate) fn sync(hwnd: HWND, group: GroupId);                  // idempotent
pub(crate) fn sync_all(hwnd: HWND);
pub(crate) fn style_needed(hwnd: HWND, group: GroupId, position: usize);
pub(crate) fn record_edit(hwnd: HWND, document: DocumentId, notification: &ScintillaNotification);
fn style_range(editor: &Editor, state: &LiveState, range: Range<usize>) -> Result<()>;
// styles.rs
pub fn apply_style_table(editor: &Editor, colors: &SyntaxColors, prose_font: &str, mono_font: &str) -> Result<()>;
```
  - `Settings.markdown_live_default: bool`
  - `Document.live: Option<bool>`: `None` follows the setting.

- [ ] **Step 1: Write the failing setting test** in `src/config/persisted.rs` tests, modelled on `code_folding_parses_as_a_bool_and_defaults_on` (l.1196):

```rust
    #[test]
    fn markdown_live_default_parses_as_a_bool_and_defaults_off() {
        assert!(!default_settings().markdown_live_default);
        let delta = parse("markdown_live_default=on\n");
        assert_eq!(delta.markdown_live_default, Some(true));
        let delta = parse("markdown_live_default=maybe\n");
        assert_eq!(delta.markdown_live_default, None);
        assert_eq!(delta.warnings.len(), 1);
    }
```
(Use the same `parse`/`warnings` names the neighbouring test uses.)

- [ ] **Step 2: Write the failing Live tests.** Create `tests/windows/markdown_live.rs`:

  - Copy from `tests/windows/markdown_preview.rs`:
    - the file header: `#![cfg(windows)]`, `mod support;`, `include!("../../src/lib.rs");`
    - the `TestMain` struct with its `impl` and `Drop`
    - `pump_until`, `pump_pending`, `pump_for`, `type_text`
  - Leave out the preview-only helpers (`mode`, `view`).
  - Then add:

```rust
use crate::live::styles;

impl TestMain {
    fn editor_api(&self) -> crate::editor::Editor {
        self.with_app(|app| app.editor().cloned().unwrap())
    }

    /// Styles the whole document now, as painting would.
    fn styled(&self) -> crate::editor::Editor {
        let editor = self.editor_api();
        let length = editor.length().unwrap();
        editor.colourise(0..length).unwrap();
        editor
    }

    fn is_live(&self) -> bool {
        crate::window::live_host::is_live(self.hwnd)
    }

    fn select(&self, at: usize) {
        self.editor_api().set_selection(at..at).unwrap();
        pump_pending();
    }
}

#[test]
fn toggling_live_hides_markup_off_the_caret_line() {
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    main.make_markdown("x\na **b** c\n");
    main.select(0);
    main.command(CommandId::MarkdownToggleLive);
    assert!(main.is_live());
    let editor = main.styled();
    // "x\n" is 2 bytes: the markers sit at 4..6 and 7..9, the bold text at 6.
    assert_eq!(editor.style_at(4).unwrap(), styles::HIDDEN);
    assert_eq!(editor.style_at(6).unwrap(), styles::BOLD);
    assert_eq!(editor.style_at(8).unwrap(), styles::HIDDEN);
}

#[test]
fn toggling_live_off_restores_lexilla() {
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    main.make_markdown("x\na **b** c\n");
    main.command(CommandId::MarkdownToggleLive);
    main.command(CommandId::MarkdownToggleLive);
    assert!(!main.is_live());
    let editor = main.styled();
    assert_eq!(
        editor.style_at(4).unwrap(),
        crate::editor::scintilla_constants::SCE_MARKDOWN_STRONG1 as u8
    );
}

#[test]
fn the_default_setting_opens_markdown_in_live() {
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    main.with_app(|app| app.settings.markdown_live_default = true);
    main.make_markdown("x\n# t\n");
    let group = main.with_app(|app| app.tabs.active_group());
    crate::window::live_host::sync(main.hwnd, group);
    assert!(main.is_live());
}

#[test]
fn live_on_a_text_tab_shows_a_notice() {
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    main.set_text("x");
    main.command(CommandId::MarkdownToggleLive);
    assert!(!main.is_live());
    assert!(main.notices().iter().any(|n| n.contains("Live Markdown is available")));
}

#[test]
fn live_is_refused_over_one_megabyte() {
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    main.make_markdown(&"a".repeat(crate::live::LIVE_MAX_BYTES + 1));
    main.command(CommandId::MarkdownToggleLive);
    assert!(!main.is_live());
    assert!(main.notices().iter().any(|n| n.contains("over 1 MB")));
}

#[test]
fn growing_past_one_megabyte_turns_live_off() {
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    main.make_markdown(&"a".repeat(crate::live::LIVE_MAX_BYTES - 1));
    main.command(CommandId::MarkdownToggleLive);
    assert!(main.is_live());
    let editor = main.editor_api();
    let end = editor.length().unwrap();
    editor.replace_target(end..end, "bbbb").unwrap();
    pump_until("live off", Duration::from_secs(2), || !main.is_live());
    assert!(main.notices().iter().any(|n| n.contains("over 1 MB")));
}

#[test]
fn switching_tabs_restores_lexilla_and_back() {
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    main.make_markdown("x\na **b** c\n");
    main.select(0);
    main.command(CommandId::MarkdownToggleLive);
    main.command(CommandId::New);
    main.make_markdown("x\na **b** c\n");
    assert!(!main.is_live(), "Live belongs to the first document only");
    let strong = crate::editor::scintilla_constants::SCE_MARKDOWN_STRONG1 as u8;
    assert_eq!(main.styled().style_at(4).unwrap(), strong);
    main.command(CommandId::SelectTab1);
    pump_pending();
    assert!(main.is_live());
    main.select(0);
    assert_eq!(main.styled().style_at(4).unwrap(), styles::HIDDEN);
}

#[test]
fn an_edit_that_closes_bold_restyles_the_opening_marker() {
    // Break caught: Scintilla only restyles from the edit onwards, but closing `**` changes the
    // opening marker earlier on the same line.
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    main.make_markdown("x\n**b\n");
    main.select(0);
    main.command(CommandId::MarkdownToggleLive);
    assert_ne!(main.styled().style_at(2).unwrap(), styles::HIDDEN);
    let editor = main.editor_api();
    editor.replace_target(5..5, "**").unwrap();
    main.select(0);
    assert_eq!(main.styled().style_at(2).unwrap(), styles::HIDDEN);
}
```
- If `TestMain::set_text` is private to its `impl`, keep the copied version as is.
- `CommandId::New` and `CommandId::SelectTab1` are existing commands. `SelectTab1` is bound to Alt+1.

- [ ] **Step 3: Run them and watch them fail**

Run: `cargo test --lib markdown_live_default`, then `cargo test --test markdown_live -- --test-threads=1`
Expected: compile errors. There's no `markdown_live_default` field, and `is_live` is still the stub.

- [ ] **Step 4: Implement the setting**

Follow the `word_wrap` path exactly:
- `defaults.rs`: add `pub const DEFAULT_MARKDOWN_LIVE_DEFAULT: bool = false;` and set the field in `default_settings()`.
- `persisted.rs`:
  - `Settings` gets `/// Markdown tabs open in Live Markdown (live mode spec §4).` and `pub markdown_live_default: bool,`.
  - `SettingsDelta` gets `pub markdown_live_default: Option<bool>,`.
  - In `apply_line`:
```rust
        "markdown_live_default" => match parse_bool(value) {
            Some(live) => delta.markdown_live_default = Some(live),
            None => warn(delta, line_number, key, value),
        },
```
  - In `apply_delta`: `if let Some(live) = delta.markdown_live_default { self.markdown_live_default = live; }`.
  - Add `markdown_live_default` to the recognized-keys doc list at l.252.
- `document.rs`: add `/// Live Markdown: Some(on/off) once toggled, None follows the setting.` and `pub live: Option<bool>,`. Set it to `None` in `untitled`, `image` and `with_content`.

- [ ] **Step 5: Implement the style table application** (`src/live/styles.rs`):

```rust
/// Installs the Live style table and the strikethrough indicator on `editor`. Runs after the
/// language's own styles and the view settings, which reset every style's font.
pub fn apply_style_table(
    editor: &crate::editor::Editor,
    colors: &SyntaxColors,
    prose_font: &str,
    mono_font: &str,
) -> crate::Result<()> {
    for def in style_table(colors) {
        let face = if def.mono { mono_font } else { prose_font };
        let style = u32::from(def.style);
        editor.set_style(style, def.foreground, def.background, def.bold, def.italic, face)?;
        editor.set_style_underline(style, def.underline)?;
        editor.set_style_visible(style, def.visible)?;
        editor.set_style_eol_filled(style, def.eol_filled)?;
    }
    editor.define_strike_indicator(STRIKE_INDICATOR, colors.comment)?;
    editor.show_annotations(true)
}
```
`set_style` takes the face as `&str` (`styling.rs:487`) and converts it itself. If it takes a `CString`, convert here the way `apply_styles` does.

- [ ] **Step 6: Implement Live state, sync and styling** in `live_host.rs`:

```rust
use crate::editor::ScintillaNotification;
use crate::live::blocks::LiveDocument;
use crate::live::styler::style_runs;
use crate::live::styles::{STRIKE_INDICATOR, apply_style_table};
use crate::live::LIVE_MAX_BYTES;
use crate::preview::incremental::Edit;
use crate::window::preview_host::ScintillaSource;
use std::collections::BTreeSet;
use std::ops::Range;

const NOT_MARKDOWN_NOTICE: &str = "Live Markdown is available for Markdown documents. \
                                   Choose View > Markdown to treat this tab as Markdown.";
const TOO_LARGE_NOTICE: &str = "Live Markdown is off for documents over 1 MB.";

pub(crate) struct LiveState {
    model: LiveDocument,
    /// Edits since the last styling pass, applied to `model` before styling.
    edits: Vec<Edit>,
    revealed: BTreeSet<usize>,
}
```
Add `live: Option<LiveState>` to `DocState`, and `oversized: Vec<DocumentId>` to `LiveRegistry`.

```rust
/// Whether `document` should be Live now: Markdown, toggled on (or the setting), within size.
fn wanted(hwnd: HWND, document: DocumentId, length: usize) -> bool {
    let Some(app) = (unsafe { host_window::app_ptr(hwnd) }) else {
        return false;
    };
    let app = unsafe { app.as_ref() };
    let Some(doc) = app.tabs.document(document) else {
        return false;
    };
    doc.language == Language::Markdown
        && doc.live.unwrap_or(app.settings.markdown_live_default)
        && length <= LIVE_MAX_BYTES
}

pub(crate) fn is_live(hwnd: HWND) -> bool {
    let group = unsafe { host_window::app_ptr(hwnd) }.map(|app| unsafe { app.as_ref() }.tabs.active_group());
    let Some(group) = group else {
        return false;
    };
    let Some(editor) = host_window::group_editor(hwnd, group) else {
        return false;
    };
    let Some((_, _, document, _)) = group_of_editor(hwnd, editor.hwnd()) else {
        return false;
    };
    with_registry(hwnd, |registry| {
        registry.docs.get(&document).is_some_and(|state| state.live.is_some())
    })
    .unwrap_or(false)
}

/// Brings group `group`'s editor in line with its active document: Live styling on, or Live
/// state dropped. Called after anything that resets styles (language, theme, settings, tab
/// switch), so it is idempotent.
pub(crate) fn sync(hwnd: HWND, group: GroupId) {
    let Some(editor) = host_window::group_editor(hwnd, group) else {
        return;
    };
    let Some((_, _, document, _)) = group_of_editor(hwnd, editor.hwnd()) else {
        return;
    };
    let length = editor.length().unwrap_or(0);
    if !wanted(hwnd, document, length) {
        let had = with_registry(hwnd, |registry| {
            registry.docs.get_mut(&document).and_then(|state| state.live.take()).is_some()
        });
        if had == Some(true) {
            let _ = editor.clear_annotations();
            let _ = editor.show_annotations(false);
        }
        return;
    }
    let needs_model = with_registry(hwnd, |registry| {
        registry.docs.get(&document).is_none_or(|state| state.live.is_none())
    })
    .unwrap_or(false);
    if needs_model {
        let Ok(model) = editor.with_document_text(LiveDocument::parse) else {
            return;
        };
        with_registry(hwnd, |registry| {
            registry.docs.entry(document).or_default().live = Some(LiveState {
                model,
                edits: Vec::new(),
                revealed: BTreeSet::new(),
            });
        });
    }
    let Some((colors, prose, mono)) = (unsafe { host_window::app_ptr(hwnd) }).map(|app| {
        let app = unsafe { app.as_ref() };
        let theme = host_window::effective_theme(hwnd);
        (
            crate::languages::syntax_colors(theme),
            app.settings.preview_font.clone(),
            app.settings.font_face.clone(),
        )
    }) else {
        return;
    };
    let _ = editor.set_lexer(0);
    let _ = apply_style_table(&editor, colors, &prose, &mono);
    // Scintilla restyles everything from here on demand (SCN_STYLENEEDED).
    let _ = editor.apply_styling(0, &[]);
    editor.invalidate();
}

pub(crate) fn sync_all(hwnd: HWND) {
    let groups = unsafe { host_window::app_ptr(hwnd) }
        .map(|app| unsafe { app.as_ref() }.tabs.group_ids())
        .unwrap_or_default();
    for group in groups {
        sync(hwnd, group);
    }
}

pub(crate) fn toggle(hwnd: HWND) {
    if host_window::active_language(hwnd) != Language::Markdown {
        host_window::push_notice(hwnd, NOT_MARKDOWN_NOTICE.to_owned());
        return;
    }
    let live = is_live(hwnd);
    let length = host_window::editor_length(hwnd);
    if !live && length > LIVE_MAX_BYTES {
        host_window::push_notice(hwnd, TOO_LARGE_NOTICE.to_owned());
        return;
    }
    set_live(hwnd, !live);
}

/// Sets the active document's Live flag and restyles every group showing it.
fn set_live(hwnd: HWND, on: bool) {
    let Some(document) = unsafe { host_window::app_ptr(hwnd) }.and_then(|mut app| {
        let doc = unsafe { app.as_mut() }.tabs.active_mut()?;
        doc.live = Some(on);
        Some(doc.id)
    }) else {
        return;
    };
    restyle_document(hwnd, document);
}

fn restyle_document(hwnd: HWND, document: DocumentId) {
    let (active, groups) = match unsafe { host_window::app_ptr(hwnd) } {
        Some(app) => {
            let app = unsafe { app.as_ref() };
            (app.tabs.active_group(), host_window::groups_showing(app, document))
        }
        None => return,
    };
    for group in groups {
        if group == active {
            host_window::apply_language(hwnd, host_window::active_language(hwnd));
        } else {
            host_window::style_group_view(hwnd, group);
        }
        // apply_language skips sync when Lexilla failed to load; sync is idempotent.
        sync(hwnd, group);
    }
}
```
- Make reachable as `host_window::…`: `effective_theme`, `apply_language`, `style_group_view` and `groups_showing`. They are `pub(super)` today; make them `pub(crate)` and re-export them where `push_notice` is.
- Add `pub(crate) fn editor_length(hwnd) -> usize` next to `with_editor`: the active editor's `length()`, or 0.
- `doc.id` is the `Document` field `id` (`document.rs:94`).

Edits and styling:
```rust
/// SCN_MODIFIED for a Live document: O(1) bookkeeping, as the preview records edits.
pub(crate) fn record_edit(hwnd: HWND, document: DocumentId, notification: &ScintillaNotification) {
    let length_over = notification.position.max(0) as usize + notification.length.max(0) as usize;
    let oversized = with_registry(hwnd, |registry| {
        let Some(live) = registry.docs.get_mut(&document).and_then(|s| s.live.as_mut()) else {
            return false;
        };
        live.edits.push(crate::window::preview_host::edit_from(notification));
        length_over > LIVE_MAX_BYTES
    })
    .unwrap_or(false);
    if oversized {
        with_registry(hwnd, |registry| registry.oversized.push(document));
        request(hwnd, None);
    }
}

/// Posts the deferred-work message once; `group` (when given) gets a reveal/reserve pass.
fn request(hwnd: HWND, group: Option<GroupId>) {
    let post = with_registry(hwnd, |registry| {
        if let Some(group) = group {
            if !registry.pending.contains(&group) {
                registry.pending.push(group);
            }
        }
        !std::mem::replace(&mut registry.posted, true)
    });
    if post == Some(true) {
        unsafe { PostMessageW(hwnd, crate::window::messages::WM_FASTPAD_LIVE_DEFERRED, 0, 0) };
    }
}
```
- Rewrite `selection_changed` (Task 11) as `request(hwnd, Some(group))`.
- In `run_deferred`, first handle `oversized`: for each document there, if the document is live and the editor length is over `LIVE_MAX_BYTES`, push `TOO_LARGE_NOTICE`, set `doc.live = Some(false)` through `tabs.document_mut(document)`, and call `restyle_document`.
- The `length_over` check is only a cheap trigger. `run_deferred` checks the real length.

```rust
/// SCN_STYLENEEDED (live mode spec §7.1): apply pending edits to the model, then style from
/// the earliest changed line to `position`.
pub(crate) fn style_needed(hwnd: HWND, group: GroupId, position: usize) {
    let Some(editor) = host_window::group_editor(hwnd, group) else {
        return;
    };
    let Some((_, _, document, _)) = group_of_editor(hwnd, editor.hwnd()) else {
        return;
    };
    let source = ScintillaSource(&editor);
    let changed_start = with_registry(hwnd, |registry| {
        let live = registry.docs.get_mut(&document)?.live.as_mut()?;
        let edits = std::mem::take(&mut live.edits);
        (!edits.is_empty()).then(|| live.model.apply(&source, &edits).start)
    })
    .flatten();
    let Ok(end_styled) = editor.end_styled() else {
        return;
    };
    let from = changed_start.map_or(end_styled, |changed| changed.min(end_styled));
    let start = editor
        .line_from_position(from)
        .and_then(|line| editor.line_start(line))
        .unwrap_or(0);
    let end = position.max(start);
    with_registry(hwnd, |registry| {
        if let Some(live) = registry.docs.get(&document).and_then(|s| s.live.as_ref()) {
            let _ = style_range(&editor, live, start..end);
        }
    });
    request(hwnd, Some(group));
}

/// The byte ranges of `lines`, each with its line ending.
fn line_ranges(editor: &Editor, lines: &BTreeSet<usize>) -> Vec<Range<usize>> {
    lines
        .iter()
        .filter_map(|line| {
            let start = editor.line_start(*line).ok()?;
            let end = editor.line_start(line + 1).ok()?.max(editor.line_end(*line).ok()?);
            Some(start..end)
        })
        .collect()
}

fn style_range(editor: &Editor, live: &LiveState, range: Range<usize>) -> crate::Result<()> {
    let spans = live.model.spans_in(range.clone());
    let revealed = line_ranges(editor, &live.revealed);
    let runs = style_runs(&spans, range.clone(), &revealed);
    editor.apply_styling(range.start, &runs)?;
    editor.set_indicator(STRIKE_INDICATOR, range.clone(), false)?;
    for (block, spans) in live.model.blocks_in(range.clone()) {
        for strike in &spans.strikes {
            let strike = block.start + strike.start..block.start + strike.end;
            editor.set_indicator(STRIKE_INDICATOR, strike, true)?;
        }
    }
    Ok(())
}
```
- `style_range` calls Scintilla while the registry borrow is held. That is safe: styling messages raise no notifications into the window procedure. Do not add calls that do, such as text edits or annotations, inside `with_registry`.
- `ScintillaSource` is a tuple struct over `&Editor` (`preview_host.rs:228`). Make its field `pub(crate)` too.

Wire it up:
- `wndproc.rs` `handle_editor_notification`: right after the group lookup, before the `SCN_FOCUSIN` check:
```rust
  if notification.code == SCN_STYLENEEDED {
      let needed = unsafe { &*(lparam as *const crate::editor::ScintillaNotification) };
      crate::window::live_host::style_needed(hwnd, group, needed.position.max(0) as usize);
      return;
  }
```
- In the `SCN_MODIFIED` text-change branch, next to `preview_host::record_edit`: `crate::window::live_host::record_edit(hwnd, document, modification);`.
- In the `SCN_UPDATEUI` branch, also call `live_host::request` (make it `pub(crate)`) when `SC_UPDATE_V_SCROLL` is set, so reservations follow scrolling (Task 15).
- `apply_language` Ok branch: after `apply_editor_settings(hwnd)`, add `crate::window::live_host::sync(hwnd, active_group)`.
- `style_group_view`: inside the `is_ok()` branch, after `apply_settings_to`, add `crate::window::live_host::sync(hwnd, id)`.
- `apply_editor_settings`: at the end, add `crate::window::live_host::sync_all(hwnd)`.
- `command_dispatch.rs`: add the arm `CommandId::MarkdownToggleLive => crate::window::live_host::toggle(hwnd),`.

- [ ] **Step 7: Run the tests**

Run: `cargo test --lib markdown_live_default`, then `cargo test --test markdown_live -- --test-threads=1`
Expected: all pass. If `switching_tabs_restores_lexilla_and_back` fails because a tab switch does not call `apply_language`, find the tab-activation path (`grep -n "fn activate_tab\|fn show_document" src/window/main_window`) and call `live_host::sync(hwnd, group)` at its end.

- [ ] **Step 8: Run the neighbouring suites**

Run: `cargo test --test highlighting --test markdown_preview -- --test-threads=1`
Expected: all pass. Live is off by default, so nothing changes for them.

- [ ] **Step 9: Commit**

```bash
git add src Cargo.toml tests/windows/markdown_live.rs
git commit -m "feat(live): Live Markdown toggle, setting and container styling"
```

---

### Task 13: Reveal the caret's lines

**Files:**
- Modify: `src/window/live_host.rs` (`reveal`, called from `run_deferred`)
- Test: `tests/windows/markdown_live.rs`

**Interfaces:**
- Consumes: `reveal::{revealed_lines, changed_lines}`, `style_range`, `LiveState.revealed`.
- Produces: `fn reveal(hwnd: HWND, group: GroupId)`.

- [ ] **Step 1: Write the failing tests** (append):

```rust
#[test]
fn the_caret_line_shows_source_and_moving_away_hides_it() {
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    main.make_markdown("x\na **b** c\n");
    main.select(0);
    main.command(CommandId::MarkdownToggleLive);
    main.select(5);
    assert_eq!(main.styled().style_at(4).unwrap(), styles::MARKER);
    main.select(0);
    assert_eq!(main.styled().style_at(4).unwrap(), styles::HIDDEN);
}

#[test]
fn every_caret_of_a_multi_cursor_reveals_its_line() {
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    main.make_markdown("**a**\nx\n**b**\n");
    main.select(6);
    main.command(CommandId::MarkdownToggleLive);
    let editor = main.editor_api();
    editor.set_selections(&[0..0, 8..8]).unwrap();
    pump_pending();
    let editor = main.styled();
    assert_eq!(editor.style_at(0).unwrap(), styles::MARKER);
    assert_eq!(editor.style_at(8).unwrap(), styles::MARKER);
}

#[test]
fn reveal_does_not_dirty_or_add_undo() {
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    main.make_markdown("**a**\nx\n");
    let editor = main.editor_api();
    editor.set_save_point().unwrap();
    main.command(CommandId::MarkdownToggleLive);
    for at in [0, 6, 0, 6] {
        main.select(at);
    }
    assert!(!editor.can_undo().unwrap());
    assert!(!main.with_app(|app| app.tabs.active().unwrap().dirty));
}
```
(`make_markdown` uses `SCI_SETTEXT`, which records undo. If `can_undo` is already true before toggling, call `SCI_EMPTYUNDOBUFFER` first through `SendMessageW`.)

- [ ] **Step 2: Run them and watch them fail**

Run: `cargo test --test markdown_live -- --test-threads=1 caret multi_cursor reveal_does_not`
Expected: the first two fail, because markers stay hidden on the caret line.

- [ ] **Step 3: Implement.** In `run_deferred`, after `format_left_table(hwnd, group)`, call `reveal(hwnd, group);`, then add:

```rust
/// Restyles the lines that entered or left the reveal set (live mode spec §5). Styling only:
/// no undo entry, no modified flag.
fn reveal(hwnd: HWND, group: GroupId) {
    let Some(editor) = host_window::group_editor(hwnd, group) else {
        return;
    };
    let Some((_, _, document, _)) = group_of_editor(hwnd, editor.hwnd()) else {
        return;
    };
    let Ok(selections) = editor.selections() else {
        return;
    };
    let new = crate::live::reveal::revealed_lines(&selections, |at| {
        editor.line_from_position(at).unwrap_or(0)
    });
    with_registry(hwnd, |registry| {
        let Some(live) = registry.docs.get_mut(&document).and_then(|s| s.live.as_mut()) else {
            return;
        };
        let changed = crate::live::reveal::changed_lines(&live.revealed, &new);
        live.revealed = new;
        if changed.is_empty() {
            return;
        }
        let Ok(end_styled) = editor.end_styled() else {
            return;
        };
        for range in line_ranges(&editor, &changed.into_iter().collect()) {
            // Lines Scintilla has not styled yet get their reveal state when it asks.
            if range.start < end_styled {
                let _ = style_range(&editor, live, range.start..range.end.min(end_styled));
            }
        }
    });
    editor.invalidate();
}
```
`style_range` restyles the whole changed lines, including their line endings. `apply_styling` sets Scintilla's styled end back to `range.end`, which is still before the old `end_styled`. To keep everything after it styled, restore the end after styling: call `editor.apply_styling(end_styled, &[])` once after the loop. `StartStyling` with no runs just moves the styled end.

- [ ] **Step 4: Run the tests**

Run: `cargo test --test markdown_live -- --test-threads=1`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add src/window/live_host.rs tests/windows/markdown_live.rs
git commit -m "feat(live): lines holding a caret or selection show source"
```

---

### Task 14: The decoration painter: bullets, checkboxes, quotes, rules, fences, tables

**Files:**
- Create: `src/live/painter.rs`
- Modify: `src/live/mod.rs` (`pub mod painter;`)
- Modify: `src/window/live_host.rs` (`GroupHooks::after_paint`; `GroupHooks` gains `painter: RefCell<Option<Painter>>` and `painted: RefCell<Vec<PaintItem>>`)
- Test: unit tests in `painter.rs`; integration in `tests/windows/markdown_live.rs`

**Interfaces:**
- Consumes:
  - `spans::Decoration`
  - `crate::preview::dwrite::Graphics`, via `preview_host::shared_graphics(hwnd) -> Result<Rc<Graphics>>` (l.568)
  - `crate::preview::render::{RectF, color_f}`
  - `crate::preview::colors::{preview_colors, PreviewColors, ColorRole}`
- Produces:
```rust
#[derive(Clone, Debug, PartialEq)]
pub enum PaintItem {
    Bullet { center: (f32, f32), radius: f32, style: BulletStyle },
    Checkbox { rect: RectF, checked: bool, at: usize },
    Bar { rect: RectF },                  // quote bar, rules, table borders, fence bar
    Label { rect: RectF, text: String },  // fence language
    Heading { rect: RectF, text: String, level: u8 },     // Task 15
    Image { rect: RectF, path: PathBuf }, // Task 16
    Placeholder { rect: RectF, text: String },            // Task 16
}
pub enum BulletStyle { Disc, Circle, Square }
pub struct Geometry<'a> {
    pub point: &'a dyn Fn(usize) -> (f32, f32),   // client x, line top
    pub line_height: f32,
    pub wrap_lines: &'a dyn Fn(usize) -> usize,   // display lines of the line holding a byte
    pub line_end: &'a dyn Fn(usize) -> usize,     // byte end of the line holding a byte
    pub text_right: f32,
}
pub fn layout(decorations: &[(usize, &Decoration)], geometry: &Geometry) -> Vec<PaintItem>;  // (block start, decoration)
pub struct Painter { /* DC render target */ }
impl Painter {
    pub fn new(graphics: Rc<Graphics>) -> Self;
    pub fn paint(&mut self, dc: HDC, client: RECT, clip: RECT, items: &[PaintItem],
                 colors: &PreviewColors, fonts: &PaintFonts,
                 images: Option<&mut ImageCache>) -> crate::Result<()>;   // images: Task 16
}
pub struct PaintFonts { pub prose: String, pub mono: String, pub size_px: f32 }
```

- [ ] **Step 1: Write the failing layout tests** (test module in `painter.rs`):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::spans::Decoration;

    /// A monospace grid: byte b of a line at column b, 10 px per byte, 20 px lines, lines
    /// of 10 bytes each.
    fn geometry_fixture<'a>(
        point: &'a dyn Fn(usize) -> (f32, f32),
        wraps: &'a dyn Fn(usize) -> usize,
        line_end: &'a dyn Fn(usize) -> usize,
    ) -> Geometry<'a> {
        Geometry { point, line_height: 20.0, wrap_lines: wraps, line_end, text_right: 400.0 }
    }

    fn grid(at: usize) -> (f32, f32) {
        ((at % 10) as f32 * 10.0, (at / 10) as f32 * 20.0)
    }

    fn one(_: usize) -> usize {
        1
    }

    fn end_of_line(at: usize) -> usize {
        at / 10 * 10 + 9
    }

    #[test]
    fn bullets_center_on_the_blanked_marker_and_style_by_depth() {
        let geometry = geometry_fixture(&grid, &one, &end_of_line);
        let d1 = Decoration::Bullet { at: 0, depth: 1 };
        let d2 = Decoration::Bullet { at: 2, depth: 2 };
        let items = layout(&[(10, &d1), (10, &d2)], &geometry);
        assert_eq!(
            items[0],
            PaintItem::Bullet { center: (5.0, 30.0), radius: 3.0, style: BulletStyle::Disc }
        );
        assert!(matches!(items[1], PaintItem::Bullet { style: BulletStyle::Circle, .. }));
    }

    #[test]
    fn a_checkbox_is_a_square_over_its_brackets() {
        let geometry = geometry_fixture(&grid, &one, &end_of_line);
        let decoration = Decoration::Checkbox { range: 2..5, checked: true };
        let items = layout(&[(0, &decoration)], &geometry);
        let PaintItem::Checkbox { rect, checked: true, at: 2 } = &items[0] else {
            panic!("{items:?}");
        };
        assert_eq!(rect.height(), 14.0);
        assert_eq!(rect.width(), 14.0);
        assert_eq!((rect.left + rect.right) / 2.0, 35.0);
    }

    #[test]
    fn a_quote_bar_spans_every_wrapped_line() {
        let wraps = |_: usize| 3;
        let geometry = geometry_fixture(&grid, &wraps, &end_of_line);
        let decoration = Decoration::QuoteBar { at: 0, depth: 2 };
        let items = layout(&[(0, &decoration)], &geometry);
        assert_eq!(items.len(), 2);
        let PaintItem::Bar { rect } = &items[1] else { panic!() };
        assert_eq!(rect.top, 0.0);
        assert_eq!(rect.bottom, 60.0);
    }

    #[test]
    fn table_rows_draw_a_border_at_each_pipe_and_below() {
        let geometry = geometry_fixture(&grid, &one, &end_of_line);
        let decoration = Decoration::TableRow { at: 0, pipes: vec![0, 4], header: true };
        let items = layout(&[(0, &decoration)], &geometry);
        // Two verticals, the top border of a header, the bottom border.
        assert_eq!(items.iter().filter(|i| matches!(i, PaintItem::Bar { .. })).count(), 4);
    }

    #[test]
    fn a_rule_runs_to_the_right_edge() {
        let geometry = geometry_fixture(&grid, &one, &end_of_line);
        let decoration = Decoration::Rule { at: 0 };
        let items = layout(&[(20, &decoration)], &geometry);
        let PaintItem::Bar { rect } = &items[0] else { panic!() };
        assert_eq!(rect.right, 400.0);
        assert!(rect.top > 40.0 && rect.bottom < 60.0);
    }
}
```

- [ ] **Step 2: Run them and watch them fail**

Run: `cargo test --lib live::painter`
Expected: compile errors.

- [ ] **Step 3: Implement the layout** (`painter.rs`, top):

```rust
//! Live Markdown's decoration painter (live mode spec §2, §7): after Scintilla paints, draws
//! bullets, checkboxes, quote bars, rules, fences, table grids, headings and images over the
//! blanked text and the annotation lines reserved for them. `layout` is pure geometry; `Painter`
//! draws through a Direct2D DC render target bound to the editor's DC.

use super::spans::Decoration;
use crate::preview::colors::{ColorRole, PreviewColors};
use crate::preview::dwrite::Graphics;
use crate::preview::render::{RectF, color_f};
use std::path::PathBuf;
use std::rc::Rc;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BulletStyle {
    Disc,
    Circle,
    Square,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PaintItem {
    Bullet { center: (f32, f32), radius: f32, style: BulletStyle },
    Checkbox { rect: RectF, checked: bool, at: usize },
    Bar { rect: RectF },
    Label { rect: RectF, text: String },
    Heading { rect: RectF, text: String, level: u8 },
    Image { rect: RectF, path: PathBuf },
    Placeholder { rect: RectF, text: String },
}

pub struct Geometry<'a> {
    pub point: &'a dyn Fn(usize) -> (f32, f32),
    pub line_height: f32,
    pub wrap_lines: &'a dyn Fn(usize) -> usize,
    pub line_end: &'a dyn Fn(usize) -> usize,
    pub text_right: f32,
}

const RULE: f32 = 1.0;
const QUOTE_BAR: f32 = 3.0;

fn bar(left: f32, top: f32, right: f32, bottom: f32) -> PaintItem {
    PaintItem::Bar { rect: RectF::new(left, top, right, bottom) }
}

pub fn layout(decorations: &[(usize, &Decoration)], g: &Geometry) -> Vec<PaintItem> {
    let h = g.line_height;
    let mut items = Vec::new();
    for &(base, decoration) in decorations {
        match decoration {
            Decoration::Bullet { at, depth } => {
                let (x0, y) = (g.point)(base + at);
                let (x1, _) = (g.point)(base + at + 1);
                let style = match depth {
                    1 => BulletStyle::Disc,
                    2 => BulletStyle::Circle,
                    _ => BulletStyle::Square,
                };
                items.push(PaintItem::Bullet {
                    center: ((x0 + x1) / 2.0, y + h / 2.0),
                    radius: (h * 0.15).round(),
                    style,
                });
            }
            Decoration::Checkbox { range, checked } => {
                let (x0, y) = (g.point)(base + range.start);
                let (x1, _) = (g.point)(base + range.end);
                let size = (h * 0.7).min(x1 - x0).round();
                let cx = (x0 + x1) / 2.0;
                let top = y + (h - size) / 2.0;
                items.push(PaintItem::Checkbox {
                    rect: RectF::new(cx - size / 2.0, top, cx + size / 2.0, top + size),
                    checked: *checked,
                    at: base + range.start,
                });
            }
            Decoration::QuoteBar { at, depth } => {
                let (x, y) = (g.point)(base + at);
                let bottom = y + h * (g.wrap_lines)(base + at) as f32;
                let step = (h / 2.0).round();
                for level in 0..*depth {
                    let left = x + f32::from(level) * step;
                    items.push(bar(left, y, left + QUOTE_BAR, bottom));
                }
            }
            Decoration::Rule { at } => {
                let (x, y) = (g.point)(base + at);
                let middle = (y + h / 2.0).round();
                items.push(bar(x, middle, g.text_right, middle + RULE));
            }
            Decoration::Fence { at, language } => {
                let (x, y) = (g.point)(base + at);
                let middle = (y + h / 2.0).round();
                items.push(bar(x, middle, g.text_right, middle + RULE));
                if !language.is_empty() {
                    let width = h * 0.6 * language.chars().count() as f32 + h;
                    items.push(PaintItem::Label {
                        rect: RectF::new(g.text_right - width, y, g.text_right, y + h),
                        text: language.clone(),
                    });
                }
            }
            Decoration::TableRow { at, pipes, header } => {
                let (_, y) = (g.point)(base + at);
                let centers: Vec<f32> = pipes
                    .iter()
                    .map(|pipe| {
                        let (x0, _) = (g.point)(base + pipe);
                        let (x1, _) = (g.point)(base + pipe + 1);
                        ((x0 + x1) / 2.0).round()
                    })
                    .collect();
                let bottom = y + h * (g.wrap_lines)(base + at) as f32;
                for x in &centers {
                    items.push(bar(*x, y, x + RULE, bottom));
                }
                if let (Some(first), Some(last)) = (centers.first(), centers.last()) {
                    if *header {
                        items.push(bar(*first, y, last + RULE, y + RULE));
                    }
                    items.push(bar(*first, bottom - RULE, last + RULE, bottom));
                }
            }
            Decoration::TableDelimiter { at, pipes } => {
                let (_, y) = (g.point)(base + at);
                for pipe in pipes {
                    let (x0, _) = (g.point)(base + pipe);
                    let (x1, _) = (g.point)(base + pipe + 1);
                    let x = ((x0 + x1) / 2.0).round();
                    items.push(bar(x, y, x + RULE, y + h));
                }
            }
            // Headings and images are laid out by `layout_reserved` (Tasks 15, 16).
            Decoration::Heading { .. } | Decoration::Image { .. } => {}
        }
    }
    items
}
```
In `bullets_center_on_the_blanked_marker…`, `radius` is `(20 * 0.15).round() = 3`, the center x is `(0 + 10) / 2 = 5`, and y is `20 + 10 = 30` (base 10 is line 1).

- [ ] **Step 4: Run the layout tests**

Run: `cargo test --lib live::painter`
Expected: 5 passed.

- [ ] **Step 5: Implement the `Painter`** (append to `painter.rs`):

```rust
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D1_ALPHA_MODE_IGNORE, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_DRAW_TEXT_OPTIONS_NONE, D2D1_ELLIPSE, D2D1_RENDER_TARGET_PROPERTIES,
    D2D1_RENDER_TARGET_TYPE_SOFTWARE, ID2D1DCRenderTarget, ID2D1RenderTarget,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT_BOLD, DWRITE_FONT_WEIGHT_NORMAL,
    DWRITE_TEXT_ALIGNMENT_TRAILING,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows_numerics::Vector2;
use windows_sys::Win32::Foundation::RECT;
use windows_sys::Win32::Graphics::Gdi::HDC;

pub struct PaintFonts {
    pub prose: String,
    pub mono: String,
    /// The body text size in pixels at the current zoom and DPI.
    pub size_px: f32,
}

pub struct Painter {
    graphics: Rc<Graphics>,
    target: Option<ID2D1DCRenderTarget>,
}

impl Painter {
    pub fn new(graphics: Rc<Graphics>) -> Self {
        Self { graphics, target: None }
    }

    fn target(&mut self) -> crate::Result<ID2D1DCRenderTarget> {
        if let Some(target) = &self.target {
            return Ok(target.clone());
        }
        // As `window/soft_paint.rs` creates its DC target: software, 96 DPI so one unit is one
        // pixel of Scintilla's client coordinates.
        let properties = D2D1_RENDER_TARGET_PROPERTIES {
            r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_IGNORE,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            ..Default::default()
        };
        let target = unsafe { self.graphics.d2d.CreateDCRenderTarget(&properties) }
            .map_err(crate::preview::dwrite::hresult_error)?;
        self.target = Some(target.clone());
        Ok(target)
    }

    /// Draws `items` clipped to `clip`. Every item fills its own background first, so drawing
    /// an area twice gives the same pixels (Scintilla repaints partial regions).
    pub fn paint(
        &mut self,
        dc: HDC,
        client: RECT,
        clip: RECT,
        items: &[PaintItem],
        colors: &PreviewColors,
        fonts: &PaintFonts,
        images: Option<&mut crate::preview::images::ImageCache>,
    ) -> crate::Result<()> {
        let target = self.target()?;
        let bounds = windows::Win32::Foundation::RECT {
            left: client.left,
            top: client.top,
            right: client.right,
            bottom: client.bottom,
        };
        unsafe { target.BindDC(windows::Win32::Graphics::Gdi::HDC(dc as _), &bounds) }
            .map_err(crate::preview::dwrite::hresult_error)?;
        let base: &ID2D1RenderTarget = &target;
        let brush = |role: ColorRole| unsafe {
            base.CreateSolidColorBrush(&color_f(colors.get(role)), None)
        };
        unsafe {
            base.BeginDraw();
            base.PushAxisAlignedClip(
                &D2D_RECT_F {
                    left: clip.left as f32,
                    top: clip.top as f32,
                    right: clip.right as f32,
                    bottom: clip.bottom as f32,
                },
                windows::Win32::Graphics::Direct2D::D2D1_ANTIALIAS_MODE_ALIASED,
            );
        }
        let result = (|| -> windows::core::Result<()> {
            let background = brush(ColorRole::Background)?;
            let text = brush(ColorRole::Text)?;
            let muted = brush(ColorRole::Muted)?;
            let border = brush(ColorRole::Border)?;
            let quote = brush(ColorRole::QuoteBar)?;
            let link = brush(ColorRole::Link)?;
            let heading = brush(ColorRole::Heading)?;
            let mut images = images;
            for item in items {
                match item {
                    PaintItem::Bullet { center, radius, style } => {
                        let ellipse = D2D1_ELLIPSE {
                            point: Vector2 { X: center.0, Y: center.1 },
                            radiusX: *radius,
                            radiusY: *radius,
                        };
                        unsafe {
                            match style {
                                BulletStyle::Disc => base.FillEllipse(&ellipse, &text),
                                BulletStyle::Circle => base.DrawEllipse(&ellipse, &text, 1.0, None),
                                BulletStyle::Square => base.FillRectangle(
                                    &RectF::new(
                                        center.0 - radius,
                                        center.1 - radius,
                                        center.0 + radius,
                                        center.1 + radius,
                                    )
                                    .to_d2d(),
                                    &text,
                                ),
                            }
                        }
                    }
                    PaintItem::Checkbox { rect, checked, .. } => unsafe {
                        base.FillRectangle(&rect.to_d2d(), &background);
                        if *checked {
                            base.FillRectangle(&rect.to_d2d(), &link);
                        }
                        base.DrawRectangle(&rect.inflate(-0.5).to_d2d(), &border, 1.0, None);
                        if *checked {
                            let (l, t, w, h) = (rect.left, rect.top, rect.width(), rect.height());
                            let a = Vector2 { X: l + w * 0.22, Y: t + h * 0.52 };
                            let b = Vector2 { X: l + w * 0.42, Y: t + h * 0.72 };
                            let c = Vector2 { X: l + w * 0.78, Y: t + h * 0.30 };
                            base.DrawLine(a, b, &background, 2.0, None);
                            base.DrawLine(b, c, &background, 2.0, None);
                        }
                    },
                    PaintItem::Bar { rect } => unsafe {
                        let brush = if rect.width() <= QUOTE_BAR && rect.height() > 2.0 {
                            &quote
                        } else {
                            &border
                        };
                        base.FillRectangle(&rect.to_d2d(), brush);
                    },
                    PaintItem::Label { rect, text: label } => {
                        let format = self
                            .graphics
                            .text_format(&fonts.mono, fonts.size_px * 0.8, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_STYLE_NORMAL)
                            .map_err(|_| windows::core::Error::from_win32())?;
                        unsafe { format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_TRAILING)? };
                        let wide: Vec<u16> = label.encode_utf16().collect();
                        unsafe {
                            base.FillRectangle(&rect.to_d2d(), &background);
                            base.DrawText(&wide, &format, &rect.to_d2d(), &muted, D2D1_DRAW_TEXT_OPTIONS_NONE, Default::default());
                        }
                    }
                    PaintItem::Heading { .. } | PaintItem::Image { .. } | PaintItem::Placeholder { .. } => {
                        draw_reserved(base, &self.graphics, item, fonts, &background, &heading, &border, &muted, images.as_deref_mut())?;
                    }
                }
            }
            Ok(())
        })();
        unsafe {
            base.PopAxisAlignedClip();
        }
        let ended = unsafe { base.EndDraw(None, None) };
        if ended.is_err() {
            self.target = None; // D2DERR_RECREATE_TARGET and friends: rebuild next paint
        }
        result.map_err(crate::preview::dwrite::hresult_error)
    }
}

/// Headings, images and placeholders (Tasks 15, 16). Until then: nothing.
#[allow(clippy::too_many_arguments)]
fn draw_reserved(
    _target: &ID2D1RenderTarget,
    _graphics: &Graphics,
    _item: &PaintItem,
    _fonts: &PaintFonts,
    _background: &windows::Win32::Graphics::Direct2D::ID2D1SolidColorBrush,
    _heading: &windows::Win32::Graphics::Direct2D::ID2D1SolidColorBrush,
    _border: &windows::Win32::Graphics::Direct2D::ID2D1SolidColorBrush,
    _muted: &windows::Win32::Graphics::Direct2D::ID2D1SolidColorBrush,
    _images: Option<&mut crate::preview::images::ImageCache>,
) -> windows::core::Result<()> {
    Ok(())
}
```
- Pass `fonts.prose` / `fonts.mono` through `Graphics::resolve_family` the way `PreviewFonts` does if a GDI weight name (e.g. "Segoe UI Semibold") is configured.
- `text_format` returns `crate::Result`; map its error to `windows::core::Error` as shown, or restructure the closure to return `crate::Result`.
- `RectF::inflate` takes a signed amount. Check its signature in `render.rs` and adapt the call.

- [ ] **Step 6: Implement `after_paint`** in `live_host.rs`. `GroupHooks` gains `painter: RefCell<Option<Painter>>`, `painted: RefCell<Vec<PaintItem>>` and `paints: Cell<u32>`, plus:

```rust
    fn after_paint(&self, hwnd: HWND, update: RECT) {
        let Some((_, editor, document, _)) = group_of_editor(self.main, self.editor) else {
            return;
        };
        let Some(items) = paint_items(self.main, &editor, document) else {
            return; // not Live
        };
        let graphics = match crate::window::preview_host::shared_graphics(self.main) {
            Ok(graphics) => graphics,
            Err(_) => return,
        };
        let mut painter = self.painter.borrow_mut();
        let painter = painter.get_or_insert_with(|| Painter::new(graphics));
        let Some((colors, fonts)) = paint_style(self.main, &editor) else {
            return;
        };
        let mut client = RECT::default();
        unsafe { GetClientRect(hwnd, &mut client) };
        let dc = unsafe { GetDC(hwnd) };
        let _ = painter.paint(dc, client, update, &items, &colors, &fonts, None);
        unsafe { ReleaseDC(hwnd, dc) };
        *self.painted.borrow_mut() = items;
        self.paints.set(self.paints.get() + 1);
    }
```
and module functions:
```rust
/// The decorations of the visible, non-revealed lines, laid out in client pixels.
fn paint_items(hwnd: HWND, editor: &Editor, document: DocumentId) -> Option<Vec<PaintItem>> {
    let first = editor.doc_line_from_visible(editor.first_visible_line().ok()?).ok()?;
    let last = first + editor.lines_on_screen().ok()? + 1;
    let start = editor.line_start(first).ok()?;
    let end = editor.line_start(last).ok()?.max(editor.line_end(last.min(editor.line_count().ok()?)).ok()?);
    let height = editor.text_height().ok()? as f32;
    let mut client = RECT::default();
    unsafe { GetClientRect(editor.hwnd(), &mut client) };
    with_registry(hwnd, |registry| {
        let live = registry.docs.get(&document)?.live.as_ref()?;
        let mut decorations = Vec::new();
        for (block, spans) in live.model.blocks_in(start..end) {
            for decoration in &spans.decorations {
                let at = block.start + decoration_at(decoration);
                let line = editor.line_from_position(at).ok()?;
                if !live.revealed.contains(&line) {
                    decorations.push((block.start, decoration));
                }
            }
        }
        let point = |at: usize| editor.point_of(at).map_or((0.0, 0.0), |(x, y)| (x as f32, y as f32));
        let wraps = |at: usize| {
            editor.line_from_position(at).and_then(|line| editor.wrap_count(line)).unwrap_or(1)
        };
        let line_end = |at: usize| {
            editor.line_from_position(at).and_then(|line| editor.line_end(line)).unwrap_or(at)
        };
        let geometry = Geometry {
            point: &point,
            line_height: height,
            wrap_lines: &wraps,
            line_end: &line_end,
            text_right: (client.right - 8) as f32,
        };
        Some(crate::live::painter::layout(&decorations, &geometry))
    })
    .flatten()
}

fn decoration_at(decoration: &Decoration) -> usize {
    match decoration {
        Decoration::Heading { at, .. }
        | Decoration::Bullet { at, .. }
        | Decoration::QuoteBar { at, .. }
        | Decoration::Rule { at }
        | Decoration::Fence { at, .. }
        | Decoration::TableRow { at, .. }
        | Decoration::TableDelimiter { at, .. }
        | Decoration::Image { at, .. } => *at,
        Decoration::Checkbox { range, .. } => range.start,
    }
}

fn paint_style(hwnd: HWND, editor: &Editor) -> Option<(PreviewColors, PaintFonts)> {
    let app = unsafe { host_window::app_ptr(hwnd)?.as_ref() };
    let theme = host_window::effective_theme(hwnd);
    let colors = crate::preview::colors::preview_colors(theme, host_window::high_contrast(hwnd));
    let dpi = unsafe { GetDpiForWindow(editor.hwnd()) } as f32;
    let zoom = editor.zoom().unwrap_or(0) as f32;
    let size_px = (f32::from(app.settings.font_size) + zoom) * dpi / 72.0;
    Some((
        colors,
        PaintFonts {
            prose: app.settings.preview_font.clone(),
            mono: app.settings.font_face.clone(),
            size_px,
        },
    ))
}
```
- `host_window::high_contrast(hwnd)`: use whatever the preview passes as `high_contrast` to `preview_colors`. Find the call with `grep -n "preview_colors(" src/window`.
- `editor.zoom()` exists (`group_layout.rs:349` calls it).
- Imports: `GetClientRect`, `GetDC`, `ReleaseDC` (Gdi or WindowsAndMessaging, as `windows_sys` places them); `GetDpiForWindow` (HiDpi).
- `painted` and `paints` are read by the integration tests through:
```rust
#[cfg(test)]
pub(crate) fn painted(hwnd: HWND, editor: HWND) -> Vec<PaintItem> { /* find the group's hooks */ }
```
  To reach a `GroupHooks` from outside, keep a `Weak<GroupHooks>` per editor HWND in `LiveRegistry` (`hooks: HashMap<isize, Weak<GroupHooks>>`), filled where the hooks are installed (Task 11).
  - The install site keeps an `Rc<GroupHooks>`, stores its `Rc::downgrade` in the registry, then passes it to `set_hooks`.
  - `painted(hwnd, editor)` upgrades the `Weak` and clones `painted`.

- [ ] **Step 7: Integration test** (append to `tests/windows/markdown_live.rs`):

```rust
fn paint_now(main: &TestMain) -> Vec<crate::live::painter::PaintItem> {
    unsafe {
        windows_sys::Win32::Graphics::Gdi::InvalidateRect(main.editor, std::ptr::null(), 0);
        windows_sys::Win32::Graphics::Gdi::UpdateWindow(main.editor);
    }
    pump_pending();
    crate::window::live_host::painted(main.hwnd, main.editor)
}

#[test]
fn live_paints_bullets_checkboxes_quotes_rules_fences_and_tables() {
    use crate::live::painter::PaintItem;
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    main.make_markdown(
        "x\n\n- a\n- [ ] b\n\n> q\n\n---\n\n```rs\nc\n```\n\n|a|b|\n|-|-|\n|c|d|\n",
    );
    main.select(0);
    main.command(CommandId::MarkdownToggleLive);
    let items = paint_now(&main);
    assert!(items.iter().any(|i| matches!(i, PaintItem::Bullet { .. })));
    assert!(items.iter().any(|i| matches!(i, PaintItem::Checkbox { checked: false, .. })));
    assert!(items.iter().any(|i| matches!(i, PaintItem::Label { text, .. } if text == "rs")));
    assert!(items.iter().filter(|i| matches!(i, PaintItem::Bar { .. })).count() >= 8);
}

#[test]
fn a_revealed_line_paints_nothing() {
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    main.make_markdown("- a\n");
    main.select(0);
    main.command(CommandId::MarkdownToggleLive);
    assert!(paint_now(&main).is_empty());
}
```
The `WindowHarness`/`TestMain` window must be visible for `WM_PAINT` to arrive. If `TestMain` creates a hidden window, call `ShowWindow(main.hwnd, SW_SHOWNA)` in `paint_now` first.

- [ ] **Step 8: Run the tests**

Run: `cargo test --lib live::painter`, then `cargo test --test markdown_live -- --test-threads=1`, then `cargo test --test markdown_preview -- --test-threads=1 binary_does_not_statically_import`
Expected: all pass. Direct2D still loads only on first use.

- [ ] **Step 9: Commit**

```bash
git add src/live src/window tests/windows/markdown_live.rs
git commit -m "feat(live): paint bullets, checkboxes, quote bars, rules, fences and table grids"
```

---

### Task 15: Large headings in reserved lines

**Files:**
- Create: `src/live/reserve.rs`
- Modify: `src/live/mod.rs`, `src/live/painter.rs` (`layout_reserved`, `draw_reserved` for headings), `src/window/live_host.rs` (`reserve` in `run_deferred`)
- Test: unit tests in `reserve.rs`; integration in `tests/windows/markdown_live.rs`

**Interfaces:**
- Produces:
```rust
// reserve.rs
pub fn heading_scale(level: u8) -> Option<f32>;  // 1 → 1.8, 2 → 1.5, 3 → 1.25, else None
pub fn extra_lines(content_height: f32, line_height: f32, wrap_lines: usize) -> usize;
// painter.rs
/// (byte of the decorated line, annotation lines it needs, item to paint)
pub fn layout_reserved(decorations: &[(usize, &Decoration)], g: &Geometry,
    measure_heading: &dyn Fn(&str, u8) -> f32, image: &dyn Fn(&str) -> ReservedImage)
    -> Vec<(usize, usize, PaintItem)>;
pub enum ReservedImage { Sized { path: PathBuf, width: f32, height: f32 }, Placeholder }
// live_host.rs
fn reserve(hwnd: HWND, group: GroupId);
```

- [ ] **Step 1: Write the failing tests** (`reserve.rs`):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heading_scales() {
        assert_eq!(heading_scale(1), Some(1.8));
        assert_eq!(heading_scale(2), Some(1.5));
        assert_eq!(heading_scale(3), Some(1.25));
        assert_eq!(heading_scale(4), None);
    }

    #[test]
    fn extra_lines_round_up_beyond_the_lines_already_shown() {
        assert_eq!(extra_lines(36.0, 20.0, 1), 1);
        assert_eq!(extra_lines(20.0, 20.0, 1), 0);
        assert_eq!(extra_lines(61.0, 20.0, 2), 2);
        assert_eq!(extra_lines(10.0, 20.0, 3), 0);
    }
}
```
And in `tests/windows/markdown_live.rs`:
```rust
#[test]
fn an_h1_reserves_lines_until_the_caret_reaches_it() {
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    main.make_markdown("x\n# Title\nbody\n");
    main.select(0);
    main.command(CommandId::MarkdownToggleLive);
    paint_now(&main);
    let editor = main.editor_api();
    assert!(editor.annotation_lines(1).unwrap() >= 1, "H1 reserves height");
    main.select(3);
    paint_now(&main);
    assert_eq!(editor.annotation_lines(1).unwrap(), 0, "revealed headings are normal size");
    main.select(0);
    paint_now(&main);
    assert!(editor.annotation_lines(1).unwrap() >= 1);
}

#[test]
fn deleting_the_hash_drops_the_reservation() {
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    main.make_markdown("x\n# Title\nbody\n");
    main.select(0);
    main.command(CommandId::MarkdownToggleLive);
    paint_now(&main);
    let editor = main.editor_api();
    editor.replace_target(2..4, "").unwrap();
    main.select(0);
    paint_now(&main);
    assert_eq!(editor.annotation_lines(1).unwrap(), 0);
}

#[test]
fn h4_reserves_nothing() {
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    main.make_markdown("x\n#### Small\n");
    main.select(0);
    main.command(CommandId::MarkdownToggleLive);
    paint_now(&main);
    assert_eq!(main.editor_api().annotation_lines(1).unwrap(), 0);
}
```

- [ ] **Step 2: Run them and watch them fail**

Run: `cargo test --lib live::reserve`, then `cargo test --test markdown_live -- --test-threads=1 reserv h4`
Expected: compile errors, then the integration tests fail because no annotations are set.

- [ ] **Step 3: Implement `reserve.rs`:**

```rust
//! How much height a rendered heading or image needs (live mode spec §2): whole display lines,
//! reserved as annotation lines under the line, beyond the lines it already wraps to.

pub fn heading_scale(level: u8) -> Option<f32> {
    match level {
        1 => Some(1.8),
        2 => Some(1.5),
        3 => Some(1.25),
        _ => None,
    }
}

pub fn extra_lines(content_height: f32, line_height: f32, wrap_lines: usize) -> usize {
    ((content_height / line_height).ceil() as usize).saturating_sub(wrap_lines)
}
```

- [ ] **Step 4: Implement heading layout and drawing** in `painter.rs`:

```rust
pub enum ReservedImage {
    Sized { path: PathBuf, width: f32, height: f32 },
    Placeholder,
}

const HEADING_RULE_GAP: f32 = 4.0;

/// Headings and images: the item and how many annotation lines it needs under its line.
pub fn layout_reserved(
    decorations: &[(usize, &Decoration)],
    g: &Geometry,
    measure_heading: &dyn Fn(&str, u8) -> f32,
    image: &dyn Fn(&str) -> ReservedImage,
) -> Vec<(usize, usize, PaintItem)> {
    let h = g.line_height;
    let mut out = Vec::new();
    for &(base, decoration) in decorations {
        match decoration {
            Decoration::Heading { at, level, text } => {
                if super::reserve::heading_scale(*level).is_none() {
                    continue;
                }
                let at = base + at;
                let (x, y) = (g.point)(at);
                let wraps = (g.wrap_lines)(at);
                let content = measure_heading(text, *level)
                    + if *level <= 2 { HEADING_RULE_GAP + RULE } else { 0.0 };
                let extra = super::reserve::extra_lines(content, h, wraps);
                let bottom = y + h * (wraps + extra) as f32;
                out.push((
                    at,
                    extra,
                    PaintItem::Heading {
                        rect: RectF::new(x, y, g.text_right, bottom),
                        text: text.clone(),
                        level: *level,
                    },
                ));
            }
            Decoration::Image { at, dest, alt } => {
                let at = base + at;
                let (x, y) = (g.point)(at);
                let top = y + h * (g.wrap_lines)(at) as f32;
                match image(dest) {
                    ReservedImage::Sized { path, width, height } => {
                        let extra = (height / h).ceil() as usize;
                        out.push((
                            at,
                            extra,
                            PaintItem::Image { rect: RectF::new(x, top, x + width, top + height), path },
                        ));
                    }
                    ReservedImage::Placeholder => out.push((
                        at,
                        1,
                        PaintItem::Placeholder {
                            rect: RectF::new(x, top, g.text_right.min(x + 320.0), top + h),
                            text: if alt.is_empty() { dest.clone() } else { alt.clone() },
                        },
                    )),
                }
            }
            _ => {}
        }
    }
    out
}
```
Fill in `draw_reserved` for `PaintItem::Heading`:
```rust
        PaintItem::Heading { rect, text, level } => {
            let scale = super::reserve::heading_scale(*level).unwrap_or(1.0);
            let format = graphics
                .text_format(&fonts.prose, fonts.size_px * scale, DWRITE_FONT_WEIGHT_BOLD, DWRITE_FONT_STYLE_NORMAL)
                .map_err(|_| windows::core::Error::from_win32())?;
            let wide: Vec<u16> = text.encode_utf16().collect();
            let layout = unsafe {
                graphics.dwrite.CreateTextLayout(&wide, &format, rect.width().max(1.0), rect.height().max(1.0))?
            };
            unsafe {
                target.FillRectangle(&rect.to_d2d(), background);
                target.DrawTextLayout(
                    Vector2 { X: rect.left, Y: rect.top },
                    &layout,
                    heading,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                );
                if *level <= 2 {
                    let line = RectF::new(rect.left, rect.bottom - RULE - 1.0, rect.right, rect.bottom - 1.0);
                    target.FillRectangle(&line.to_d2d(), border);
                }
            }
        }
```
In `paint_items` (live_host), also run `layout_reserved` and append its items:
- **`measure_heading`:** build a DirectWrite layout with the scaled prose font at width `text_right - x`, and return `GetMetrics().height`. Use `Graphics` from `shared_graphics`.
- **`image`:** return `ReservedImage::Placeholder` for now. Task 16 fills it in.

- [ ] **Step 5: Implement `reserve`** in `live_host.rs` and call it in `run_deferred` after `reveal`:

```rust
/// Sets each visible line's annotation lines to what its heading or image needs (0 for other
/// lines and revealed ones). Runs from the deferred message: changing line heights inside
/// Scintilla's paint or notifications is not safe.
fn reserve(hwnd: HWND, group: GroupId) {
    let Some(editor) = host_window::group_editor(hwnd, group) else {
        return;
    };
    let Some((_, _, document, _)) = group_of_editor(hwnd, editor.hwnd()) else {
        return;
    };
    let Some(wanted) = reserved_lines(hwnd, &editor, document) else {
        return;
    };
    let mut changed = false;
    for (line, count) in wanted {
        if editor.annotation_lines(line).unwrap_or(0) != count {
            let _ = editor.set_annotation_lines(line, count, u32::from(crate::live::styles::ANNOTATION));
            changed = true;
        }
    }
    if changed {
        editor.invalidate();
    }
}
```
- `reserved_lines(hwnd, editor, document) -> Option<Vec<(usize, usize)>>` covers every line in the visible range plus the revealed lines.
- It returns the `extra` from `layout_reserved` for non-revealed heading and image lines, and 0 for every other line in the range.
- Compute the `layout_reserved` items once and share them with `paint_items`: factor out `fn reserved_items(hwnd, editor, document) -> Option<Vec<(usize, usize, PaintItem)>>`. Build the range and geometry in one helper used by both, so they never disagree.

Call `request(hwnd, Some(group))` from these places too, so reservations follow the view:
- `sync` (after turning Live on)
- the `SCN_UPDATEUI` `V_SCROLL` path (Task 12)
- `WM_SIZE` of a group editor
- zoom (`SCN_ZOOM`)

- [ ] **Step 6: Run the tests**

Run: `cargo test --lib live::reserve live::painter`, then `cargo test --test markdown_live -- --test-threads=1`
Expected: all pass.

- [ ] **Step 7: Commit**

```bash
git add src/live src/window tests/windows/markdown_live.rs
git commit -m "feat(live): large headings drawn into reserved annotation lines"
```

---

### Task 16: Image thumbnails

**Files:**
- Modify: `src/window/live_host.rs` (`LiveRegistry.images: Option<ImageCache>`; `image` closure; `image_decoded`)
- Modify: `src/live/painter.rs` (`draw_reserved` for `Image` and `Placeholder`)
- Modify: `src/window/messages.rs` (`WM_FASTPAD_LIVE_IMAGE` = the next free `WM_APP + n`), `wndproc.rs` (arm)
- Test: `tests/windows/markdown_live.rs`

**Interfaces:**
- Consumes:
  - `preview::images::ImageCache::{new, request, drain, size, is_failed, bitmap}`
  - `preview::links::resolve_image_path(dest, document_dir)`
  - `preview_host::active_document(hwnd)` for the folder (l.323)
- Produces: `pub(crate) fn image_decoded(hwnd: HWND)`.

- [ ] **Step 1: Write the failing tests:**

```rust
/// A fresh folder holding `note.md` with `text` and a 40×100 SVG `p.svg`, opened as Markdown.
fn open_note_with_image(main: &TestMain, name: &str, text: &str) {
    let dir = std::env::temp_dir().join(format!("fastpad-live-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("p.svg"),
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="100"><rect width="40" height="100" fill="red"/></svg>"#,
    )
    .unwrap();
    std::fs::write(dir.join("note.md"), text).unwrap();
    window::open_path(main.hwnd, &dir.join("note.md")).unwrap();
    pump_until("note loaded", Duration::from_secs(3), || {
        !main.with_app(|app| app.populating_file)
    });
    main.with_app(|app| app.tabs.set_active_language(Language::Markdown));
}

#[test]
fn a_local_image_reserves_its_height_and_paints() {
    use crate::live::painter::PaintItem;
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    open_note_with_image(&main, "image", "x\n![a](p.svg)\n");
    main.select(0);
    main.command(CommandId::MarkdownToggleLive);
    pump_until("image decoded", Duration::from_secs(3), || {
        paint_now(&main).iter().any(|i| matches!(i, PaintItem::Image { .. }))
    });
    let line_height = main.editor_api().text_height().unwrap() as f32;
    let expected = (100.0 / line_height).ceil() as usize;
    assert_eq!(main.editor_api().annotation_lines(1).unwrap(), expected);
}

#[test]
fn a_missing_image_reserves_one_placeholder_line() {
    use crate::live::painter::PaintItem;
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    main.make_markdown("x\n![gone](missing.png)\n");
    main.select(0);
    main.command(CommandId::MarkdownToggleLive);
    let items = paint_now(&main);
    pump_pending();
    assert!(items.iter().any(|i| matches!(i, PaintItem::Placeholder { text, .. } if text == "gone")));
    assert_eq!(main.editor_api().annotation_lines(1).unwrap(), 1);
}
```
`open_note_with_image` follows `relative_markdown_links_open_in_a_tab` in `markdown_preview.rs`, l.418. It writes the files, opens them with `window::open_path` so relative paths resolve against the folder, and waits for `populating_file` to clear. The image is an SVG: `decode_image` sends `.svg` to `svg::decode_svg`, which takes its natural size from `width`/`height`.

- [ ] **Step 2: Run them and watch them fail**

Run: `cargo test --test markdown_live -- --test-threads=1 image`
Expected: both fail. Placeholders are already reserved by Task 15's stub closure, so the missing-image test may already pass. The local-image test fails.

- [ ] **Step 3: Implement**

`messages.rs`: `pub const WM_FASTPAD_LIVE_IMAGE: u32 = WM_APP + 0x5C;` (the next free value).

`live_host.rs`: add `images: Option<ImageCache>` to `LiveRegistry`. The `image` closure used by `reserved_items`:
```rust
        let folder = crate::window::preview_host::active_document(hwnd).and_then(|(_, _, folder)| folder);
        let text_width = (geometry.text_right - left_x).max(32.0);
        let scale = dpi / 96.0;
        let image = |dest: &str| -> ReservedImage {
            let Some(path) = crate::preview::links::resolve_image_path(dest, folder.as_deref()) else {
                return ReservedImage::Placeholder;
            };
            with_registry(hwnd, |registry| {
                let cache = registry.images.get_or_insert_with(|| {
                    ImageCache::new(hwnd, crate::window::messages::WM_FASTPAD_LIVE_IMAGE)
                });
                cache.request(&path, text_width as u32);
                if cache.is_failed(&path) {
                    return ReservedImage::Placeholder;
                }
                match cache.size(&path) {
                    Some((width, height)) => {
                        let (width, height) = (width as f32, height as f32);
                        let max_height = 300.0 * scale;
                        let fit = (max_height / height).min(text_width / width).min(1.0);
                        ReservedImage::Sized { path, width: width * fit, height: height * fit }
                    }
                    None => ReservedImage::Placeholder, // still decoding: one line until it lands
                }
            })
            .unwrap_or(ReservedImage::Placeholder)
        };
```
- `left_x` is the text's left edge: `point_of(line_start(first)).0`.
- `dpi` comes from `GetDpiForWindow`.
- `ImageCache::request` with a missing file marks it failed immediately (`images.rs:213`), which gives the placeholder.

```rust
/// A Live thumbnail finished decoding: re-reserve and repaint every Live group.
pub(crate) fn image_decoded(hwnd: HWND) {
    let changed = with_registry(hwnd, |registry| registry.images.as_mut().is_some_and(ImageCache::drain));
    if changed != Some(true) {
        return;
    }
    let groups = unsafe { host_window::app_ptr(hwnd) }
        .map(|app| unsafe { app.as_ref() }.tabs.group_ids())
        .unwrap_or_default();
    for group in groups {
        request(hwnd, Some(group));
        if let Some(editor) = host_window::group_editor(hwnd, group) {
            editor.invalidate();
        }
    }
}
```
`wndproc.rs`: add the arm `WM_FASTPAD_LIVE_IMAGE => { crate::window::live_host::image_decoded(hwnd); 0 }`.

`after_paint` passes the cache: take it out of the registry for the duration of the paint, then put it back:
```rust
        let mut images = with_registry(self.main, |registry| registry.images.take()).flatten();
        let _ = painter.paint(dc, client, update, &items, &colors, &fonts, images.as_mut());
        with_registry(self.main, |registry| registry.images = images);
```
`draw_reserved`:
```rust
        PaintItem::Image { rect, path } => {
            if let Some(cache) = images {
                if let Some(bitmap) = cache.bitmap(target, path) {
                    unsafe {
                        target.FillRectangle(&rect.to_d2d(), background);
                        target.DrawBitmap(
                            &bitmap,
                            Some(&rect.to_d2d()),
                            1.0,
                            windows::Win32::Graphics::Direct2D::D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                            None,
                        );
                    }
                }
            }
        }
        PaintItem::Placeholder { rect, text } => {
            let format = graphics
                .text_format(&fonts.prose, fonts.size_px * 0.9, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_STYLE_NORMAL)
                .map_err(|_| windows::core::Error::from_win32())?;
            let label: Vec<u16> = format!("\u{1F5BC} {text}").encode_utf16().collect();
            unsafe {
                target.FillRectangle(&rect.to_d2d(), background);
                target.DrawRectangle(&rect.inflate(-0.5).to_d2d(), border, 1.0, None);
                target.DrawText(
                    &label,
                    &format,
                    &rect.inflate(-4.0).to_d2d(),
                    muted,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                    Default::default(),
                );
            }
        }
```
`ImageCache::bitmap` creates bitmaps for one target. When `Painter` drops its target after a device error, call `cache.release_bitmaps()` on the next paint. Track this with a `recreated` flag on `Painter`.

- [ ] **Step 4: Run the tests**

Run: `cargo test --test markdown_live -- --test-threads=1`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add src tests/windows
git commit -m "feat(live): local image thumbnails and placeholders"
```

---

### Task 17: Checkbox clicks, Ctrl+Click links, hand cursor

**Files:**
- Modify: `src/window/live_host.rs` (`GroupHooks::{mouse_down, set_cursor}`; `jump_to_anchor`)
- Modify: `src/window/preview_host.rs` (extract the body of `follow` (l.1073-1101) into `pub(crate) fn open_link(hwnd: HWND, dest: &str, anchor: impl FnOnce(&str) -> bool)`; `follow` calls it with the preview's `scroll_to_anchor`)
- Test: `tests/windows/markdown_live.rs`

**Interfaces:**
- Consumes:
  - `GroupHooks.painted` (the checkbox rects and their `at`)
  - `LiveDocument::blocks_in(..).links`
  - `preview::links::SlugSet`
- Produces: `pub(crate) fn open_link(...)` in preview_host, and `fn jump_to_anchor(hwnd, group, anchor) -> bool` in live_host.

- [ ] **Step 1: Write the failing tests:**

```rust
fn click(main: &TestMain, x: f32, y: f32, ctrl: bool) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MK_CONTROL, MK_LBUTTON, WM_LBUTTONDOWN, WM_LBUTTONUP};
    let lparam = ((y as isize) << 16) | (x as isize & 0xFFFF);
    let keys = MK_LBUTTON as usize | if ctrl { MK_CONTROL as usize } else { 0 };
    unsafe {
        SendMessageW(main.editor, WM_LBUTTONDOWN, keys, lparam);
        SendMessageW(main.editor, WM_LBUTTONUP, keys & !(MK_LBUTTON as usize), lparam);
    }
    pump_pending();
}

#[test]
fn checkbox_toggle_undoes_in_one_step() {
    use crate::live::painter::PaintItem;
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    main.make_markdown("x\n- [ ] task\n");
    main.select(0);
    main.command(CommandId::MarkdownToggleLive);
    let items = paint_now(&main);
    let rect = items
        .iter()
        .find_map(|item| match item {
            PaintItem::Checkbox { rect, .. } => Some(*rect),
            _ => None,
        })
        .unwrap();
    let editor = main.editor_api();
    let before_caret = editor.selection().unwrap();
    click(&main, (rect.left + rect.right) / 2.0, (rect.top + rect.bottom) / 2.0, false);
    assert_eq!(editor.text().unwrap(), "x\n- [x] task\n");
    assert_eq!(editor.selection().unwrap(), before_caret, "the caret stays put");
    editor.undo().unwrap();
    assert_eq!(editor.text().unwrap(), "x\n- [ ] task\n");
}

#[test]
fn ctrl_click_on_an_anchor_link_moves_to_the_heading() {
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    let text = "x\n[go](#far-away)\n\na\n\nb\n\n## Far away\n";
    main.make_markdown(text);
    main.select(0);
    main.command(CommandId::MarkdownToggleLive);
    paint_now(&main);
    let editor = main.editor_api();
    let (x, y) = editor.point_of(text.find("go").unwrap()).unwrap();
    click(&main, x as f32 + 2.0, y as f32 + 2.0, true);
    let heading_line = 7;
    assert_eq!(editor.line_from_position(editor.selection().unwrap().end).unwrap(), heading_line);
}

#[test]
fn a_plain_click_on_a_link_only_places_the_caret() {
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    let text = "x\n[go](#nowhere)\n";
    main.make_markdown(text);
    main.select(0);
    main.command(CommandId::MarkdownToggleLive);
    paint_now(&main);
    let editor = main.editor_api();
    let (x, y) = editor.point_of(text.find("go").unwrap()).unwrap();
    click(&main, x as f32 + 2.0, y as f32 + 2.0, false);
    assert_eq!(editor.line_from_position(editor.selection().unwrap().end).unwrap(), 1);
    assert!(main.notices().is_empty());
}
```

- [ ] **Step 2: Run them and watch them fail**

Run: `cargo test --test markdown_live -- --test-threads=1 checkbox ctrl_click plain_click`
Expected: the first two fail. The click goes to Scintilla, so it places the caret and does nothing else.

- [ ] **Step 3: Implement** in `GroupHooks`:

```rust
    fn mouse_down(&self, x: i32, y: i32, ctrl: bool) -> bool {
        let (x, y) = (x as f32, y as f32);
        let checkbox = self.painted.borrow().iter().find_map(|item| match item {
            PaintItem::Checkbox { rect, checked, at } if rect.contains(x, y) => Some((*at, *checked)),
            _ => None,
        });
        if let Some((at, checked)) = checkbox {
            let Some((_, editor, _, _)) = group_of_editor(self.main, self.editor) else {
                return false;
            };
            // "[ ]" ↔ "[x]": one replace is one undo step; the caret does not move.
            let mark = if checked { " " } else { "x" };
            return editor.replace_target(at + 1..at + 2, mark).is_ok();
        }
        if ctrl {
            return follow_link_at(self.main, self.editor, x as i32, y as i32);
        }
        false
    }

    fn set_cursor(&self, x: i32, y: i32, ctrl: bool) -> bool {
        let (fx, fy) = (x as f32, y as f32);
        let over_box = self.painted.borrow().iter().any(|item| {
            matches!(item, PaintItem::Checkbox { rect, .. } if rect.contains(fx, fy))
        });
        if over_box || (ctrl && link_at(self.main, self.editor, x, y).is_some()) {
            unsafe { SetCursor(LoadCursorW(std::ptr::null_mut(), IDC_HAND)) };
            return true;
        }
        false
    }
```
and:
```rust
/// The destination of the rendered link under client point (x, y), if any.
fn link_at(hwnd: HWND, editor_hwnd: HWND, x: i32, y: i32) -> Option<String> {
    let (_, editor, document, _) = group_of_editor(hwnd, editor_hwnd)?;
    let at = editor.position_at(x, y).ok()?;
    let line = editor.line_from_position(at).ok()?;
    with_registry(hwnd, |registry| {
        let live = registry.docs.get(&document)?.live.as_ref()?;
        if live.revealed.contains(&line) {
            return None;
        }
        live.model.blocks_in(at..at + 1).find_map(|(block, spans)| {
            spans.links.iter().find_map(|link| {
                let range = block.start + link.range.start..block.start + link.range.end;
                range.contains(&at).then(|| link.dest.clone())
            })
        })
    })
    .flatten()
}

fn follow_link_at(hwnd: HWND, editor_hwnd: HWND, x: i32, y: i32) -> bool {
    let Some(dest) = link_at(hwnd, editor_hwnd, x, y) else {
        return false;
    };
    let Some((group, ..)) = group_of_editor(hwnd, editor_hwnd) else {
        return false;
    };
    crate::window::preview_host::open_link(hwnd, &dest, |anchor| jump_to_anchor(hwnd, group, anchor));
    true
}

/// Moves the caret to the heading whose GitHub slug is `anchor` (as the preview's anchors).
fn jump_to_anchor(hwnd: HWND, group: GroupId, anchor: &str) -> bool {
    let Some(editor) = host_window::group_editor(hwnd, group) else {
        return false;
    };
    let Some((_, _, document, _)) = group_of_editor(hwnd, editor.hwnd()) else {
        return false;
    };
    let length = editor.length().unwrap_or(0);
    let target = with_registry(hwnd, |registry| {
        let live = registry.docs.get(&document)?.live.as_ref()?;
        let mut slugs = crate::preview::links::SlugSet::default();
        live.model.blocks_in(0..length).find_map(|(block, spans)| {
            spans.decorations.iter().find_map(|decoration| match decoration {
                Decoration::Heading { at, text, .. } if slugs.unique(text) == anchor => {
                    Some(block.start + at)
                }
                _ => None,
            })
        })
    })
    .flatten();
    let Some(target) = target else {
        return false;
    };
    let _ = editor.set_selection(target..target);
    let _ = editor.scroll_caret_into_view();
    true
}
```
- `SlugSet` must be constructible. Use `SlugSet::default()` or its `new()`, whichever `links.rs` offers.
- `open_link` notices ("FastPad does not open this kind of link…", anchor not found) behave exactly as in the preview.
- Imports: `SetCursor`, `LoadCursorW`, `IDC_HAND` (WindowsAndMessaging).

- [ ] **Step 4: Run the tests**

Run: `cargo test --test markdown_live --test markdown_preview -- --test-threads=1`
Expected: all pass. The preview's link tests prove the `open_link` extraction kept behaviour.

- [ ] **Step 5: Commit**

```bash
git add src/window tests/windows/markdown_live.rs
git commit -m "feat(live): click checkboxes, Ctrl+Click links, hand cursor"
```

---

### Task 18: Live with Split, typing latency, docs, final verification

**Files:**
- Test: `tests/windows/markdown_live.rs`
- Modify: `README.md` (Markdown section, l.84-96 and l.281-289)

- [ ] **Step 1: Write the scroll-sync and perf tests:**

```rust
fn long_markdown_with_headings() -> String {
    let mut text = String::new();
    for index in 0..400 {
        if index % 50 == 0 {
            text.push_str(&format!("# Heading {index}\n\n"));
        } else {
            text.push_str(&format!("Paragraph {index}\n\n"));
        }
    }
    text
}

#[test]
fn split_scroll_sync_stays_aligned_past_reserved_lines() {
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    main.make_markdown(&long_markdown_with_headings());
    main.select(0);
    main.command(CommandId::MarkdownToggleLive);
    main.command(CommandId::MarkdownPreviewSide);
    let view = main.with_app(|app| app.active_group().unwrap().preview.view).unwrap();
    pump_until("render", Duration::from_secs(3), || view.stats().block_count > 0);
    let editor = main.editor_api();
    let line = 600;
    let visible = editor.visible_from_doc_line(line).unwrap();
    editor.set_first_visible_line(visible).unwrap();
    unsafe { windows_sys::Win32::Graphics::Gdi::UpdateWindow(main.editor) };
    pump_until("preview follows", Duration::from_secs(3), || {
        let top = view.top_line();
        top + 20 >= line && top <= line + 20
    });
}

fn p95(mut samples: Vec<u64>) -> u64 {
    samples.sort_unstable();
    samples[samples.len() * 95 / 100]
}

#[test]
#[ignore = "performance measurement: cargo test --release --test markdown_live -- --ignored --test-threads=1"]
fn typing_with_live_on_costs_the_same_as_off() {
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    main.make_markdown(&long_markdown_with_headings());
    let measure = |main: &TestMain| {
        let mut samples = Vec::new();
        type_text(main.editor, "\n\n");
        for index in 0..200 {
            if index % 40 == 0 {
                type_text(main.editor, "\n");
            }
            let started = Instant::now();
            type_text(main.editor, "x");
            unsafe { windows_sys::Win32::Graphics::Gdi::UpdateWindow(main.editor) };
            samples.push(started.elapsed().as_micros() as u64);
        }
        p95(samples)
    };
    let baseline = measure(&main);
    main.command(CommandId::MarkdownToggleLive);
    pump_for(Duration::from_millis(300));
    let live = measure(&main);
    println!("keystroke p95: off {baseline} us, live {live} us");
    assert!(live <= baseline + baseline / 10 + 100);
}
```
(Use the exact method names `set_first_visible_line` and `visible_from_doc_line` that `document_text.rs` exports. Those are the names in its method list.)

- [ ] **Step 2: Run them**

Run: `cargo test --test markdown_live -- --test-threads=1 split_scroll`, then `cargo test --release --test markdown_live -- --ignored --test-threads=1`
Expected: the scroll test passes and the perf assertion holds.
- If the perf test fails, profile `style_needed`. The likely cost is `line_ranges` or `spans_in` running on every keystroke.
- Keep the fix inside `live_host`/`styler`. Do not raise the threshold.

- [ ] **Step 3: README**

Add a "Live Markdown" paragraph to the Markdown section (README l.84-96) and a row to the shortcuts table (l.281-289):

```markdown
### Live Markdown

Press **Ctrl+Alt+V** (View → Live Markdown) to see Markdown rendered while you type: headings
are large, `**bold**` shows as **bold**, links show only their text, bullets, checkboxes,
tables, rules and local images are drawn in place. The line you're on shows its plain source
so you can edit it. Click a checkbox to tick it; Ctrl+Click a link to follow it. Set
`markdown_live_default=on` in fastpad.ini to open Markdown files in Live. Live is off for files
over 1 MB.

In every Markdown file: **Ctrl+B / Ctrl+I / Ctrl+`** toggle bold, italic and code, **Ctrl+K**
inserts a link, **Enter** continues a list, **Tab / Shift+Tab** nest list items and move
between table cells, and a table you edited is re-aligned when you leave it.
```

- [ ] **Step 4: Live check in the real app**

1. Back up the settings: `Copy-Item "$env:APPDATA\FastPad\fastpad.ini" "$env:TEMP\fastpad.ini.bak"`.
2. Build and run: `cargo build --release`, then `target\release\FastPad.exe docs\superpowers\specs\2026-10-06-markdown-live-mode-design.md`.
3. Press Ctrl+Alt+V and check:
   - headings H1–H6
   - bold, italic, strike and code
   - links with Ctrl+Click
   - bullets and nested bullets
   - clicking a task checkbox
   - quotes and rules
   - a fenced block with its language label
   - a table: the grid, Tab between cells, and re-alignment after an edit
   - a local image
   - the caret line showing source
   - multiple carets (Ctrl+Alt+Down)
4. Repeat in a dark theme and at zoom +3 and −2.
5. Open Split next to Live and scroll.
6. Restore the settings: `Copy-Item "$env:TEMP\fastpad.ini.bak" "$env:APPDATA\FastPad\fastpad.ini" -Force`.

- [ ] **Step 5: Full verification**

Run, in order:
- `cargo clippy --all-targets -- -D warnings`
- `cargo test --lib -- --test-threads=1`
- `cargo test --tests -- --test-threads=1`. This runs every integration target, including the harness dialogs.
- `pwsh -File tools/audit-dependencies.ps1`

Expected: everything passes, and the audit reports no new crates or features.

- [ ] **Step 6: Commit and open the PR**

```bash
git add README.md tests/windows/markdown_live.rs
git commit -m "docs: Live Markdown and the Markdown writing helpers"
git push -u origin feat/markdown-live-mode
gh pr create --title "Live Markdown mode" --body "Implements docs/superpowers/specs/2026-10-06-markdown-live-mode-design.md (plan: docs/superpowers/plans/2026-10-06-markdown-live-mode.md)."
```
Push through the gh credential helper, as the project memory says.
