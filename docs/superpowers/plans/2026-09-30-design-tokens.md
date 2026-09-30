# Design Tokens Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add FastPad's design-token layer (one `scale`, shared metrics, palette roles, a type ramp, and contrast tests) with no visible change.

**Architecture:** A new `src/window/design/` module owns the shared metrics, the text-style table and a test-only contrast helper. `Palette` keeps its flat `Copy` shape and gains six role fields. Existing call sites move onto the tokens only where the value is identical today. Everything is `const` or `static`, so startup cost is unchanged.

**Tech Stack:** Rust 2024, `windows-sys` (GDI), in-crate unit tests run with `cargo test --lib`.

**Spec:** `docs/superpowers/specs/2026-09-30-design-tokens-design.md`

## Global Constraints

- **No visible change.** Every value introduced equals the value in use now or is not yet consumed.
- **Startup:** everything is `const` or `static`; no allocation and no theme query before first paint. Font creation happens at the same points as today. Paint never triggers a load (`chrome.rs`).
- **Old constants are removed when replaced.** No aliases and no re-exports left behind.
- **Existing `Palette` fields are unchanged and not renamed.**
- **No new settings and no `fastpad.ini` changes.**
- **Testing practice:** compile with Clippy, run only targeted tests, and run the full suite once at final review. Integration tests under `tests/windows/` need `native/out` DLLs and `--test-threads=1`; this plan uses only in-crate unit tests until Task 6.
- **Formatting:** CI runs `cargo fmt --check`, so run `cargo fmt` before every commit.
- **Compile-check command** (used throughout): `cargo clippy --all-targets -- -D warnings`

## Deviations from the spec, found while reading the code

The plan follows these, and Task 6 amends the spec to match.

1. **The two `scale` copies are not identical.** `titlebar::scale` treats DPI 0 as 1 (giving near-zero sizes), while `panel::scale` treats DPI 0 as 96. The single `metrics::scale` uses the 96 fallback. It is the only behaviour change in this plan, and it only affects a DPI of 0.
2. **Shared heights.** Only `PANEL_HEADER` (38) and `SIDEBAR_ROW` (26) are shared by design and move to `metrics`. The field height 28, bar height 36 and command-palette row 26 are repeated but diverge in later steps (palette field and row become 32), so consolidating them would have to be undone.
3. **Unused items are omitted** because unused `pub(crate)` items fail `-D warnings`: no `STROKE`, no `OVERLAY_RADIUS`, no `Caption` text style. Step 2 and step 3 add them when first used. The `GRID` constant exists only under `#[cfg(test)]`.
4. **Contrast helpers already exist** as private functions in `palette.rs` tests. Task 3 moves them into `design/contrast.rs` instead of writing new ones.
5. **Contrast failures are recorded, not fixed.** To keep the "no visible change" rule, any existing pair that falls short goes into a known-exceptions list with its measured ratio, and the list is reported to the user. Corrections belong to step 4.

## Review Focus

Failure modes the spec implies that no single feature task would otherwise test. Each is pinned by a test in the task noted.

1. **DPI 0 reaching `scale`** should give the 96 fallback, never a zero-size layout. (Task 1)
2. **High contrast must never blend.** The new roles map to system colors in the high-contrast palette. (Task 4)
3. **A known exception that starts passing** must fail the test until it is removed, so the exception list can't rot. (Task 3)
4. **Fonts at unusual DPIs** (0, 144, 192) must still be created, non-null and sized proportionally. (Task 5)
5. **Palette/theme index alignment** (`PALETTES` versus `Theme::ALL`) must survive the new fields; the existing distinct-palette test keeps covering it and is re-run in Task 4. (Task 4)

---

## File Structure

| File | Responsibility |
|---|---|
| `src/window/design/mod.rs` (create) | Module root; declares `metrics`, `type_ramp`, and test-only `contrast`. |
| `src/window/design/metrics.rs` (create) | The single `scale`, shared 96-DPI constants, and their tests. |
| `src/window/design/type_ramp.rs` (create) | `TextStyle`, its size/weight table, `create` (wraps `create_ui_font`). |
| `src/window/design/contrast.rs` (create, test-only) | WCAG luminance and contrast ratio. |
| `src/window/palette.rs` (modify) | Six new role fields, their per-theme values, the palette-wide contrast test. |
| `src/window/mod.rs` (modify) | Declare `pub(crate) mod design;`. |
| `src/window/titlebar.rs`, `panel.rs` (modify) | Delete their `scale` copies. |
| `src/window/soft_paint.rs` (modify) | Delete radius and focus constants. |
| ~35 files that import `scale` (modify) | Import from `design::metrics`. Listed in Task 1. |
| `src/window/{about,settings_dialog,side_panel,titlebar}.rs` (modify) | Create text fonts through the ramp. |
| `docs/superpowers/specs/2026-09-30-design-tokens-design.md` (modify) | Amend for the deviations above. |

---

### Task 1: The `design` module and a single `scale`

**Files:**
- Create: `src/window/design/mod.rs`, `src/window/design/metrics.rs`
- Modify: `src/window/mod.rs`, `src/window/titlebar.rs:207-210`, `src/window/panel.rs:25-28`, and every file that imports `scale` (see Step 6)

**Interfaces:**
- Consumes: nothing.
- Produces: `crate::window::design::metrics::scale(value: i32, dpi: u32) -> i32` (`const fn`, `pub(crate)`). DPI 0 is treated as 96.

- [ ] **Step 1: Create the module root**

Create `src/window/design/mod.rs`:

```rust
//! FastPad's design tokens: the shared metrics, the type ramp, and (in tests) the contrast
//! helpers that guard the palettes. One owner per decision, so a look can change in one place.

pub(crate) mod metrics;
```

In `src/window/mod.rs`, add `pub(crate) mod design;` in alphabetical position, after `pub(crate) mod copy_host;` (line 6):

```rust
pub(crate) mod copy_host;
pub(crate) mod design;
pub(crate) mod document_store;
```

- [ ] **Step 2: Write the failing tests**

Create `src/window/design/metrics.rs` with only the tests first:

```rust
//! Sizes shared by more than one surface, in pixels at 96 DPI, and the one `scale` that turns
//! them into device pixels.

#[cfg(test)]
mod tests {
    use super::scale;

    #[test]
    fn scale_is_the_identity_at_96_dpi() {
        assert_eq!(scale(26, 96), 26);
        assert_eq!(scale(0, 96), 0);
    }

    #[test]
    fn scale_rounds_half_up_at_common_dpis() {
        // Break caught: a layout that drifts by a pixel per row at 125 % and 150 %.
        assert_eq!(scale(12, 120), 15); // 14.5 + 0.5 = 15
        assert_eq!(scale(26, 144), 39); // 39.0
        assert_eq!(scale(10, 192), 20);
        assert_eq!(scale(1, 144), 2); // 1.5 rounds up
    }

    #[test]
    fn a_dpi_of_zero_falls_back_to_96_instead_of_collapsing_the_layout() {
        // Break caught: `GetDpiForWindow` returns 0 for a bad handle; the old titlebar copy treated
        // that as a DPI of 1 and scaled every size to about zero.
        assert_eq!(scale(26, 0), 26);
        assert_eq!(scale(44, 0), 44);
    }
}
```

- [ ] **Step 3: Run the test to see it fail**

Run: `cargo test --lib window::design::metrics`
Expected: compile error `cannot find function scale in module super`.

- [ ] **Step 4: Write `scale`**

Insert above the `#[cfg(test)]` block in `src/window/design/metrics.rs`:

```rust
/// `value` (pixels at 96 DPI) scaled to `dpi`, rounded half up. A DPI of 0, which Windows returns
/// for a bad handle, counts as 96.
pub(crate) const fn scale(value: i32, dpi: u32) -> i32 {
    let dpi = if dpi == 0 { 96 } else { dpi };
    ((value as i64 * dpi as i64 + 48) / 96) as i32
}
```

- [ ] **Step 5: Run the test to see it pass**

Run: `cargo test --lib window::design::metrics`
Expected: 3 passed.

- [ ] **Step 6: Delete the two old copies and repoint every importer**

Delete the `scale` function from `src/window/titlebar.rs` (lines 207-210) and from `src/window/panel.rs` (lines 25-28).

Then repoint importers. First the mechanical part, which handles inline paths and single-item imports:

```bash
cd /d/Projects/FastPad
grep -rlE 'window::(panel|titlebar)::scale\b' src tests | xargs sed -i -E 's/crate::window::(panel|titlebar)::scale\b/crate::window::design::metrics::scale/g'
```

Then edit these grouped imports by hand. For each, remove `scale` from the group and add `use crate::window::design::metrics::scale;` (files under `src/window/` may write it as `use super::design::metrics::scale;`). Keep each file's other imports as they are:

| File | Old import |
|---|---|
| `src/window/activity_bar.rs:8` | `use crate::window::panel::{fill, scale};` |
| `src/window/about.rs:11` | `use super::panel::{inset, scale, text_height};` |
| `src/window/tab_drag.rs:13` | `use crate::window::titlebar::{Point, scale};` |
| `src/window/split_tree.rs:4` | `use super::titlebar::{Point, Rect, scale};` |
| `src/window/menu_band.rs:8` | `use crate::window::panel::{fill, scale};` |
| `src/window/side_panel.rs:20` | `use crate::window::panel::{create_child, fill, scale};` |
| `src/window/shortcuts_page.rs:7` | `use super::panel::{inset, scale};` |
| `src/window/settings_dialog.rs:13` | `use super::panel::{inset, scale};` |
| `src/window/search_view/paint.rs:11` | `use crate::window::panel::{fill, inset, scale};` |
| `src/window/search_view/edits.rs:11` | `use crate::window::panel::{create_child, fill, scale, text_height};` |
| `src/window/inline_name.rs:17` | `use crate::window::panel::{create_child, scale};` |
| `src/window/favorites_view.rs:11` | `use crate::window::panel::{fill, scale};` |
| `src/window/find_bar.rs:287` | `use crate::window::panel::{create_child, create_panel, fill, inset, scale, text_height};` |
| `src/window/command_palette.rs:10` | `use crate::window::panel::{create_child, create_panel, fill, inset, scale, text_height};` |
| `src/window/option_toggles.rs:7` | `use crate::window::panel::{fill, scale};` |
| `src/window/notebook_view/paint.rs:15` | `use crate::window::panel::{fill, inset, scale};` |

Example for `favorites_view.rs`:

```rust
use crate::window::design::metrics::scale;
use crate::window::panel::fill;
```

`titlebar.rs` and `panel.rs` use `scale` internally, so give each `use super::design::metrics::scale;`.

Files that reach `scale` some other way (for example `settings_dialog/layout.rs`, `settings_dialog/painting.rs`, `search_view/*`) show up as compile errors in the next step. Fix each the same way.

- [ ] **Step 7: Compile, fix the remaining import sites, then run the window unit tests once**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: PASS after fixing any `cannot find function scale` errors and any now-unused `scale` import warnings. Repeat until clean.

Run: `cargo fmt && cargo test --lib window::`
Expected: all in-crate `window::` tests pass. (One broad run is justified because this task touches about 35 files. Do not run `tests/windows/` yet.)

- [ ] **Step 8: Commit**

```bash
git add -A src
git commit -m "refactor: one scale in design::metrics (DPI 0 now falls back to 96)"
```

---

### Task 2: Shared metrics and radii

**Files:**
- Modify: `src/window/design/metrics.rs`, `src/window/soft_paint.rs:37-41`, `src/window/side_panel.rs:55`, `src/window/notebook_layout.rs:7,11`, `src/window/favorites_view.rs:31-32`, plus every user of the removed names

**Interfaces:**
- Consumes: `metrics::scale` from Task 1.
- Produces, all `pub(crate) const i32` in `crate::window::design::metrics`, in pixels at 96 DPI: `CONTROL_RADIUS = 4`, `FOCUS_RING = 2`, `FOCUS_GAP = 1`, `PANEL_HEADER = 38`, `SIDEBAR_ROW = 26`. Test-only: `GRID: i32 = 4`.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `src/window/design/metrics.rs`:

```rust
    use super::{CONTROL_RADIUS, FOCUS_GAP, FOCUS_RING, GRID, PANEL_HEADER, SIDEBAR_ROW};

    #[test]
    fn shared_metrics_keep_the_values_the_old_constants_had() {
        // Break caught: a "no visible change" refactor that shifts a control by a pixel.
        assert_eq!(CONTROL_RADIUS, 4); // was soft_paint::RADIUS_AT_96_DPI
        assert_eq!(FOCUS_RING, 2); // was soft_paint::FOCUS_WIDTH_AT_96_DPI
        assert_eq!(FOCUS_GAP, 1); // was soft_paint::FOCUS_GAP_AT_96_DPI
        assert_eq!(PANEL_HEADER, 38); // was side_panel::HEADER_HEIGHT_96
        assert_eq!(SIDEBAR_ROW, 26); // was notebook_layout::ROW_HEIGHT
    }

    #[test]
    fn the_layout_sizes_still_off_the_4px_grid_are_the_known_ones() {
        // Later steps move these onto the grid; when one moves, this list shrinks. Hairline strokes
        // (the focus ring and gap) are exempt from the grid.
        let sizes = [
            ("CONTROL_RADIUS", CONTROL_RADIUS),
            ("PANEL_HEADER", PANEL_HEADER),
            ("SIDEBAR_ROW", SIDEBAR_ROW),
        ];
        let off_grid: Vec<&str> = sizes
            .iter()
            .filter(|(_, value)| value % GRID != 0)
            .map(|(name, _)| *name)
            .collect();
        assert_eq!(off_grid, ["PANEL_HEADER", "SIDEBAR_ROW"]);
    }
```

- [ ] **Step 2: Run the test to see it fail**

Run: `cargo test --lib window::design::metrics`
Expected: compile errors, the constants don't exist.

- [ ] **Step 3: Add the constants**

In `src/window/design/metrics.rs`, above `#[cfg(test)]`, after `scale`:

```rust
/// The corner radius of cards, controls and buttons.
pub(crate) const CONTROL_RADIUS: i32 = 4;
/// The keyboard-focus ring's stroke, and its gap outside the control it rings.
pub(crate) const FOCUS_RING: i32 = 2;
pub(crate) const FOCUS_GAP: i32 = 1;
/// The height of a side panel's title row, shared by every sidebar view.
pub(crate) const PANEL_HEADER: i32 = 38;
/// The height of a row in the sidebar's lists (notebook tree, open editors, favorites).
pub(crate) const SIDEBAR_ROW: i32 = 26;
/// The layout grid. Only tests use it, to track the sizes not yet on it.
#[cfg(test)]
pub(crate) const GRID: i32 = 4;
```

- [ ] **Step 4: Move the users off the old constants**

Rename the three `soft_paint` constants everywhere:

```bash
cd /d/Projects/FastPad
grep -rlE 'RADIUS_AT_96_DPI|FOCUS_WIDTH_AT_96_DPI|FOCUS_GAP_AT_96_DPI' src tests | xargs sed -i -E 's/FOCUS_WIDTH_AT_96_DPI/FOCUS_RING/g; s/FOCUS_GAP_AT_96_DPI/FOCUS_GAP/g; s/\bRADIUS_AT_96_DPI\b/CONTROL_RADIUS/g'
```

Then delete the three now-renamed definitions in `src/window/soft_paint.rs` (the `pub(crate) const CONTROL_RADIUS`, `FOCUS_RING` and `FOCUS_GAP` lines and their doc comments, originally lines 37-41). `TITLE_HEIGHT_AT_96_DPI`, `TITLE_CLOSE_WIDTH_AT_96_DPI` and `GLYPH_FONT` stay.

Fix the imports in the users: `about.rs`, `settings_dialog.rs`, `settings_dialog/painting.rs`, `settings_dialog/layout.rs`, and `soft_paint.rs` itself if it uses them. Remove the three names from the `soft_paint::{...}` import and add:

```rust
use super::design::metrics::{CONTROL_RADIUS, FOCUS_GAP, FOCUS_RING};
```

(Use `crate::window::design::metrics::...` in files under `settings_dialog/`.) Import only the names each file uses.

Rename the two row and header constants by hand:

- `side_panel.rs:55` delete `pub(crate) const HEADER_HEIGHT_96: i32 = 38;`. Replace every `HEADER_HEIGHT_96` with `PANEL_HEADER` and import it from `design::metrics` in `side_panel.rs`, `notebook_layout.rs`, `search_view/geometry.rs` and `main_window/tests/sidebar_layout.rs:241` (that test writes the path inline as `crate::window::side_panel::HEADER_HEIGHT_96`; make it `crate::window::design::metrics::PANEL_HEADER`).
- `notebook_layout.rs:11` delete `pub(crate) const ROW_HEIGHT: i32 = 26;`. Replace `ROW_HEIGHT` with `SIDEBAR_ROW` in `notebook_layout.rs`, `notebook_view.rs` (lines 11, 422, 443) and `notebook_view/paint.rs` (lines 13, 623, 624), importing from `design::metrics`.
- `favorites_view.rs:31-32`: delete `HEADER_AT_96_DPI` and `ROW_AT_96_DPI`; replace their uses with `PANEL_HEADER` and `SIDEBAR_ROW`.
- Leave `search_view.rs`'s own `HEADER_AT_96_DPI` (38), `ROW_AT_96_DPI` (42), `FIELD_HEIGHT_AT_96_DPI` alone: they are that view's own row and field, not the shared panel header.

- [ ] **Step 5: Compile and run the targeted tests**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: PASS. Fix any leftover unresolved names or unused imports.

Run: `cargo fmt && cargo test --lib window::design && cargo test --lib window::soft_paint && cargo test --lib window::notebook_layout`
Expected: PASS. (A filter that matches nothing simply reports 0 tests.)

- [ ] **Step 6: Commit**

```bash
git add -A src
git commit -m "refactor: radius, focus ring and sidebar heights live in design::metrics"
```

---

### Task 3: Contrast helper and the baseline contrast test

**Files:**
- Create: `src/window/design/contrast.rs`
- Modify: `src/window/design/mod.rs`, `src/window/palette.rs` (tests module, lines ~490-506 and ~577)

**Interfaces:**
- Consumes: `Palette::for_theme`, `Theme::ALL`.
- Produces: `crate::window::design::contrast::{luminance(color: u32) -> f64, ratio(a: u32, b: u32) -> f64}` (test-only, `pub(crate)`). Inside `palette.rs` tests: `fn contrast_pairs(p: &Palette) -> Vec<Pair>` and `const KNOWN_SHORT: &[(Theme, &str)]`.

- [ ] **Step 1: Write the helper with its own tests**

Create `src/window/design/contrast.rs`:

```rust
//! WCAG 2.x contrast for the palette tests. Colors are Windows `COLORREF`s (`0x00BBGGRR`).

/// WCAG relative luminance of a `COLORREF`.
pub(crate) fn luminance(color: u32) -> f64 {
    let channel = |shift: u32| {
        let value = f64::from((color >> shift) & 0xFF) / 255.0;
        if value <= 0.040_45 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(0) + 0.7152 * channel(8) + 0.0722 * channel(16)
}

/// The contrast ratio of two colors, from 1:1 (identical) to 21:1 (black on white). Symmetric.
pub(crate) fn ratio(a: u32, b: u32) -> f64 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

#[cfg(test)]
mod tests {
    use super::ratio;

    #[test]
    fn black_on_white_is_21_to_1_and_a_color_on_itself_is_1_to_1() {
        assert!((ratio(0x0000_0000, 0x00FF_FFFF) - 21.0).abs() < 1e-9);
        assert!((ratio(0x0080_4020, 0x0080_4020) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn ratio_is_symmetric() {
        assert!((ratio(0x0012_3456, 0x00AB_CDEF) - ratio(0x00AB_CDEF, 0x0012_3456)).abs() < 1e-12);
    }
}
```

In `src/window/design/mod.rs` add below `pub(crate) mod metrics;`:

```rust
#[cfg(test)]
pub(crate) mod contrast;
```

- [ ] **Step 2: Point the existing palette tests at the shared helper**

In `src/window/palette.rs` tests, delete the private `luminance` and `contrast` functions (the block starting `/// WCAG relative luminance of a COLORREF.` through the end of `fn contrast`). Add to the test module's imports `use crate::window::design::contrast::ratio;` and change the one call in `file_icon_colours_stay_visible_on_selected_and_hovered_rows` from `contrast(color, background)` to `ratio(color, background)`.

- [ ] **Step 3: Add the palette-wide contrast test with an empty exceptions list**

Add to the `tests` module in `src/window/palette.rs`:

```rust
    struct Pair {
        name: &'static str,
        foreground: u32,
        background: u32,
        minimum: f64,
    }

    /// The text and UI pairs every themed palette must keep legible: 4.5:1 for text, 3:1 for
    /// secondary text and interactive shapes.
    fn contrast_pairs(p: &Palette) -> Vec<Pair> {
        let pair = |name, foreground, background, minimum| Pair {
            name,
            foreground,
            background,
            minimum,
        };
        vec![
            pair("editor text", p.editor_foreground, p.editor_background, 4.5),
            pair("strip text", p.strip_foreground, p.strip_background, 4.5),
            pair("muted text on strip", p.muted_foreground, p.strip_background, 4.5),
            pair("muted text on editor", p.muted_foreground, p.editor_background, 4.5),
            pair("error text", p.error_foreground, p.editor_background, 4.5),
            pair("line numbers", p.line_number_foreground, p.editor_background, 3.0),
        ]
    }

    /// Pairs that fall short today, each recorded with the measured ratio and left as they are so
    /// step 1 changes no color. A later step fixes the color and removes the entry. The test
    /// below fails if an entry starts passing, so this list cannot go stale.
    const KNOWN_SHORT: &[(Theme, &str)] = &[];

    #[test]
    fn every_theme_keeps_its_text_and_shapes_legible() {
        // Break caught: a palette edit that leaves text or a control nearly invisible on its
        // background, in any of the eight themes.
        for theme in Theme::ALL {
            let palette = Palette::for_theme(theme, false);
            for pair in contrast_pairs(&palette) {
                let measured = ratio(pair.foreground, pair.background);
                let known = KNOWN_SHORT.contains(&(theme, pair.name));
                if measured >= pair.minimum {
                    assert!(
                        !known,
                        "{theme:?} {}: now {measured:.2}:1, so remove it from KNOWN_SHORT",
                        pair.name
                    );
                } else {
                    assert!(
                        known,
                        "{theme:?} {}: {measured:.2}:1 is below {}:1",
                        pair.name, pair.minimum
                    );
                }
            }
        }
    }
```

- [ ] **Step 4: Run the tests and record any failures**

Run: `cargo test --lib window::design::contrast && cargo test --lib window::palette`
Expected: the `contrast` tests and all existing palette tests pass. The new test either passes, or fails with lines such as `Light muted text on strip: 4.31:1 is below 4.5:1`.

For each failing line, add an entry to `KNOWN_SHORT` with the measured ratio in a comment, and change nothing else. Example:

```rust
    const KNOWN_SHORT: &[(Theme, &str)] = &[
        (Theme::Light, "muted text on strip"), // 4.31:1, needs 4.5:1
    ];
```

Re-run until the test passes. Write the full list of exceptions (theme, pair, ratio) into the commit message body and report it to the user when the task is done. Do not change any color here.

- [ ] **Step 5: Compile and commit**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: PASS.

```bash
cargo fmt
git add -A src
git commit -m "test: contrast guard over every theme, helpers moved to design::contrast"
```

---

### Task 4: Palette roles

**Files:**
- Modify: `src/window/palette.rs` (struct at lines 17-44, the four literal palettes, `catppuccin()`, `high_contrast()`, imports, tests)

**Interfaces:**
- Consumes: `design::contrast::ratio`, `contrast_pairs`, `KNOWN_SHORT` from Task 3.
- Produces: six new `pub u32` fields on `Palette`: `accent`, `on_accent`, `stroke`, `disabled_foreground`, `warning_foreground`, `success_foreground`. Existing fields are unchanged.

- [ ] **Step 1: Write the failing tests**

In `src/window/palette.rs` tests, extend the pairs and add role tests. Add these entries to the `vec![...]` in `contrast_pairs`:

```rust
            pair("on accent", p.on_accent, p.accent, 4.5),
            pair("warning text", p.warning_foreground, p.editor_background, 4.5),
            pair("success text", p.success_foreground, p.editor_background, 4.5),
            pair("accent on editor", p.accent, p.editor_background, 3.0),
            pair("accent on strip", p.accent, p.strip_background, 3.0),
```

Add these tests:

```rust
    #[test]
    fn stroke_is_the_border_color_borders_use_today() {
        // Break caught: moving borders onto `stroke` changing a border's color, which would break
        // "no visible change".
        for theme in Theme::ALL {
            let palette = Palette::for_theme(theme, false);
            assert_eq!(palette.stroke, palette.pressed_background, "{theme:?}");
        }
    }

    #[test]
    fn the_new_roles_are_distinct_from_the_surfaces_they_sit_on() {
        // Break caught: an accent or status color that equals the background.
        for theme in Theme::ALL {
            let palette = Palette::for_theme(theme, false);
            for color in [
                palette.accent,
                palette.warning_foreground,
                palette.success_foreground,
                palette.disabled_foreground,
            ] {
                assert_ne!(color, palette.editor_background, "{theme:?}");
            }
            assert_ne!(palette.accent, palette.on_accent, "{theme:?}");
            assert_ne!(palette.disabled_foreground, palette.muted_foreground, "{theme:?}");
        }
    }

    #[test]
    fn high_contrast_maps_the_new_roles_to_system_colors_and_never_blends() {
        // Break caught: a blended or hard-coded color in high contrast, where only system
        // color pairs are allowed.
        use windows_sys::Win32::Graphics::Gdi::COLOR_GRAYTEXT;
        // High contrast ignores the theme, so any theme works here.
        let palette = Palette::for_theme(Theme::CatppuccinMocha, true);
        unsafe {
            assert_eq!(palette.accent, GetSysColor(COLOR_HIGHLIGHT));
            assert_eq!(palette.on_accent, GetSysColor(COLOR_HIGHLIGHTTEXT));
            assert_eq!(palette.stroke, GetSysColor(COLOR_WINDOWTEXT));
            assert_eq!(palette.disabled_foreground, GetSysColor(COLOR_GRAYTEXT));
            assert_eq!(palette.warning_foreground, GetSysColor(COLOR_WINDOWTEXT));
            assert_eq!(palette.success_foreground, GetSysColor(COLOR_WINDOWTEXT));
        }
    }
```

- [ ] **Step 2: Run to see the failure**

Run: `cargo test --lib window::palette`
Expected: compile errors, the fields don't exist.

- [ ] **Step 3: Add the fields**

In the `Palette` struct, after `error_foreground`:

```rust
    /// Interactive emphasis: the focus ring, the selection pill and a primary button's fill.
    pub accent: u32,
    /// Text and glyphs drawn on `accent`.
    pub on_accent: u32,
    /// 1px separators and control borders.
    pub stroke: u32,
    /// Disabled text and glyphs. Exempt from contrast rules, as disabled controls are.
    pub disabled_foreground: u32,
    /// Warning and success text on `editor_background`.
    pub warning_foreground: u32,
    pub success_foreground: u32,
```

- [ ] **Step 4: Add the values per theme**

Add these lines to each palette literal (after its `error_foreground:` line):

```rust
// LIGHT
    accent: rgb(0, 95, 184),
    on_accent: WHITE,
    stroke: rgb(204, 204, 204),
    disabled_foreground: rgb(160, 160, 160),
    warning_foreground: rgb(157, 93, 0),
    success_foreground: rgb(15, 123, 15),

// DARK
    accent: rgb(96, 205, 255),
    on_accent: rgb(0, 0, 0),
    stroke: rgb(72, 72, 76),
    disabled_foreground: rgb(110, 110, 110),
    warning_foreground: rgb(252, 225, 0),
    success_foreground: rgb(108, 203, 95),

// PAPER
    accent: rgb(0, 110, 100),
    on_accent: WHITE,
    stroke: rgb(207, 205, 194),
    disabled_foreground: rgb(160, 158, 148),
    warning_foreground: rgb(150, 90, 0),
    success_foreground: rgb(30, 120, 50),

// LAMP
    accent: rgb(110, 190, 170),
    on_accent: rgb(20, 24, 22),
    stroke: rgb(62, 60, 55),
    disabled_foreground: rgb(95, 92, 84),
    warning_foreground: rgb(225, 185, 90),
    success_foreground: rgb(140, 200, 120),
```

(Drop the `// LIGHT`-style headers; they only say which literal each block belongs to.) In `const fn catppuccin(flavor, dark)`, after `error_foreground: flavor.red,` add:

```rust
        accent: flavor.blue,
        on_accent: if dark { flavor.crust } else { WHITE },
        stroke: flavor.surface1,
        disabled_foreground: flavor.overlay1,
        // Latte's yellow and green are too pale on its light base for text, so the light flavor
        // pulls them two-thirds of the way to its text color.
        warning_foreground: if dark {
            flavor.yellow
        } else {
            catppuccin::blend(flavor.text, flavor.yellow, 170)
        },
        success_foreground: if dark {
            flavor.green
        } else {
            catppuccin::blend(flavor.text, flavor.green, 170)
        },
```

In `fn high_contrast()`, add `COLOR_GRAYTEXT` to the `windows_sys::Win32::Graphics::Gdi` import list at the top of the file, and after `error_foreground: text,` add:

```rust
            accent: highlight,
            on_accent: highlight_text,
            stroke: text,
            disabled_foreground: color(COLOR_GRAYTEXT),
            warning_foreground: text,
            success_foreground: text,
```

- [ ] **Step 5: Run the tests**

Run: `cargo test --lib window::palette`
Expected: everything passes except possibly the contrast test, which now covers the new pairs.

If it fails on a new-role pair, first check the values above are typed exactly. If a pair still falls short:
- **Catppuccin Latte `warning text` or `success text`:** raise the blend alpha from 170 to 190 and re-run. Repeat in steps of 20 until it passes.
- **Any other new-role pair:** these values were chosen to pass, so re-check the typing before anything else. If it still fails, treat it like Task 3 and list it in `KNOWN_SHORT` with its ratio, then report it.

- [ ] **Step 6: Fix hand-built palettes if any break**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: PASS. The existing hand-built palettes in tests (`option_toggles.rs`, `notebook_view/tests.rs`, `side_panel.rs`, `search_view/tests.rs`, `row_list.rs`) use `..Palette::neutral()`, so they should compile unchanged. If one lists every field, add the six new fields copied from `Palette::neutral()`.

- [ ] **Step 7: Commit**

```bash
cargo fmt
git add -A src
git commit -m "feat: palette roles for accent, stroke, disabled, warning and success"
```

---

### Task 5: The type ramp

**Files:**
- Create: `src/window/design/type_ramp.rs`
- Modify: `src/window/design/mod.rs`, `src/window/titlebar.rs:352-353`, `src/window/side_panel.rs:91-101` and `:168`, `src/window/settings_dialog.rs:309-311`, `src/window/about.rs:390-391`

**Interfaces:**
- Consumes: `metrics::scale`, `titlebar::create_ui_font(pixel_height: i32, face: &str, weight: i32, italic: bool) -> HFONT`.
- Produces: `crate::window::design::type_ramp::{TextStyle, Spec, create(style: TextStyle, dpi: u32) -> HFONT, UI_FACE: &str}`. `TextStyle` has `Body`, `BodyItalic`, `BodyBold`, `PanelHeader`, `DialogBody`, `Heading`, `Title`; `TextStyle::spec(self) -> Spec` is a `const fn`.

- [ ] **Step 1: Write the failing tests**

Create `src/window/design/type_ramp.rs` with the tests only:

```rust
//! The text styles FastPad's chrome uses: a pixel height at 96 DPI, a weight and an italic flag
//! each, and the one function that turns a style into a GDI font. Icon fonts and the editor and
//! preview fonts are not part of the ramp.

#[cfg(test)]
mod tests {
    use super::{TextStyle, create};
    use windows_sys::Win32::Graphics::Gdi::{
        DeleteObject, FW_BOLD, FW_NORMAL, FW_SEMIBOLD, GetObjectW, LOGFONTW,
    };

    #[test]
    fn every_style_keeps_the_size_and_weight_its_call_site_used() {
        // Break caught: a "no visible change" refactor that shifts a size or weight. These are the
        // values the fonts were created with before the ramp existed; step 2 changes them here.
        let spec = |style: TextStyle| {
            let s = style.spec();
            (s.px, s.weight, s.italic)
        };
        assert_eq!(spec(TextStyle::Body), (12, FW_NORMAL as i32, false)); // strip, tabs, sidebar text
        assert_eq!(spec(TextStyle::BodyItalic), (12, FW_NORMAL as i32, true)); // preview tab, notices
        assert_eq!(spec(TextStyle::BodyBold), (12, FW_BOLD as i32, false)); // search match
        assert_eq!(spec(TextStyle::PanelHeader), (11, FW_SEMIBOLD as i32, false));
        assert_eq!(spec(TextStyle::DialogBody), (13, FW_NORMAL as i32, false));
        assert_eq!(spec(TextStyle::Heading), (14, FW_SEMIBOLD as i32, false));
        assert_eq!(spec(TextStyle::Title), (18, FW_SEMIBOLD as i32, false));
    }

    #[test]
    fn fonts_are_created_at_unusual_dpis_and_scale_with_them() {
        // Break caught: a DPI of 0 (bad handle) or a scaled monitor producing a null font, or a
        // font whose height ignores the DPI.
        for (dpi, expected_height) in [(0_u32, 12), (96, 12), (144, 18), (192, 24)] {
            let font = create(TextStyle::Body, dpi);
            assert!(!font.is_null(), "dpi {dpi}");
            let mut log: LOGFONTW = unsafe { std::mem::zeroed() };
            let written = unsafe {
                GetObjectW(
                    font,
                    std::mem::size_of::<LOGFONTW>() as i32,
                    (&mut log as *mut LOGFONTW).cast(),
                )
            };
            assert!(written > 0, "dpi {dpi}");
            assert_eq!(log.lfHeight, -expected_height, "dpi {dpi}");
            unsafe { DeleteObject(font) };
        }
    }
}
```

Add to `src/window/design/mod.rs`:

```rust
pub(crate) mod type_ramp;
```

- [ ] **Step 2: Run to see the failure**

Run: `cargo test --lib window::design::type_ramp`
Expected: compile errors, `TextStyle` and `create` don't exist.

- [ ] **Step 3: Write the ramp**

Insert above the `#[cfg(test)]` block in `src/window/design/type_ramp.rs`:

```rust
use super::metrics::scale;
use crate::window::titlebar::create_ui_font;
use windows_sys::Win32::Graphics::Gdi::{FW_BOLD, FW_NORMAL, FW_SEMIBOLD, HFONT};

/// The chrome's text face. Segoe UI Variable with a fallback replaces it in step 2.
pub(crate) const UI_FACE: &str = "Segoe UI";

/// A named text style. Fonts for icons, the editor and the Markdown preview are not styles.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TextStyle {
    /// Title strip, tabs and sidebar row text.
    Body,
    /// The preview tab's label and notices inside sidebar lists.
    BodyItalic,
    /// The match in a Search result's snippet.
    BodyBold,
    /// Sidebar header titles.
    PanelHeader,
    /// Text in the Settings and About dialogs.
    DialogBody,
    /// A dialog's section headings.
    Heading,
    /// A dialog's title.
    Title,
}

/// A style's pixel height at 96 DPI, GDI weight and italic flag.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Spec {
    pub(crate) px: i32,
    pub(crate) weight: i32,
    pub(crate) italic: bool,
}

impl TextStyle {
    pub(crate) const fn spec(self) -> Spec {
        let (px, weight, italic) = match self {
            Self::Body => (12, FW_NORMAL, false),
            Self::BodyItalic => (12, FW_NORMAL, true),
            Self::BodyBold => (12, FW_BOLD, false),
            Self::PanelHeader => (11, FW_SEMIBOLD, false),
            Self::DialogBody => (13, FW_NORMAL, false),
            Self::Heading => (14, FW_SEMIBOLD, false),
            Self::Title => (18, FW_SEMIBOLD, false),
        };
        Spec {
            px,
            weight: weight as i32,
            italic,
        }
    }
}

/// A GDI font for `style` at `dpi`. The caller owns it and deletes it.
pub(crate) fn create(style: TextStyle, dpi: u32) -> HFONT {
    let spec = style.spec();
    create_ui_font(scale(spec.px, dpi), UI_FACE, spec.weight, spec.italic)
}
```

- [ ] **Step 4: Run to see it pass**

Run: `cargo test --lib window::design::type_ramp`
Expected: 2 passed.

- [ ] **Step 5: Move the call sites onto the ramp**

`src/window/titlebar.rs`, in `TitleFonts::create` replace the `text` and `italic` lines. Keep `glyph` (an icon font):

```rust
                text: type_ramp::create(TextStyle::Body, dpi),
                italic: type_ramp::create(TextStyle::BodyItalic, dpi),
                glyph: create_font(scale(10, dpi), "Segoe MDL2 Assets"),
```

Add `use super::design::type_ramp::{self, TextStyle};`. The private `create_font` helper stays, because the glyph font still uses it.

`src/window/side_panel.rs`, replace `UiFonts::create` and the bold-text font:

```rust
    fn create(dpi: u32) -> Self {
        let normal = FW_NORMAL as i32;
        Self {
            text: type_ramp::create(TextStyle::Body, dpi),
            text_bold: std::ptr::null_mut(),
            bold: type_ramp::create(TextStyle::PanelHeader, dpi),
            italic: type_ramp::create(TextStyle::BodyItalic, dpi),
            glyph: create_ui_font(scale(12, dpi), "Segoe MDL2 Assets", normal, false),
            bar_glyph: create_ui_font(scale(16, dpi), "Segoe MDL2 Assets", normal, false),
        }
    }
```

and in `text_bold`:

```rust
        let font = type_ramp::create(TextStyle::BodyBold, dpi);
```

Add `use crate::window::design::type_ramp::{self, TextStyle};`. Remove `FW_SEMIBOLD` and `FW_BOLD` from the imports if they become unused.

`src/window/settings_dialog.rs:309-311`:

```rust
    let title_font = type_ramp::create(TextStyle::Title, dpi);
    let heading_font = type_ramp::create(TextStyle::Heading, dpi);
    let body_font = type_ramp::create(TextStyle::DialogBody, dpi);
```

`src/window/about.rs:390-391`:

```rust
    let title_font = type_ramp::create(TextStyle::Title, dpi);
    let body_font = type_ramp::create(TextStyle::DialogBody, dpi);
```

Add `use super::design::type_ramp::{self, TextStyle};` to both. Leave `link_font` (`create_underlined_font(scale(13, dpi))`) and `glyph_font` untouched. Remove now-unused `FW_*` imports.

Leave the test-only `create_ui_font` calls in `notebook_view/tests.rs` and `search_view/tests.rs` as they are.

- [ ] **Step 6: Compile and run the targeted tests**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: PASS (fix unused imports if reported).

Run: `cargo fmt && cargo test --lib window::design && cargo test --lib window::about && cargo test --lib window::settings_dialog && cargo test --lib window::side_panel`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add -A src
git commit -m "feat: text styles for the chrome fonts through design::type_ramp"
```

---

### Task 6: Spec amendment and final verification

**Files:**
- Modify: `docs/superpowers/specs/2026-09-30-design-tokens-design.md`

**Interfaces:**
- Consumes: everything above.
- Produces: a spec that matches what was built, and a verified branch.

- [ ] **Step 1: Amend the spec**

Make these edits:

- §2 table, "Metrics scope" row: replace the sentence with "Consolidate `scale` (one function; a DPI of 0 counts as 96, which fixes the title bar copy's collapse to near-zero sizes), the control radius, the focus ring and gap, the sidebar panel header (38) and the sidebar row height (26). The field height 28, bar height 36 and command-palette row are repeated but diverge in later steps, so they stay per surface."
- §3.2: change "the identical copies" to "the two copies (they differed for a DPI of 0)". Replace the constants list with `CONTROL_RADIUS = 4`, `FOCUS_RING = 2`, `FOCUS_GAP = 1`, `PANEL_HEADER = 38`, `SIDEBAR_ROW = 26`, and a test-only `GRID = 4`. Delete the sentence about `STROKE` and the sentence about listing heights repeated across surfaces.
- §3.3: delete the `Caption` row and add "`Caption` (12 regular) and `OVERLAY_RADIUS = 8` are added by the step that first uses them, because unused items fail `-D warnings`." Add `BodyItalic` and `BodyBold` rows (12 italic, 12 bold) to the table.
- §3.4: replace the risks paragraph's "or fixed with a minimal value change" with "Step 1 records each such pair in a known-exceptions list and changes no color; step 4 fixes them." Add: "Latte's `warning_foreground` and `success_foreground` are the flavor's yellow and green pulled two-thirds toward its text color, because the raw swatches fail 4.5:1 on Latte's base."
- §5 risk bullet 3: same wording as the §3.4 change.

- [ ] **Step 2: Full compile check**

Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings`
Expected: no output from `fmt`, and Clippy PASS.

- [ ] **Step 3: Full test run, once**

Build the native DLLs if `native/out` is empty (see `tools/build-native.ps1`), then:

Run: `cargo test --locked -- --test-threads=1`
Expected: all tests pass. Any failure is a regression from this work: investigate the diff for the failing area before doing anything else.

- [ ] **Step 4: Startup benchmark check**

Nothing on the startup path changed (constants, statics and the same font creations), so this is a confirmation and not a tuning step. Follow `benchmarks/README.md` to run the startup benchmark on the reference machine with `tools/benchmark.ps1 -EnforceReference`. On any other machine, run it without `-EnforceReference` and compare against a run from the commit before this branch's first task, judging p95 with the bootstrap confidence interval as the README describes.

Expected: no difference beyond noise.

- [ ] **Step 5: Optional live smoke check**

Only if the app builds locally. Back up the user's settings first, run, then restore:

```powershell
Copy-Item "$env:LOCALAPPDATA\FastPad\fastpad.ini" "$env:TEMP\fastpad.ini.bak"
# run the app, cycle themes, open Settings and About, open the sidebar views, then close it
Copy-Item "$env:TEMP\fastpad.ini.bak" "$env:LOCALAPPDATA\FastPad\fastpad.ini" -Force
```

Expected: the chrome looks identical to before, in every theme and at the system's DPI.

- [ ] **Step 6: Commit and report**

```bash
git add docs/superpowers/specs/2026-09-30-design-tokens-design.md
git commit -m "docs: amend the design tokens spec to match what was built"
```

Report to the user: the list of `KNOWN_SHORT` contrast exceptions with measured ratios (from Task 3, and Task 4 if any), the DPI-0 behaviour change, and the benchmark result.
