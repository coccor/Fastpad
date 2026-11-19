# Preview Find Highlights Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Shade the find bar's matches, and more strongly its current match, in the rendered Markdown preview (Preview and Split modes), and bring the current match into view.

**Architecture:** The preview searches its own rendered run text (`selection::text_runs`) with the find bar's `Matcher`. A pure module finds the hits and picks the current one from the editor's match ranges by block and ordinal. The view stores the query, recomputes hits after every document update, and paints them as a layer under the selection. The find code in `main_window/find.rs` pushes the query and the editor's ranges to the active group's preview.

**Tech Stack:** Rust, Win32 (windows-sys), Direct2D/DirectWrite (windows crate), `crate::search::Matcher`.

**Spec:** `docs/superpowers/specs/2026-11-19-preview-find-highlights-design.md`

## Global Constraints

- At most `COUNT_CAP` (1000) hits; no hits when the source is over `COUNT_MAX_BYTES` (32 MiB). Both from `window/find_bar/count.rs`.
- A pattern that doesn't compile or matches empty text gives no hits (use `find_bar::regex_matcher` for regex, `Matcher::new` otherwise; treat `Err` as no hits).
- Colours: `palette.accent` (high contrast: `palette.selection_background`). Match fill opacity 60/255, current fill 130/255 plus a 1 px outline at full opacity, matching `editor/scintilla/find_highlight_ops.rs`.
- The bar's count stays the editor's. Live mode is untouched.
- Commit messages: no `Co-Authored-By` trailer. Style: `feat(preview): ...`.
- Every new test opens with a `// Break caught: ...` comment, as the existing tests do.

## Review Focus

- **Markup before the match** (`**a** a`): the current preview hit must be the ordinal-matched one, not the first `a`. Pinned in Task 1. Known limit, accepted with approach 1: when the query also matches hidden markup in the same block (`[a](a) a`, the URL's `a`), the ordinal is off by the hidden matches and the current shade lands one match later. Do not test or "fix" this beyond the spec.
- **Edit while the bar is open:** after a reparse (`Update::Replaced`) hits must be recomputed against the new blocks, never index a block that no longer exists. Pinned in Task 3.
- **Collapsed `<details>`:** hits inside a closed section are not reported; opening it shows them. Pinned in Task 1 (closed) and by the view using `run_units`' open state.
- **Closing the bar / emptying the query / switching to Source:** highlights vanish and nothing panics with no preview. Pinned in Task 4.
- **Non-ASCII text** (`é`, emoji before a match): UTF-16 offsets, not bytes, reach DirectWrite. Pinned in Task 1.

---

### Task 1: Pure matching in `preview/find.rs`

**Files:**
- Create: `src/preview/find.rs` (with `#[cfg(test)] mod tests;` → `src/preview/find/tests.rs`)
- Modify: `src/preview/mod.rs` (add `pub mod find;`)

**Interfaces:**
- Produces:
  - `#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub struct FindHit { pub block: usize, pub run: u32, pub start: u32, pub end: u32 }` — UTF-16 offsets into the run's text.
  - `pub fn find_hits<'a>(blocks: impl Iterator<Item = (usize, Vec<RunText<'a>>)>, matcher: &Matcher, cap: usize) -> Vec<FindHit>` — document order, at most `cap`.
  - `pub fn current_hit(blocks: &[Block], hits: &[FindHit], editor_matches: &[Range<usize>], current: &Range<usize>) -> Option<usize>` — index into `hits`.

- [ ] **Step 1: Write failing tests** in `src/preview/find/tests.rs`, building blocks with `crate::preview::model::parse_document` and runs with `text_runs(&block.kind, &mut |_, open| open)`:
  - `hits_are_found_in_rendered_text_not_markup`: source `"**bold** text\n"`, query `"bold"` → one hit `{block 0, run 0, start 0, end 4}`; query `"**"` → no hits.
  - `offsets_are_utf16`: source `"é😀 x\n"`, query `"x"` → `start == 4, end == 5`.
  - `hits_stop_at_the_cap`: `"a a a\n"`, query `"a"`, cap 2 → 2 hits.
  - `a_closed_details_hides_its_hits`: `"<details><summary>s</summary>\n\nx\n</details>\n"`, open closure returning `false` → query `"x"` gives no hits; returning `true` → one.
  - `the_current_hit_is_the_same_ordinal_in_its_block`: source `"**a** a a\n\na\n"`; editor matches (every `a` in the source) `2..3, 6..7, 8..9, 11..12`; current `6..7` → index 1. Current `11..12` → index 3 (block 1's first).
  - `too_few_hits_fall_back_to_the_blocks_last`: same source, hits with block 0's third removed → current `8..9` gives index 1.
  - `no_hit_in_the_block_means_no_current`: current inside a block with zero hits → `None`.
- [ ] **Step 2: Run** `cargo test --lib preview::find` — expect compile failure (module missing).
- [ ] **Step 3: Implement.** `find_hits` runs `matcher.find_up_to(text, remaining)` per run (`RunText::Rich(r)` → `&r.text`, `RunText::Code(c)` → `c`) and converts byte offsets to UTF-16 with a running count over `char::len_utf16`. `current_hit`: the block is the one whose `bytes` contain `current.start`; `k` = number of `editor_matches` with `start` in that block's `bytes` and `start < current.start`; pick the `k`-th hit with that `block`, else the block's last, else `None`.
- [ ] **Step 4: Run** `cargo test --lib preview::find` — all pass.
- [ ] **Step 5: Commit** `feat(preview): find query hits in rendered text`.

### Task 2: Colour role and painting layer

**Files:**
- Modify: `src/preview/colors.rs` (`ColorRole::FindMatch`, add to `ALL` (now 16), `PreviewColors::find_match`, set in every const and in `preview_colors`)
- Modify: `src/preview/render.rs` (`SelectionPaint` gets `pub matches: &'a dyn Fn(u32) -> Vec<(u32, u32, bool)>`; `NONE` returns empty)
- Modify: any other `SelectionPaint { .. }` constructors (`grep -rn "SelectionPaint {" src`), e.g. Live's renderer, passing `&|_| Vec::new()`.

**Interfaces:**
- Produces: `ColorRole::FindMatch`; `SelectionPaint::matches(run) -> Vec<(start, end, is_current)>`.

- [ ] **Step 1: Failing test** in `colors.rs` tests: `find_matches_use_the_accent_or_highlight_in_high_contrast` — for `Theme::Light` palette, `preview_colors(..).find_match == palette.accent`; high contrast → `palette.selection_background`. (Follow the existing tests' way of building a palette.)
- [ ] **Step 2: Run** `cargo test --lib preview::colors` — fails to compile.
- [ ] **Step 3: Implement** the role and colour. In `draw_ops`' `DrawOp::Text` arm, before the selection fill: for each `(start, end, current)` from `selection.matches(run)`, `range_rects` → `FillRectangle` with the `FindMatch` brush at opacity `60.0/255.0` (`130.0/255.0` when current), reset to 1.0 after; for the current one also `DrawRectangle` 1 px at opacity 1.0. Same `SetOpacity` pattern as the diagram tint.
- [ ] **Step 4: Run** `cargo test --lib preview` and `cargo clippy --all-targets -- -D warnings` — pass.
- [ ] **Step 5: Commit** `feat(preview): a find-match colour and paint layer`.

### Task 3: View state, recompute and reveal

**Files:**
- Modify: `src/preview/view.rs` (state, public API, recompute in `accept_update`)
- Modify: `src/preview/view/painting.rs` (pass `matches` closure)
- Modify: `src/preview/view/selecting.rs` (make `run_units`' open-state logic reusable: a `pub(super) fn block_runs(state, block) -> Vec<RunText>`-shaped helper, or compute texts there)
- Test: `src/preview/view/tests.rs`

**Interfaces:**
- Consumes: Task 1's `find_hits`, `current_hit`, `FindHit`; Task 2's `SelectionPaint::matches`.
- Produces on `PreviewView`:
  - `pub struct PreviewFind { pub matcher: Matcher, pub editor_matches: Vec<Range<usize>>, pub current: Option<Range<usize>> }` (in `preview/find.rs`).
  - `pub fn set_find(&self, find: Option<PreviewFind>, reveal: bool)` — stores it, recomputes hits and current, invalidates; when `reveal` and there is a current hit, scrolls it into view.
  - `#[cfg(test)] pub fn find_hits(&self) -> (Vec<FindHit>, Option<usize>)`.

- [ ] **Step 1: Failing tests** in `view/tests.rs` (use the existing helpers there that create a view and load a document):
  - `set_find_shades_hits_and_clearing_removes_them`: document `"one two one\n"`, `set_find(Some(..query "one"..), false)` → 2 hits; `set_find(None, false)` → 0.
  - `an_edit_recomputes_the_hits`: after set_find for `"one"`, `apply_edits` replacing the text with `"one\n\none\n\none\n"` → 3 hits, all with `block < blocks.len()`.
  - `reveal_scrolls_the_current_hit_into_view`: 200 paragraphs `"para N"`, query `"para 150"`, current = editor range of that text → after `set_find(.., true)`, `top_line()` is within a few lines before 2×150 (the source line of block 150) and the hit's block is laid out.
- [ ] **Step 2: Run** `cargo test --lib preview::view` — fail.
- [ ] **Step 3: Implement.** `ViewState` gains `find: Option<PreviewFind>`, `hits: Vec<FindHit>`, `current_hit: Option<usize>`. A `fn recompute_find(state)` builds run texts for every block (reuse the open-state closure from `run_units`), skips when `document` source length > `COUNT_MAX_BYTES` (use the last block's `bytes.end`), calls `find_hits(.., COUNT_CAP)` and `current_hit`. Call it from `set_find` and at the end of `accept_update` when `update != Unchanged` and `find.is_some()`; clear `hits` in `release`. Painting: `matches = |run| hits for (index, run)`, found with `partition_point` on `(block, run)` since hits are sorted. Reveal: `ensure_layouts(&[block])`, get the run's rect via `range_rects` relative to the block top (`heights.top(block) + run.y + rect.top`), and if outside `[scroll_y, scroll_y + view_height - bar]`, `set_scroll(state, y - view_height / 3.0, false)`.
- [ ] **Step 4: Run** `cargo test --lib preview` — pass.
- [ ] **Step 5: Commit** `feat(preview): keep and paint the find bar's matches`.

### Task 4: Host wiring and integration test

**Files:**
- Modify: `src/window/main_window/find.rs` (`refresh_find_count`, `select_match`, `clear_find_highlights`)
- Modify: `src/window/preview_host.rs` (`pub(crate) fn set_find(hwnd, find: Option<PreviewFind>, reveal: bool)`: no-op unless `mode(hwnd)` is `Split` or `Preview` and `preview_shown(hwnd)`; and on switching a tab into Split/Preview, call `crate::window::main_window::find::refresh_find_count` so an open bar shows at once)
- Test: `src/window/main_window/tests/find_bar.rs`

**Interfaces:**
- Consumes: Task 3's `PreviewView::set_find`, `PreviewFind`, `find_hits`.

- [ ] **Step 1: Failing test** `the_preview_shades_find_matches_and_follows_f3` in `find_bar.rs`, using the Markdown setup pattern of `tests/live_focus.rs::fixture` but `ViewMode::Preview`: text `"**a** b a\n\na\n"`; open the bar, `set_find_query(.., "a")`, call `refresh_find_count` directly (skip the timer); pump until the preview has blocks. Assert `preview_host::view(hwnd).unwrap().find_hits().0.len() == 3`. `execute_command(FindNext)` twice → current hit index changes to 1 then 2. `close_find_bar` → 0 hits. Then switch to `ViewMode::Source` with the bar open and assert no panic and `refresh_find_count` still marks the editor.
- [ ] **Step 2: Run** `cargo test --lib find_bar` — fail.
- [ ] **Step 3: Implement.** In `refresh_find_count`, after marking the editor, build `PreviewFind` from the same query/options (`regex_matcher` when `options.regex`, else `Matcher::new`; on error pass `None`), `found.ranges` and `found.current()`, and call `preview_host::set_find(hwnd, .., false)`. In `select_match`, when the bar is visible, call it with `reveal: true` and the new current (reuse the last ranges by calling `refresh_find_count` logic, or pass `editor_matches` empty plus `current`, so `current_hit` uses ordinal 0 until the recount lands — prefer calling a shared helper that recounts once). In `clear_find_highlights`, `set_find(hwnd, None, false)`.
- [ ] **Step 4: Run** `cargo test --lib find_bar preview` then the full `cargo test` and `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` — all pass.
- [ ] **Step 5: Commit** `feat(preview): show find matches in Preview and Split`.

### Task 5: Manual check and clean-up

- [ ] Copy `native/out/x64` DLLs into `target/debug`, run the app, open a Markdown file in Preview, Ctrl+F a word that appears in bold and in a link: matches shaded, F3 moves and scrolls, Esc clears. Repeat in Split.
- [ ] `cargo clean` and remove `native/src`; keep `native/out` and `native/cache`.
