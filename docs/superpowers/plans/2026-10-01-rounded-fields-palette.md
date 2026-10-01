# Rounded Fields and Palette Inside Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Round the command palette's search field and selected row, and the sidebar Search view's field boxes and button hover fills, with the GDI helper from step 3a.

**Architecture:** One new helper, `round::fill_bordered`, draws a 1px-bordered rounded box as two `fill_rounded` calls. The palette and the Search view call it and `fill_rounded`. The palette's outer frame stays square and high contrast stays square, through `radius_for`.

**Tech Stack:** Rust 2024, windows-sys GDI, `design::round` (`fill_rounded`, `radius_for`, `Corners`), `design::metrics` (`CONTROL_RADIUS`, `ROW_INSET_X`, `ROW_INSET_Y`, `scale`).

**Spec:** `docs/superpowers/specs/2026-10-01-rounded-fields-palette-design.md`

## Global Constraints

- No Direct2D, no new library loads, no allocation; nothing new runs before the first editable frame.
- Radius is `CONTROL_RADIUS` (4) through `radius_for(palette, CONTROL_RADIUS, dpi)`; insets are `ROW_INSET_X` and `ROW_INSET_Y` through `scale`. In high contrast (`palette.high_contrast`) every shape stays square with system colors.
- The palette's outer 1px frame stays square. Row height (26px), text positions, field layout and hit-testing do not change; only painted shapes do.
- Out of scope: the notebook and Favorites header buttons, tab and caption buttons, the focus ring, anything else in `row_list`.
- Colors are `COLORREF` (`0x00BBGGRR`). No attribution lines in commit messages; never skip hooks.
- Testing practice: compile with `cargo clippy --all-targets -- -D warnings`, run only the targeted tests with `-- --test-threads=1`; the controller runs the full suite once at the end.

## Review Focus

- A box smaller than 2px in either direction (the inner fill is empty): draws without panicking or leaving the rect.
- A field whose `Edit` child sits inside the rounded box: the `Edit` is borderless and inset, so it must not cover the rounded inner corners.
- The palette's owner-drawn row: the whole row rect must still be filled with the body color first, so no stale pixels remain when the selection moves.
- High contrast: corners equal the fill and border colors exactly, with no blended pixels.
- A hover fill on a toggle sitting inside the field box: `behind` is the field's fill (`editor_background`), not the panel color.

---

### Task 1: The bordered rounded box

**Files:**
- Modify: `src/window/design/round.rs`

**Interfaces:**
- Consumes: `fill_rounded(dc, rect, radius, corners, color, behind)`, `Corners::ALL` (both in this file).
- Produces: `round::fill_bordered(dc: HDC, rect: RECT, radius: i32, fill: u32, border: u32, behind: u32)` (`unsafe fn`, `pub(crate)`).

- [ ] **Step 1: Write the failing tests**

In the existing `tests` module of `design/round.rs` (it already has `with_canvas`, `BEHIND`, `FILL`), add:

```rust
    const BORDER: u32 = 0x0000_8040;

    #[test]
    fn a_bordered_box_has_a_rounded_outside_a_border_and_a_fill() {
        // Break caught: a border that is not drawn, or one that stays square at the corner.
        let rect = RECT { left: 10, top: 10, right: 30, bottom: 30 };
        with_canvas(
            |dc| unsafe { fill_bordered(dc, rect, 6, FILL, BORDER, BEHIND) },
            |pixel| {
                assert_eq!(pixel(10, 10), BEHIND, "outer corner stays the surface color");
                assert_eq!(pixel(20, 10), BORDER, "top border");
                assert_eq!(pixel(10, 20), BORDER, "left border");
                assert_eq!(pixel(29, 20), BORDER, "right border");
                assert_eq!(pixel(20, 29), BORDER, "bottom border");
                assert_eq!(pixel(20, 11), FILL, "just inside the top border");
                assert_eq!(pixel(20, 20), FILL, "center");
                assert_eq!(pixel(9, 20), BEHIND, "outside the rect");
            },
        );
    }

    #[test]
    fn a_bordered_box_with_no_radius_is_two_square_fills() {
        let rect = RECT { left: 10, top: 10, right: 30, bottom: 30 };
        with_canvas(
            |dc| unsafe { fill_bordered(dc, rect, 0, FILL, BORDER, BEHIND) },
            |pixel| {
                assert_eq!(pixel(10, 10), BORDER, "corner is border");
                assert_eq!(pixel(11, 11), FILL, "just inside the corner");
            },
        );
    }

    #[test]
    fn a_bordered_box_too_small_for_a_fill_stays_inside_its_rect() {
        let rect = RECT { left: 10, top: 10, right: 12, bottom: 12 };
        with_canvas(
            |dc| unsafe { fill_bordered(dc, rect, 4, FILL, BORDER, BEHIND) },
            |pixel| {
                assert_eq!(pixel(9, 9), BEHIND);
                assert_eq!(pixel(12, 12), BEHIND);
            },
        );
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --lib window::design::round -- --test-threads=1`
Expected: compile error, `fill_bordered` not found.

- [ ] **Step 3: Implement**

In `design/round.rs`, after `fill_rounded`:

```rust
/// Fills `rect` as a box with a one-pixel `border`: the outer rounded shape in the border color,
/// then the inner shape in `fill`, inset by one pixel with the radius reduced to match, so the
/// border keeps an even width around the corners. `behind` is the flat color under the box.
pub(crate) unsafe fn fill_bordered(
    dc: HDC,
    rect: RECT,
    radius: i32,
    fill_color: u32,
    border: u32,
    behind: u32,
) {
    let inner = RECT {
        left: rect.left + 1,
        top: rect.top + 1,
        right: rect.right - 1,
        bottom: rect.bottom - 1,
    };
    unsafe {
        fill_rounded(dc, rect, radius, Corners::ALL, border, behind);
        fill_rounded(dc, inner, (radius - 1).max(0), Corners::ALL, fill_color, border);
    }
}
```

Remove any `allow(dead_code)` need by Task 2 and 3 using it; if clippy complains about `fill_bordered` being unused after this task, add `#[allow(dead_code, reason = "used by the palette and Search view in the next tasks")]` on it and remove the attribute in Task 2.

- [ ] **Step 4: Run to verify they pass**

Run: `cargo test --lib window::design::round -- --test-threads=1`
Expected: PASS.

- [ ] **Step 5: Lint and commit**

Run: `cargo fmt && cargo clippy --all-targets -- -D warnings`

```bash
git add src/window/design
git commit -m "feat: fill_bordered draws a one-pixel-bordered rounded box"
```

---

### Task 2: The command palette's field and selected row

**Files:**
- Modify: `src/window/command_palette.rs` (`paint_panel` ~line 870, `draw_item` ~line 900-960, `draw_quick_open_row` ~line 1000-1040)
- Test: `src/window/command_palette.rs` tests (or the nearest existing test module for the palette)

**Interfaces:**
- Consumes: `round::{fill_bordered, fill_rounded, radius_for, Corners}`, `metrics::{CONTROL_RADIUS, ROW_INSET_X, ROW_INSET_Y, scale}`, the existing `colors` (a palette copy), `layout.field`, and the DPI the palette already keeps for its own scaling (look at how `ROW_HEIGHT_AT_96_DPI` is scaled and reuse that DPI source).
- Produces: two small free functions that take a DC so they can be tested with a memory DC: `paint_field(dc, field: RECT, colors: &Palette, dpi: u32)` and `paint_row_background(dc, row: RECT, selected: bool, colors: &Palette, dpi: u32)`.

- [ ] **Step 1: Write the failing tests**

Add tests using a memory DC (copy the helper pattern from `design/round.rs` tests: `CreateCompatibleDC`, a 200x80 bitmap, a `GetPixel` reader):

- `paint_field` into a rect `(10,10)-(110,40)` after filling the surface with `colors.strip_background`: the outer corner pixel `(10,10)` equals `strip_background`; a pixel on the top border `(60,10)` equals `selection_background`; a pixel just inside `(60,11)` equals `editor_background`.
- `paint_row_background` with `selected = true` on row `(0,0)-(100,26)` after filling the surface with `strip_background`: `(1,13)` equals `strip_background` (inside the 4px inset), `(4,13)` equals `hover_background`, `(50,0)` equals `strip_background` (1px top gap), the fill's outer corner `(4,1)` equals `strip_background`. With `selected = false` every pixel stays `strip_background`.
- The same two with `colors.high_contrast = true`: the field's corner `(10,10)` equals `selection_background`; the selected row's `(0,0)` equals `hover_background` (the whole row, square, as today).

Build `colors` from `Palette::for_theme(Theme::ALL[0], false)` (`Theme` is in `crate::platform::theme`) so the test uses real palette colors. Run the tests and see them fail to compile (the functions do not exist).

- [ ] **Step 2: Implement**

`paint_field`:

```rust
fn paint_field(dc: HDC, field: RECT, colors: &Palette, dpi: u32) {
    unsafe {
        fill_bordered(
            dc,
            field,
            radius_for(colors, CONTROL_RADIUS, dpi),
            colors.editor_background,
            colors.selection_background,
            colors.strip_background,
        );
    }
}
```

`paint_row_background`: first fill the whole row with `colors.strip_background` (this keeps the current behaviour of repainting the row), then if `selected`, draw the `hover_background` fill: in high contrast over the whole row, otherwise `fill_rounded` over `row` inset by `scale(ROW_INSET_X, dpi)` left and right and `scale(ROW_INSET_Y, dpi)` top and bottom, `Corners::ALL`, radius `radius_for(colors, CONTROL_RADIUS, dpi)`, `behind = colors.strip_background`.

Use them: in `paint_panel`, replace the two lines `fill(dc, layout.field, selection_background); fill(dc, inset(layout.field, 1), editor_background);` with `paint_field(dc, layout.field, &colors, dpi)`; keep the outer frame fills. In `draw_item` and `draw_quick_open_row`, replace `fill(dc, item.rcItem, background)` with `paint_row_background(dc, item.rcItem, selected, &colors, dpi)`; the foreground and text colors keep coming from the existing `selected` branches, and the text rect is unchanged. `draw_quick_open_row` has its own `background` and `selected`; read it and keep its colors: the selected background there is the same `hover_background`, so the same function applies; if it differs, pass that color as an extra parameter to `paint_row_background` instead of changing the colors.

If the palette's DPI is not at hand in `paint_panel` or `draw_item`, find how those functions already scale (they must scale something) and use the same source; do not hard-code 96.

Update the doc comment of `paint_panel` ("the field box ... accent outline") to say the field box is rounded.

If Task 1 left an `allow(dead_code)` on `fill_bordered`, remove it.

- [ ] **Step 3: Run**

Run: `cargo clippy --all-targets -- -D warnings`, then `cargo test --lib window::command_palette -- --test-threads=1` and `cargo test --lib window::main_window::tests::command_palette -- --test-threads=1` (or the nearest palette test modules). Expected: PASS; update an existing pixel expectation only where it read the old square field corner or full-row selected fill, without weakening other assertions.

- [ ] **Step 4: Commit**

```bash
git add src
git commit -m "feat: rounded search field and selected row in the command palette"
```

---

### Task 3: The Search view's fields and button fills

**Files:**
- Modify: `src/window/search_view/paint.rs` (the two field boxes ~lines 172 and 216, the hover fills ~185, 203, 222), `src/window/option_toggles.rs` (its `paint` hover fills)
- Test: `src/window/search_view/tests.rs` (find a test that paints the view into a DC; if there is none, extract `paint_field(dc, rect, palette, dpi)` as in Task 2 and test that)

**Interfaces:**
- Consumes: `round::{fill_bordered, fill_rounded, radius_for, Corners}`, `metrics::CONTROL_RADIUS`, `paint.background`, `palette`, `dpi`.
- Produces: nothing new beyond an optional `paint_field` helper.

- [ ] **Step 1: Write the failing tests**

Add tests with a memory DC: after filling a surface with `palette.strip_background`/`paint.background` (use the same color the view uses for `paint.background`), the field box's outer corner pixel equals that background, its top-border pixel equals `selection_background`, and the pixel just inside equals `editor_background`; a hover fill on a button rect is rounded (its outer corner equals the background) and its middle equals `hover_background`; with `high_contrast = true` the field's corner equals `selection_background` and the hover fill's corner equals `hover_background`. Run and see them fail.

- [ ] **Step 2: Implement**

In `search_view/paint.rs`:
- Replace each `fill(paint.hdc, field, palette.selection_background); fill(paint.hdc, inset(field, 1), palette.editor_background);` pair (the search field and the replace field) with `fill_bordered(paint.hdc, field, radius, palette.editor_background, palette.selection_background, paint.background)` where `let radius = radius_for(&palette, CONTROL_RADIUS, dpi);` is computed once.
- Replace each `if hover { fill(paint.hdc, <rect>, palette.hover_background); }` (the chevron, the clear button and replace-all) with `fill_rounded(paint.hdc, <rect>, radius, Corners::ALL, palette.hover_background, paint.background)` under the same condition.

In `option_toggles.rs` `paint`: its hover and pressed fills sit inside the search field, so `behind` is `palette.editor_background`; give them the same `fill_rounded` with `CONTROL_RADIUS`. It needs the DPI: if its signature has none, add a `dpi: u32` parameter and update its caller (and its tests) with the DPI the caller already has. If `option_toggles::paint` has any selected or active fill (an on-state background), round that one the same way.

Do not change hit-testing, rects or layout.

- [ ] **Step 3: Run**

Run: `cargo clippy --all-targets -- -D warnings`, then `cargo test --lib window::search_view -- --test-threads=1`, `window::option_toggles` and `window::main_window::tests::search_view` (each with `-- --test-threads=1`). Expected: PASS; update existing pixel expectations only where they read an old square corner, without weakening others.

- [ ] **Step 4: Commit**

```bash
git add src
git commit -m "feat: rounded Search view fields and button fills"
```
