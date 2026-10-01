# One keyboard focus ring: design

- Status: approved in conversation on 2026-10-01. Written spec awaiting review.
- Branch: `feat/focus-ring`, stacked on `feat/rounded-fields-palette` (PR #47), which is stacked on #46, #45 and #44.
- Step 3c of the Windows design alignment, and the first step that uses the `accent` color role.

## 1. Goal

Give the chrome's keyboard focus indicators the same look the dialogs already have: a 2px accent ring with rounded corners.

- **Activity bar:** the focused button shows the ring in place of the dotted `DrawFocusRect`.
- **Notebook view:** the focused header row (Open editors header, an Open editors row, the notebook name row) shows the ring in place of the 1px square outline.
- **A build you can run** at the end, so the ring is judged by eye.

## 2. Findings that set the scope

- Only two chrome surfaces draw a keyboard focus indicator: the activity bar (`activity_bar.rs` `paint_keyboard_focus`, `DrawFocusRect` on the button rect inset by 3px) and the notebook view (`notebook_view/paint.rs`, `paint_outline` in `selection_background` around the `Cursor::EditorsHeader`, `Cursor::Editor(i)` and `Cursor::Root` rows, drawn only while the panel has focus).
- The dialogs (About, Settings, shortcuts page) already draw an accent ring through Direct2D (`about.rs`, `settings_dialog/painting.rs`, `shortcuts_page.rs`), using `FOCUS_RING` (2) and `FOCUS_GAP` (1). They are not touched.
- The tab strip is not a keyboard focus target, and in the command palette the selected row is the focus indicator. Neither gets a new focus state in this piece.

## 3. Decisions

| Question | Decision |
|---|---|
| Color | `palette.accent` (the first consumer of the role). The theme contrast tests already require `accent` to reach 3:1 against the editor and strip backgrounds. In high contrast `accent` is the system highlight color. |
| Size | `FOCUS_RING` (2px) wide, scaled, with the ring's outer edge `FOCUS_GAP` (1px, scaled) inside the control's rect, so it never touches a neighbor. |
| Radius | `CONTROL_RADIUS` (4) through `radius_for`; square in high contrast. |
| How to draw | GDI only: a new `round::stroke_ring`. Straight edges are plain fills. Corner pixels read the pixel already under them and blend toward it by the ring's coverage, so the ring sits correctly over any background (a selected row, a hover, a flat panel). |
| Not in this piece | The dialogs' ring, the drop-band outline in high contrast (`notebook_view/paint.rs`, the `paint_band` use of `paint_outline`), new focus states for tabs or palette rows, and any other use of `accent`. |

## 4. Design

### 4.1 The helper (`design/round.rs`)

- `ring_coverage(x, y, radius, width) -> u32`: how much of the corner pixel (`x`, `y`) lies inside the ring band, from 0 to 255. It is the 4x4-sample coverage of the outer arc (radius `radius`) minus that of the inner arc (radius `radius - width`, floored at 0), clamped at 0. It requires a radius above 0; a square ring (radius 0, as in high contrast) is drawn as four plain strips and has no corner pixels.
- `stroke_ring(dc, rect, radius, width, color)`: clamps `radius` to half the shorter side and `width` to the radius or to at least 1 when the radius is 0. It fills the four straight edge strips with `FillRect`, excluding the corner squares, and for each corner square pixel with coverage above 0 reads `GetPixel`, blends `color` over it at that coverage, and writes `SetPixelV`. An empty or inverted rect draws nothing.
- Cost: at most `4 * radius^2` pixel reads and writes (at 192 DPI about 256), only when focus is drawn, which is on a focus change or a repaint of the focused control.
- `GetPixel` works on both the memory DC the sidebar paints into and the activity bar's paint DC. If a DC cannot be read, `GetPixel` returns `CLR_INVALID`; the helper then skips that pixel rather than writing a wrong color.

### 4.2 Activity bar (`activity_bar.rs`)

- `paint_keyboard_focus` computes the focused button's rect as it does now, then calls `stroke_ring` with that rect inset by `FOCUS_GAP` (scaled), `radius_for(palette, CONTROL_RADIUS, dpi)`, `scale(FOCUS_RING, dpi)` and `palette.accent`. The old `scale(3, dpi)` inset and `DrawFocusRect` go. If the function has no palette in scope, take it from the same place the bar's paint reads its colors.

### 4.3 Notebook view (`notebook_view/paint.rs`)

- In the `paint.focused` block, `paint_outline(dc, rect, palette.selection_background, dpi)` becomes `stroke_ring` over `rect` inset by `FOCUS_GAP`, with the same radius, width and `palette.accent`.
- `paint_outline` stays for the high-contrast drop band at its other call site.

### 4.4 Metrics

- `FOCUS_RING` and `FOCUS_GAP` already exist in `design/metrics.rs`. Their users after this step are the dialogs (as now) and these two surfaces.

## 5. Tests

- `ring_coverage`: the outermost corner pixel is 0; a pixel on the outer arc's inside edge, within `width` of it, is above 0; a pixel deeper than `width` from the arc is 0; symmetry across the diagonal; radius 0 gives 255 within the band and 0 outside.
- `stroke_ring` on a memory DC with a known fill: the middle of the top, left, right and bottom edge strips equals the ring color; the interior (more than `width` in) is untouched; the outermost corner pixel is untouched; a corner pixel on the arc is a blend of the ring color and the original pixel (neither of the two); the rect's outside is untouched; a rect smaller than twice the width and an empty rect do not panic or draw outside.
- A ring over a non-flat background: two different background colors under the same corner pixel give two different blended results (the blend uses the underlying pixel).
- High contrast: `radius_for` gives 0, so the ring is square and its corner pixel equals the ring color exactly.
- Activity bar: `paint_keyboard_focus` is bound to a window handle, so its drawing is covered by the helper tests above and by the build.
- Notebook view: a test that a focused `Cursor::Root` row has the accent color on its top edge strip and an unfocused panel has none.

## 6. Risks and checks

- **Reading pixels.** `GetPixel` on the sidebar's memory DC is cheap. On a window DC it can be slower, so the activity bar's call site should paint into the bar's existing buffer if it has one; if it paints straight to the window DC, a few hundred reads on a focus repaint are acceptable, and the build is for checking there is no flicker.
- **Ring over a selection.** The notebook's focused row is usually also the selected row. The ring is accent over `selection_background`; the contrast tests cover accent against the editor and strip backgrounds, not the selection fill, so the build is where this pairing is judged by eye. If it fails visually, the fix is a ring color or a gap, in one place.
- **DPI.** Width, gap and radius scale through `scale`. Tests run at 96 DPI; scaling is covered by the metrics and `radius_for` tests.
- **Startup.** Nothing new is loaded or allocated. The ring is only drawn when a control has keyboard focus.
- **Verification:** Clippy, the targeted tests for the changed modules, the full suite once, then the build. Before the build, `fastpad.ini` is backed up.

## 7. After this piece

The geometry step is complete (rounded tabs and rows, rounded fields and palette interior, one focus ring). Remaining design steps, each with its own spec: Segoe UI Variable and Fluent Icons with a Windows 10 fallback, color and accent (consuming `accent` for selection, and the five known low-contrast theme pairs), and the dialogs and copy pass.
