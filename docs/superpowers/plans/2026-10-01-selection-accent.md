# Accent Selection Indicator Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The selected row of every sidebar list, the command palette and the dropdown list gets a short rounded accent bar on its left edge.

**Architecture:** One pure geometry function (`round::selection_bar_rect`) and one GDI painter (`round::paint_selection_bar`) in `design/round.rs`, two constants in `design/metrics.rs`. `row_list::paint` (shared by Favorites, Notebook, Open editors and Search) and the command palette call the painter; the Direct2D dropdown draws the same rect as a `Shape::Round`. Fills and palette values do not change.

**Tech Stack:** Rust 2024, windows-sys GDI, the app's Direct2D `Shape::Round` (dropdown only).

**Spec:** `docs/superpowers/specs/2026-10-01-selection-accent-design.md`

## Global Constraints

- No color value changes; the bar is additive over the existing selected fill.
- Bar color is `palette.accent`; no bar when `palette.high_contrast`.
- Bar: 3px wide, 16px tall at 96 DPI (`scale`), flush with the backplate's left edge, vertically centered, height clamped to backplate height minus `2 * scale(CONTROL_RADIUS, dpi)` and floored at 1. Radius half the width.
- `row_list::paint` draws the bar only for the selected row while `focused`. Hover never gets one.
- GDI only for chrome (Direct2D is loaded only when a dialog opens); the dropdown list is already Direct2D.
- Nothing is loaded or allocated at startup. No attribution lines in commits.
- Run Clippy (`cargo clippy --all-targets`) to compile, and only the targeted tests named in each task (`-- --test-threads=1`). The full suite runs once at the end (Task 4).

## Review Focus

- A focused selected row at a 20px row height (small backplate): the bar clamps and stays inside the fill (Task 1 test, Task 2 test).
- Selected row inside a tree-drag band: the bar blends toward the selected fill, not the panel (Task 2 passes the fill as `behind`).
- High contrast: no bar anywhere (Tasks 1, 2, 3 tests).
- 150% DPI: bar still clears the narrowest content inset (Task 2 clearance test).
- Unfocused panel: no bar (Task 2 test).

---

### Task 1: Bar constants, geometry and painter

**Files:**
- Modify: `src/window/design/metrics.rs` (constants, near `FOCUS_GAP`; the `shared_metrics_keep_the_values...` test)
- Modify: `src/window/design/round.rs` (add `selection_bar_rect`, `paint_selection_bar`, tests in its `tests` module)

**Interfaces:**
- Produces: `metrics::SELECTION_BAR_WIDTH: i32 = 3`, `metrics::SELECTION_BAR_HEIGHT: i32 = 16`; `round::selection_bar_rect(backplate: RECT, dpi: u32) -> RECT`; `unsafe round::paint_selection_bar(dc: HDC, backplate: RECT, palette: &Palette, behind: u32, dpi: u32)` (pub(crate)).

- [ ] **Step 1: Write the failing tests** in `round.rs`'s `tests` module (reuse its memory-DC helper if there is one; otherwise create a bitmap as other tests in the file do):

```rust
#[test]
fn the_selection_bar_is_flush_left_centered_and_clamped() {
    // Break caught: a bar that drifts off the row's left edge, off center, or into the
    // backplate's rounded corners on a short row.
    let tall = RECT { left: 4, top: 1, right: 96, bottom: 25 }; // 24px, a real sidebar row
    let bar = selection_bar_rect(tall, 96);
    assert_eq!((bar.left, bar.right), (4, 7));
    assert_eq!((bar.top, bar.bottom), (5, 21)); // 16px, centered
    let short = RECT { left: 4, top: 21, right: 96, bottom: 39 }; // 18px: room is 18 - 8 = 10
    let bar = selection_bar_rect(short, 96);
    assert_eq!(bar.bottom - bar.top, 10);
    assert_eq!(bar.top - short.top, short.bottom - bar.bottom);
    let tiny = RECT { left: 0, top: 0, right: 50, bottom: 6 };
    assert_eq!(selection_bar_rect(tiny, 96).bottom - selection_bar_rect(tiny, 96).top, 1);
    // Scales with DPI.
    let big = RECT { left: 8, top: 2, right: 190, bottom: 50 }; // 48px at 192 DPI
    let bar = selection_bar_rect(big, 192);
    assert_eq!((bar.right - bar.left, bar.bottom - bar.top), (6, 32));
    assert_eq!(bar.left, 8);
}

#[test]
fn the_selection_bar_paints_accent_over_the_fill_and_nothing_in_high_contrast() {
    // Break caught: a bar in the wrong color, bleeding past its rect, or drawn in high contrast
    // where it would vanish into the highlight fill.
    // Create a 100x40 memory bitmap DC filled with FILL (0x00aa_bbcc), as the other tests here do.
    // backplate = RECT { left: 4, top: 1, right: 96, bottom: 39 }; palette = Palette::neutral()
    // with accent = 0x0000_00ff.
    // paint_selection_bar(dc, backplate, &palette, FILL, 96), then:
    //   middle pixel of the bar (5, 20) == accent
    //   pixel just right of the bar (7, 20) == FILL
    //   pixel just above the bar (5, bar.top - 1) == FILL
    //   outermost corner pixel (bar.left, bar.top) != accent (rounded)
    // Then refill FILL, set palette.high_contrast = true, paint again: (5, 20) == FILL.
}
```
Write the second test out fully (bitmap setup as in `row_list.rs`'s `rounded_corners_blend_toward...` test).

- [ ] **Step 2: Run to verify failure** — `cargo test --lib window::design::round -- --test-threads=1`; expected: compile error, `selection_bar_rect` not found.

- [ ] **Step 3: Implement.** In `metrics.rs` after `FOCUS_GAP`:

```rust
/// The accent bar on a selected row: width and height at 96 DPI.
pub(crate) const SELECTION_BAR_WIDTH: i32 = 3;
pub(crate) const SELECTION_BAR_HEIGHT: i32 = 16;
```
and add `assert_eq!(SELECTION_BAR_WIDTH, 3); assert_eq!(SELECTION_BAR_HEIGHT, 16);` to `shared_metrics_keep_the_values_the_old_constants_had` (and the imports). In `round.rs` (extend the `metrics` import with the two constants):

```rust
/// The accent bar for a selected row inside `backplate`: flush with its left edge and vertically
/// centered, `SELECTION_BAR_HEIGHT` tall but never closer than the corner radius to the top or
/// bottom, so it stays clear of the backplate's rounded corners.
pub(crate) fn selection_bar_rect(backplate: RECT, dpi: u32) -> RECT {
    let room = backplate.bottom - backplate.top - 2 * scale(CONTROL_RADIUS, dpi);
    let height = scale(SELECTION_BAR_HEIGHT, dpi).min(room).max(1);
    let top = backplate.top + (backplate.bottom - backplate.top - height) / 2;
    RECT {
        left: backplate.left,
        top,
        right: backplate.left + scale(SELECTION_BAR_WIDTH, dpi),
        bottom: top + height,
    }
}

/// Paints the selected row's accent bar over its fill (`behind` is the fill's color, which the
/// rounded ends blend toward). Nothing in high contrast, where the selection is already the
/// highlight color and `accent` is that same color.
pub(crate) unsafe fn paint_selection_bar(
    dc: HDC,
    backplate: RECT,
    palette: &Palette,
    behind: u32,
    dpi: u32,
) {
    if palette.high_contrast {
        return;
    }
    let rect = selection_bar_rect(backplate, dpi);
    unsafe {
        fill_rounded(dc, rect, (rect.right - rect.left) / 2, Corners::ALL, palette.accent, behind);
    }
}
```

- [ ] **Step 4: Run to verify pass** — `cargo test --lib window::design:: -- --test-threads=1`; expected: PASS.

- [ ] **Step 5: Commit** — `git add -A src && git commit -m "feat: selection bar geometry and painter"`

---

### Task 2: Sidebar lists (row_list), Favorites open bar, clearance

**Files:**
- Modify: `src/window/row_list.rs` (`paint`, around the `if look.selected` block; its tests)
- Modify: `src/window/favorites_view.rs:311-321` (open-notebook bar color)
- Test: `src/window/row_list.rs` tests, `src/window/notebook_view/tests.rs`, and the Favorites tests where the open bar is covered

**Interfaces:**
- Consumes: `round::paint_selection_bar(dc, backplate, palette, behind, dpi)` from Task 1.

- [ ] **Step 1: Write the failing tests.**
  - `row_list.rs`: a test `a_focused_selected_row_gets_the_accent_bar_and_no_other_row_does` using the first painting test's setup (20px rows, `selected = Some(1)`, `hover = Some(2)`, palette with `accent = 0x0000_ffaa`-style unique color, 100x100 area). Backplate of row 1 is `(4, 21)-(96, 39)` so the bar is x 4..7, y 25..35. Expect `GetPixel(5, 30) == accent` when `focused`; `GetPixel(7, 30) == selection_background`; with `focused = false`: `GetPixel(5, 30) == inactive_selection_background`; hovered row 2: `GetPixel(5, 50) == hover_background`; unselected row: no accent anywhere in the column x=5; high contrast (`high_contrast: true`): `GetPixel(5, 30) == selection_background`.
  - `rounded_corners_blend_toward_the_color_under_their_own_row` stays as is (the bar is not at its probe pixels (4, 21) and (12, 30)); confirm it still passes.
  - `notebook_view/tests.rs`: `the_selection_bar_clears_the_narrowest_row_content` — for dpi in `[96, 120, 144, 192]`: `scale(ROW_INSET_X, dpi) + scale(SELECTION_BAR_WIDTH, dpi) <= scale(super::LEFT_PAD, dpi)`.
  - Favorites: a test that the open notebook's bar pixel equals `palette.accent` (find the existing test for the open bar via `grep -n "open" src/window/favorites_view.rs` tests, or add one painting a row with `open: true`).

- [ ] **Step 2: Run to verify failure** — `cargo test --lib window::row_list -- --test-threads=1`; expected: the new test FAILS (no bar yet); the first painting test passes for now.

- [ ] **Step 3: Implement.** In `row_list.rs`, import `paint_selection_bar` with the other `round` imports. In `paint`, inside `if look.selected { ... }` after the `fill_rounded` call:

```rust
                if focused {
                    paint_selection_bar(hdc, backplate, palette, background, dpi);
                }
```
In `favorites_view.rs` change the open-notebook bar's `palette.selection_background` to `palette.accent` (comment stays accurate).

- [ ] **Step 4: Fix the existing tests the bar now touches.** `painting_draws_only_the_rows_in_view...` asserts `GetPixel(dc, 4, 20 + 13)` equals the selection fill; for the focused pass that pixel is now accent (bar x 4..7, y 25..35 in the 20..40 row). Move that probe to `(7, 20 + 13)` for both passes (the fill's left edge just right of the bar) and keep every other assertion. Run the whole `window::row_list`, `window::favorites_view`, `window::notebook_view`, `window::search_view`, `window::open_editors` test modules and fix any other probe pixel the same way, keeping each test's intent. A probe that needs the pre-bar value moves right of x=7 or off y 25..35 of the selected row, never loses its assertion.

- [ ] **Step 5: Run to verify pass** — `cargo test --lib window::row_list window::favorites_view window::notebook_view window::search_view window::open_editors -- --test-threads=1`; expected: PASS.

- [ ] **Step 6: Commit** — `git add -A src && git commit -m "feat: selected sidebar rows get the accent bar"`

---

### Task 3: Command palette and dropdown list

**Files:**
- Modify: `src/window/command_palette.rs` (`paint_row_background`, ~line 422)
- Modify: `src/window/command_palette/tests.rs` (`painting` module, ~lines 456-501)
- Modify: `src/window/dropdown_list.rs` (~lines 441-478; no test, judged in the build)

**Interfaces:**
- Consumes: `round::paint_selection_bar`, `round::selection_bar_rect` from Task 1.

- [ ] **Step 1: Write the failing test** in the `painting` module: `the_selected_row_has_an_accent_bar` — with `ROW` (0,0)-(100,26), `paint_row_background(dc, ROW, true, &colors, 96)`; the fill rect is (4,1)-(96,25) so the bar is x 4..7, y 5..21: `pixel(5, 13) == colors.accent`, `pixel(7, 13) == colors.hover_background`. In the existing `an_unselected_row_stays_the_strip_color` test nothing changes. In high contrast the existing `high_contrast_keeps_a_square_field...` test already asserts the row fill; add `pixel(5, 13) == colors.hover_background` there (no bar).

- [ ] **Step 2: Run to verify failure** — `cargo test --lib window::command_palette -- --test-threads=1`; expected: the new test FAILS.

- [ ] **Step 3: Implement.** In `paint_row_background`, after the `fill_rounded` call for the selected row, add `paint_selection_bar(dc, fill_rect, colors, colors.hover_background, dpi);`. Fix `the_selected_row_is_an_inset_rounded_fill`: its `pixel(4, 13)` "fill edge" probe becomes `pixel(7, 13)`.
  In `dropdown_list.rs`, the list needs its DPI for the bar: find where the list's `radius` is computed (it uses `radius_for(.., dpi)`), store `dpi` on the list struct next to `radius`, and after the selected row's `frame.shape(Shape::Round { rect: highlight, .. })` add, when `index == list.model.selected && !colors.high_contrast`:

```rust
frame.shape(Shape::Round {
    rect: selection_bar_rect(highlight, list.dpi),
    radius: scale(SELECTION_BAR_WIDTH, list.dpi) / 2,
    color: colors.accent,
});
```
(import `selection_bar_rect`, `SELECTION_BAR_WIDTH`, `scale` as needed; match the field name the list struct actually uses.)

- [ ] **Step 4: Run to verify pass** — `cargo clippy --all-targets` (compiles the dropdown) then `cargo test --lib window::command_palette window::dropdown_list -- --test-threads=1`; expected: PASS, no warnings.

- [ ] **Step 5: Commit** — `git add -A src && git commit -m "feat: palette and dropdown selected rows get the accent bar"`

---

### Task 4: Verify and build the preview

**Files:** none changed unless a check fails.

- [ ] **Step 1:** `cargo clippy --all-targets -- -D warnings` (or the project's usual Clippy invocation); expected: clean.
- [ ] **Step 2:** Full suite once: `cargo test --lib -- --test-threads=1`; expected: all pass (previous baseline 1562 passed, 3 ignored, plus the new tests).
- [ ] **Step 3:** Back up `fastpad.ini` (see memory note on live checks), run `./tools/package.ps1` via PowerShell, copy the build to `dist/step5-preview/` as earlier steps did, restore the settings file.
- [ ] **Step 4:** Report the preview path and what to check by eye: the bar on the focused selected row in Favorites, Notebook, Open editors and Search; the dimmer fill with no bar when the panel loses focus; the command palette's and a dropdown's selected row; the Light, Latte and Paper themes; 150% DPI; high contrast (no bar).
