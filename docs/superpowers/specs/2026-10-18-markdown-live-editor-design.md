# FastPad Live Markdown Editor Design

Status: Draft design, pending review  
Date: 18 October 2026  
Builds on: `2026-10-06-markdown-live-mode-design.md` ("the old Live spec", dropped), `2026-09-16-markdown-preview-design.md`, `2026-09-17-preview-html-rendering-design.md`  
Replaces: the rendering half of the old Live spec. Its writing helpers (§8) and Markdown-scoped keys (§9) already shipped and are reused unchanged.

## 1. Purpose

Writing Markdown means typing raw source and reading the result in the Split preview. Live mode shows the rendered result where the user types.

The first attempt drew Live mode inside Scintilla and was dropped: Scintilla gives every line the same height, cannot hide text the user might delete, and cannot style the block being edited. This design adds a **new editor view written in FastPad**: a DirectWrite text view built only for Markdown.

Nothing is replaced. Scintilla stays the editor for every file, and stays the source view for Markdown. Live is a new, optional way to write a Markdown tab, next to the source view and the preview.

Success: a whole note can be written in Live without opening the source view or Split, and typing in it feels as fast as typing in Scintilla.

## 2. Decisions

| Topic | Decision |
|---|---|
| What it is | A new view for Markdown tabs. Scintilla, Lexilla and the preview are unchanged. |
| Text model | The tab's **Scintilla document stays the only copy of the text**. Live reads it and edits it through the tab's own Scintilla, which stays alive but hidden while Live is shown. Undo, the save point, autosave, recovery, find/replace, wikilink indexing and preview sync keep working because every edit still arrives as `SCN_MODIFIED`. |
| Rendering | Our own: Direct2D + DirectWrite, one `IDWriteTextLayout` per source line, real per-line heights (large headings, image rows). Reuses `preview/dwrite.rs`, `render.rs`, `colors.rs`, `images.rs`, `svg.rs`. |
| Reveal unit | **The block holding the caret**, from the blank line before it to the blank line after it: a paragraph, a heading, a whole table, a whole code block with its fences, the front matter; inside a list or quote, the lines between the blank lines around the caret. That whole block shows its source; every other line is rendered. With a selection, every block it touches is revealed. (Phase 3 review; phases 1–3 first revealed only the caret's line.) |
| Revealed line look | The source with all its markup shown, in the Markdown marker colour, and the rest of its formatting kept: a heading keeps its size, bold stays bold, code keeps its font. The block is highlighted with the editor's current-line colour. Nothing is hidden, so the caret never lands inside invisible markup. |
| Parsing | `pulldown-cmark` with the preview's options, reparsed per block with `preview/incremental.rs`'s block rules, so typing costs one block. |
| Library shape | A new module `src/live/` with a pure, Win32-free core (model adapter trait, line spans, reveal, caret motion, layout cache keys) and a thin Win32 view on top. The core never names Scintilla. |
| Entering Live | Per-tab toggle **Ctrl+Alt+V** (`markdown.toggleLive`), View → Live Markdown, palette entry; setting `markdown_live_default` (default off). |

Rejected:
- **Live inside Scintilla.** Tried and dropped (see the old Live spec).
- **Live with its own text buffer and undo.** Doubles every feature that already works on the Scintilla document (save, dirty state, recovery, external-change reload, find, autosave), and toggling would need a full sync. Can be revisited if Live ever needs to stand alone.
- **Typora-style always-rendered editing.** Needs the caret to step over hidden markup and a command for every style change.
- **Obsidian's per-span reveal.** Markup appears and disappears as the caret moves within a line; the user asked for the whole line.

## 3. Scope

### In scope (v1)

- Rendering everything the old Live spec §6 listed: headings at real size, bold/italic/strike, inline code, links, bullets, ordered lists, task checkboxes, quotes, rules, fenced and indented code, GFM tables, local images, dimmed raw HTML / front matter / footnotes / math.
- Wikilinks `[[Note]]` rendered as links (Ctrl+Click follows them as in source view).
- Caret, single selection (keyboard, mouse, double-click word, triple-click line), drag-select with autoscroll.
- Typing, Backspace/Delete, Enter/Tab through the existing Markdown writing helpers, clipboard, undo/redo, all through Scintilla.
- IME composition (Japanese, Chinese, Korean input) with the candidate window at the caret.
- Word wrap always on (Live is prose); zoom with Ctrl+wheel and the existing zoom commands.
- Find bar: matches highlighted, the current match revealed.
- Clickable checkboxes, Ctrl+Click on links, hand cursor.
- Live on the left of Split; scroll sync by source line.
- Light and dark themes, DPI changes.

### Out of scope (v1)

- Multiple cursors and rectangular selection. Using them (Ctrl+Alt+click, Alt+drag, Ctrl+D) switches the tab to the source view with a notice.
- Syntax highlighting inside code blocks.
- Drag-and-drop of text inside Live (file drop for attachments stays, through the existing handler).
- Folding. (Line numbers show in a gutter, following the editor's `line_numbers` setting: added after phase 1 review.)
- A full UI Automation text provider. v1 exposes the document as an MSAA value and the caret through the system caret (§9); a UIA `TextPattern` is follow-up work.
- Files above 4 MB: Live refuses with a notice.

## 4. Turning Live on and off

- Live is per tab and not saved in the session. Reopening a Markdown file follows `markdown_live_default`.
- **On:** the tab's Scintilla is hidden (it keeps its document, selection and undo), the Live view is shown in its place and takes focus, and the caret and first visible line carry over.
- **Off:** the reverse. Live keeps nothing that Scintilla does not also have, so turning it off loses nothing.
- In a split, each editor group's Live state is its own, the same as the preview mode.
- A non-Markdown tab, or a language change away from Markdown, turns Live off or shows the preview's "only for Markdown" notice.
- Full preview hides Live as it hides the source view.

## 5. Reveal

- Revealed set = every block the selection touches (a caret is an empty selection). A block runs from the blank line before it to the blank line after it; a top-level block the parser sees as one (a fenced code block holding blank lines, the front matter) is revealed whole, and inside a list or quote only the lines between the blank lines around the caret are. A blank line is a block of its own.
- Multi-line constructs reveal as a whole: clicking into a table shows every row's pipes, clicking into a code block shows both fences.
- Revealed lines keep their formatting (heading size, bold, italic, code font) and show their markup in the marker colour; the revealed block is highlighted with the editor's current-line colour.
- Revealing only changes how lines are drawn. It never edits the text, never enters undo and never marks the document modified.
- A line whose height changes when revealed (a heading, an image line) moves the lines below it. To keep the caret steady, the view is scrolled so the caret's line keeps its screen position when the reveal set changes.

## 6. Rendering

Each source line becomes one `IDWriteTextLayout`, wrapped to the text width.

- **Rendered line:** the line's spans (§7.2) decide each range: hidden ranges are removed from the layout string, styled ranges get font weight, style, size, strike-through, underline and colour through DirectWrite ranges and drawing effects. Bullets, checkboxes and images are inline objects (the preview's `inline_object.rs` pattern). Quote bars, rules, code-block and table backgrounds, and table grids are drawn by the line painter from per-line decorations.
- **Revealed line:** the full source text, prose font, Markdown role colours, no hidden ranges.
- **Offset maps:** each laid-out line keeps a map between its UTF-8 source offsets and its UTF-16 layout positions, skipping hidden ranges. Caret placement, hit testing, selection drawing and find highlights all go through it.
- **Heights:** a `HeightIndex` like the preview's holds every line's height, estimated until the line is laid out. Only visible lines (plus a margin) are laid out; layouts are cached by (line text hash, revealed, width, zoom, theme).
- **Fonts:** prose in `preview_font` at `preview_font_size`, code in the editor font. Headings 1.8× / 1.5× / 1.25× / bold for H4–H6, as the preview.
- **Tables:** a rendered table is laid out as a grid (columns sized to their widest cell, as the preview does), not as padded text. A revealed row shows its source row. Table auto-format from the writing helpers still runs when the caret leaves.
- **Images:** local PNG/JPEG/GIF/SVG on their own line render as a scaled thumbnail (at most 300 px tall at 100% zoom); remote or missing images show the alt text placeholder. Nothing is fetched.

## 7. Architecture

### 7.1 The library (`src/live/`)

| Unit | Job | Win32? |
|---|---|---|
| `model.rs` | `TextSource` trait: length, line count, line ↔ offset, read a line's bytes, selection get/set, and edit ops (`replace(range, text)`, undo group begin/end). One implementation over `Editor` (Scintilla), one in-memory implementation for tests. | No |
| `blocks.rs` | Block index over the document, updated from `SCN_MODIFIED` edits; reparses only touched blocks (reuses `preview/incremental.rs` rules). | No |
| `spans.rs` | One block's text → per-line spans (`Hidden`, `Bold`, `Italic`, `Strike`, `Code`, `Link`, `Heading(n)`, `Dim`, …) and decorations (bullet, checkbox, quote depth, rule, fence, table cells, image). Revived and adapted from the dropped `src/live/spans.rs` (in git at `b99b099^`). | No |
| `reveal.rs` | Selection → revealed source lines; old vs new set → lines to re-lay. | No |
| `motion.rs` | Caret movement over laid-out lines: left/right by grapheme (skipping hidden ranges on rendered lines), word, line start/end, up/down by visual row with a remembered x, page up/down. Takes a `LineGeometry` trait so it is tested without DirectWrite. | No |
| `layout.rs` | Builds and caches per-line `IDWriteTextLayout`s and offset maps; hit testing (`HitTestPoint` / `HitTestTextPosition`). | DirectWrite |
| `heights.rs` | Line height index and scroll anchoring (adapted from `preview/heights.rs`). | No |
| `paint.rs` | Draws visible lines, decorations, selection, find highlights and the caret with an `ID2D1HwndRenderTarget`. | Direct2D |
| `view.rs` | The `FastPadLive` window: paint, scroll bars, mouse, keyboard, `WM_CHAR`, IME, focus, caret blink, DPI, cursor. | Win32 |

`window/markdown_host.rs` gains the toggle, the per-tab state and the wiring between notifications and the view. Direct2D/DirectWrite/WIC stay lazily loaded on first use, as the preview does.

### 7.2 Data flow

- **Typing:** a key or char in the Live view becomes an edit on the tab's Scintilla (`replace` on the `TextSource`). Scintilla sends `SCN_MODIFIED` as for any edit. The host forwards it to the preview, the library and the Live view; Live marks the touched block dirty, reparses it, re-lays the lines whose spans changed and repaints only those rows.
- **Writing helpers:** Enter, Tab and Ctrl+B/I/`/K call the existing `markdown_edit` functions on the same Scintilla, so behaviour is identical in both views.
- **Caret/selection:** Live keeps Scintilla's selection current (`SETSEL`) so find, status bar, commands and switching views see the same caret. Changing the selection recomputes the reveal set.
- **Undo/redo:** sent to Scintilla. Live re-lays from the resulting `SCN_MODIFIED`s and takes the caret from Scintilla's selection afterwards.
- **External edits** (reload, background replace, agents writing the file): arrive as Scintilla edits like everything else.

### 7.3 Commands in Live

Commands that only read or set the selection, or edit text through `Editor`, work unchanged because Scintilla is still there: save, find/replace, go to line, select all, cut/copy/paste, line moves, comment toggles, case changes, wikilink completion's insertion. A command that needs Scintilla's own UI (autocomplete popup, multi-cursor commands, rectangular selection, fold commands) switches to the source view first with a notice. The command list is checked one by one in the plan.

## 8. Input

- **Keys:** arrows, Home/End, Ctrl+arrows, PgUp/PgDn, Ctrl+Home/End, with Shift to extend; Backspace, Delete, Ctrl+Backspace/Delete; Enter and Tab through the writing helpers. Everything else goes through FastPad's keymap and command dispatch as in the source view.
- **Characters:** `WM_CHAR` (with the existing C0 filter and surrogate pairing) inserts at the selection, as one undo step per typed run, matching Scintilla's grouping.
- **IME:** `WM_IME_STARTCOMPOSITION` places the composition and candidate windows at the caret (`ImmSetCompositionWindow` / `ImmSetCandidateWindow`); the result string arrives as `WM_CHAR`s / `WM_IME_COMPOSITION` and is inserted as one edit. Inline composition drawing is out of scope for v1 (the IME draws its own window).
- **Mouse:** click places the caret; Shift+click extends; double-click selects a word; triple-click a source line; drag selects with autoscroll; Ctrl+click follows links; click on a checkbox toggles it as one undoable edit; wheel and Ctrl+wheel scroll and zoom.
- **Caret:** the system caret (`CreateCaret`/`SetCaretPos`) is kept at the drawn caret so screen readers, magnifiers and IMEs can find it; the drawn caret is painted by Live.

## 9. Accessibility

v1: the Live window answers `WM_GETOBJECT` with an MSAA object whose role is editable text, whose value is the document text and whose focus follows the caret, and keeps the system caret in place. That is roughly what Scintilla offers today. A UIA text provider (`ITextProvider` / `TextPattern`) is the follow-up that makes Live better than the source view for screen readers.

## 10. Performance

- Nothing in Live loads or runs until the first Live tab is opened. Startup and first paint are unchanged.
- Keystroke to paint must stay within the existing typing-latency budget (`src/perf`), measured on a 10k-line note and the benchmark fixture `benchmarks/fixtures/sample.md`.
- Per keystroke: one block reparse, re-layout of the changed lines only, a partial repaint of the rows that changed.
- Opening a 4 MB note in Live lays out only the visible screen; heights below are estimates until scrolled to.

## 11. Testing

- **Unit (pure core, in-memory `TextSource`):**
  - `spans`: every element; hidden and styled ranges; offsets match the source.
  - `blocks`: scripted edit sequences give the same spans as a full parse.
  - `reveal`: caret, selections across lines, wrapped lines.
  - `motion`: grapheme, word, line and visual-row motion, skipping hidden ranges, with a fake `LineGeometry`.
  - offset maps: UTF-8 ↔ UTF-16 with hidden ranges, surrogate pairs and combining marks.
- **Integration (`tests/windows`, `--test-threads=1`)**, driving the real app with `SendInput`:
  - Toggle on/off keeps text, caret and undo; undo across a toggle.
  - Typing, Backspace, Enter list continuation and Ctrl+B in Live produce the same text as in the source view.
  - Reveal on caret move does not mark the document modified.
  - Checkbox click toggles and undoes; Ctrl+click on a link and a wikilink.
  - Find highlights and reveals the match.
  - Live + Split scroll sync past headings and images.
  - An external change to the file while in Live.
  - IME: a composition through the IMM test path inserts one edit.
- **Perf:** a Live typing scenario added to `src/perf`, gated in CI like the existing ones.
- **Manual check:** the real app on a sample note with every element, light and dark, several zoom levels and DPIs, with `fastpad.ini` backed up and restored.

## 12. Delivery

Large, so it ships in phases, each behind the toggle and each mergeable on its own:

1. **Core and plain view:** `TextSource`, the Live window showing the document as plain wrapped text with caret, selection, keyboard, mouse, clipboard, undo, IME and the toggle. No Markdown rendering yet. Proves the text engine and typing latency.
2. **Inline rendering:** spans, blocks, reveal; headings, emphasis, code, links, wikilinks, dimmed blocks.
3. **Block rendering:** lists, checkboxes, quotes, rules, code blocks, tables, images.
4. **Integration:** find, Split sync, command audit, MSAA, perf scenario, README.

> **Phase 1 shipped** (plan `docs/superpowers/plans/2026-10-18-markdown-live-editor-phase-1.md`), with these differences from this spec: Live state is per (group, document); typing and horizontal keys are forwarded to the hidden Scintilla rather than edited through a `TextSource` API; wikilink completion is off in Live; more than one selection switches back to the source view; a hidden Scintilla reports no selection changes, so Live polls them after its input and after every command; `imm32` is loaded lazily; there is no context menu and no text drag-and-drop yet. The MSAA object of §9 is not in phase 1: the Live window answers `WM_GETTEXT` and keeps the system caret in place, and the MSAA object moves to phase 4. Typing latency is held to 4 ms p95 (the source view runs 1.0–1.4 ms); matching the source view needs a present limited to the changed rectangle, also phase 4.

> **Phase 2 shipped** (plan `docs/superpowers/plans/2026-10-19-markdown-live-editor-phase-2.md`), with that plan's deviations: heading sizes are the preview's (2.0 / 1.5 / 1.25 / 1.0 / 0.875 / 0.875, bold); block constructs (lists, quotes, code blocks, tables, rules, images) stay source until phase 3; revealed lines change colour only, so typing markers never re-wraps them; footnotes and math are dimmed by a small scanner, since the preview's options parse neither; a setext underline hides to an empty row; edits are logged and applied in one flush before the next measurement or paint; block spans are parsed lazily; a selection ending at a line start does not reveal that line; Ctrl+click on Markdown links goes through the preview's link path, and a `#heading` link moves the editor's caret to the heading. Code backgrounds are drawn before the selection so a selection over inline code stays visible.

> **Phase 3 shipped** (plan `docs/superpowers/plans/2026-10-19-markdown-live-editor-phase-3.md`), with that plan's deviations and these found while building it:
> - **Revealed lines (after the phase 3 review):** the caret reveals its whole block, between blank lines (a whole table, a whole code block with its fences), not just its line. Revealed lines keep their formatting (a heading keeps its size, bold stays bold) with the markup shown in the marker colour, and the block is highlighted with the editor's current-line colour. They drop their indent and margin decorations and show their prefix as source.
> - **Containers:** a container's indent is fixed (quote 1 em, bullet item 1.5 em, ordered item 2 em), since an item's later lines do not know its marker's width.
> - **Code blocks:** they use the code font on the prose rows, so line numbers stay in step.
> - **Tables:**
>   - Columns are sized as the preview sizes them, and a table too wide for the view shows as source.
>   - Rows are not striped.
>   - As in GFM, a pipe inside a code span splits cells unless it is escaped.
>   - A row whose columns move is re-spaced in place rather than laid out again.
> - **Images:** a line holding only an image shows its thumbnail (at most 300 DIPs times the zoom); remote, missing or not-yet-decoded images, and images inside text, show their alt text, dimmed and italic. Decoding runs on the view's own worker.
> - **Checkboxes and tasks:** a checkbox click is the one edit Live makes on its own: one undo step that keeps the selection. Checked task text is struck through and dimmed.
> - **Block index:** Live keeps its own block index (top-level block ranges and reference definitions, updated with the preview's sentinel rule) instead of the preview's document. That rebuilt the preview's rendering model of a block on every keystroke: 2.3 ms per keystroke for the 10k-line note's single 500 KB paragraph.
>
> Release keystroke p95, Live (source view): 10k-line note 2.3–2.6 ms (0.7); 60-row table 2.8 ms (0.7); 300-item list 2.0 ms (0.8); 1 MB note 1.9–2.0 ms (0.8–0.9); 500-line code block 1.6 ms (0.7). Entering Live on the 1 MB note takes 53–57 ms to the first frame.

## 13. Open questions

- Should a rendered heading show its `#` markers dimmed in the gutter (as some editors do) so the level is visible without revealing? Default: no.
- Should `markdown_live_default` also apply to new untitled notes and daily notes? Default: yes.
