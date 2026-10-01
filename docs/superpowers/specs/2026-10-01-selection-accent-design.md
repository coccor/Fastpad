# Accent selection indicator: design

- Status: approved in conversation on 2026-10-01. Written spec awaiting review.
- Branch: `feat/selection-accent`, stacked on `feat/fonts-text-size` (PR #49), which is stacked on #48, #47, #46, #45 and #44.
- Step 5 of the Windows design alignment, narrowed to one piece: the `accent` role marks the selected row.

## 1. Goal

Selected rows gain the short rounded accent bar Windows 11 puts on the left edge of a selected navigation row. The fills stay as they are, so the bar is additive and no theme changes its selected-row look.

- **A build you can run** at the end, so the bar is judged by eye.

## 2. Findings that set the scope

- `row_list::paint` is the one painter of selected and hover rows for the Favorites view, the Notebook view (its three lists), the Open editors section and the Search results. Each is called with `focused`, which picks `selection_background` (focused) or `inactive_selection_background`.
- The command palette paints its own rows (`command_palette.rs` `paint_row_background`): the selected row fills with `hover_background`, inset and rounded, or the whole row in high contrast.
- The dropdown list (`dropdown_list.rs`) paints through Direct2D (`Shape::Round`): the selected row's pill is `selection_background`.
- The Favorites view already draws a 3px bar on the open notebook's row (`favorites_view.rs`, "The open notebook gets an accent bar"), colored `selection_background`.
- `accent` is consumed only by the focus ring and the dialogs today.

## 3. Decisions

| Question | Decision |
|---|---|
| Fills | Unchanged. The bar sits on the existing selected fill. |
| Color | `palette.accent`. |
| Shape | `SELECTION_BAR_WIDTH` 3px and `SELECTION_BAR_HEIGHT` 16px (both at 96 DPI, scaled), a pill (radius half the width), vertically centered in the row's backplate, `SELECTION_BAR_INSET` 4px from the backplate's left edge. The height is clamped to the backplate's height minus twice the inset. |
| When | Only the selected row, and in `row_list::paint` only while `focused`. An unfocused panel shows the dimmer fill and no bar. Hover never gets one. The palette's and the dropdown's selected row always get it (they are focused while open). |
| High contrast | No bar. The selection is already the full system highlight and `accent` is that same color there, so a bar would be invisible on it. |
| Open-notebook bar | The Favorites open-notebook bar changes from `selection_background` to `accent`, keeping its 3px width. |
| Not in this piece | Field borders (`selection_background`), the focus ring, any palette value, the five known-short contrast pairs, following the Windows accent color, and accent on tabs or the activity bar. |

## 4. Design

### 4.1 Geometry and drawing (`design/metrics.rs`, `design/round.rs`)

- `metrics`: `SELECTION_BAR_WIDTH` (3), `SELECTION_BAR_HEIGHT` (16), `SELECTION_BAR_INSET` (4).
- `round::selection_bar_rect(backplate: RECT, dpi: u32) -> RECT`: pure. Left = `backplate.left + scale(INSET)`, width `scale(WIDTH)`, height `min(scale(HEIGHT), backplate height - 2 * scale(INSET))` floored at 1, centered vertically.
- `round::paint_selection_bar(dc, backplate, palette, behind, dpi)`: does nothing when `palette.high_contrast`; otherwise `fill_rounded` of `selection_bar_rect` with radius half the bar's width, `Corners::ALL`, `palette.accent`, blending toward `behind` (the selected fill's color) at its corners.
- The Direct2D dropdown draws the same rect (`selection_bar_rect`) as a `Shape::Round` in `accent`, skipped in high contrast.

### 4.2 Call sites

- `row_list::paint`: after the selected row's fill, when `focused`, `paint_selection_bar(hdc, backplate, palette, selection_background, dpi)`. `draw_row` then draws over the row as before; the bar sits left of the row's glyph (the row content already starts right of the row inset, see 5).
- `command_palette.rs` `paint_row_background`: after the selected fill, the bar over `fill_rect`, `behind` = `hover_background`.
- `dropdown_list.rs`: next to the selected row's `Shape::Round`.
- `favorites_view.rs`: the open-notebook bar's color becomes `palette.accent`.

## 5. Tests

- `selection_bar_rect`: 3px wide and 16px tall at 96 DPI, centered vertically, `INSET` from the left; scales at 144 and 192 DPI; a short backplate clamps the height and never goes below 1.
- `paint_selection_bar` on a memory DC: the bar's middle pixel is the accent; the pixel just right of the bar is the fill underneath; the rounded ends' corner pixels are a blend (neither accent nor fill); high contrast paints nothing.
- `row_list::paint`: a focused selected row has the accent at the bar's middle; an unfocused selected row, a hovered row and an unselected row have none; high contrast has none. The existing row tests guard the fills.
- Command palette: a selected row has the bar; an unselected row does not.
- Favorites: the open-notebook bar's pixel equals `accent`.
- Dropdown: the pure rect is shared, so the unit test above covers its geometry; its drawing is judged in the build.
- Row content clear of the bar: a test that, at 96/144/192 DPI, the glyph's left edge in each `row_list` surface is right of the bar's right edge (`INSET + WIDTH`). If a surface's glyph starts left of it, that surface's content inset grows in the same step.

## 6. Risks and checks

- **Glyph collision.** The bar occupies x 4..7 (at 96 DPI) of the row. Row content starts after the row's own padding; the test in section 5 pins that it clears the bar, and a surface that does not gets its inset raised.
- **Latte and Paper.** The bar is accent over the selected fill; accent already clears 3:1 against the editor and strip backgrounds (existing contrast test), not against the selected fill, so the build is where that pairing is judged. A fix would be a bar color or width, in one place.
- **Startup.** Nothing is loaded or allocated; the bar draws only for selected rows when painted.
- **Verification:** Clippy, targeted tests for the changed modules, the full suite once, then the build. Before the build, `fastpad.ini` is backed up.

## 7. After this piece

Remaining and parked: the five known-short contrast pairs, high-contrast focus visibility on the active editor row, the optional Windows accent color, accent on tab and activity-bar indicators, then the dialogs and copy pass.
