# Find highlights in the Markdown preview

Date: 2026-11-19
Builds on: `2026-09-16-markdown-preview-design.md`, `2026-10-22-preview-selection-design.md`

## Intent

With the find bar open on a Markdown tab in Preview or Split mode, the rendered preview shows
where the query matches, as the source editor does: every match shaded, the current one shaded
more strongly. Today find marks matches only in the Scintilla editor, which Preview mode hides,
and the preview never follows the editor in Preview mode, so F3 shows nothing.

## Goals

- Every match of the find bar's query, under its options (case, whole word, regex), is shaded in
  the rendered text of the active tab's preview, in Preview and Split modes.
- The current match (the one the editor's selection is) gets the stronger shade.
- F3 / Shift+F3, Enter in the find bar, and opening a Search result bring the current match into
  view in the preview when it is off screen.
- Highlights follow the find bar: typing a query, changing an option, editing the document and
  switching tabs update them; closing the bar or emptying the query removes them.

## Non-goals

- The bar's count stays the editor's count. The preview does not count.
- No replace from the preview, and no find started from the preview's own text.
- Matches inside collapsed `<details>`, image alt text, Mermaid/SVG diagrams and other drawn
  content are not shown, and a collapsed section is not opened to show one.
- Live mode is unchanged.

## Approaches

1. **Search the rendered text (chosen).** The preview runs the query on its own rendered run
   text (`selection::text_runs`) and shades what it finds. Small, and touches no parser code.
   Trade-off: a query that only matches markup (`**`, a link URL, an entity) is counted by the
   bar but shows nothing in the preview, and a match split across two runs (two table cells) is
   not found. When a block's hidden markup also matches (`[a](a) a`), the current-match shade in
   that block can land one match late.
2. **Track each rendered character's source position.** The model would record a source map
   for every `RichText`, and the editor's own match ranges would be drawn exactly. The count and
   the shading always agree, but it means a sizeable change to `preview/model.rs` (entities, HTML
   whitespace collapsing, inline HTML), and matches in hidden markup still have nowhere to show.
   Rejected as too much risk for the gain.

## Design

- **Matching (pure, `preview/find.rs`).** Given the blocks, a `Matcher` built from the query and
  options (`crate::search::Matcher`, the matcher Search and the regex count use), and the
  open/closed state of `<details>`, return the matches per block as `(run, utf16 start, utf16
  end)`, in document order. Matching runs per run, on its text; byte ranges are converted to
  UTF-16. A pattern that doesn't compile or matches empty text gives no matches. Bounded like the
  bar's count: at most `COUNT_CAP` matches, none for documents over `COUNT_MAX_BYTES`.
- **Current match.** The host passes the editor's current match as a source byte range. The
  preview finds the block whose `bytes` contain its start, and the match's ordinal `k` among the
  editor's matches in that block (from the ranges `find_matches` already returns). The current
  preview match is the `k`-th one in that block; if the block has fewer, the last one; if none,
  there is no current match. Unit-tested with markup before the match (`**a** a`, links, code).
- **State.** The preview view keeps the find query, options and current-match anchor, and
  recomputes its matches when they change and after each reparse (so a document edit updates
  them without the host). Clearing the query clears them.
- **Host.** `refresh_find_count` and `select_match` (`window/main_window/find.rs`) pass the
  query, options, all editor match ranges and the current one to the active group's preview,
  when its tab is a Markdown tab in Preview or Split mode. `clear_find_highlights` clears it too.
  Switching a tab's mode to Preview or Split with the bar open refreshes it.
- **Painting.** `draw_ops` gets a second per-run range layer for matches, drawn after the run's
  own backgrounds and before the selection fill, so the selection stays on top. Colour: the
  editor's find colour (`palette.accent`, as `group_layout.rs` gives the editor) through a new
  `ColorRole::FindMatch`, filled at the editor's opacities (60/255 for a match, 130/255 plus a
  1px outline for the current one). In high contrast, matches use `Highlight` with an outline so
  they don't depend on opacity.
- **Scrolling.** When the current match changes, the preview lays out its block if needed and,
  if the match's rectangle is outside the viewport, scrolls so it sits about a third from the
  top. It is a programmatic scroll (`set_scroll(.., user: false)`), which posts no
  `WM_FASTPAD_PREVIEW_SCROLLED`, so in Split it never moves the editor.

## Left to the builder

- How the match list is stored per block (flat sorted vec vs per block) and how `draw_ops`
  receives it, as long as painting a block doesn't scan the whole list.
- Test coverage: pure matching and current-match tests in `preview/find.rs`; one integration
  test in `window/main_window/tests/` that opens the bar on a Markdown tab in Preview mode and
  checks the preview's match count and current match via a test accessor.
