# Rounded tabs and row backplates: design

- Status: approved in conversation on 2026-10-01. Written spec awaiting review.
- Branch: `feat/rounded-tabs-rows`, stacked on `feat/type-and-icons` (PR #45), which is stacked on `feat/ui-quality-improvements` (PR #44).
- Step 3a of the Windows design alignment. Step 3b is the 8px command palette with its shadow, and 3c is one keyboard focus ring. Both reuse what this piece builds.

## 1. Goal

Give the tab strip and the sidebar lists the softer Windows 11 shapes, without loading Direct2D or adding any startup work.

- **Rounded tabs:** the active, hovered and pressed tabs get rounded top corners.
- **Row backplates:** hover and selected rows in the sidebar lists become rounded, inset fills.
- **A build you can run** at the end, so the shapes are judged by eye.

## 2. Decisions

| Question | Decision |
|---|---|
| How to round | GDI only. A small helper fills the body with `FillRect` and draws each corner pixel as a blend of the shape color and the known background color, from an analytic coverage value. Direct2D stays dialog-only (`soft_paint.rs` loads it lazily, so `d2d1.dll` stays out of startup). |
| Tab radius | 8px at 96 DPI, scaled, on the top two corners only. The bottom edge stays flush, so the active tab still joins the editor. |
| Row backplate | 4px radius (`CONTROL_RADIUS`), inset 4px from the left and right panel edges and 1px top and bottom. |
| Close-button hover (tab) | A 4px-rounded square in place of the square fill. |
| High contrast | Shapes stay square with system colors. No blending in high contrast. |
| Not in this piece | Search results rows, field borders, the command palette, focus rings, hit-testing, row heights, text and icon positions, colors. |

## 3. Design

### 3.1 The helper (`design/round.rs`)

- `fill_rounded(dc, rect, radius, corners, color, behind)`: `corners` says which of the four corners are rounded (a small flags value), `behind` is the color of the surface under the shape. It fills the body with `FillRect` in at most three rectangles (the middle band and the straight side strips) and draws the corner squares pixel by pixel.
- `coverage(x, y, radius) -> u8` is a pure function: how much of pixel (x, y) in a corner square of the given radius lies inside the arc, from 0 to 255. Pixel centers are sampled in a 4x4 grid, so edges are smoothed without a table or allocation.
- The radius is clamped to half the shorter side of the rect, as in `soft_paint::rounded`.
- `blend` from `catppuccin::blend` is reused if its signature fits; otherwise a three-line channel blend sits beside `coverage`.
- Cost: about `radius^2` pixel writes per rounded corner, only for the one or two shapes that are hovered, selected or active. No allocation, no loads, so the first-paint rules are untouched.

### 3.2 Tabs (`group_strip.rs`)

- The fills at `group_strip.rs:375-380` (active, hover, idle) go through `fill_rounded` with the top corners rounded for the active and hovered tab. The idle tab keeps the plain strip background. `behind` is `strip_background`.
- The active tab's `editor_background` fill stays flush at the bottom with the editor.
- The 2px top accent bar (shown only with several groups) keeps its place and is clipped to the rounded outline by being drawn first and then covered at the corners.
- The close button's hover and pressed squares become 4px rounded.
- Titlebar caption buttons stay square: they sit at the window edge.

### 3.3 Sidebar rows (`row_list.rs`, `favorites_view.rs`)

- `row_list.rs:319-328` draws the selected, inactive-selected and hover backgrounds. They become `fill_rounded` over the row rect inset by 4px left and right and 1px top and bottom, all four corners rounded, `behind` the panel background.
- Favorites use the same function through the same call. The open notebook's 3px left accent bar stays at the panel edge.
- Text, icons, indentation, row height (`SIDEBAR_ROW`), scrolling and hit-testing do not change: only the painted fill is inset.

### 3.4 Metrics (`design/metrics.rs`)

- New constants at 96 DPI: `TAB_RADIUS = 8`, `ROW_INSET_X = 4` and `ROW_INSET_Y = 1`. `CONTROL_RADIUS` is reused for the row and close-button radius. The metrics tests pin the new values, and the off-grid list stays as it is (8, 4 and 1 are on the 4px grid or below it; 1 is not a layout size).

## 4. Tests

- Unit tests for `coverage`: the far corner pixel is 0, the pixel nearest the arc center is 255, the corner is symmetric across the diagonal, radius 0 gives full coverage, and values never decrease moving toward the center along a row.
- Unit tests for the blend: coverage 0 gives the background, 255 gives the shape color, and 128 is within one channel step of the midpoint.
- A test that `fill_rounded` with a radius larger than half the side clamps and does not panic or draw outside `rect`.
- A paint test that draws a rounded rect into a memory DC and checks that the center pixel is the shape color, a pixel outside the rect is untouched, and the outermost corner pixel equals `behind`.
- The metrics tests pin `TAB_RADIUS`, `ROW_INSET_X` and `ROW_INSET_Y`.
- A high-contrast test that the row and tab fills stay square (no blended pixels at a corner).

## 5. Risks and checks

- **Corner blend on a non-flat surface.** The blend assumes `behind` is a flat color. The sidebar panel and the tab strip are flat, so it holds. If the active tab's neighbor tab overlaps its corner, the corner pixels would blend toward `strip_background`, which is the neighbor's color too, so it is correct.
- **Selection over a hover.** A row that is both hovered and selected draws one fill, as now, so no double blend.
- **DPI.** Radius and insets scale through `scale`. The paint test runs at 96, 144 and 192.
- **Startup.** Nothing new is loaded or allocated before the first frame. The benchmark gate in `benchmarks/README.md` is re-run as a confirmation.
- **Verification:** Clippy, the targeted tests for the changed modules, the full suite once, then the build. Before the build, `fastpad.ini` is backed up.

## 6. Next pieces

- 3b: the 8px command palette with a shadow, and rounded Search fields.
- 3c: one keyboard focus ring (accent, 2px, 1px gap) replacing the dotted rectangle and the 1px outline, with focus shown in the tab strip and palette rows.
