# Design tokens: design

- Status: approved in conversation on 2026-09-30. Written spec awaiting review.
- Branch: `feat/ui-quality-improvements`.
- Step 1 of the Windows-design alignment work. The later steps (type and icons, geometry and states, color, dialogs and copy) consume what this step builds.

## 1. Goal

FastPad's chrome is custom-painted, and its look lives in scattered places: a flat 20-field `Palette` where `hover_background`, `pressed_background` and `selection_background` also act as border and accent, roughly 290 per-file `*_AT_96_DPI` size constants across 25 files, and font sizes chosen at each `create_ui_font` call. The Windows design guidelines the work follows describe a token system (a type ramp, a 4px grid, two corner radii, semantic color roles). This step adds that layer so the later visual steps change a value in one place instead of editing many files.

- **No visible change.** Step 1 must render identically to today. Every value it introduces either equals the value in use now or is not yet consumed.
- **One place per decision:** color roles, shared metrics, radii and text styles each have a single owner.
- **Cheap to revert:** the 13px body-text trial in step 2 must be a one-number change.
- **Guarded:** a contrast test covers every compiled-in theme, so a later color edit can't silently hurt legibility.

## 2. Decisions

| Question | Decision |
|---|---|
| Where it lives | A new module `src/window/design/` with `metrics.rs`, `type_ramp.rs` and a test-only `contrast.rs`. `Palette` stays in `src/window/palette.rs` and gains the new roles. |
| Palette shape | Keep the flat `Copy` struct and the static per-theme arrays. Add roles as fields. No trait, no lookup by name. Resolving a palette stays an array index. |
| New color roles | `accent`, `on_accent`, `stroke`, `disabled_foreground`, `warning_foreground`, `success_foreground`. Keyboard focus uses `accent`; it gets no role of its own. |
| Migrating consumers | Only where the value is identical today. Everything else keeps reading its current field until the step that changes it. |
| Metrics scope | Consolidate `scale` (one function; a DPI of 0 counts as 96, which fixes the title bar copy's collapse to near-zero sizes), the control radius, the focus ring and gap, the sidebar panel header (38) and the sidebar row height (26). The field height 28, bar height 36 and command-palette row are repeated but diverge in later steps, so they stay per surface. |
| Grid | Documented as 4px. Values that are off-grid today (22, 26, 30, 34, 38, 42, 46) keep their current value in step 1 and are normalised by later steps. |
| Type ramp values in step 1 | The values in use today. Step 2 changes them (section 3.3). |
| Font face | Still "Segoe UI" in step 1. The Segoe UI Variable choice with fallback arrives in step 2. |
| Old constants | Removed when replaced, with no aliases left behind. |
| Settings and ini keys | None added. |

## 3. Design

### 3.1 Color roles (`window/palette.rs`)

| Role | Meaning | Value in step 1 |
|---|---|---|
| `accent` | Interactive emphasis: focus ring, selection pill, primary button. | Set per theme to a color that is a proper accent. It is not consumed yet; consumers keep `selection_background` until step 4. |
| `on_accent` | Text and glyphs drawn on `accent`. | Per theme, chosen to pass 4.5:1 against `accent`. |
| `stroke` | 1px separators and control borders. | Equal to the theme's `pressed_background`, which is what borders use today. Border consumers may move to it in step 1. |
| `disabled_foreground` | Disabled text and glyphs. | Per theme; today disabled reuses `muted_foreground` or `pressed_background`. |
| `warning_foreground`, `success_foreground` | Semantic status text. | Per theme; used by later steps. |

- High contrast: every new role maps to a system color pair (`accent` to `COLOR_HIGHLIGHT`, `on_accent` to `COLOR_HIGHLIGHTTEXT`, `stroke` to `COLOR_WINDOWTEXT`, `disabled_foreground` to `COLOR_GRAYTEXT`, warning and success to `COLOR_WINDOWTEXT`). High contrast never blends.
- Catppuccin themes take `accent` from the flavour's `blue`, `warning_foreground` from `yellow` and `success_foreground` from `green`, following the style guide mapping already used in `catppuccin()`.
- Existing fields are unchanged, so every current palette test keeps its meaning.

### 3.2 Metrics (`design/metrics.rs`)

- `scale(value, dpi)`: the one implementation of `(v * dpi + 48) / 96`. The two copies (they differed for a DPI of 0) in `window/titlebar.rs` and `window/panel.rs` are deleted.
- Named constants at 96 DPI: `CONTROL_RADIUS = 4`, `FOCUS_RING = 2`, `FOCUS_GAP = 1` (moved from `soft_paint.rs`), `PANEL_HEADER = 38` and `SIDEBAR_ROW = 26`.
- A test-only `GRID = 4` constant and a small `on_grid` helper used only by a test that lists every remaining off-grid value, so later steps can watch that list shrink.

### 3.3 Type ramp (`design/type_ramp.rs`)

One table of text styles, each with a pixel height at 96 DPI and a weight, and one function that builds the GDI font for a style at a DPI. It wraps the existing `create_ui_font`. Step 1 records today's sizes; step 2 changes the numbers.

| Style | Step 1 (today) | Step 2 (target) |
|---|---|---|
| `Body` | 12 regular (chrome) | 13 regular |
| `BodyItalic` | 12 italic | 12 italic |
| `BodyBold` | 12 bold | 12 bold |
| `PanelHeader` | 11 semibold | 13 semibold |
| `Heading` | 14 semibold | 14 semibold |
| `Title` | 18 semibold | 18 semibold |
| `DialogBody` | 13 regular | removed; dialogs use `Body` |

- The four call-site groups that create fonts today (title strip and tabs, side panel, settings dialog, about dialog) switch to styles. Editor and Markdown preview fonts are user settings and stay out of the ramp.
- `Caption` (12 regular) and `OVERLAY_RADIUS = 8` are added by the step that first uses them, because unused items fail `-D warnings`.
- Reverting the 13px trial means changing the `Body` row back to 12.

### 3.4 Contrast tests (`design/contrast.rs`, `#[cfg(test)]`)

- A WCAG relative-luminance function and a `ratio(a, b)` helper.
- One test loops `Theme::ALL` over `Palette::for_theme(theme, false)` and asserts, per theme:
  - text 4.5:1: `editor_foreground` on `editor_background`, `strip_foreground` and `muted_foreground` on `strip_background`, `muted_foreground` and `error_foreground` on `editor_background`, `on_accent` on `accent`, `warning_foreground` and `success_foreground` on `editor_background`;
  - non-text 3:1: `accent` on `editor_background` and on `strip_background`;
  - `line_number_foreground` on `editor_background` at 3:1, an allowance for secondary, non-essential text;
  - `stroke` is not tested, because it is a decorative separator.
- Latte's `warning_foreground` and `success_foreground` are the flavor's yellow and green pulled two-thirds toward its text color, because the raw swatches fail 4.5:1 on Latte's base.
- The tests do not cover high contrast, which uses system colors.

## 4. Non-goals

- No change to any visible size, color, font or icon. Those are steps 2 to 5.
- No Mica, no motion, no themed confirmation dialog.
- No new settings and no ini changes.
- No renaming of existing `Palette` fields.
- No change to the editor's syntax colors or the file-icon sets.

## 5. Constraints and risks

- **Startup latency.** Everything is `const` or `static`, with no allocation and no theme query before first paint. Font creation happens at the same points as today. `benchmarks/README.md` gates are re-run after the change.
- **First-paint rule.** Paint never triggers a load (`chrome.rs`), so the ramp only wraps existing font creation. It adds none.
- **The contrast test may find failures in today's themes.** Any current pair below its threshold is listed in the test's failure output before any color is touched. Step 1 records each such pair in a known-exceptions list and changes no color; step 4 fixes them. Step 1 does not silently retune a theme.
- **Reach.** About 25 files use `*_AT_96_DPI` constants. Limiting migration to shared values keeps this step reviewable. The remaining constants move with the steps that change them.
- **`Palette` derives `Copy` and `Eq`.** Adding fields is compatible. Tests that build a `Palette` by hand need the new fields.

## 6. Verification

- `cargo clippy` for the compile check, per the repo's testing practice.
- A targeted unit test run for `window::design`, `window::palette` and the modules whose constants moved. The full suite runs once at final review.
- A test that asserts each migrated constant equals the value it replaced, so "no visible change" is checked and not assumed.
- The startup benchmark gates from `benchmarks/README.md`.

## 7. Follow-on steps (out of scope here)

2. **Type and icons:** flip the ramp to 13px body, Segoe UI Variable with fallback, Segoe Fluent Icons on Windows 11, 16px sidebar glyphs, and Windows text-size scaling for chrome.
3. **Geometry and states:** rounded active tab and 4px hover backplates (a spike first), 8px command palette with shadow, one focus ring, inactive-window state, metrics normalised to the grid.
4. **Color:** consume `accent`, the optional Windows-accent setting, and any contrast fixes.
5. **Dialogs and copy:** a themed confirmation dialog, sentence-case copy pass, the theme setting's Windows color settings link.
