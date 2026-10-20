# Live Markdown Editor, Phase 2: Inline Rendering — Implementation Plan

> **For agentic workers:** Steps use checkbox (`- [ ]`) syntax for tracking. Work task by task; each task ends compiling, with its named tests green, and is committed on its own.

**Goal:** The Live view renders Markdown inline. Every source line except the ones the selection touches is drawn rendered: headings at real size with their `#` markers hidden, bold, italic and strike-through without their markers, inline code in the code font on the code background, links and wikilinks as underlined link text, and raw HTML, front matter, footnotes and math dimmed. The line holding the caret (every line a selection touches) is **revealed**: plain source in the prose font, coloured by Markdown role. Typing stays within the phase 1 budget (4 ms p95).

**Spec:** `docs/superpowers/specs/2026-10-18-markdown-live-editor-design.md`, §5, §6 (inline parts), §7.1, §7.2 and §12 phase 2. Builds on phase 1 (`docs/superpowers/plans/2026-10-18-markdown-live-editor-phase-1.md`) and keeps its deviations 1–11.

**Tech stack:** as phase 1. `pulldown-cmark` 0.13.4 (already a dependency) with the preview's options; `preview::incremental::PreviewDocument` for the block index; DirectWrite text ranges and drawing effects for styling.

## Architecture

Phase 1 lays out each source line as plain text: `LiveLayout::lay_out` builds `OffsetMap::plain(line)` and one `IDWriteTextLayout`. Phase 2 puts a **style** between the text and the layout:

```
Scintilla text ──SCN_MODIFIED──► EditLog ──flush──► LiveDocument (block index + spans per block)
                                                              │
selection ──► reveal::revealed_lines ──► revealed: Range<line>│
                                                              ▼
                                        styles::line_style(spans of the line, revealed?) ─► LineStyle
                                                              │
                                                              ▼
                     LiveLayout::lay_out: OffsetMap::new(line, pieces) + DirectWrite ranges ─► LineLayout
```

- **`spans.rs`** (pure, revived from `b99b099^:src/live/spans.rs`): one block's text → `Span`s (`Hidden`, `Bold`, `Italic`, `Strike`, `Code`, `Link`, `Heading(n)`, `Dim`) and `LinkSpan`s, in byte offsets relative to the block. Spans may overlap (`Bold` inside `Link`); only `Hidden` ranges are exclusive.
- **`blocks.rs`** (pure, revived from `b99b099^:src/live/blocks.rs`): `LiveDocument` wraps a `PreviewDocument` for block boundaries and keeps each block's spans, filled lazily, reparsing only the blocks an edit touched. Answers "the absolute spans overlapping these bytes" and "the link at this position".
- **`reveal.rs`** (pure): the selection → the contiguous range of revealed source lines (Live has one selection; phase 1 deviation 5).
- **`styles.rs`** (pure): a line's spans and its revealed flag → `LineStyle` (the shown/hidden pieces for `OffsetMap::new`, the styled runs, the heading level). `LineStyle` is `PartialEq`: a laid-out line is laid out again only when its style or text changed. The kind → DirectWrite effect table lives here, so it is tested without DirectWrite.
- **`layout.rs`**: lays a line out from its `LineStyle`: hidden pieces leave the layout string, runs become `SetFontWeight` / `SetFontStyle` / `SetStrikethrough` / `SetUnderline` / `SetFontFamilyName` + `SetFontSize`, a heading line gets its own size and line spacing. Colours are stored per layout as `(range, Paint)` and bound to brushes at paint time.
- **`view.rs`**: owns the `LiveDocument`, an `EditLog` and the revealed range; flushes pending edits before anything measures or paints; keeps the caret steady when the revealed set changes.

### Decisions this plan makes

1. **Horizontal caret motion stays Scintilla's (phase 1 deviation 3 holds).** The caret's line is always revealed, so on the caret's line source and screen positions agree, as in phase 1. Left/Right, Ctrl+Left/Right, Backspace and Delete never act on a rendered line: a move that leaves the line lands on the next or previous line, which the next reveal pass shows as source before it is drawn. The reveal pass runs from `selection_changed` and again at the start of every paint, so a frame never shows the caret on a rendered line. The view keeps no grapheme or word motion of its own. Where the view itself computes a target on a rendered line (Up/Down, PgUp/PgDn, mouse), it hit-tests the line **as it is drawn now** (rendered), maps the result back to a source byte through the line's `OffsetMap`, and the reveal follows. A hit never lands inside hidden markup: a leading hit on the first character of a styled run maps to the run's first byte (after the opening marker), and a trailing hit on its last character maps to the byte after that character (before the closing marker, new `OffsetMap::to_byte_after`). So a click on the right half of the last letter of `**bold**` puts the caret before the closing `**`, and typing extends the bold text. Up/Down keep their `goal_x` from the line the run started on, as in phase 1.
2. **Revealed-line height changes keep the caret steady.** When the revealed range changes (any selection change, an edit that moves the caret to another line), the view takes the new caret's rectangle in the layout **before** the change (the rendered line the user clicked or moved onto), applies the change (re-lays the lines that changed state), takes the caret's rectangle again, and moves `scroll_y` by the difference in `top`. The caret's row therefore keeps its screen position, whether the height change is above it (a heading above being un-revealed), on its own line (a heading becoming source) or both. Then `scroll_to_caret` runs as before, so a move that leaves the screen still scrolls. Edits keep phase 1's rule (the first visible line stays put); only a change of the revealed range anchors on the caret. With a selection, the anchor is the caret end. During a mouse drag the lines between anchor and caret reveal as the drag reaches them, and the caret (under the pointer) stays put.
3. **Per-line layouts get styled ranges from `LineStyle`, colours at paint time.** `styles::line_style` turns the absolute spans overlapping a line into line-relative pieces and runs; `layout.rs` maps each run's byte range to UTF-16 through the line's `OffsetMap` and applies the run's DirectWrite effects (from `styles::effects`, a pure table). Colours cannot be applied at layout time, because `ID2D1SolidColorBrush`es belong to the render target, which is created lazily and recreated on `D2DERR_RECREATE_TARGET`. So each `LineLayout` keeps `colors: Vec<(DWRITE_TEXT_RANGE, Paint)>` and `backgrounds: Vec<(u32, u32, Paint)>`; the painter keeps one brush per `Paint` with a `generation` counter, and before drawing a layout whose `bound` generation is older, calls `SetDrawingEffect(brush, range)` for its colour runs (drawing effects do not change metrics, so nothing is re-laid). Code backgrounds are filled from `HitTestTextRange` rectangles before the text is drawn.
4. **Revealed lines change colour only.** A revealed line has no hidden pieces and no size, weight, style or family changes: every span becomes a colour (markers in the Operator colour, heading text in the Heading colour, emphasis in the Emphasis colour, links in the Link colour, inline code in the code colours, dimmed ranges in the Comment colour), the same roles `languages/markdown.rs` gives the source view. Typing `**` on the caret line therefore never re-wraps the line beyond the characters typed. Inline code keeps its background on a revealed line (no metric change).

## Global constraints

- **No new crates; no `windows` crate feature changes.** DirectWrite range setters and `SetDrawingEffect` are in the features the preview already uses.
- **Lazy graphics** (phase 1): `assert_no_preview_imports()` and `launching_with_a_markdown_file_loads_no_preview_graphics_library` keep passing. The pure modules (`spans`, `blocks`, `reveal`, `styles`) name no Win32 or DirectWrite type.
- **Parse options:** `spans.rs` parses blocks with `LIVE_PARSE_OPTIONS = PARSE_OPTIONS.difference(Options::ENABLE_YAML_STYLE_METADATA_BLOCKS)`. Front matter is found by `blocks.rs`'s own scanner; parsing a block on its own with the YAML option would read a later `---` block as metadata. Footnotes and math stay off (the preview does not parse them); Live dims them lexically (deviation 4).
- **The core never names Scintilla** (spec §2). `SourceText` is implemented for `EditorText` and `MemoryText` in `text.rs`, so `LiveDocument` reads Scintilla through the existing `LiveText` adapter.
- **Revealing never edits.** No code path in this phase calls a Scintilla edit. Ctrl+click follows links without moving the caret.
- **Typing latency:** Live p95 ≤ 4 ms on the 1 MB `sample_markdown` and the 10k-line note (phase 1 deviation 11), now with reparsing and styled layouts.
- **No backward-compatibility code.** Phase 1's plain path (`OffsetMap::plain` in `lay_out`) is replaced, not kept beside the styled one.
- Compile with `cargo clippy --all-targets` during tasks and run only the tests named in the step. The full suite runs once, in Task 9.
- Window tests run with `--test-threads=1`. In a worktree, copy `native/out` DLLs in first.
- Any run of the real app is wrapped in a backup and restore of `%APPDATA%\FastPad\fastpad.ini`.
- Work on branch `feat/markdown-live-editor-phase-2`, cut from `feat/markdown-live-editor`; merge by PR once phase 1 is merged, with the CI checks green. Commit messages carry no `Co-Authored-By` trailer.

## Deviations from the spec (decided while planning)

1. **Heading sizes are the preview's**, `preview::layout::HEADING_SCALE` = 2.0 / 1.5 / 1.25 / 1.0 / 0.875 / 0.875, all bold. The spec says "1.8× / 1.5× / 1.25× / bold for H4–H6, as the preview"; the preview's own table wins, so Live and the preview agree. The constant is made `pub(crate)` and shared.
2. **Block constructs stay plain source in phase 2.** Lists, task items, quotes, rules, fenced and indented code, tables and images render as their source text (bullets, `>`, fences, pipes and `![…](…)` visible) until phase 3. Inline styling still applies inside list items and quotes, but not inside code blocks, tables or images. The revived span parser keeps the block handling it needs to get inline spans right (suppression inside code blocks, tables and HTML blocks) and drops the old block decorations; phase 3 brings decorations back.
3. **Revealed lines are colour-only** (Decision 4). The source view also bolds and italicises; Live does not, so that typing markers never re-wraps a revealed line.
4. **Footnotes and math are dimmed lexically.** The preview's options parse neither, so `[^label]` references, `[^label]:` definition lines, `$…$` and `$$…$$` are found by a small scanner over paragraph and heading source (skipping code spans and `\$`). A `[^label]` that pulldown-cmark resolves as a shortcut link (because a `[^label]: …` line is a valid link reference definition) is dimmed, not rendered as a link. Spans inside a math range are dropped, so `$a*b*c$` is not italicised.
5. **Setext underlines hide to an empty row.** A rendered setext heading's `===` / `---` line keeps one row of normal height with nothing drawn. Collapsing it to zero height would need motion to skip zero-height lines; not worth it in this phase.
6. **Edits are logged, then flushed.** `SCN_MODIFIED` records an `incremental::Edit` in an `EditLog` (64 edits, then a full reparse, as the preview) and shifts the line cache at once (phase 1). The block reparse and restyle run in `flush`, before the next measurement or paint. A replace-all of thousands of edits then reparses once.
7. **Block spans are parsed lazily.** Entering Live builds the block index for the whole note (needed for block boundaries) but parses a block's spans only when a line in it is styled. Opening a 4 MB note parses the index once and spans for the visible screen.
8. **A selection ending at the very start of a line does not reveal that line** (the old `reveal.rs` rule): it covers the previous line's line ending, not a character of the line.
9. **Ctrl+click on Markdown links lands in this phase**, with the hand cursor while Ctrl is held over a link: links render now, so they must also work. Destinations go through the preview's link path (`WM_FASTPAD_PREVIEW_LINK` → `preview_host::follow_link`), so web, mail, `#anchor` and local-file links behave as in the preview. Wikilinks keep phase 1's `follow_link_at`.

## Review focus

1. **Positions on rendered lines.** Hit tests, Up/Down targets and `to_byte_after` with hidden ranges, surrogate pairs and lines whose content is entirely hidden. *(Tests: Task 4, Task 6 `hit_test_on_a_rendered_line_never_lands_in_a_marker`, Task 7 `click_on_rendered_bold_places_the_caret_inside_it`.)*
2. **Incremental spans equal a full parse** after any edit sequence, including ones that open or close a fence, an emphasis run across lines, or the front matter. *(Tests: Task 2 `scripted_edits_match_a_full_parse`.)*
3. **The caret stays steady** when a heading above or under it changes between rendered and revealed. *(Tests: Task 7 `clicking_below_a_heading_keeps_the_caret_under_the_pointer`, `moving_onto_a_heading_keeps_the_caret_row_in_place`.)*
4. **Only changed lines are re-laid**, and a keystroke still costs one block reparse. *(Tests: Task 6 `restyle_keeps_lines_whose_style_did_not_change`, Task 9 perf.)*
5. **Brushes never outlive the target.** A recreated target re-binds every layout's colours before drawing it. *(Test: Task 7 `recreating_the_target_rebinds_colours`.)*
6. **Revealing never edits or dirties.** *(Test: Task 7 `moving_the_caret_through_rendered_lines_never_dirties`.)*

---

## File structure

| File | Status | Responsibility |
|---|---|---|
| `src/preview/incremental.rs` | modify | `PreviewDocument::refdef_dest`; `Edit::from_change` shared by the preview host and Live |
| `src/preview/layout.rs` | modify | `HEADING_SCALE`, `CODE_SCALE` become `pub(crate)` |
| `src/window/preview_host.rs` | modify | `record_edit` uses `Edit::from_change` |
| `src/live/mod.rs` | modify | declare `spans`, `blocks`, `reveal`, `styles` |
| `src/live/spans.rs` | create (revived) | block text → spans and links |
| `src/live/blocks.rs` | create (revived) | `LiveDocument`: block index, lazy spans, front matter, edits |
| `src/live/reveal.rs` | create (revived) | selection → revealed line range |
| `src/live/styles.rs` | create | `LineStyle`, `Paint`, `Effect`, `line_style`, `effects`, `StyledLines`, `Styled` |
| `src/live/text.rs` | modify | `SourceText` for `EditorText` and `MemoryText` |
| `src/live/offsets.rs` | modify | `to_byte_after`, `hidden_ranges` |
| `src/live/layout.rs`, `src/live/layout/tests.rs` | modify | styled layouts, `restyle`, heading line spacing, colour runs, backgrounds |
| `src/live/view.rs` | modify | document, edit log, flush, reveal pass, caret anchor, `LiveColors` roles |
| `src/live/view/paint.rs` | modify | `Paint` brushes, binding, backgrounds, reveal pass at paint start |
| `src/live/view/mouse.rs` | modify | Ctrl+click on Markdown links, hand cursor |
| `src/live/view/keys.rs` | modify | Ctrl down/up refresh the cursor |
| `src/window/live_host.rs` | modify | `edited` passes the `Edit`; `appearance` fills role colours and the code font |
| `tests/windows/markdown_live.rs` | modify | rendering, reveal, anchor, link and perf tests |
| `README.md`, the spec | modify | phase 2 note |

---

### Task 1: Preview support

**Files:** `src/preview/incremental.rs`, `src/preview/layout.rs`, `src/window/preview_host.rs`.

- [ ] `PreviewDocument::refdef_dest(&self, label: &str) -> Option<&str>`: the destination of the definition whose `key` equals `normalize_label(label)` (the old `blocks.rs` used it; it no longer exists).
- [ ] `Edit::from_change(position: usize, length: usize, inserted: bool, lines_delta: isize) -> Edit`, and `preview_host::record_edit` built on it, so the preview and Live convert `SCN_MODIFIED` the same way.
- [ ] `HEADING_SCALE` and `CODE_SCALE` in `preview/layout.rs` become `pub(crate)`.
- [ ] Tests: `refdef_dest_matches_labels_case_insensitively`, `edit_from_an_insertion_and_a_deletion`.
- [ ] `cargo test --lib preview::incremental`. Commit `refactor(preview): reference lookups and edit conversion for Live`.

### Task 2: `live::spans` and `live::blocks` — revive the span parser and block index (pure)

**Files:** `src/live/mod.rs`, `src/live/spans.rs`, `src/live/blocks.rs`, `src/live/text.rs`.

- [ ] Start from `git show b99b099^:src/live/spans.rs` and `…/blocks.rs`. New span model:
  ```rust
  pub const LIVE_PARSE_OPTIONS: Options =
      PARSE_OPTIONS.difference(Options::ENABLE_YAML_STYLE_METADATA_BLOCKS);

  #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
  pub enum SpanKind { Hidden, Bold, Italic, Strike, Code, Link, Heading(u8), Dim }

  #[derive(Clone, Debug, PartialEq, Eq)]
  pub struct Span { pub range: Range<usize>, pub kind: SpanKind }

  #[derive(Clone, Debug, PartialEq, Eq)]
  pub struct LinkSpan { pub range: Range<usize>, pub dest: String, pub wiki: bool }

  #[derive(Clone, Debug, Default, PartialEq, Eq)]
  pub struct BlockSpans { pub spans: Vec<Span>, pub links: Vec<LinkSpan> }

  pub fn parse_block(text: &str, refs: &dyn Fn(&str) -> Option<String>) -> BlockSpans;
  ```
  Spans may overlap, except that `finish` merges `Hidden` ranges and clips every other span out of them. `BoldItalic` goes (DirectWrite applies `Bold` and `Italic` independently); strike-through becomes a `Strike` span instead of the old separate `strikes` list.
- [ ] Rules, element by element:
  - **ATX heading:** leading `#`s and their gap, and a closing `#` run, are `Hidden`; the text is `Heading(n)` (keep `heading_spans`). **Setext:** text lines `Heading(n)`, the underline line `Hidden` (deviation 5). Emphasis inside a heading still emits `Bold`/`Italic` on top of `Heading(n)`.
  - **Strong / emphasis / strike:** markers `Hidden` (`wrap`), text `Bold` / `Italic` / `Strike`.
  - **Inline code:** backtick runs `Hidden`, content `Code`.
  - **Links:** track the first inner event's start (`text_start`) and the last inner event's end (`text_end`); `whole.start..text_start` and `text_end..whole.end` are `Hidden`, `text_start..text_end` is `Link`. This covers `[text](url)`, `[text][ref]`, `[text]`, wikilinks `[[Note]]` / `[[Note|Alias]]` / `[[Note#Heading]]` (`LinkType::WikiLink`, `wiki: true`) and autolinks `<url>` (brackets hidden). A link with no inner events (`[](url)`) hides nothing. `LinkSpan.range` is the whole source range.
  - **Footnote-shaped links:** a link whose source starts with `[^` emits `Dim` over its whole range and no `Link`/`Hidden` (deviation 4).
  - **Raw HTML** (inline and blocks): `Dim`.
  - **Footnotes and math:** `fn lexical_dims(text: &str, range: Range<usize>) -> Vec<Range<usize>>` scans each paragraph's and heading's source range for `[^label]` (no spaces in the label), a `[^label]:` definition at a line start (up to three spaces) through the end of the paragraph, `$$…$$` and `$…$` (an opening `$` not followed by whitespace, a closing `$` not preceded by whitespace and not followed by a digit), skipping code spans and backslash escapes. Each range is `Dim`; `finish` drops every other span that lies inside one and clips spans that cross it.
  - **Block constructs** (deviation 2): list items, quotes, code blocks, tables, rules, images emit nothing of their own. Code blocks, tables and HTML blocks raise `suppress`, so their text gets no inline spans. Images emit nothing and do not suppress their alt text's emphasis; their source shows as written.
- [ ] `blocks.rs`:
  ```rust
  pub struct LiveDocument {
      preview: PreviewDocument,
      /// Each block's spans, relative to its start; `None` until a line in the block is styled.
      spans: RefCell<Vec<Option<Rc<BlockSpans>>>>,
      front_matter: usize,
  }
  impl LiveDocument {
      pub fn parse(source: &str) -> Self;
      /// Applies edits already in `source`; returns the bytes whose spans may have changed.
      pub fn apply(&mut self, source: &(impl SourceText + ?Sized), edits: &[Edit]) -> Range<usize>;
      pub fn reparse(&mut self, source: &(impl SourceText + ?Sized));
      /// Absolute spans overlapping `bytes`, sorted by start; parses blocks it has not parsed yet.
      pub fn spans_in(&self, source: &(impl SourceText + ?Sized), bytes: Range<usize>) -> Vec<Span>;
      pub fn link_at(&self, source: &(impl SourceText + ?Sized), position: usize) -> Option<LinkSpan>;
      pub fn front_matter_end(&self) -> usize;
  }
  ```
  `apply` keeps the old logic (preview `Update`, re-parse every block an edit touched, front matter moving restyles from 0) but sets the touched blocks' slots to `None` instead of parsing them. `spans_in` adds `0..front_matter` as one `Dim` span and skips blocks that start inside the front matter. References resolve through `PreviewDocument::refdef_dest`.
- [ ] `text.rs`: `impl SourceText for EditorText<'_>` (as `preview_host::ScintillaSource`: `range_bytes` + lossy UTF-8) and for `MemoryText`.
- [ ] Tests, `spans`: carried over from the old file with the new kinds: `strong_hides_its_markers`, `nested_emphasis_is_bold_and_italic`, `strikethrough_hides_markers`, `inline_code_hides_backticks`, `inline_link_shows_only_its_text`, `reference_links_resolve_through_refs`, `autolink_hides_angle_brackets`, `atx_heading_splits_marker_and_text`, `setext_heading_hides_its_underline`, `emphasis_inside_a_heading_is_heading_and_bold`, `inline_html_is_dim`, `crlf_offsets_are_byte_exact`, `multibyte_text_keeps_byte_offsets`, `a_setext_heading_whose_text_starts_with_a_hash_is_setext`. New: `wikilink_shows_its_target`, `piped_wikilink_shows_its_alias`, `wikilink_with_a_heading_shows_it`, `empty_link_text_hides_nothing`, `bold_inside_a_link_is_both`, `footnote_reference_is_dim`, `footnote_definition_is_dim`, `a_resolved_footnote_reference_is_dim_not_a_link`, `inline_math_is_dim_and_unstyled_inside`, `display_math_across_lines_is_dim`, `a_dollar_amount_is_not_math` (`costs $5 and $6`), `code_blocks_and_tables_get_no_inline_spans`, `list_items_and_quotes_keep_inline_spans`, `hidden_ranges_never_overlap_other_spans`.
- [ ] Tests, `blocks`: carried over: `spans_are_absolute`, `typing_an_open_fence_matches_a_full_parse`, `typing_a_lone_strong_marker_matches_a_full_parse`, `reference_links_use_definitions_elsewhere_in_the_document`, `apply_reports_the_changed_bytes`, `front_matter_is_one_dim_span`, `front_matter_end_with_crlf`, `a_rule_without_a_closing_fence_is_not_front_matter`, `closing_the_front_matter_restyles_from_the_start`. New: `spans_parse_lazily_and_once` (a counter under `#[cfg(test)]`), `link_at_finds_a_wikilink_and_a_link`, `a_later_rule_block_is_not_read_as_front_matter`, and the property test `scripted_edits_match_a_full_parse` (a fixed-seed sequence of 500 random single-character and multi-line inserts and deletes over a note with every inline element, a fence and front matter; after each, `spans_in(0..len)` equals a fresh `LiveDocument::parse`'s).
- [ ] `cargo test --lib live::spans live::blocks`. Commit `feat(live): span parser and incremental block index`.

### Task 3: `live::reveal` and `live::styles` (pure)

**Files:** `src/live/reveal.rs`, `src/live/styles.rs`, `src/live/mod.rs`.

- [ ] `reveal.rs` (revived, narrowed to one selection):
  ```rust
  /// The source lines the selection `anchor..caret` touches (deviation 8 for an end at a line start).
  pub fn revealed_lines(anchor: usize, caret: usize, line_of: impl Fn(usize) -> usize) -> Range<usize>;
  /// `revealed` after an edit on `first_line` that added or removed `lines_delta` lines.
  pub fn shift(revealed: Range<usize>, first_line: usize, lines_delta: isize) -> Range<usize>;
  /// Lines revealed in exactly one of `old` and `new` (at most two ranges).
  pub fn changed(old: Range<usize>, new: Range<usize>) -> Vec<Range<usize>>;
  ```
- [ ] `styles.rs`:
  ```rust
  /// A colour role the painter keeps a brush for.
  #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
  pub enum Paint { Text, Heading, Emphasis, Marker, Link, Code, CodeBackground, Dim }

  #[derive(Clone, Copy, Debug, PartialEq, Eq)]
  pub enum Effect { Bold, Italic, Strike, Underline, CodeFont, Color(Paint), Background(Paint) }

  #[derive(Clone, Debug, Default, PartialEq, Eq)]
  pub struct LineStyle {
      pub revealed: bool,
      /// The line's heading level when rendered; `None` on revealed lines.
      pub heading: Option<u8>,
      /// Shown/hidden pieces covering the line, for `OffsetMap::new`.
      pub pieces: Vec<(Range<usize>, bool)>,
      /// Line-relative byte ranges and their span kinds, Hidden excluded.
      pub runs: Vec<(Range<usize>, SpanKind)>,
  }

  /// The effects `kind` has on a rendered or a revealed line.
  pub fn effects(kind: SpanKind, revealed: bool) -> &'static [Effect];
  /// The style of the line whose bytes (line ending excluded) are `line`, from the absolute `spans`.
  pub fn line_style(spans: &[Span], line: Range<usize>, revealed: bool) -> LineStyle;

  /// Lines with their styles: what `LiveLayout` lays out.
  pub trait StyledLines: LiveText {
      fn line_style(&self, line: usize) -> LineStyle;
  }
  /// The document text, its spans and the revealed lines.
  pub struct Styled<'a, T: LiveText> { pub text: &'a T, pub document: &'a LiveDocument, pub revealed: Range<usize> }
  impl<T: LiveText + SourceText> LiveText for Styled<'_, T> { /* delegates */ }
  impl<T: LiveText + SourceText> StyledLines for Styled<'_, T> { /* spans_in(line bytes) → line_style */ }
  #[cfg(test)] impl StyledLines for MemoryText { /* plain: one shown piece, no runs */ }
  ```
  The effect table (Decision 4): rendered `Heading(_)` → `Color(Heading)` (size and weight come from `heading`), `Bold` → `Bold`, `Italic` → `Italic`, `Strike` → `Strike`, `Code` → `CodeFont, Color(Code), Background(CodeBackground)`, `Link` → `Underline, Color(Link)`, `Dim` → `Color(Dim)`. Revealed: `Hidden` → `Color(Marker)` (on a revealed line `Hidden` ranges become runs), `Heading(_)` → `Color(Heading)`, `Bold`/`Italic` → `Color(Emphasis)`, `Strike`/`Dim` → `Color(Dim)`, `Code` → `Color(Code), Background(CodeBackground)`, `Link` → `Color(Link)`. When two runs colour the same bytes, the later kind in `Dim, Link, Code, Heading, Marker, Emphasis` order loses (Dim wins over everything).
  A line is a heading line when a `Heading(n)` span covers its first non-hidden byte; a setext underline has no shown bytes and no `heading`.
- [ ] Tests: `reveal`: `a_caret_reveals_its_line`, `a_selection_reveals_every_line_it_touches`, `a_selection_ending_at_a_line_start_does_not_reveal_that_line`, `a_backwards_selection_reveals_the_same_lines`, `shift_follows_lines_added_above_and_inside`, `changed_is_the_symmetric_difference`. `styles`: `a_rendered_bold_line_hides_markers_and_bolds_the_text`, `a_revealed_line_has_one_shown_piece_and_marker_colours`, `a_heading_line_has_its_level_only_when_rendered`, `a_setext_underline_is_all_hidden`, `spans_are_clipped_to_the_line`, `a_span_crossing_lines_styles_each_line`, `dim_colour_wins`, `effects_never_change_metrics_on_revealed_lines` (no `Bold`, `Italic`, `CodeFont` in any revealed entry).
- [ ] `cargo test --lib live::reveal live::styles`. Commit `feat(live): reveal range and line styles`.

### Task 4: `OffsetMap` for hit tests on rendered lines (pure)

**Files:** `src/live/offsets.rs`.

- [ ] Add:
  ```rust
  /// The byte just past the character whose last UTF-16 unit is `utf16 - 1`: where a trailing
  /// hit on that character lands, before any hidden range that follows it. 0 for 0.
  pub fn to_byte_after(&self, utf16: u32) -> usize;
  /// The UTF-16 range showing bytes `range`, with hidden bytes collapsed.
  pub fn to_utf16_range(&self, range: Range<usize>) -> Range<u32>;
  ```
- [ ] Tests: `trailing_hit_on_the_last_bold_letter_stays_inside`, `trailing_hit_before_a_hidden_link_tail_stays_in_the_text`, `trailing_hit_on_an_astral_char_lands_after_both_units`, `a_fully_hidden_line_maps_everything_to_zero_and_the_end`, `utf16_range_of_a_run_skips_hidden_bytes`.
- [ ] `cargo test --lib live::offsets`. Commit `feat(live): offset maps for hits on rendered lines`.

### Task 5: Styled line layouts

**Files:** `src/live/layout.rs`, `src/live/layout/tests.rs`.

- [ ] `LiveFonts` gains `code_family: String, code_weight: DWRITE_FONT_WEIGHT` (from `settings.font_face`, resolved with `resolve_family`) and keeps `size` / `line_height` for body text. `LiveFonts::new` takes the code face.
- [ ] Every `LiveLayout` method that may lay out a line takes `&impl StyledLines` instead of `&impl LiveText` (`ensure`, `line`, `caret_rect`, `position_at`, `selection_rects`, `geometry`, `reset`, `edit`, `set_width`, `set_fonts`).
- [ ] `lay_out(&self, text: &str, style: LineStyle) -> Result<LineLayout>`:
  - `OffsetMap::new(text, style.pieces.iter().cloned())`.
  - Heading line (`style.heading = Some(n)`): `SetFontSize(size × HEADING_SCALE[n-1])` and `SetFontWeight(BOLD)` over the whole text, and `SetLineSpacing(UNIFORM, line_height × scale, baseline × scale)` on the layout (an `IDWriteTextLayout` is an `IDWriteTextFormat`; the format's uniform spacing would clip a large heading).
  - Each run: `offsets.to_utf16_range(range)`, then its `effects(kind, revealed)`: `Bold` → `SetFontWeight(SEMI_BOLD)` (the preview's weight), `Italic` → `SetFontStyle(ITALIC)`, `Strike`/`Underline` → the setters, `CodeFont` → `SetFontFamilyName(code_family)` + `SetFontSize(size × CODE_SCALE)`; `Color(p)` → pushed to `colors`; `Background(p)` → pushed to `backgrounds`.
  - `LineLayout` gains `style: LineStyle`, `colors: Vec<(DWRITE_TEXT_RANGE, Paint)>`, `backgrounds: Vec<(u32, u32, Paint)>`, `bound: Cell<u64>` (the brush generation its drawing effects were set for; 0 = none).
- [ ] `pub fn restyle(&mut self, text: &impl StyledLines, lines: Range<usize>) -> Vec<usize>`: for each laid-out line in `lines`, compare `text.line_style(line)` with the stored `style`; drop the layout when they differ (its measured height stays in the `HeightIndex` until the line is laid out again) and return the dropped lines.
- [ ] Estimates: `estimate` multiplies by the heading scale when the line's style says it is a heading (one `line_style` call; spans are parsed lazily, so this is cheap only for lines near the screen: `reset` estimates without styles, and `ensure` measures).
- [ ] `position_at` / `hit_row`: a trailing hit maps through `to_byte_after`, a leading hit through `to_byte`, so hits never land in hidden markup (Decision 1).
- [ ] `impl LineLayout { pub fn bind(&self, brushes: &dyn Fn(Paint) -> ID2D1Brush, generation: u64) }` sets each colour run's drawing effect when `bound != generation`.
- [ ] Tests (DirectWrite, in-process as phase 1): `a_rendered_heading_is_taller_than_its_revealed_line`, `hidden_markers_leave_the_layout_text`, `hit_test_on_a_rendered_line_never_lands_in_a_marker` (every x across `a **bold** [link](u) end`, every hit outside the hidden bytes), `caret_rects_on_a_rendered_line_round_trip`, `restyle_keeps_lines_whose_style_did_not_change`, `restyle_drops_a_line_whose_reveal_changed`, `inline_code_uses_the_code_family`, `a_long_rendered_heading_wraps_at_its_own_size`. Existing phase 1 tests keep passing with `MemoryText`'s plain style.
- [ ] `cargo test --lib live::layout`. Commit `feat(live): styled line layouts`.

### Task 6: The view follows the document: edit log, flush, reveal pass

**Files:** `src/live/view.rs`, `src/window/live_host.rs`.

- [ ] `ViewState` gains `document: LiveDocument`, `edits: EditLog`, `revealed: Range<usize>`. `create` and `reload` parse the document (`LiveDocument::parse(&text.slice(0..len))`) and compute `revealed` from the selection.
- [ ] `LiveView::edited(&self, edit: Edit, first_line: usize)` (signature change; `live_host::edited` builds the `Edit` with `Edit::from_change`): records the edit, shifts the line cache and heights (`layout.edit`, as phase 1), shifts `revealed` with `reveal::shift`, and invalidates from the edited line's top to the bottom when lines were added or removed, else the edited line. It does not reparse.
- [ ] `fn flush(state: &mut ViewState)`: takes the `EditLog`; `Pending::Full` → `document.reparse`, `layout.restyle(all)`; `Pending::Edits` → `document.apply` → the changed bytes → their lines → `layout.restyle(lines)`; each dropped line on screen is invalidated (to the bottom when its height may change: always, unless the line is laid out again here with the same height). Idempotent when nothing is pending.
- [ ] `fn sync_reveal(state: &mut ViewState) -> bool` (Decision 2): new = `revealed_lines(selection)`; if equal, nothing. Else: `before = caret_rect(caret)` with the **old** `revealed`; set `revealed = new`; `layout.restyle` each range of `reveal::changed(old, new)`; `after = caret_rect(caret)`; `scroll_y += after.top - before.top`; clamp; invalidate the whole view (heights may change). Returns whether it changed anything.
- [ ] Order at every entry point that measures (`selection_changed`, keys, mouse, `scroll_to_caret`, `top_line`, `set_top_line`) and at the **start of `WM_PAINT`, before the update rectangle is read**: `flush`, then `sync_reveal`. `selection_changed` then runs phase 1's caret/scroll logic.
- [ ] `#[cfg(test)]` helpers: `debug_line(line) -> Option<(String /*layout text*/, bool /*revealed*/, f32 /*height*/)>`, `revealed() -> Range<usize>`, `caret_client_y() -> Option<i32>`.
- [ ] Tests (`markdown_live.rs`): `rendered_lines_hide_markup_and_the_caret_line_shows_source` (`# Title\n\n**bold** and [link](u)\n`, caret on line 2: line 0 text is `Title` and taller than a row, line 2 shows source; move the caret to line 0: it shows `# Title`, line 2 shows `bold and link`), `a_selection_reveals_every_line_it_touches`, `typing_bold_markers_restyles_only_the_caret_line_until_it_is_left`, `opening_a_fence_restyles_the_lines_below`, `undo_restores_the_rendering`, `a_replace_all_reparses_once` (Ctrl+H replace-all over 200 matches; the reparse counter is 1), `an_external_change_while_live_rerenders`, `front_matter_is_dimmed_and_not_a_heading`.
- [ ] `cargo test --test markdown_live -- --test-threads=1`. Commit `feat(live): follow the document's spans and reveal the caret line`.

### Task 7: Painting styled lines; caret anchoring checks

**Files:** `src/live/view.rs`, `src/live/view/paint.rs`, `src/window/live_host.rs`.

- [ ] `LiveColors` gains the role colours: `heading, emphasis, marker, link, code, code_background, dim`, filled in `live_host::appearance` from `languages::syntax_colors(theme)` (`heading`, `emphasis`, `operator`, `link`, `code`, `code_background`, `comment`) for the cached theme, so the high-contrast and custom themes follow the source view. `appearance` also passes `settings.font_face` for `LiveFonts`.
- [ ] `Brushes` holds one brush per `Paint` (`Text` is the existing text brush) and `ViewState` a `brush_generation: u64`, bumped whenever brushes are created (new target, `D2DERR_RECREATE_TARGET`, colour change).
- [ ] Paint, per visible line: `laid.bind(brush_of, generation)`; fill each `backgrounds` range's `HitTestTextRange` rectangles (code background) after the selection fill and before `DrawTextLayout`; then draw the text with the `Text` brush as the default (runs with a drawing effect use theirs).
- [ ] Caret anchoring is exercised through real input here (Task 6 built it).
- [ ] Tests: `clicking_below_a_heading_keeps_the_caret_under_the_pointer` (caret on an H1 line, click a line below it; the H1 shrinks above the click; `caret_client_y` equals the click's row top within 1 px), `moving_onto_a_heading_keeps_the_caret_row_in_place` (Down onto a rendered H1; the caret's client y is where the rendered H1's row was), `leaving_a_heading_upwards_keeps_the_caret_row_in_place`, `click_on_rendered_bold_places_the_caret_inside_it`, `up_down_cross_rendered_lines_by_their_drawn_rows`, `moving_the_caret_through_rendered_lines_never_dirties` (`can_undo` and the save point unchanged after arrows, clicks and Ctrl+End), `inline_code_paints_its_background` (`GetPixel` inside the code run's rectangle is the code background), `recreating_the_target_rebinds_colours` (a `#[cfg(test)] view.drop_target()`, repaint, link pixels are the link colour), `theme_change_recolours_rendered_lines`.
- [ ] `cargo test --test markdown_live -- --test-threads=1`. Commit `feat(live): paint styled lines`.

### Task 8: Links

**Files:** `src/live/view/mouse.rs`, `src/live/view/keys.rs`, `src/live/view.rs`, `src/window/markdown_host.rs`.

- [ ] Ctrl+click at a hit position: first `markdown_host::follow_link_at` (wikilinks, unchanged); if it declines, `document.link_at(position)`; a non-wiki link posts `WM_FASTPAD_PREVIEW_LINK` to the main window with `wparam` = the Live hwnd and `lparam` = `Box<String>` of the destination (the main window's handler already maps a group child to its group and frees the box when there is none). The click takes no caret move, as in phase 1.
- [ ] `WM_SETCURSOR`: in the text area, `IDC_HAND` when Ctrl is down and the point is over a link span (rendered or revealed), else the I-beam. `WM_KEYDOWN` / `WM_KEYUP` of `VK_CONTROL` re-evaluate the cursor at the current pointer position (`GetCursorPos` → `ScreenToClient` → `SetCursor`).
- [ ] Tests: `ctrl_click_on_a_rendered_link_opens_it` (a `#heading` link moves the caret to the heading, which needs no browser), `ctrl_click_on_a_rendered_wikilink_follows_it`, `plain_click_on_a_link_places_the_caret`, `the_cursor_is_a_hand_over_a_link_with_ctrl`.
- [ ] `cargo test --test markdown_live -- --test-threads=1`. Commit `feat(live): follow rendered links`.

### Task 9: Typing latency, README, full verification

**Files:** `tests/windows/markdown_live.rs`, `README.md`, the spec.

- [ ] Perf: the phase 1 test `typing_in_live_costs_the_same_as_typing_in_source` keeps asserting Live p95 ≤ 4 ms on the 1 MB sample and the 10k-line note, now with rendering. Add `typing_inside_a_long_paragraph_stays_in_budget` (caret inside a 40-line paragraph, so every keystroke reparses that block and restyles its lines) and `entering_live_on_a_1_mb_note` (time from toggle to first paint; assert ≤ 200 ms and print it). Record the numbers in the PR. If over budget, profile first (likely: restyling lines that did not change, reparsing more than one block, `spans_in` copying whole blocks, invalidating to the bottom when no height changed).
- [ ] README: the Live Markdown section says inline Markdown now renders and the caret's line shows source.
- [ ] Spec: "Phase 2 shipped" note under §12 with this plan's deviations 1–9.
- [ ] Full suite: `cargo clippy --all-targets -- -D warnings`, `cargo test`, `cargo test --test '*' -- --test-threads=1`, and the ignored perf tests for `markdown_live` and `markdown_preview`.
- [ ] Manual check in the real app (back up and restore `fastpad.ini`): a note with every inline element, H1–H6, setext headings, front matter, footnotes, math, raw HTML, CJK and emoji inside bold and links; move through it with arrows, PgUp/PgDn, clicks and drags; type markers and watch only the caret line change; light, dark and high contrast; 100% and 150% DPI; zoom; Split with Live left.
- [ ] Commit `docs(live): phase 2 README and spec note`, open the PR.

## Done when

- Every line except the caret's (and the selection's) renders headings, emphasis, strike-through, inline code, links, wikilinks and dimmed HTML, front matter, footnotes and math, and the caret's line shows its source in role colours.
- Moving the caret onto or off a heading keeps the caret where it was on screen.
- Ctrl+click follows rendered links and wikilinks.
- Revealing never edits or dirties the document; text typed in Live is byte-for-byte what the source view would produce.
- Typing stays within 4 ms p95 on the 1 MB and 10k-line notes; the full suite is green on CI.
