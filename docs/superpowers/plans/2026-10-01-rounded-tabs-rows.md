# Rounded Tabs and Row Backplates Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Round the active and hovered tab's top corners and the sidebar's hover and selected row fills, with GDI only and no startup work.

**Architecture:** A new `design/round.rs` holds a pure corner-coverage function and `fill_rounded`, which fills the body with `FillRect` and writes only the corner pixels, blended toward the known background. `group_strip.rs` and `row_list.rs` call it. High contrast passes radius 0, so its shapes stay square with system colors.

**Tech Stack:** Rust 2024, windows-sys GDI (`FillRect`, `SetPixelV`), existing `catppuccin::blend`, `design::metrics::scale`.

**Spec:** `docs/superpowers/specs/2026-10-01-rounded-tabs-rows-design.md`

## Global Constraints

- No Direct2D and no new library loads; nothing runs before the first editable frame that did not run before. `fill_rounded` allocates nothing.
- Radii and insets are pixels at 96 DPI scaled through `design::metrics::scale`. `TAB_RADIUS = 8`, `ROW_INSET_X = 4`, `ROW_INSET_Y = 1`; the row and close-button radius is the existing `CONTROL_RADIUS` (4).
- High contrast (`palette.high_contrast`) draws square fills: radius 0, no blending.
- Row height (`SIDEBAR_ROW`), text, icon positions, indentation and hit-testing do not change; only painted fills move.
- Titlebar caption buttons, Search rows and fields, the command palette and focus rings are out of scope.
- Colors are `COLORREF` (`0x00BBGGRR`). Never skip hooks; no attribution lines in commit messages.
- Testing practice: compile with `cargo clippy --all-targets -- -D warnings`, run only the targeted tests (`-- --test-threads=1` for window tests); the controller runs the full suite once at the end.

## Review Focus

- A radius larger than half the rect (a 10px-tall row with radius 8, or a tab narrower than 2x radius): must clamp, never panic or draw outside the rect.
- A zero-size or inverted rect: draws nothing.
- DPI 0 or 192: radius and insets scale, corners stay symmetric.
- A row both hovered and selected paints one fill (no double blend).
- The tab strip scrolls (clips at `layout.tabs`): rounded corners must not draw outside the clip.
- High contrast: corners are exactly the fill color, with no blended pixels.

---

### Task 1: Metrics and the rounded-fill helper

**Files:**
- Modify: `src/window/design/metrics.rs`, `src/window/design/mod.rs`
- Create: `src/window/design/round.rs`

**Interfaces:**
- Consumes: `crate::catppuccin::blend(foreground: u32, background: u32, alpha: u32) -> u32` (alpha 0..=255), `crate::window::panel::fill(dc, rect, color)`, `crate::window::palette::Palette` (field `high_contrast: bool`), `design::metrics::scale`.
- Produces:
  - `metrics::TAB_RADIUS: i32 = 8`, `metrics::ROW_INSET_X: i32 = 4`, `metrics::ROW_INSET_Y: i32 = 1`.
  - `round::Corners` with consts `NONE`, `TOP_LEFT`, `TOP_RIGHT`, `BOTTOM_LEFT`, `BOTTOM_RIGHT`, `TOP`, `ALL`.
  - `round::coverage(x: i32, y: i32, radius: i32) -> u32` (0..=255).
  - `round::radius_for(palette: &Palette, at_96_dpi: i32, dpi: u32) -> i32` (0 in high contrast).
  - `round::fill_rounded(dc: HDC, rect: RECT, radius: i32, corners: Corners, color: u32, behind: u32)` (`unsafe fn`).

- [ ] **Step 1: Write the failing tests**

In `src/window/design/metrics.rs` tests, import `ROW_INSET_X, ROW_INSET_Y, TAB_RADIUS`, add to the value test:

```rust
        assert_eq!(TAB_RADIUS, 8);
        assert_eq!(ROW_INSET_X, 4);
        assert_eq!(ROW_INSET_Y, 1);
```

Do not add them to the off-grid `sizes` array (they are radii and insets, not layout sizes; the expected off-grid list stays `["PANEL_HEADER", "SIDEBAR_ROW"]`).

Create `src/window/design/round.rs` containing only the test module below, and add `pub(crate) mod round;` to `design/mod.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::Graphics::Gdi::{
        CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetPixel,
        ReleaseDC, SelectObject,
    };

    #[test]
    fn the_outermost_corner_pixel_is_outside_and_the_inner_ones_are_inside() {
        // Break caught: an inverted arc that fills the corner and trims the middle.
        for radius in [2, 4, 8, 16] {
            assert_eq!(coverage(0, 0, radius), 0, "outer pixel at radius {radius}");
            assert_eq!(
                coverage(radius - 1, radius - 1, radius),
                255,
                "inner pixel at radius {radius}"
            );
        }
    }

    #[test]
    fn coverage_is_symmetric_across_the_diagonal_and_grows_toward_the_center() {
        // Break caught: a swapped x and y, or coverage falling off toward the middle.
        let radius = 8;
        for y in 0..radius {
            for x in 0..radius {
                assert_eq!(coverage(x, y, radius), coverage(y, x, radius), "({x}, {y})");
                if x + 1 < radius {
                    assert!(coverage(x + 1, y, radius) >= coverage(x, y, radius), "({x}, {y})");
                }
            }
        }
    }

    #[test]
    fn a_zero_radius_has_full_coverage() {
        assert_eq!(coverage(0, 0, 0), 255);
    }

    #[test]
    fn the_high_contrast_palette_gets_square_corners() {
        use crate::window::palette::{Palette, Theme};
        let normal = Palette::for_theme(Theme::ALL[0], false);
        let high = Palette::for_theme(Theme::ALL[0], true);
        assert_eq!(radius_for(&normal, 8, 96), 8);
        assert_eq!(radius_for(&normal, 8, 192), 16);
        assert_eq!(radius_for(&high, 8, 96), 0);
    }

    const BEHIND: u32 = 0x0010_2030;
    const FILL: u32 = 0x00ee_ddcc;

    /// Paints into a 40x40 memory bitmap, then hands `read` a pixel reader.
    fn with_canvas(draw: impl FnOnce(HDC), read: impl FnOnce(&dyn Fn(i32, i32) -> u32)) {
        unsafe {
            let screen = GetDC(std::ptr::null_mut());
            let dc = CreateCompatibleDC(screen);
            let bitmap = CreateCompatibleBitmap(screen, 40, 40);
            let previous = SelectObject(dc, bitmap);
            crate::window::panel::fill(
                dc,
                RECT { left: 0, top: 0, right: 40, bottom: 40 },
                BEHIND,
            );
            draw(dc);
            read(&|x, y| GetPixel(dc, x, y));
            SelectObject(dc, previous);
            DeleteObject(bitmap);
            DeleteDC(dc);
            ReleaseDC(std::ptr::null_mut(), screen);
        }
    }

    #[test]
    fn a_rounded_fill_covers_the_middle_and_leaves_the_far_corner_and_outside_alone() {
        let rect = RECT { left: 10, top: 10, right: 30, bottom: 30 };
        with_canvas(
            |dc| unsafe { fill_rounded(dc, rect, 6, Corners::ALL, FILL, BEHIND) },
            |pixel| {
                assert_eq!(pixel(20, 20), FILL, "center");
                assert_eq!(pixel(10, 10), BEHIND, "outer corner");
                assert_eq!(pixel(29, 29), BEHIND, "opposite outer corner");
                assert_eq!(pixel(20, 10), FILL, "top edge, middle");
                assert_eq!(pixel(9, 20), BEHIND, "left of the rect");
                assert_eq!(pixel(30, 20), BEHIND, "right of the rect");
            },
        );
    }

    #[test]
    fn only_the_requested_corners_are_rounded() {
        let rect = RECT { left: 10, top: 10, right: 30, bottom: 30 };
        with_canvas(
            |dc| unsafe { fill_rounded(dc, rect, 6, Corners::TOP, FILL, BEHIND) },
            |pixel| {
                assert_eq!(pixel(10, 10), BEHIND, "top-left is rounded");
                assert_eq!(pixel(29, 10), BEHIND, "top-right is rounded");
                assert_eq!(pixel(10, 29), FILL, "bottom-left stays square");
                assert_eq!(pixel(29, 29), FILL, "bottom-right stays square");
            },
        );
    }

    #[test]
    fn an_edge_pixel_is_a_blend_between_the_fill_and_the_background() {
        let rect = RECT { left: 10, top: 10, right: 30, bottom: 30 };
        with_canvas(
            |dc| unsafe { fill_rounded(dc, rect, 8, Corners::ALL, FILL, BEHIND) },
            |pixel| {
                // (11, 12) lies on the arc of an 8px radius, so it is neither pure color.
                let edge = pixel(11, 12);
                assert_ne!(edge, FILL);
                assert_ne!(edge, BEHIND);
            },
        );
    }

    #[test]
    fn a_radius_larger_than_the_rect_clamps_and_stays_inside_it() {
        let rect = RECT { left: 10, top: 10, right: 30, bottom: 16 };
        with_canvas(
            |dc| unsafe { fill_rounded(dc, rect, 50, Corners::ALL, FILL, BEHIND) },
            |pixel| {
                assert_eq!(pixel(20, 12), FILL, "inside");
                assert_eq!(pixel(20, 9), BEHIND, "above");
                assert_eq!(pixel(20, 16), BEHIND, "below");
                assert_eq!(pixel(9, 12), BEHIND, "left");
                assert_eq!(pixel(30, 12), BEHIND, "right");
            },
        );
    }

    #[test]
    fn an_empty_or_inverted_rect_draws_nothing() {
        let inverted = RECT { left: 30, top: 30, right: 10, bottom: 10 };
        with_canvas(
            |dc| unsafe { fill_rounded(dc, inverted, 4, Corners::ALL, FILL, BEHIND) },
            |pixel| assert_eq!(pixel(20, 20), BEHIND),
        );
    }

    #[test]
    fn a_zero_radius_is_a_plain_fill() {
        let rect = RECT { left: 10, top: 10, right: 30, bottom: 30 };
        with_canvas(
            |dc| unsafe { fill_rounded(dc, rect, 0, Corners::ALL, FILL, BEHIND) },
            |pixel| assert_eq!(pixel(10, 10), FILL),
        );
    }
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --lib window::design -- --test-threads=1`
Expected: compile errors (`TAB_RADIUS`, `coverage`, `fill_rounded`, `Corners`, `radius_for` not found).

- [ ] **Step 3: Implement**

In `design/metrics.rs`, after `CONTROL_RADIUS`:

```rust
/// The corner radius of the rounded top of a tab.
pub(crate) const TAB_RADIUS: i32 = 8;
/// How far a sidebar row's hover and selection fill is inset from the panel's left and right edges.
pub(crate) const ROW_INSET_X: i32 = 4;
/// How far that fill is inset from the row's top and bottom, which leaves a gap between rows.
pub(crate) const ROW_INSET_Y: i32 = 1;
```

At the top of `design/round.rs` (above the tests):

```rust
//! Rounded fills drawn with GDI alone, so the tab strip and the sidebar need no Direct2D (which
//! the app loads only when a dialog opens). The body is plain `FillRect`; only the corner pixels
//! are computed, each one a blend of the shape color and the known color behind it.

use windows_sys::Win32::Foundation::RECT;
use windows_sys::Win32::Graphics::Gdi::{HDC, SetPixelV};

use crate::catppuccin::blend;
use crate::window::design::metrics::scale;
use crate::window::palette::Palette;
use crate::window::panel::fill;

/// Which corners of a rect are rounded.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Corners(u8);

impl Corners {
    pub(crate) const NONE: Corners = Corners(0);
    pub(crate) const TOP_LEFT: Corners = Corners(1);
    pub(crate) const TOP_RIGHT: Corners = Corners(2);
    pub(crate) const BOTTOM_LEFT: Corners = Corners(4);
    pub(crate) const BOTTOM_RIGHT: Corners = Corners(8);
    pub(crate) const TOP: Corners = Corners(3);
    pub(crate) const ALL: Corners = Corners(15);

    const fn has(self, corner: Corners) -> bool {
        self.0 & corner.0 != 0
    }
}

/// `at_96_dpi` scaled to `dpi`, or 0 in high contrast, which keeps square, unblended fills.
pub(crate) fn radius_for(palette: &Palette, at_96_dpi: i32, dpi: u32) -> i32 {
    if palette.high_contrast { 0 } else { scale(at_96_dpi, dpi) }
}

/// How much of the pixel (`x`, `y`) of a corner square lies inside the corner's arc, 0 to 255.
/// (0, 0) is the outermost pixel and the arc's center is `radius` pixels in from both edges. Each
/// pixel is sampled at 4x4 points. A radius of 0 has no corner, so it counts as fully inside.
pub(crate) fn coverage(x: i32, y: i32, radius: i32) -> u32 {
    if radius <= 0 {
        return 255;
    }
    let center = 8 * radius;
    let mut inside = 0;
    for j in 0..4 {
        for i in 0..4 {
            let dx = 8 * x + 2 * i + 1 - center;
            let dy = 8 * y + 2 * j + 1 - center;
            if dx * dx + dy * dy <= center * center {
                inside += 1;
            }
        }
    }
    (inside * 255 + 8) / 16
}

/// Fills `rect` with `color`, rounding the `corners` by `radius` pixels (clamped to half the
/// shorter side). `behind` is the flat color already under the shape, which the smoothed edge
/// pixels blend toward. An empty or inverted rect draws nothing.
pub(crate) unsafe fn fill_rounded(
    dc: HDC,
    rect: RECT,
    radius: i32,
    corners: Corners,
    color: u32,
    behind: u32,
) {
    let width = rect.right - rect.left;
    let height = rect.bottom - rect.top;
    if width <= 0 || height <= 0 {
        return;
    }
    let radius = radius.min(width / 2).min(height / 2).max(0);
    if radius == 0 || corners == Corners::NONE {
        unsafe { fill(dc, rect, color) };
        return;
    }
    let inset = |corner: Corners| if corners.has(corner) { radius } else { 0 };
    unsafe {
        let band = |top, bottom, left_inset, right_inset| {
            fill(
                dc,
                RECT {
                    left: rect.left + left_inset,
                    top,
                    right: rect.right - right_inset,
                    bottom,
                },
                color,
            );
        };
        band(
            rect.top,
            rect.top + radius,
            inset(Corners::TOP_LEFT),
            inset(Corners::TOP_RIGHT),
        );
        band(rect.top + radius, rect.bottom - radius, 0, 0);
        band(
            rect.bottom - radius,
            rect.bottom,
            inset(Corners::BOTTOM_LEFT),
            inset(Corners::BOTTOM_RIGHT),
        );
        for (corner, flip_x, flip_y) in [
            (Corners::TOP_LEFT, false, false),
            (Corners::TOP_RIGHT, true, false),
            (Corners::BOTTOM_LEFT, false, true),
            (Corners::BOTTOM_RIGHT, true, true),
        ] {
            if !corners.has(corner) {
                continue;
            }
            for y in 0..radius {
                for x in 0..radius {
                    let covered = coverage(x, y, radius);
                    if covered == 0 {
                        continue;
                    }
                    let px = if flip_x { rect.right - 1 - x } else { rect.left + x };
                    let py = if flip_y { rect.bottom - 1 - y } else { rect.top + y };
                    SetPixelV(dc, px, py, blend(color, behind, covered));
                }
            }
        }
    }
}
```

If `window::palette::Theme::ALL` or `Palette::for_theme` differ in name from the test's use, adapt the test to the real names without weakening it.

- [ ] **Step 4: Run to verify they pass**

Run: `cargo test --lib window::design -- --test-threads=1`
Expected: PASS. If `an_edge_pixel_is_a_blend_between_the_fill_and_the_background` fails because (11, 12) is fully inside or outside at radius 8, compute with `coverage` which pixel in the top-left 8x8 corner has coverage strictly between 1 and 254 and use that pixel (offset by the rect's left and top); keep the assertion.

- [ ] **Step 5: Lint and commit**

Run: `cargo fmt && cargo clippy --all-targets -- -D warnings`

```bash
git add src/window/design
git commit -m "feat: GDI rounded fills in design::round and the tab and row metrics"
```

---

### Task 2: Rounded tabs

**Files:**
- Modify: `src/window/group_strip.rs` (the `draw` function around lines 342-481), `docs/superpowers/specs/2026-10-01-rounded-tabs-rows-design.md` (one sentence, below)
- Test: the existing tests for `group_strip` (find the nearest paint test; if none paints into a DC, add one using the memory-DC pattern in the `design/round.rs` tests)

**Interfaces:**
- Consumes: `round::{fill_rounded, radius_for, Corners}`, `metrics::{TAB_RADIUS, CONTROL_RADIUS}`, existing `draw` locals (`tab`, `background`, `selected`, `tab_hovered`, `close`, `close_hovered`, `palette`, `dpi`, `input.accent`).
- Produces: nothing new.

- [ ] **Step 1: Write the failing test**

Add a test (in the group_strip tests, matching how they build `StripPaint` and a DC; reuse an existing helper if there is one) asserting, for the active tab at DPI 96: the pixel at the tab's outermost top-left corner equals `palette.strip_background`, a pixel in the middle of the tab equals `palette.active_tab_background()`, and the pixel at the tab's bottom-left corner equals `palette.active_tab_background()` (the bottom stays square, flush with the editor). Run it and see it fail on the corner pixel (it is `active_tab_background()` today).

- [ ] **Step 2: Implement**

In `group_strip.rs` `draw`, import `design::metrics::{CONTROL_RADIUS, TAB_RADIUS}` and `design::round::{Corners, fill_rounded, radius_for}`. Replace `fill(dc, tab, background);` with:

```rust
            let tab_radius = radius_for(&palette, TAB_RADIUS, dpi);
            fill_rounded(
                dc,
                tab,
                tab_radius,
                if selected || tab_hovered { Corners::TOP } else { Corners::NONE },
                background,
                palette.strip_background,
            );
```

The idle tab keeps `Corners::NONE`, which is a plain fill of the strip color. Change the accent bar so it does not stick out past the rounded corners: its rect becomes `Rect::new(tab.left + tab_radius, tab.top, tab.right - tab_radius, tab.top + scale(2, dpi))`. Replace the close button's `fill(dc, close.centered_square(scale(24, dpi)), ...)` with `fill_rounded(dc, close.centered_square(scale(24, dpi)), radius_for(&palette, CONTROL_RADIUS, dpi), Corners::ALL, <the same pressed/hover color>, background)`; `background` is the tab's fill, which is what is behind the close button.

All of this stays inside the existing clip (`IntersectClipRect` to `layout.tabs`), so a scrolled tab's corner pixels are clipped with it.

In the spec, replace the sentence "The 2px top accent bar (shown only with several groups) keeps its place and is clipped to the rounded outline by being drawn first and then covered at the corners." with "The 2px top accent bar (shown only with several groups) is inset by the tab radius on each side, so it sits on the flat part of the top edge."

- [ ] **Step 3: Run**

Run: `cargo clippy --all-targets -- -D warnings`, then `cargo test --lib window::group_strip -- --test-threads=1` and the nearest tab-strip test module under `window::main_window::tests` (`-- --test-threads=1`). Expected: PASS; update any existing pixel expectation that read a tab corner as the tab fill, to the new strip-colored corner, without weakening other assertions.

- [ ] **Step 4: Commit**

```bash
git add src docs
git commit -m "feat: rounded top corners on the active and hovered tab"
```

---

### Task 3: Rounded, inset sidebar row fills

**Files:**
- Modify: `src/window/row_list.rs` (`paint`, ~lines 290-330), every caller of `row_list::paint` (`src/window/notebook_view/paint.rs` at ~683 and ~760, plus any in `favorites_view.rs` or elsewhere; the compiler lists them)
- Test: `src/window/row_list.rs` tests (~line 574)

**Interfaces:**
- Consumes: `round::{fill_rounded, radius_for, Corners}`, `metrics::{CONTROL_RADIUS, ROW_INSET_X, ROW_INSET_Y, scale}`.
- Produces: `row_list::paint(hdc, area, state, palette, focused, behind: u32, dpi: u32, draw_row)`. `behind` is the color the caller filled `area` with just before calling.

- [ ] **Step 1: Write the failing test**

Extend `painting_draws_only_the_rows_in_view_with_the_selection_and_hover_backgrounds`: call the new signature with `behind = palette.editor_background` and `dpi = 96`, and add assertions that the selected row's fill is inset: the pixel at `(area.left + 1, row_top + 13)` (inside the 4px inset) equals `editor_background`, the pixel at `(area.left + 12, row_top + 13)` equals the selection color, the pixel at `(area.left + 12, row_top)` (the 1px top gap) equals `editor_background`, and the outermost corner of the fill (`area.left + 4`, `row_top + 1`) equals `editor_background`. Add a second test with `palette.high_contrast = true` asserting that same corner pixel `(area.left + 4, row_top + 1)` equals the selection color, and that `(area.left + 1, row_top)` does too (the fill covers the whole row in high contrast). Run and see them fail to compile (signature) and then on the inset.

- [ ] **Step 2: Implement**

In `row_list::paint`, compute once before the loop:

```rust
        let radius = radius_for(palette, CONTROL_RADIUS, dpi);
        let inset_x = scale(ROW_INSET_X, dpi);
        let inset_y = scale(ROW_INSET_Y, dpi);
```

In the loop, after building `rect`, build `let backplate = RECT { left: rect.left + inset_x, top: rect.top + inset_y, right: rect.right - inset_x, bottom: rect.bottom - inset_y };` and replace the two `fill(hdc, rect, ...)` calls by `fill_rounded(hdc, backplate, radius, Corners::ALL, <the same color>, behind)`. In high contrast use `rect` instead of `backplate` (`if palette.high_contrast { rect } else { backplate }`), so system-color rows look as before. `draw_row` still receives the unchanged `rect`.

Update every caller to pass `behind` (the color it filled the list area with immediately before) and `dpi` (the caller already has one in scope for its own scaling; use it). Look at each call site to find that fill, and do not guess: if a caller does not fill the area first, tell the controller instead of choosing a color.

Update the doc comment of `paint` to say the fills are rounded and inset, and why `behind` is needed.

- [ ] **Step 3: Run**

Run: `cargo clippy --all-targets -- -D warnings`, then `cargo test --lib window::row_list -- --test-threads=1`, `window::notebook_view`, `window::favorites_view` and `window::main_window::tests::sidebar_layout` (each with `-- --test-threads=1`). Expected: PASS; update pixel expectations that read a row fill near the left or right edge, without weakening others.

- [ ] **Step 4: Commit**

```bash
git add src
git commit -m "feat: rounded, inset hover and selection fills in sidebar rows"
```
