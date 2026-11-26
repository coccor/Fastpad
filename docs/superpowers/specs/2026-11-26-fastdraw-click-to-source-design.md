# FastDraw diagram view: click to source, and a diagram toolbar

Date: 2026-11-26
Status: draft, awaiting approval (builder check done)

## Intent

A `.fdraw` tab in FastPad should behave like the FastDraw VS Code extension's preview
(`vscode/` in coccor/FastDraw, `media/preview.js`, `src/core.ts`):

- Clicking a box, a line or a piece of text in the diagram opens the text beside it with that
  element's line selected.
- The line the cursor is on has its elements outlined in the diagram.
- A small toolbar on the diagram holds fit, zoom and "Open in FastDraw".

The user asked for parity with the VS Code extension, so where this doc is silent, do what
`preview.js` does.

## Goals

1. **Click to source.** A click (press and release without dragging) on a diagram element does this:
   - In **Preview** mode, the tab switches to **Split**.
   - The editor gets focus, the caret goes to the element's line, the **whole line** is selected,
     and the line is scrolled into view. The whole-line selection is the user's own request; VS Code
     only puts the caret on the line.
   - When elements overlap, the topmost one wins, in the order VS Code uses (`hitsOf` in `core.ts`,
     `hitAt` in `preview.js`): free text over connectors over shapes, and among shapes the one drawn
     later. Elements with `line == 0` are never hits, as in VS Code.
   - Connectors are hit within a few pixels of their polyline. VS Code uses a screen-space tolerance
     through `nearLine`.
   - Clicking empty canvas does nothing.
2. **Outline the cursor's line.** In Split, every element whose `line` equals the editor caret's
   line gets an outline drawn over the picture: a rectangle for shapes and text, a polyline for
   connectors. It updates as the caret moves. It uses the accent or selection colour, as VS Code's
   `.fd-mark` does.
3. **Diagram toolbar**, floating in the diagram pane's top-right corner, drawn by the image view.
   It shows in both Split and Preview, for FastDraw tabs only. From left to right:
   - **Fit:** the whole diagram in view (today's default). Shown as on while active.
   - **Fill:** scale to the pane's width and scroll vertically. Shown as on while active.
     *My call:* Fill always fills the width, enlarging small diagrams past 100% up to the view's
     existing maximum zoom. Fit keeps today's cap of 100% (`zoom::fit_scale`), so the two buttons
     differ on small diagrams as well as tall ones.
   - **−**, then the **zoom %** (click it for 100%), then **+**. Same steps as `zoom::step_in` and
     `zoom::step_out`.
   - **Open in FastDraw:** the existing `CommandId::OpenInFastDraw` / `preview_host::open_in_fastdraw`.
   - Each button has a tooltip.
   - Fit and Fill are re-applied when the pane resizes or the diagram redraws. Any manual zoom
     (buttons, Ctrl+wheel, `+`/`-`/`0` keys) leaves them.
   - *My call on persistence:* the zoom (Fit, Fill or a scale) survives edits and redraws of the same
     tab, as today. A newly shown document starts at Fit. Nothing is saved across restarts.
   - *My call on accessibility:* the toolbar's buttons are exposed through the existing UIA module
     (`image_view/accessible.rs`) as buttons with names, if it can hold child elements without a
     rewrite. The keyboard already covers zoom (`+`, `-`, `0`). Add `F` for Fit and `W` for Fill only
     if they are free in the view.
4. **Move "Open in FastDraw".** It comes out of the top editor toolbar (`editor_toolbar.rs`, the
   `has_button`/`button_rect` path) whenever the diagram is visible (Split, Preview), and lives in the
   diagram toolbar. *My call:* in **Source** mode, where there is no diagram toolbar, the top-toolbar
   button stays. Otherwise Source mode would lose it, because that button only ever shows on FastDraw
   tabs (`editor_toolbar.rs:339`). The command and its menu, palette and shortcut entries stay.

## Non-goals

- A Problems panel, or squiggles from `fastdraw check`. FastPad keeps its error band.
- Light/dark settings, and the hosted-or-local editor choice.
- `.look` tabs (postponed, see `2026-11-17-fastdraw-look-tabs-design.md`).
- A "goto line" command-line argument or IPC message.
- Plain images and SVG tabs keep today's behaviour: click to zoom in, Ctrl+click to zoom out, and no
  toolbar.

## Anti-goals

- Clicking must never edit the text or mark the tab dirty.
- Dragging to pan must still work. A press that moves past the drag distance is a pan, not a click.
- Find what was clicked from the same scene that was drawn. A stale scene after an edit must not
  select the wrong line: drop the hit list when the text changes, and replace it when the new
  render arrives.

## Constraints and what exists

- **Renderer:** `fastdraw-render` is pinned at f0d3905.
  - `pub fn drawn_scene(&Source) -> scene::Scene` (lib.rs:138, public but `#[doc(hidden)]`, intended
    for parity tests; using it here is accepted) returns `shapes`, `texts` and `conns`,
    each with a `line: u32` and a box (`x, y, w, h`) or points (`pts`).
  - The SVG's viewBox is the scene's `x y w h` (`render_svg`), so scene coordinates are SVG user
    units. A view point maps to the scene through the view's current scale and offset, plus `sc.x`
    and `sc.y`.
  - No change to the FastDraw repo is needed.
- **Rendering path:** `ImageView::show_fastdraw(text, …)` → `start_svg_decode`, off the UI thread →
  `decode::fastdraw_source`, which calls `preview/fastdraw.rs::draw` and then `rasterize_svg`. When
  Direct2D can draw the SVG itself (`vector.rs`), no pixels are made at all.
  - `draw` runs again on every re-decode (zoom or resize), not only after edits.
  - Build the hit list inside `draw_unguarded`, from the same adjusted `Source` it passes to
    `render`: the file is dropped for a refused look, and dark is turned off for a refused dark look.
    Otherwise the hits won't match the picture. Carry the list in `Drawn` and deliver it with the
    rendered result.
  - Cache the hit list per (text, file, dark), so that zooming and resizing don't rebuild the scene
    or reread the look file.
  - A public render-with-scene function would avoid the second build, but it needs a renderer change
    and a pin bump. Not now.
- **Mouse:** `image_view/mod.rs` `WM_LBUTTONUP` (line 1280) currently zooms on click. For FastDraw it
  becomes "hit-test, and if something was hit, tell the host".
  - The image view is a child window, so it notifies the host with a posted or sent message, the way
    it already posts `WM_FASTPAD_IMAGE_DECODED`.
  - `preview_host` then switches the mode (`CommandId::MarkdownViewSplit` path, line ~690) and
    selects the line through the editor (Scintilla `SCI_GOTOLINE` plus `SCI_SETSEL` over the line,
    as in `editor/scintilla/line_ops.rs`).
- **Caret to preview:** nothing follows caret moves today. The Markdown top-line sync only reads
  the top line when a parse lands. The outline needs a new hook on `SCN_UPDATEUI` with a selection
  change (`main_window/wndproc.rs:782`). The hook passes the caret line to the active FastDraw tab's
  `ImageView` only when the line changes.
- **Toolbar drawing:** Direct2D, over the picture, in the image view.
  - It is DPI-aware, keeps its own hover and pressed states, and has tooltips (the editor toolbar's
    tooltip code, `ensure_tooltip`, is a model).
  - Clicks on the toolbar must not start a pan or a hit-test.
  - It uses the icon font the editor toolbar already uses.

## Approaches considered

- **Toolbar on the diagram (chosen).** It is drawn by the image view in the diagram pane's corner,
  matches VS Code, and stays next to the diagram in Split. The cost is new drawing, input and
  tooltip code in the image view.
- **Controls in the top editor toolbar row.** It reuses existing painting and tooltips, but the
  controls end up above the text half in Split and the row gets crowded. Rejected by the user.
- **Hit-testing:**
  - **Rust over `drawn_scene` (chosen).** The data is already public in the pinned renderer.
  - **`data-line` attributes in the SVG.** Rejected: resvg drops them, it needs a renderer change, and
    it means parsing the SVG again.

## Left to the builder

- The exact pixel sizes, padding and icon glyphs of the toolbar.
- Where the hit list is stored in `ImageView` state.
- Whether the outline is drawn into the cached picture or as a separate Direct2D pass. Prefer a
  separate pass, so moving the caret doesn't redraw the picture.
- Tests: hit order and tolerance (pure functions, unit-tested), view↔scene coordinate mapping at a few
  zooms and offsets, Fill scale, and the editor-toolbar button no longer showing on FastDraw tabs.
