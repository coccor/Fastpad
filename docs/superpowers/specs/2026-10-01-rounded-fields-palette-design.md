# Rounded fields and the command palette's inside: design

- Status: approved in conversation on 2026-10-01. Written spec awaiting review.
- Branch: `feat/rounded-fields-palette`, stacked on `feat/rounded-tabs-rows` (PR #46), which is stacked on #45 and #44.
- Step 3b of the Windows design alignment. Step 3c is one keyboard focus ring.

## 1. Goal

Carry the rounded shapes from the tabs and sidebar rows (step 3a) to the remaining flat controls, using the same GDI-only helper.

- **Command palette:** a rounded, bordered search field and a rounded, inset selected row.
- **Sidebar Search view:** rounded, bordered fields and rounded hover fills on its buttons.
- **A build you can run** at the end, so the shapes are judged by eye.

## 2. Decisions

| Question | Decision |
|---|---|
| Palette outer shape | Stays a square 1px frame. The palette is a child window over the editor, so rounded outer corners and a shadow would have to show the editor pixels underneath, which the flat-color blend cannot do. A window region gives jagged corners, and a real popup window changes focus and z-order handling. Both are out of scope. |
| Rounding | The same GDI helper as step 3a, `design/round.rs`. Nothing is loaded and nothing is allocated. |
| Bordered box | A new `round::fill_bordered`: the outer rounded fill in the border color, then the inner fill inset by 1px with radius minus 1, so the border edge is smooth. |
| Radius | `CONTROL_RADIUS` (4) for fields, selected rows and buttons. |
| Palette selected row | A 4px-rounded fill inset by `ROW_INSET_X` and `ROW_INSET_Y` (the step 3a constants), with the same colors as today. Row height (26px), text and hit-testing do not change. |
| Search view | The search and replace field boxes (a 1px border around the field fill) become rounded bordered boxes. Its hover-square buttons (clear, replace all, the option toggles) become 4px-rounded fills. |
| High contrast | Everything stays square with system colors, through `radius_for`. |
| Not in this piece | The palette's outer corners and shadow, the notebook and Favorites header buttons, tab and caption buttons, and the focus ring. |

## 3. Design

### 3.1 The helper (`design/round.rs`)

- `fill_bordered(dc, rect, radius, fill, border, behind)`: `fill_rounded(dc, rect, radius, Corners::ALL, border, behind)`, then `fill_rounded(dc, inset(rect, 1 scaled to at least 1px), (radius - 1).max(0), Corners::ALL, fill, border)`. The inner fill's `behind` is the border color, which is what lies under its corners. In high contrast the radius is 0, so the box is two square fills, as today.
- The border width is one device pixel at every DPI, as the existing 1px borders are today (they are `1` pixel `fill` calls, not scaled). Check the current code and keep whichever it uses.

### 3.2 Command palette (`command_palette.rs`)

- The search field box (a border in `selection_background` around an `editor_background` fill, painted at about lines 886-887) goes through `fill_bordered` with `behind = strip_background`, the palette body.
- The selected row's fill (`hover_background`, painted at about 929-951 and 1029) becomes a rounded fill over the row rect inset by `ROW_INSET_X` and `ROW_INSET_Y`, with `behind = strip_background`. The text and the row rect handed to the text drawing do not change.
- The outer 1px frame (`pressed_background` around the `strip_background` body, about line 884-885) is unchanged.

### 3.3 Sidebar Search view (`search_view/paint.rs`)

- The two field boxes (about lines 172-173 and 216-217) go through `fill_bordered` with `behind = paint.background`.
- The hover-square buttons (about lines 185, 203 and 222) become `fill_rounded` with `CONTROL_RADIUS` and `behind = paint.background`.
- Hit-testing and layout do not change: only the painted shapes do.

## 4. Tests

- `fill_bordered`: the outermost corner pixel equals `behind`; a pixel on the straight border equals the border color; a pixel one step inside the border equals the fill; at radius 0 it is two square fills; a rect too small for the border draws without panicking or leaving the rect.
- A palette paint test that the selected row's fill is inset (left gap `ROW_INSET_X`, top gap `ROW_INSET_Y`, outer corner equals the body color), and that the row's text rect is unchanged.
- A Search view paint test that a field box's outer corner equals `paint.background` and its border and fill pixels are right.
- High contrast: corners stay the fill and border colors with no blended pixels.

## 5. Risks and checks

- **A flat color behind each shape.** The palette body and the Search view's panel are flat, so the corner blend is exact. The palette's selected row sits on the body color, not on the border.
- **The palette row text.** Only the fill is inset, so the text position is unchanged; the build is for checking that the inset fill still looks balanced around it.
- **DPI.** Radius and insets scale through `scale`. Paint tests run at 96 DPI; scaling is covered by the existing `radius_for` and metrics tests.
- **Startup.** Nothing new is loaded or allocated. The palette and Search view paint only when shown.
- **Verification:** Clippy, the targeted tests for the changed modules, the full suite once, then the build. Before the build, `fastpad.ini` is backed up.

## 6. Next piece

- 3c: one keyboard focus ring (accent, 2px, 1px gap) replacing the dotted rectangle and the 1px outline, with focus shown in the tab strip and palette rows.
