# Note Index, Front Matter and Tags (PR 1b) Implementation Plan

> **For agentic workers:** Steps use checkbox (`- [ ]`) syntax for tracking. Each part ends with its own check.

**Goal:** FastPad reads every Markdown note's front matter and tags into an index, keeps it current as notes change, shows the notebook by tag in a new Tags view, and renders front matter and tags in the preview.

**Spec:** `docs/superpowers/specs/2026-10-16-second-brain-phase-1-design.md` §6 and §8 (links in §7 are PR 1c).

**Architecture:**
- **Parsing** (pure, done first): `library::frontmatter` reads the YAML subset; `library::facts` reads a note's tags, aliases and properties with `pulldown-cmark`, using the preview's options plus `ENABLE_YAML_STYLE_METADATA_BLOCKS`.
- **Index** (`library::index`, pure plus file I/O): `NoteIndex` maps each Markdown note to its stamp and `NoteFacts`.
  - A worker builds a new index from the previous one (`Arc`, shared read-only): it reuses facts whose size and write time match, and reads only changed notes.
  - On the first build of a notebook the previous index comes from the cache file `libraries\<folder key>.notes.cache`.
  - The worker writes the cache when anything changed, then posts the new index boxed (`WM_FASTPAD_INDEX_READY`, generation-checked). The UI thread only swaps it in.
- **Triggers** (one path): any time the note list changes (a scan lands, a save, a watcher report), `request_index` starts a build from the current note list. At most one build runs at a time; a request during a build runs one more afterwards.
- **Tags view:** a fifth sidebar view, `SidebarView::Tags`, with an activity bar button, built like the Favorites view on `row_list`, as a tree of tags (split on `/`) whose expanded tags list their notes.
- **Preview:** front matter renders as a two-column table at the top. Inline tags are spans with a new `InlineStyle::Tag`, drawn as chips and clickable: a click shows the Tags view with that tag selected.

## Global constraints

- No new dependencies and no new `windows-sys` features.
- Nothing new before first paint: the index builds after the first scan lands.
- The index reflects saved text, not unsaved edits.
- Compile gate per part: `cargo clippy --all-targets --all-features -- -D warnings`; targeted tests per part; full suite at the end.

## Deliberate deviations from the spec

- **The Tags view has no filter box** (spec §8.1) in this PR. The tree, counts, nesting, opening notes and accessibility are in; the filter needs an edit control in the panel and comes later.
- **Front matter uses the preview's table block**, headed "Property" and "Value", rather than a new muted properties block. That keeps layout, selection and accessibility unchanged.

## Parts

### Part 1: Parsing (done)

- [x] `src/library/frontmatter.rs` with tests: start and end rules, BOM, CRLF, the 200-line limit, scalars, quotes, comments, flow and block lists, raw text for the rest, tags and aliases.
- [x] `src/library/facts.rs` with tests: the §6.3 accepted and rejected tag examples; nothing in code, links or HTML; line numbers; front matter tags, aliases and properties; parent tags.

### Part 2: The index (`src/library/index.rs`)

- `NoteIndex { notes: HashMap<String, IndexedNote> }`, keyed by lowercase relative path, with `IndexedNote { path, size, mtime, facts }`.
- `build(folder, listed: &[NoteEntry], previous: &NoteIndex) -> (NoteIndex, bool changed)`: Markdown notes (`.md`, `.markdown`) of 2 MB or less; facts reused when size and mtime match `previous`, else read with `file::loader::load`. An unreadable note is left out.
- `TagTree`: `NoteIndex::tags() -> Vec<TagNode>`. Each node has a name and full path, the spelling seen most often (ties by natural order), its notes (direct) and the count of notes with it or any tag under it, and its children, sorted naturally. Tag identity ignores case (Unicode lowercase).
- Cache: `encode` and `parse` of versioned lines, `version=1`, then per note `note=<size>|<mtime>|<path>` followed by `tag=<f|i>|<line>|<name>`, `alias=<text>` and `prop=<key>|<value>` lines. Values escape `\`, CR and LF. A wrong version or a malformed line discards the whole cache.
- **Tests:** reuse by stamp; a changed stamp rereads; non-Markdown and large notes skipped; the cache round trip, including escapes; a bad cache is discarded; the tag tree (nesting, counts, majority spelling, order).

### Part 3: Index wiring (`src/window/library_host/index.rs`)

- `LibraryHost.index: Arc<NoteIndex>`, `index_generation`, `index_running`, `index_again`.
- `request_index(hwnd)`: snapshot the Markdown entries of `state.notes`, then spawn the worker. The worker loads the cache when the index is empty, builds, writes the cache when it changed, and posts `Box<IndexReady { generation, folder, index }>`.
- `WM_FASTPAD_INDEX_READY = WM_APP + 21`, handled with the other boxed messages. It swaps the index in when the generation and folder match, refreshes the side panel if the Tags view is shown, and runs once more if a request arrived meanwhile.
- Triggers: `install` (after a scan), `document_saved` (a Markdown note), and the watcher's `mark_outside_changes`, whenever an entry's stamp changed.
- Reset (empty index, generation bumped) when the notebook closes, switches, or notes mode goes off, next to `stop_watcher`.
- **Window tests:** a note written outside FastPad appears in the index with its tags; a save updates its tags; the cache is written and a second window's first build reads it.

### Part 4: The Tags view

- `SidebarView::Tags`, token `tags`, with every match site from the map updated. The activity bar gets a fifth button (`GLYPH_TAG` `U+E8EC`, label "Tags") before Settings. The hard-coded 4s for buttons become `ActivityButton::ALL.len()`.
- `src/window/tags_view.rs`, modelled on `favorites_view.rs`:
  - Rows: tag rows (depth, chevron, `#name`, count) and note rows under an expanded tag. The expanded set lives in the view.
  - Click: a tag row toggles; a note row opens a preview tab (double click or Enter: a permanent tab). Left and Right collapse and expand.
  - States: "No notebook open.", "Indexing notes…", and "No tags yet. Add #tags to your notes, or tags: in their front matter."
  - Accessibility: outline items with expanded and collapsed state, depth as the value, and names like "#project, 12 notes".
- `CommandId::ShowTagsView = 269`, keymap id `view.showTags`, palette "View: Show tags", no default key.
- `tags_view::reveal(hwnd, tag)`: shows the view with the tag's ancestors expanded and the tag selected (used by the preview).
- **Tests:** a unit test of the row model (nesting, counts, expansion); window tests that the view lists tags from the index, a click opens a note, and `sidebar_view=tags` round-trips.

### Part 5: Front matter and tags in the preview

- `PARSE_OPTIONS` gains `ENABLE_YAML_STYLE_METADATA_BLOCKS`. The preview builder turns a metadata block into `BlockKind::Table` (head "Property", "Value"; one row per entry, `Value::display`), parsed with `frontmatter::parse` from the source.
- `InlineStyle::Tag(String)`: `TextBuilder::push` splits prose text with `facts::inline_tags`, outside code, links and HTML. Layout draws a `RoundedFill` in `CodeBackground` behind the tag and colors it as a link. In high contrast the chip draws a `RoundedStroke` instead, since `CodeBackground` equals the page there. Each tag is a link target with destination `fastpad-tag:<name>`.
- `links::classify_link` gains `LinkAction::Tag(String)` for `fastpad-tag:`. `preview_host::follow` calls `tags_view::reveal`.
- The incremental-parse property test gains a fixture that starts with front matter.
- **Tests:** model tests for a front matter table and tag spans; `classify_link` for tags; the incremental property test.

### Part 6: Finish

- [ ] README: a "Tags and front matter" subsection, and the Tags view in the sidebar description.
- [ ] Spec §17: notes for 1b.
- [ ] Full suite against the baseline failures; clippy; fmt.
- [ ] Commit.
