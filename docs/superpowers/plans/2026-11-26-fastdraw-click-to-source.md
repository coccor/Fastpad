# FastDraw click-to-source and diagram toolbar: implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task by task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A `.fdraw` tab behaves like the FastDraw VS Code preview:
- Clicking an element selects its line in the text, switching to Split first.
- The caret line's elements are outlined in the diagram.
- A floating toolbar holds Fit, Fill, zoom and Open in FastDraw.

**Architecture:**
- **Hits.** The hit list (each element's box or points, plus its source line) is built in Rust from
  `fastdraw_render::drawn_scene`. It uses the same `Source` as the picture, and is carried to
  `ImageView` with the decoded image.
- **Clicks.** `ImageView` maps a click to scene coordinates, hit-tests, and posts the line to the
  main window. `preview_host` switches the mode and selects the line.
- **Toolbar and outlines.** Both are painted by `ImageView` in Direct2D, over the picture.

**Tech stack:** Rust, Win32 (windows-sys), Direct2D/DirectWrite (windows crate), Scintilla, and
`fastdraw-render` pinned at f0d3905.

**Spec:** `docs/superpowers/specs/2026-11-26-fastdraw-click-to-source-design.md`

## Global constraints

- **No change to the FastDraw repo, and no bump of the `fastdraw-render` pin.**
- **Only FastDraw tabs change.** Plain images and SVG tabs keep click-to-zoom, Ctrl+click
  zoom-out, and no toolbar.
- **Clicks never edit text or dirty the tab.** Dragging past the drag distance pans, and is never
  a click.
- **Hit order is VS Code's:**
  - The list is front to back: shapes in scene order, then connectors, then free texts.
  - The hit is the last match in that list.
  - Connector tolerance is 6 device-independent pixels, converted to scene units at the current
    zoom.
  - Elements with `line == 0` are never hit.
- **Lines:** scene lines are 1-based and Scintilla lines are 0-based. Convert only at the
  `preview_host` and editor boundary.
- **Fill** scales to the pane's width, up to `zoom::STEPS` max (16.0) and down to
  `STEPS[0] / 100.0`. **Fit** keeps its 100% cap.
- **Zoom persistence:**
  - A new document starts at Fit.
  - Zoom (Fit, Fill or a scale) survives edits of the same tab.
  - Nothing is saved across restarts.
- **Open in FastDraw** shows in the top editor toolbar only in **Source** mode. In Split and
  Preview it is in the diagram toolbar. The command, menu, palette and shortcut stay.
- **Accessibility:**
  - `image_view/accessible.rs` is one MSAA object with no children, and adding children would be
    a rewrite. So the toolbar is **not** exposed as MSAA children (design: "if it can hold child
    elements without a rewrite").
  - Instead, FastDraw views add keys: `F` for Fit, `W` for Fill (the image view handles neither
    today), alongside the existing `+`, `-` and `0`.
  - Each button has a tooltip.
- **Commits** never carry a co-author trailer.
- **Before running the app,** copy `native/out/x64` DLLs into `target/debug`.
- **When done,** run `cargo clean` and remove `native/src`; keep `native/out` and `native/cache`.

## Review focus

The cases most likely to bite a user, each pinned by a test in the task that owns the code:

1. **Clicking while a redraw is pending after an edit.** The old picture is still on screen.
   Expected: nothing is selected, rather than a wrong line. Test in Task 3,
   `a_click_while_an_edit_redraws_selects_nothing`.
2. **Clicking empty canvas, or a diagram in its error state (nothing drawn).** Expected: the mode
   stays as it is and nothing is selected. Test in Task 3,
   `a_click_on_empty_canvas_changes_nothing`.
3. **A drag that starts on an element.** Expected: it pans, and nothing is selected. Test in Task 3,
   `a_drag_from_an_element_pans_and_selects_nothing`.
4. **A toolbar button over a diagram element.** Expected: only the button acts. Test in Task 6,
   `a_toolbar_click_does_not_reach_the_diagram`.
5. **Zoomed and panned, or dark look.** Expected: the click still finds the right element. Test in
   Task 3, `scene_point_round_trips_at_any_zoom_and_offset`, plus the UI test run at 200%.

---

### Task 1: The hit list and hit test (pure)

**Files:**
- Create: `src/preview/fastdraw_hits.rs`
- Modify: `src/preview/mod.rs` (add `pub mod fastdraw_hits;`)

**Interfaces:**
- Produces:
  - `#[derive(Clone, Debug, PartialEq)] pub enum HitShape { Box { x: f64, y: f64, w: f64, h: f64 }, Line(Vec<(f64, f64)>) }`
  - `#[derive(Clone, Debug, PartialEq)] pub struct Hit { pub line: u32, pub shape: HitShape }`
  - `#[derive(Clone, Debug, Default, PartialEq)] pub struct Hits { pub origin: (f64, f64), pub size: (f64, f64), pub items: Vec<Hit> }`.
    `origin` is the scene's `(x, y)` and `size` its `(w, h)`, which together are the SVG viewBox.
  - `pub fn hits_of(scene: &fastdraw_render::scene::Scene) -> Hits`. The order is shapes, then
    conns, then texts. Elements with `line == 0` are skipped.
  - `pub fn hit_at(hits: &Hits, point: (f64, f64), tolerance: f64) -> Option<u32>`. `point` is in
    scene units. It returns the line of the last item containing the point: an inclusive box test,
    or for lines, distance to a segment within `tolerance`.
  - `pub fn on_line(hits: &Hits, line: u32) -> impl Iterator<Item = &HitShape>`.

- [ ] **Step 1: Write the failing tests** in `fastdraw_hits.rs` `mod tests`.
  - Build `Hits` by hand for the pure tests.
  - Build one from `fastdraw_render::drawn_scene(&Source { text, file: None, dark: false })` with
    text `"a: rect \"A\" at 0,0\nb: rect \"B\" at 200,0\na -> b\n"`.

  ```rust
  #[test] fn hits_follow_the_scene_order_shapes_lines_texts()   // lines [1,2,3] for the scene above (shapes 1,2 then the conn on line 3)
  #[test] fn the_later_of_two_overlapping_boxes_wins()          // boxes (0,0,100,100) line 1, (50,50,100,100) line 2 → hit_at((60.0,60.0),1.0)==Some(2); hit_at((10.0,10.0),1.0)==Some(1)
  #[test] fn a_line_is_hit_within_the_tolerance_only()          // Line [(0,0),(100,0)] line 5 → hit_at((50.0,5.0),6.0)==Some(5); hit_at((50.0,7.0),6.0)==None
  #[test] fn a_line_drawn_over_a_box_wins_over_the_box()        // box line 1 then Line through it line 2 → Some(2)
  #[test] fn empty_space_hits_nothing()                         // hit_at((-500.0,-500.0),6.0)==None
  #[test] fn elements_on_line_zero_are_never_hits()             // hits_of skips line 0 (construct a Scene via drawn_scene and assert all items have line > 0)
  #[test] fn on_line_lists_every_element_written_on_that_line() // two boxes on line 4 → 2
  ```

- [ ] **Step 2: Run and confirm failure.** Run `cargo test --lib fastdraw_hits`. Expected: a
  compile error (module missing).
- [ ] **Step 3: Implement** the three functions. Use point-to-segment distance as in
  `preview.js` `nearLine`, with `t` clamped to [0, 1]. A one-point line is a point.
- [ ] **Step 4: Run and confirm it passes.** Run `cargo test --lib fastdraw_hits`. Expected: 7
  passed.
- [ ] **Step 5: Commit.** `git commit -m "feat(fastdraw): hit list and hit test for diagram elements"`

### Task 2: Carry the hits with the drawing

**Files:**
- Modify: `src/preview/fastdraw.rs` (`Drawn`, `draw_unguarded`, `guarded`)
- Modify: `src/image_view/decode.rs` (`DiagramFacts`, `fastdraw_source`)

**Interfaces:**
- Consumes: `fastdraw_hits::{Hits, hits_of}`.
- Produces:
  - `Drawn.hits: std::sync::Arc<Hits>`. It is empty in `guarded`'s panic case and when
    `show == false`.
  - `DiagramFacts.hits: Arc<Hits>`. Drop `Eq` from `DiagramFacts` and keep `PartialEq`.
  - Private `fn cached_hits(source: &Source) -> Arc<Hits>` in `fastdraw.rs`: a single-entry
    `Mutex<Option<((u64, Option<PathBuf>, bool), Arc<Hits>)>>`, keyed on (a hash of the text,
    the file, dark).

- [ ] **Step 1: Write the failing tests** in `fastdraw.rs` `mod tests`:

  ```rust
  #[test] fn a_drawing_carries_a_hit_for_each_element_with_its_line()  // draw("a: rect \"A\" at 0,0\nb: rect \"B\" at 200,0\n", None, false).hits.items lines == [1,2]
  #[test] fn the_hits_share_the_svg_viewbox()                          // hits.origin/size equal the 4 numbers in the svg's viewBox attribute
  #[test] fn a_refused_look_hits_the_drawing_it_shows()                // with look "//server/x.look": hits non-empty and origin/size match that svg's viewBox
  #[test] fn a_hidden_drawing_has_no_hits()                            // text with "nosuchkind" error: show==false and hits.items.is_empty()
  #[test] fn drawing_the_same_source_twice_reuses_the_hits()           // Arc::ptr_eq(&draw(t,..).hits, &draw(t,..).hits)
  ```

- [ ] **Step 2: Run and confirm failure.** Run `cargo test --lib preview::fastdraw`. Expected: a
  compile error (no `hits` field).
- [ ] **Step 3: Implement.** In `draw_unguarded`, build `Source { text, file, dark }` once, with
  the same `file` and `dark` that `render` gets after the refusal logic. Pass it to both `render`
  and `cached_hits`. In `fastdraw_source`, copy `drawn.hits` into `DiagramFacts`.
- [ ] **Step 4: Run and confirm it passes.** Run
  `cargo test --lib preview::fastdraw image_view::decode`. Expected: all pass.
- [ ] **Step 5: Commit.** `git commit -m "feat(fastdraw): drawings carry their hit list"`

### Task 3: Click an element to select its line

**Files:**
- Modify: `src/image_view/zoom.rs`. Add the mapping functions.
- Modify: `src/image_view/mod.rs`:
  - add `hits: Option<Arc<Hits>>` to `ViewState`
  - set it in `accept_decoded`
  - clear it in `show_fastdraw` when the text or file changes, and in `clear_picture`
  - change the `WM_LBUTTONUP` click branch
  - update the `press` doc comment
- Modify: `src/window/messages.rs`. Add `WM_FASTPAD_DIAGRAM_CLICKED`: `WM_APP + <next free>`,
  wparam = view hwnd, lparam = 1-based line, posted to the root window.
- Modify: `src/window/main_window/wndproc.rs`. Dispatch it to `preview_host::diagram_clicked`.
- Modify: `src/window/preview_host.rs`. Add `diagram_clicked`.
- Modify: `src/editor/scintilla/document_text.rs`. Add `select_line`, with a `#[cfg(not(windows))]`
  twin as `go_to_line` has.
- Test: `tests/windows/image_preview.rs`.

**Interfaces:**
- Consumes: `fastdraw_hits::{Hits, hit_at}`, `DiagramFacts.hits`.
- Produces:
  - In `zoom.rs`:
    - `pub fn to_scene(client: (f32, f32), offset: (f32, f32), scale: f32, origin: (f64, f64)) -> (f64, f64)`.
      This is `(client - offset) / scale + origin`. `scale` is device pixels per scene unit,
      because the SVG's natural size is the viewBox size.
    - `pub fn from_scene(point: (f64, f64), offset: (f32, f32), scale: f32, origin: (f64, f64)) -> (f32, f32)`,
      the inverse.
  - `ImageView::hit_line(&self, client: (i32, i32)) -> Option<u32>`, used by the click and by tests.
  - `#[cfg(test)] ImageView::client_point(&self, scene: (f64, f64)) -> (i32, i32)`.
  - `pub fn select_line(&self, line: usize) -> Result<()>` on the Scintilla editor. It runs
    `SCI_SETSEL(SCI_POSITIONFROMLINE(line), SCI_GETLINEENDPOSITION(line))`, then
    `scroll_caret_into_view`.
  - `pub(crate) fn diagram_clicked(hwnd: HWND, view: HWND, line: u32)`.

- [ ] **Step 1: Write the failing unit test** in `zoom.rs`:

  ```rust
  #[test] fn scene_point_round_trips_at_any_zoom_and_offset()
  // for scale in [0.1, 1.0, 2.0, 16.0], offset in [(0,0), (-300,40)], origin (-12.5, 8.0):
  //   to_scene(from_scene((40.0, 70.0), ..), ..) ≈ (40.0, 70.0) within 1/scale
  ```

- [ ] **Step 2: Write the failing UI tests** in `image_preview.rs`:
  - Each opens `d.fdraw` = `"a: rect \"A\" at 0,0\nb: rect \"B\" at 200,0\n"`.
  - Each waits with `FDRAW_DRAW_TIMEOUT`.
  - Each clicks with a `click_at(view, x, y)` helper, a generalisation of `click`.
  - Each gets the point from `view.client_point(centre of B's hit box)`.

  ```rust
  #[test] fn clicking_a_diagram_box_opens_split_with_its_line_selected()
  // Preview → click B → mode == Split, editor selection == range of line index 1 ("b: rect …" without EOL), editor has focus
  #[test] fn clicking_a_box_at_200_percent_still_selects_its_line()   // view.set_zoom(Zoom::Scale(2.0)) first
  #[test] fn a_click_on_empty_canvas_changes_nothing()               // click far outside both boxes (and on an error-state diagram) → mode stays Preview, selection unchanged
  #[test] fn a_drag_from_an_element_pans_and_selects_nothing()       // zoom 4.0, DOWN on B, MOVE 50px, UP → offset changed, mode Preview
  #[test] fn a_click_while_an_edit_redraws_selects_nothing()         // set_text(new text) then click before the redraw lands → view.hit_line(..) == None and mode Preview
  #[test] fn a_click_still_zooms_a_plain_image()                     // existing a_click_zooms_the_image_in_… stays green unchanged
  ```

- [ ] **Step 3: Run and confirm failure.** Run `cargo test --test in_process image_preview`.
  Expected: the new tests fail (compile error or a wrong mode).
- [ ] **Step 4: Implement.**
  - In `WM_LBUTTONUP`, when `state.shown` is `Shown::FastDraw`, compute `hit_line`. The tolerance
    is `6.0 * dpi_scale / scale`.
    - On `Some(line)`, post `WM_FASTPAD_DIAGRAM_CLICKED`.
    - On `None`, do nothing. A FastDraw view never zooms on click.
  - `diagram_clicked` does the following:
    - It finds the group owning `view` (as `group_of_child` does).
    - If the mode is Preview, it calls `set_group_mode(.., ViewMode::Split)`.
    - It calls `editor.select_line(line - 1)` and `SetFocus(editor.hwnd())`.
- [ ] **Step 5: Run and confirm it passes.** Run `cargo test --test in_process image_preview` and
  `cargo test --lib image_view::zoom`. Expected: all pass.
- [ ] **Step 6: Commit.** `git commit -m "feat(fastdraw): clicking an element selects its line beside the diagram"`

### Task 4: Outline the caret line's elements

**Files:**
- Modify: `src/image_view/mod.rs`:
  - `ViewState.marked_line: Option<u32>`
  - `ImageView::set_marked_line`
  - `ImageStats.marked: usize`, the count of shapes outlined
- Modify: `src/image_view/paint.rs`. After the picture and tile, and before the band, draw the
  outlines.
- Modify: `src/window/main_window/wndproc.rs`. In the `SCN_UPDATEUI` with `SC_UPDATE_SELECTION`
  branch, call `preview_host::caret_moved(hwnd, group)` for FastDraw documents.
- Modify: `src/window/preview_host.rs`. Add `caret_moved`. Also push the line when the mode becomes
  Split and when a new image lands.
- Test: `tests/windows/image_preview.rs`.

**Interfaces:**
- Consumes: `fastdraw_hits::on_line`, `zoom::from_scene`.
- Produces:
  - `pub fn set_marked_line(&self, line: Option<u32>)`. It invalidates only when the value
    changes.
  - `pub(crate) fn caret_moved(hwnd: HWND, group: GroupId)`. It sends
    `Some(caret_line + 1)` in Split and `None` otherwise.

- [ ] **Step 1: Write the failing UI tests:**

  ```rust
  #[test] fn the_caret_line_outlines_its_element_in_split()
  // open d.fdraw, set Split, put caret on line index 1 → pump → view.stats().marked == 1; caret to line index 0 → marked == 1 and marked_line == Some(1)
  #[test] fn no_outline_in_preview_or_on_a_line_with_no_element()
  // Preview → marked == 0; Split with caret on an empty trailing line → marked == 0
  ```

- [ ] **Step 2: Run and confirm failure.** Run `cargo test --test in_process image_preview`.
  Expected: FAIL (no `marked` field).
- [ ] **Step 3: Implement.**
  - Paint each `on_line` shape mapped through `from_scene`.
  - Boxes are drawn as rectangles inflated by 2 DIP, and lines as polylines.
  - The stroke is 2 DIP in the preview's accent colour. Use the selection or link colour from
    `PreviewColors`, and if neither exists, a fixed `0x3B82F6`.
  - Draw it as a separate pass, without touching cached bitmaps.
- [ ] **Step 4: Run and confirm it passes.** Same command. Expected: PASS.
- [ ] **Step 5: Commit.** `git commit -m "feat(fastdraw): outline the elements on the caret's line"`

### Task 5: Fill zoom, and the F and W keys

**Files:**
- Modify: `src/image_view/zoom.rs`. Add `Zoom::FillWidth` and `fill_width_scale`.
- Modify: `src/image_view/mod.rs`:
  - add a `FillWidth` arm wherever `recompute`, `recompute_scale_only`, status and the accessible
    value match on `Zoom`
  - re-apply on `WM_SIZE` as Fit does
  - add the `F` and `W` keys in `WM_KEYDOWN` for FastDraw only
- Modify: any other exhaustive `match` on `Zoom` (`zoom_host.rs`, the status bar). The compiler
  lists them. FillWidth shows its percentage, as Fit does.

**Interfaces:**
- Produces:
  - `Zoom::FillWidth`.
  - `pub fn fill_width_scale(image: (f32, f32), view: (f32, f32)) -> f32`. This is
    `view.0 / image.0`, clamped to `[STEPS[0] / 100.0, STEPS[STEPS.len() - 1]]`, and 1.0 for an
    empty image.

- [ ] **Step 1: Write the failing tests:**

  ```rust
  // zoom.rs
  #[test] fn fill_width_enlarges_a_small_picture_past_100_percent() // fill_width_scale((100.,50.),(400.,300.)) == 4.0
  #[test] fn fill_width_shrinks_a_wide_picture_to_the_width()       // ((2000.,100.),(500.,300.)) == 0.25
  #[test] fn fill_width_is_capped_at_the_largest_step()             // ((10.,10.),(1000.,10.)) == 16.0
  // image_preview.rs
  #[test] fn w_fills_the_width_and_f_fits_a_fdraw()                 // WM_KEYDOWN 'W' → view.zoom()==FillWidth, scale ≈ client_width/natural_width; 'F' → Fit
  #[test] fn fill_survives_an_edit_of_the_diagram()                 // W, set_text(other diagram), redraw → zoom()==FillWidth
  ```

- [ ] **Step 2: Run and confirm failure.** Expected: compile errors.
- [ ] **Step 3: Implement.** The `show_fastdraw` reset to `Fit` stays only for `new_document`.
  Check that it already does this for edits.
- [ ] **Step 4: Run and confirm it passes.** Run `cargo test --lib image_view::zoom` and
  `cargo test --test in_process image_preview`. Expected: PASS.
- [ ] **Step 5: Commit.** `git commit -m "feat(fastdraw): Fill zoom, and F and W keys"`

### Task 6: The diagram toolbar

**Files:**
- Create: `src/image_view/toolbar.rs`. This holds the layout, hit-testing, painting and the
  hover/press state.
- Modify: `src/image_view/mod.rs`:
  - `ViewState.toolbar: toolbar::ToolbarState`, shown only for `Shown::FastDraw`
  - mouse routing: `WM_LBUTTONDOWN` on a button presses it and starts neither `press` nor `drag`;
    `WM_LBUTTONUP` on the same button acts
  - `WM_MOUSEMOVE` hover, and `WM_MOUSELEAVE` through `TrackMouseEvent`
  - a tooltip control (model it on `editor_toolbar::ensure_tooltip`)
- Modify: `src/image_view/paint.rs`. Paint the toolbar last, above the band.

**Interfaces:**
- Consumes: `Zoom::{Fit, FillWidth, Scale}`, `zoom::{step_in, step_out}`,
  `CommandId::OpenInFastDraw`.
- Produces:
  - `#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum Button { Fit, Fill, ZoomOut, Percent, ZoomIn, OpenInFastDraw }`,
    in this left-to-right order.
  - `pub fn layout(client_width: f32, dpi_scale: f32) -> [(Button, RectF); 6]`. The strip sits
    8 DIP from the top and right edges. Buttons are 28×28 DIP and Percent is 52 DIP wide.
    Padding is 4 DIP and the gap 2 DIP.
  - `pub fn button_at(client_width: f32, dpi_scale: f32, point: (f32, f32)) -> Option<Button>`.
  - `pub fn tooltip(button: Button) -> &'static str`. The texts:
    - "Fit the whole diagram (F)"
    - "Fill the width (W)"
    - "Zoom out (-)"
    - "Actual size (0)"
    - "Zoom in (+)"
    - "Open in FastDraw"
  - `#[cfg(test)] ImageView::press_button(&self, button: Button)`.

- [ ] **Step 1: Write the failing tests:**

  ```rust
  // toolbar.rs
  #[test] fn the_strip_sits_in_the_top_right_corner()     // layout(800.,1.0): last rect right == 792, first rect top == 8, buttons strictly left→right
  #[test] fn the_strip_scales_with_dpi()                  // layout(800.,2.0) widths are double layout(800.,1.0) widths
  #[test] fn button_at_finds_each_button_and_misses_gaps() // centre of each rect → that Button; a point 1px left of the strip → None
  // image_preview.rs
  #[test] fn the_toolbar_buttons_set_the_zoom()           // Fill → FillWidth, Fit → Fit, ZoomIn from 1.0 → 1.5, Percent → Scale(1.0)
  #[test] fn a_toolbar_click_does_not_reach_the_diagram() // diagram at a scale where box B lies under the Fit button; click Fit → mode stays Preview, zoom == Fit
  #[test] fn a_plain_image_has_no_toolbar()               // bmp tab: clicking where the Fit button would be zooms to 1.5 as before
  ```

- [ ] **Step 2: Run and confirm failure.** Expected: compile errors.
- [ ] **Step 3: Implement.**
  - **Painting:** a rounded background in the preview's surface colour, with a 1 DIP border in its
    border colour. Fit or Fill shows as on (filled) while it is the current zoom. Hover and press
    states are tints.
  - **Icons:** use a DirectWrite text format with "Segoe Fluent Icons", falling back to
    "Segoe MDL2 Assets", the font the editor toolbar draws from. The builder picks the glyphs.
    Percent is drawn as text in the body format.
  - **Open in FastDraw** posts `WM_COMMAND` with `CommandId::OpenInFastDraw` to the root window,
    which runs `preview_host::open_in_fastdraw`.
- [ ] **Step 4: Run and confirm it passes.** Run `cargo test --lib image_view::toolbar` and
  `cargo test --test in_process image_preview`. Expected: PASS.
- [ ] **Step 5: Commit.** `git commit -m "feat(fastdraw): zoom and open toolbar on the diagram"`

### Task 7: Open in FastDraw leaves the top toolbar outside Source mode

**Files:**
- Modify: `src/window/editor_toolbar.rs`. In `layout`, the `button` condition becomes
  `language == FastDraw && mode == ViewMode::Source`.
- Modify: `src/window/preview_host.rs`, if `set_mode` does not already re-lay-out the editor
  toolbar after a mode change. Make it do so.
- Test: `src/window/editor_toolbar.rs` tests, or `tests/windows/image_preview.rs`, whichever can
  reach `has_button`.

- [ ] **Step 1: Write the failing test:**

  ```rust
  #[test] fn open_in_fastdraw_is_on_the_top_toolbar_only_in_source()
  // fdraw tab: Preview → has_button == false; Split → false; Source → true; back to Preview → false
  ```

- [ ] **Step 2: Run and confirm failure.** Expected: FAIL, because `has_button` is true in Preview.
- [ ] **Step 3: Implement** the condition and the re-layout.
- [ ] **Step 4: Run and confirm it passes.** Then run the full suite: `cargo test` and
  `cargo clippy --all-targets -- -D warnings`. Expected: all green.
- [ ] **Step 5: Commit.** `git commit -m "feat(fastdraw): Open in FastDraw lives on the diagram toolbar"`

### Task 8: Manual check in the real app

- [ ] Copy the `native/out/x64` DLLs into `target/debug`, then run `cargo run` with a fresh
  `--new-window`. This avoids catching another session's single-instance traffic.
- [ ] Open `examples`, or any `.fdraw` with connectors and labels, and check each of these:
  - Clicking a box, a connector and a label each selects the right line in Split.
  - Clicking empty space does nothing, and dragging pans.
  - Moving the caret moves the outline.
  - Fit, Fill, −, %, + and Open in FastDraw all work, and each tooltip shows.
  - In dark mode, the outline and toolbar can be read.
  - The top toolbar's button shows only in Source.
- [ ] Clean up: run `cargo clean` and `rm -rf native/src`.
