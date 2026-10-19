# Live Markdown Editor, Phase 3: Block Rendering — Implementation Plan

> **For agentic workers:** Steps use checkbox (`- [ ]`) syntax for tracking. Work task by task; each task ends compiling, with its named tests green, and is committed on its own.

**Goal:** The Live view renders Markdown's block constructs. On every line the selection does not touch:
- **Lists:** bullets and numbers sit in a hanging indent; task items show a checkbox that toggles on click.
- **Quotes:** a bar per nesting level.
- **Rules:** a drawn line.
- **Code blocks:** the code font on a full-width code background.
- **Tables:** a grid with aligned columns.
- **Images:** local images show as thumbnails; anything else shows its alt text.

The caret's line stays revealed source, as in phase 2. Typing stays within 4 ms p95.

**Spec:** `docs/superpowers/specs/2026-10-18-markdown-live-editor-design.md`: §5 (multi-line constructs reveal by line), §6 (bullets, checkboxes, quote bars, rules, code and table backgrounds, grids, images), §8 (checkbox click) and §12 phase 3. Builds on phases 1 and 2 (`…-phase-1.md`, `…-phase-2.md`) and keeps their deviations.

**Tech stack:** as phase 2. Also used:
- `preview::inline_object::inline_box` for table cell spacers;
- `preview::images::ImageCache` and `preview::links::resolve_image_path` for thumbnails;
- `preview::layout::CODE_SCALE`.

## Architecture

Phase 2 styles a line from its inline spans. Phase 3 adds each line's **block context**, found by the same per-block parse:

```
parse_block ──► BlockSpans { spans, links, lines: Vec<LineBlock> }   (pure, per block, lazy)
                                     │
LiveDocument::line_block(line) ──────┤
                                     ▼
styles::line_style(spans, line block, revealed) ─► LineStyle { pieces, runs, heading, block: BlockStyle }
                                     │
LiveLayout::lay_out ─► LineLayout { layout, offsets, indent, decorations }   (DirectWrite)
                                     │
paint: backgrounds → decorations (bars, bullets, boxes, rules, grids, images) → selection → text
```

- **`spans.rs`:** each block's parse also yields, for every line that is not plain top-level prose, a `LineBlock`. It holds:
  - the line's **containers**, outermost first (quotes, and list items with their marker on the item's first line);
  - the line's **prefix**: the bytes a rendered line hides (indent, `>`, list marker, task box);
  - the line's **body**: text, code, a code fence, a rule, a table row or delimiter, or an image.
- **`blocks.rs`:** `LiveDocument::line_block(source, line_bytes)` returns the absolute `LineBlock` of a line, and `table_extent` returns where a table's lines start and end.
- **`offsets.rs`:** a piece can now be an **object**: a source range, possibly empty, shown as one U+FFFC unit. Table cell spacers use it.
- **`styles.rs`:** `LineStyle` gains `block: BlockStyle` (indent, marker, body kind). The prefix becomes hidden pieces, and table pipes become object pieces. Block-level effects also go in the pure table, such as code lines in the code font and checked tasks struck through and dimmed.
- **`layout.rs`:**
  - A line is laid out at `width − indent` and drawn at `x = indent`. Every x the layout reports or accepts (caret, hits, selection, rows) goes through `LineLayout::indent`, in one place.
  - Each line keeps `decorations`, in line coordinates, that the painter draws.
  - Table rows get spacer objects sized from the table's measured columns. Image lines get the image's height.
- **`view/paint.rs`:** draws the decorations and image bitmaps, and owns the `ImageCache`.

### Decisions this plan makes

1. **Containers become a hanging indent, drawn in the margin.**
   - On a rendered line, the prefix (leading spaces, every `>`, the list marker and its gap, a task's `[ ]`/`[x]` and its gap) is hidden. The text is laid out at `width − indent` and drawn at `x = indent`, so wrapped rows of a list item or quote line up under its text.
   - Indent per container: quote = `0.25 em` bar + `0.75 em` gap; list item = `1.5 em`, or `2 em` for an ordered list (its later lines do not know their marker's width).
   - Lazy continuation lines and an item's later paragraphs get the same containers without a marker.
   - The painter draws the decorations in the indent:
     - **quote bars**, one per level, spanning the line's full height so bars join across lines;
     - **bullets** `•` / `◦` / `▪` by list depth;
     - **ordered numbers** as written (`3.`, `7)`), right-aligned in the marker box;
     - **checkboxes**.
2. **Revealed lines stay source (phase 2 Decision 4), with one exception.** A revealed line has no indent and no decorations; it shows its prefix as Marker-coloured source at `x = 0`.
   - The exception is a **code block's lines (content and fences)**, which use the code font in both states. Typing in a code block then never re-wraps, and the revealed line keeps its height.
   - The cost is that a list item's text shifts left by its indent when the caret enters it, as in VS Code's preview-less source view. This is a review point for the manual check. If it reads badly, a later change can keep container indents on revealed lines.
3. **Code blocks.**
   - **Content lines:** the code font at `CODE_SCALE` with no inline spans, and a code background filled across the whole text width, not just under the glyphs. That fill is a decoration: one rectangle per line, so the background is continuous down the block.
   - **Indented code:** its four columns are hidden.
   - **Opening fence:** the backticks or tildes are hidden; the info string (language) shows Dim in the code font, right-aligned (`DWRITE_TEXT_ALIGNMENT_TRAILING`).
   - **Closing fence:** hides to an empty code-background row, like phase 2 deviation 5.
   - **Unclosed fence:** runs to the block's end, as the parser says.
4. **Tables are one layout per row, with spacer objects.**
   - On a rendered row, each cell's surrounding whitespace is hidden. Each pipe, and an empty object piece at the start of a row without a leading pipe, becomes an inline object (`inline_box(width, row height)`) whose width moves the next cell to its column. Alignment (`:--`, `:-:`, `--:`) splits the slack between the spacers before and after the cell.
   - Inline spans inside cells now render: phase 2's table suppression is lifted for cells, but not for the delimiter row. The header row is bold.
   - Rows do not wrap (`DWRITE_WORD_WRAPPING_NO_WRAP`).
   - The delimiter row hides to a thin row (`0.5 ×` row height) with the header's bottom border.
   - **Column widths:**
     - Columns are sized to their widest cell plus `0.5 em` padding on each side.
     - The layout measures each row's cell widths by laying the row out once with zero-width spacers and reading each cell's extent (`HitTestTextRange`).
     - Those widths are cached per row text (`HashMap<u64 /* row text hash */, Rc<[f32]>>` in `LiveLayout`, cleared when the fonts or width change).
     - Typing in one cell therefore measures one row. The columns are the per-column maxima over the table's rows (`StyledLines::table_lines`).
   - **Re-laying:** `restyle` widens any table row in its range to the whole table, recomputes the columns, and drops the rows whose style **or** columns changed.
   - **Too wide:** a table whose columns exceed the text width renders every row as revealed source. Horizontal scrolling is out of scope.
   - **Decorations:**
     - cell borders as hairlines in the Rule paint, the outer border included;
     - a header fill in `CodeBackground`;
     - every other body row striped in `CodeBackground` at half opacity.
   - **A revealed row** is plain source. Other rows keep the grid, re-laid only if the edit changes a column width.
5. **Images.**
   - **Thumbnail:** a line whose only content is one image (optionally wrapped in a link) with a local destination (`resolve_image_path` against the document's folder) renders as a thumbnail. All its text is hidden; the line's height is the image's, scaled to at most `300 ×` zoom-scale DIPs tall and to the text width, plus `0.25 em` above and below. The painter draws the bitmap there.
   - **Decoding** is async through a per-view `ImageCache` that posts `WM_FASTPAD_LIVE_IMAGE` (`WM_APP + 0x5E`) to the Live window. The painter requests each visible image line's path at the text width in device pixels.
   - **Until decoded:** the line renders like the fallback below.
   - **When decoded:** `drain` drops the layouts of laid-out lines whose style has an image, and the next paint lays them out at their real height. Phase 1's rule keeps the first visible line steady, so an image decoding above the screen does not move the view.
   - **Fallback:** a remote, missing or failed image (and any image inside a paragraph) hides `![` and `](…)` and shows its alt text Dim and italic. It never fetches anything.
   - **Device loss:** `release_bitmaps` runs on a new target.
   - **Folder:** the document's folder comes from `preview_host::group_document` through `LiveView::set_document_dir`.
6. **Checkbox clicks are edits; nothing else is.**
   - A plain click inside a rendered checkbox's rectangle replaces the box's inner byte (` ` ↔ `x`) through the hidden Scintilla. It is one undo action (`SCI_BEGINUNDOACTION` / `SCI_ENDUNDOACTION`), with the caret and selection kept where they were. It is the only code path in this phase that edits.
   - The hand cursor shows over a checkbox (no Ctrl needed) and over links with Ctrl (phase 2).
   - A click elsewhere on a decoration (bar, bullet, rule, image, table spacer) places the caret at the nearest text position on that line. An image or rule line places it at the line's start.
7. **Rules.** A thematic break hides its text, keeps one row, and draws a hairline across the text width at mid-row in the Rule paint.

## Global constraints

- **No new crates; no `windows` crate feature changes.** `inline_box`, `ImageCache`, `DWRITE_WORD_WRAPPING_NO_WRAP`, `DrawBitmap`, `DrawLine` and `FillEllipse` are in features the preview already uses.
- **Lazy graphics:** `assert_no_preview_imports()` and `launching_with_a_markdown_file_loads_no_preview_graphics_library` keep passing. `spans`, `blocks`, `styles` and `offsets` stay pure, naming no Win32 or DirectWrite type. The `ImageCache` is created with the view, never before.
- **Revealing never edits.** Only a checkbox click (Decision 6) edits, and it never moves the selection.
- **Incremental equals full.** After any edit sequence, every line's `LineBlock` from the incremental document equals a fresh parse's, as the spans already do.
- **Typing latency:** Live p95 ≤ 4 ms on:
  - the 1 MB `sample_markdown`;
  - the 10k-line note;
  - a new 300-item nested list;
  - a new 60-row table, typing in a cell.
- **No backward-compatibility code.** Phase 2's block suppression in `spans.rs` is replaced, not kept beside the new path.
- Compile with `cargo clippy --all-targets` during tasks, and run only the tests named in each step. The full suite runs once, in Task 9.
- Window tests run with `--test-threads=1`. Perf tests are always run with the `typing_in_live` filter, never as a bare `--ignored`, which would regenerate the icon assets.
- Any run of the real app is wrapped in a backup and restore of `%APPDATA%\FastPad\fastpad.ini`.
- Work continues on `feat/markdown-live-editor` (PR #72). The PR is merged only when phase 4 is done too. Commit messages carry no `Co-Authored-By` trailer.

## Deviations from the spec (decided while planning)

1. **Revealed lines drop their indent and decorations** (Decision 2). The spec says only that a revealed line shows its source; this plan keeps phase 2's colour-only rule, except that code-block lines stay in the code font.
2. **Columns are sized as the preview sizes them**, not padded as the source is. The spec's "columns sized to their widest cell, as the preview does" holds, and a table too wide for the view falls back to source (Decision 4).
3. **Checked task text is struck through and dimmed**, as the old Live spec drew it. The preview only checks the box.
4. **Fences and the table delimiter keep a row** (a thin row for the delimiter), so caret motion never meets a zero-height line (phase 2 deviation 5's reasoning).
5. **An image inside a paragraph shows its alt text.** Only a line holding nothing but an image renders a thumbnail.
6. **No syntax highlighting in code blocks** (spec §3, out of scope); the code is drawn in the Code paint.

## Review focus

1. **Every x goes through the indent.** Caret rects, hits, selection rects, Up/Down `goal_x` and row geometry stay consistent on indented lines and across indented and unindented lines. *(Tests: Task 5 `hits_and_carets_round_trip_on_an_indented_line`; Task 7 `up_down_keep_their_column_across_list_levels`.)*
2. **Incremental line blocks equal a full parse,** including edits that open or close a fence, turn a paragraph into a list or a table, or unindent a nested item. *(Task 2 `scripted_block_edits_match_a_full_parse`.)*
3. **Table columns follow edits.** Columns re-lay without re-laying unchanged rows, and the measurement cache stays correct when the fonts or width change. *(Task 5 `widening_a_cell_moves_its_column_in_every_row`, `typing_in_a_cell_that_stays_narrow_lays_out_one_row`.)*
4. **The caret stays steady** when a tall image line or a code block's font change sits above or under it as it changes state. *(Task 7 `moving_onto_an_image_line_keeps_the_caret_row_in_place`.)*
5. **The checkbox click is exactly one undoable edit**, never moves the selection, and is never triggered by revealing. *(Task 6 `clicking_a_checkbox_toggles_it_in_one_undo_step`.)*
6. **Images never block or fetch.** Remote destinations, UNC paths and missing files never touch the network or the disk beyond `metadata`. *(Task 4 `remote_and_unc_images_show_alt_text`.)*

---

## File structure

| File | Status | Responsibility |
|---|---|---|
| `src/live/spans.rs` | modify | `LineBlock`, `Container`, `Marker`, `Body`; per-line block context; cell spans in tables |
| `src/live/blocks.rs` | modify | `line_block`, `table_lines`; block contexts follow edits |
| `src/live/offsets.rs` | modify | object pieces |
| `src/live/styles.rs` | modify | `BlockStyle`; prefix hiding; block effects; `StyledLines::table_lines`; new `Paint`s |
| `src/live/layout.rs`, `src/live/layout/tests.rs` | modify | indent, decorations, code lines, tables, image heights, cell-width cache |
| `src/live/layout/tables.rs` | create | column measurement and spacer widths |
| `src/live/view.rs` | modify | `ImageCache`, `set_document_dir`, image drain, new colours |
| `src/live/view/paint.rs` | modify | decorations, bitmaps, image requests |
| `src/live/view/mouse.rs` | modify | checkbox toggle, hand over checkboxes, clicks on decorations |
| `src/live/view/window_proc.rs` | modify | `WM_FASTPAD_LIVE_IMAGE` |
| `src/window/messages.rs` | modify | `WM_FASTPAD_LIVE_IMAGE` |
| `src/window/live_host.rs` | modify | document folder; decoration colours |
| `tests/windows/markdown_live.rs` | modify | block rendering, checkbox, image, table and perf tests |
| `README.md`, the spec | modify | phase 3 note |

---

### Task 1: `live::spans` — block context per line (pure)

**Files:** `src/live/spans.rs`.

- [ ] New types:
  ```rust
  #[derive(Clone, Debug, PartialEq, Eq)]
  pub enum Marker {
      Bullet,
      /// `3.` or `7)`: the bytes drawn in the margin, relative to the line.
      Ordered(Range<usize>),
      /// The byte inside `[ ]` / `[x]`, relative to the line.
      Task { checked: bool, inner: usize },
  }

  #[derive(Clone, Debug, PartialEq, Eq)]
  pub enum Container {
      Quote,
      /// A list item at `depth` (1 = top level); `marker` only on the item's first line.
      Item { depth: u8, ordered: bool, marker: Option<Marker> },
  }

  #[derive(Clone, Copy, Debug, PartialEq, Eq)]
  pub enum Align { None, Left, Center, Right }

  #[derive(Clone, Debug, PartialEq, Eq)]
  pub enum Body {
      Text,
      Code,
      /// An opening fence: its info string, relative to the line; a closing fence has `None`.
      Fence { info: Option<Range<usize>> },
      Rule,
      /// Cell contents (whitespace trimmed) and the pipes, relative to the line.
      TableRow { header: bool, cells: Vec<Range<usize>>, pipes: Vec<usize>, aligns: Rc<[Align]> },
      TableDelimiter,
      /// The line's only content is this image.
      Image { dest: String, alt: Range<usize> },
  }

  #[derive(Clone, Debug, PartialEq, Eq)]
  pub struct LineBlock {
      /// The line's start, relative to the block.
      pub start: usize,
      /// Bytes a rendered line hides before its body, relative to the line.
      pub prefix: Range<usize>,
      pub containers: Vec<Container>,
      pub body: Body,
  }
  ```
  `BlockSpans` gains `lines: Vec<LineBlock>`, sorted by `start`. It is sparse: a top-level text line has no entry.
- [ ] Rules, revived from `b99b099^:src/live/spans.rs` (`item`, `task`, `quote_markers`, `code_block`, `table`, `pipe_positions`) and reshaped into per-line contexts:
  - **Quotes:** each `>` (up to three leading spaces, one optional following space) adds a `Quote` and extends `prefix`. A lazy continuation line inherits the previous line's quote depth.
  - **List items:**
    - **Marker line:** the item's first line gets `Item { marker: Some(..) }`, and its prefix covers the indent, the marker, the gap and a task box plus its gap.
    - **Later lines** of the item get `Item { marker: None }`. Their prefix covers the item's content indent, or less when the line is lazily indented.
    - **Depth:** nested lists add depth.
    - **Checked tasks:** the text gets `Strike` and `Dim` spans (deviation 3).
    - **Ordered markers:** the number and delimiter are the `Ordered` range.
  - **Code blocks:**
    - Fenced: the first line gets `Body::Fence { info }`; a closing fence line gets `Body::Fence { info: None }`; every line between gets `Body::Code`.
    - Indented: every line gets `Body::Code`, and the prefix covers four columns.
    - Container prefixes still apply.
  - **Tables:** row 0 is `TableRow { header: true }`, row 1 is `TableDelimiter`, and the rest are `TableRow { header: false }`. Alignments are parsed from the delimiter row and shared (`Rc<[Align]>`). Inline spans inside cells are emitted (the phase 2 suppression is lifted for `Tag::TableCell` content); pipes and cell whitespace emit no spans.
  - **Rules:** `Body::Rule`.
  - **Image lines:** a paragraph line whose events between the line's bounds are one `Image` (optionally inside one `Link`) and whitespace. The alt range is the image's inner text.
  - **HTML blocks** keep phase 2's `Dim` and suppression.
- [ ] Tests (some carried over from the old file):
  - **Lists:** `bullet_items_have_a_marker_and_a_hidden_prefix`, `nested_items_carry_their_depth`, `an_items_second_paragraph_has_no_marker`, `ordered_markers_keep_their_number_range`, `a_checked_task_is_struck_and_dim`, `an_unchecked_task_records_its_inner_byte`, `a_lazy_continuation_keeps_the_items_containers`.
  - **Quotes:** `quote_lines_carry_their_depth`, `a_list_inside_a_quote_has_both_containers`.
  - **Code blocks:** `fenced_code_marks_fences_and_code_lines`, `an_unclosed_fence_runs_to_the_block_end`, `indented_code_hides_four_columns`, `fenced_code_inside_a_list_item_keeps_the_item_prefix`.
  - **Tables:** `table_rows_have_cells_pipes_and_aligns`, `escaped_and_code_pipes_are_not_separators`, `table_cells_keep_inline_spans`.
  - **Other bodies:** `a_rule_is_a_rule_and_a_setext_underline_is_not`, `an_image_alone_on_its_line_is_an_image_line`, `an_image_inside_text_is_not`.
  - **Encoding:** `crlf_prefixes_and_cells_are_byte_exact`.
- [ ] `cargo test --lib live::spans`. Commit `feat(live): block context per line`.

### Task 2: `live::blocks` — line blocks follow the document (pure)

**Files:** `src/live/blocks.rs`.

- [ ] Add these functions to `LiveDocument`:
  ```rust
  /// The block context of the line whose bytes are `line`, absolute; `None` for plain prose.
  pub fn line_block(&self, source: &(impl SourceText + ?Sized), line: Range<usize>) -> Option<LineBlock>;
  /// The absolute starts of the first and last lines of the table holding `at` (a table's rows and
  /// delimiter share one `aligns` `Rc`, which is how they are found).
  pub fn table_extent(&self, source: &(impl SourceText + ?Sized), at: usize) -> Option<(usize, usize)>;
  ```
  Both use the same lazily parsed `BlockSpans` as `spans_in`: one parse per block serves spans, links and lines. A line inside the front matter has no block.
- [ ] `apply`'s changed-bytes result already covers the whole re-parsed block. Check that this holds when an edit changes only a block's line contexts (indenting an item) and not its spans.
- [ ] Tests:
  - `line_block_is_absolute`;
  - `indenting_an_item_changes_its_depth_and_reports_the_block`;
  - `typing_a_pipe_row_after_a_paragraph_makes_a_table`;
  - `front_matter_lines_have_no_block`;
  - extend `scripted_edits_match_a_full_parse` to compare every line's `line_block`. It becomes `scripted_block_edits_match_a_full_parse`, with a nested list, a quote, a table, both fence kinds and an image added to its note.
- [ ] `cargo test --lib live::blocks`. Commit `feat(live): line blocks follow edits`.

### Task 3: Object pieces and block styles (pure)

**Files:** `src/live/offsets.rs`, `src/live/styles.rs`.

- [ ] `OffsetMap::new` takes `(Range<usize>, Show)` with `enum Show { Text, Hidden, Object }`.
  - An object piece shows as one U+FFFC unit whatever its source length, even zero.
  - `to_utf16` of a byte inside an object maps to its start.
  - `to_byte(u)` on the object's unit maps to the object's source start; `to_byte_after` maps to its end.
  - `fn objects(&self) -> impl Iterator<Item = u32>` lists their UTF-16 positions.
  - The phase 2 call sites pass `Show::Text` / `Show::Hidden`.
- [ ] `styles.rs`:
  ```rust
  #[derive(Clone, Debug, Default, PartialEq, Eq)]
  pub struct BlockStyle {
      /// Containers drawn in the margin, outermost first; empty on revealed lines.
      pub containers: Vec<Container>,
      pub body: BodyStyle,
  }

  #[derive(Clone, Debug, Default, PartialEq, Eq)]
  pub enum BodyStyle {
      #[default]
      Text,
      /// Code font, full-width background; `fence` for fence lines; `info` is trailing-aligned.
      Code { fence: bool, info: bool },
      Rule,
      TableRow { header: bool, cells: usize, aligns: Rc<[Align]> },
      TableDelimiter,
      Image { dest: String },
  }
  ```
  `LineStyle` gains `block: BlockStyle`.
- [ ] `line_style(spans, line_block, line, revealed)`:
  - **Rendered lines:**
    - The prefix is hidden.
    - `Code` / `Fence` lines get no inline runs and one `Code` run over the shown text. A fence's markers are hidden, and its info shows as a `Dim` run.
    - `Rule` and `TableDelimiter` lines hide everything.
    - Table rows hide the whitespace around each cell and turn each pipe into an object. A row without a leading pipe gets an empty object at its start; one without a trailing pipe gets an empty object at its end. The header row's cells get a `Bold` run.
    - An `Image` line hides everything; `BodyStyle::Image` carries `dest`. The layout decides between thumbnail and alt-text fallback (Decision 5) and, for the fallback, re-styles the line through `LineStyle::image_fallback()`, which shows the alt range as `Dim` + `Italic`.
  - **Revealed lines:** as phase 2 (one shown piece, colour runs), except `Code`/`Fence` lines keep `BodyStyle::Code` (Decision 2). Their containers are empty.
- [ ] New `Paint`s: `Rule` (the Comment colour), `QuoteBar` (Comment), `Quote` (text inside a quote, also Comment, ranked below `Emphasis`). Rendered text inside a `Quote` container gets a `Quote` colour run under its other runs. All three use the existing `dim` brush (`LiveColors.dim` is already the Comment colour), so `LiveColors` gains nothing. The style-only runs are two new `SpanKind`s the parser never emits: `CodeBlock` (colour only; the body sets the font) and `Quote`.
- [ ] `StyledLines` gains `fn table_lines(&self, line: usize) -> Option<Range<usize>>` (the table's lines, from `table_extent`). `Styled` implements it; `MemoryText` returns `None`.
- [ ] Tests:
  - **Offsets:** `an_object_piece_is_one_unit`, `an_empty_object_maps_to_its_position`, `hits_on_an_object_land_on_its_edges`.
  - **Styles:**
    - containers and prefixes: `a_rendered_item_hides_its_prefix_and_keeps_its_containers`, `a_revealed_item_shows_its_prefix_without_containers`;
    - code blocks: `code_lines_use_the_code_body_in_both_states`, `a_fence_hides_its_markers_and_dims_its_info`;
    - tables: `table_pipes_become_objects_and_cells_are_trimmed`, `a_row_without_outer_pipes_gets_empty_objects`;
    - rules, images and quotes: `a_rule_hides_everything`, `an_image_line_hides_everything_and_falls_back_to_dim_alt`, `quote_text_is_coloured_below_links`.
- [ ] `cargo test --lib live::offsets live::styles`. Commit `feat(live): block styles and object pieces`.

### Task 4: Layout — indent, decorations, code, rules, images

**Files:** `src/live/layout.rs`, `src/live/layout/tests.rs`.

- [ ] `LineLayout` gains `indent: f32` and `decorations: Vec<Decoration>`:
  ```rust
  pub enum Decoration {
      /// A filled rectangle (line coordinates; `right == f32::MAX` = to the text's right edge).
      Fill { rect: RectF, paint: Paint },
      Line { from: (f32, f32), to: (f32, f32), paint: Paint },
      Bullet { center: (f32, f32), radius: f32, depth: u8 },
      /// An ordered marker's text, right-aligned in `rect`.
      Number { rect: RectF, text: String },
      Checkbox { rect: RectF, checked: bool, inner: usize },
      Image { rect: RectF, path: PathBuf },
  }
  ```
  - **Indent:** from Decision 1's container widths in em (`fonts.size`).
  - **Container decorations:** quote bars are `Fill`s from the line's top to its bottom. A marker sits on the first row's centre line, using the row's baseline (`DWRITE_LINE_METRICS`).
- [ ] **Indent in one place.**
  - `lay_out` builds the layout at `width − indent` and stores `indent`.
  - `caret_rect`, `position_at`, `hit_row`, `selection_rects`, `range_rects` and `rows_of` add or subtract `indent` through two helpers: `LineLayout::to_line_x` and `from_line_x`.
  - `Geometry::x_of` and `position_at_x` need no change, since they go through those helpers.
- [ ] **Code lines** (`BodyStyle::Code`):
  - The whole text uses the code family at `size × CODE_SCALE`, on the prose rows' spacing, so line numbers stay in step.
  - `Fill { 0..MAX, CodeBackground }` covers the line's height.
  - An opening fence with `info` sets `DWRITE_TEXT_ALIGNMENT_TRAILING`.
  - A closing fence keeps one code row.
- [ ] **Rules:** an empty layout with one row's height and `Line` at mid-row across the text width. **Table delimiters:** half a row's height.
- [ ] **Images:**
  - `StyledLines` gains `fn image(&self, dest: &str) -> ImageInfo`, with `enum ImageInfo { Ready { path, size: (u32, u32) }, Pending(PathBuf), Unavailable }`.
  - `Styled` gets it from an `images: &dyn Fn(&str) -> ImageInfo` field, which the view builds from its `ImageCache` and document folder.
  - **Ready:** an empty layout whose height is the scaled image plus padding, and `Image { rect, path }`.
  - **Otherwise:** the fallback style from Task 3.
  - `LineLayout` records `image: Option<PathBuf>`, so the view can drop image lines on drain.
- [ ] `estimate` stays byte-based. Image heights are known only once a line is laid out, and phase 1's first-visible-line anchor absorbs the jump.
- [ ] Tests:
  - **Indent:** `an_item_is_laid_out_at_its_indent`, `hits_and_carets_round_trip_on_an_indented_line` (every caret position on a wrapped item maps back through `position_at`), `wrapped_item_rows_start_at_the_indent`, `quote_bars_cover_the_line_height`, `nested_quote_and_list_indents_add_up`, `a_revealed_item_has_no_indent`.
  - **Code and rules:** `code_lines_use_the_code_font_and_fill_the_width`, `a_rule_is_one_row_with_a_line`.
  - **Images:** `a_ready_image_line_is_as_tall_as_its_scaled_image` (a fake `image` closure), `a_tall_image_is_capped_at_300_dips_times_zoom`, `a_pending_image_line_shows_its_alt_text`.
  - **Image security:** `remote_and_unc_images_show_alt_text` (`https://…` and `\\host\share\a.png` give `Unavailable` without a filesystem call: a counting closure).
- [ ] `cargo test --lib live::layout`. Commit `feat(live): indented lines, code blocks, rules and image lines`.

### Task 5: Layout — tables

**Files:** `src/live/layout/tables.rs`, `src/live/layout.rs`, `src/live/layout/tests.rs`.

- [ ] `tables.rs`:
  ```rust
  /// Each cell's natural width in a row, laid out with zero-width spacers.
  pub(super) fn measure_row(layout: &IDWriteTextLayout, offsets: &OffsetMap, cells: usize) -> Result<Vec<f32>>;
  /// Column widths: per-column maxima, plus padding.
  pub(super) fn columns(rows: &[Rc<[f32]>], padding: f32) -> Vec<f32>;
  /// The spacer widths placing each cell in its column with its alignment.
  pub(super) fn spacers(cells: &[f32], columns: &[f32], aligns: &[Align], padding: f32) -> Vec<f32>;
  ```
- [ ] **Layout cache:**
  - `LiveLayout` gains `cell_widths: HashMap<u64, Rc<[f32]>>`, keyed by a hash of the row's text and header flag and cleared in `set_fonts` / `set_width`.
  - It also gains `table_columns: HashMap<usize /* first line */, Vec<f32>>`, rebuilt for a table when one of its rows is restyled or laid out.
- [ ] **Laying out a table row:**
  - Find its table (`table_lines`).
  - Make sure every row's cell widths are cached; a missing row is measured, which builds a throwaway layout.
  - Compute the columns. If their sum exceeds the text width, lay the row out as revealed source (Decision 4).
  - Otherwise build the layout with `NO_WRAP`, set each object's inline box with `SetInlineObject(inline_box(spacer, row height), range)`, and add the decorations: hairlines and fills from the column edges.
  - `LineLayout` stores the `columns` it used.
- [ ] **`restyle`:** a table row in `lines` widens the range to its table. The table's columns are recomputed, and a row is dropped when its style changed or its stored `columns` differ.
- [ ] Tests:
  - **Alignment:** `cells_line_up_across_rows`, `alignment_places_the_slack`, `a_row_with_fewer_cells_leaves_the_rest_empty`.
  - **Following edits:** `widening_a_cell_moves_its_column_in_every_row`, `typing_in_a_cell_that_stays_narrow_lays_out_one_row` (a `#[cfg(test)]` lay-out counter).
  - **Fallback and states:** `a_table_wider_than_the_view_shows_as_source`, `a_revealed_row_is_source_and_the_rest_keep_the_grid`.
  - **Hits and fonts:** `hits_on_a_spacer_land_on_a_cell_edge`, `changing_the_width_remeasures`.
- [ ] `cargo test --lib live::layout`. Commit `feat(live): table grids`.

### Task 6: Painting decorations and images; checkbox clicks

**Files:** `src/live/view.rs`, `src/live/view/paint.rs`, `src/live/view/mouse.rs`, `src/live/view/window_proc.rs`, `src/window/messages.rs`, `src/window/live_host.rs`.

- [ ] **Colours:** The `Rule`, `QuoteBar` and `Quote` paints use the `dim` brush (Task 3), so no new colours are needed.
- [ ] **Paint order per visible line:**
  1. `Fill` decorations (code backgrounds, table header and stripes);
  2. phase 2's inline code backgrounds;
  3. the selection;
  4. `Line`, `Bullet` (`FillEllipse`, or `DrawEllipse` for depth 2 and a square `FillRectangle` for depth 3 and deeper), `Number` (a small trailing-aligned layout, cached per string in `LiveLayout`), `Checkbox` (the preview's box-and-fill drawing), and `Image` (`images.bitmap(target, path)` → `DrawBitmap` into `rect`, or nothing until it is ready);
  5. the text.

  Fills that extend to `f32::MAX` end at the text area's right edge.
- [ ] **Images:**
  - `ViewState` gains `images: ImageCache` (notify = the Live hwnd, `WM_FASTPAD_LIVE_IMAGE`) and `document_dir: Option<PathBuf>`.
  - `LiveView::set_document_dir(dir)` changes the folder. When it differs, it calls `images.clear()` and `reload`. `live_host::show_text` calls it before showing.
  - At paint, every visible line whose style is an image with a `Pending` path calls `images.request(path, text width in device px)`.
  - `WM_FASTPAD_LIVE_IMAGE` runs `images.drain()`. On a change, it drops every laid-out line whose `image` is `Some`, or whose style is an image line that fell back, and invalidates.
  - A new target calls `images.release_bitmaps()`.
- [ ] **Checkbox click** (Decision 6):
  - `WM_LBUTTONDOWN` without modifiers hit-tests the visible lines' `Checkbox` rects first.
  - On a hit, `editor.begin_undo_action()`, replace `inner..inner+1` with `x` or a space through `SCI_SETTARGETRANGE` / `SCI_REPLACETARGET`, then `end_undo_action()`.
  - The selection is restored to what it was. No caret move and no drag start.
  - `update_cursor` shows `IDC_HAND` over a checkbox.
- [ ] **Other decoration clicks:** a click on a rule or image line places the caret at the line's start. A click in an indent places it at the line's first text position; this falls out of `position_at` clamping, so check it rather than code it.
- [ ] **Tests** (`markdown_live.rs`):
  - **Painting:** `a_bullet_list_draws_bullets_in_the_margin` (`GetPixel` at a bullet's centre is the text colour; the text starts at the indent), `quote_bars_join_across_lines`, `a_rule_draws_a_line`, `a_code_block_background_fills_the_width`, `a_table_draws_its_grid_and_lines_up_columns` (each row's second cell has the same client x).
  - **Images:** `a_local_image_renders_as_a_thumbnail` (a PNG written to a temp folder next to a saved note; wait for `WM_FASTPAD_LIVE_IMAGE`; the line is taller than a row and a pixel inside matches the image), `a_missing_image_shows_its_alt_text`.
  - **Checkboxes:** `clicking_a_checkbox_toggles_it_in_one_undo_step` (text changes, one Ctrl+Z restores it, the selection is unchanged), `the_cursor_is_a_hand_over_a_checkbox`.
  - **Device loss:** `recreating_the_target_redraws_images`.
- [ ] `cargo test --test markdown_live -- --test-threads=1`. Commit `feat(live): paint block decorations, images and checkboxes`.

### Task 7: Motion and reveal across blocks

**Files:** `src/live/view.rs`, `src/live/layout.rs` (fixes only), `tests/windows/markdown_live.rs`.

Phase 2's reveal pass and caret anchoring already cover blocks. This task checks them through real input and fixes what the checks find.

- [ ] Tests:
  - **Lists and quotes:**
    - `up_down_keep_their_column_across_list_levels` (`goal_x` is the client x, so text at different indents lines up with the pointer, not the source column);
    - `moving_into_a_list_item_reveals_only_that_line`;
    - `enter_in_a_rendered_list_continues_it` (Enter goes through the writing helpers; the new item renders once the caret leaves it).
  - **Code blocks:** `typing_in_a_code_block_never_changes_its_height`.
  - **Tables:** `moving_through_a_table_reveals_one_row_at_a_time`; `leaving_a_table_row_formats_the_table_and_keeps_the_grid` (the writing helpers' format-on-leave runs, and every row re-lays with the new columns).
  - **Images and rules:** `moving_onto_an_image_line_keeps_the_caret_row_in_place`, `page_down_over_an_image_line_moves_by_a_page`, `clicking_a_rule_places_the_caret_on_it`.
  - **No dirtying:** `revealing_blocks_never_dirties` (arrows, clicks and Ctrl+End through every construct; `can_undo` and the save point are unchanged).
- [ ] `cargo test --test markdown_live -- --test-threads=1`. Commit `test(live): motion and reveal across block constructs` (with any fixes it needed, under `fix(live): …` commits of their own).

### Task 8: Typing latency on block constructs

**Files:** `tests/windows/markdown_live.rs`.

- [ ] Add the ignored perf tests:
  - `typing_in_live_inside_a_long_list_stays_in_budget` (caret in item 150 of a 300-item list nested three deep, so each keystroke reparses the whole list block);
  - `typing_in_live_inside_a_table_stays_in_budget` (60 rows by 5 columns, typing in row 30's middle cell);
  - `typing_in_live_inside_a_code_block_stays_in_budget` (a 500-line fence).

  All assert p95 ≤ 4 ms and print the numbers, like the phase 2 perf tests.
- [ ] Run all perf tests:

  ```
  cargo test --release --test markdown_live typing_in_live -- --ignored --test-threads=1 --nocapture
  ```

  If one is over budget, profile it first; the phase 2 lesson was a reload that parsed twice. The likely costs are:
  - re-laying unchanged table rows;
  - measuring every row instead of the edited one;
  - restyling a whole list on each keystroke when only the caret's item changed (the block reparse is expected; the re-layout is not);
  - number layouts created per frame instead of cached.
- [ ] Commit `test(live): block rendering perf`.

### Task 9: README, spec note, full verification

**Files:** `README.md`, the spec.

- [ ] README: the Live Markdown section says lists, task checkboxes (clickable), quotes, rules, code blocks, tables and local images now render, and that only find, Split sync and the command audit remain for the next release.
- [ ] Spec: a "Phase 3 shipped" note under §12 with this plan's deviations 1–6 and any found while building.
- [ ] Full suite:
  - `cargo clippy --all-targets -- -D warnings`;
  - `cargo test --no-fail-fast -- --test-threads=1` (the known failure `tabs::window_close_reviews_dirty_tabs_in_order_and_cancel_aborts_shutdown` is pre-existing on `main`);
  - the `typing_in_live` perf tests.
- [ ] Manual check in the real app, with `fastpad.ini` backed up and restored:
  - **The note:** every block construct nested inside each other (a task list in a quote, a fence in a list item, a table after a list), plus an image next to the note and a missing one.
  - **Movement and editing:** move through it with arrows, PgUp/PgDn, clicks and drags; toggle checkboxes and undo.
  - **Typing:** type in a table cell and a code block.
  - **Appearance:** light, dark and high contrast; 100% and 150% DPI; zoom.
  - **Layout:** Split with Live on the left.
  - **Review point:** judge Decision 2's indent shift on revealed list lines and record the verdict in the spec note.
- [ ] Commit `docs(live): phase 3 README and spec note` and push to `feat/markdown-live-editor` (PR #72 stays open until phase 4).

## Done when

- Every rendered line shows its block construct as Decision 1–7 describe: hanging-indented lists with bullets, numbers and clickable checkboxes; quote bars; rules; full-width code blocks; aligned table grids; local image thumbnails, with alt text for anything else.
- The caret's line shows its source; code-block lines keep the code font.
- A checkbox click is one undoable edit that leaves the selection alone; nothing else in Live edits on its own.
- Moving onto or off a tall image or a code block keeps the caret where it was on screen.
- Typing stays within 4 ms p95 on the 1 MB note, the 10k-line note, a long list, a table and a code block, and the full suite is green on CI.
