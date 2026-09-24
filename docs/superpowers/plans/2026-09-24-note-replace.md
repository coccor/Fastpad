# Replace Across Notes (3b) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add VS Code-style replace across the open notebook's notes to the Search view (Ctrl+Shift+H, a replace field, Replace all, and per-row replace), plus `$1` expansion in both the Search replace and the editor find bar's regex Replace.

**Architecture:**
- **Pure code:**
  - `crate::search::Matcher` gains replacement expansion.
  - `src/library/text_replace.rs` (new, no Win32) counts and writes replacements into closed notes. Each write checks the note's stamp first, keeps the file's encoding, BOM and line endings, and uses `save_atomic`.
- **Window code:**
  - `text_search_host.rs` runs the replace flow: count on a worker, confirm, apply to open tabs in the editor (one undo action each, not saved), write closed notes on a worker, report in a notification, and re-run the query.
  - `search_view.rs` gains the chevron, the replace field, the Replace all button and the per-row replace button.
  - The find bar's regex Replace and Replace all expand `$1` through the same `Matcher`.

**Tech Stack:** Rust 2024, `windows-sys` 0.61, Scintilla, GDI, and MSAA. No new crates: `regex` 1.13.1 is already a dependency.

**Spec:** `docs/superpowers/specs/2026-09-24-note-search-design.md`, sections 11, 12, 12a and 13 for 3b. Sections 4–10 and 17 describe what 3a built, which this plan extends. §17 is where deviations are recorded.

**Branch:** `feat/note-replace`, stacked on `feat/note-search` (PR #11). The PR targets `feat/note-search`.

## Global Constraints

- **Scope:** 3b only. No Ctrl+P and no links.
- **Dependencies:** no new crates.
- **Latency rules:**
  - Nothing new runs before first paint or first input.
  - The UI thread never reads or writes a closed note's file. Counting and writing run on worker threads.
  - The UI thread never reads a written note's metadata either. The write worker takes each saved file's `Stamp` right after `save_atomic`, and the UI thread records it with `LibraryState::record_written`, which touches no disk. `add_note`, which calls `store::stamp`, is not used for replace writes.
  - Applying a replacement to open tabs happens on the UI thread, through Scintilla. It waits while `modal::modal_active(hwnd)` or `file_population_active(hwnd)` holds, the same guard `text_search_host::timer` uses.
- **App-borrow rule (`app_ptr` contract):**
  - While holding a `&mut` borrowed from App-derived state (`with_view`, `library_host::with_state`, `with_host`, `app_ptr(...).as_mut()`), never call:
    - `SetFocus`, `SetCapture` or `UpdateWindow`;
    - `CreateWindowExW`;
    - `SetWindowTextW` on a child, or `GetWindowTextW` (`WM_GETTEXT`);
    - `SendMessageW` to another window;
    - `modal::*`, including `modal::confirm`;
    - a document swap.
  - Take the values you need, end the borrow, then make the call.
- **Worker threads** touch an `HWND` only through `PostMessageW` to the main window. A payload is `Box::into_raw`. The worker frees the box itself if the post fails, and a stale generation is dropped and freed on the UI side.
- **Values (spec, verbatim):**

  | Name | Value |
  |---|---|
  | Replace shortcut | Ctrl+Shift+H (`ReplaceInNotes`). Ctrl+H stays the find bar's Replace. |
  | Replace all | A button at the right of the replace field, and Ctrl+Alt+Enter while focus is in the search box or the replace field |
  | Replace field placeholder | "Replace" |
  | Expansion | In regex mode `$1`, `${name}` and `$$` expand, via `regex::Captures::expand`. In plain mode the text is literal. This applies in the Search view and in the find bar's regex Replace / Replace all. |
  | Replace all enabled | Only when a search has finished (`SearchState::Done`) with at least one result |
  | Per-row replace | A button on the hovered or selected row. It replaces in that note only, and the row then disappears. |
  | Capped results | Replace all only touches the listed notes (§12a) |
  | Open tabs (active or background, clean or dirty) | Changed in the editor as one undo action, not saved |
  | Closed notes | Stamp check (size + last write time against `TextHit.stamp`), decode, replace, encode with the original `Encoding` (BOM kept), `file::saver::save_atomic` |

- **Wording (verbatim):**
  - **Replace all confirmation:** `Replace N matches in M notes with "<text>"?`, where N and M use a thousands separator and the singular forms are "1 match" and "1 note".
    - When the results are capped: `Replace N matches in the 500 listed notes with "<text>"? More notes match; search again to replace in the rest.`
    - When any target note isn't open, a second line is appended: `Notes that aren't open are saved and can't be undone.`
  - **Per-row confirmation**, asked only when the note isn't open: `Replace N matches in "<note name>" with "<text>"? The note is saved and this can't be undone.`
  - **Report notification:** `Replaced N matches in M notes.`
    - Notes that changed since the search add: `K notes were skipped because they changed since the search.`
    - Notes that couldn't be written add: `K notes couldn't be written.`
    - Skipped and failed note names are listed after these lines, one per line, at most 10, then `…and K more`.
    - Singular forms: "1 match", "1 note", "1 note was skipped because it changed since the search.", "1 note couldn't be written."
  - **Palette row:** "Search: Replace in notes".
  - **Accessible names:** "Toggle replace" (the chevron, a button whose state is expanded or collapsed), "Replace" (the field), "Replace all" (button), and "Replace in <note name>" (the per-row button).
- **`CommandId`:** `ReplaceInNotes = 190`, with the accelerator Ctrl+Shift+H. `is_sidebar()` is true. It's a palette row too. Table growth is stated as a delta: `COMMANDS` +1, `ENTRIES` +1, `accelerator_specs` +1 (checked against the code after 3a: 81 → 82, 68 → 69 and 51 → 52).
- **Timer:** `text_search_host::REPLACE_TIMER_ID = 0x4650_5250` retries a held count every 50 ms. It is distinct from `TEXT_SEARCH_TIMER_ID` (`0x4650_5453`), `RECOVERY_TIMER_ID` (`0x4650_5243`), `PREVIEW_TIMER_ID` (`0x4650_5056`), `LIBRARY_WRITE_TIMER_ID` (`0x4650_4C57`) and `AUTOSAVE_TIMER_ID` (`0x4650_4153`); no other code uses the value.
- **Window messages:**
  - `WM_FASTPAD_REPLACE_COUNTED = WM_APP + 14`: the count pass's result.
  - `WM_FASTPAD_REPLACE_WRITTEN = WM_APP + 15`: the write pass's report.
- **Notes mode off:** there is no Search view, so `ReplaceInNotes` does nothing. The find bar's `$1` works in both modes.
- **Commits:** no attribution lines. Never commit `native/out` or `target/`.
- **Tests:**
  - Compile with `cargo clippy --all-targets -- -D warnings`.
  - Run `cargo fmt --all`, then `cargo fmt --all -- --check`.
  - Run only each task's targeted tests. The full suite runs once, at the final review.
  - Put multiple test filters after `--`.
  - Window tests and `tests/windows/*` need `--test-threads=1`.
  - Tests use scratch profiles and scratch folders only, never the real `%LOCALAPPDATA%\FastPad` or Documents. Back up and restore `fastpad.ini` and `folders.ini` around any run of the live app.
  - Never search the whole disk. Crate sources are in `C:\Users\korn3\.cargo\registry\src\`.
- **No backward compatibility:** no migrations.

## Review Focus

1. **A note changed on disk between the search and the replace** (a sync client, or another editor). It is skipped, listed in the report and never overwritten. A note deleted in between is reported as couldn't-be-written, with no crash. Pinned in Task 2 (`a_note_changed_since_the_search_is_skipped_not_overwritten`).
2. **Encodings and line endings survive a write byte for byte** apart from the replaced text: UTF-8, UTF-8 with BOM, UTF-16 LE with BOM, CRLF and LF, and a file with no final newline. Pinned in Task 2 (`a_write_keeps_the_encoding_bom_and_line_endings`).
3. **`$1` with a group that didn't take part in the match, `$$`, `${name}`, and a literal `$` in plain mode.** Expansion follows `regex::Captures::expand` exactly in regex mode, and plain mode never expands. Pinned in Task 1 (`expansion_follows_the_regex_crate_and_plain_mode_is_literal`).
4. **A replacement that creates a new match** (replacing `a` with `aa`), or whose text contains the query. Each original match is replaced exactly once, and nothing loops. Pinned in Task 1 (`a_replacement_containing_the_query_is_applied_once`).
5. **Replace all while a note in the results is open in a background tab with unsaved edits.** That tab is changed in the editor from its live text, with one undo action, and is never written to disk. The same note is never also written as a closed note. Pinned in Task 6 (`a_background_dirty_tab_is_replaced_in_the_editor_not_on_disk`).

## File Map

| File | Responsibility | Task |
|---|---|---|
| `src/search/matcher.rs` | `Matcher::replacements`, `Matcher::replace_text` (expansion) | 1 |
| `src/library/text_replace.rs` (new), `src/library/mod.rs`, `src/library/text_search.rs` | `ReplaceTarget`, `ReplaceCount`, `count`, `ReplaceReport`, `apply`; `LibraryState::record_written`; `Stamp::of` | 2 |
| `src/editor/scintilla.rs`, `src/window/find_bar.rs`, `src/window/main_window.rs` | `Editor::replace_ranges_with` (replacing `replace_ranges`); `find_bar::replacement_for` (replacing `is_match`); the find bar's regex Replace and Replace all expand `$1` | 3 |
| `src/window/{commands,menus,command_palette,main_window,search_view}.rs` | `ReplaceInNotes` (Ctrl+Shift+H), its palette row, `single_line_selection` made `pub(crate)`, and dispatch to `search_view::show_replace` | 4 |
| `src/window/search_view.rs`, `src/window/side_panel.rs`, `src/window/main_window.rs` (tests) | The chevron, the replace EDIT, the Replace all button, the per-row button, Ctrl+Alt+Enter, layout, painting and hit-testing. `option_toggles.rs` is unchanged. | 5 |
| `src/window/text_search_host.rs`, `src/window/{messages,mod,main_window,search_view,tabs,status}.rs` | The replace flow: count worker, confirm, open-tab application, write worker, report, library update, re-run; `Tabs::note_background_edit`; a several-line notification on the status bar. `library_host.rs` is unchanged. | 6 |
| `src/window/sidebar_accessibility.rs`, `search_view.rs`, `main_window.rs` (tests), `tests/windows/library.rs`, `README.md`, spec §17 | Accessibility for the new controls, the end-to-end test, docs and implementation notes | 7 |

## Task Interfaces (the contract every task follows)

The names below are binding. A task may add private helpers, but it must not rename these.

- **Task 1: expansion** (`src/search/matcher.rs`)
  - `pub fn replacements(&self, text: &str, template: &str) -> Vec<(Range<usize>, String)>` gives every match, in `find_iter` order, paired with its expanded replacement. The byte ranges are ascending and never overlap; no match gives an empty `Vec`.
    - In regex mode the replacement is expanded with the captures of that match (`regex::Captures::expand`), following the per-line / whole-text rule of `find_iter`.
    - In plain mode the template is used literally.
  - `pub fn replace_text(&self, text: &str, template: &str) -> (String, usize)` gives the whole text with every match replaced, and the number of matches.
- **Task 2: file replace** (`src/library/text_replace.rs`, `pub mod text_replace;`)
  - `impl Stamp { pub(super) fn of(metadata: &std::fs::Metadata) -> Stamp }` in `text_search.rs`, which `read_note` uses too. It stays `pub(super)`: nothing in `window/` needs it.
  - `#[derive(Clone, Debug, Eq, PartialEq)] pub struct ReplaceTarget { pub path: PathBuf, pub stamp: Stamp }`. `path` is relative to the notebook, and `stamp` comes from `TextHit`.
  - `#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)] pub struct ReplaceCount { pub matches: usize, pub notes: usize }`.
  - `pub fn count(notebook: &Path, targets: &[ReplaceTarget], overlays: &HashMap<PathBuf, String>, matcher: &Matcher, cancel: &AtomicBool) -> ReplaceCount`. Overlay keys use `library::path_key`. An overlay's text wins, as in search. Unreadable notes count 0. It never compares stamps.
  - `#[derive(Clone, Debug, Default, Eq, PartialEq)] pub struct ReplaceReport { pub matches: usize, pub written: Vec<(PathBuf, Stamp)>, pub changed: Vec<PathBuf>, pub failed: Vec<(PathBuf, String)> }`. Every path is relative to the notebook.
    - `written` holds the notes that were replaced into and saved, each with the stamp of the saved file, so the library can be updated without reading the disk. The count of notes replaced into is `written.len()`.
    - `changed` holds the notes whose stamp didn't match.
    - `failed` holds each note that couldn't be read, decoded or saved, with the error text.
  - `pub fn apply(notebook: &Path, targets: &[ReplaceTarget], matcher: &Matcher, template: &str, cancel: &AtomicBool) -> ReplaceReport`. It handles one note at a time:
    - Read the note and compare its `Stamp` (the size, plus `last_write_time` from the opened file's metadata) with `target.stamp`. A mismatch goes to `changed`.
    - Decode, then `replace_text`. Zero matches means the note is untouched.
    - Encode with the original `Encoding`, then `save_atomic`.
    - Then take `Stamp::of(&std::fs::metadata(path))` on the worker and push `(path, stamp)` to `written`. If that read fails, the note's matches still count, but it is left out of `written` (the library isn't told; the next rescan takes the change).
    - Cancel is checked before each note. A note already being written finishes, and the partial report is returned.
  - `pub fn record_written(&mut self, relative: &Path, stamp: Stamp) -> bool` on `LibraryState` (`src/library/mod.rs`) makes the update `add_note`'s existing-entry branch makes (size, mtime, `online_only = false`, `touched.push`) from `stamp`, with no disk access. It returns false, changing nothing, when the note isn't listed.
- **Task 3: find bar `$1`**
  - `Editor::replace_ranges_with(&self, edits: &[(Range<usize>, String)]) -> Result<usize>` takes the edits in ascending order, applies them from the end of the document backwards as one undo action, returns the count, and returns `Ok(0)` for an empty list. `Editor::replace_ranges` is removed.
  - `pub(crate) fn find_bar::replacement_for(editor: &Editor, query: &str, replacement: &str, options: MatchOptions, selection: Range<usize>) -> Option<String>` replaces `is_match`. It returns the text Enter-replace inserts when the selection is exactly a match: in regex mode one of `Matcher::replacements`' matches, expanded.
  - In regex mode the find bar's Replace (current) and Replace all compute their text with `Matcher::replacements` over `with_document_text`, then apply it with `replace_ranges_with`. Plain mode is unchanged.
- **Task 4: command**
  - `CommandId::ReplaceInNotes = 190` (Ctrl+Shift+H) calls `search_view::show_replace(hwnd)`.
  - `show_replace` shows the Search view, opens the replace field and focuses it (Task 5 completes the field). If the editor has a single-line selection, the selection fills the search box, as with Ctrl+Shift+F.
  - `SearchView.replace_open: bool`; `main_window::single_line_selection` becomes `pub(crate)`; `search_view::replace_open(hwnd) -> bool` (test-only until Task 6).
- **Task 5: view UI** (`src/window/search_view.rs`)
  - Fields:
    - `SearchView.replace_open: bool` (Task 4).
    - `SearchView.replace_edit: Option<HWND>`, created the first time the field opens.
    - `SearchView.replace_text: String`, cached on `EN_CHANGE` (no `WM_GETTEXT` under a borrow).
    - `SearchView.row_hover_button: Option<usize>`.
  - Functions:
    - `pub(crate) fn show_replace(hwnd: HWND)`;
    - `pub(crate) fn toggle_replace(hwnd: HWND)`;
    - `pub(crate) fn replace_text(hwnd: HWND) -> String`;
    - `pub(crate) fn replace_all_enabled(hwnd: HWND) -> bool`, and the rule it reads, `SearchView::replace_all_enabled(&self)`;
    - `pub(crate) fn edit_changed(hwnd: HWND, edit: HWND)`, which `side_panel` calls for `EN_CHANGE` from either field;
    - `SearchView::chevron_rect(client, dpi) -> RECT`, `replace_field_rect(client, dpi) -> RECT`, `replace_all_rect(client, dpi) -> RECT` and `row_replace_rect(row: RECT, dpi) -> RECT`, which are associated functions;
    - `SearchView::summary_rect(&self, client, dpi)` (now a method: the summary moves down while the replace row shows).
  - A click on Replace all, or Ctrl+Alt+Enter, calls `text_search_host::replace_all(hwnd)`. A click on a row's button calls `text_search_host::replace_in(hwnd, path: &Path)`. Task 5 hit-tests and swallows these presses and marks `replace_text(hwnd)` and `replace_all_enabled(hwnd)` `expect(dead_code)`; Task 6 wires the calls and removes both attributes.
- **Task 6: flow** (`src/window/text_search_host.rs`)
  - `pub(crate) fn replace_all(hwnd: HWND)` and `pub(crate) fn replace_in(hwnd: HWND, relative: &Path)` build a `Matcher` from `search_view::run_query`, and the targets from `search_view::replace_candidates(hwnd) -> (Vec<Candidate>, bool)` (the `bool` is "capped"), where `pub(crate) struct Candidate { pub path: PathBuf, pub name: String, pub stamp: Option<Stamp> }`.
  - Both need a finished search with results and the replace field open, and do nothing while a replace runs or a modal loop or file population is active.
  - The overlays for the count are the editor texts of every open target tab, active or background, clean or dirty.
  - They spawn the count worker, which posts `WM_FASTPAD_REPLACE_COUNTED` with a `Box<ReplaceCounted { generation, count: ReplaceCount, plan }>`.
  - `pub(crate) fn replace_counted(hwnd: HWND, lparam: LPARAM)`:
    - A count that arrives during a modal loop or file population is held and retried on `REPLACE_TIMER_ID` every 50 ms (`pub(crate) fn replace_timer(hwnd: HWND)`).
    - Confirm with the wording above (`confirm_text`, `row_confirm_text`), using `modal::confirm` with nothing borrowed.
    - On yes, split the targets again as the tabs are now. Apply to the open tabs on the UI thread with `main_window::replace_in_document(hwnd, id, matcher, template) -> Option<usize>` (swapping background documents in through `with_inactive_document`, under the modal and population guard; `Tabs::note_background_edit` marks a background tab edited). Each tab gets one undo action through `Matcher::replacements` and `replace_ranges_with`.
    - A closed note whose hit has no stamp (it came from a tab that has since closed) is never written: it is left out of `apply`'s targets and added to the report's `changed`.
    - Then spawn the write worker for the closed notes. It posts `WM_FASTPAD_REPLACE_WRITTEN` with a `Box<ReplaceWritten { generation, report: ReplaceReport, tab_matches: usize, tab_notes: usize }>`.
  - `pub(crate) fn replace_written(hwnd: HWND, lparam: LPARAM)`:
    - Update the library for each `(path, stamp)` in `written` with `LibraryState::record_written` inside `library_host::with_state`.
    - Push the report notification (`report_text`), with `tab_notes + written.len()` notes.
    - Re-run the query with `run_now`.
  - A generation counter drops stale count and write messages, for example after a notebook change. It is separate from the search generation. `pub(crate) fn cancel_replace(hwnd: HWND)` bumps it; `forget` and `WM_DESTROY` call it.
- **Task 7: accessibility, e2e and docs.** `sidebar_accessibility::{STATE_UNAVAILABLE, expander_item, action_item}`. The chevron (after the toggles), the field, the Replace all button and the selected row's replace button are exposed as described under Wording. There is an e2e test on the real exe, README shortcuts, and spec §17 notes.

---

### Task 1: Matcher expansion (`replacements`, `replace_text`)

**Files:**
- Modify: `src/search/matcher.rs`
  - `impl Matcher`: add `pub fn replacements` and `pub fn replace_text` right after `find_iter` (line 161), before `find_at`.
  - `fn each_match` (lines 260–274): its line loop moves into a new private `fn each_segment`, which `each_match` and `replacements` both call. `each_match`'s behavior doesn't change.
  - `mod tests`: add a private helper `crate_replace_all` and two tests right before `a_matcher_can_be_shared_with_the_search_thread` (line 856).

**Interfaces:**
- Consumes:
  - `regex` 1.13.1: `Regex::captures_iter`, `Captures::get_match`, `Captures::expand`.
  - The existing `Engine`, `per_line` and `find_iter`.
- Produces:
  - `pub fn replacements(&self, text: &str, template: &str) -> Vec<(Range<usize>, String)>`
  - `pub fn replace_text(&self, text: &str, template: &str) -> (String, usize)`
  - Private: `fn each_segment(&self, text: &str, visit: &mut dyn FnMut(&str, usize) -> bool)`.

**Decisions this task settles:**
- **`Captures::expand` semantics** (checked in `regex-automata-0.4.18/src/util/interpolate.rs`, which `regex-1.13.1`'s `Captures::expand` calls):
  - `$$` is a literal `$`.
  - `$name` takes the longest run of `[0-9A-Za-z_]`. So `$1a` names a group "1a", which doesn't exist and expands to nothing. An all-digit name is a group number.
  - `${name}` is a braced name that can hold any characters except `}`. A `${` with no closing `}` is literal text.
  - A `$` that starts no reference (at the end, or before a space) is a literal `$`.
  - A group that didn't take part in the match, an out-of-range number and an unknown name all expand to nothing.
  - `Matcher` never hand-parses the template. It calls `expand` on the captures of the match itself.
- **Which captures:**
  - Each match's captures come from `regex.captures_iter` over the same segment `find_iter` matches in: each line without its `\r`/`\n` in per-line mode, or the whole text in whole-text mode. So `^`, `$` and `\b` judge exactly as in `find_iter`.
  - `captures_iter` and `find_iter` walk the same searcher (the same leftmost-first matches and the same empty-match stepping). Empty matches are dropped in both, as in `segment_matches`. The test asserts the two give the same ranges.
- **Group numbering under whole word:** `regex_engine` wraps the pattern as `\b(?:{query})\b`. The wrapper is non-capturing, so `$1` is still the user's first group, and named groups keep their names. The test pins this.
- **Plain mode** never expands. Every match gets `template.to_owned()`, including `$1`, `$$` and `${x}`.
- **`replace_text`** builds the new text from the original text and the list of matches in one pass. A replacement is never searched again, so `a` → `aa` ends and replaces each original match once.
- **Order:** `replacements` returns its byte ranges in ascending order, never overlapping, as `find_iter` does. `Editor::replace_ranges_with` (Task 3) and the Search replace (Task 6) rely on it; the first test asserts it for every pattern.

- [ ] **Step 1: Write the failing tests**

In `src/search/matcher.rs`'s `mod tests`, add the helper and the two tests right before `#[test] fn a_matcher_can_be_shared_with_the_search_thread()`. `RegexBuilder` is already in scope through `use super::*;`.

```rust
    /// `text` with every match replaced by the `regex` crate itself: `Regex::replace_all` on each
    /// line, or on the whole text when the pattern names a newline, as `Matcher` matches.
    fn crate_replace_all(
        pattern: &str,
        options: MatchOptions,
        text: &str,
        template: &str,
    ) -> String {
        let per_line = !pattern.contains(r"\n");
        let pattern = if options.whole_word {
            format!(r"\b(?:{pattern})\b")
        } else {
            pattern.to_owned()
        };
        let regex = RegexBuilder::new(&pattern)
            .case_insensitive(!options.case)
            .multi_line(!per_line)
            .build()
            .unwrap();
        if !per_line {
            return regex.replace_all(text, template).into_owned();
        }
        text.split('\n')
            .map(|line| regex.replace_all(line, template).into_owned())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn expansion_follows_the_regex_crate_and_plain_mode_is_literal() {
        // Break caught (review focus 3): a hand-rolled `$1` that differs from the crate (a group
        // that took no part printed as "$2" or panicking, `$$` left doubled, `$1a` read as group
        // 1), every match expanded with the first match's groups, group numbers shifted by the
        // whole-word wrapper, or plain mode expanding `$1`.
        let regex = options(false, false, true);
        let text = "a1 b c3\nd4";
        let found = matcher(r"(\w)(\d)?", regex);
        assert_eq!(
            found.replace_text(text, "[$1|$2]").0,
            "[a|1] [b|] [c|3]\n[d|4]"
        );
        assert_eq!(found.replace_text(text, "$$1").0, "$1 $1 $1\n$1");
        assert_eq!(found.replace_text(text, "$0$0").0, "a1a1 bb c3c3\nd4d4");
        let named = matcher(r"(?<letter>\w)(?<digit>\d)?", regex);
        assert_eq!(named.replace_text(text, "${digit}$letter").0, "1a b 3c\n4d");
        let one = matcher(r"(\w)", regex);
        assert_eq!(
            one.replace_text("x", "$1a|${1}a|$9|${nope}|$|${1").0,
            "|xa|||$|${1"
        );

        // Every template against the crate's own `replace_all`, in each mode `Matcher` has.
        let templates = [
            "$1-$2",
            "${2}${1}",
            "$$",
            "$",
            "$1a",
            "${9}",
            "<$0>",
            "no groups",
        ];
        let patterns = [
            (
                r"(\w+)@(\w+)",
                options(false, false, true),
                "ann@site, BOB@HOST x@",
            ),
            (
                r"(\w+)@(\w+)",
                options(true, false, true),
                "ann@site, BOB@HOST x@",
            ),
            (
                r"(fo+)(x)?",
                options(false, true, true),
                "foo foobar fooo foox",
            ),
            (r"^(\w)(\w*)$", options(false, false, true), "ab\ncd\nef"),
            (r"(\w)\n(\w)", options(false, false, true), "a\nb c\nd"),
            (r"(?<left>\w)\r\n(?<right>\w)", regex, "a\r\nb"),
        ];
        for (pattern, options, text) in patterns {
            let found = matcher(pattern, options);
            for template in templates {
                assert_eq!(
                    found.replace_text(text, template).0,
                    crate_replace_all(pattern, options, text, template),
                    "{pattern:?} {options:?} {template:?}"
                );
            }
            let ranges: Vec<Range<usize>> = found
                .replacements(text, "$1")
                .into_iter()
                .map(|(range, _)| range)
                .collect();
            assert_eq!(
                ranges,
                found.find_iter(text),
                "{pattern:?}: the same matches"
            );
            // `Editor::replace_ranges_with` and the Search replace rely on this order.
            assert!(
                ranges.windows(2).all(|pair| pair[0].end <= pair[1].start),
                "{pattern:?}: ascending and never overlapping"
            );
        }
        let word = matcher(r"(\w+)@(\w+)", options(false, true, true));
        assert_eq!(
            word.replace_text("ann@site x", "$2 at $1").0,
            "site at ann x",
            "whole word keeps the pattern's group numbers"
        );

        // Plain mode: the template is text, whatever it holds.
        for case in [false, true] {
            let plain = matcher("$1", options(case, false, false));
            assert_eq!(
                plain.replacements("a $1 b $1", "$2$$ ${x}"),
                [
                    (2..4, "$2$$ ${x}".to_owned()),
                    (7..9, "$2$$ ${x}".to_owned())
                ]
            );
        }
        let folded = matcher("é", MatchOptions::default());
        assert_eq!(folded.replace_text("é É", "$0").0, "$0 $0");
    }

    #[test]
    fn a_replacement_containing_the_query_is_applied_once() {
        // Break caught (review focus 4): replacing until nothing matches, so `a` → `aa` never
        // ends, or matching again inside text already replaced.
        let plain = MatchOptions::default();
        assert_eq!(
            matcher("a", plain).replace_text("banana", "aa"),
            ("baanaanaa".to_owned(), 3)
        );
        assert_eq!(
            matcher("(a)", options(false, false, true)).replace_text("banana", "$1$1"),
            ("baanaanaa".to_owned(), 3)
        );
        assert_eq!(
            matcher("foo", plain).replace_text("foo Foo", "FOO foo"),
            ("FOO foo FOO foo".to_owned(), 2)
        );
        // A replacement that joins the text beside it into a new match is not matched either.
        assert_eq!(
            matcher("ab", plain).replace_text("aabb", "a"),
            ("aab".to_owned(), 1)
        );
        assert_eq!(
            matcher(r"a\nb", options(false, false, true)).replace_text("a\nb", "a\nb a\nb"),
            ("a\nb a\nb".to_owned(), 1)
        );
        // Line endings, and the text around the matches, are kept as they are.
        assert_eq!(
            matcher("foo", plain).replace_text("foo\r\nx foo\nfoo", "bar"),
            ("bar\r\nx bar\nbar".to_owned(), 3)
        );
        // An empty match at a position is never replaced; without a match the text is as it was.
        assert_eq!(
            matcher(r"x|\b", options(false, false, true)).replace_text("ab x", "_"),
            ("ab _".to_owned(), 1)
        );
        assert_eq!(
            matcher("zeta", plain).replace_text("alpha", "beta"),
            ("alpha".to_owned(), 0)
        );
    }
```

The crate oracle's per-line split keeps each line's `\r`, but `Matcher`'s doesn't. None of these patterns can take in a `\r` at a line's end, which is why the per-line `^…$` case uses LF-only text. `matches_never_span_lines_or_take_in_a_lines_carriage_return` already covers `\r` handling.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib -- search::matcher`
Expected: a compile error, `no method named replace_text found for struct Matcher` (and `replacements`).

- [ ] **Step 3: Implement**

In `impl Matcher`, right after `find_iter` and before the `find_at` doc comment, add:

```rust
    /// Every match, in `find_iter`'s order (ascending byte ranges into `text`, never overlapping),
    /// with the text that replaces it. No match gives an empty `Vec`.
    ///
    /// - In regex mode it is `template` expanded with that match's own captures by
    ///   `regex::Captures::expand`: `$1` and `${1}` are group 1, `$name` and `${name}` a named
    ///   group, and `$$` is a `$`. A group that took no part in the match, or doesn't exist, is
    ///   empty, and `$1a` names a group "1a". The captures come from the compiled regex on the
    ///   same line (or the whole text) that `find_iter` matches in, so `^`, `$` and `\b` judge
    ///   the same way. Whole word wraps the pattern in a group that captures nothing, so the
    ///   numbers are the pattern's own.
    /// - In plain mode it is `template` itself, never expanded.
    pub fn replacements(&self, text: &str, template: &str) -> Vec<(Range<usize>, String)> {
        let Engine::Regex(regex) = &self.engine else {
            return self
                .find_iter(text)
                .into_iter()
                .map(|range| (range, template.to_owned()))
                .collect();
        };
        let mut all = Vec::new();
        self.each_segment(text, &mut |segment, base| {
            for captures in regex.captures_iter(segment) {
                let found = captures.get_match();
                // As in `segment_matches`: an empty match at a position (`\b`) is never a match.
                if found.is_empty() {
                    continue;
                }
                let mut replacement = String::new();
                captures.expand(template, &mut replacement);
                all.push((found.start() + base..found.end() + base, replacement));
            }
            true
        });
        all
    }

    /// `text` with every match replaced by its `replacements` text, and how many there were. Each
    /// match of the original text is replaced once: a replacement that contains the query, or
    /// makes a new match with the text beside it, is never matched again.
    pub fn replace_text(&self, text: &str, template: &str) -> (String, usize) {
        let edits = self.replacements(text, template);
        if edits.is_empty() {
            return (text.to_owned(), 0);
        }
        let mut replaced = String::with_capacity(text.len());
        let mut copied = 0;
        for (range, replacement) in &edits {
            replaced.push_str(&text[copied..range.start]);
            replaced.push_str(replacement);
            copied = range.end;
        }
        replaced.push_str(&text[copied..]);
        (replaced, edits.len())
    }
```

Replace `each_match` (from its `/// Calls \`emit\` with each match until it returns false.` doc comment to its closing brace) with:

```rust
    /// Calls `emit` with each match until it returns false.
    fn each_match(&self, text: &str, emit: &mut dyn FnMut(Range<usize>) -> bool) {
        self.each_segment(text, &mut |segment, base| {
            self.segment_matches(segment, base, emit)
        });
    }

    /// Calls `visit` with each piece of `text` that is matched on its own, and the byte of the
    /// text it starts at, until it returns false: each line without its `\n` or `\r\n`, or the
    /// whole text when matching isn't per line.
    fn each_segment(&self, text: &str, visit: &mut dyn FnMut(&str, usize) -> bool) {
        if !self.per_line {
            visit(text, 0);
            return;
        }
        let mut start = 0;
        for line in text.split('\n') {
            let body = line.strip_suffix('\r').unwrap_or(line);
            if !visit(body, start) {
                return;
            }
            start += line.len() + 1;
        }
    }
```

`find_at` and `last_before` keep their own line walks: they start from a line other than the first, and walk backwards.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib -- search::matcher`
Expected: all 21 `matcher` tests pass (19 existing, 2 new). `forty_megabytes_of_text_are_searched_quickly` still passes, because `each_match` only gained one indirect call per segment.

Run: `cargo clippy --all-targets -- -D warnings`, then `cargo fmt --all`, then `cargo fmt --all -- --check`
Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add src/search/matcher.rs
git commit -m "feat(search): Matcher::replacements and replace_text expand \$1, \${name} and \$\$ with each match's own captures in regex mode, and keep the template literal in plain mode"
```

---

### Task 2: `src/library/text_replace.rs` (ReplaceTarget, ReplaceCount, count, ReplaceReport, apply) and `LibraryState::record_written`

**Files:**
- Modify: `src/library/mod.rs`
  - Add `pub mod text_replace;` between `pub mod store;` and `pub mod text_search;` (line 13).
  - `impl LibraryState`: add `pub fn record_written` right after `add_note` (lines 467–502).
  - `mod tests`: add `a_replace_write_is_recorded_as_a_save_is_without_reading_the_disk` right after `a_truncated_rescan_caps_the_merged_notes_list_at_the_scan_limit`, before the `entries` helper.
- Modify: `src/library/text_search.rs`
  - Add `impl Stamp { pub(super) fn of(metadata: &std::fs::Metadata) -> Stamp }` after the `Stamp` struct (lines 58–63).
  - `fn read_note` (lines 372–405): build its stamp with `Stamp::of(&metadata)`. The value is unchanged.
- Create: `src/library/text_replace.rs`

**Interfaces:**
- Consumes:
  - From Task 1: `Matcher::replace_text`. Also `Matcher::find_iter`.
  - `library::path_key` (`pub(crate)`, `mod.rs:131`).
  - `text_search::{Stamp, MAX_NOTE_BYTES}`.
  - `file::encoding::{decode, encode}` (`DecodedText.encoding`) and `file::saver::save_atomic`.
  - `library::store::stats_taken` (test only), to prove `record_written` does no disk access.
- Produces:
  - Everything in the Task Interfaces list for `text_replace`, with exactly these signatures: `ReplaceTarget`, `ReplaceCount`, `count`, `ReplaceReport { matches, written: Vec<(PathBuf, Stamp)>, changed, failed }` and `apply`.
  - `pub fn record_written(&mut self, relative: &Path, stamp: Stamp) -> bool` on `LibraryState`.
  - `Stamp::of(&Metadata) -> Stamp`, `pub(super)`, so it is visible across `library`. The search and the replace take a stamp the same way. Nothing in `window/` needs it: `apply` returns the stamps and `record_written` takes them, so it stays `pub(super)`.
  - Private: `apply_until`, the loop `apply` runs, with the cancel check as a closure so a test can cancel between two notes.

**Decisions this task settles:**
- **The stamp check:**
  - `apply` opens the note with `File::open` and takes `Stamp::of(&file.metadata())`, which is exactly what the search did (`read_note`). It then reads at most `MAX_NOTE_BYTES + 1` bytes.
  - The note counts as changed (`changed`, nothing written) when the stamp differs from `target.stamp`, or when the number of bytes read differs from the stamp's size. That second check catches a file being rewritten during the read, and a file that grew past 4 MB.
  - The check runs right before the write. A change landing in the milliseconds between the read and `ReplaceFileW` isn't seen; that's accepted, and Task 7 records it in §17.
- **Failures:** a note that can't be opened (deleted since the search), read, decoded or saved goes to `failed` with the error text:
  - an I/O error uses `io::Error`'s `Display`;
  - a save failure uses `FastPadError`'s `Display`;
  - a decode failure uses the fixed "The note isn't UTF-8 or UTF-16 text.".
  - Nothing is written to the note, and no crash. A deleted note is never recreated, because the open fails before any write.
- **Zero matches:** a note whose stamp matches but where `replace_text` finds nothing is left untouched, and it appears in no list.
- **Encoding:** the new text is `encoding::encode(&text, decoded.encoding)`, so UTF-8, UTF-8 with BOM, UTF-16 LE and UTF-16 BE all round-trip with their BOM. Line endings and a missing final newline survive, because the decoded text is never normalized, and `replace_text` copies everything outside the matches verbatim.
- **`written` carries each saved note's stamp.** Right after `save_atomic` returns, `replace_note` reads `std::fs::metadata` of the note, on the worker, and records `Stamp::of` it. Task 6 hands these stamps to `LibraryState::record_written`, so the UI thread never reads a written note's metadata (500 notes' worth under `add_note` would break the latency rule). There is no separate `notes` list: the notes replaced into are `written`, and their count is `written.len()`.
- **When that metadata read fails** (the note vanished in the moment after its save), the note's matches still count in `matches`, but it is left out of `written`: the library isn't told, the next rescan takes its new size and time as it would any outside change, and the report's note count misses it. It is not put in `failed`, because it was written. Task 7 records this in §17.
- **`record_written`** makes the update `add_note` makes for a note already listed (size, mtime, `online_only = false`, a `touched` entry), from the given stamp, with no disk access. `Stamp.mtime` and `NoteEntry.mtime` are both FILETIME ticks, so the stamp's values go in as they are. A path that isn't listed changes nothing and returns false: `add_note`'s insert branch isn't needed, since a replace only writes notes the search listed.
- **A cancelled `apply` returns a partial report:** the notes saved before the cancel are in `written`, and every path in the report is relative to the notebook. The cancel test pins both, cancelling between the first and the second note through `apply_until`.
- **`count`:**
  - Overlay keys go through `path_key`, and an overlay's text is counted instead of the disk's.
  - A note that can't be read or decoded, or is over `MAX_NOTE_BYTES`, counts 0.
  - `count` does **not** check stamps; `apply` does.
  - A note with 0 matches doesn't count toward `notes`.
  - Cancel is checked before each note. A cancelled count is partial, and the caller drops it.
- **Cancel in `apply`** is checked before each note. A note already being written finishes, because the check sits only at the top of the loop, and the report so far is returned.
- **One byte buffer** is reused for every note in a pass, as the search does.

- [ ] **Step 1: Write the failing tests**

Add `pub mod text_replace;` to `src/library/mod.rs` between `pub mod store;` and `pub mod text_search;`. Then create `src/library/text_replace.rs` with only the test module for now:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::MatchOptions;
    use std::time::{Duration, SystemTime};

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(label: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "fastpad-text-replace-{label}-{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).unwrap();
            Self(root)
        }

        fn path(&self, relative: &str) -> PathBuf {
            self.0.join(relative)
        }

        /// Writes `bytes` at `relative` and returns it as a target, stamped as a search would
        /// stamp what it read.
        fn note(&self, relative: &str, bytes: &[u8]) -> ReplaceTarget {
            let path = self.path(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, bytes).unwrap();
            ReplaceTarget {
                path: PathBuf::from(relative),
                stamp: stamp_of(&path),
            }
        }

        fn read(&self, relative: &str) -> Vec<u8> {
            std::fs::read(self.path(relative)).unwrap()
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn stamp_of(path: &Path) -> Stamp {
        Stamp::of(&std::fs::File::open(path).unwrap().metadata().unwrap())
    }

    fn plain(query: &str) -> Matcher {
        Matcher::new(query, MatchOptions::default()).unwrap()
    }

    fn not_cancelled() -> AtomicBool {
        AtomicBool::new(false)
    }

    #[test]
    fn a_note_changed_since_the_search_is_skipped_not_overwritten() {
        // Break caught (review focus 1): a sync client's or another editor's change overwritten
        // by a replace made from an older search, a deleted note recreated or crashing the
        // write, or a note without a match rewritten anyway.
        let scratch = Scratch::new("changed");
        let kept = scratch.note("kept.md", b"foo one");
        let grown = scratch.note("grown.md", b"foo two");
        let same_size = scratch.note("same-size.md", b"foo six");
        let deleted = scratch.note("deleted.md", b"foo ten");
        let no_match = scratch.note("no-match.md", b"nothing");
        std::fs::write(scratch.path("grown.md"), b"foo two, and more").unwrap();
        // The same size, and a last write time that is surely not the one the search took.
        std::fs::write(scratch.path("same-size.md"), b"foo SIX").unwrap();
        std::fs::File::options()
            .write(true)
            .open(scratch.path("same-size.md"))
            .unwrap()
            .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000))
            .unwrap();
        std::fs::remove_file(scratch.path("deleted.md")).unwrap();
        let no_match_before = stamp_of(&scratch.path("no-match.md"));
        let targets = [kept, grown, same_size, deleted, no_match];

        let report = apply(&scratch.0, &targets, &plain("foo"), "bar", &not_cancelled());

        assert_eq!(scratch.read("kept.md"), b"bar one");
        assert_eq!(scratch.read("grown.md"), b"foo two, and more");
        assert_eq!(scratch.read("same-size.md"), b"foo SIX");
        assert!(!scratch.path("deleted.md").exists(), "never recreated");
        assert_eq!(scratch.read("no-match.md"), b"nothing");
        assert_eq!(stamp_of(&scratch.path("no-match.md")), no_match_before);
        assert_eq!(report.matches, 1);
        assert_eq!(
            report.written,
            [(PathBuf::from("kept.md"), stamp_of(&scratch.path("kept.md")))],
            "relative to the notebook, with the saved file's stamp"
        );
        assert_eq!(
            report.changed,
            [PathBuf::from("grown.md"), PathBuf::from("same-size.md")]
        );
        assert_eq!(report.failed.len(), 1);
        assert_eq!(report.failed[0].0, PathBuf::from("deleted.md"));
        assert!(!report.failed[0].1.is_empty(), "the error is named");
    }

    #[test]
    fn a_write_keeps_the_encoding_bom_and_line_endings() {
        // Break caught (review focus 2): a BOM dropped or added, UTF-16 written back as UTF-8,
        // CRLF turned into LF (or the other way), a final newline added, or the replacement's
        // own non-ASCII text mis-encoded.
        fn utf16(text: &str, bom: [u8; 2], unit: fn(u16) -> [u8; 2]) -> Vec<u8> {
            let mut bytes = bom.to_vec();
            bytes.extend(text.encode_utf16().flat_map(unit));
            bytes
        }
        let scratch = Scratch::new("encodings");
        let mixed = "foo\r\nfoo\nlast foo";
        let bom_text = "foo ă\r\nbar\r\n";
        let cases: [(&str, Vec<u8>, Vec<u8>); 5] = [
            (
                "utf8.md",
                mixed.as_bytes().to_vec(),
                b"\xC8\x9B\xC4\x83\r\n\xC8\x9B\xC4\x83\nlast \xC8\x9B\xC4\x83".to_vec(),
            ),
            (
                "bom.md",
                [&[0xEF, 0xBB, 0xBF][..], bom_text.as_bytes()].concat(),
                [&[0xEF, 0xBB, 0xBF][..], "ță ă\r\nbar\r\n".as_bytes()].concat(),
            ),
            (
                "utf16le.md",
                utf16(mixed, [0xFF, 0xFE], u16::to_le_bytes),
                utf16("ță\r\nță\nlast ță", [0xFF, 0xFE], u16::to_le_bytes),
            ),
            (
                "utf16be.md",
                utf16(bom_text, [0xFE, 0xFF], u16::to_be_bytes),
                utf16("ță ă\r\nbar\r\n", [0xFE, 0xFF], u16::to_be_bytes),
            ),
            (
                "no-final-newline.md",
                b"a foo".to_vec(),
                "a ță".as_bytes().to_vec(),
            ),
        ];
        let targets: Vec<ReplaceTarget> = cases
            .iter()
            .map(|(name, before, _)| scratch.note(name, before))
            .collect();

        let report = apply(&scratch.0, &targets, &plain("foo"), "ță", &not_cancelled());

        for (name, _, after) in &cases {
            assert_eq!(&scratch.read(name), after, "{name}");
        }
        assert_eq!(report.matches, 3 + 1 + 3 + 1 + 1);
        assert_eq!(report.written.len(), 5);
        assert!(report.changed.is_empty() && report.failed.is_empty());
    }

    #[test]
    fn regex_mode_expands_groups_in_each_notes_matches() {
        let scratch = Scratch::new("groups");
        let targets = [
            scratch.note("a.md", b"ann@site\r\nbob@host"),
            scratch.note("b.md", b"x@y and $1@z"),
        ];
        let regex = Matcher::new(
            r"(\w+)@(\w+)",
            MatchOptions {
                regex: true,
                ..MatchOptions::default()
            },
        )
        .unwrap();

        let report = apply(&scratch.0, &targets, &regex, "$2 ($1) $$", &not_cancelled());

        assert_eq!(scratch.read("a.md"), b"site (ann) $\r\nhost (bob) $");
        assert_eq!(scratch.read("b.md"), b"y (x) $ and $z (1) $");
        assert_eq!(report.matches, 4);
        let written: Vec<&Path> = report
            .written
            .iter()
            .map(|(path, _)| path.as_path())
            .collect();
        assert_eq!(written, [Path::new("a.md"), Path::new("b.md")]);
    }

    #[test]
    fn a_count_takes_overlays_over_the_disk_and_an_unreadable_note_counts_nothing() {
        // Break caught: the prompt's N counted from the disk while a dirty tab holds other text,
        // an overlay missed because its path differs in case, or a missing note stopping the
        // count.
        let scratch = Scratch::new("count");
        let open = scratch.note("Open.md", b"foo foo foo");
        let closed = scratch.note("sub/closed.md", b"foo\r\nfoo");
        let none = scratch.note("none.md", b"nothing");
        let missing = ReplaceTarget {
            path: PathBuf::from("missing.md"),
            stamp: open.stamp,
        };
        let binary = scratch.note("binary.md", b"foo \xFF");
        let overlays = HashMap::from([(PathBuf::from("open.MD"), "foo".to_owned())]);
        let targets = [open, closed, none, missing, binary];

        let counted = count(
            &scratch.0,
            &targets,
            &overlays,
            &plain("foo"),
            &not_cancelled(),
        );

        assert_eq!(
            counted,
            ReplaceCount {
                matches: 3,
                notes: 2
            }
        );
        let cancelled = AtomicBool::new(true);
        assert_eq!(
            count(&scratch.0, &targets, &overlays, &plain("foo"), &cancelled),
            ReplaceCount::default()
        );
    }

    #[test]
    fn a_cancelled_replace_stops_before_the_next_note_and_reports_what_it_wrote() {
        // Break caught: a cancel that throws away the report of notes already saved (the library
        // never learns of FastPad's own writes), one that keeps writing, or a report whose paths
        // aren't relative to the notebook.
        let scratch = Scratch::new("cancel");
        let targets = [
            scratch.note("a.md", b"foo"),
            scratch.note("b.md", b"foo foo"),
        ];

        let cancelled = AtomicBool::new(true);
        let report = apply(&scratch.0, &targets, &plain("foo"), "bar", &cancelled);
        assert_eq!(report, ReplaceReport::default());
        assert_eq!(scratch.read("a.md"), b"foo");

        // Cancelled after the first note: its write is reported, the second note is untouched.
        let mut asked = 0;
        let report = apply_until(&scratch.0, &targets, &plain("foo"), "bar", &mut || {
            asked += 1;
            asked > 1
        });
        assert_eq!(scratch.read("a.md"), b"bar");
        assert_eq!(scratch.read("b.md"), b"foo foo");
        assert_eq!(report.matches, 1);
        assert_eq!(
            report.written,
            [(PathBuf::from("a.md"), stamp_of(&scratch.path("a.md")))]
        );
        assert!(report.changed.is_empty() && report.failed.is_empty());
    }
}
```

About the fixture:
- `ț` is U+021B (`C8 9B`) and `ă` is U+0103 (`C4 83`).
- The UTF-16 expectations are built with `str::encode_utf16` and a literal BOM, not with `encoding::encode`, so a bug in `encode` can't hide itself.
- `File::set_modified` needs write access, which `File::options().write(true)` gives. It sets a last write time the search can't have seen, so the same-size case never depends on the clock's resolution.
- A written note's expected stamp is `stamp_of` the file after the replace, so the tests pin that `written` carries the saved file's stamp, not the search's.

In `src/library/mod.rs`'s `mod tests`, right after `a_truncated_rescan_caps_the_merged_notes_list_at_the_scan_limit` (before `/// \`count\` index entries that exist only in memory.`), add:

```rust
    #[test]
    fn a_replace_write_is_recorded_as_a_save_is_without_reading_the_disk() {
        // Break caught: FastPad's own replace read as an outside change by the next rescan (the
        // old size or time kept, or the note missing from `touched`), an online-only flag left
        // set so search keeps skipping the note, a stamp taken from the disk on the UI thread
        // for each written note, or a note that isn't listed added.
        let scratch = Scratch::new("record-written");
        let mut state = bare_state(&scratch, entries(2), false);
        state.notes[1].online_only = true;
        let stamp = text_search::Stamp {
            size: 42,
            mtime: 133_700_000_000_000_000,
        };
        let taken = store::stats_taken();

        assert!(state.record_written(Path::new("F1.md"), stamp));
        assert!(!state.record_written(Path::new("missing.md"), stamp));

        assert_eq!(store::stats_taken(), taken, "no stamp taken from the disk");
        let note = &state.notes[1];
        assert_eq!(
            (note.size, note.mtime, note.online_only),
            (42, stamp.mtime, false)
        );
        assert_eq!(state.notes[0].size, 0, "only that note changes");
        assert_eq!(state.notes.len(), 2, "nothing is added");
        assert_eq!(state.touched, [PathBuf::from("F1.md")]);
    }
```

`entries(2)` lists `f0.md` and `f1.md`; `same_path` ignores case, so `F1.md` is `f1.md`. `store::stats_taken` counts every `store::stamp` this thread takes, which is how `add_note` reads the disk.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib -- library::text_replace library::tests::a_replace_write_is_recorded`
Expected: a compile error, `cannot find type ReplaceTarget in this scope` (and `apply`, `apply_until`, `count`, `ReplaceCount`, `ReplaceReport`, `no function or associated item named of found for struct Stamp`, and `no method named record_written found for struct LibraryState`).

- [ ] **Step 3: Implement `Stamp::of`**

In `src/library/text_search.rs`, right after the `Stamp` struct:

```rust
impl Stamp {
    /// The stamp of a file from its metadata: the search and the replace both take it this way
    /// (the replace also right after each save), so a note nobody touched in between has the
    /// same stamp.
    pub(super) fn of(metadata: &std::fs::Metadata) -> Stamp {
        Stamp {
            size: metadata.len(),
            mtime: metadata.last_write_time(),
        }
    }
}
```

At the end of `read_note`, replace

```rust
    Ok((
        decoded.text,
        Stamp {
            size: metadata.len(),
            mtime: metadata.last_write_time(),
        },
    ))
```

with

```rust
    Ok((decoded.text, Stamp::of(&metadata)))
```

`use std::os::windows::fs::MetadataExt;` stays, because `Stamp::of` uses it.

- [ ] **Step 4: Implement `text_replace`**

Put this above the `#[cfg(test)] mod tests` in `src/library/text_replace.rs`:

```rust
//! Replace across notes, run on worker threads: counts the matches in the notes a search listed,
//! and writes the replacements into notes that aren't open, one note at a time. A write happens
//! only when the note is still what the search read (its stamp), and keeps the note's encoding,
//! BOM and line endings. No Win32 and no window.

use super::path_key;
use super::text_search::{MAX_NOTE_BYTES, Stamp};
use crate::file::{encoding, saver};
use crate::search::Matcher;
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};

/// Why a note that couldn't be decoded wasn't written; the search never lists one.
const NOT_TEXT: &str = "The note isn't UTF-8 or UTF-16 text.";

/// A note to replace in.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplaceTarget {
    /// Relative to the notebook, as the note list has it.
    pub path: PathBuf,
    /// What the search read (`TextHit.stamp`).
    pub stamp: Stamp,
}

/// How many matches a replace would change, and in how many notes.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ReplaceCount {
    pub matches: usize,
    /// Notes with at least one match.
    pub notes: usize,
}

/// What `apply` did. Every path is relative to the notebook.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ReplaceReport {
    /// Matches replaced, in every note saved.
    pub matches: usize,
    /// The notes that were replaced into and saved, each with the stamp of the saved file (read
    /// on the worker right after the save), so the library takes in the new size and time
    /// without touching the disk (`LibraryState::record_written`). Its length is how many notes
    /// were replaced into.
    pub written: Vec<(PathBuf, Stamp)>,
    /// The notes whose stamp no longer matched: changed since the search, and left as they are.
    pub changed: Vec<PathBuf>,
    /// The notes that couldn't be read, decoded or saved, with the error.
    pub failed: Vec<(PathBuf, String)>,
}

/// Counts the matches `matcher` finds in `targets`, as `Matcher::find_iter` finds them.
///
/// - `overlays` holds dirty tabs' text by relative path, compared with `path_key`; an overlay's
///   text is counted instead of the disk's, as in search.
/// - A note that can't be read or decoded, or is over `MAX_NOTE_BYTES`, counts 0.
/// - `cancel` is read before each note. A cancelled count is partial; the caller drops it.
pub fn count(
    notebook: &Path,
    targets: &[ReplaceTarget],
    overlays: &HashMap<PathBuf, String>,
    matcher: &Matcher,
    cancel: &AtomicBool,
) -> ReplaceCount {
    let overlays: HashMap<String, &str> = overlays
        .iter()
        .map(|(path, text)| (path_key(path), text.as_str()))
        .collect();
    let mut total = ReplaceCount::default();
    let mut bytes = Vec::new();
    for target in targets {
        if cancel.load(Relaxed) {
            break;
        }
        let matches = match overlays.get(&path_key(&target.path)) {
            Some(text) => matcher.find_iter(text).len(),
            None => disk_text(&notebook.join(&target.path), &mut bytes)
                .map_or(0, |text| matcher.find_iter(&text).len()),
        };
        if matches > 0 {
            total.matches += matches;
            total.notes += 1;
        }
    }
    total
}

/// Replaces every match of `matcher` in `targets` with `template` (expanded as
/// `Matcher::replacements` expands it), one note at a time:
///
/// - The note is read, and its stamp (the opened file's size and last write time, as the search
///   takes them) compared with `target.stamp`. A mismatch, or bytes that don't add up to the
///   stamp's size, goes to `changed` and the note is left as it is.
/// - The bytes are decoded, and `Matcher::replace_text` gives the new text. With no match the
///   note is left as it is and is in no list.
/// - The text is encoded with the note's own `Encoding` (so a BOM stays) and written with
///   `saver::save_atomic`. Line endings stay as they were, since the text is never normalized.
///   The saved file's stamp is then read, here on the worker, and the note goes to `written`
///   with it. If that read fails (the note vanished in the moment after its save), its matches
///   still count but it is left out of `written`, so the library isn't told of it.
/// - A note that can't be opened (deleted since the search, say), read, decoded or saved goes to
///   `failed` with the error, and nothing is written to it.
/// - `cancel` is read before each note; a note already being written is finished, and the
///   report of the notes done so far is returned.
///
/// The stamp is checked just before the write, so a change in the moment between the two is
/// not seen.
pub fn apply(
    notebook: &Path,
    targets: &[ReplaceTarget],
    matcher: &Matcher,
    template: &str,
    cancel: &AtomicBool,
) -> ReplaceReport {
    apply_until(notebook, targets, matcher, template, &mut || {
        cancel.load(Relaxed)
    })
}

/// `apply`'s loop, asking `cancelled` before each note.
fn apply_until(
    notebook: &Path,
    targets: &[ReplaceTarget],
    matcher: &Matcher,
    template: &str,
    cancelled: &mut dyn FnMut() -> bool,
) -> ReplaceReport {
    let mut report = ReplaceReport::default();
    let mut bytes = Vec::new();
    for target in targets {
        if cancelled() {
            break;
        }
        match replace_note(notebook, target, matcher, template, &mut bytes) {
            Ok(Replaced::Written { matches, stamp }) => {
                report.matches += matches;
                if let Some(stamp) = stamp {
                    report.written.push((target.path.clone(), stamp));
                }
            }
            Ok(Replaced::NoMatch) => {}
            Ok(Replaced::Changed) => report.changed.push(target.path.clone()),
            Err(error) => report.failed.push((target.path.clone(), error)),
        }
    }
    report
}

enum Replaced {
    /// Saved, with the saved file's stamp (`None` when it couldn't be read after the save).
    Written {
        matches: usize,
        stamp: Option<Stamp>,
    },
    NoMatch,
    Changed,
}

fn replace_note(
    notebook: &Path,
    target: &ReplaceTarget,
    matcher: &Matcher,
    template: &str,
    bytes: &mut Vec<u8>,
) -> Result<Replaced, String> {
    let path = notebook.join(&target.path);
    let stamp = read_note(&path, bytes)?;
    // A file over the limit is read only to `MAX_NOTE_BYTES + 1`, so its length never matches
    // a stamp the search took (every hit's is within the limit).
    if stamp != target.stamp || bytes.len() as u64 != stamp.size {
        return Ok(Replaced::Changed);
    }
    let decoded = encoding::decode(bytes).map_err(|_| NOT_TEXT.to_owned())?;
    let (text, matches) = matcher.replace_text(&decoded.text, template);
    if matches == 0 {
        return Ok(Replaced::NoMatch);
    }
    saver::save_atomic(&path, &encoding::encode(&text, decoded.encoding))
        .map_err(|error| error.to_string())?;
    // The saved file's stamp, taken here on the worker so the UI thread never reads the disk
    // to update the library.
    let stamp = std::fs::metadata(&path)
        .ok()
        .map(|metadata| Stamp::of(&metadata));
    Ok(Replaced::Written { matches, stamp })
}

/// Reads the note at `path` into `bytes`, at most `MAX_NOTE_BYTES + 1` of them, and returns
/// the stamp of the opened file.
fn read_note(path: &Path, bytes: &mut Vec<u8>) -> Result<Stamp, String> {
    let file = std::fs::File::open(path).map_err(|error| error.to_string())?;
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    bytes.clear();
    file.take(MAX_NOTE_BYTES + 1)
        .read_to_end(bytes)
        .map_err(|error| error.to_string())?;
    Ok(Stamp::of(&metadata))
}

/// The note's text from disk, or `None` when it can't be read or decoded or is over the limit.
fn disk_text(path: &Path, bytes: &mut Vec<u8>) -> Option<String> {
    read_note(path, bytes).ok()?;
    if bytes.len() as u64 > MAX_NOTE_BYTES {
        return None;
    }
    encoding::decode(bytes).ok().map(|decoded| decoded.text)
}
```

- [ ] **Step 5: Implement `LibraryState::record_written`**

In `src/library/mod.rs`'s `impl LibraryState`, right after `add_note`'s closing brace, add:

```rust
    /// Records a note FastPad just wrote outside the editor (a Search replace) from the stamp
    /// the writer took of the saved file, so the next rescan reads no outside change. It is the
    /// update `add_note` makes for a note already listed (size, time, `online_only` cleared, a
    /// `touched` entry), with no disk access: the UI thread calls it for every written note.
    /// Returns false, changing nothing, when `relative` isn't listed.
    pub fn record_written(&mut self, relative: &Path, stamp: text_search::Stamp) -> bool {
        let Some(existing) = self
            .notes
            .iter_mut()
            .find(|note| same_path(&note.path, relative))
        else {
            return false;
        };
        // Both are FILETIME ticks: `Stamp::of` takes `last_write_time`, as the scan does.
        existing.size = stamp.size;
        existing.mtime = stamp.mtime;
        // FastPad just wrote the file, so its data is on this PC.
        existing.online_only = false;
        self.touched.push(relative.to_path_buf());
        true
    }
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test --lib -- library::text_replace library::text_search library::tests::a_replace_write_is_recorded`
Expected: the 5 `text_replace` tests, the 14 existing `text_search` tests and the new `library` test pass (20 in all). `text_search` still passing shows `Stamp::of` gives the stamp `read_note` gave before.

Run: `cargo clippy --all-targets -- -D warnings`, then `cargo fmt --all`, then `cargo fmt --all -- --check`
Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add src/library/mod.rs src/library/text_search.rs src/library/text_replace.rs
git commit -m "feat(library): text_replace counts matches in the listed notes and writes replacements into closed notes, skipping a note changed since the search and keeping its encoding, BOM and line endings; each write reports the saved file's stamp, which LibraryState::record_written takes without reading the disk"
```

---

### Task 3: Find bar `$1` (`Editor::replace_ranges_with`; regex Replace and Replace all use `Matcher::replacements`)

**Files:**
- Modify: `src/editor/scintilla.rs`
  - `replace_ranges` (the `#[cfg(windows)]` fn at lines 614–628 and its `#[cfg(not(windows))]` stub at lines 630–635) becomes `replace_ranges_with`. `replace_ranges` has no other caller, so it's removed rather than kept alongside. The doc comment of `replace_all` (line 568) names `replace_ranges`, and changes to match.
  - `mod tests`: `replace_ranges_replaces_from_the_end_as_one_undo_step` (line 1670) becomes `replace_ranges_with_puts_each_text_in_its_range_from_the_end_as_one_undo_step`.
- Modify: `src/window/find_bar.rs`
  - `is_match` (lines 219–245) becomes `replacement_for`, which returns the text to insert. `is_match` has one caller, `replace_current`.
  - `replace_all` (lines 247–271): the regex branch uses `Matcher::replacements` and `replace_ranges_with`. Its doc comment changes.
- Modify: `src/window/main_window.rs`
  - `replace_current` (lines 1803–1830) calls `find_bar::replacement_for` and inserts its text.
  - `mod tests`: add `the_find_bars_regex_replace_expands_groups_and_plain_replace_is_literal` right before `a_case_insensitive_regex_folds_accented_capitals_and_f3_wraps` (line 13528).

**Interfaces:**
- Consumes:
  - From Task 1: `Matcher::replacements`.
  - `Editor::with_document_text`, `Editor::replace_target`, `begin_undo_action` and `end_undo_action`.
  - `find_bar::regex_matcher` and `search_flags`.
- Produces:
  - `Editor::replace_ranges_with(&self, edits: &[(Range<usize>, String)]) -> Result<usize>`, with the same cfg pair as every other `Editor` method. It takes the ranges in ascending order (as `Matcher::replacements` returns them), applies them from the end, returns the count, and returns `Ok(0)` for an empty list. `Editor::replace_ranges` is removed; no task uses it. The non-Windows stub returns `Err(FastPadError::Invariant("Scintilla editor is only supported on Windows"))`.
  - `find_bar::replacement_for(editor: &Editor, query: &str, replacement: &str, options: MatchOptions, selection: Range<usize>) -> Option<String>` (`pub(crate)`). It replaces `is_match`.

**Decisions this task settles:**
- **Replace (Enter) in regex mode** replaces the selection only when it is **one of `Matcher::replacements`' matches** over the document (the same matches as `find_iter`), and it inserts that match's own expanded text.
  - Before, the test was `find_at(selection.start) == selection`. The two differ only for a hand-made selection that overlaps an earlier `find_iter` match. For example, in "aaa" with `aa`, `find_iter` gives only 0..2, so a selection of 1..3 isn't replaced.
  - A selection that F3 made is always a match of either kind: find next starts at the end of the previous match, or at the caret.
  - The existing whole-word tests (Enter doesn't replace "foo" inside "foobar") still hold, because `find_iter` never yields that match.
- **Plain mode is unchanged:** the same Scintilla check (`search_in_target` over the selection), with `replacement` inserted as typed. `$1` stays literal.
- **Replace all in regex mode** computes every `(range, expanded text)` pair with `Matcher::replacements` inside `with_document_text`, so the text is never copied. Once the closure has returned, it applies them from the end backwards as one undo action. Since the edits are collected before the first one lands, nothing is re-matched.
- **Cost:** Enter in regex mode expands every match in the document, which is one `String` per match, to find the selected one. That's linear in the document, like `find_at`'s whole-text fallback. The `Matcher` is still compiled per press (§17).

- [ ] **Step 1: Write the failing find bar test**

In `src/window/main_window.rs`'s `mod tests`, right before `#[test] fn a_case_insensitive_regex_folds_accented_capitals_and_f3_wraps()`:

```rust
    #[test]
    fn the_find_bars_regex_replace_expands_groups_and_plain_replace_is_literal() {
        // Break caught (spec §12a): the find bar's regex Replace inserting "$2/${year}" as
        // typed, Replace All expanding every match with the first match's groups, Enter using
        // another match's captures, group numbers shifted by the whole-word wrapper, or plain
        // mode expanding `$1`.
        use crate::search::SearchOption;
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        editor
            .populate_clean("2024-09 and 1999-01\r\n2001-12")
            .unwrap();
        execute_command(window.hwnd, CommandId::Replace);
        super::toggle_find_option(window.hwnd, SearchOption::Regex);
        set_find_query(window.hwnd, r"(?<year>\d{4})-(\d{2})");
        set_replace_text(window.hwnd, "$2/${year} $$");

        editor.set_selection(12..19).unwrap();
        super::replace_current(window.hwnd);
        assert_eq!(
            editor.text().unwrap(),
            "2024-09 and 01/1999 $\r\n2001-12",
            "Enter expands the selected match's own groups"
        );
        assert_eq!(
            editor.selection().unwrap(),
            23..30,
            "then moves to the next"
        );

        super::replace_all_matches(window.hwnd);
        assert_eq!(
            editor.text().unwrap(),
            "09/2024 $ and 01/1999 $\r\n12/2001 $"
        );
        editor.undo().unwrap();
        assert_eq!(
            editor.text().unwrap(),
            "2024-09 and 01/1999 $\r\n2001-12",
            "Replace All is one undo step"
        );

        super::toggle_find_option(window.hwnd, SearchOption::WholeWord);
        set_find_query(window.hwnd, "(fo+)");
        set_replace_text(window.hwnd, "<$1>");
        editor.populate_clean("foo foobar fooo").unwrap();
        super::replace_all_matches(window.hwnd);
        assert_eq!(editor.text().unwrap(), "<foo> foobar <fooo>");

        super::toggle_find_option(window.hwnd, SearchOption::WholeWord);
        super::toggle_find_option(window.hwnd, SearchOption::Regex);
        set_find_query(window.hwnd, "$1");
        set_replace_text(window.hwnd, "$2$$");
        editor.populate_clean("a $1 b $1").unwrap();
        editor.set_selection(2..4).unwrap();
        super::replace_current(window.hwnd);
        assert_eq!(
            editor.text().unwrap(),
            "a $2$$ b $1",
            "plain Enter is literal"
        );
        super::replace_all_matches(window.hwnd);
        assert_eq!(
            editor.text().unwrap(),
            "a $2$$ b $2$$",
            "plain Replace All too"
        );
    }
```

`set_find_query` and `set_replace_text` are the existing helpers in this module (`set_find_query` at line 13297; `set_replace_text` just after `opening_a_result_whose_text_changed_shows_no_match`).

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test --lib -- window::main_window::tests::the_find_bars_regex_replace_expands_groups_and_plain_replace_is_literal --test-threads=1`
Expected: FAIL at the first `assert_eq!`: `left: "2024-09 and $2/${year} $$\r\n2001-12"`, `right: "2024-09 and 01/1999 $\r\n2001-12"`. Replace inserts its text literally today.

- [ ] **Step 3: Change the editor test**

In `src/editor/scintilla.rs`'s `mod tests`, replace `replace_ranges_replaces_from_the_end_as_one_undo_step` with:

```rust
    #[test]
    fn replace_ranges_with_puts_each_text_in_its_range_from_the_end_as_one_undo_step() {
        // Break caught: ascending edits (as `Matcher::replacements` gives them) replaced front to
        // back (later ranges shifted onto the wrong text), an edit given another edit's text,
        // one undo step per range, or an empty list failing instead of replacing nothing.
        let editor = test_editor();
        editor.populate_clean("foo bar foo baz foo").unwrap();
        let edits = [
            (0..3, "a".to_owned()),
            (8..11, "ță".to_owned()),
            (16..19, "quux".to_owned()),
        ];
        assert_eq!(editor.replace_ranges_with(&edits).unwrap(), 3);
        assert_eq!(editor.text().unwrap(), "a bar ță baz quux");
        editor.undo().unwrap();
        assert_eq!(editor.text().unwrap(), "foo bar foo baz foo");
        assert_eq!(editor.replace_ranges_with(&[]).unwrap(), 0);
    }
```

Run: `cargo test --lib -- editor::scintilla::tests::replace_ranges_with --test-threads=1`
Expected: a compile error, `no method named replace_ranges_with found for struct TestEditor`.

- [ ] **Step 4: Implement `Editor::replace_ranges_with`**

In `src/editor/scintilla.rs`, replace both `replace_ranges` functions (the `#[cfg(windows)]` one with its doc comment, and the `#[cfg(not(windows))]` stub) with:

```rust
    /// Replaces each edit's range with its text, as one undo action, and returns how many. The
    /// ranges come in ascending order, none overlapping, as `Matcher::replacements` gives them;
    /// they are applied from the last backwards so the earlier positions stay valid. An empty
    /// list returns `Ok(0)` without opening an undo action.
    #[cfg(windows)]
    pub fn replace_ranges_with(&self, edits: &[(Range<usize>, String)]) -> Result<usize> {
        if edits.is_empty() {
            return Ok(0);
        }
        self.begin_undo_action();
        let result = edits
            .iter()
            .rev()
            .try_for_each(|(range, text)| self.replace_target(range.clone(), text).map(drop));
        self.end_undo_action();
        result.map(|()| edits.len())
    }

    #[cfg(not(windows))]
    pub fn replace_ranges_with(&self, _edits: &[(Range<usize>, String)]) -> Result<usize> {
        Err(FastPadError::Invariant(
            "Scintilla editor is only supported on Windows",
        ))
    }
```

The undo action is closed even when a `replace_target` fails partway, as before.

In the doc comment of `replace_all` (the `#[cfg(windows)]` one, just above), replace these two lines

```rust
    /// For plain search flags (the find bar's regex mode uses `replace_ranges`); an empty match
    /// ends it rather than being replaced forever.
```

with

```rust
    /// For plain search flags (the find bar's regex mode uses `replace_ranges_with`); an empty
    /// match ends it rather than being replaced forever.
```

- [ ] **Step 5: Implement the find bar's expansion**

In `src/window/find_bar.rs`, replace `is_match` (from its `/// Whether \`selection\` is exactly a match…` doc comment to its closing brace) with:

```rust
/// The text Replace puts in place of `selection` when the selection is exactly a match of
/// `query` under `options`, else `None`, as Replace checks before it replaces the selection.
///
/// - Plain mode: the match is Scintilla's, found where the selection starts, and the text is
///   `replacement` as it is.
/// - Regex mode: the match is one of `Matcher::replacements` over the document, and the text is
///   `replacement` expanded with that match's captures (`$1`, `${name}`, `$$`).
pub(crate) fn replacement_for(
    editor: &Editor,
    query: &str,
    replacement: &str,
    options: MatchOptions,
    selection: Range<usize>,
) -> Option<String> {
    if query.is_empty() || selection.is_empty() {
        return None;
    }
    if options.regex {
        let matcher = regex_matcher(query, options)?;
        return editor
            .with_document_text(|text| {
                matcher
                    .replacements(text, replacement)
                    .into_iter()
                    .find(|(range, _)| *range == selection)
                    .map(|(_, text)| text)
            })
            .ok()
            .flatten();
    }
    let found = editor
        .search_in_target(query, selection.clone(), search_flags(options))
        .ok()
        .flatten();
    (found == Some(selection)).then(|| replacement.to_owned())
}
```

In `replace_all`, replace the doc comment with:

```rust
/// Replaces every match of `query` under `options` as one undo action, and returns how many.
/// Plain mode puts `replacement` in as it is, through Scintilla's search. Regex mode replaces
/// `Matcher`'s matches from the end backwards, each with `replacement` expanded with its own
/// captures (`Matcher::replacements`).
```

and replace its last lines,

```rust
    let Ok(ranges) = editor.with_document_text(|text| matcher.find_iter(text)) else {
        return 0;
    };
    editor.replace_ranges(&ranges, replacement).unwrap_or(0)
}
```

with

```rust
    let Ok(edits) = editor.with_document_text(|text| matcher.replacements(text, replacement))
    else {
        return 0;
    };
    editor.replace_ranges_with(&edits).unwrap_or(0)
}
```

In `src/window/main_window.rs`'s `replace_current`, replace the comment and the `if let` head, from `// Only replace when the selection is exactly a match…` through `let _ = editor.replace_target(selection, &replacement);`, with:

```rust
    // Only replace when the selection is exactly a match under the options (a case-insensitive
    // "CAT" for "cat", a regex's match, a whole word); otherwise this Enter just moves to the
    // next match, as in a bare Find field. In regex mode the replacement expands `$1` with the
    // selected match's groups; in plain mode it is literal.
    if let Ok(selection) = editor.selection()
        && let Some(text) =
            find_bar::replacement_for(&editor, &query, &replacement, options, selection.clone())
    {
        let _ = editor.replace_target(selection, &text);
```

The rest of `replace_current` (the `is_live_for` check and `find_next`) doesn't change. `replace_all_matches` doesn't change either: it already calls `find_bar::replace_all`.

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test --lib -- editor::scintilla::tests::replace_ranges_with window::main_window::tests::the_find_bars_regex_replace window::main_window::tests::a_whole_word_regex window::main_window::tests::replace_current window::main_window::tests::a_regex_that_can_match_empty --test-threads=1`
Expected: 6 tests pass:
- the new editor test;
- the new find bar test;
- `a_whole_word_regex_matches_a_non_ascii_word_only_as_a_whole_word`;
- `a_whole_word_regex_skips_longer_words_forward_backward_and_in_replace` (Enter still doesn't replace inside "foobar", and Replace All is still one undo step);
- `replace_current_replaces_a_selection_that_matches_under_the_options` (plain, case-insensitive);
- `a_regex_that_can_match_empty_text_shows_no_match_and_replaces_nothing`.

Run: `cargo clippy --all-targets -- -D warnings`, then `cargo fmt --all`, then `cargo fmt --all -- --check`
Expected: clean. `rg -n "replace_ranges\b|is_match\(" src` finds only `inner.is_match("")` in `matcher.rs`.

- [ ] **Step 7: Commit**

```bash
git add src/editor/scintilla.rs src/window/find_bar.rs src/window/main_window.rs
git commit -m "feat(find): the find bar's regex Replace and Replace all expand \$1, \${name} and \$\$ with each match's captures through Matcher::replacements, applied from the end as one undo action; plain mode stays literal"
```

---

### Task 4: `ReplaceInNotes` (190, Ctrl+Shift+H), its palette row, and `show_replace`

**Files:**
- Modify:
  - `src/window/commands.rs`: the `CommandId` enum, `needs_document`, `is_sidebar`, `TryFrom<u16>`'s `COMMANDS`, and the `tests` module.
  - `src/window/menus.rs`: `accelerator_specs()` and the tests `shortcut_and_menu_commands_share_command_ids` and `tab_zoom_and_direction_shortcuts_are_bound`.
  - `src/window/command_palette.rs`: `ENTRIES` and the `tests` module.
  - `src/window/main_window.rs`: `single_line_selection` (made `pub(crate)`), `execute_command_with_note`'s dispatch, the test `with_notes_mode_off_ctrl_shift_f_and_the_search_toggles_do_nothing`, and one new test.
  - `src/window/search_view.rs`: `SearchView` (the `replace_open` field), `SearchView::new`, and the new `show_replace` and `replace_open`.

**Interfaces:**
- Consumes:
  - `side_panel::show_view`, `search_view::show_with_query` and `main_window::single_line_selection` (3a).
  - `execute_command_with_note`'s `is_sidebar()` guard (3a), which already skips sidebar commands with notes mode off.
- Produces:
  - `CommandId::ReplaceInNotes = 190`: `is_sidebar()` is true, `needs_document()` is false, `search_option()` is `None`.
  - The accelerator `Ctrl+Shift+H` → `ReplaceInNotes`. Ctrl+H stays `Replace`.
  - The palette row `"Search: Replace in notes"`, after `"Search: Replace"`.
  - `pub(crate) fn single_line_selection(hwnd: HWND) -> Option<String>` in `main_window` (was private).
  - `SearchView.replace_open: bool` and `pub(crate) fn show_replace(hwnd: HWND)` in `search_view`. This task's `show_replace` shows Search, prefills the box from a one-line selection and sets `replace_open`. Task 5 replaces its body so that it also makes, shows and focuses the replace field.
  - `#[cfg(test)] pub(crate) fn replace_open(hwnd: HWND) -> bool` in `search_view`. Task 6 drops the `cfg(test)`.
- Table sizes change by these deltas. The numbers after 3a are shown for checking:
  - `COMMANDS` in `commands.rs`: +1 (81 → 82).
  - `command_palette::ENTRIES`: +1 (68 → 69).
  - `accelerator_specs()`: +1 (51 → 52).

- [ ] **Step 1: Write the failing command test** in `src/window/commands.rs`'s `tests` module, after `find_next_and_previous_have_stable_values_and_need_a_document`

```rust
    #[test]
    fn replace_in_notes_is_190_a_sidebar_command_and_needs_no_document() {
        // Break caught: 3b's first command renumbered onto another command's value, greyed out
        // while no tab is open, or left running with notes mode off, where there is no Search
        // view (spec §5).
        assert_eq!(CommandId::ReplaceInNotes as u16, 190);
        assert_eq!(CommandId::try_from(190), Ok(CommandId::ReplaceInNotes));
        assert!(CommandId::ReplaceInNotes.is_sidebar());
        assert!(!CommandId::ReplaceInNotes.needs_document());
        assert_eq!(CommandId::ReplaceInNotes.search_option(), None);
        assert_eq!(CommandId::try_from(191), Err(()));
    }
```

Run: `cargo test --lib -- window::commands --test-threads=1`
Expected: compile error ``no variant or associated item named `ReplaceInNotes` found for enum `CommandId` ``.

- [ ] **Step 2: Add the command** in `src/window/commands.rs`

1. In the enum, after `FindPrevious = 189,`, add:

```rust
    ReplaceInNotes = 190,
```

2. In `needs_document`'s `matches!` list, after `| Self::SearchToggleRegex`, add:

```rust
                | Self::ReplaceInNotes
```

3. In `is_sidebar`'s `matches!` list, after `| Self::SearchToggleRegex`, add:

```rust
                | Self::ReplaceInNotes
```

4. In `TryFrom<u16>`, change `const COMMANDS: [CommandId; 81] = [` to `const COMMANDS: [CommandId; 82] = [`, and after `CommandId::FindPrevious,` add:

```rust
            CommandId::ReplaceInNotes,
```

Run: `cargo test --lib -- window::commands --test-threads=1`
Expected: every `window::commands` test passes.

- [ ] **Step 3: Bind Ctrl+Shift+H** in `src/window/menus.rs`

1. Change `pub const fn accelerator_specs() -> [AcceleratorSpec; 51] {` to `pub const fn accelerator_specs() -> [AcceleratorSpec; 52] {`, and after `accelerator(FCONTROL, b'H', CommandId::Replace),` add:

```rust
        accelerator(FCONTROL | FSHIFT, b'H', CommandId::ReplaceInNotes),
```

2. In `shortcut_and_menu_commands_share_command_ids`, change `assert_eq!(specs.len(), 51);` to `assert_eq!(specs.len(), 52);`.

3. In `tab_zoom_and_direction_shortcuts_are_bound`, after the two `VK_F3` assertions at the end, add:

```rust
        // Break caught: Ctrl+Shift+H unbound, or taking Ctrl+H from the find bar's Replace
        // (spec §11).
        assert_eq!(
            bound(FCONTROL | FSHIFT, u16::from(b'H')),
            Some(CommandId::ReplaceInNotes)
        );
        assert_eq!(bound(FCONTROL, u16::from(b'H')), Some(CommandId::Replace));
```

`every_shortcut_chord_maps_to_exactly_one_command` needs no change: Ctrl+Shift+H was free.

- [ ] **Step 4: Add the palette row** in `src/window/command_palette.rs`

1. Change `pub(crate) const ENTRIES: [PaletteEntry; 68] = [` to `pub(crate) const ENTRIES: [PaletteEntry; 69] = [`, and after `entry("Search: Replace", CommandId::Replace),` add:

```rust
    entry("Search: Replace in notes", CommandId::ReplaceInNotes),
```

2. In the `tests` module, after `the_search_option_rows_are_listed_without_shortcuts`, add:

```rust
    #[test]
    fn replace_in_notes_is_listed_with_its_shortcut() {
        // Break caught: the palette never offering 3b's replace, or its row showing Ctrl+H, the
        // find bar's Replace.
        assert_eq!(labels("replace in notes")[0], "Search: Replace in notes");
        assert_eq!(
            shortcut_text(CommandId::ReplaceInNotes).as_deref(),
            Some("Ctrl+Shift+H")
        );
        assert_eq!(shortcut_text(CommandId::Replace).as_deref(), Some("Ctrl+H"));
    }
```

`every_command_except_tab_positions_and_the_palette_is_listed_once` needs no change: it walks 100..200 and finds the new command listed once.

Run: `cargo test --lib -- window::menus window::command_palette --test-threads=1`
Expected: all pass.

- [ ] **Step 5: Write the failing window tests** in `src/window/main_window.rs`'s `tests` module

1. In `with_notes_mode_off_ctrl_shift_f_and_the_search_toggles_do_nothing`, add `CommandId::ReplaceInNotes,` to the `for command in [...]` list, after `CommandId::SearchToggleRegex,`.

2. After `ctrl_shift_f_takes_a_single_line_selection_and_ignores_a_multi_line_one`, add:

```rust
    #[test]
    fn ctrl_shift_h_shows_search_with_the_replace_field_and_takes_a_single_line_selection() {
        // Break caught: Ctrl+Shift+H dead in the running app, Search shown without the replace
        // field, the selection Ctrl+Shift+F takes ignored, or Ctrl+Shift+F closing the field
        // again (spec §11: it leaves the field as it is).
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
            GetKeyboardState, SetKeyboardState, VK_CONTROL, VK_SHIFT,
        };
        use windows_sys::Win32::UI::WindowsAndMessaging::{MSG, WM_KEYDOWN};
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("replace-shortcut");
        scratch.note("a.md", "alpha beta");
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        scratch.install(window.hwnd);
        let identity = unsafe { super::window_identity(window.hwnd).unwrap() };
        editor.populate_clean("alpha beta\r\ngamma").unwrap();
        editor.set_selection(6..10).unwrap();
        assert!(!crate::window::search_view::replace_open(window.hwnd));

        let mut keys = [0u8; 256];
        unsafe { GetKeyboardState(keys.as_mut_ptr()) };
        let original = keys;
        keys[VK_CONTROL as usize] = 0x80;
        keys[VK_SHIFT as usize] = 0x80;
        unsafe { SetKeyboardState(keys.as_ptr()) };
        let message = MSG {
            hwnd: editor.hwnd(),
            message: WM_KEYDOWN,
            wParam: usize::from(b'H'),
            ..Default::default()
        };
        let translated = unsafe { super::translate_accelerator(window.hwnd, &identity, &message) };
        unsafe { SetKeyboardState(original.as_ptr()) };

        assert!(translated, "Ctrl+Shift+H was not translated");
        assert_eq!(
            crate::window::side_panel::current_view(window.hwnd),
            crate::config::SidebarView::Search
        );
        assert!(crate::window::search_view::replace_open(window.hwnd));
        assert_eq!(
            crate::window::search_view::current_query(window.hwnd)
                .map(|(query, _)| query)
                .as_deref(),
            Some("beta")
        );
        // The prefill searches at once, as Ctrl+Shift+F's does.
        pump_until(window.hwnd, || {
            crate::window::search_view::shown_results(window.hwnd).len() == 1
        });

        execute_command(window.hwnd, CommandId::ShowSearchView);
        assert!(
            crate::window::search_view::replace_open(window.hwnd),
            "Ctrl+Shift+F leaves the field open"
        );
    }
```

Run: `cargo test --lib -- ctrl_shift_h_shows_search with_notes_mode_off_ctrl_shift_f --test-threads=1`
Expected: compile error ``cannot find function `replace_open` in module `crate::window::search_view` ``.

- [ ] **Step 6: Add `show_replace`** in `src/window/search_view.rs`, and dispatch to it

1. In `struct SearchView`, after the `options` field (`pub(crate) options: MatchOptions,`), add:

```rust
    /// The replace field is open (spec §11). Ctrl+Shift+H opens it, the chevron opens and closes
    /// it, and Ctrl+Shift+F leaves it as it is.
    replace_open: bool,
```

2. In `SearchView::new`, after `options: MatchOptions::default(),`, add:

```rust
            replace_open: false,
```

3. After `show_with_query`, add:

```rust
/// Ctrl+Shift+H (spec §11): shows Search with the replace field open. A one-line selection in the
/// active editor fills the search box and searches at once, as Ctrl+Shift+F's does
/// (`show_with_query` escapes it while regex is on).
pub(crate) fn show_replace(hwnd: HWND) {
    // Read before the view takes the focus.
    let prefill = super::main_window::single_line_selection(hwnd);
    side_panel::show_view(hwnd, SidebarView::Search, true);
    if let Some(text) = prefill {
        show_with_query(hwnd, &text);
    }
    let opened = with_view(hwnd, |view| {
        (!std::mem::replace(&mut view.replace_open, true)).then_some(view.panel)
    })
    .flatten();
    if let Some(panel) = opened {
        invalidate(panel);
    }
}
```

4. After the `#[cfg(test)] pub(crate) fn edit_hwnd` accessor, add:

```rust
/// Whether the replace field is open.
#[cfg(test)]
pub(crate) fn replace_open(hwnd: HWND) -> bool {
    with_view(hwnd, |view| view.replace_open).unwrap_or(false)
}
```

5. In `src/window/main_window.rs`, change `fn single_line_selection(hwnd: HWND) -> Option<String> {` to `pub(crate) fn single_line_selection(hwnd: HWND) -> Option<String> {`.

6. In `execute_command_with_note`'s `match command`, after `CommandId::ShowSearchView => show_search_view(hwnd),`, add:

```rust
        CommandId::ReplaceInNotes => crate::window::search_view::show_replace(hwnd),
```

Run: `cargo test --lib -- ctrl_shift_h_shows_search with_notes_mode_off_ctrl_shift_f ctrl_shift_f_takes --test-threads=1`
Expected: all three pass.

- [ ] **Step 7: Lint, format and commit**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: no warnings.

Run: `cargo fmt --all`, then `cargo fmt --all -- --check`
Expected: the check prints nothing.

```bash
git add src/window/commands.rs src/window/menus.rs src/window/command_palette.rs src/window/main_window.rs src/window/search_view.rs
git commit -m "feat(search): Ctrl+Shift+H (ReplaceInNotes, 190) shows the Search view with its replace field open and takes a one-line selection as Ctrl+Shift+F does; the palette lists \"Search: Replace in notes\""
```

---

### Task 5: The Search view's replace UI (chevron, replace field, Replace all, the row button)

**Files:**
- Modify `src/window/search_view.rs`:
  - constants, a new `HeaderButton` enum, and a new `button_color` helper;
  - the `SearchView` fields and `SearchView::new`;
  - `SearchView::{field_rect, list_area, summary_rect, tooltip_tools, paint, draw_row}`;
  - new `SearchView::{chevron_rect, replace_field_rect, replace_all_rect, row_replace_rect, head_bottom, replace_all_enabled, row_button_shown, row_button_at, header_button_at}`;
  - `layout` (with a new `place_field`), `hidden`, `header_hit`, `ensure_edit`, `create_edit`, `handle`, `paint_placeholder` and `search_edit_proc`;
  - new `edit_changed`, `replace_changed`, `replace_text`, `replace_all_enabled`, `toggle_replace`, `set_replace_open`, `ensure_replace_edit`, `replace_field_pressed` and `key_down`;
  - `show_replace`'s body (Task 4);
  - `AccessibleView::{accessible_item, accessible_hit}`, which only follow `summary_rect`'s new signature;
  - the `tests` module, and a `#[cfg(test)] replace_edit_hwnd` accessor.
- Modify `src/window/side_panel.rs`: the panel's `WM_COMMAND`/`EN_CHANGE` arm calls `search_view::edit_changed`.
- Modify `src/window/main_window.rs` (tests only): a new `type_into_replace` helper and two tests.
- `src/window/option_toggles.rs` is unchanged: the chevron and the replace buttons are drawn with `side_panel::draw_text` in the MDL2 glyph font, as the rows' icons are.

**Interfaces:**
- Consumes:
  - `SearchView.replace_open` and `show_replace` (Task 4).
  - `main_window::single_line_selection` (Task 4).
  - `side_panel::{show_view, with_accessible_events}` and `sidebar_accessibility::row_rect` (3a).
- Produces, by the contract:
  - Fields: `SearchView.replace_edit: Option<HWND>`, `SearchView.replace_text: String` (kept at `EN_CHANGE`), and `SearchView.row_hover_button: Option<usize>`.
  - Functions:
    - `pub(crate) fn show_replace(hwnd: HWND)`;
    - `pub(crate) fn toggle_replace(hwnd: HWND)`;
    - `pub(crate) fn replace_text(hwnd: HWND) -> String`;
    - `pub(crate) fn replace_all_enabled(hwnd: HWND) -> bool`.
  - Associated functions: `SearchView::chevron_rect(client: RECT, dpi: u32) -> RECT`, `replace_field_rect(client: RECT, dpi: u32) -> RECT`, `replace_all_rect(client: RECT, dpi: u32) -> RECT` and `row_replace_rect(row: RECT, dpi: u32) -> RECT`.
- Produces, added here:
  - `pub(crate) fn edit_changed(hwnd: HWND, edit: HWND)`, which routes `EN_CHANGE` from either field.
  - `SearchView::replace_all_enabled(&self) -> bool`, the rule `replace_all_enabled(hwnd)` reads.
  - `SearchView::summary_rect` becomes `pub(crate) fn summary_rect(&self, client: RECT, dpi: u32) -> RECT`, since the summary moves down while the replace row shows.
  - `#[cfg(test)] pub(crate) fn replace_edit_hwnd(hwnd: HWND) -> Option<HWND>`.
- **Left for Task 6.** Task 6 owns `text_search_host::replace_all` and `replace_in`, so this task hit-tests three inputs and swallows them. Task 6 Step 4 (items 6 and 7) adds the calls.
  - A press on Replace all.
  - A press on a row's replace button.
  - Ctrl+Alt+Enter in either field.
  - `replace_text(hwnd)` and `replace_all_enabled(hwnd)` have no reader until Task 6, so they carry `expect(dead_code)` attributes, which Task 6 Step 4 removes (items 2 and 3).

**Layout** at 96 DPI, scaled with `panel::scale`:
- The chevron is 18 px wide, 4 px in from the left, as tall as the search field.
- The search field now starts 2 px after the chevron, at 24 px, where it used to start at 8 px. Its right edge is unchanged.
- While the replace field is open, a 34 px replace row comes under the 38 px header. It holds the replace field, which is as tall as the search field and has the same left edge. Replace all, 26 px wide, sits 6 px to its right and ends at the search field's right edge.
- The summary line, the list and the notice move down by the replace row's height.
- A row's replace button is a 22 px square, vertically centered, 12 px in from the row's right edge. It shows on the hovered row and the selected row while the replace field is open. Both text lines of that row stop 6 px short of it.
- Glyphs (Segoe MDL2 Assets):
  - the chevron: `E76C` (ChevronRight) while the replace field is closed, `E70D` (ChevronDown) while it is open;
  - Replace all and the row button: both `E8AB` (Switch); their tooltips tell them apart.
- Colours:
  - a disabled button is drawn in `line_number_foreground`;
  - a hovered, enabled button gets the `hover_background` fill.

- [ ] **Step 1: Write the failing unit tests** in `src/window/search_view.rs`'s `tests` module

1. Change the `use super::{...}` list at the top of the module to:

```rust
    use super::{
        LOADING, NO_MATCH, NO_NOTEBOOK, PADDING_AT_96_DPI, REPLACE_ROW_AT_96_DPI, ROW_AT_96_DPI,
        ROW_INSET_AT_96_DPI, ROW_LINE_AT_96_DPI, SearchState, SearchView, TOO_SHORT, fit_before,
        notice_text, placeholder, skipped_tooltip, status_text, summary_text,
    };
```

and after `use std::path::{Path, PathBuf};` add:

```rust
    use windows_sys::Win32::Foundation::RECT;
```

2. At the end of the module, add:

```rust
    #[test]
    fn the_replace_row_sits_under_the_box_with_replace_all_at_its_right() {
        // Break caught: the chevron drawn over the box, the replace field overlapping the search
        // field or its button, or the summary and results left under the replace row.
        for dpi in [96, 144, 192] {
            let client = RECT {
                left: 0,
                top: 0,
                right: 320,
                bottom: 600,
            };
            let chevron = SearchView::chevron_rect(client, dpi);
            let field = SearchView::field_rect(client, dpi);
            assert!(chevron.left > client.left, "{dpi}");
            assert!(chevron.right < field.left, "{dpi}");
            assert_eq!((chevron.top, chevron.bottom), (field.top, field.bottom));

            let replace = SearchView::replace_field_rect(client, dpi);
            let all = SearchView::replace_all_rect(client, dpi);
            assert_eq!(replace.left, field.left);
            assert!(replace.top > field.bottom, "{dpi}");
            assert_eq!(replace.bottom - replace.top, field.bottom - field.top);
            assert!(replace.right < all.left, "{dpi}");
            assert_eq!(all.right, field.right);
            assert_eq!((all.top, all.bottom), (replace.top, replace.bottom));

            let mut view = SearchView::new(std::ptr::null_mut(), dpi);
            let closed = view.list_area(client, dpi);
            assert!(view.summary_rect(client, dpi).top >= field.bottom, "{dpi}");
            view.replace_open = true;
            let open = view.list_area(client, dpi);
            assert_eq!(open.top - closed.top, scale(REPLACE_ROW_AT_96_DPI, dpi));
            assert!(view.summary_rect(client, dpi).top >= replace.bottom, "{dpi}");
        }
    }

    #[test]
    fn replace_all_waits_for_a_finished_search_with_results() {
        // Break caught: Replace all pressable while results stream in (the list and its stamps
        // aren't final), after a pattern error, or with nothing listed (spec §11).
        let mut view = SearchView::new(std::ptr::null_mut(), 96);
        view.notebook = Some(PathBuf::from(r"C:\notes"));
        view.loaded = true;
        assert!(!view.replace_all_enabled(), "idle");
        view.begin("needle", 10);
        view.apply(batch(vec![hit("a", "")], 4, None), HEIGHT);
        assert!(!view.replace_all_enabled(), "running");
        view.apply(batch(Vec::new(), 10, Some(RunEnd::Completed)), HEIGHT);
        assert!(view.replace_all_enabled());
        view.search = SearchState::PatternError("Unclosed group".to_owned());
        assert!(!view.replace_all_enabled(), "a pattern error");
        view.begin("zzz", 10);
        view.apply(batch(Vec::new(), 10, Some(RunEnd::Completed)), HEIGHT);
        assert!(!view.replace_all_enabled(), "nothing listed");
    }

    #[test]
    fn a_row_shows_its_replace_button_while_hovered_or_selected_and_the_field_is_open() {
        // Break caught: a replace button on every row (a stray click replaces in the wrong
        // note), none on the selected row for keyboard users, or buttons with the field closed.
        let mut view = SearchView::new(std::ptr::null_mut(), 96);
        view.begin("needle", 10);
        view.apply(
            batch(
                vec![hit("a", ""), hit("b", ""), hit("c", "")],
                10,
                Some(RunEnd::Completed),
            ),
            HEIGHT,
        );
        view.list.select(0, HEIGHT);
        view.list.set_hover(Some(2));
        assert!((0..3).all(|index| !view.row_button_shown(index)));
        view.replace_open = true;
        assert!(view.row_button_shown(0), "selected");
        assert!(view.row_button_shown(2), "hovered");
        assert!(!view.row_button_shown(1));

        let row = RECT {
            left: 0,
            top: 84,
            right: 300,
            bottom: 126,
        };
        let button = SearchView::row_replace_rect(row, 96);
        assert_eq!(button.right, 300 - scale(PADDING_AT_96_DPI, 96));
        assert!(button.left > row.left);
        assert!(button.top > row.top && button.bottom < row.bottom);
        assert_eq!(button.bottom - button.top, button.right - button.left);
    }
```

Run: `cargo test --lib -- window::search_view --test-threads=1`
Expected: compile errors: `REPLACE_ROW_AT_96_DPI` not found in `super`, and no function `chevron_rect` or method `replace_all_enabled` on `SearchView`.

- [ ] **Step 2: Add the constants, the new state and the rectangles** in `src/window/search_view.rs`

1. Change the `KeyboardAndMouse` import to:

```rust
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetFocus, GetKeyState, ReleaseCapture, SetCapture, SetFocus, TME_LEAVE, TRACKMOUSEEVENT,
    TrackMouseEvent, VIRTUAL_KEY, VK_CONTROL, VK_DOWN, VK_ESCAPE, VK_MENU, VK_NEXT, VK_RETURN,
    VK_UP,
};
```

and add `WM_SYSKEYDOWN` to the `WindowsAndMessaging` import list (after `WM_SETTEXT`).

2. After `const TOOLTIP_WIDTH_AT_96_DPI: i32 = 300;`, add:

```rust
/// The chevron left of the search box that opens and closes the replace field (spec §11).
const CHEVRON_LEFT_AT_96_DPI: i32 = 4;
const CHEVRON_WIDTH_AT_96_DPI: i32 = 18;
const CHEVRON_GAP_AT_96_DPI: i32 = 2;
/// The replace field's row, under the header while the field is open.
const REPLACE_ROW_AT_96_DPI: i32 = 34;
/// Replace all, at the right of the replace field.
const REPLACE_ALL_WIDTH_AT_96_DPI: i32 = 26;
/// A result's replace button: a square at the row's right end.
const ROW_BUTTON_AT_96_DPI: i32 = 22;
/// Segoe MDL2 Assets: ChevronRight, ChevronDown, and Switch for both replace buttons.
const CHEVRON_CLOSED_GLYPH: &str = "\u{E76C}";
const CHEVRON_OPEN_GLYPH: &str = "\u{E70D}";
const REPLACE_GLYPH: &str = "\u{E8AB}";
const REPLACE_PLACEHOLDER: &str = "Replace";
const REPLACE_HOOK_ID: usize = 0x4650_5352;
/// The header buttons' and the row button's tooltips, after `STATUS_TOOL`.
const CHEVRON_TOOL: usize = 4;
const REPLACE_ALL_TOOL: usize = 5;
const ROW_REPLACE_TOOL: usize = 6;

/// A painted button in the Search view's header.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HeaderButton {
    Chevron,
    ReplaceAll,
}

/// A painted button's glyph color: dim while it can't run, the hover color under the pointer.
fn button_color(enabled: bool, hover: bool, normal: u32, palette: &Palette) -> u32 {
    if !enabled {
        palette.line_number_foreground
    } else if hover {
        palette.hover_foreground
    } else {
        normal
    }
}

fn key_down(key: VIRTUAL_KEY) -> bool {
    (unsafe { GetKeyState(i32::from(key)) }) < 0
}
```

3. In `struct SearchView`, after the `replace_open` field (Task 4), add:

```rust
    /// The replace field, made the first time it opens.
    replace_edit: Option<HWND>,
    /// Making the replace field failed and was reported; it is not tried again.
    replace_edit_failed: bool,
    /// The replace field's text, kept at each `EN_CHANGE` (`replace_changed`), so the replace and
    /// screen readers read it without a `WM_GETTEXT` under the App borrow.
    replace_text: String,
    /// The result whose replace button is under the pointer.
    row_hover_button: Option<usize>,
    /// The header button under the pointer.
    header_hover: Option<HeaderButton>,
```

4. In `SearchView::new`, after `replace_open: false,`, add:

```rust
            replace_edit: None,
            replace_edit_failed: false,
            replace_text: String::new(),
            row_hover_button: None,
            header_hover: None,
```

5. Replace `SearchView::field_rect`, `list_area` and `summary_rect` with:

```rust
    /// The painted search field, border included, right of the chevron.
    pub(crate) fn field_rect(client: RECT, dpi: u32) -> RECT {
        let margin = scale(FIELD_MARGIN_AT_96_DPI, dpi);
        let height = scale(FIELD_HEIGHT_AT_96_DPI, dpi);
        let top = client.top + (scale(HEADER_AT_96_DPI, dpi) - height) / 2;
        let left = client.left
            + scale(CHEVRON_LEFT_AT_96_DPI, dpi)
            + scale(CHEVRON_WIDTH_AT_96_DPI, dpi)
            + scale(CHEVRON_GAP_AT_96_DPI, dpi);
        RECT {
            left,
            top,
            right: (client.right - margin).max(left),
            bottom: top + height,
        }
    }

    /// The chevron that opens and closes the replace field, left of the search field and as tall.
    pub(crate) fn chevron_rect(client: RECT, dpi: u32) -> RECT {
        let field = Self::field_rect(client, dpi);
        let left = client.left + scale(CHEVRON_LEFT_AT_96_DPI, dpi);
        RECT {
            left,
            top: field.top,
            right: left + scale(CHEVRON_WIDTH_AT_96_DPI, dpi),
            bottom: field.bottom,
        }
    }

    /// Replace all, in the replace row: as tall as the search field, ending at its right edge.
    pub(crate) fn replace_all_rect(client: RECT, dpi: u32) -> RECT {
        let field = Self::field_rect(client, dpi);
        let height = field.bottom - field.top;
        let top = client.top
            + scale(HEADER_AT_96_DPI, dpi)
            + (scale(REPLACE_ROW_AT_96_DPI, dpi) - height) / 2;
        RECT {
            left: (field.right - scale(REPLACE_ALL_WIDTH_AT_96_DPI, dpi)).max(field.left),
            top,
            right: field.right,
            bottom: top + height,
        }
    }

    /// The painted replace field, border included: under the search field, short of Replace all.
    pub(crate) fn replace_field_rect(client: RECT, dpi: u32) -> RECT {
        let field = Self::field_rect(client, dpi);
        let all = Self::replace_all_rect(client, dpi);
        RECT {
            left: field.left,
            top: all.top,
            right: (all.left - scale(GAP_AT_96_DPI, dpi)).max(field.left),
            bottom: all.bottom,
        }
    }

    /// A result's replace button in `row` (the whole row's rectangle): a square, vertically
    /// centered, in from the right edge by the rows' padding, clear of the scroll thumb.
    pub(crate) fn row_replace_rect(row: RECT, dpi: u32) -> RECT {
        let size = scale(ROW_BUTTON_AT_96_DPI, dpi);
        let right = (row.right - scale(PADDING_AT_96_DPI, dpi)).max(row.left);
        let top = row.top + (row.bottom - row.top - size) / 2;
        RECT {
            left: (right - size).max(row.left),
            top,
            right,
            bottom: top + size,
        }
    }

    /// Where the header ends, and the replace row under it while the replace field is open.
    fn head_bottom(&self, client: RECT, dpi: u32) -> i32 {
        let replace = if self.replace_open {
            scale(REPLACE_ROW_AT_96_DPI, dpi)
        } else {
            0
        };
        (client.top + scale(HEADER_AT_96_DPI, dpi) + replace).min(client.bottom)
    }

    /// Where the results are: under the header, the replace row while it shows, and the summary
    /// line, above the status line when it shows.
    pub(crate) fn list_area(&self, client: RECT, dpi: u32) -> RECT {
        let top = (self.head_bottom(client, dpi) + scale(LINE_AT_96_DPI, dpi)).min(client.bottom);
        let bottom = if self.status_line().is_some() {
            (client.bottom - scale(LINE_AT_96_DPI, dpi)).max(top)
        } else {
            client.bottom
        };
        RECT {
            top,
            bottom,
            ..client
        }
    }

    /// The summary line under the header (and the replace row), where the notice shows too.
    pub(crate) fn summary_rect(&self, client: RECT, dpi: u32) -> RECT {
        let pad = scale(PADDING_AT_96_DPI, dpi);
        let top = self.head_bottom(client, dpi);
        RECT {
            left: client.left + pad,
            top,
            right: client.right - pad,
            bottom: (top + scale(LINE_AT_96_DPI, dpi)).min(client.bottom),
        }
    }

    /// Whether Replace all and the rows' replace buttons can run: a search finished with results
    /// (spec §11), and no notice shows in their place.
    pub(crate) fn replace_all_enabled(&self) -> bool {
        matches!(self.search, SearchState::Done { .. })
            && !self.results.is_empty()
            && self.notice().is_none()
    }

    /// Whether row `index` shows its replace button: the hovered and the selected row, while the
    /// replace field is open.
    fn row_button_shown(&self, index: usize) -> bool {
        self.replace_open && (self.list.hover == Some(index) || self.list.selected == Some(index))
    }

    /// The row whose replace button is under `point`, while that button shows.
    fn row_button_at(&self, point: POINT, client: RECT, dpi: u32) -> Option<usize> {
        let index = self.row_under(point, client, dpi)?;
        if !self.row_button_shown(index) {
            return None;
        }
        let (row, _) =
            sidebar_accessibility::row_rect(self.list_area(client, dpi), &self.list, index);
        inside(Self::row_replace_rect(row, dpi), point).then_some(index)
    }

    /// The header button under `point`: the chevron while the box shows, Replace all while the
    /// replace field is open.
    fn header_button_at(&self, point: POINT, client: RECT, dpi: u32) -> Option<HeaderButton> {
        self.edit?;
        if inside(Self::chevron_rect(client, dpi), point) {
            Some(HeaderButton::Chevron)
        } else if self.replace_open && inside(Self::replace_all_rect(client, dpi), point) {
            Some(HeaderButton::ReplaceAll)
        } else {
            None
        }
    }
```

6. Three callers of the old `SearchView::summary_rect(client, dpi)` become `self.summary_rect(client, dpi)`. They are `paint` (replaced in Step 3), `accessible_item`'s `SearchChild::Summary` arm, and `accessible_hit`'s summary test. In the last two, change `SearchView::summary_rect(client, dpi)` to `self.summary_rect(client, dpi)`.

Run: `cargo test --lib -- window::search_view --test-threads=1`
Expected: all pass, including the three new tests.

- [ ] **Step 3: Paint the chevron, the replace row and the row button** in `src/window/search_view.rs`

1. Replace `SearchView::paint` with:

```rust
    pub(crate) fn paint(&self, paint: &ViewPaint) {
        let dpi = paint.dpi;
        let palette = paint.palette;
        let client = paint.client;
        let line = DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX | DT_LEFT | DT_END_ELLIPSIS;
        let glyph = DT_SINGLELINE | DT_VCENTER | DT_CENTER | DT_NOPREFIX;
        unsafe {
            fill(paint.hdc, client, paint.background);
            if self.edit.is_some() {
                let field = Self::field_rect(client, dpi);
                fill(paint.hdc, field, palette.selection_background);
                fill(paint.hdc, inset(field, 1), palette.editor_background);
                option_toggles::paint(
                    paint.hdc,
                    &option_toggles::toggle_rects(field, dpi),
                    self.options,
                    self.toggle_hover,
                    &palette,
                    paint.fonts.text,
                );
                let chevron = Self::chevron_rect(client, dpi);
                let hover = self.header_hover == Some(HeaderButton::Chevron);
                if hover {
                    fill(paint.hdc, chevron, palette.hover_background);
                }
                draw_text(
                    paint.hdc,
                    if self.replace_open {
                        CHEVRON_OPEN_GLYPH
                    } else {
                        CHEVRON_CLOSED_GLYPH
                    },
                    chevron,
                    paint.fonts.glyph,
                    button_color(true, hover, palette.muted_foreground, &palette),
                    glyph,
                );
                if self.replace_open {
                    let replace = Self::replace_field_rect(client, dpi);
                    fill(paint.hdc, replace, palette.selection_background);
                    fill(paint.hdc, inset(replace, 1), palette.editor_background);
                    let all = Self::replace_all_rect(client, dpi);
                    let enabled = self.replace_all_enabled();
                    let hover = enabled && self.header_hover == Some(HeaderButton::ReplaceAll);
                    if hover {
                        fill(paint.hdc, all, palette.hover_background);
                    }
                    draw_text(
                        paint.hdc,
                        REPLACE_GLYPH,
                        all,
                        paint.fonts.glyph,
                        button_color(enabled, hover, palette.editor_foreground, &palette),
                        glyph,
                    );
                }
            }
            let summary = self.summary_rect(client, dpi);
            if let Some(notice) = self.notice() {
                draw_text(
                    paint.hdc,
                    notice,
                    summary,
                    paint.fonts.text,
                    palette.muted_foreground,
                    line,
                );
                return;
            }
            if let Some((text, error)) = self.summary() {
                let color = if error {
                    palette.error_foreground
                } else {
                    palette.muted_foreground
                };
                draw_text(paint.hdc, &text, summary, paint.fonts.text, color, line);
            }
            if let Some(status) = self.status_line() {
                draw_text(
                    paint.hdc,
                    &status,
                    Self::status_rect(client, dpi),
                    paint.fonts.text,
                    palette.muted_foreground,
                    line,
                );
            }
        }
        let area = self.list_area(client, dpi);
        row_list::paint(
            paint.hdc,
            area,
            &self.list,
            &palette,
            paint.focused,
            &mut |hdc, index, rect, look| self.draw_row(hdc, index, rect, look, paint),
        );
    }
```

2. Replace `SearchView::draw_row` with:

```rust
    /// One result on two lines: the file icon, the name and its folder in dim text, then the
    /// snippet with its match in bold. The hovered and the selected row show their replace
    /// button at the right end while the replace field is open; both lines stop short of it.
    fn draw_row(&self, hdc: HDC, index: usize, rect: RECT, look: RowLook, paint: &ViewPaint) {
        let Some(result) = self.results.get(index) else {
            return;
        };
        let dpi = paint.dpi;
        let palette = paint.palette;
        let pad = scale(PADDING_AT_96_DPI, dpi);
        let slot = scale(ROW_LINE_AT_96_DPI, dpi);
        let line = DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX;
        let first = RECT {
            top: rect.top + scale(ROW_INSET_AT_96_DPI, dpi),
            bottom: rect.top + scale(ROW_INSET_AT_96_DPI, dpi) + slot,
            ..rect
        };
        let glyph = RECT {
            left: rect.left + pad,
            right: rect.left + pad + scale(GLYPH_AT_96_DPI, dpi),
            ..first
        };
        let foreground = row_foreground(look, &palette);
        // Over the focused selection, dim text takes the selection's text color to stay legible.
        let muted = if look.selected && look.focused {
            foreground
        } else {
            palette.muted_foreground
        };
        let button = self
            .row_button_shown(index)
            .then(|| Self::row_replace_rect(rect, dpi));
        let right = button.map_or(rect.right - pad, |button| {
            button.left - scale(GAP_AT_96_DPI, dpi)
        });
        let text = RECT {
            left: glyph.right + scale(GAP_AT_96_DPI, dpi),
            right,
            ..first
        };
        let second = RECT {
            top: first.bottom,
            bottom: first.bottom + slot,
            ..text
        };
        unsafe {
            draw_text(
                hdc,
                DOCUMENT_GLYPH,
                glyph,
                paint.fonts.glyph,
                muted,
                line | DT_CENTER,
            );
            let width = draw_text(
                hdc,
                &result.name,
                text,
                paint.fonts.text,
                foreground,
                line | DT_LEFT | DT_END_ELLIPSIS,
            );
            if !result.folder.is_empty() {
                draw_text(
                    hdc,
                    &result.folder,
                    RECT {
                        left: text.left + width + scale(GAP_AT_96_DPI, dpi),
                        ..text
                    },
                    paint.fonts.text,
                    muted,
                    line | DT_LEFT | DT_END_ELLIPSIS,
                );
            }
            draw_snippet(hdc, &result.snippet, second, paint.fonts, foreground);
            if let Some(button) = button {
                let enabled = self.replace_all_enabled();
                let hover = enabled && self.row_hover_button == Some(index);
                if hover {
                    fill(hdc, button, palette.hover_background);
                }
                draw_text(
                    hdc,
                    REPLACE_GLYPH,
                    button,
                    paint.fonts.glyph,
                    button_color(enabled, hover, foreground, &palette),
                    line | DT_CENTER,
                );
            }
        }
    }
```

3. In `SearchView::tooltip_tools`, before `tools.push((STATUS_TOOL, ...));`, add:

```rust
        let chevron = if self.edit.is_some() {
            "Toggle replace"
        } else {
            ""
        };
        tools.push((
            CHEVRON_TOOL,
            edges(Self::chevron_rect(client, dpi)),
            chevron.to_owned(),
        ));
        let all = if self.replace_open {
            "Replace all (Ctrl+Alt+Enter)"
        } else {
            ""
        };
        tools.push((
            REPLACE_ALL_TOOL,
            edges(Self::replace_all_rect(client, dpi)),
            all.to_owned(),
        ));
        let row = self
            .row_hover_button
            .filter(|_| self.replace_open)
            .map(|index| {
                let (row, _) =
                    sidebar_accessibility::row_rect(self.list_area(client, dpi), &self.list, index);
                Self::row_replace_rect(row, dpi)
            });
        tools.push((
            ROW_REPLACE_TOOL,
            edges(row.unwrap_or_default()),
            if row.is_some() { "Replace" } else { "" }.to_owned(),
        ));
```

- [ ] **Step 4: Make, place and route the replace field** in `src/window/search_view.rs`

1. Replace `create_edit` with:

```rust
/// A hidden field inside `panel`: the search box (`SEARCH_HOOK_ID`) or the replace field
/// (`REPLACE_HOOK_ID`). The subclass tells them apart by its ID.
fn create_edit(panel: HWND, hook: usize) -> crate::Result<HWND> {
    let edit = create_child(panel, &wide_null("Edit"), WS_CHILD | ES_AUTOHSCROLL as u32)?;
    if unsafe { SetWindowSubclass(edit, Some(search_edit_proc), hook, 0) } == 0 {
        let error = last_error();
        unsafe {
            DestroyWindow(edit);
        }
        return Err(error);
    }
    Ok(edit)
}
```

and in `ensure_edit` change `match create_edit(panel) {` to `match create_edit(panel, SEARCH_HOOK_ID) {`.

2. After `ensure_edit`, add:

```rust
/// The replace field, made now if the view has none yet. A failure is reported once.
fn ensure_replace_edit(hwnd: HWND) -> Option<HWND> {
    let (panel, edit, failed) = with_view(hwnd, |view| {
        (view.panel, view.replace_edit, view.replace_edit_failed)
    })?;
    if edit.is_some() || failed {
        return edit;
    }
    // Made with nothing of the App borrowed: creating the Edit sends messages to the panel.
    match create_edit(panel, REPLACE_HOOK_ID) {
        Ok(edit) => {
            if with_view(hwnd, |view| view.replace_edit = Some(edit)).is_none() {
                unsafe { DestroyWindow(edit) };
                return None;
            }
            Some(edit)
        }
        Err(error) => {
            with_view(hwnd, |view| view.replace_edit_failed = true);
            super::main_window::push_notice(
                hwnd,
                format!("FastPad could not show the replace field: {error}"),
            );
            None
        }
    }
}

/// Opens or closes the replace field, making it the first time it opens, and lays the view out
/// again. Returns the field while it is open; `None` when it is closed or could not be made (then
/// it stays closed). Closing it with the caret inside moves the caret to the search box.
fn set_replace_open(hwnd: HWND, open: bool) -> Option<HWND> {
    let replace = if open {
        Some(ensure_replace_edit(hwnd)?)
    } else {
        with_view(hwnd, |view| view.replace_edit).flatten()
    };
    let (panel, edit) = with_view(hwnd, |view| {
        if view.replace_open != open {
            view.replace_open = open;
            // The replace children come and go, and the rows move down or up.
            view.order = view.order.wrapping_add(1);
            view.row_hover_button = None;
            view.header_hover = None;
        }
        (view.panel, view.edit)
    })?;
    // Focused with nothing of the App borrowed: SetFocus sends focus messages.
    if !open
        && let (Some(replace), Some(edit)) = (replace, edit)
        && unsafe { GetFocus() } == replace
    {
        unsafe {
            SetFocus(edit);
        }
    }
    layout(hwnd);
    invalidate(panel);
    if open { replace } else { None }
}

/// The chevron (spec §11): opens the replace field with the caret in it, or closes it.
pub(crate) fn toggle_replace(hwnd: HWND) {
    let Some(open) = with_view(hwnd, |view| !view.replace_open) else {
        return;
    };
    let replace =
        side_panel::with_accessible_events(hwnd, || set_replace_open(hwnd, open));
    if let Some(replace) = replace {
        unsafe {
            SetFocus(replace);
            SendMessageW(replace, EM_SETSEL, 0, -1);
        }
    }
}

/// `EN_CHANGE` from one of the view's fields (`side_panel` passes the control).
pub(crate) fn edit_changed(hwnd: HWND, edit: HWND) {
    if with_view(hwnd, |view| view.replace_edit == Some(edit)).unwrap_or(false) {
        replace_changed(hwnd, edit);
    } else {
        query_changed(hwnd);
    }
}

/// `EN_CHANGE` from the replace field: keeps its text. No search runs.
fn replace_changed(hwnd: HWND, edit: HWND) {
    // Read with nothing of the App borrowed.
    let text = window_text(edit);
    with_view(hwnd, |view| view.replace_text = text);
}

/// The replace field's text as `EN_CHANGE` last kept it; empty before it is first opened.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "the replace flow reads it from Task 6")
)]
pub(crate) fn replace_text(hwnd: HWND) -> String {
    with_view(hwnd, |view| view.replace_text.clone()).unwrap_or_default()
}

/// Whether Replace all can run now (`SearchView::replace_all_enabled`).
#[expect(dead_code, reason = "the replace flow reads it from Task 6")]
pub(crate) fn replace_all_enabled(hwnd: HWND) -> bool {
    with_view(hwnd, |view| view.replace_all_enabled()).unwrap_or(false)
}
```

3. Replace `layout` with:

```rust
/// Places the box in the header, short of the toggles, and the replace field in its row while it
/// is open, and shows them while the Search view shows, making the box the first time the view
/// shows a notebook. Part of `side_panel::layout`, and run whenever the view, the notebook or the
/// replace field's state changes. It never makes the box without a notebook: at startup that
/// would come before the first paint. `shown` makes it once the user opens the view.
pub(crate) fn layout(hwnd: HWND) {
    let Some(has_notebook) = with_view(hwnd, |view| view.notebook.is_some()) else {
        return;
    };
    let show = side_panel::current_view(hwnd) == SidebarView::Search;
    let edit = if show && has_notebook {
        ensure_edit(hwnd)
    } else {
        with_view(hwnd, |view| view.edit).flatten()
    };
    let Some(edit) = edit else {
        return;
    };
    let panel = unsafe { GetParent(edit) };
    let (client, dpi) = geometry(panel);
    let text_font = super::main_window::ui_fonts(hwnd).text;
    let inset_x = scale(FIELD_TEXT_INSET_AT_96_DPI, dpi);
    place_field(
        edit,
        SearchView::field_rect(client, dpi),
        text_font,
        inset_x,
        option_toggles::reserved_width(dpi),
    );
    let (replace, replace_open) = with_view(hwnd, |view| {
        view.list.row_height = scale(ROW_AT_96_DPI, dpi);
        (view.replace_edit, view.replace_open)
    })
    .unwrap_or((None, false));
    if let Some(replace) = replace {
        place_field(
            replace,
            SearchView::replace_field_rect(client, dpi),
            text_font,
            inset_x,
            inset_x,
        );
    }
    if show {
        unsafe {
            ShowWindow(edit, SW_SHOWNA);
        }
    } else {
        hide_box(edit);
    }
    if let Some(replace) = replace {
        if show && replace_open {
            unsafe {
                ShowWindow(replace, SW_SHOWNA);
            }
        } else {
            hide_box(replace);
        }
    }
}

/// Gives `edit` the sidebar's text font and centers it vertically in the painted `field`, `inset`
/// in from its left edge and `reserved` short of its right edge.
fn place_field(edit: HWND, field: RECT, font: HFONT, inset: i32, reserved: i32) {
    unsafe {
        if !font.is_null() {
            SendMessageW(edit, WM_SETFONT, font as WPARAM, 0);
        }
    }
    let text = text_height(edit, font).clamp(1, (field.bottom - field.top - 2).max(1));
    let top = field.top + (field.bottom - field.top - text) / 2;
    let width = (field.right - field.left - inset - reserved).max(0);
    unsafe {
        MoveWindow(edit, field.left + inset, top, width, text, 1);
    }
}
```

4. Replace `hidden` with:

```rust
/// `side_panel::show_view` switched away from Search. The query and the replace text stay. The
/// tooltips go, or they would show over the other view.
pub(crate) fn hidden(hwnd: HWND) {
    let Some((edit, replace, tooltip, tools)) = with_view(hwnd, |view| {
        (
            view.edit,
            view.replace_edit,
            view.tooltip,
            std::mem::take(&mut view.tools_shown),
        )
    }) else {
        return;
    };
    if let Some(tooltip) = tooltip {
        for (id, _, _) in tools {
            tooltip.set_tool(id, RECT::default(), "");
        }
    }
    if let Some(replace) = replace {
        hide_box(replace);
    }
    if let Some(edit) = edit {
        hide_box(edit);
    }
}
```

5. Replace `show_replace` (Task 4's) with:

```rust
/// Ctrl+Shift+H (spec §11): shows Search with the replace field open and the caret in it. A
/// one-line selection in the active editor fills the search box and searches at once, as
/// Ctrl+Shift+F's does (`show_with_query` escapes it while regex is on).
pub(crate) fn show_replace(hwnd: HWND) {
    // Read before the view takes the focus.
    let prefill = super::main_window::single_line_selection(hwnd);
    side_panel::show_view(hwnd, SidebarView::Search, false);
    if let Some(text) = prefill {
        show_with_query(hwnd, &text);
    }
    let replace = side_panel::with_accessible_events(hwnd, || set_replace_open(hwnd, true));
    let edit = with_view(hwnd, |view| view.edit).flatten();
    // Focused with nothing of the App borrowed.
    unsafe {
        match (replace, edit) {
            (Some(replace), _) => {
                SetFocus(replace);
                SendMessageW(replace, EM_SETSEL, 0, -1);
            }
            // No replace field could be made (reported once): the box takes the caret instead.
            (None, Some(edit)) => {
                SetFocus(edit);
            }
            (None, None) => {}
        }
    }
}
```

6. After the `#[cfg(test)] pub(crate) fn replace_open` accessor (Task 4), add:

```rust
#[cfg(test)]
pub(crate) fn replace_edit_hwnd(hwnd: HWND) -> Option<HWND> {
    with_view(hwnd, |view| view.replace_edit).flatten()
}
```

7. In `src/window/side_panel.rs`, replace the arm

```rust
        // The search box's text changed: re-run the search.
        WM_COMMAND if lparam != 0 && ((wparam >> 16) & 0xffff) as u32 == EN_CHANGE => {
            crate::window::search_view::query_changed(main);
            0
        }
```

with

```rust
        // The search box's text changed (the search runs again), or the replace field's (its
        // text is kept).
        WM_COMMAND if lparam != 0 && ((wparam >> 16) & 0xffff) as u32 == EN_CHANGE => {
            crate::window::search_view::edit_changed(main, lparam as HWND);
            0
        }
```

- [ ] **Step 5: Hit-test the new buttons and handle the replace field's keys** in `src/window/search_view.rs`

1. Replace `header_hit` with:

```rust
/// Whether panel point (`x`, `y`) is on the chevron or the painted search field, which are client
/// area, not window caption. Both show once the box exists.
pub(crate) fn header_hit(hwnd: HWND, panel: HWND, x: i32, y: i32) -> bool {
    let (client, dpi) = geometry(panel);
    let point = POINT { x, y };
    with_view(hwnd, |view| view.edit.is_some()).unwrap_or(false)
        && (inside(SearchView::field_rect(client, dpi), point)
            || inside(SearchView::chevron_rect(client, dpi), point))
}
```

2. After `field_pressed`, add:

```rust
/// A press on the replace field's padding, outside the field's Edit, puts the caret in it.
/// Reports whether the press was on the field.
fn replace_field_pressed(hwnd: HWND, panel: HWND, at: POINT) -> bool {
    let (client, dpi) = geometry(panel);
    let Some(replace) = with_view(hwnd, |view| {
        view.replace_edit.filter(|_| view.replace_open)
    })
    .flatten() else {
        return false;
    };
    if !inside(SearchView::replace_field_rect(client, dpi), at) {
        return false;
    }
    // Focused with nothing of the App borrowed.
    unsafe {
        SetFocus(replace);
    }
    true
}
```

3. In `handle`, replace the `WM_MOUSEMOVE` arm's `let changed = with_view(...)` statement with:

```rust
            let changed = with_view(hwnd, |view| {
                if let Some(grab) = view.thumb_grab {
                    let area = view.list_area(client, dpi);
                    return view.list.drag_thumb(grab, at.y - area.top, height(area));
                }
                let toggle = view.toggle_at(at, client, dpi);
                let toggle_changed = std::mem::replace(&mut view.toggle_hover, toggle) != toggle;
                let header = view.header_button_at(at, client, dpi);
                let header_changed = std::mem::replace(&mut view.header_hover, header) != header;
                let hover = view.row_under(at, client, dpi);
                let row_changed = view.list.set_hover(hover);
                // After the hover moved: the button shows on the hovered row.
                let button = view.row_button_at(at, client, dpi);
                let button_changed =
                    std::mem::replace(&mut view.row_hover_button, button) != button;
                toggle_changed || header_changed || row_changed || button_changed
            })
            .unwrap_or(false);
```

4. In `handle`, replace the `WM_MOUSELEAVE` arm's `let changed = with_view(...)` statement with:

```rust
            let changed = with_view(hwnd, |view| {
                let toggle_changed = view.toggle_hover.take().is_some();
                let header_changed = view.header_hover.take().is_some();
                let button_changed = view.row_hover_button.take().is_some();
                let row_changed = view.list.set_hover(None);
                toggle_changed || header_changed || button_changed || row_changed
            })
            .unwrap_or(false);
```

5. In `handle`'s `WM_LBUTTONDOWN | WM_LBUTTONDBLCLK` arm, replace

```rust
            if field_pressed(hwnd, panel, at) {
                return Some(0);
            }
```

with

```rust
            // A double click on a button is only its first press again.
            let pressed = message == WM_LBUTTONDOWN;
            match with_view(hwnd, |view| view.header_button_at(at, client, dpi)).flatten() {
                Some(HeaderButton::Chevron) => {
                    if pressed {
                        toggle_replace(hwnd);
                    }
                    return Some(0);
                }
                Some(HeaderButton::ReplaceAll) => return Some(0),
                None => {}
            }
            if replace_field_pressed(hwnd, panel, at) || field_pressed(hwnd, panel, at) {
                return Some(0);
            }
            if with_view(hwnd, |view| view.row_button_at(at, client, dpi))
                .flatten()
                .is_some()
            {
                return Some(0);
            }
```

6. Replace `paint_placeholder` with:

```rust
/// A field's placeholder, painted where typed text starts: "Search text in <notebook>" in the
/// box, "Replace" in the replace field.
fn paint_placeholder(hwnd: HWND, edit: HWND, replace: bool) -> bool {
    let Some((text, colors)) = with_view(hwnd, |view| {
        let text = if replace {
            REPLACE_PLACEHOLDER.to_owned()
        } else {
            view.placeholder.clone()
        };
        (text, view.colors)
    }) else {
        return false;
    };
    let mut paint = PAINTSTRUCT::default();
    let dc = unsafe { BeginPaint(edit, &mut paint) };
    if dc.is_null() {
        return true;
    }
    unsafe {
        let mut client = RECT::default();
        GetClientRect(edit, &mut client);
        fill(dc, client, colors.editor_background);
        let font = SendMessageW(edit, WM_GETFONT, 0, 0);
        client.left += (SendMessageW(edit, EM_GETMARGINS, 0, 0) & 0xffff) as i32;
        draw_text(
            dc,
            &text,
            client,
            font as _,
            colors.muted_foreground,
            DT_SINGLELINE | DT_NOPREFIX | DT_END_ELLIPSIS,
        );
        EndPaint(edit, &paint);
    }
    true
}
```

7. Replace `search_edit_proc` with:

```rust
/// The subclass of both fields. `subclass_id` tells them apart: `SEARCH_HOOK_ID` for the search
/// box, `REPLACE_HOOK_ID` for the replace field.
unsafe extern "system" fn search_edit_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    subclass_id: usize,
    _ref_data: usize,
) -> LRESULT {
    if message == WM_NCDESTROY {
        // The last message: drop the hook and let the Edit finish; nothing else is looked up.
        unsafe {
            RemoveWindowSubclass(hwnd, Some(search_edit_proc), subclass_id);
            return DefSubclassProc(hwnd, message, wparam, lparam);
        }
    }
    let replace = subclass_id == REPLACE_HOOK_ID;
    let panel = unsafe { GetParent(hwnd) };
    let main = unsafe { GetParent(panel) };
    // A single-line Edit beeps at Enter and Escape characters; both are handled on key down.
    if message == WM_CHAR && matches!(wparam as u16, 0x0d | 0x1b) {
        return 0;
    }
    // Alt+C, Alt+W and Alt+R flip the toggles before the menu band sees the letter (spec §4).
    if let Some(option) = option_toggles::alt_option(message, wparam, lparam) {
        toggle_option(main, option);
        return 0;
    }
    if option_toggles::is_toggle_char(message, wparam, lparam) {
        return 0;
    }
    if message == WM_PAINT
        && unsafe { GetWindowTextLengthW(hwnd) } == 0
        && paint_placeholder(main, hwnd, replace)
    {
        return 0;
    }
    // Ctrl+Alt+Enter in either field is Replace all (spec §11). With Ctrl held it can come as
    // WM_KEYDOWN or as WM_SYSKEYDOWN; either way it never opens a result.
    if matches!(message, WM_KEYDOWN | WM_SYSKEYDOWN)
        && wparam as u16 == VK_RETURN
        && key_down(VK_CONTROL)
        && key_down(VK_MENU)
    {
        return 0;
    }
    if message == WM_KEYDOWN {
        let ctrl = key_down(VK_CONTROL);
        match wparam as u16 {
            VK_DOWN | VK_NEXT => {
                enter_results(main, panel);
                return 0;
            }
            // Up from the replace field goes back to the search box above it.
            VK_UP if replace => {
                if let Some(edit) = with_view(main, |view| view.edit).flatten() {
                    unsafe {
                        SetFocus(edit);
                    }
                }
                return 0;
            }
            // Enter in the replace field replaces nothing: Replace all is Ctrl+Alt+Enter.
            VK_RETURN if replace => return 0,
            VK_RETURN if ctrl => {
                open_selected(main, OpenMode::Permanent, true);
                return 0;
            }
            VK_RETURN => {
                open_selected(main, OpenMode::Preview, false);
                return 0;
            }
            VK_ESCAPE => {
                // Esc clears the field; in an empty one it returns to the editor (spec §4).
                if unsafe { GetWindowTextLengthW(hwnd) } > 0 {
                    let empty = wide_null("");
                    // WM_SETTEXT comes back through this proc, which repaints the placeholder,
                    // and EN_CHANGE clears the results or the kept replace text.
                    unsafe {
                        SetWindowTextW(hwnd, empty.as_ptr());
                    }
                } else {
                    super::main_window::focus_content(main);
                }
                return 0;
            }
            _ => {}
        }
    }
    // The Edit repaints only the text it changes; the placeholder must go, or come back, whole.
    let edits_text = matches!(
        message,
        WM_CHAR
            | WM_KEYDOWN
            | WM_PASTE
            | WM_CUT
            | WM_CLEAR
            | WM_UNDO
            | WM_SETTEXT
            | EM_UNDO
            | EM_REPLACESEL
    );
    let was_empty = edits_text && unsafe { GetWindowTextLengthW(hwnd) } == 0;
    let result = unsafe { DefSubclassProc(hwnd, message, wparam, lparam) };
    if edits_text && was_empty != (unsafe { GetWindowTextLengthW(hwnd) } == 0) {
        unsafe {
            InvalidateRect(hwnd, std::ptr::null(), 1);
        }
    }
    result
}
```

Run: `cargo test --lib -- window::search_view --test-threads=1`
Expected: all pass.

- [ ] **Step 6: Write the window tests** in `src/window/main_window.rs`'s `tests` module

After `type_into_search`, add:

```rust
    fn type_into_replace(hwnd: HWND, text: &str) {
        let edit = crate::window::search_view::replace_edit_hwnd(hwnd).unwrap();
        let wide = crate::platform::wide_null(text);
        // The Edit sends EN_CHANGE to the panel, which keeps the text; no search runs.
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::SetWindowTextW(edit, wide.as_ptr());
        }
    }
```

After `ctrl_shift_h_shows_search_with_the_replace_field_and_takes_a_single_line_selection` (Task 4), add:

```rust
    #[test]
    fn ctrl_shift_h_focuses_the_replace_field_and_typing_there_runs_no_search() {
        // Break caught: the caret left in the search box, the replace text read by WM_GETTEXT
        // under the App borrow instead of kept, a keystroke in the replace field restarting the
        // search, or Esc and Up in it doing what they do in the box.
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetFocus, VK_ESCAPE, VK_UP};
        use windows_sys::Win32::UI::WindowsAndMessaging::WM_KEYDOWN;
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("replace-field");
        scratch.note("a.md", "alpha needle");
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        scratch.install(window.hwnd);

        execute_command(window.hwnd, CommandId::ReplaceInNotes);
        let replace = crate::window::search_view::replace_edit_hwnd(window.hwnd).unwrap();
        let search_box = crate::window::search_view::edit_hwnd(window.hwnd).unwrap();
        assert!(is_shown(replace));
        assert_eq!(unsafe { GetFocus() }, replace);

        search_for(window.hwnd, "needle");
        let generation = search_generation(window.hwnd);
        type_into_replace(window.hwnd, "pin");
        assert_eq!(crate::window::search_view::replace_text(window.hwnd), "pin");
        pump_past_debounce(window.hwnd);
        assert_eq!(
            search_generation(window.hwnd),
            generation,
            "typing a replacement runs no search"
        );
        assert_eq!(search_rows(window.hwnd).len(), 1);

        unsafe { SendMessageW(replace, WM_KEYDOWN, VK_UP as usize, 0) };
        assert_eq!(unsafe { GetFocus() }, search_box, "Up goes to the box");
        unsafe { SendMessageW(replace, WM_KEYDOWN, VK_ESCAPE as usize, 0) };
        assert_eq!(
            crate::window::search_view::replace_text(window.hwnd),
            "",
            "Esc clears the field"
        );
        unsafe { windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus(replace) };
        unsafe { SendMessageW(replace, WM_KEYDOWN, VK_ESCAPE as usize, 0) };
        assert_eq!(
            unsafe { GetFocus() },
            editor.hwnd(),
            "Esc in the empty field returns to the editor"
        );
    }

    #[test]
    fn the_chevron_opens_and_closes_the_replace_field() {
        // Break caught: a chevron that does nothing, a replace field made before the user asks
        // for it, one left showing (or holding the caret) once closed, or the results kept under
        // the replace row.
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetFocus;
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("replace-chevron");
        scratch.note("a.md", "needle");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        scratch.install(window.hwnd);
        crate::window::side_panel::show_view(window.hwnd, crate::config::SidebarView::Search, true);
        search_for(window.hwnd, "needle");
        assert!(
            crate::window::search_view::replace_edit_hwnd(window.hwnd).is_none(),
            "made the first time it opens"
        );
        let panel = sidebar_panel(window.hwnd);
        let (width, height) = client_size(panel);
        let client = RECT {
            left: 0,
            top: 0,
            right: width,
            bottom: height,
        };
        let dpi = unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForWindow(panel) }.max(96);
        let chevron = crate::window::search_view::SearchView::chevron_rect(client, dpi);
        let (x, y) = (
            (chevron.left + chevron.right) / 2,
            (chevron.top + chevron.bottom) / 2,
        );
        let list_top = || {
            app_mut(window.hwnd)
                .sidebar
                .as_ref()
                .unwrap()
                .search
                .list_area(client, dpi)
                .top
        };
        let closed_top = list_top();

        click(panel, x, y);
        assert!(crate::window::search_view::replace_open(window.hwnd));
        let replace = crate::window::search_view::replace_edit_hwnd(window.hwnd).unwrap();
        assert!(is_shown(replace));
        assert_eq!(unsafe { GetFocus() }, replace);
        assert!(list_top() > closed_top, "the results move under the replace row");

        click(panel, x, y);
        assert!(!crate::window::search_view::replace_open(window.hwnd));
        assert!(!is_shown(replace));
        assert_eq!(
            unsafe { GetFocus() },
            crate::window::search_view::edit_hwnd(window.hwnd).unwrap(),
            "the caret goes back to the search box"
        );
        assert_eq!(list_top(), closed_top);
    }
```

Run: `cargo test --lib -- ctrl_shift_h the_chevron_opens with_notes_mode_off_ctrl_shift_f the_search_view_ --test-threads=1`
Expected: all pass, including 3a's Search view tests. The search field moved right, and nothing in 3a depends on its left edge.

- [ ] **Step 7: Lint, format and commit**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: no warnings. The two `expect(dead_code)` attributes are fulfilled.

Run: `cargo fmt --all`, then `cargo fmt --all -- --check`
Expected: the check prints nothing.

```bash
git add src/window/search_view.rs src/window/side_panel.rs src/window/main_window.rs
git commit -m "feat(search): the Search view gains a chevron that opens a replace field (placeholder \"Replace\", its text kept at EN_CHANGE), a Replace all button at its right and a replace button on the hovered and selected rows; Up, Esc and Enter in the field, and Ctrl+Alt+Enter never opening a result"
```

---

### Task 6: The replace flow (count, confirm, open tabs, write, report, re-run)

**Files:**
- Modify `src/window/text_search_host.rs`:
  - the imports and constants;
  - `TextSearchHost`'s fields;
  - `forget`;
  - new `Candidate`, `ReplacePlan`, `ReplaceCounted`, `ReplaceWritten`, `replace_all`, `replace_in`, `start_replace`, `spawn_count`, `replace_counted`, `replace_timer`, `confirm_and_apply`, `apply_plan`, `spawn_write`, `replace_written`, `report_written`, `finish_replace`, `cancel_replace`, `open_notes`, `target_overlays`, `post_boxed`, `note_name`, `count_of`, `confirm_text`, `row_confirm_text` and `report_text`;
  - test accessors and the `tests` module.
- Modify `src/window/search_view.rs`:
  - `handle` and `search_edit_proc`, where the three inputs Task 5 swallowed now call the flow;
  - `replace_open` (no longer `cfg(test)`);
  - `replace_text` and `replace_all_enabled`, which lose their `expect(dead_code)`;
  - `thousands`, made `pub(crate)`;
  - a new `replace_candidates`.
- Modify `src/window/main_window.rs`:
  - `read_inactive_text`, now a caller of the new `with_inactive_document`;
  - a new `replace_in_document`;
  - the window procedure's `WM_DESTROY`, `WM_TIMER` and `_` arms;
  - the `tests` module.
- Modify `src/window/tabs.rs`: a new `Tabs::note_background_edit` and a test.
- Modify `src/window/messages.rs` and `src/window/mod.rs`: `WM_FASTPAD_REPLACE_COUNTED` and `WM_FASTPAD_REPLACE_WRITTEN`.
- Modify `src/window/status.rs`: `status_text` shows a notification of several lines on the bar's one line, and gains a test.
- `src/window/library_host.rs` is unchanged. The report calls its existing `with_state` with `LibraryState::record_written` (Task 2) for each written note, with the stamp the write worker took, so the UI thread reads no note's metadata.

**Interfaces:**
- Consumes:
  - `Matcher::replacements` (Task 1). It gives byte ranges into the text it is given, and `with_document_text`'s text is Scintilla's buffer, so they are Scintilla positions.
  - `text_replace::{ReplaceTarget, ReplaceCount, count, ReplaceReport, apply}` (Task 2). `ReplaceReport.written` is `Vec<(PathBuf, Stamp)>`, relative to the notebook; its length is the notes replaced into. A cancelled `apply` returns the report so far.
  - `LibraryState::record_written(&Path, Stamp) -> bool` (Task 2), which updates a listed note's size and time with no disk access.
  - `Editor::replace_ranges_with` (Task 3), which applies the edits from the end backwards as one undo action.
  - `search_view::run_query` (3a), `search_view::replace_open` (Task 4), `search_view::{replace_text, replace_all_enabled}` (Task 5), `busy` and `run_now` (3a), `modal::confirm`, and `main_window::{push_notice, document_text}`.
- Produces, by the contract:
  - `pub(crate) fn replace_all(hwnd: HWND)` and `pub(crate) fn replace_in(hwnd: HWND, relative: &Path)`;
  - `pub(crate) fn replace_counted(hwnd: HWND, lparam: LPARAM)` and `pub(crate) fn replace_written(hwnd: HWND, lparam: LPARAM)`;
  - the payloads `ReplaceCounted { generation, count: ReplaceCount, plan }` and `ReplaceWritten { generation, report: ReplaceReport, tab_matches: usize, tab_notes: usize }`;
  - `WM_FASTPAD_REPLACE_COUNTED = WM_APP + 14` and `WM_FASTPAD_REPLACE_WRITTEN = WM_APP + 15`.
- Produces, added here:
  - `pub(crate) struct Candidate { pub path: PathBuf, pub name: String, pub stamp: Option<Stamp> }` and `search_view::replace_candidates(hwnd) -> (Vec<Candidate>, bool)`, where the `bool` is "capped".
  - `pub(crate) const REPLACE_TIMER_ID: usize` and `pub(crate) fn replace_timer(hwnd: HWND)`. A count that arrives inside a modal loop or a file population is held, and this timer retries it.
  - `pub(crate) fn cancel_replace(hwnd: HWND)`, called by `forget` and by `WM_DESTROY`.
  - `pub(crate) fn confirm_text(count: ReplaceCount, listed: usize, capped: bool, any_closed: bool, template: &str) -> String`, `pub(crate) fn row_confirm_text(matches: usize, name: &str, template: &str) -> String` and `pub(crate) fn report_text(matches: usize, notes: usize, changed: &[String], failed: &[String]) -> String`.
  - `main_window::replace_in_document(hwnd: HWND, id: DocumentId, matcher: &Matcher, template: &str) -> Option<usize>` and the private `with_inactive_document`.
  - `Tabs::note_background_edit(&mut self, id: DocumentId) -> bool`.

**The flow**, which is what the tests below pin:
1. **`replace_all` and `replace_in`** do nothing in any of these cases:
   - the field is closed;
   - Replace all is disabled (`replace_all_enabled` is false);
   - a replace is already running (`replacing`);
   - a modal loop or a file population is active (`busy`).
2. **Setting up the count.** Each listed note, or the one row, becomes a `Candidate`.
   - The overlays are the editor texts of **every open target tab**, active or background, clean or dirty (`target_overlays`, read with nothing borrowed). Search's `dirty_overlays` takes only the dirty tabs, but the replace changes an open tab's editor text, so the count must read that text.
   - The count worker gets every candidate as a `ReplaceTarget`. A hit that came from a tab's text has no stamp, so it gets `OVERLAY_STAMP`, which only the count sees.
3. **When the count arrives:**
   - A stale generation is dropped.
   - Inside a modal loop or a population, it is held and retried on `REPLACE_TIMER_ID` every 50 ms.
   - With zero matches, the query simply runs again.
4. **The question.** It is asked with nothing borrowed.
   - Replace all always asks. The "aren't open" line is added when any candidate isn't open now.
   - A per-row replace asks only when its note isn't open.
   - After No, nothing changes.
   - After Yes, a generation that changed during the question drops the plan.
5. **The split, after Yes.** The candidates are split again, so a tab opened or closed during the question counts as it is now:
   - An open tab (active or background, clean or dirty) goes to `replace_in_document`.
   - A closed note with a stamp becomes a write target.
   - A closed note without one (its tab closed since the search) is never written: it is left out of `apply`'s targets and added to the report's `changed`, reported as changed since the search.
6. **The write and the report.**
   - The write worker runs only when there are closed targets. It adds the pre-skipped notes to `report.changed`.
   - `report_written` then does three things:
     - calls `record_written` for each written path with its stamp, so the library takes the new size and time without reading the disk;
     - pushes the report;
     - runs the query again.

- [ ] **Step 1: Write the failing pure tests** in `src/window/text_search_host.rs`'s `tests` module

1. Change the module's first `use super::{...}` to:

```rust
    use super::{
        MIN_QUERY_CHARS, Narrowing, confirm_text, list_mark, narrows, note_mark, overlay_mark,
        report_text, row_confirm_text, searchable,
    };
    use crate::library::text_replace::ReplaceCount;
```

2. At the end of the module, add:

```rust
    #[test]
    fn the_question_counts_matches_and_notes_and_warns_about_closed_notes() {
        // Break caught: "1 matches", a count without its thousands separator, the capped
        // question claiming only the notes that matched, the warning missing when a note will be
        // saved, or a per-row question for a note it can't undo (spec §11, §12a).
        let count = |matches, notes| ReplaceCount { matches, notes };
        assert_eq!(
            confirm_text(count(1, 1), 1, false, false, "x"),
            "Replace 1 match in 1 note with \"x\"?"
        );
        assert_eq!(
            confirm_text(count(1_234, 12), 12, false, true, "y"),
            "Replace 1,234 matches in 12 notes with \"y\"?\nNotes that aren't open are saved and can't be undone."
        );
        assert_eq!(
            confirm_text(count(2_000, 480), 500, true, false, ""),
            "Replace 2,000 matches in the 500 listed notes with \"\"? More notes match; search again to replace in the rest."
        );
        assert_eq!(
            confirm_text(count(2_000, 480), 500, true, true, "z"),
            "Replace 2,000 matches in the 500 listed notes with \"z\"? More notes match; search again to replace in the rest.\nNotes that aren't open are saved and can't be undone."
        );
        assert_eq!(
            row_confirm_text(3, "Q1 budget", "z"),
            "Replace 3 matches in \"Q1 budget\" with \"z\"? The note is saved and this can't be undone."
        );
        assert_eq!(
            row_confirm_text(1, "a", "b"),
            "Replace 1 match in \"a\" with \"b\"? The note is saved and this can't be undone."
        );
    }

    #[test]
    fn the_report_counts_what_was_replaced_and_names_what_was_not() {
        // Break caught: a skipped note reported as replaced, "1 notes were skipped", a note that
        // couldn't be written left unnamed, or a list of 400 names in one notification.
        let names = |names: &[&str]| names.iter().map(|name| (*name).to_owned()).collect::<Vec<_>>();
        assert_eq!(report_text(1, 1, &[], &[]), "Replaced 1 match in 1 note.");
        assert_eq!(
            report_text(5, 2, &names(&["b"]), &[]),
            "Replaced 5 matches in 2 notes.\n1 note was skipped because it changed since the search.\nb"
        );
        assert_eq!(
            report_text(0, 0, &[], &names(&["c", "d"])),
            "Replaced 0 matches in 0 notes.\n2 notes couldn't be written.\nc\nd"
        );
        assert_eq!(
            report_text(3_000, 1, &names(&["x"]), &names(&["y"])),
            "Replaced 3,000 matches in 1 note.\n1 note was skipped because it changed since the search.\n1 note couldn't be written.\nx\ny"
        );
        let changed = (0..12).map(|index| format!("n{index}")).collect::<Vec<_>>();
        let report = report_text(9, 9, &changed, &names(&["f"]));
        let lines = report.lines().collect::<Vec<_>>();
        assert_eq!(lines[1], "12 notes were skipped because they changed since the search.");
        assert_eq!(lines[2], "1 note couldn't be written.");
        let named = changed[..10].iter().map(String::as_str).collect::<Vec<_>>();
        assert_eq!(lines[3..13], named[..]);
        assert_eq!(lines[13], "\u{2026}and 3 more");
        assert_eq!(lines.len(), 14);
    }
```

Run: `cargo test --lib -- window::text_search_host --test-threads=1`
Expected: compile error ``unresolved imports `super::confirm_text`, `super::report_text`, `super::row_confirm_text` ``.

- [ ] **Step 2: Add the messages** in `src/window/messages.rs` and `src/window/mod.rs`

1. In `src/window/messages.rs`, after `pub const WM_FASTPAD_TEXT_SEARCH_BATCH: u32 = WM_APP + 13;`, add:

```rust
// Not part of the deferred chain: a replace's count, then its write report, each a `Box` the
// receiver frees. A post that fails because the window is gone is freed on the worker.
pub const WM_FASTPAD_REPLACE_COUNTED: u32 = WM_APP + 14;
pub const WM_FASTPAD_REPLACE_WRITTEN: u32 = WM_APP + 15;
```

2. In its `tests` module, add `WM_FASTPAD_REPLACE_COUNTED, WM_FASTPAD_REPLACE_WRITTEN, WM_FASTPAD_TEXT_SEARCH_BATCH,` to the `use super::{...}` list, and add the test:

```rust
    #[test]
    fn the_replace_messages_follow_the_search_batch_and_are_never_deferred() {
        // Break caught: a replace payload renumbered onto another message (whose handler would
        // free the wrong Box), or held as a deferred unit and re-posted with its lparam lost.
        assert_eq!(WM_FASTPAD_REPLACE_COUNTED, WM_FASTPAD_TEXT_SEARCH_BATCH + 1);
        assert_eq!(WM_FASTPAD_REPLACE_WRITTEN, WM_FASTPAD_TEXT_SEARCH_BATCH + 2);
        for message in [WM_FASTPAD_REPLACE_COUNTED, WM_FASTPAD_REPLACE_WRITTEN] {
            assert_eq!(classify_deferred_message(message, false), None);
        }
    }
```

3. In `src/window/mod.rs`, add `WM_FASTPAD_REPLACE_COUNTED, WM_FASTPAD_REPLACE_WRITTEN,` to the `pub use messages::{...}` list, in alphabetical order after `WM_FASTPAD_RECOVERY,`.

- [ ] **Step 3: Add the flow** in `src/window/text_search_host.rs`

1. Replace the imports with:

```rust
use crate::document::DocumentId;
use crate::library::model::same_path;
use crate::library::text_replace::{self, ReplaceCount, ReplaceReport, ReplaceTarget};
use crate::library::text_search::{self, Progress, RunEnd, SearchNote, Stamp, TextHit};
use crate::library::{self, NoteEntry, path_key};
use crate::search::{MatchOptions, Matcher};
use crate::window::search_view::thousands;
use crate::window::{library_host, search_view};
use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use windows_sys::Win32::Foundation::{HWND, LPARAM};
use windows_sys::Win32::UI::WindowsAndMessaging::{KillTimer, PostMessageW, SetTimer};
```

2. After `pub(crate) const MIN_QUERY_CHARS: usize = 2;`, add:

```rust
/// Retries a replace count that arrived inside a modal loop or a file population (`replace_timer`).
pub(crate) const REPLACE_TIMER_ID: usize = 0x4650_5250;
const REPLACE_RETRY_MS: u32 = 50;
/// The most note names a replace report lists before "…and K more".
const REPORT_NAMES: usize = 10;
/// A count target's stamp for a hit found in a tab's text. The count reads that tab's text (an
/// overlay) and never compares stamps; no write target ever carries it.
const OVERLAY_STAMP: Stamp = Stamp { size: 0, mtime: 0 };
```

3. In `struct TextSearchHost`, after the `list` field, add:

```rust
    /// Bumped by every replace start and by `cancel_replace`: a count or write report of any
    /// other generation is stale. Separate from the search's `generation`.
    replace_generation: u64,
    replace_cancel: Option<Arc<AtomicBool>>,
    /// A replace is between its start and its report, or its question's No: another press waits.
    replacing: bool,
    /// A count that arrived inside a modal loop or a file population, asked about once both end.
    held: Option<Box<ReplaceCounted>>,
```

4. Replace `forget` with:

```rust
/// A notebook change or close, or notes mode off: cancels the search and any replace, and
/// forgets what the old notebook's searches found. A write already running finishes its file;
/// its report is stale and dropped.
pub(crate) fn forget(hwnd: HWND) {
    cancel(hwnd);
    cancel_replace(hwnd);
    with_host(hwnd, |host| {
        host.previous = None;
        host.list = None;
    });
}
```

5. After `dirty_overlays`, add:

```rust
/// A listed note a replace may change: its path relative to the notebook, its name for the
/// question and the report, and the stamp the search read (`None` when the hit came from a tab's
/// text).
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Candidate {
    pub path: PathBuf,
    pub name: String,
    pub stamp: Option<Stamp>,
}

/// What a replace is for, carried from the start through the count to the write.
#[derive(Debug)]
pub(crate) struct ReplacePlan {
    notebook: PathBuf,
    matcher: Matcher,
    template: String,
    candidates: Vec<Candidate>,
    /// The results were capped: only the listed notes are replaced (spec §12a).
    capped: bool,
    /// A row's replace: one note, asked about only if it isn't open.
    single: bool,
}

/// One `WM_FASTPAD_REPLACE_COUNTED` payload, posted as `Box::into_raw` in `lparam`.
#[derive(Debug)]
pub(crate) struct ReplaceCounted {
    pub generation: u64,
    pub count: ReplaceCount,
    plan: ReplacePlan,
}

/// One `WM_FASTPAD_REPLACE_WRITTEN` payload: the closed notes' report and what the open tabs
/// took on the UI thread before the write began.
#[derive(Debug)]
pub(crate) struct ReplaceWritten {
    pub generation: u64,
    pub report: ReplaceReport,
    pub tab_matches: usize,
    pub tab_notes: usize,
}

/// Replace all (spec §11): every listed note, after the question.
pub(crate) fn replace_all(hwnd: HWND) {
    let (candidates, capped) = search_view::replace_candidates(hwnd);
    start_replace(hwnd, candidates, capped, false);
}

/// A row's replace button: that note only, asked about only if it isn't open.
pub(crate) fn replace_in(hwnd: HWND, relative: &Path) {
    let (candidates, _) = search_view::replace_candidates(hwnd);
    let one = candidates
        .into_iter()
        .filter(|candidate| same_path(&candidate.path, relative))
        .collect::<Vec<_>>();
    if !one.is_empty() {
        start_replace(hwnd, one, false, true);
    }
}

/// Counts the matches in `candidates` on a worker. The question waits for the count.
fn start_replace(hwnd: HWND, candidates: Vec<Candidate>, capped: bool, single: bool) {
    if !search_view::replace_open(hwnd)
        || !search_view::replace_all_enabled(hwnd)
        || busy(hwnd)
        || with_host(hwnd, |host| host.replacing).unwrap_or(true)
    {
        return;
    }
    // The results' query and options, not the box's text, which may be newer.
    let Some((query, options)) = search_view::run_query(hwnd) else {
        return;
    };
    let Ok(matcher) = Matcher::new(&query, options) else {
        return;
    };
    let Some(notebook) = library_host::folder(hwnd) else {
        return;
    };
    let template = search_view::replace_text(hwnd);
    // Read with nothing of the App borrowed: a background tab is swapped into the editor.
    let overlays = target_overlays(hwnd, &notebook, &candidates);
    let targets = candidates
        .iter()
        .map(|candidate| ReplaceTarget {
            path: candidate.path.clone(),
            stamp: candidate.stamp.unwrap_or(OVERLAY_STAMP),
        })
        .collect();
    let Some((generation, cancel)) = with_host(hwnd, |host| {
        host.replace_generation = host.replace_generation.wrapping_add(1);
        let flag = Arc::new(AtomicBool::new(false));
        host.replace_cancel = Some(Arc::clone(&flag));
        host.replacing = true;
        (host.replace_generation, flag)
    }) else {
        return;
    };
    spawn_count(
        hwnd,
        CountJob {
            generation,
            cancel,
            overlays,
            targets,
            plan: ReplacePlan {
                notebook,
                matcher,
                template,
                candidates,
                capped,
                single,
            },
        },
    );
}

/// Everything the count worker owns.
struct CountJob {
    generation: u64,
    cancel: Arc<AtomicBool>,
    overlays: HashMap<PathBuf, String>,
    targets: Vec<ReplaceTarget>,
    plan: ReplacePlan,
}

/// Posts `payload` to `target` as `Box::into_raw`, freeing it here when the post fails (the window
/// is gone: nothing else will).
fn post_boxed<T>(target: isize, message: u32, payload: T) -> bool {
    let payload = Box::into_raw(Box::new(payload));
    let posted = unsafe { PostMessageW(target as HWND, message, 0, payload as isize) } != 0;
    if !posted {
        drop(unsafe { Box::from_raw(payload) });
    }
    posted
}

fn spawn_count(hwnd: HWND, job: CountJob) {
    let target = hwnd as isize;
    std::thread::spawn(move || {
        let CountJob {
            generation,
            cancel,
            overlays,
            targets,
            plan,
        } = job;
        let count = text_replace::count(
            &plan.notebook,
            &targets,
            &overlays,
            &plan.matcher,
            &cancel,
        );
        drop(overlays);
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        post_boxed(
            target,
            crate::window::WM_FASTPAD_REPLACE_COUNTED,
            ReplaceCounted {
                generation,
                count,
                plan,
            },
        );
    });
}

/// `WM_FASTPAD_REPLACE_COUNTED`: takes the box back. A stale generation is dropped. Inside a
/// modal loop or a file population the count is held, since the question is a modal loop of its
/// own and a background tab is swapped into the editor, and `replace_timer` asks once both end.
pub(crate) fn replace_counted(hwnd: HWND, lparam: LPARAM) {
    if lparam == 0 {
        return;
    }
    let counted = unsafe { Box::from_raw(lparam as *mut ReplaceCounted) };
    if with_host(hwnd, |host| host.replace_generation) != Some(counted.generation) {
        return;
    }
    if busy(hwnd) {
        with_host(hwnd, |host| host.held = Some(counted));
        unsafe {
            SetTimer(hwnd, REPLACE_TIMER_ID, REPLACE_RETRY_MS, None);
        }
        return;
    }
    confirm_and_apply(hwnd, *counted);
}

/// `WM_TIMER` for `REPLACE_TIMER_ID`: asks about a held count once no modal loop or file
/// population runs. Until then the timer stays armed.
pub(crate) fn replace_timer(hwnd: HWND) {
    if busy(hwnd) {
        return;
    }
    unsafe {
        KillTimer(hwnd, REPLACE_TIMER_ID);
    }
    let Some(counted) = with_host(hwnd, |host| host.held.take()).flatten() else {
        return;
    };
    if with_host(hwnd, |host| host.replace_generation) != Some(counted.generation) {
        return;
    }
    confirm_and_apply(hwnd, *counted);
}

/// Asks (spec §11), then applies: the open tabs in the editor now, the closed notes on a worker.
fn confirm_and_apply(hwnd: HWND, counted: ReplaceCounted) {
    let ReplaceCounted {
        generation,
        count,
        plan,
    } = counted;
    if count.matches == 0 {
        // The notes changed since the search: the results should show that.
        finish_replace(hwnd);
        run_now(hwnd);
        return;
    }
    let open = open_notes(hwnd, &plan.notebook);
    let any_closed = plan
        .candidates
        .iter()
        .any(|candidate| !open.contains_key(&path_key(&candidate.path)));
    let question = match plan.candidates.first() {
        Some(candidate) if plan.single => {
            any_closed.then(|| row_confirm_text(count.matches, &candidate.name, &plan.template))
        }
        _ => Some(confirm_text(
            count,
            plan.candidates.len(),
            plan.capped,
            any_closed,
            &plan.template,
        )),
    };
    // Asked with nothing of the App borrowed: the question is a nested modal loop.
    if let Some(question) = question
        && !super::modal::confirm(hwnd, &question)
    {
        finish_replace(hwnd);
        return;
    }
    // A notebook change while the question was up makes the plan stale.
    if with_host(hwnd, |host| host.replace_generation) != Some(generation) {
        return;
    }
    apply_plan(hwnd, generation, plan);
}

/// Splits the candidates as the tabs are now and applies the plan.
fn apply_plan(hwnd: HWND, generation: u64, plan: ReplacePlan) {
    let ReplacePlan {
        notebook,
        matcher,
        template,
        candidates,
        ..
    } = plan;
    let open = open_notes(hwnd, &notebook);
    let mut tab_matches = 0;
    let mut tab_notes = 0;
    let mut closed = Vec::new();
    let mut stale = Vec::new();
    for candidate in candidates {
        match open.get(&path_key(&candidate.path)) {
            // An open tab, active or background, clean or dirty: changed in the editor from its
            // live text as one undo action, and not saved (spec §12).
            Some(&id) => {
                match super::main_window::replace_in_document(hwnd, id, &matcher, &template) {
                    Some(0) => {}
                    Some(replaced) => {
                        tab_matches += replaced;
                        tab_notes += 1;
                    }
                    None => stale.push(candidate.path),
                }
            }
            None => match candidate.stamp {
                Some(stamp) => closed.push(ReplaceTarget {
                    path: candidate.path,
                    stamp,
                }),
                // Found in a tab's text, and that tab has closed since: its file was never read.
                None => stale.push(candidate.path),
            },
        }
    }
    if closed.is_empty() {
        let report = ReplaceReport {
            changed: stale,
            ..ReplaceReport::default()
        };
        report_written(hwnd, report, tab_matches, tab_notes);
        return;
    }
    let Some(cancel) = with_host(hwnd, |host| host.replace_cancel.clone()).flatten() else {
        return;
    };
    spawn_write(
        hwnd,
        WriteJob {
            generation,
            cancel,
            notebook,
            matcher,
            template,
            targets: closed,
            stale,
            tab_matches,
            tab_notes,
        },
    );
}

/// Everything the write worker owns.
struct WriteJob {
    generation: u64,
    cancel: Arc<AtomicBool>,
    notebook: PathBuf,
    matcher: Matcher,
    template: String,
    targets: Vec<ReplaceTarget>,
    /// Notes skipped before the write began, reported as changed since the search.
    stale: Vec<PathBuf>,
    tab_matches: usize,
    tab_notes: usize,
}

fn spawn_write(hwnd: HWND, job: WriteJob) {
    let target = hwnd as isize;
    std::thread::spawn(move || {
        let WriteJob {
            generation,
            cancel,
            notebook,
            matcher,
            template,
            targets,
            stale,
            tab_matches,
            tab_notes,
        } = job;
        let mut report = text_replace::apply(&notebook, &targets, &matcher, &template, &cancel);
        report.changed.extend(stale);
        post_boxed(
            target,
            crate::window::WM_FASTPAD_REPLACE_WRITTEN,
            ReplaceWritten {
                generation,
                report,
                tab_matches,
                tab_notes,
            },
        );
    });
}

/// `WM_FASTPAD_REPLACE_WRITTEN`: takes the box back. A stale generation is dropped: the notebook
/// it wrote into is no longer open.
pub(crate) fn replace_written(hwnd: HWND, lparam: LPARAM) {
    if lparam == 0 {
        return;
    }
    let written = *unsafe { Box::from_raw(lparam as *mut ReplaceWritten) };
    if with_host(hwnd, |host| host.replace_generation) != Some(written.generation) {
        return;
    }
    report_written(hwnd, written.report, written.tab_matches, written.tab_notes);
}

/// The end of a replace: the library takes each written note's new size and time from the stamp
/// the write worker took (`record_written`, no disk access), as it does for a FastPad save, so
/// the next rescan reads no outside change. Then the report is pushed and the query runs again,
/// so the results show what still matches.
fn report_written(hwnd: HWND, report: ReplaceReport, tab_matches: usize, tab_notes: usize) {
    if !report.written.is_empty() {
        library_host::with_state(hwnd, |state| {
            for (path, stamp) in &report.written {
                state.record_written(path, *stamp);
            }
        });
    }
    let changed = report
        .changed
        .iter()
        .map(|path| note_name(path))
        .collect::<Vec<_>>();
    let failed = report
        .failed
        .iter()
        .map(|(path, _)| note_name(path))
        .collect::<Vec<_>>();
    let text = report_text(
        tab_matches + report.matches,
        tab_notes + report.written.len(),
        &changed,
        &failed,
    );
    finish_replace(hwnd);
    super::main_window::push_notice(hwnd, text);
    run_now(hwnd);
}

fn finish_replace(hwnd: HWND) {
    with_host(hwnd, |host| {
        host.replacing = false;
        host.replace_cancel = None;
    });
}

/// Stops a replace: a count or write still running stops before its next note (a note being
/// written finishes), and whatever it posts is stale. A held count is dropped.
pub(crate) fn cancel_replace(hwnd: HWND) {
    with_host(hwnd, |host| {
        if let Some(flag) = host.replace_cancel.take() {
            flag.store(true, Ordering::Relaxed);
        }
        host.replace_generation = host.replace_generation.wrapping_add(1);
        host.replacing = false;
        host.held = None;
    });
    unsafe {
        KillTimer(hwnd, REPLACE_TIMER_ID);
    }
}

/// The tabs whose file is inside `notebook`, active or background, clean or dirty, keyed by the
/// `path_key` of the path relative to it. Read from the tabs only: no disk.
fn open_notes(hwnd: HWND, notebook: &Path) -> HashMap<String, DocumentId> {
    unsafe { super::main_window::app_ptr(hwnd) }
        .map(|app| {
            unsafe { app.as_ref() }
                .tabs
                .documents()
                .filter_map(|document| {
                    let path = document.path.as_deref()?;
                    library::is_inside(notebook, path)
                        .then(|| (path_key(&library::record_path(notebook, path)), document.id))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The editor text of every open tab among `candidates`, active or background, clean or dirty,
/// keyed by the candidate's relative path, for the count: the replace changes an open tab's
/// editor text, so that is the text to count in. A background tab is swapped into the editor and
/// back (`document_text`); no file is read. Call it with nothing of the App borrowed.
fn target_overlays(
    hwnd: HWND,
    notebook: &Path,
    candidates: &[Candidate],
) -> HashMap<PathBuf, String> {
    let open = open_notes(hwnd, notebook);
    let mut overlays = HashMap::new();
    for candidate in candidates {
        if let Some(&id) = open.get(&path_key(&candidate.path))
            && let Some(text) = super::main_window::document_text(hwnd, id)
        {
            overlays.insert(candidate.path.clone(), text);
        }
    }
    overlays
}

/// A note's name as the Search view shows it: the file name without its extension.
fn note_name(path: &Path) -> String {
    path.file_stem().map_or_else(
        || path.display().to_string(),
        |stem| stem.to_string_lossy().into_owned(),
    )
}

/// "1 match" or "1,234 matches".
fn count_of(count: usize, one: &str, many: &str) -> String {
    if count == 1 {
        format!("1 {one}")
    } else {
        format!("{} {many}", thousands(count))
    }
}

/// Replace all's question (spec wording). `listed` is how many notes the results list; a capped
/// search names them instead of the notes that matched.
pub(crate) fn confirm_text(
    count: ReplaceCount,
    listed: usize,
    capped: bool,
    any_closed: bool,
    template: &str,
) -> String {
    let matches = count_of(count.matches, "match", "matches");
    let mut text = if capped {
        format!(
            "Replace {matches} in the {} listed notes with \"{template}\"? More notes match; search again to replace in the rest.",
            thousands(listed)
        )
    } else {
        format!(
            "Replace {matches} in {} with \"{template}\"?",
            count_of(count.notes, "note", "notes")
        )
    };
    if any_closed {
        text.push_str("\nNotes that aren't open are saved and can't be undone.");
    }
    text
}

/// A row's question, asked only for a note that isn't open.
pub(crate) fn row_confirm_text(matches: usize, name: &str, template: &str) -> String {
    format!(
        "Replace {} in \"{name}\" with \"{template}\"? The note is saved and this can't be undone.",
        count_of(matches, "match", "matches")
    )
}

/// The report notification: what was replaced, what was skipped and what couldn't be written,
/// then the skipped and failed notes' names, one per line, at most `REPORT_NAMES`.
pub(crate) fn report_text(
    matches: usize,
    notes: usize,
    changed: &[String],
    failed: &[String],
) -> String {
    let mut lines = vec![format!(
        "Replaced {} in {}.",
        count_of(matches, "match", "matches"),
        count_of(notes, "note", "notes")
    )];
    match changed.len() {
        0 => {}
        1 => lines.push("1 note was skipped because it changed since the search.".to_owned()),
        skipped => lines.push(format!(
            "{} notes were skipped because they changed since the search.",
            thousands(skipped)
        )),
    }
    match failed.len() {
        0 => {}
        1 => lines.push("1 note couldn't be written.".to_owned()),
        unwritten => lines.push(format!(
            "{} notes couldn't be written.",
            thousands(unwritten)
        )),
    }
    let total = changed.len() + failed.len();
    lines.extend(changed.iter().chain(failed).take(REPORT_NAMES).cloned());
    if total > REPORT_NAMES {
        lines.push(format!(
            "\u{2026}and {} more",
            thousands(total - REPORT_NAMES)
        ));
    }
    lines.join("\n")
}
```

6. After the `#[cfg(test)] pub(crate) fn test_batch` accessor, add:

```rust
/// A replace is between its start and its report.
#[cfg(test)]
pub(crate) fn replacing(hwnd: HWND) -> bool {
    with_host(hwnd, |host| host.replacing).unwrap_or(false)
}

/// A count waits for a modal loop or a file population to end.
#[cfg(test)]
pub(crate) fn replace_held(hwnd: HWND) -> bool {
    with_host(hwnd, |host| host.held.is_some()).unwrap_or(false)
}
```

- [ ] **Step 4: Give the view's inputs to the flow** in `src/window/search_view.rs`

1. Change `fn thousands(value: usize) -> String {` to `pub(crate) fn thousands(value: usize) -> String {`.

2. Remove the attribute Task 5 put above `pub(crate) fn replace_text(hwnd: HWND) -> String`, all four lines of it:

```rust
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "the replace flow reads it from Task 6")
)]
```

`start_replace` reads it now.

3. Remove the attribute Task 5 put above `pub(crate) fn replace_all_enabled(hwnd: HWND) -> bool`:

```rust
#[expect(dead_code, reason = "the replace flow reads it from Task 6")]
```

`start_replace` reads it now. After this step no `expect(dead_code` is left in `search_view.rs`: `rg -n "expect\(dead_code" src/window/search_view.rs` prints nothing.

4. Remove the `#[cfg(test)]` above `pub(crate) fn replace_open(hwnd: HWND) -> bool` (Task 4), and change its doc comment to `/// Whether the replace field is open: Replace all and the rows' buttons need it.`

5. After `result_paths`, add:

```rust
/// The listed notes in list order, as a replace takes them (spec §12a: never a note that isn't
/// listed), and whether the results were capped.
pub(crate) fn replace_candidates(hwnd: HWND) -> (Vec<text_search_host::Candidate>, bool) {
    with_view(hwnd, |view| {
        let candidates = view
            .results
            .iter()
            .map(|hit| text_search_host::Candidate {
                path: hit.path.clone(),
                name: hit.name.clone(),
                stamp: hit.stamp,
            })
            .collect();
        let capped = matches!(view.search, SearchState::Done { capped: true, .. });
        (candidates, capped)
    })
    .unwrap_or_default()
}
```

6. In `handle`, replace

```rust
                Some(HeaderButton::ReplaceAll) => return Some(0),
```

with

```rust
                Some(HeaderButton::ReplaceAll) => {
                    if pressed {
                        text_search_host::replace_all(hwnd);
                    }
                    return Some(0);
                }
```

and replace

```rust
            if with_view(hwnd, |view| view.row_button_at(at, client, dpi))
                .flatten()
                .is_some()
            {
                return Some(0);
            }
```

with

```rust
            if let Some(path) = with_view(hwnd, |view| {
                let index = view.row_button_at(at, client, dpi)?;
                Some(view.results[index].path.clone())
            })
            .flatten()
            {
                if pressed {
                    text_search_host::replace_in(hwnd, &path);
                }
                return Some(0);
            }
```

7. In `search_edit_proc`, replace

```rust
    {
        return 0;
    }
    if message == WM_KEYDOWN {
```

(the end of the Ctrl+Alt+Enter test) with

```rust
    {
        text_search_host::replace_all(main);
        return 0;
    }
    if message == WM_KEYDOWN {
```

- [ ] **Step 5: Apply to a tab, and dispatch the messages** in `src/window/main_window.rs` and `src/window/tabs.rs`

1. In `src/window/tabs.rs`, after `note_active_text_change`, add:

```rust
    /// A background tab's text was changed in the editor while it was swapped in with
    /// notifications suppressed (a Search replace, note-search spec §12a). Records what
    /// `SCN_SAVEPOINTLEFT` and `SCN_MODIFIED` would have for the active tab: dirty, a new
    /// generation (so recovery snapshots it), and a preview kept. Returns whether the strip
    /// changed.
    pub(crate) fn note_background_edit(&mut self, id: DocumentId) -> bool {
        let Some(document) = self.documents.iter_mut().find(|document| document.id == id) else {
            return false;
        };
        document.generation = document.generation.saturating_add(1);
        let changed = !document.dirty || document.preview;
        document.dirty = true;
        document.preview = false;
        if changed {
            self.view.update(&self.documents);
        }
        changed
    }
```

and in its `tests` module add:

```rust
    #[test]
    fn a_background_edit_marks_only_that_tab_dirty_and_keeps_its_preview() {
        // Break caught: a Search replace into a background tab leaving it clean (closing it
        // would drop the replacement without asking), marking the active tab instead, or leaving
        // a preview that the next click replaces with its edits in it.
        let mut tabs = Tabs::with_document(document(1));
        tabs.push(preview(2)).unwrap();
        tabs.activate(DocumentId(1)).unwrap();
        let before = tabs.document(DocumentId(2)).unwrap().generation;
        assert!(tabs.note_background_edit(DocumentId(2)));
        let edited = tabs.document(DocumentId(2)).unwrap();
        assert!(edited.dirty && !edited.preview);
        assert_eq!(edited.generation, before + 1);
        assert!(!tabs.active().unwrap().dirty);
        assert!(!tabs.note_background_edit(DocumentId(2)), "already dirty");
        assert!(!tabs.note_background_edit(DocumentId(9)), "no such tab");
    }
```

The module already imports `Document` and `DocumentId`.

2. In `src/window/main_window.rs`, replace `read_inactive_text` with:

```rust
/// Scintilla can only read or change the document shown in the view, so an inactive tab is
/// swapped in, `f` runs on it, and it is swapped out again, with notifications suppressed
/// (`populating_file`), restoring the visible selection and scroll position.
fn with_inactive_document<R>(
    hwnd: HWND,
    identity: &WindowIdentity,
    editor: &Editor,
    target: &crate::editor::EditorDocument,
    active: &crate::editor::EditorDocument,
    f: impl FnOnce(&Editor) -> Result<R>,
) -> Result<R> {
    use crate::editor::scintilla_constants::{SCI_GETFIRSTVISIBLELINE, SCI_SETFIRSTVISIBLELINE};
    let selection = editor.selection();
    let first_line = unsafe { SendMessageW(editor.hwnd(), SCI_GETFIRSTVISIBLELINE, 0, 0) };
    set_file_population(hwnd, true);
    let value = editor.use_document(target).and_then(|_| f(editor));
    let restored = if identity.is_live_for(hwnd) {
        editor.use_document(active)
    } else {
        Err(crate::FastPadError::Invariant(
            "main window was destroyed while a background tab was swapped in",
        ))
    };
    if restored.is_ok() {
        if let Ok(selection) = selection {
            let _ = editor.set_selection(selection);
        }
        unsafe {
            SendMessageW(
                editor.hwnd(),
                SCI_SETFIRSTVISIBLELINE,
                first_line as usize,
                0,
            );
        }
    }
    if identity.is_live_for(hwnd) {
        set_file_population(hwnd, false);
    }
    restored?;
    value
}

/// An inactive tab's text (`with_inactive_document`).
fn read_inactive_text(
    hwnd: HWND,
    identity: &WindowIdentity,
    editor: &Editor,
    target: &crate::editor::EditorDocument,
    active: &crate::editor::EditorDocument,
) -> Result<String> {
    with_inactive_document(hwnd, identity, editor, target, active, Editor::text)
}
```

3. After `document_text`, add:

```rust
/// Replaces every match of `matcher` in tab `id`'s live text with `template` (expanded in regex
/// mode), in the editor, as one undo action (note-search spec §12). The tab is not saved. The
/// active tab's edit raises Scintilla's notifications as typing does. A background tab is
/// swapped in (`with_inactive_document`, notifications suppressed) and then marked edited by
/// hand (`Tabs::note_background_edit`). Returns how many matches were replaced, or `None`
/// without an editor or that tab, while a file is being populated, or when Scintilla fails. Call
/// it with nothing of the App borrowed.
pub(crate) fn replace_in_document(
    hwnd: HWND,
    id: DocumentId,
    matcher: &crate::search::Matcher,
    template: &str,
) -> Option<usize> {
    let identity = unsafe { window_identity(hwnd) }?;
    let (editor, inactive) = unsafe { app_ptr(hwnd) }.and_then(|app| {
        let app = unsafe { app.as_ref() };
        // While a file is populated the editor may show a document that is not the active tab's.
        if app.populating_file {
            return None;
        }
        let editor = app.editor.clone()?;
        let active = app.tabs.active()?;
        let target = app.tabs.document(id)?;
        if target.id == active.id {
            return Some((editor, None));
        }
        Some((editor, Some((target.handle.clone(), active.handle.clone()))))
    })?;
    let replace = |editor: &Editor| -> Result<usize> {
        let edits = editor.with_document_text(|text| matcher.replacements(text, template))?;
        if edits.is_empty() {
            return Ok(0);
        }
        editor.replace_ranges_with(&edits)
    };
    match inactive {
        None => replace(&editor).ok(),
        Some((target, active)) => {
            let replaced =
                with_inactive_document(hwnd, &identity, &editor, &target, &active, replace)
                    .ok()?;
            if replaced > 0 && identity.is_live_for(hwnd) {
                let changed = unsafe { app_ptr(hwnd) }
                    .is_some_and(|mut app| unsafe { app.as_mut() }.tabs.note_background_edit(id));
                if changed {
                    invalidate_title_strip(hwnd);
                }
            }
            Some(replaced)
        }
    }
}
```

4. In the window procedure's `WM_DESTROY` arm, after `crate::window::text_search_host::cancel(hwnd);`, add:

```rust
            crate::window::text_search_host::cancel_replace(hwnd);
```

5. After the `WM_TIMER if wparam == crate::window::text_search_host::TEXT_SEARCH_TIMER_ID` arm, add:

```rust
        WM_TIMER if wparam == crate::window::text_search_host::REPLACE_TIMER_ID => {
            crate::window::text_search_host::replace_timer(hwnd);
            0
        }
```

6. In the `_ =>` arm, after the `WM_FASTPAD_TEXT_SEARCH_BATCH` block, add:

```rust
            if message == crate::window::WM_FASTPAD_REPLACE_COUNTED {
                crate::window::text_search_host::replace_counted(hwnd, lparam);
                return 0;
            }
            if message == crate::window::WM_FASTPAD_REPLACE_WRITTEN {
                crate::window::text_search_host::replace_written(hwnd, lparam);
                return 0;
            }
```

- [ ] **Step 6: Show a report of several lines on the bar's one line** in `src/window/status.rs`

1. Replace `status_text` with:

```rust
pub fn status_text(notifications: &NotificationCenter) -> Option<String> {
    let (first, rest) = notifications.pending().split_first()?;
    // The bar is one line: a notification of several (a replace report naming notes) shows them
    // one after another.
    let message = first.message.lines().collect::<Vec<_>>().join(" ");
    Some(if rest.is_empty() {
        format!("{message} (click to dismiss)")
    } else {
        format!("{message} (+{} more, click to dismiss)", rest.len())
    })
}
```

2. In its `tests` module, add:

```rust
    #[test]
    fn a_notification_of_several_lines_shows_on_the_bars_one_line() {
        // Break caught: a replace report's line breaks drawn as boxes, or everything after the
        // first line lost from the bar.
        let mut center = NotificationCenter::new();
        center.push("Replaced 1 match in 1 note.\n1 note couldn't be written.\nb");
        assert_eq!(
            status_text(&center).as_deref(),
            Some("Replaced 1 match in 1 note. 1 note couldn't be written. b (click to dismiss)")
        );
    }
```

Run: `cargo test --lib -- window::text_search_host window::messages window::status window::tabs --test-threads=1`
Expected: all pass, including the two wording tests.

- [ ] **Step 7: Write the window tests** in `src/window/main_window.rs`'s `tests` module

After `the_chevron_opens_and_closes_the_replace_field` (Task 5), add:

```rust
    /// Opens the replace field, runs the search for `query` to its end, and types `replacement`.
    fn search_to_replace(hwnd: HWND, query: &str, replacement: &str) {
        crate::window::search_view::show_replace(hwnd);
        search_for(hwnd, query);
        type_into_replace(hwnd, replacement);
    }

    /// Pumps until a replace report is pushed, and returns it.
    fn wait_for_report(hwnd: HWND) -> String {
        let report =
            || notices(hwnd).into_iter().find(|notice| notice.starts_with("Replaced "));
        pump_until(hwnd, || report().is_some());
        report().unwrap()
    }

    #[test]
    fn a_background_dirty_tab_is_replaced_in_the_editor_not_on_disk() {
        // Break caught (Review Focus 5): a background tab's unsaved edits replaced from the
        // note's disk text or written over on disk, the same note also written as a closed note,
        // the active tab changed in the background tab's place, the background tab left clean,
        // or its replacement taking more than one undo.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("replace-background-dirty");
        let a = scratch.note("a.md", "old needle\r\n");
        let b = scratch.note("b.md", "b needle\n");
        let c = scratch.note("c.md", "c needle");
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        scratch.install(window.hwnd);
        // Leaving a tab autosaves it (`switching_tabs_autosaves_the_tab_being_left`); a's edits
        // must stay unsaved.
        execute_command(window.hwnd, CommandId::ToggleFolderAutosave);
        crate::window::modal::take_last_confirm();
        super::open_path(window.hwnd, &a).unwrap();
        pump_posted_messages(window.hwnd);
        editor.set_text("typed needle here\r\n").unwrap();
        let a_id = app_mut(window.hwnd).tabs.active().unwrap().id;
        assert!(app_mut(window.hwnd).tabs.active().unwrap().dirty);
        super::open_path(window.hwnd, &b).unwrap();
        pump_posted_messages(window.hwnd);
        let b_id = app_mut(window.hwnd).tabs.active().unwrap().id;
        assert_ne!(a_id, b_id);

        search_to_replace(window.hwnd, "needle", "pin");
        assert_eq!(search_rows(window.hwnd).len(), 3);
        crate::window::answer_next_confirm(|_| true);
        crate::window::text_search_host::replace_all(window.hwnd);
        assert_eq!(wait_for_report(window.hwnd), "Replaced 3 matches in 3 notes.");
        assert_eq!(
            crate::window::modal::take_last_confirm().as_deref(),
            Some(
                "Replace 3 matches in 3 notes with \"pin\"?\nNotes that aren't open are saved and can't be undone."
            )
        );

        assert_eq!(
            std::fs::read_to_string(&a).unwrap(),
            "old needle\r\n",
            "a's file is never written"
        );
        assert_eq!(
            std::fs::read_to_string(&b).unwrap(),
            "b needle\n",
            "b is open too: changed in the editor only"
        );
        assert_eq!(std::fs::read_to_string(&c).unwrap(), "c pin", "c is closed: written");
        assert_eq!(app_mut(window.hwnd).tabs.active().unwrap().id, b_id, "b stays in front");
        assert_eq!(editor.text().unwrap(), "b pin\n");
        assert!(app_mut(window.hwnd).tabs.document(a_id).unwrap().dirty);

        assert!(super::activate_document_by_id(window.hwnd, a_id));
        assert_eq!(
            editor.text().unwrap(),
            "typed pin here\r\n",
            "replaced in the tab's live text"
        );
        editor.undo().unwrap();
        assert_eq!(editor.text().unwrap(), "typed needle here\r\n", "one undo action");
    }

    #[test]
    fn replace_all_writes_the_closed_notes_updates_the_library_and_searches_again() {
        // Break caught: a closed note left unwritten, a note written that the search never
        // listed, the library keeping the old size (the next rescan would read FastPad's own
        // write as an outside change), or the results still listing notes with nothing left to
        // match.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("replace-closed");
        let a = scratch.note("a.md", "one needle, two needle\r\n");
        std::fs::create_dir_all(scratch.folder().join("sub")).unwrap();
        let b = scratch.note(r"sub\b.md", "needle\n");
        let c = scratch.note("c.md", "nothing");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        scratch.install(window.hwnd);
        search_to_replace(window.hwnd, "needle", "pin");
        let before = search_generation(window.hwnd);

        crate::window::answer_next_confirm(|_| true);
        crate::window::text_search_host::replace_all(window.hwnd);
        assert_eq!(wait_for_report(window.hwnd), "Replaced 3 matches in 2 notes.");
        assert_eq!(
            crate::window::modal::take_last_confirm().as_deref(),
            Some(
                "Replace 3 matches in 2 notes with \"pin\"?\nNotes that aren't open are saved and can't be undone."
            )
        );
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "one pin, two pin\r\n");
        assert_eq!(std::fs::read_to_string(&b).unwrap(), "pin\n");
        assert_eq!(std::fs::read_to_string(&c).unwrap(), "nothing");
        let size = crate::window::library_host::with_state(window.hwnd, |state| {
            state
                .notes
                .iter()
                .find(|note| crate::library::model::same_path(&note.path, std::path::Path::new("a.md")))
                .map(|note| note.size)
        })
        .flatten();
        assert_eq!(size, Some("one pin, two pin\r\n".len() as u64));
        assert!(!crate::window::text_search_host::replacing(window.hwnd));

        wait_for_search(window.hwnd, before);
        assert_eq!(
            crate::window::search_view::summary(window.hwnd),
            Some((crate::window::search_view::NO_MATCH.to_owned(), false))
        );
    }

    #[test]
    fn declining_the_question_writes_nothing_and_a_later_replace_still_runs() {
        // Break caught: a No that still writes the closed notes or changes the open tab, or one
        // that leaves the replace marked as running, so Replace all never works again.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("replace-declined");
        let a = scratch.note("a.md", "needle");
        let b = scratch.note("b.md", "b needle");
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        scratch.install(window.hwnd);
        super::open_path(window.hwnd, &b).unwrap();
        pump_posted_messages(window.hwnd);
        search_to_replace(window.hwnd, "needle", "pin");

        let asked = std::rc::Rc::new(std::cell::Cell::new(false));
        let answered = std::rc::Rc::clone(&asked);
        crate::window::answer_next_confirm(move |_| {
            answered.set(true);
            false
        });
        crate::window::text_search_host::replace_all(window.hwnd);
        pump_until(window.hwnd, || asked.get());
        pump_past_debounce(window.hwnd);
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "needle");
        assert_eq!(editor.text().unwrap(), "b needle");
        assert!(!notices(window.hwnd).iter().any(|notice| notice.starts_with("Replaced ")));
        assert!(!crate::window::text_search_host::replacing(window.hwnd));

        crate::window::answer_next_confirm(|_| true);
        crate::window::text_search_host::replace_all(window.hwnd);
        assert_eq!(wait_for_report(window.hwnd), "Replaced 2 matches in 2 notes.");
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "pin");
        assert_eq!(editor.text().unwrap(), "b pin");
    }

    #[test]
    fn a_note_changed_on_disk_since_the_search_is_skipped_and_named_in_the_report() {
        // Break caught (Review Focus 1, in the window): a sync client's newer text overwritten
        // with a replacement of the text the search read, or the skip left out of the report.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("replace-changed");
        let a = scratch.note("a.md", "needle");
        let b = scratch.note("b.md", "needle");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        scratch.install(window.hwnd);
        search_to_replace(window.hwnd, "needle", "pin");
        std::fs::write(&b, "needle, edited elsewhere").unwrap();

        crate::window::answer_next_confirm(|_| true);
        crate::window::text_search_host::replace_all(window.hwnd);
        assert_eq!(
            wait_for_report(window.hwnd),
            "Replaced 1 match in 1 note.\n1 note was skipped because it changed since the search.\nb"
        );
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "pin");
        assert_eq!(std::fs::read_to_string(&b).unwrap(), "needle, edited elsewhere");
    }

    #[test]
    fn the_row_replace_changes_one_note_and_asks_only_when_it_is_closed() {
        // Break caught: a row's button replacing in every result, asking about a note whose
        // change one Ctrl+Z undoes, or saving a closed note without asking.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("replace-row");
        let a = scratch.note("a.md", "needle");
        let b = scratch.note("b.md", "b needle");
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        scratch.install(window.hwnd);
        super::open_path(window.hwnd, &b).unwrap();
        pump_posted_messages(window.hwnd);
        search_to_replace(window.hwnd, "needle", "pin");
        crate::window::modal::take_last_confirm();
        let before = search_generation(window.hwnd);

        crate::window::text_search_host::replace_in(window.hwnd, std::path::Path::new("b.md"));
        assert_eq!(wait_for_report(window.hwnd), "Replaced 1 match in 1 note.");
        assert_eq!(crate::window::modal::take_last_confirm(), None, "b is open");
        assert_eq!(editor.text().unwrap(), "b pin");
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "needle");
        wait_for_search(window.hwnd, before);
        assert_eq!(
            search_rows(window.hwnd),
            vec![search_row("a", "needle")],
            "b's row is gone"
        );

        app_mut(window.hwnd).notifications.dismiss_all();
        crate::window::answer_next_confirm(|_| true);
        crate::window::text_search_host::replace_in(window.hwnd, std::path::Path::new("a.md"));
        assert_eq!(wait_for_report(window.hwnd), "Replaced 1 match in 1 note.");
        assert_eq!(
            crate::window::modal::take_last_confirm().as_deref(),
            Some(
                "Replace 1 match in \"a\" with \"pin\"? The note is saved and this can't be undone."
            )
        );
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "pin");
    }

    #[test]
    fn ctrl_alt_enter_replaces_an_open_tab_with_its_groups_as_one_undo_action() {
        // Break caught: Ctrl+Alt+Enter opening a result instead, `$1` inserted literally in regex
        // mode (spec §12a), the tab saved, or the replacement taking one undo per match.
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
            GetKeyboardState, SetKeyboardState, VK_CONTROL, VK_MENU, VK_RETURN,
        };
        use windows_sys::Win32::UI::WindowsAndMessaging::WM_KEYDOWN;
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("replace-ctrl-alt-enter");
        let a = scratch.note("a.md", "x needle y needle");
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        scratch.install(window.hwnd);
        execute_command(window.hwnd, CommandId::ToggleFolderAutosave);
        super::open_path(window.hwnd, &a).unwrap();
        pump_posted_messages(window.hwnd);
        crate::window::search_view::show_replace(window.hwnd);
        crate::window::search_view::toggle_option(window.hwnd, crate::search::SearchOption::Regex);
        search_for(window.hwnd, "n(ee)dle");
        type_into_replace(window.hwnd, "[$1]");
        let replace = crate::window::search_view::replace_edit_hwnd(window.hwnd).unwrap();

        crate::window::answer_next_confirm(|_| true);
        let mut keys = [0u8; 256];
        unsafe { GetKeyboardState(keys.as_mut_ptr()) };
        let original = keys;
        keys[VK_CONTROL as usize] = 0x80;
        keys[VK_MENU as usize] = 0x80;
        unsafe { SetKeyboardState(keys.as_ptr()) };
        unsafe { SendMessageW(replace, WM_KEYDOWN, VK_RETURN as usize, 0) };
        unsafe { SetKeyboardState(original.as_ptr()) };

        assert_eq!(wait_for_report(window.hwnd), "Replaced 2 matches in 1 note.");
        assert_eq!(
            crate::window::modal::take_last_confirm().as_deref(),
            Some("Replace 2 matches in 1 note with \"[$1]\"?"),
            "every note is open: no warning line"
        );
        assert_eq!(editor.text().unwrap(), "x [ee] y [ee]");
        assert!(app_mut(window.hwnd).tabs.active().unwrap().dirty);
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "x needle y needle");
        editor.undo().unwrap();
        assert_eq!(editor.text().unwrap(), "x needle y needle", "one undo action");
    }

    #[test]
    fn a_count_that_arrives_after_a_notebook_change_is_dropped() {
        // Break caught: the question asked, or the old notebook's notes written, after the user
        // switched notebooks while the count ran.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("replace-stale");
        let a = scratch.note("a.md", "needle");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        scratch.install(window.hwnd);
        search_to_replace(window.hwnd, "needle", "pin");
        crate::window::modal::take_last_confirm();

        crate::window::answer_next_confirm(|_| true);
        crate::window::text_search_host::replace_all(window.hwnd);
        // What a notebook change does (`library_host`'s notebook switch calls it).
        crate::window::text_search_host::forget(window.hwnd);
        pump_past_debounce(window.hwnd);
        assert_eq!(crate::window::modal::take_last_confirm(), None);
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "needle");
        assert!(!crate::window::text_search_host::replacing(window.hwnd));
        // The queued answer was never used: take it, so no later test gets it.
        assert!(crate::window::modal::confirm(window.hwnd, "drain"));
    }

    #[test]
    fn a_count_that_arrives_while_a_file_is_populated_asks_once_it_ends() {
        // Break caught: the question (a nested modal loop) or a background-tab swap run in the
        // middle of a file population, or a held count lost so the replace never asks.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("replace-held");
        let a = scratch.note("a.md", "needle");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        scratch.install(window.hwnd);
        search_to_replace(window.hwnd, "needle", "pin");
        let asked = std::rc::Rc::new(std::cell::Cell::new(false));
        let answered = std::rc::Rc::clone(&asked);
        crate::window::answer_next_confirm(move |_| {
            answered.set(true);
            false
        });

        crate::window::text_search_host::replace_all(window.hwnd);
        app_mut(window.hwnd).populating_file = true;
        pump_until(window.hwnd, || {
            crate::window::text_search_host::replace_held(window.hwnd)
        });
        pump_past_debounce(window.hwnd);
        assert!(!asked.get(), "no question during the population");
        app_mut(window.hwnd).populating_file = false;
        pump_until(window.hwnd, || asked.get());
        assert!(!crate::window::text_search_host::replace_held(window.hwnd));
        assert!(!crate::window::text_search_host::replacing(window.hwnd));
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "needle");
    }
```

The tests module imports only `PathBuf` from `std::path`, so `Path` is spelled out in full.

Run: `cargo test --lib -- a_background_dirty_tab replace_all_writes declining_the_question a_note_changed_on_disk the_row_replace ctrl_alt_enter_replaces a_count_that_arrives --test-threads=1`
Expected: all eight pass.

Also run 3a's snapshot and overlay tests, which go through the refactored `read_inactive_text`:

Run: `cargo test --lib -- snapshot dirty_tab overlay --test-threads=1`
Expected: all pass.

- [ ] **Step 8: Lint, format and commit**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: no warnings.

Run: `cargo fmt --all`, then `cargo fmt --all -- --check`
Expected: the check prints nothing.

```bash
git add src/window/text_search_host.rs src/window/search_view.rs src/window/main_window.rs src/window/tabs.rs src/window/messages.rs src/window/mod.rs src/window/status.rs
git commit -m "feat(search): Replace all and a row's replace count on a worker, ask, change open tabs (background ones swapped in) in the editor as one undo action, write closed notes on a worker, update the library for FastPad's own writes, report skipped and failed notes, and search again"
```

---

### Task 7: Accessibility for the replace controls, the end-to-end test, README and §17

**Files:**
- Modify `src/window/sidebar_accessibility.rs`: a new `STATE_UNAVAILABLE`, new `expander_item` and `action_item`, and a test.
- Modify `src/window/search_view.rs`:
  - `SearchChild`;
  - `SearchView::{head_children}`, plus new `replace_field_shown` and `row_replace_child`;
  - the `AccessibleView` impl: `accessible_item`, `accessible_hit`, `accessible_select` and `accessible_identity`;
  - `announce_toggle`, now a caller of a new `announce_state`;
  - `set_replace_open`, `begin_search` and `apply_batch`.
- Modify `src/window/main_window.rs` (tests only): 3a's `the_search_view_exposes_its_box_toggles_summary_and_results`, where the summary moves from index 4 to 5, and a new test.
- Modify `tests/windows/library.rs`:
  - `AccessibleVtable::get_acc_state` gets its real type;
  - new `Accessible::state`, `edit_text`, `replace_all_available` and one test;
  - the `support::process` import.
- Modify `README.md`: the Notes bullet list and the shortcut table.
- Modify `docs/superpowers/specs/2026-09-24-note-search-design.md`: §17's bullet on the find bar's literal Replace is rewritten for `$1`, and §17 gains the 3b implementation notes.

**Interfaces:**
- Consumes the Search view's replace state (Task 5) and the flow (Task 6).
- Produces:
  - `pub(crate) const STATE_UNAVAILABLE: u32 = 0x0000_0001`.
  - `pub(crate) fn expander_item(name: &str, expanded: bool, rect: RECT) -> AccessibleItem`: a push button with `STATE_EXPANDED` or `STATE_COLLAPSED`, and no focus.
  - `pub(crate) fn action_item(name: &str, enabled: bool, rect: RECT) -> AccessibleItem`: a push button, `STATE_UNAVAILABLE` while it can't run, and no focus.
- **The Search view's MSAA children, in order:**
  1. the box;
  2. the three toggles;
  3. **"Toggle replace"**;
  4. while the field is open, **"Replace"** (its full object is the Edit's own) and **"Replace all"**;
  5. the summary line and the status line, while they show;
  6. while the field is open, **"Replace in <name>"**, the selected row's button;
  7. the results.
- The chevron comes after the toggles, so the box and the toggles keep 3a's child IDs 1 to 4.
- There is a single row button, the selected row's: one button per row would double the children. It comes before the results, so the results' IDs don't move as the selection does.
- **Events:**
  - Opening and closing the field raises `EVENT_OBJECT_STATECHANGE` on the chevron.
  - Replace all becoming available or unavailable raises it on Replace all.
  - The children coming and going raise the panel's reorder through `with_accessible_events` (Task 5).

- [ ] **Step 1: Write the failing tests**

1. In `src/window/sidebar_accessibility.rs`'s `tests` module (it has `use super::*;`), add:

```rust
    #[test]
    fn the_chevron_says_whether_it_is_expanded_and_an_unavailable_button_says_so() {
        // Break caught: a chevron read as a plain button with no hint of what it opens, or
        // Replace all read as pressable while a search still runs.
        let open = expander_item("Toggle replace", true, ROW);
        assert_eq!(open.role, ROLE_SYSTEM_PUSHBUTTON);
        assert_ne!(open.state & STATE_EXPANDED, 0);
        assert_eq!(open.state & (STATE_COLLAPSED | STATE_FOCUSABLE), 0);
        assert_eq!(default_action(&open), "Press");
        let closed = expander_item("Toggle replace", false, ROW);
        assert_ne!(closed.state & STATE_COLLAPSED, 0);
        assert_eq!(closed.state & STATE_EXPANDED, 0);

        let ready = action_item("Replace all", true, ROW);
        assert_eq!(ready.role, ROLE_SYSTEM_PUSHBUTTON);
        assert_eq!(ready.state & (STATE_UNAVAILABLE | STATE_FOCUSABLE), 0);
        assert_ne!(
            action_item("Replace all", false, ROW).state & STATE_UNAVAILABLE,
            0
        );
    }
```

2. In `src/window/main_window.rs`'s `tests` module, in `the_search_view_exposes_its_box_toggles_summary_and_results`, replace

```rust
        assert_eq!(shown[4].role, ROLE_SYSTEM_STATICTEXT);
        assert_eq!(shown[4].name, "2 notes");
```

with

```rust
        // The chevron comes after the toggles, so the box and the toggles keep IDs 1 to 4.
        assert_eq!(shown[4].name, "Toggle replace");
        assert_eq!(shown[5].role, ROLE_SYSTEM_STATICTEXT);
        assert_eq!(shown[5].name, "2 notes");
```

and after `ctrl_alt_enter_replaces_an_open_tab_with_its_groups_as_one_undo_action` (Task 6) add:

```rust
    #[test]
    fn the_replace_controls_are_exposed_with_their_names_and_states() {
        // Break caught (spec §11 names): the chevron missing or read without its expanded state
        // (or silent when it changes), the replace field or Replace all invisible to a screen
        // reader, Replace all read as pressable while a search runs, or the row's button unnamed.
        use crate::window::sidebar_accessibility::{
            STATE_COLLAPSED, STATE_EXPANDED, STATE_UNAVAILABLE, take_raised,
        };
        use windows_sys::Win32::UI::Accessibility::{ROLE_SYSTEM_PUSHBUTTON, ROLE_SYSTEM_TEXT};
        use windows_sys::Win32::UI::WindowsAndMessaging::EVENT_OBJECT_STATECHANGE;
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("replace-msaa");
        scratch.note("a.md", "one beta");
        scratch.note("b.md", "beta two");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        scratch.install(window.hwnd);
        crate::window::side_panel::show_view(window.hwnd, crate::config::SidebarView::Search, true);
        search_for(window.hwnd, "beta");
        let panel = sidebar_panel(window.hwnd);
        let items = || {
            (0..crate::window::side_panel::accessible_item_count(panel))
                .filter_map(|index| crate::window::side_panel::accessible_item(panel, index))
                .collect::<Vec<_>>()
        };
        let index_of =
            |name: &str| items().iter().position(|item| item.name == name);

        let shown = items();
        assert_eq!(shown[0].role, ROLE_SYSTEM_TEXT, "the box keeps child ID 1");
        assert_eq!(index_of("Toggle replace"), Some(4), "after the three toggles");
        assert_eq!(shown[4].role, ROLE_SYSTEM_PUSHBUTTON);
        assert_ne!(shown[4].state & STATE_COLLAPSED, 0);
        assert_eq!(index_of("Replace"), None);
        assert_eq!(index_of("Replace all"), None);
        assert_eq!(index_of("Replace in a"), None);

        take_raised();
        crate::window::search_view::toggle_replace(window.hwnd);
        assert!(
            take_raised().contains(&(panel as usize, EVENT_OBJECT_STATECHANGE, 5)),
            "the chevron (ID 5) raises a state change"
        );
        let shown = items();
        assert_ne!(shown[4].state & STATE_EXPANDED, 0);
        let field = &shown[index_of("Replace").expect("the replace field is a child")];
        assert_eq!(field.role, ROLE_SYSTEM_TEXT);
        assert_eq!(
            field.window,
            crate::window::search_view::replace_edit_hwnd(window.hwnd).unwrap()
        );
        let all = &shown[index_of("Replace all").expect("Replace all is a child")];
        assert_eq!(all.role, ROLE_SYSTEM_PUSHBUTTON);
        assert_eq!(all.state & STATE_UNAVAILABLE, 0);
        let row = &shown[index_of("Replace in a").expect("the selected row's button")];
        assert_eq!(row.role, ROLE_SYSTEM_PUSHBUTTON);
        type_into_replace(window.hwnd, "x");
        assert_eq!(items()[index_of("Replace").unwrap()].value, "x");

        take_raised();
        // The same query again: its results stay, and Replace all waits for the search.
        crate::window::text_search_host::run_now(window.hwnd);
        let all_index = index_of("Replace all").unwrap();
        assert_ne!(items()[all_index].state & STATE_UNAVAILABLE, 0);
        assert!(
            take_raised().contains(&(
                panel as usize,
                EVENT_OBJECT_STATECHANGE,
                all_index as i32 + 1
            )),
            "Replace all says it became unavailable"
        );
    }
```

Run: `cargo test --lib -- window::sidebar_accessibility the_replace_controls_are_exposed the_search_view_exposes --test-threads=1`
Expected: compile errors ``cannot find function `expander_item` in this scope`` and ``cannot find value `STATE_UNAVAILABLE` in this scope``.

- [ ] **Step 2: Add the item kinds** in `src/window/sidebar_accessibility.rs`

1. Before `pub(crate) const STATE_SELECTED: u32 = 0x0000_0002;`, so the constants stay in bit order, add:

```rust
pub(crate) const STATE_UNAVAILABLE: u32 = 0x0000_0001;
```

2. After `button_item`, add:

```rust
/// A painted button that opens and closes something, such as the Search view's replace
/// chevron: a push button whose state says whether it is expanded or collapsed. It takes no
/// keyboard focus; Ctrl+Shift+H and a click open the field.
pub(crate) fn expander_item(name: &str, expanded: bool, rect: RECT) -> AccessibleItem {
    let mut item = button_item(name, false, false, rect);
    item.state &= !STATE_FOCUSABLE;
    item.state |= if expanded {
        STATE_EXPANDED
    } else {
        STATE_COLLAPSED
    };
    item
}

/// A painted push button that is unavailable while its action can't run, such as Replace all
/// before a search has finished. It takes no keyboard focus; Ctrl+Alt+Enter runs Replace all.
pub(crate) fn action_item(name: &str, enabled: bool, rect: RECT) -> AccessibleItem {
    let mut item = button_item(name, false, false, rect);
    item.state &= !STATE_FOCUSABLE;
    if !enabled {
        item.state |= STATE_UNAVAILABLE;
    }
    item
}
```

- [ ] **Step 3: Expose the replace controls** in `src/window/search_view.rs`

1. Replace `enum SearchChild` with:

```rust
/// One of the Search view's MSAA children (see the view's `AccessibleView` impl).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SearchChild {
    Box,
    Toggle(SearchOption),
    Chevron,
    ReplaceField,
    ReplaceAll,
    Summary,
    Status,
    /// The replace button of the selected result `usize`.
    RowReplace(usize),
    Result(usize),
}
```

2. Replace `SearchView::head_children` with:

```rust
    /// The MSAA children before the results. The chevron comes after the toggles, so the box and
    /// the toggles keep the child IDs they had before replace existed.
    fn head_children(&self) -> Vec<SearchChild> {
        let mut head = Vec::with_capacity(10);
        if self.box_shown() {
            head.push(SearchChild::Box);
            head.extend(SearchOption::ALL.map(SearchChild::Toggle));
            head.push(SearchChild::Chevron);
            if self.replace_field_shown() {
                head.push(SearchChild::ReplaceField);
                head.push(SearchChild::ReplaceAll);
            }
        }
        let (summary, status) = self.shown_lines();
        if summary.is_some() {
            head.push(SearchChild::Summary);
        }
        if status.is_some() {
            head.push(SearchChild::Status);
        }
        if let Some(row) = self.row_replace_child() {
            head.push(SearchChild::RowReplace(row));
        }
        head
    }

    /// Whether the replace field shows. Read from its style, as `box_shown` is.
    fn replace_field_shown(&self) -> bool {
        self.replace_open
            && self.replace_edit.is_some_and(|edit| {
                (unsafe { GetWindowLongPtrW(edit, GWL_STYLE) }) as u32 & WS_VISIBLE != 0
            })
    }

    /// The selected result whose replace button is a child. There is one such child, the
    /// selected row's, so a screen reader user reaches it from the row they are on and the
    /// results keep their IDs as the selection moves.
    fn row_replace_child(&self) -> Option<usize> {
        let row = self.list.selected?;
        (self.replace_field_shown() && self.notice().is_none() && row < self.results.len())
            .then_some(row)
    }
```

3. In `accessible_item`'s `match`, after the `SearchChild::Toggle(option) => {...}` arm, add:

```rust
            SearchChild::Chevron => sidebar_accessibility::expander_item(
                "Toggle replace",
                self.replace_open,
                SearchView::chevron_rect(client, dpi),
            ),
            SearchChild::ReplaceField => {
                let replace = self.replace_edit?;
                // The kept text: a WM_GETTEXT here would run under the App borrow.
                sidebar_accessibility::field_item(
                    REPLACE_PLACEHOLDER,
                    self.replace_text.clone(),
                    unsafe { GetFocus() } == replace,
                    SearchView::replace_field_rect(client, dpi),
                    replace,
                )
            }
            SearchChild::ReplaceAll => sidebar_accessibility::action_item(
                "Replace all",
                self.replace_all_enabled(),
                SearchView::replace_all_rect(client, dpi),
            ),
            SearchChild::RowReplace(row) => {
                let hit = self.results.get(row)?;
                let (rect, visible) =
                    sidebar_accessibility::row_rect(self.list_area(client, dpi), &self.list, row);
                let mut item = sidebar_accessibility::action_item(
                    &format!("Replace in {}", hit.name),
                    self.replace_all_enabled(),
                    SearchView::row_replace_rect(rect, dpi),
                );
                if !visible {
                    item.state |= sidebar_accessibility::STATE_OFFSCREEN;
                }
                item
            }
```

4. Replace `accessible_hit` with:

```rust
    fn accessible_hit(&self, point: POINT, client: RECT, dpi: u32) -> Option<usize> {
        let field = SearchView::field_rect(client, dpi);
        let (summary, status) = self.shown_lines();
        let row_button = self.row_replace_child().filter(|&row| {
            let (rect, _) =
                sidebar_accessibility::row_rect(self.list_area(client, dpi), &self.list, row);
            inside(SearchView::row_replace_rect(rect, dpi), point)
        });
        let replace = self.replace_field_shown();
        let child = if self.box_shown() && inside(field, point) {
            option_toggles::hit(&option_toggles::toggle_rects(field, dpi), point)
                .map_or(SearchChild::Box, SearchChild::Toggle)
        } else if self.box_shown() && inside(SearchView::chevron_rect(client, dpi), point) {
            SearchChild::Chevron
        } else if replace && inside(SearchView::replace_all_rect(client, dpi), point) {
            SearchChild::ReplaceAll
        } else if replace && inside(SearchView::replace_field_rect(client, dpi), point) {
            SearchChild::ReplaceField
        } else if summary.is_some() && inside(self.summary_rect(client, dpi), point) {
            SearchChild::Summary
        } else if status.is_some() && inside(SearchView::status_rect(client, dpi), point) {
            SearchChild::Status
        } else if let Some(row) = row_button {
            SearchChild::RowReplace(row)
        } else {
            SearchChild::Result(self.row_under(point, client, dpi)?)
        };
        self.child_index(child)
    }
```

5. Replace `accessible_select` with:

```rust
    /// Selects a result. The row button's child scrolls its row into view, so its default action
    /// (a click on its center) lands on it.
    fn accessible_select(&mut self, index: usize, client: RECT, dpi: u32) {
        if let Some(SearchChild::Result(row) | SearchChild::RowReplace(row)) = self.child_at(index)
        {
            let area = self.list_area(client, dpi);
            self.list.select(row, area.bottom - area.top);
        }
    }
```

6. In `accessible_identity`'s `match`, after the `SearchChild::Toggle(option) => {...}` arm, add:

```rust
            SearchChild::Chevron => sidebar_accessibility::identity_of(&"replace toggle"),
            SearchChild::ReplaceField => sidebar_accessibility::identity_of(&"replace field"),
            SearchChild::ReplaceAll => sidebar_accessibility::identity_of(&"replace all"),
            SearchChild::RowReplace(row) => {
                sidebar_accessibility::identity_of(&("replace in", &self.results.get(row)?.path))
            }
```

7. Replace `announce_toggle` with:

```rust
/// Tells screen readers a toggle's checked state changed.
fn announce_toggle(hwnd: HWND, option: SearchOption) {
    announce_state(hwnd, SearchChild::Toggle(option));
}

/// Raises `EVENT_OBJECT_STATECHANGE` for `child`, if it is one of the view's children now: a
/// toggle, the chevron or Replace all. Raised with nothing of the App borrowed.
fn announce_state(hwnd: HWND, child: SearchChild) {
    if side_panel::current_view(hwnd) != SidebarView::Search {
        return;
    }
    if let Some((panel, index)) =
        with_view(hwnd, |view| Some((view.panel, view.child_index(child)?))).flatten()
    {
        sidebar_accessibility::notify(EVENT_OBJECT_STATECHANGE, panel, Some(index));
    }
}
```

8. In `set_replace_open`, replace

```rust
    let (panel, edit) = with_view(hwnd, |view| {
        if view.replace_open != open {
            view.replace_open = open;
            // The replace children come and go, and the rows move down or up.
            view.order = view.order.wrapping_add(1);
            view.row_hover_button = None;
            view.header_hover = None;
        }
        (view.panel, view.edit)
    })?;
```

with

```rust
    let (panel, edit, changed) = with_view(hwnd, |view| {
        let changed = view.replace_open != open;
        if changed {
            view.replace_open = open;
            // The replace children come and go, and the rows move down or up.
            view.order = view.order.wrapping_add(1);
            view.row_hover_button = None;
            view.header_hover = None;
        }
        (view.panel, view.edit, changed)
    })?;
```

and replace its last lines

```rust
    layout(hwnd);
    invalidate(panel);
    if open { replace } else { None }
```

with

```rust
    layout(hwnd);
    invalidate(panel);
    if changed {
        announce_state(hwnd, SearchChild::Chevron);
    }
    if open { replace } else { None }
```

9. Replace `begin_search` and `apply_batch` with:

```rust
/// `text_search_host::run_now` started a search for `query` over `total` notes. Replace all
/// waits for it, and says so.
pub(crate) fn begin_search(hwnd: HWND, query: &str, total: usize) {
    let Some((panel, flipped)) = with_view(hwnd, |view| {
        let enabled = view.replace_all_enabled();
        view.begin(query, total);
        (view.panel, enabled != view.replace_all_enabled())
    }) else {
        return;
    };
    invalidate(panel);
    if flipped {
        announce_state(hwnd, SearchChild::ReplaceAll);
    }
    announce_lines(hwnd, false);
}

/// A batch of the current search (`text_search_host::batch_arrived`). The panel repaints only if
/// something it shows changed. Replace all says when it became available.
pub(crate) fn apply_batch(hwnd: HWND, batch: SearchBatch) {
    let settled = batch.end.is_some();
    let Some((panel, enabled)) = with_view(hwnd, |view| (view.panel, view.replace_all_enabled()))
    else {
        return;
    };
    let (client, dpi) = geometry(panel);
    let (changed, flipped) = with_view(hwnd, |view| {
        let area = view.list_area(client, dpi);
        let changed = view.apply(batch, height(area));
        (changed, view.replace_all_enabled() != enabled)
    })
    .unwrap_or((false, false));
    if changed || flipped {
        invalidate(panel);
    }
    if flipped {
        announce_state(hwnd, SearchChild::ReplaceAll);
    }
    announce_lines(hwnd, settled);
}
```

Run: `cargo test --lib -- window::sidebar_accessibility window::search_view the_replace_controls_are_exposed the_search_view_exposes the_summary_speaks --test-threads=1`
Expected: all pass.

- [ ] **Step 4: Write the end-to-end test** in `tests/windows/library.rs`

1. Change `use support::process::{FastPadProcess, wait_and_cancel_dialog, wait_for_process_exit};` to:

```rust
use support::process::{
    FastPadProcess, wait_and_cancel_dialog, wait_and_dismiss_dialog, wait_for_process_exit,
};
```

2. In `struct AccessibleVtable`, replace `get_acc_state: usize,` with:

```rust
    get_acc_state: unsafe extern "system" fn(*mut c_void, VARIANT, *mut VARIANT) -> HRESULT,
```

3. In `impl Accessible`, after `role`, add:

```rust
    fn state(&self, child: i32) -> Option<u32> {
        let mut value = VARIANT::default();
        let result =
            unsafe { (self.vtable().get_acc_state)(self.0, child_variant(child), &mut value) };
        (result >= 0).then_some(unsafe { value.Anonymous.Anonymous.Anonymous.lVal } as u32)
    }
```

4. After `selection`, add:

```rust
/// `STATE_SYSTEM_UNAVAILABLE`: a button that can't be pressed now.
const STATE_SYSTEM_UNAVAILABLE: u32 = 0x0000_0001;

/// An Edit's text in another process. `WM_GETTEXT` is marshalled across processes;
/// `GetWindowTextW` would read an empty caption.
fn edit_text(edit: HWND) -> String {
    use windows_sys::Win32::UI::WindowsAndMessaging::{WM_GETTEXT, WM_GETTEXTLENGTH};
    let length = unsafe { SendMessageW(edit, WM_GETTEXTLENGTH, 0, 0) }.max(0) as usize;
    let mut text = vec![0_u16; length + 1];
    let copied = unsafe { SendMessageW(edit, WM_GETTEXT, text.len(), text.as_mut_ptr() as isize) }
        .max(0) as usize;
    String::from_utf16_lossy(&text[..copied.min(length)])
}

/// Whether the panel lists Replace all as a button that can be pressed now.
fn replace_all_available(panel: HWND) -> bool {
    Accessible::from_window(panel).is_some_and(|accessible| {
        accessible.children().iter().any(|(id, name)| {
            name == "Replace all"
                && accessible
                    .state(*id)
                    .is_some_and(|state| state & STATE_SYSTEM_UNAVAILABLE == 0)
        })
    })
}
```

5. At the end of the file, add:

```rust
#[test]
fn replacing_in_the_notebook_writes_the_closed_notes_and_changes_the_open_tab() {
    // Break caught: Ctrl+Shift+H not reaching the replace field in the real exe, Replace all
    // missing from what a screen reader sees or never available, the question never shown, a
    // closed note left unwritten or its LF endings changed, the open tab's file written instead
    // of its text changed in the editor, a note that didn't match touched, or the results left
    // showing notes with nothing to match.
    let _lock = LIBRARY_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _dpi = DpiContext::per_monitor_v2();
    let _com = ComApartment::initialize();
    let data = Scratch::new("text-replace");
    let a = data.note("a.md", "alpha invoice\r\n");
    let b = data.note("b.md", "invoice\ninvoice\n");
    let c = data.note("c.md", "nothing to see");
    let mut process =
        FastPadProcess::spawn_with_local_app_data([data.folder()], &data.root).unwrap();
    let hwnd = process.wait_for_main_window(WAIT).unwrap();
    let editor = find_child_by_class(hwnd, "Scintilla").unwrap();
    wait_for_library(&data);
    let panel = find_child_by_class(hwnd, SIDE_PANEL_CLASS).unwrap();
    forward(&data, &a);
    wait_until("a to open", || {
        scintilla_text(editor).is_ok_and(|text| text == "alpha invoice\r\n")
    });

    command(hwnd, CommandId::ShowSearchView);
    wait_until("the search box to take focus", || {
        focused_window(hwnd).is_ok_and(|focus| focus != editor && focus != panel)
    });
    let search_box = focused_window(hwnd).unwrap();
    for unit in "invoice".encode_utf16() {
        unsafe {
            PostMessageW(search_box, WM_CHAR, unit as usize, 0);
        }
    }
    wait_until("both results", || {
        panel_lists(panel, "a: alpha invoice") && panel_lists(panel, "b: invoice")
    });
    assert!(!panel_lists(panel, "c: nothing to see"));

    // Ctrl+Shift+H's command; the in-process tests pin the accelerator itself.
    command(hwnd, CommandId::ReplaceInNotes);
    wait_until("the replace field to take focus", || {
        focused_window(hwnd)
            .is_ok_and(|focus| focus != editor && focus != panel && focus != search_box)
    });
    let replace = focused_window(hwnd).unwrap();
    for unit in "bill".encode_utf16() {
        unsafe {
            PostMessageW(replace, WM_CHAR, unit as usize, 0);
        }
    }
    wait_until("the replacement", || edit_text(replace) == "bill");
    wait_until("Replace all to be available", || replace_all_available(panel));

    click_child(panel, "Replace all");
    // OK on "Replace 3 matches in 2 notes with "bill"?", with its line about b being saved.
    wait_and_dismiss_dialog(process.id(), WAIT).unwrap();
    wait_until("b to be written", || read(&b) == "bill\nbill\n");
    wait_until("a's tab to change in the editor", || {
        scintilla_text(editor).is_ok_and(|text| text == "alpha bill\r\n")
    });
    assert_eq!(read(&c), "nothing to see");
    wait_until("the search to run again", || {
        panel_lists(panel, "No notes match.")
    });
    // Closing autosaves a's tab (a note in the notebook), so no prompt stops the exit.
    close(process, hwnd);
}
```

Run: `cargo build`, then `cargo test --test library -- replacing_in_the_notebook searching_the_notebook --test-threads=1`
Expected: both pass. Close every FastPad window in the session first: the harness refuses to run otherwise.

- [ ] **Step 5: README** in `README.md`

1. In the Notes bullet list, after the **Ctrl+Shift+F** bullet, add:

```markdown
- **Ctrl+Shift+H** opens a replace field under the search. **Replace all** (or
  **Ctrl+Alt+Enter**) replaces in every listed note after asking, and each result has its own
  replace button. Notes open in tabs change in the editor, where one Ctrl+Z undoes it; the others
  are saved, skipping any that changed since the search. With regular expressions on, `$1`
  inserts a group, in the find bar's Replace too.
```

2. In the shortcut table, after the row `| Find next / previous | ... | Match case / whole word / regex | ... |`, add:

```markdown
| Replace in notes | `Ctrl+Shift+H` | | Replace all (in Search) | `Ctrl+Alt+Enter` |
```

- [ ] **Step 6: Implementation notes** in `docs/superpowers/specs/2026-09-24-note-search-design.md`

1. In §17, replace the bullet

```markdown
- **Replace in the find bar inserts its text literally,** in both modes (no `$1` or `\1`). A
  plain match's length is Scintilla's `SCI_GETTARGETEND`; a regex match's is the `Matcher`'s.
```

with

```markdown
- **Replace in the find bar** inserts its text literally in plain mode. Since 3b, regex mode
  expands `$1`, `${name}` and `$$` through `Matcher::replacements`, in Replace and in Replace
  all, and Enter replaces the selection only when it is one of `find_iter`'s matches over the
  document, not any `find_at` match (in "aaa" with `aa`, a selection of 1..3 isn't replaced). A
  plain match's length is Scintilla's `SCI_GETTARGETEND`; a regex match's is the `Matcher`'s.
  `\1` is never expanded.
```

2. At the end of §17, add:

```markdown
- **3b's layout.** The chevron sits left of the search box, where the box used to begin, so the
  box starts 16 px further right at 96 DPI. While the field is open, a 34 px replace row comes
  under the header, and the summary and the results move down by that much. Replace all and the
  rows' replace buttons share MDL2's Switch glyph (`E8AB`); their tooltips tell them apart
  ("Replace all (Ctrl+Alt+Enter)", "Replace").
- **The row button shows on the hovered and the selected row**, and only while the replace field
  is open (§11 says "hovering over or selecting"). Like Replace all, it runs only when a search
  has finished with results.
- **Ctrl+Alt+Enter and both buttons need the replace field open**, so a replace with an empty
  replacement never starts from the search box while the field is hidden. Enter in the replace
  field does nothing, Up goes back to the search box, and Esc clears the field (in an empty field
  it returns to the editor, as in the box). Alt+C, Alt+W and Alt+R work in the replace field too.
- **The replace text is kept at `EN_CHANGE`** (`SearchView.replace_text`), as the box's is, so
  neither the replace nor a screen reader sends `WM_GETTEXT` under the App borrow. It survives a
  notebook change; the query doesn't (§15).
- **A second Replace all while one runs is ignored.** The replace has its own generation and
  cancel flag, separate from the search's. A notebook change (`forget`) and `WM_DESTROY` cancel
  it. A write already running finishes its note, and its report is dropped.
- **A count that arrives inside a modal loop or a file population waits.** It is held, and
  `REPLACE_TIMER_ID` retries it every 50 ms, since the question is a modal loop of its own and a
  background tab is swapped into the editor. That is the guard the search's debounce uses.
- **The tabs are split again after the question.** A note opened in a tab while the question was
  up is changed in the editor, never written. A hit found in a tab's text whose tab has since
  closed has no stamp (`TextHit.stamp` is `None`), so it can't be checked: it is left out of the
  write and reported as changed since the search, not written.
- **A background tab is changed while swapped in with notifications suppressed**
  (`with_inactive_document`, which `read_inactive_text` now uses too). It is then marked dirty,
  given a new recovery generation and taken out of preview by hand (`Tabs::note_background_edit`).
  Autosave only saves the active tab, so a background tab's replacement waits, like any of its
  edits, for autosave on leaving or closing.
- **The library takes FastPad's writes without reading the disk on the UI thread.** After each
  save, `text_replace::apply` reads the saved file's metadata on the worker and returns it with
  the path in `ReplaceReport.written` (`Stamp::of`, a small addition to `text_search` that the
  search's `read_note` now uses too). `LibraryState::record_written` then makes the update
  `add_note` makes for a listed note (size, time, `online_only` cleared, `touched`) from that
  stamp. `ReplaceReport` has no separate `notes` list: the notes replaced into are `written`. A
  note whose metadata can't be read right after its save (it vanished in that moment) keeps its
  matches in the count but is left out of `written`, so the library isn't told (the next rescan
  takes the change as any outside one) and the report's note count misses it.
- **The count reads every open target tab's editor text,** clean or dirty, not only the dirty
  tabs' overlays that search uses, because the replace changes an open tab's text in the editor.
- **The stamp check and the write are not one step.** `apply` compares the stamp just before
  `save_atomic`; a change landing between the two isn't seen and is overwritten. A note deleted
  in exactly that moment is recreated, because `save_atomic` moves its temporary file into place
  with `MoveFileExW` when there is no file to replace. Both windows are accepted.
- **The report is one notification whose lines are joined on the status bar**, which is one line:
  `status_text` joins them with spaces. The names are still one per line in the message.
- **Accessibility:**
  - The chevron is a push button with the expanded or collapsed state ("Toggle replace").
  - The replace field is a text child whose full object is the Edit's own ("Replace").
  - Replace all is a push button that is unavailable while it can't run.
  - The row button is exposed once, for the selected row ("Replace in <name>"), before the results.
  - The chevron comes after the toggles, so the box and the toggles keep 3a's child IDs 1 to 4, and the summary line moves from child ID 5 to 6.
  - The chevron, and Replace all as it becomes available or unavailable, raise `EVENT_OBJECT_STATECHANGE`.
- **§13's `plan_replacements`** is `text_replace::count` in the code.
```

- [ ] **Step 7: Lint, format, and commit**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: no warnings.

Run: `cargo fmt --all`, then `cargo fmt --all -- --check`
Expected: the check prints nothing.

```bash
git add src/window/sidebar_accessibility.rs src/window/search_view.rs src/window/main_window.rs tests/windows/library.rs README.md docs/superpowers/specs/2026-09-24-note-search-design.md
git commit -m "feat(search): screen readers get the replace chevron (expanded or collapsed), the replace field, Replace all (unavailable until a search finishes) and the selected row's replace button; an end-to-end replace in the real exe, the README shortcuts and the 3b implementation notes"
```
