# FastPad Preview Text Selection Design

Status: Draft design  
Date: 22 October 2026  
Builds on: `2026-09-16-markdown-preview-design.md` (which listed "text selection and copy inside the preview" as out of scope), `2026-10-22-markdown-view-modes-design.md`

## 1. Purpose

Source and Live are text editors: their text can be selected and copied. The Markdown preview
(Split's right half, and Preview) is drawn with Direct2D and cannot. Ctrl+C there copies from
the hidden editor's selection instead.

This design lets the user select the preview's text with the mouse and copy it, as in a
browser. Success: any text in the preview can be selected and copied, and pasting it into
Word or Outlook keeps its formatting, while a plain-text editor gets clean text without
Markdown markers.

The SVG preview is a picture (resvg draws any text into pixels) and is out of scope.

## 2. Decisions

| Topic | Decision |
|---|---|
| What is selectable | Every text run the preview draws from the document: paragraphs, headings, list item text, quote and alert text, code blocks, table cells (front matter included), `<details>` summaries, and image alt text (as the image's text). List markers, the alert title, the disclosure triangle and checkboxes are decoration: not selectable. |
| Selection model | A range between two **text points** `(block, run, offset)`: the block's index in the document, the run's index among the block's selectable runs in reading order, and a UTF-16 offset in the run. Points compare in that order, which is document order. |
| Runs | One shared walk, `model::text_runs(&BlockKind)`, lists a block's selectable runs in reading order: the source of truth for both layout (which records each run's `IDWriteTextLayout` and origin) and copy (which reads the model). Collapsed `<details>` contribute only their summary, as they draw. A test checks the two agree on every block kind. |
| Copy without layout | The preview lays out only blocks near the viewport. Copy reads selected text from the **model**, so Select All + Copy on a 1 MB note lays out nothing. Only the two end points need layouts, and the user put them on screen. |
| Mouse | Press and drag selects, with `SetCapture` and autoscroll at the top and bottom edges. Double-click selects a word, triple-click a run (a paragraph, a cell, a code block). Shift+click extends. A click on empty space clears. |
| Links | A press on a link and a release without moving past the system drag threshold still follows it. Dragging from a link selects, as in a browser. |
| Keyboard | Ctrl+A selects all, Ctrl+C and Ctrl+Insert copy, the Menu key and Shift+F10 open the context menu, and Escape clears a selection before it does anything else. Without a caret, Shift+arrows do nothing. |
| Commands | Copy and Select all, run while the preview has focus, act on the preview, not on the hidden editor. |
| Context menu | Right-click: Copy (grayed without a selection) and Select all; over a link also Open link and Copy link address. |
| Clipboard | `CF_UNICODETEXT` and `HTML Format` in one clipboard transaction. |
| Plain text | Runs joined in reading order. Blocks are separated by a blank line, list items by one newline with `- `, `N. ` or `[x] ` / `[ ] ` before them, nesting indented two spaces per level, quote lines prefixed `> `, table cells separated by a tab and rows by a newline. A code block's text is kept exactly. An image gives its alt text. |
| HTML | Semantic elements from the model: `h1`–`h6`, `p`, `ul`/`ol`/`li` (with a checkbox for tasks), `blockquote`, `pre><code`, `table`/`thead`/`tbody`/`tr`/`th`/`td`, `strong`, `em`, `del`, `code`, `mark`, `kbd`, `a href`. Images give their alt text; no file paths leave the machine. A partially selected run contributes its selected part with the styles that overlap it. |
| Painting | Each run's selected range is filled behind its text, after the run's own backgrounds (code blocks, cells, inline code), in the editor's selection colour, or its inactive selection colour while the preview is not focused. In high contrast the fill is `Highlight` and the selected text is drawn in `HighlightText`. |
| Lifetime | The selection survives scrolling, resizing, zoom, theme and font changes (the runs are the same; only their layouts change). An edit to the document, a reload, switching tabs or opening and closing a `<details>` clears it. |
| Shared geometry | The preview and Live each have a private copy of `range_rects` (DirectWrite hit-testing of a range). Both use one shared copy, in `preview/dwrite.rs`, with a point hit-test beside it. |

### Rejected

- **Copying Markdown source.** Asked and declined: the preview is for reading, and its copy
  should paste like a web page's.
- **Selection in the editor's terms (source bytes).** Rendered text has no one-to-one mapping to
  source bytes (entities, escapes, HTML, hidden markers), so endpoints could not be placed back
  on screen reliably.
- **Laying out every selected block to copy it.** Select All on a 1 MB note would lay out
  thousands of blocks for one copy.
- **A caret and keyboard selection.** Needed for a caret-browsing mode, not for copying.
  Follow-up work, with a UI Automation text pattern.

## 3. Model: runs

```rust
pub struct TextRunRef<'a> {
    pub text: RunText<'a>,      // Rich(&RichText) or Code(&str)
    pub context: RunContext,     // what the run is, for plain text and HTML
}
pub enum RunContext {
    Paragraph, Heading(u8), Code { language }, Cell { table, row, col, header },
    ListItem { depth, ordered: Option<u64>, task: Option<bool>, first },
    Summary, ...  // plus the quote depth around any of them
}
pub fn text_runs(kind: &BlockKind) -> Vec<TextRunRef<'_>>;
```

Offsets are UTF-16, as the layouts' are. `RichText` text maps to UTF-16 with its existing
`utf16_len`; a helper slices a `RichText` by a UTF-16 range, keeping the spans that overlap it.

## 4. Layout

`LaidBlock` gains `runs: Vec<LaidRun>`: for each `text_runs` entry, the layout, its origin in
block coordinates, and whether it sits inside a horizontally scrolling part (a code block or
table) with that part's clip. `layout_kind` pushes a run where it pushes the run's
`DrawOp::Text`, and tags the op with the run index so painting can fill the selection behind
exactly that op.

## 5. View

- State: `selection: Option<(TextPoint, TextPoint)>` (anchor, focus), plus the drag state (unit,
  origin, click count and time, autoscroll timer).
- `point_at(x, y)`: the block under `y` (clamped to the first and last block), then the run
  nearest the point within it (the run containing it, else the nearest by vertical then
  horizontal distance), then `HitTestPoint` in that run, clamped to it.
- Word boundaries use the same rule as Live (`live::text::word_at`'s character classes).
- Painting asks, per text op, for the run's selected UTF-16 range and fills its rectangles.
- Copy builds plain text and HTML from `text_runs` over the selected blocks, slicing the first
  and last run.

## 6. Accessibility

The preview stays an MSAA document with its links as children. The selection is not exposed;
a UI Automation text pattern is follow-up work (listed in the preview spec's out of scope).

## 7. Testing

- Unit: `text_runs` on every block kind, including collapsed details; layout's runs agree with
  `text_runs`; text point ordering; RichText slicing at span edges and surrogate pairs; plain
  text and HTML for each block kind, partial runs, nested lists and quotes; the CF_HTML header
  offsets.
- Window tests (`markdown_preview.rs`): drag selects across blocks and the copied text matches;
  double and triple click; Shift+click; Ctrl+A then Ctrl+C on a 1 MB note lays out no new
  blocks; a click on a link still follows it, a drag from it selects; Escape clears first;
  Copy from the menu while the preview has focus does not copy the editor's selection; the
  selection survives a resize and clears on an edit.
