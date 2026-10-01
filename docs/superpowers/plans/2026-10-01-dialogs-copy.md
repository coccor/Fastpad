# Themed Prompts and Sentence-Case Copy Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The close and confirm prompts open as a themed popup with action-named buttons instead of `MessageBoxW`, and user-facing labels use Windows 11 sentence case.

**Architecture:** A new `src/window/prompt.rs` holds the popup: pure layout and key logic (Task 2), then the window, paint and modal loop modelled on `about.rs` (Task 3). `modal.rs` keeps its two entry points and test hooks and swaps `MessageBoxW` for `prompt::show` (Task 4); `confirm` gains an action label. The copy pass is independent and goes first (Task 1).

**Tech Stack:** Rust 2024, windows-sys, GDI plus the app's `Canvas`/`Frame` (Direct2D with GDI fallback) from `soft_paint`.

**Spec:** `docs/superpowers/specs/2026-10-01-dialogs-copy-design.md`

## Global Constraints

- The startup-fatal `MessageBoxW` in `src/main.rs`, the About box and the Settings dialog are not changed.
- Buttons: close prompt **Save** (primary, focused first), **Don't save**, **Cancel**; confirm `{action}` (primary) and **Cancel**. Cancel is always the last button and is what Esc, the title ×, `WM_CLOSE` and a closing owner choose. Callers: **Replace** (Search replace, tree-copy replace), **Delete** (note delete, folder delete).
- Close prompt message: `Save changes to {title}?` (no "before closing"). Every other message text is unchanged.
- Window: 400px wide at 96 DPI (grows only if the buttons need more), padding 20, buttons min 88x30 at 96 DPI (height via `scale_text`), `type_ramp` Title and Body fonts, rounded corners with DWM shadow like About, no startup cost (class and Direct2D load at the first prompt).
- High contrast: square corners, system colors only, primary filled with the highlight pair, focus ring on the focused button.
- Keys: Enter/Space chooses the focused button, Esc is Cancel, Tab/Shift+Tab cycle, close prompt `S` = Save and `D` = Don't save.
- Copy: sentence case; proper nouns stay (Catppuccin, font names, FastPad, Markdown, Explorer, Windows). "Reveal in Explorer" is unchanged.
- No attribution lines in commits. Compile via `cargo clippy --all-targets`; run only the targeted tests named in each task with `-- --test-threads=1` (use `--lib`). Full suite once in Task 5.

## Review Focus

- A very long message and a very long button label at 225% text size and 192 DPI: nothing clips or overlaps (Task 2 layout test).
- Esc, the title × and a closing owner all return Cancel, never a destructive choice (Tasks 3, 4 tests).
- Enter with the default focus never picks Cancel; the default is the primary (Task 2, 3 tests).
- The owner window is re-enabled and regains activation after the prompt (Task 3, checked in the build).
- Every changed string is also changed where a test, accessible name or command-palette title mirrors it (Task 1 grep).

---

### Task 1: Sentence-case copy

**Files (grep each old string; change every non-comment use and every test that pins it):**
- Modify: `src/window/menus.rs` (~386), `src/window/settings_model.rs` (~21, ~406), `src/window/settings_dialog/shortcuts_input.rs` (~120), `src/window/accessibility.rs` (~70, ~1061), `src/window/preview_host.rs` (~1385), `src/window/preview_host/tests.rs` (~16-22), `src/window/command_palette.rs` (entries listed below), `src/window/shortcuts_model.rs` (~22), `src/languages/registry.rs` (~67), `src/window/main_window/tests/json_and_recovery.rs` (~168), comments that quote a string.

**Changes** (old -> new):
- Menus: "Split Right" -> "Split right", "Split Down" -> "Split down".
- Settings: "Notes and Session" -> "Notes and session", "Keyboard Shortcuts" -> "Keyboard shortcuts" (page title; `Self::Shortcuts`).
- Shortcuts context menu: "Reset Keybinding" -> "Reset keybinding".
- Preview buttons and accessible names: "Open Preview" -> "Open preview", "Open Preview to the Side" -> "Open preview to the side" (`preview_host.rs`, `accessibility.rs`, tests).
- Language name: "Plain Text" -> "Plain text" (`languages/registry.rs`), palette "Language: Plain Text" -> "Language: Plain text", status-bar test string "Plain Text    UTF-8" -> "Plain text    UTF-8".
- Command palette entries (`command_palette.rs`) and `shortcuts_model.rs` extra titles: "Markdown Preview: Side by Side" -> "Markdown preview: Side by side", "Markdown Preview: Full" -> "Markdown preview: Full", "Markdown Preview: Cycle" -> "Markdown preview: Cycle", "Close Markdown Preview" -> "Close Markdown preview", "View: Split Editor Right" -> "View: Split editor right", "View: Split Editor Down" -> "View: Split editor down", "View: Close Editor Group" -> "View: Close editor group", "View: Move Editor into Next Group" -> "View: Move editor into next group", "View: Move Editor into Previous Group" -> "View: Move editor into previous group", "Preferences: Open Keyboard Shortcuts" -> "Preferences: Open keyboard shortcuts".

- [ ] **Step 1: Find every use.** Run `rg -n "Split Right|Split Down|Notes and Session|Keyboard Shortcuts|Reset Keybinding|Open Preview|Plain Text|Markdown Preview|Close Markdown|Split Editor|Close Editor Group|Move Editor|Open Keyboard" src tests benches README.md docs/*.md` (use the Grep tool). Also grep `command_palette.rs` for any other entry whose words after the first (and after a `Category: ` prefix) are capitalized common words, and add them to the list in your report, applying the same rule. Check that "Plain Text" is not used as a persisted key, a language lookup by name or a test fixture of a saved file; if it is, keep the stored value and change only the displayed name, and say so.
- [ ] **Step 2: Update the tests first** (strings the tests pin), run `cargo test --lib window::command_palette window::preview_host window::accessibility window::settings_model window::shortcuts_model window::status window::main_window::tests::json_and_recovery languages:: -- --test-threads=1`; expected: FAIL on the old strings.
- [ ] **Step 3: Change the source strings** as listed. Palette search is case-insensitive, so no query logic changes; check `command_palette` tests that filter by label still pass.
- [ ] **Step 4: Run the same targeted tests**; expected PASS. Run `cargo clippy --all-targets`.
- [ ] **Step 5: Commit** — `git add -A src && git commit -m "feat: sentence-case labels in menus, settings, preview buttons and the command palette"`

---

### Task 2: Prompt layout and key logic (pure)

**Files:**
- Create: `src/window/prompt.rs` (module with the pure parts and their tests; the window comes in Task 3)
- Modify: `src/window/mod.rs` (declare `mod prompt;` beside `mod about;`)

**Interfaces:**
- Produces: `Layout` (pub(crate)): `width, height, title_band, title, title_close, message, footer, buttons: Vec<RECT>`, `calculate(dpi, width, title_height, message_height, label_widths: &[i32]) -> Layout`; `dialog_width(dpi, label_widths: &[i32]) -> i32`; `content_width(dpi, label_widths: &[i32]) -> i32`; `next_focus(current, count, forward) -> usize`; `key_choice(key: u16, focus: usize, count: usize, quick: &[(u16, usize)]) -> Option<usize>`; `Target { Button(usize), TitleClose }` and `Layout::target_at(x, y) -> Option<Target>`.

- [ ] **Step 1: Write the failing tests** in `prompt.rs`'s `tests` module:

```rust
use super::*;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_ESCAPE, VK_RETURN, VK_SPACE};

fn inside(outer: RECT, inner: RECT) -> bool {
    inner.left >= outer.left && inner.top >= outer.top
        && inner.right <= outer.right && inner.bottom <= outer.bottom
}

#[test]
fn everything_fits_at_every_dpi_and_text_size() {
    // Break caught: a long message or a long button label clipping or overlapping at 192 DPI
    // and 225 % text.
    let _factor = crate::window::design::text_scale::FactorGuard::new();
    for factor in [100, 225] {
        crate::window::design::text_scale::set_factor_for_test(factor);
        for dpi in [96, 120, 144, 192] {
            for labels in [vec![88, 60], vec![60, 120, 60], vec![420, 70]] {
                let width = dialog_width(dpi, &labels);
                let layout = Layout::calculate(dpi, width, 24 * factor as i32 / 100, 300, &labels);
                let client = RECT { left: 0, top: 0, right: layout.width, bottom: layout.height };
                for rect in [layout.title_band, layout.title, layout.message, layout.footer]
                    .into_iter().chain(layout.buttons.iter().copied())
                {
                    assert!(inside(client, rect), "{factor} {dpi} {labels:?}");
                }
                assert!(layout.title_band.bottom <= layout.message.top);
                assert!(layout.message.bottom <= layout.footer.top);
                for button in &layout.buttons {
                    assert!(inside(layout.footer, *button));
                }
                for pair in layout.buttons.windows(2) {
                    assert!(pair[0].right < pair[1].left, "buttons never overlap");
                }
            }
        }
    }
}

#[test]
fn the_dialog_is_400px_and_grows_only_for_wide_buttons() {
    assert_eq!(dialog_width(96, &[60, 70]), 400);
    assert!(dialog_width(96, &[420, 70]) > 400);
    assert_eq!(dialog_width(192, &[60, 70]), 800);
}

#[test]
fn the_message_height_grows_the_dialog() {
    let short = Layout::calculate(96, 400, 24, 20, &[60, 70]);
    let long = Layout::calculate(96, 400, 24, 120, &[60, 70]);
    assert_eq!(long.height - short.height, 100);
}

#[test]
fn buttons_are_right_aligned_with_the_last_at_the_padding() {
    let layout = Layout::calculate(96, 400, 24, 40, &[60, 70, 60]);
    assert_eq!(layout.buttons.len(), 3);
    assert_eq!(layout.buttons[2].right, 400 - 20);
    assert!(layout.buttons[0].left < layout.buttons[1].left);
    assert_eq!(layout.target_at(layout.buttons[1].left + 1, layout.buttons[1].top + 1), Some(Target::Button(1)));
    assert_eq!(layout.target_at(layout.title_close.left + 1, layout.title_close.top + 1), Some(Target::TitleClose));
    assert_eq!(layout.target_at(1, layout.message.top + 1), None);
}

#[test]
fn keys_choose_cancel_on_escape_the_focus_on_enter_and_quick_letters() {
    // Break caught: Esc or a stray key choosing a destructive button; Enter ignoring the focus.
    let quick = [(u16::from(b'S'), 0), (u16::from(b'D'), 1)];
    assert_eq!(key_choice(VK_ESCAPE, 0, 3, &quick), Some(2));
    assert_eq!(key_choice(VK_RETURN, 0, 3, &quick), Some(0));
    assert_eq!(key_choice(VK_RETURN, 1, 3, &quick), Some(1));
    assert_eq!(key_choice(VK_SPACE, 2, 3, &quick), Some(2));
    assert_eq!(key_choice(u16::from(b'S'), 2, 3, &quick), Some(0));
    assert_eq!(key_choice(u16::from(b'D'), 0, 3, &quick), Some(1));
    assert_eq!(key_choice(u16::from(b'X'), 0, 3, &quick), None);
    assert_eq!(key_choice(u16::from(b'S'), 0, 2, &[]), None, "no quick keys, no choice");
    assert_eq!(key_choice(VK_ESCAPE, 0, 2, &[]), Some(1));
}

#[test]
fn tab_cycles_forward_and_back_and_wraps() {
    assert_eq!(next_focus(0, 3, true), 1);
    assert_eq!(next_focus(2, 3, true), 0);
    assert_eq!(next_focus(0, 3, false), 2);
    assert_eq!(next_focus(0, 1, true), 0);
}
```

- [ ] **Step 2: Run to verify failure** — `cargo test --lib window::prompt -- --test-threads=1`; expected: compile errors (items missing).
- [ ] **Step 3: Implement** the pure parts at the top of `prompt.rs`:

```rust
//! The themed prompt that replaces `MessageBoxW` for the close and confirm questions: an owned
//! popup in the theme's colors with action-named buttons, running its own modal loop like
//! `about.rs`. This file's first half is pure layout and key logic.

use super::design::metrics::scale;
use super::design::text_scale::scale_text;
use super::soft_paint::{TITLE_CLOSE_WIDTH_AT_96_DPI, TITLE_HEIGHT_AT_96_DPI};
use windows_sys::Win32::Foundation::RECT;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_ESCAPE, VK_RETURN, VK_SPACE};

const WIDTH_AT_96_DPI: i32 = 400;
const PADDING_AT_96_DPI: i32 = 20;
const TITLE_PAD_AT_96_DPI: i32 = 10;
const MESSAGE_GAP_AT_96_DPI: i32 = 12;
const FOOTER_GAP_AT_96_DPI: i32 = 20;
const FOOTER_HEIGHT_AT_96_DPI: i32 = 56;
const BUTTON_MIN_WIDTH_AT_96_DPI: i32 = 88;
const BUTTON_HEIGHT_AT_96_DPI: i32 = 30;
const BUTTON_PAD_AT_96_DPI: i32 = 16;
const BUTTON_GAP_AT_96_DPI: i32 = 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Target {
    Button(usize),
    TitleClose,
}

fn button_widths(dpi: u32, label_widths: &[i32]) -> Vec<i32> {
    label_widths
        .iter()
        .map(|width| {
            scale(BUTTON_MIN_WIDTH_AT_96_DPI, dpi).max(width + 2 * scale(BUTTON_PAD_AT_96_DPI, dpi))
        })
        .collect()
}

fn buttons_total(dpi: u32, widths: &[i32]) -> i32 {
    widths.iter().sum::<i32>() + scale(BUTTON_GAP_AT_96_DPI, dpi) * (widths.len() as i32 - 1).max(0)
}

/// The dialog's width: 400px at 96 DPI, or wider when the buttons need more.
pub(crate) fn dialog_width(dpi: u32, label_widths: &[i32]) -> i32 {
    let needed = buttons_total(dpi, &button_widths(dpi, label_widths)) + 2 * scale(PADDING_AT_96_DPI, dpi);
    scale(WIDTH_AT_96_DPI, dpi).max(needed)
}

/// The width the message wraps to.
pub(crate) fn content_width(dpi: u32, label_widths: &[i32]) -> i32 {
    dialog_width(dpi, label_widths) - 2 * scale(PADDING_AT_96_DPI, dpi)
}

#[derive(Clone, Debug)]
pub(crate) struct Layout {
    pub width: i32,
    pub height: i32,
    pub title_band: RECT,
    pub title: RECT,
    pub title_close: RECT,
    pub message: RECT,
    pub footer: RECT,
    pub buttons: Vec<RECT>,
    pub(crate) dpi: u32,
}

impl Layout {
    /// `width` is `dialog_width`; `title_height` and `message_height` are the measured text heights.
    pub(crate) fn calculate(dpi: u32, width: i32, title_height: i32, message_height: i32, label_widths: &[i32]) -> Self {
        let padding = scale(PADDING_AT_96_DPI, dpi);
        let band_height = scale_text(TITLE_HEIGHT_AT_96_DPI, dpi)
            .max(title_height + 2 * scale(TITLE_PAD_AT_96_DPI, dpi));
        let title_band = RECT { left: 0, top: 0, right: width, bottom: band_height };
        let title_close = RECT {
            left: width - scale(TITLE_CLOSE_WIDTH_AT_96_DPI, dpi),
            top: 0,
            right: width,
            bottom: scale_text(TITLE_HEIGHT_AT_96_DPI, dpi).min(band_height),
        };
        let title_top = (band_height - title_height) / 2;
        let title = RECT { left: padding, top: title_top, right: title_close.left, bottom: title_top + title_height };
        let message_top = band_height + scale(MESSAGE_GAP_AT_96_DPI, dpi);
        let message = RECT { left: padding, top: message_top, right: width - padding, bottom: message_top + message_height };
        let footer_top = message.bottom + scale(FOOTER_GAP_AT_96_DPI, dpi);
        let footer = RECT { left: 0, top: footer_top, right: width, bottom: footer_top + scale_text(FOOTER_HEIGHT_AT_96_DPI, dpi) };
        let button_height = scale_text(BUTTON_HEIGHT_AT_96_DPI, dpi);
        let button_top = footer.top + (footer.bottom - footer.top - button_height) / 2;
        let gap = scale(BUTTON_GAP_AT_96_DPI, dpi);
        let mut right = width - padding;
        let mut buttons: Vec<RECT> = button_widths(dpi, label_widths)
            .into_iter()
            .rev()
            .map(|button_width| {
                let rect = RECT { left: right - button_width, top: button_top, right, bottom: button_top + button_height };
                right = rect.left - gap;
                rect
            })
            .collect();
        buttons.reverse();
        Self { width, height: footer.bottom, title_band, title, title_close, message, footer, buttons, dpi }
    }

    pub(crate) fn target_at(&self, x: i32, y: i32) -> Option<Target> {
        let inside = |rect: &RECT| x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom;
        if inside(&self.title_close) {
            return Some(Target::TitleClose);
        }
        self.buttons.iter().position(inside).map(Target::Button)
    }
}

/// The focus after Tab (`forward`) or Shift+Tab, wrapping.
pub(crate) fn next_focus(current: usize, count: usize, forward: bool) -> usize {
    if count == 0 {
        return 0;
    }
    if forward { (current + 1) % count } else { (current + count - 1) % count }
}

/// The button a key chooses, if any: Esc is the last (Cancel), Enter and Space the focused one,
/// and `quick` maps letters to buttons.
pub(crate) fn key_choice(key: u16, focus: usize, count: usize, quick: &[(u16, usize)]) -> Option<usize> {
    match key {
        VK_ESCAPE => count.checked_sub(1),
        VK_RETURN | VK_SPACE => Some(focus.min(count.saturating_sub(1))),
        _ => quick.iter().find(|(letter, index)| *letter == key && *index < count).map(|(_, index)| *index),
    }
}
```
Run `cargo fmt` on the file. Declare `mod prompt;` in `src/window/mod.rs`; an `#[allow(dead_code, reason = "used by modal.rs from the next task")]` on the module's items is acceptable until Task 4 and must be removed there.

- [ ] **Step 4: Run to verify pass** — `cargo test --lib window::prompt -- --test-threads=1`; expected PASS. `cargo clippy --all-targets` clean.
- [ ] **Step 5: Commit** — `git add -A src && git commit -m "feat: themed prompt layout and key logic"`

---

### Task 3: The prompt window, paint and modal loop

**Files:**
- Modify: `src/window/prompt.rs` (add the window half and its tests)

**Interfaces:**
- Consumes: Task 2's `Layout`, `Target`, `dialog_width`, `content_width`, `next_focus`, `key_choice`.
- Produces: `pub(crate) struct Spec<'a> { pub message: &'a str, pub buttons: &'a [&'a str], pub quick_keys: &'a [(u16, usize)] }` and `pub(crate) fn show(owner: HWND, colors: Palette, spec: &Spec) -> usize` (the chosen button index; Cancel, the last, for Esc, the ×, `WM_CLOSE` and any failure to open); test hook `#[cfg(test)] pub(crate) fn answer_next(answer: impl FnOnce(HWND) + 'static)` that runs once the window is shown, before the loop (the answer posts input), like `about::answer_next`.

Model it on `src/window/about.rs`: read `show`, `create`, `register_class`, `about_proc`, `paint`/`paint_into`/`compose` and its tests (`painted_about`, `the_box_paints_its_soft_look_whether_direct2d_or_the_gdi_fallback_paints`) first, and keep the same ordering and comments' reasons: hidden native frame (`WM_NCCALCSIZE => 0`, `WS_POPUP | WS_CAPTION | WS_CLIPCHILDREN`, `WS_EX_TOOLWINDOW`), `DWMWCP_ROUND` and the 1px `DwmExtendFrameIntoClientArea` margin, `WM_ERASEBKGND => 1`, buffered paint through `paint_buffered`, `Canvas::load()`, the `GetWindowLongPtrW == 0` guard at the top of the procedure, no borrow held across anything that re-enters, `PostQuitMessage` re-post, owner re-enabled before `DestroyWindow`.

- [ ] **Step 1: Write the failing tests** (in `prompt.rs`'s tests, following About's painting tests and its test hook):
  - `show_returns_the_primary_on_enter`: `answer_next(|dialog| post WM_KEYDOWN VK_RETURN to it)`; `show(owner, Palette::neutral(), &Spec { message: "Delete it?", buttons: &["Delete", "Cancel"], quick_keys: &[] })` returns 0. Owner: the test main window used by About's tests (see how its tests obtain one), or a plain top-level test window.
  - `escape_and_the_title_close_return_cancel`: Esc returns 1; a click on the × (post `WM_LBUTTONDOWN` and `WM_LBUTTONUP` at its center) returns 1; `WM_CLOSE` returns 1.
  - `tab_then_enter_picks_the_second_button` (three buttons, one Tab) and `a_quick_key_picks_its_button` (`S` returns 0, `D` returns 1 with the close prompt's quick keys).
  - `the_primary_button_is_accent_filled_and_the_second_is_not`: paint into a memory bitmap through the GDI fallback (`Canvas` fallback as About's painting test does) with `Palette::for_theme(Theme::ALL[1], false)`: the middle pixel of `buttons[0]` is `accent`, of `buttons[1]` is not `accent`, a window corner pixel differs from the client background at the rounded-corner setting is not tested (DWM rounds the window itself): instead assert the primary button's own corner pixel is a blend, not `accent`.
  - `high_contrast_fills_the_primary_with_the_highlight_pair`: with `Palette::for_theme(.., true)` the primary's middle pixel is `palette.accent` (the highlight), `buttons[1]` is the strip/panel color, and the primary's corner pixel equals `accent` (square).
  - `a_long_message_grows_the_window`: two prompts, one with a 40-word message, `layout.height` larger for the long one at the same DPI.
  Write the `compose` and state types the tests need to exist; use `answer_next` to drive the real window.
- [ ] **Step 2: Run to verify failure** — `cargo test --lib window::prompt -- --test-threads=1`; expected FAIL (no `show` yet).
- [ ] **Step 3: Implement** the window half:
  - `struct Prompt { colors: Palette, layout: Layout, message: String, labels: Vec<String>, quick: Vec<(u16, usize)>, title_font, body_font, glyph_font, focus: usize, hot: Option<Target>, pressed: Option<Target>, tracking_leave: bool, canvas: Canvas }` with `Drop` deleting the fonts, in `GWLP_USERDATA`.
  - A thread-local `CHOICE: Cell<Option<usize>>`; `finish(hwnd, index)` stores the choice then calls `close`. `show` clears it before the loop and returns `CHOICE.take().unwrap_or(last)` where `last = spec.buttons.len() - 1`.
  - `create`: fonts via `type_ramp::create(TextStyle::Title/Body, dpi)` and the glyph font as About; measure each label with `measure` (single-line width), the message with `DrawTextW(DT_CALCRECT | DT_WORDBREAK | DT_NOPREFIX)` on a rect `content_width` wide on the body font, then `Layout::calculate(dpi, dialog_width(..), text_height(title_font), measured_height, &label_widths)`. Center over the owner. Title text is `"FastPad"`. Initial `focus = 0`.
  - `prompt_proc`: `WM_KEYDOWN` -> `VK_TAB` cycles with `next_focus`, any other key through `key_choice` -> `finish`; mouse move/leave/down/up as About with `Target::Button(i)` and `TitleClose`; `WM_NCHITTEST` drags only over `title_band` outside the ×; `WM_CLOSE` -> `finish(hwnd, last)`; `WM_SETCURSOR` arrow.
  - `compose`: background `colors.panel_background()`, the title band `strip_background` with the title in `editor_foreground` and `title_close(...)` as About, rules in `tones.card` under the band and over the footer, the message in `editor_foreground` (`DT_LEFT | DT_WORDBREAK | DT_NOPREFIX`), buttons through `tones.soft(frame, rect, radius, fill)` — primary (index 0): `accent` / `accent_hot` / `accent_down` with `tones.on_accent` text; the others `tones.control` / `card_hot` as the Settings dialog's secondary controls do — and the focus ring exactly as About's. In high contrast the existing `Tones` already maps these to the system pairs; verify the test above passes rather than adding branches. (Spec 4.3 mentions a 1px `stroke` border; About draws none because the DWM shadow is the edge — follow About and say so in your report.)
  - The test hook `ANSWERS` and `answer_next`, run in `show` right after `ShowWindow`/`SetFocus` exactly as `about::show` does under `#[cfg(test)]`; a prompt opened in a test without an answer panics with a clear message, as About's does.
- [ ] **Step 4: Run to verify pass** — `cargo test --lib window::prompt -- --test-threads=1`; expected PASS. `cargo clippy --all-targets` clean (dead-code allowance from Task 2 still in place).
- [ ] **Step 5: Commit** — `git add -A src && git commit -m "feat: the themed prompt window"`

---

### Task 4: Wire modal.rs and the call sites

**Files:**
- Modify: `src/window/modal.rs` (`prompt_close_decision`, `confirm`, hooks; remove the now-unused `MessageBoxW` imports if nothing else in the file uses them)
- Modify: `src/window/library_host/pins.rs` (`confirmed` gains `action: &str`), callers `src/window/copy_host.rs` (~248), `src/window/library_host/note_actions.rs` (~303), `src/window/library_host/folders.rs` (~206), `src/window/text_search_host.rs` (~804)
- Modify: `src/window/prompt.rs` (remove the Task 2 dead-code allowance)
- Test: the existing tests that exercise those flows (grep `take_last_confirm`, `answer_next_confirm`, `answer_next_close_prompt`)

**Interfaces:**
- Consumes: `prompt::show(owner, colors, &Spec) -> usize`, `main_window::current_palette(hwnd)` (call with nothing of the `App` borrowed, as the existing code does).
- Produces: `modal::confirm(hwnd, text, action)`, `library_host::confirmed(hwnd, question, action)`, `#[cfg(test)] modal::take_last_confirm_action() -> Option<String>`.

- [ ] **Step 1: Write the failing tests.** In the existing flow tests add assertions on the action label via `take_last_confirm_action()`: Search replace (single-note and multi-note) -> `"Replace"`; tree-copy replace -> `"Replace"`; note delete and folder delete -> `"Delete"`. In `modal.rs`'s tests (or the nearest existing ones) assert that the close prompt's hook still returns Save, Discard and Cancel decisions as before, and that its production path builds the spec `["Save", "Don't save", "Cancel"]` with quick keys `S`/`D` and message `Save changes to {title}?`: extract the spec-building into a small pure fn `close_spec(title) -> (String, [&str; 3], [(u16, usize); 2])` and test it directly (message text, labels, quick keys, primary first, Cancel last). Likewise `confirm_labels(action) -> [String; 2]`-style pure helper tested for `["Replace", "Cancel"]`.
- [ ] **Step 2: Run to verify failure** — `cargo test --lib window::modal window::text_search_host window::copy_host window::library_host -- --test-threads=1`; expected FAIL / compile errors.
- [ ] **Step 3: Implement.**

```rust
pub(super) fn prompt_close_decision(hwnd: HWND, title: &str) -> CloseDecision {
    let _modal = ModalScope::enter(hwnd);
    #[cfg(test)]
    if let Some(answer) = CLOSE_ANSWERS.with(|answers| answers.borrow_mut().pop_front()) {
        return answer(hwnd);
    }
    let (message, buttons, quick) = close_spec(title);
    let spec = prompt::Spec { message: &message, buttons: &buttons, quick_keys: &quick };
    match prompt::show(hwnd, current_palette(hwnd), &spec) {
        0 => CloseDecision::Save,
        1 => CloseDecision::Discard,
        _ => CloseDecision::Cancel,
    }
}

pub(crate) fn confirm(hwnd: HWND, text: &str, action: &str) -> bool {
    let _modal = ModalScope::enter(hwnd);
    #[cfg(test)]
    {
        LAST_CONFIRM.with(|last| *last.borrow_mut() = Some(text.to_owned()));
        LAST_CONFIRM_ACTION.with(|last| *last.borrow_mut() = Some(action.to_owned()));
    }
    #[cfg(test)]
    if let Some(answer) = CONFIRM_ANSWERS.with(|answers| answers.borrow_mut().pop_front()) {
        return answer(hwnd);
    }
    let buttons = [action, "Cancel"];
    let spec = prompt::Spec { message: text, buttons: &buttons, quick_keys: &[] };
    prompt::show(hwnd, current_palette(hwnd), &spec) == 0
}
```
`close_spec` returns `(format!("Save changes to {title}?"), ["Save", "Don't save", "Cancel"], [(u16::from(b'S'), 0), (u16::from(b'D'), 1)])`. Add `LAST_CONFIRM_ACTION` and `take_last_confirm_action` beside `LAST_CONFIRM` with the same `#[allow(dead_code, reason = ...)]` pattern. `confirmed(hwnd, question, action)` forwards `action`; update the four call sites (Replace, Replace, Delete, Delete). Remove the Task 2 dead-code allowance. Update the module doc comment (`MessageBoxW` no longer used here except nothing) and `about.rs`'s doc line is left alone.
- [ ] **Step 4: Run to verify pass** — the same targeted modules plus `window::prompt`; expected PASS. `cargo clippy --all-targets` clean (no dead code, no unused imports).
- [ ] **Step 5: Commit** — `git add -A src && git commit -m "feat: close and confirm prompts use the themed window with action-named buttons"`

---

### Task 5: Verify and build the preview

**Files:** none changed unless a check fails.

- [ ] **Step 1:** `cargo clippy --all-targets -- -D warnings`; expected clean.
- [ ] **Step 2:** Full suite once: `cargo test --lib -- --test-threads=1`; expected all pass (baseline 1571 passed, 3 ignored, plus the new tests).
- [ ] **Step 3:** Back up `fastpad.ini` (see the memory note on live checks), run `./tools/package.ps1` via PowerShell, copy the build folder to `dist/step6-preview/`, restore the settings file.
- [ ] **Step 4:** Report the preview path and what to check by eye: close a modified tab in Light, Dark, Latte and high contrast (Save / Don't save / Cancel, `S` and `D`, Esc, ×); Search replace in a notebook and a note delete and a folder delete (Replace / Delete / Cancel); the prompt centered over the window, draggable by its title band, focus back in the editor afterwards; 150% DPI and a larger Windows text size; the changed labels in the Split menu, Settings (Notes and session, Keyboard shortcuts), the preview buttons and the command palette.
