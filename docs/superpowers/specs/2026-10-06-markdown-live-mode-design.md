# FastPad Markdown Live Mode Design

> **Note:** Live mode was dropped on 2026-10-06 (Scintilla cannot keep the edited block rendered); only the writing helpers (§8) and scoped bindings (§9) shipped.

Status: Approved design (pending written-spec review)  
Date: 6 October 2026  
Builds on: `2026-09-16-markdown-preview-design.md`, `2026-09-17-preview-html-rendering-design.md`, `2026-09-28-syntax-highlighting-design.md`

## 1. Purpose

Writing Markdown today means typing in the editor and reading the result in the Split preview, so the eyes travel between panes on every sentence. Live mode shows the rendered result in the editor itself: markup is hidden or drawn as its rendered form on every line except the ones being edited, which show plain source.

Live mode is a third way of editing Markdown, alongside plain source (preview Off) and the Split/Full preview. It stays the Scintilla source editor: typing, undo, find, multi-cursor and the keymap behave exactly as in source mode, and the file on disk is only ever changed by the user's own edits and the explicit helpers in §8.

Success: a whole document can be written without opening Split.

## 2. Decisions

| Topic | Decision |
|---|---|
| Model | Live preview inside Scintilla (Obsidian-style), not rendered blocks (Typora-style) or WYSIWYG |
| Reveal | Every line holding a caret or touching a selection shows source; all other lines are rendered |
| Styling | Container lexer: FastPad styles Live buffers itself from `pulldown-cmark` source ranges; Lexilla is off for the buffer while Live is on |
| Hiding markup | Two style kinds: **hidden** (Scintilla invisible style, zero width) and **blanked** (foreground = background, keeps width so the painter can draw over it) |
| Decorations | One painter, run after Scintilla's `WM_PAINT` in the existing editor subclass, draws headings, bullets, checkboxes, quote bars, rules, table grid, code-fence bars and image thumbnails |
| Large headings | Drawn by the painter at full size into vertical space reserved with blank annotation lines below the heading line |
| Entering Live | Separate per-tab toggle (Ctrl+Alt+V), independent of the preview cycle; `markdown_live_default` setting opens Markdown tabs in Live |
| Size limit | Live unavailable above 1 MB |

Rejected alternatives:
- **Rendered editable blocks (Typora-style).** Reuses the preview renderer for tables and images, but needs a new subsystem for the caret moving between rendered and source blocks, selections that cross blocks, undo, find and scrolling.
- **True WYSIWYG.** Needs a rich-text editing engine plus a Markdown writer that saves text exactly as written. Far larger than this feature is worth.
- **Keeping Lexilla's markdown lexer.** It styles `**bold**` as one span with its markers included, so the markers cannot be hidden.
- **A transparent overlay window over Scintilla.** Fragile to keep in sync with scrolling, zoom and wrapping. The painter instead runs inside the editor's own paint and takes positions from Scintilla.
- **Real per-style font sizes for headings.** Scintilla gives every line the height of the largest font in any style, so a 24pt heading style would make every line 24pt tall.

## 3. Scope

### In scope

- The Live toggle, its setting, menu and palette entries (§4).
- Rendering every CommonMark/GFM element FastPad's preview already parses (§6).
- Clickable task checkboxes, Ctrl+Click on links, local image thumbnails, aligned tables.
- Writing helpers in every Markdown tab, Live or not: format toggles, list continuation, table auto-format (§8).
- Markdown-only key bindings so Ctrl+B is Bold in Markdown tabs and Toggle Sidebar elsewhere (§9).

### Out of scope

- Syntax highlighting inside code blocks.
- Rendering raw HTML (it stays dimmed source).
- Math and footnotes (dimmed source).
- Remote images (alt-text placeholder only, nothing is fetched) and animated GIFs.
- Table alignment for wide (CJK) characters: widths are counted in characters.
- Folding.

## 4. Turning Live on and off

- Live is **per tab**. It is not saved in the session; reopening a file follows `markdown_live_default`.
- **Ctrl+Alt+V** toggles it (command `markdown.toggleLive`). It also appears as View → "Live Markdown" (a checked item) and in the command palette.
- On a tab that is not Markdown, the command shows the same kind of notice the preview commands show.
- New setting `markdown_live_default` (bool, default `false`): Markdown tabs open in Live.
- Above **1 MB**, Live is unavailable. The toggle shows a notice; the default setting skips the file silently. The check is made when Live is turned on (or a tab opens); a buffer that grows past 1 MB while in Live turns Live off with the same notice.
- Live and the preview are independent: Live can sit on the left of Split. Full hides the editor, so Live has no visible effect there.

**On:** Lexilla is switched off for the buffer (container lexer), the prose font (`preview_font`) and the Live style table are applied, the block index is built (§7), and the visible range is styled.

**Off:** annotations are cleared, Lexilla and the editor font are restored, and the buffer is restyled.

## 5. Reveal

- The reveal set is every line that holds a caret or touches a selection, across all selections.
- Revealed lines get **source styles**: the full markup is visible, prose stays in the prose font, markup gets the existing Markdown role colours (headings use the Heading role, markers use Operator, and so on). Revealed headings are normal size.
- A selection covering the whole document (Ctrl+A) reveals everything. This is accepted: simple and predictable.
- Revealing changes only styling and annotations. It never enters the undo history and never marks the document modified.
- Hidden text is never on a revealed line, so Scintilla's rule that users cannot delete invisible text never applies.
- A rendered heading or image line that becomes revealed loses its reserved lines; the text below moves by that many lines. This jump is accepted (Obsidian has the same).

## 6. Rendering each element

Unless stated, "hidden" and "blanked" refer to the two style kinds in §2, and only apply to lines that are not revealed.

| Element | Rendered form |
|---|---|
| ATX/Setext heading | Line text blanked; markers hidden. The painter draws the heading text in the prose font, bold, at 1.8× (H1), 1.5× (H2), 1.25× (H3), across the heading line and its reserved lines. H1 and H2 get a thin rule underneath. H4–H6 are bold in the Heading colour at normal size and reserve nothing. Setext underlines (`===`, `---`) are hidden. |
| Strong / emphasis / strikethrough | Markers hidden; text bold / italic / struck through. |
| Inline code | Backticks hidden; text in the editor font on the code background. |
| Link `[text](url)`, reference link, autolink | `[`, `](url)` / `][ref]` hidden; text in the Link colour, underlined. Ctrl+Click opens it (§6.1). |
| Bullet list item | Marker blanked; the painter draws a bullet (• level 1, ◦ level 2, ▪ level 3+). |
| Ordered list item | Number kept visible, in the Operator colour. |
| Task item `- [ ]` / `- [x]` | Marker and brackets blanked; the painter draws a checkbox. Checked items are struck through and dimmed. Clicking the box toggles it (§6.1). |
| Block quote | `>` blanked; the painter draws a vertical bar per nesting level; text in the Comment colour. |
| Thematic break | Text blanked; the painter draws a horizontal rule across the text width. |
| Fenced code block | Fence lines blanked; the painter draws a thin bar with the language name at the right on the opening fence line. Code lines use the editor font and a code background filled to the full line width (`SCI_STYLESETEOLFILLED`). |
| Indented code block | Editor font and code background as above. |
| GFM table | Whole table in the editor font. `|` characters and the delimiter row blanked; the painter draws the grid (cell borders and header separator). The header row is bold. Columns line up only when the source is padded (§8.3). |
| Image `![alt](path)` | Syntax hidden except the alt text, dimmed. For a local PNG/JPEG/GIF/SVG the painter draws a thumbnail (at most 300px tall at 100% zoom, width scaled to fit the text area) in lines reserved below. A remote or missing image shows a small placeholder with the alt text and reserves one line. |
| Raw HTML, front matter, footnotes, math | Dimmed source. Nothing hidden. |

Word wrap follows the existing setting. Colours come from the existing theme roles in light and dark themes. Zoom scales every painted decoration with the text. Line-number margins stay blank beside reserved lines (Scintilla's own behaviour for annotation lines).

### 6.1 Mouse

- A plain click places the caret, which reveals the line. Clicking inside a rendered heading's reserved lines places the caret on the heading line.
- **Ctrl+Click** on a rendered link opens it exactly as preview links open (`preview/links.rs`): web and mail links in the browser, `#anchor` moves the caret to the matching heading, local files in a FastPad tab.
- Clicking a **checkbox** toggles `[ ]` ↔ `[x]` as one undoable edit and does not move the caret or selection.
- The cursor is a hand over checkboxes, and over links while Ctrl is held.

### 6.2 Find

Find highlights on a blanked, painter-drawn heading are covered by the painted heading. Find always moves the caret to the current match, which reveals that line, so the current match is always visible.

## 7. Architecture

New module `src/live/`, plus hooks in the window and keymap code. The text-processing units are pure functions with no window code, so they are unit tested directly.

| Unit | Job | Depends on |
|---|---|---|
| `live/spans.rs` | **Pure.** The Markdown text of one block → `(range, kind)` spans and per-line decorations (heading level, bullet/checkbox position, quote depth, rule, table cells, image reference, code fence and language). Uses `pulldown-cmark`'s offset iterator with the preview's parse options (`model.rs`). | `pulldown-cmark` |
| `live/blocks.rs` | **Pure.** The block-boundary index, updated after each edit. Finds the block an edit touched (blank-line separated, extended to cover open fences and lists) so only that block is reparsed. Reuses `preview/incremental.rs`'s boundary rules where they fit. | `spans` |
| `live/reveal.rs` | **Pure.** Selections → revealed line set; the difference between old and new sets gives the lines to restyle. | — |
| `live/styles.rs` | (span kind, revealed) → Scintilla style number; builds the style table (hidden, blanked, heading colours, mono, code background) from theme roles. | `languages` theme roles |
| `live/reserve.rs` | Number of annotation lines each rendered heading or image needs, from its DirectWrite layout height or scaled image height divided by the line height, minus one. Recomputed on resize, zoom, theme and font changes. | `preview/dwrite.rs`, `preview/images.rs` |
| `live/painter.rs` | Runs after `DefSubclassProc` handles `WM_PAINT` in the editor subclass. For visible lines, draws the decorations using a Direct2D DC render target, with positions from `SCI_POINTXFROMPOSITION` / `SCI_POINTYFROMPOSITION`. Images decoded and cached through the preview's WIC code. Direct2D, DirectWrite and WIC are loaded lazily on first Live use, as the preview loads them. | `preview/dwrite.rs`, `preview/images.rs`, `preview/svg.rs`, `preview/colors.rs` |
| `window/live_host.rs` | Per-tab Live state, the toggle command, the setting, the 1 MB check. Wires Scintilla notifications to the units above; handles checkbox clicks and Ctrl+Click. | all of the above, `preview/links.rs` |
| `editor/markdown_edit.rs` | **Pure** text functions for the helpers (§8) plus the command glue. | — |

Reserved lines are annotations: `SCI_ANNOTATIONSETTEXT` with N empty lines, shown with `SCI_ANNOTATIONSETVISIBLE(ANNOTATION_STANDARD)` and a style whose background matches the editor's. FastPad uses annotations nowhere else today, so Live owns them.

### 7.1 Data flow

- **Typing.** `SCN_MODIFIED` marks the touched block dirty (O(1) bookkeeping). On `SCN_STYLENEEDED` for the visible range, dirty blocks are reparsed, styles applied (revealed lines get source styles), and reservations updated for heading/image lines whose layout changed. Scintilla paints, then the painter draws.
- **Caret or selection change** (`SCN_UPDATEUI` with `SC_UPDATE_SELECTION`). The reveal set is recomputed; lines that entered or left it are restyled and their reservations dropped or restored. Nothing is reparsed.
- **Scroll, resize, zoom.** The painter redraws the visible lines; resize, zoom and font changes recompute reservations.

Every step on the typing path is bounded by the size of one block.

### 7.2 Split scroll sync

Preview scroll sync uses document lines, not display lines, so the reserved annotation lines do not shift the mapping. This is verified by an integration test (§10).

## 8. Writing helpers

The helpers work in every Markdown tab, whether or not Live is on.

### 8.1 Format toggles

| Command | Key (Markdown scope) | Marker |
|---|---|---|
| `markdown.bold` | Ctrl+B | `**` |
| `markdown.italic` | Ctrl+I | `*` |
| `markdown.code` | Ctrl+\` | `` ` `` |
| `markdown.link` | Ctrl+K | `[sel](|)` |

- With a selection: wrap it, or unwrap it if it is already wrapped by that marker (markers just inside or just outside the selection both count).
- With no selection: act on the word under the caret; when the caret is not in a word, insert the pair and put the caret between them.
- Every selection of a multi-cursor is handled; the whole command is one undo step.
- `markdown.link`: wraps the selection as `[sel]()` with the caret between the parentheses. With no selection, inserts `[]()` with the caret between the brackets.

### 8.2 List continuation

- Enter in a list item (caret at the end, or mid-line, which splits the item) starts the next item with the same indent and marker. Ordered items increment the number; task items continue as `- [ ] `.
- Enter on an empty item removes its marker and ends the list.
- Tab / Shift+Tab with the caret in a list item (no multi-line selection) nest / un-nest the item by the list's indent width.
- Outside lists, Enter and Tab behave as today. These keys are handled in the Markdown buffer's input filter (`editor/input_filter.rs`), not as key bindings.

### 8.3 Table auto-format

- When the caret leaves a table it was in and the table's text changed while the caret was in it, the table's source is padded so the pipes line up. This is one undo step.
- If the table is already aligned, nothing is written and the document stays unmodified.
- Widths count characters; wide characters are out of scope. Escaped pipes (`\|`) and pipes inside code spans are cell content, not separators.
- Tab / Shift+Tab inside a table move the caret to the next / previous cell, selecting the cell's content.

## 9. Markdown-only key bindings

- Key bindings gain a **scope**: global (all existing bindings) or Markdown.
- Shortcut dispatch checks Markdown-scoped bindings first, only when the active tab is Markdown; otherwise it falls through to the global binding. Ctrl+B stays Toggle Sidebar outside Markdown tabs.
- The shortcuts editor shows each binding's scope. Conflict checks compare bindings in the same scope, and a Markdown-scoped key against the global bindings for that key (reported as "overrides X in Markdown files", not as an error).
- In v1 only the four format commands are Markdown-scoped. User overrides in `fastpad.ini` keep their command's scope.
- `markdown.toggleLive` is global (Ctrl+Alt+V), so it can show its notice on non-Markdown tabs.

## 10. Testing

Follows the project rules: input tests go through the real path, and runs are targeted until the final review.

- **Unit tests:**
  - `spans`: each element, asserting the hidden and blanked ranges.
  - `blocks`: after scripted edit sequences, the incremental index and spans equal a full parse.
  - `reveal`: single cursor, multi-cursor and multi-line selections.
  - `markdown_edit`: every toggle case, list continuation and termination, nesting, table padding including escapes and code spans.
- **Integration tests** (`tests/windows`, `--test-threads=1`):
  - Toggle on and off; the default setting; the 1 MB refusal and the turn-off when growing past it.
  - Reveal on caret move and with multiple cursors; revealing does not mark the document modified.
  - Checkbox click edits and undoes.
  - Ctrl+Click on a link.
  - Ctrl+B in `.md` versus `.txt` through the real accelerator path.
  - Enter and Tab through real keystrokes.
  - Live with Split: scroll sync stays aligned past headings and images.
  - The harness's command, menu and palette lists updated for the new commands.
- **Perf:** a Live typing scenario in `src/perf`; typing latency must stay within the existing budget.
- **Live check:** the real app on a sample document with every element, in light and dark themes and at several zoom levels, with `fastpad.ini` backed up and restored around the run.
