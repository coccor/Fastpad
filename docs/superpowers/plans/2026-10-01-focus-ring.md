# One Keyboard Focus Ring Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Draw the chrome's keyboard focus indicators (activity bar buttons, notebook header rows) as a 2px accent ring with rounded corners, in GDI.

**Architecture:** `design/round.rs` gets `ring_coverage` (corner-pixel coverage of a ring band), `stroke_ring` (straight edges as fills, corner pixels blended over the pixel already there) and `paint_focus_ring`, which applies the ring metrics and the accent color to a control's rect. The activity bar and the notebook view call `paint_focus_ring` in place of `DrawFocusRect` and `paint_outline`.

**Tech Stack:** Rust 2024, windows-sys GDI (`FillRect`, `GetPixel`, `SetPixelV`), `catppuccin::blend`, `design::metrics`.

**Spec:** `docs/superpowers/specs/2026-10-01-focus-ring-design.md`

## Global Constraints

- No Direct2D, no new library loads, no allocation; nothing new runs before the first editable frame.
- Ring metrics: width `FOCUS_RING` (2), gap `FOCUS_GAP` (1), radius `CONTROL_RADIUS` (4), each scaled through `design::metrics::scale`; color `palette.accent`. In high contrast (`palette.high_contrast`) the radius is 0 through `radius_for` (a square ring).
- Layout, hit-testing, row heights and every other color stay as they are. The dialogs' Direct2D rings, the drop-band outline in high contrast (`paint_band`'s use of `paint_outline`) and any other use of `accent` are out of scope.
- A `GetPixel` result of `CLR_INVALID` (0xFFFF_FFFF) means the pixel could not be read: skip that pixel.
- Colors are `COLORREF` (`0x00BBGGRR`). No attribution lines in commit messages; never skip hooks.
- Testing practice: compile with `cargo clippy --all-targets -- -D warnings`, run only the targeted tests with `-- --test-threads=1`; the controller runs the full suite once at the end.

## Review Focus

- A rect smaller than twice the ring width, or an empty or inverted rect: draws nothing outside the rect and never panics.
- A radius larger than half the shorter side, or a width larger than the radius: both clamp.
- The ring over a selected or hovered row (a non-flat background): corner pixels blend with the pixel underneath, not with a fixed color.
- DPI 192: width 4, gap 2, radius 8; corners stay symmetric.
- The unfocused panel draws no ring.

---

### Task 1: The ring helper

**Files:**
- Modify: `src/window/design/round.rs`

**Interfaces:**
- Consumes: `coverage`-style 4x4 sampling already in this file (`coverage(x, y, radius)`), `blend`, `radius_for`, `scale`, `FOCUS_RING`, `FOCUS_GAP`, `CONTROL_RADIUS` (in `design::metrics`), `Palette` (fields `accent`, `high_contrast`), `crate::window::panel::fill`.
- Produces:
  - `round::ring_coverage(x: i32, y: i32, radius: i32, width: i32) -> u32` (0..=255; requires `radius > 0` and `0 < width <= radius`).
  - `round::stroke_ring(dc: HDC, rect: RECT, radius: i32, width: i32, color: u32)` (`unsafe fn`, `pub(crate)`).
  - `round::paint_focus_ring(dc: HDC, control: RECT, palette: &Palette, dpi: u32)` (`unsafe fn`, `pub(crate)`).

- [ ] **Step 1: Write the failing tests**

In the `tests` module of `design/round.rs` (it has `with_canvas`, `BEHIND`, `FILL`, `BORDER`), add:

```rust
    #[test]
    fn the_ring_band_is_empty_outside_and_deeper_than_its_width() {
        // Break caught: a ring that fills the whole corner, or a band that is not hollow.
        let radius = 8;
        let width = 2;
        assert_eq!(ring_coverage(0, 0, radius, width), 0, "outside the outer arc");
        assert_eq!(
            ring_coverage(radius - 1, radius - 1, radius, width),
            0,
            "deep inside, past the band"
        );
        // Along the middle of the top edge of the corner square the band is the top `width` rows.
        assert_eq!(ring_coverage(radius - 1, 0, radius, width), 255);
        assert_eq!(ring_coverage(radius - 1, 1, radius, width), 255);
        assert_eq!(ring_coverage(radius - 1, 2, radius, width), 0);
    }

    #[test]
    fn ring_coverage_is_symmetric_across_the_diagonal() {
        let (radius, width) = (8, 2);
        for y in 0..radius {
            for x in 0..radius {
                assert_eq!(
                    ring_coverage(x, y, radius, width),
                    ring_coverage(y, x, radius, width),
                    "({x}, {y})"
                );
            }
        }
    }

    #[test]
    fn a_ring_draws_its_edges_and_leaves_the_inside_and_outside_alone() {
        let rect = RECT { left: 10, top: 10, right: 30, bottom: 30 };
        with_canvas(
            |dc| unsafe { stroke_ring(dc, rect, 6, 2, FILL) },
            |pixel| {
                assert_eq!(pixel(20, 10), FILL, "top edge");
                assert_eq!(pixel(20, 11), FILL, "top edge, second row");
                assert_eq!(pixel(20, 12), BEHIND, "just inside the ring");
                assert_eq!(pixel(10, 20), FILL, "left edge");
                assert_eq!(pixel(29, 20), FILL, "right edge");
                assert_eq!(pixel(20, 29), FILL, "bottom edge");
                assert_eq!(pixel(20, 20), BEHIND, "interior");
                assert_eq!(pixel(10, 10), BEHIND, "outer corner pixel");
                assert_eq!(pixel(9, 20), BEHIND, "left of the rect");
                assert_eq!(pixel(20, 30), BEHIND, "below the rect");
            },
        );
    }

    #[test]
    fn a_ring_corner_pixel_blends_with_the_pixel_underneath() {
        // Break caught: a corner blended toward a fixed color, which shows a fringe over a
        // selected or hovered row.
        let rect = RECT { left: 10, top: 10, right: 30, bottom: 30 };
        let sample = |under: u32| {
            let mut result = 0;
            with_canvas(
                |dc| unsafe {
                    crate::window::panel::fill(
                        dc,
                        RECT { left: 0, top: 0, right: 40, bottom: 40 },
                        under,
                    );
                    stroke_ring(dc, rect, 8, 2, FILL);
                },
                |pixel| result = pixel(11, 12),
            );
            result
        };
        // (11, 12) lies on the outer arc of an 8px radius at (10, 10): partly covered.
        let over_dark = sample(0x0000_0000);
        let over_light = sample(0x00ff_ffff);
        assert_ne!(over_dark, over_light);
        assert_ne!(over_dark, FILL);
        assert_ne!(over_light, FILL);
    }

    #[test]
    fn a_square_ring_has_a_hard_corner() {
        let rect = RECT { left: 10, top: 10, right: 30, bottom: 30 };
        with_canvas(
            |dc| unsafe { stroke_ring(dc, rect, 0, 2, FILL) },
            |pixel| {
                assert_eq!(pixel(10, 10), FILL, "corner");
                assert_eq!(pixel(11, 11), FILL, "corner, second pixel");
                assert_eq!(pixel(12, 12), BEHIND, "inside");
                assert_eq!(pixel(29, 29), FILL, "opposite corner");
            },
        );
    }

    #[test]
    fn a_ring_in_a_tiny_or_empty_rect_stays_inside_it() {
        let tiny = RECT { left: 10, top: 10, right: 13, bottom: 13 };
        with_canvas(
            |dc| unsafe { stroke_ring(dc, tiny, 6, 4, FILL) },
            |pixel| {
                assert_eq!(pixel(9, 9), BEHIND);
                assert_eq!(pixel(13, 13), BEHIND);
                assert_eq!(pixel(13, 10), BEHIND);
                assert_eq!(pixel(10, 13), BEHIND);
            },
        );
        let inverted = RECT { left: 30, top: 30, right: 10, bottom: 10 };
        with_canvas(
            |dc| unsafe { stroke_ring(dc, inverted, 4, 2, FILL) },
            |pixel| assert_eq!(pixel(20, 20), BEHIND),
        );
    }

    #[test]
    fn the_focus_ring_uses_the_accent_color_inset_by_the_gap() {
        use crate::window::palette::{Palette, Theme};
        let mut palette = Palette::for_theme(Theme::ALL[0], false);
        palette.accent = FILL;
        let control = RECT { left: 10, top: 10, right: 30, bottom: 30 };
        with_canvas(
            |dc| unsafe { paint_focus_ring(dc, control, &palette, 96) },
            |pixel| {
                assert_eq!(pixel(20, 10), BEHIND, "the gap row is left alone");
                assert_eq!(pixel(20, 11), FILL, "ring's first row, one inside the control");
                assert_eq!(pixel(20, 12), FILL, "ring's second row");
                assert_eq!(pixel(20, 13), BEHIND, "inside the ring");
            },
        );
    }
```

Do not use `Theme::ALL[0]` if the test module already imports a different path; match how the existing `radius_for` test in this file builds its palette.

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --lib window::design::round -- --test-threads=1`
Expected: compile errors (`ring_coverage`, `stroke_ring`, `paint_focus_ring` not found).

- [ ] **Step 3: Implement**

First generalise the sampling: change `coverage` so it delegates to a private function that takes the arc's center and radius separately, with no change in behaviour (`coverage` keeps its signature and its existing tests keep passing):

```rust
/// How much of the pixel (`x`, `y`) lies within `radius` pixels of the point (`center`, `center`),
/// 0 to 255, sampled at 4x4 points.
fn arc_coverage(x: i32, y: i32, center: i32, radius: i32) -> u32 {
    let center = 8 * center;
    let radius = 8 * radius;
    let mut inside = 0;
    for j in 0..4 {
        for i in 0..4 {
            let dx = 8 * x + 2 * i + 1 - center;
            let dy = 8 * y + 2 * j + 1 - center;
            if dx * dx + dy * dy <= radius * radius {
                inside += 1;
            }
        }
    }
    (inside * 255 + 8) / 16
}
```

`coverage(x, y, radius)` becomes `if radius <= 0 { 255 } else { arc_coverage(x, y, radius, radius) }`. Then add:

```rust
/// How much of the corner-square pixel (`x`, `y`) lies in a ring of `width` pixels just inside
/// an arc of `radius` pixels: the outer arc's coverage minus the inner arc's. (0, 0) is the
/// outermost pixel. Requires `radius > 0` and `0 < width <= radius`.
pub(crate) fn ring_coverage(x: i32, y: i32, radius: i32, width: i32) -> u32 {
    let outer = arc_coverage(x, y, radius, radius);
    let inner = if radius > width { arc_coverage(x, y, radius, radius - width) } else { 0 };
    outer.saturating_sub(inner)
}

/// Draws a `width`-pixel ring just inside `rect`, rounded by `radius`. Straight edges are plain
/// fills; each corner pixel is `color` blended over the pixel already there, so the ring sits
/// correctly on any background. `radius` and `width` clamp to what the rect can hold, and an
/// empty or inverted rect draws nothing.
pub(crate) unsafe fn stroke_ring(dc: HDC, rect: RECT, radius: i32, width: i32, color: u32) {
    let w = rect.right - rect.left;
    let h = rect.bottom - rect.top;
    if w <= 0 || h <= 0 || width <= 0 {
        return;
    }
    let radius = radius.min(w / 2).min(h / 2).max(0);
    let width = width.min(w / 2).min(h / 2).max(1).min(if radius > 0 { radius } else { i32::MAX });
    let strip = |left, top, right, bottom| {
        if right > left && bottom > top {
            unsafe { fill(dc, RECT { left, top, right, bottom }, color) };
        }
    };
    // The straight edges stop short of the corner squares, which are drawn pixel by pixel.
    strip(rect.left + radius, rect.top, rect.right - radius, rect.top + width);
    strip(rect.left + radius, rect.bottom - width, rect.right - radius, rect.bottom);
    strip(rect.left, rect.top + radius, rect.left + width, rect.bottom - radius);
    strip(rect.right - width, rect.top + radius, rect.right, rect.bottom - radius);
    if radius == 0 {
        // A square ring: the corner squares are the `width` x `width` blocks.
        strip(rect.left, rect.top, rect.left + width, rect.top + width);
        strip(rect.right - width, rect.top, rect.right, rect.top + width);
        strip(rect.left, rect.bottom - width, rect.left + width, rect.bottom);
        strip(rect.right - width, rect.bottom - width, rect.right, rect.bottom);
        return;
    }
    for (flip_x, flip_y) in [(false, false), (true, false), (false, true), (true, true)] {
        for y in 0..radius {
            for x in 0..radius {
                let covered = ring_coverage(x, y, radius, width);
                if covered == 0 {
                    continue;
                }
                let px = if flip_x { rect.right - 1 - x } else { rect.left + x };
                let py = if flip_y { rect.bottom - 1 - y } else { rect.top + y };
                let under = unsafe { GetPixel(dc, px, py) };
                if under == CLR_INVALID {
                    continue;
                }
                unsafe { SetPixelV(dc, px, py, blend(color, under, covered)) };
            }
        }
    }
}

/// The keyboard focus ring around `control`: `FOCUS_RING` wide in the accent color, with its outer
/// edge `FOCUS_GAP` inside the control's rect, rounded by `CONTROL_RADIUS` (square in high
/// contrast), all scaled to `dpi`.
pub(crate) unsafe fn paint_focus_ring(dc: HDC, control: RECT, palette: &Palette, dpi: u32) {
    let gap = scale(FOCUS_GAP, dpi);
    let rect = RECT {
        left: control.left + gap,
        top: control.top + gap,
        right: control.right - gap,
        bottom: control.bottom - gap,
    };
    unsafe {
        stroke_ring(
            dc,
            rect,
            radius_for(palette, CONTROL_RADIUS, dpi),
            scale(FOCUS_RING, dpi),
            palette.accent,
        );
    }
}
```

Add `GetPixel` to the `windows_sys::Win32::Graphics::Gdi` import, `CLR_INVALID` from the same module (it is exported from `windows_sys::Win32::Graphics::Gdi`; if it is not, define `const CLR_INVALID: u32 = 0xFFFF_FFFF;`), and `CONTROL_RADIUS, FOCUS_GAP, FOCUS_RING` to the `design::metrics` import. `stroke_ring` and `paint_focus_ring` are unused until Task 2: put `#[allow(dead_code, reason = "used by the activity bar and the notebook view in the next task")]` on `ring_coverage`, `stroke_ring` and `paint_focus_ring` (the tests use them, so only the non-test build needs it) and remove the attributes in Task 2.

If a test's hand-computed pixel does not match (the 4x4 sampling is exact at integer pixel edges but a pixel that straddles the arc is partly covered), recompute with `ring_coverage` which pixel is strictly between 1 and 254 and use that pixel; keep the assertion that it differs from both colors.

- [ ] **Step 4: Run to verify they pass**

Run: `cargo test --lib window::design::round -- --test-threads=1`
Expected: PASS, including every earlier round.rs test.

- [ ] **Step 5: Lint and commit**

Run: `cargo fmt && cargo clippy --all-targets -- -D warnings`

```bash
git add src/window/design
git commit -m "feat: GDI focus ring in design::round"
```

---

### Task 2: Use the ring in the activity bar and the notebook view

**Files:**
- Modify: `src/window/activity_bar.rs` (`paint_keyboard_focus`, ~line 509, and its one call at ~351), `src/window/notebook_view/paint.rs` (the `paint.focused` block, ~line 940-957), `src/window/design/round.rs` (remove the three `allow(dead_code)`), `docs/superpowers/specs/2026-10-01-focus-ring-design.md` (two sentences, below)
- Test: the nearest existing notebook paint test module (`src/window/notebook_view/tests.rs`)

**Interfaces:**
- Consumes: `round::paint_focus_ring(dc, control, palette, dpi)`.
- Produces: `activity_bar::paint_keyboard_focus(bar, hdc, palette: &Palette)` (a new `palette` parameter).

- [ ] **Step 1: Write the failing test**

In the notebook view tests, add a test that paints a view with keyboard focus on the root row (`Cursor::Root`, a notebook open, `paint.focused = true`) into a memory DC and checks that the pixel in the middle of the top edge of the root row's rect, one gap-pixel inside it (at 96 DPI `rect.top + 1`, `rect.left + rect.width / 2`... use the row rect the layout reports), equals `palette.accent`, while the same view with `paint.focused = false` has no accent pixel there. Reuse the harness the nearest existing notebook paint test uses (it already builds a `ViewPaint` and a DC). Run it and see it fail (today that pixel is `selection_background` or the row fill).

- [ ] **Step 2: Implement**

Notebook view: in the `paint.focused` block replace

```rust
            if let Some(rect) = outlined {
                paint_outline(dc, rect, palette.selection_background, dpi);
            }
```

with `paint_focus_ring(dc, rect, &palette, dpi)` (import `design::round::paint_focus_ring`; keep whatever borrow form `palette` already has in that scope). `paint_outline` stays, because `paint_band` still calls it.

Activity bar: give `paint_keyboard_focus` a `palette: &Palette` parameter, call it as `paint_keyboard_focus(bar, dc, &palette)` from `paint` (the `palette` local is already in scope there), and replace its body from the `let rect = ...inset(...)` to the `DrawFocusRect` call with:

```rust
    let rect = button_rects(client, dpi)[super::side_panel::bar_focus(main)];
    unsafe { paint_focus_ring(hdc, rect, palette, dpi) };
```

The old `scale(3, dpi)` inset and the `DrawFocusRect` call are removed. Update the doc comment ("Draws the keyboard focus rectangle") to say it draws the focus ring. Remove the three `allow(dead_code)` attributes in `round.rs`.

The `paint_keyboard_focus` wrapper needs a window handle (`GetFocus`, `GetParent`), so it has no unit test of its own: the drawing is covered by the `round.rs` tests and the build. In the spec, replace the sentence "Activity bar: a test that `paint_keyboard_focus` draws the accent color on the focused button's edge and nothing in the old dotted rectangle position inside it (use the nearest existing activity bar paint test as the harness)." with "Activity bar: `paint_keyboard_focus` is bound to a window handle, so its drawing is covered by the helper tests above and by the build." Also, in section 4.1, replace "For a radius of 0 it is 255 for the pixels inside the `width` band and 0 elsewhere, which keeps one code path." with "It requires a radius above 0; a square ring (radius 0, as in high contrast) is drawn as four plain strips and has no corner pixels."

- [ ] **Step 3: Run**

Run: `cargo fmt && cargo clippy --all-targets -- -D warnings`, then `cargo test --lib window::notebook_view -- --test-threads=1`, `window::activity_bar`, `window::design::round` and `window::main_window::tests::sidebar_layout` (each with `-- --test-threads=1`). Expected: PASS. Update an existing notebook pixel expectation only where it read the old `selection_background` outline on a focused row, without weakening other assertions.

- [ ] **Step 4: Commit**

```bash
git add src docs
git commit -m "feat: accent focus ring on the activity bar and the notebook header rows"
```
