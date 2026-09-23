# Note Text Search (3a) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn the sidebar's Search view into a VS Code-style search of the text of every note in the open notebook, with match case, whole word and regex toggles shared with the editor's find bar, and Ctrl+Shift+F.

**Architecture:**
- **Pure code:**
  - `src/search/` (new) holds the `Matcher` (plain and regex matching with the three options) and the snippet cutter.
  - `src/library/text_search.rs` (new) is the worker loop. It reads each note, or its dirty tab's text, and streams `TextHit` batches to a sink.
- **Window code:**
  - `src/window/text_search_host.rs` (new) owns the debounce timer, the generation counter, the cancel flag and the worker thread, and turns posted batches into Search view updates.
  - `search_view.rs` switches from name matches to text hits, with two-line rows, a summary line and a status line.
  - `option_toggles.rs` (new) draws and hit-tests the three toggles. Both the Search field and the find bar use it.
- **The find bar** passes the toggles to Scintilla as search flags, and opening a Search result seeds it with the query and options.

**Tech Stack:** Rust 2024, `windows-sys` 0.61, Scintilla, GDI, hand-built MSAA. **One new crate:** `regex = "=1.13.1"`, default features. It's already in `C:\Users\korn3\.cargo\registry\src\`.

**Spec:** `docs/superpowers/specs/2026-09-24-note-search-design.md` (sections 1–10 and 14–16 for 3a; sections 11–13 are 3b and out of scope here). It builds on `docs/superpowers/specs/2026-09-23-note-sidebar-design.md`.

**Branch:** `feat/note-search`, stacked on `feat/note-sidebar` (PR #10). The PR targets `feat/note-sidebar`.

## Global Constraints

- **Scope:** 3a only. No replace UI, no `ReplaceInNotes` command and no `text_replace.rs`. Those are 3b, a later plan. `Matcher::find_iter` is still built and tested here (spec §6, "used by 3b").
- **Dependencies:** exactly one new crate, `regex = "=1.13.1"` (default features), in `[dependencies]` of `Cargo.toml`. `Cargo.lock` gains `regex`, `regex-automata`, `regex-syntax` and, if the crate pulls them in, `aho-corasick` and `memchr`.
- **Latency rules:**
  - Nothing new runs before first paint or first input.
  - The UI thread never reads note files for search.
  - A keystroke in the box only resets a 150 ms timer.
  - Handling one batch takes under 2 ms: a binary-search insert per hit and one `InvalidateRect`.
  - Nothing is kept between searches, beyond the displayed results (at most 500) and the narrowing record (the previous query, its options and its hit paths).
- **App-borrow rule (`app_ptr` contract):** never call `SetFocus`, `SetCapture`, `CreateWindowExW`, `UpdateWindow`, `SetWindowTextW` on a child, `SendMessageW` to another window, or `modal::*` while holding a `&mut` borrowed from App-derived state (`with_view`, `library_host::with_state`, `app_ptr(...).as_mut()`). Take the values you need, drop the borrow, then call.
- **Worker threads** never touch an `HWND` except by `PostMessageW` to the main window. A payload is `Box::into_raw`. If `PostMessageW` fails, the worker frees the box itself, as `library_host::spawn_load` does.
- **Values (spec, verbatim):**

  | Name | Value |
  |---|---|
  | Debounce | 150 ms |
  | Result cap | 500 notes (summary "500+ notes") |
  | Largest note searched | 4,194,304 bytes (larger is skipped as `TooLarge`) |
  | Batch size | 50 hits or 50 ms, whichever comes first, plus a final batch |
  | Snippet | at most 40 characters before the match and 80 after it, with `…` at each cut and leading white space removed. The 80 are counted from the match's start (Task 2; Task 9 records it in spec §17) |
  | Minimum query length | 2 characters (`chars().count()`). Shorter shows "Type at least 2 characters." |
  | Toggle shortcuts | Alt+C, Alt+W, Alt+R, active only while focus is in the Search box, the Search results or the find bar |
  | Show Search | Ctrl+Shift+F (Ctrl+K removed) |
  | Format JSON | Shift+Alt+F |
  | Find next / previous | F3 / Shift+F3 (new accelerators, Task 7) |

- **Wording (verbatim):**
  - Placeholder: "Search text in <notebook name>".
  - Summary: "1 note", "N notes" or "500+ notes". "N" uses a thousands separator.
  - "No notes match."
  - "Type at least 2 characters."
  - "Open a notebook to search it."
  - "Loading…"
  - While a search runs: "Searching… N of M".
  - Skipped notes: "1 note wasn't searched" or "N notes weren't searched". The tooltip has one line per non-zero reason: "N online only", "N larger than 4 MB", "N couldn't be read", "N not text".
  - "The pattern matches empty text."
  - "The pattern is too large." (a regex over the crate's size limit; not in the spec's list, so Task 9 records it in §17)
  - A regex syntax error shows the crate's description, capitalised, for example "Unclosed group".
  - Toggle names: "Match case", "Match whole word", "Use regular expression". Their tooltips add the shortcut, e.g. "Match case (Alt+C)".
- **`CommandId`:**
  - New discriminants start at 185: `SearchToggleCase = 185`, `SearchToggleWholeWord = 186`, `SearchToggleRegex = 187` (Task 6), then `FindNext = 188` and `FindPrevious = 189` (Task 7).
  - 3b's `ReplaceInNotes` starts at **190**.
  - The three toggles are palette-only, with no accelerators, and `is_sidebar()` is true for them. The palette labels are "Search: Toggle match case", "Search: Toggle whole word" and "Search: Toggle regular expression".
  - `FindNext` and `FindPrevious` need a document, are not sidebar commands, and have the accelerators F3 and Shift+F3, the Search-menu entries "Find &next\tF3" and "Find pre&vious\tShift+F3", and the palette rows "Search: Find next" and "Search: Find previous". Nothing binds F3 today.
  - Table lengths change by delta, whatever the table holds when a task starts: `COMMANDS` +3 (Task 6) and +2 (Task 7); `command_palette::ENTRIES` +3 and +2; `accelerator_specs()` +0 (Task 6: Ctrl+K goes, Shift+Alt+F comes) and +2 (Task 7).
- **Window message:** `WM_FASTPAD_TEXT_SEARCH_BATCH = WM_APP + 13`.
- **Timer:** `TEXT_SEARCH_TIMER_ID: usize = 0x4650_5453` on the **main window** (the same pattern as `LIBRARY_WRITE_TIMER_ID`), defined in `text_search_host.rs`. This deviates from spec §7, which said "on the panel, with the timer ID in `ids.rs`". `ids.rs` holds library IDs, and every other timer lives on the main window. Task 9 records the deviation in spec §17.
- **Notes mode off:** no Search view exists. Ctrl+Shift+F and the three toggle commands do nothing, because `is_sidebar()` commands are disabled. The find bar's toggles work in both modes.
- **Commits:** no attribution lines in commit messages. Never commit `native/out` or `target/`.
- **Tests:**
  - Compile with `cargo clippy --all-targets -- -D warnings`, and check formatting with `cargo fmt --all -- --check`.
  - Run only each task's targeted tests. The full suite runs once, at the final review.
  - A test command with more than one filter puts the filters after `--` (`cargo test --lib -- a b --test-threads=1`).
  - In-process window tests and `tests/windows/*` need `-- --test-threads=1`.
  - Tests never touch the real profile (`%LOCALAPPDATA%\FastPad`) or the real Documents folder. In-process tests already use a scratch profile under cfg(test). Real-exe tests use a scratch `LOCALAPPDATA` and a scratch notes folder. Back up and restore `fastpad.ini` and `folders.ini` around any run of the live app.
  - Never search the whole disk. Crate sources are in `C:\Users\korn3\.cargo\registry\src\`.
- **No backward compatibility:** nothing here changes a file format. If something would, no migration is written.
- **Temporary dead-code attributes:** an item written before its first production caller carries `#[cfg_attr(not(test), expect(dead_code, reason = "…"))]`, and the task that first calls it removes the attribute (a kept `expect` fails clippy as unfulfilled):
  - `search_view::toggle_option`: added in Task 4, removed in Task 5.
  - `search_view::show_with_query` and `search_view::options`: added in Task 4, both removed in Task 6. `options` is called only by `show_with_query`, so both go live together.
  - `option_toggles::label`: added in Task 5, removed in Task 8.

## Review Focus

1. **Typing fast while a search is streaming.** Each keystroke cancels the running worker. Batches from an older generation are dropped and freed, never shown. The list never flickers back to results from an older query. Pinned in Task 4 (`a_batch_from_an_older_generation_is_dropped`).
2. **A note that is open with unsaved edits.** The search matches the editor text, not the disk, both ways: a phrase only typed in the editor is found, and a phrase deleted in the editor but still on disk is not. Pinned in Task 3 (`an_overlay_wins_over_the_disk_both_ways`) and Task 4 (`a_dirty_tab_is_searched_as_the_editor_has_it`).
3. **Non-ASCII text and case folding.** Snippets are cut on character boundaries, never inside a UTF-8 sequence. The bold highlight covers exactly the matched text after folding (É/é, Ж/ж). A match at the very start or end of a line cuts cleanly. Pinned in Task 1 (`folding_maps_offsets_back_to_the_original_text`) and Task 2 (`a_cut_never_splits_a_multibyte_character`).
4. **Whole word next to `_`, digits and punctuation, in plain and regex mode.** `foo` matches `foo.` and `(foo)` but not `foo_bar` or `foo2`, and the same in regex mode. A later whole-word occurrence is found after an earlier partial one ("foobar foo"). Pinned in Task 1 (`whole_word_skips_a_partial_hit_and_finds_the_next`).
5. **Opening a result when the note's text changed since the search.** The find bar searches the live text. If the phrase is gone, the note still opens and the find bar shows its no-match state. Nothing panics on a stale snippet. Pinned in Task 7 (`opening_a_result_whose_text_changed_shows_no_match`).

## File Map

| File | Responsibility | Task |
|---|---|---|
| `Cargo.toml`, `Cargo.lock`, `LICENSES.md` | `regex = "=1.13.1"`; the four new locked crates listed | 1 |
| `src/search/mod.rs`, `src/search/matcher.rs` (new), `src/lib.rs` | `MatchOptions`, `SearchOption`, `Matcher`, `PatternError`, `escape` | 1 |
| `src/search/snippet.rs` (new) | `Snippet`, `cut`, `first_snippet`, `BEFORE_CHARS`, `AFTER_CHARS` | 2 |
| `src/library/text_search.rs` (new), `src/library/mod.rs`, `src/library/name_search.rs` | `SearchNote`, `Stamp`, `TextHit`, `SkipReason`, `Progress`, `RunEnd`, `run`, `hit_cmp`, constants; `NoteEntry.online_only` and everywhere `NoteEntry` is built; `folder_of` becomes `pub(super)` | 3 |
| `src/window/text_search_host.rs` (new), `src/window/{messages,mod,main_window,library_host}.rs`, `src/app.rs` | Debounce, generation, cancel, spawning, the batch message, overlays (`main_window::document_text`), narrowing record; library-change and reload hooks; cancel on notebook change, notes mode off and window destroy | 4 |
| `src/window/search_view.rs` | Results become `Vec<TextHit>`; `SearchState`; `begin_search`, `apply_batch`; notice, summary and status text; stops using `name_search` | 4 |
| `src/window/option_toggles.rs` (new) | Toggle geometry, painting, hit-testing, Alt-key mapping, labels, tooltips | 5 |
| `src/window/palette.rs` | `Palette.error_foreground` in every palette | 5 |
| `src/window/side_panel.rs` | `UiFonts.text_bold`; the Search tooltip destroyed with the panel; `WM_SYSKEYDOWN`/`WM_SYSCHAR` routed to the Search view | 5 |
| `src/window/tooltip.rs` | `Tooltip::set_max_width` | 5 |
| `src/window/search_view.rs`, `src/window/main_window.rs` (tests) | Two-line rows, bold highlight (`DT_EXPANDTABS`), toggles inside the field, summary line, pattern-error line, status line and its tooltip, Esc behavior, Alt keys, the box made in `shown` | 5 |
| `src/window/{commands,menus,command_palette,main_window,search_view}.rs`, `tests/windows/json_commands.rs`, `README.md`, `docs/superpowers/specs/2026-09-23-note-sidebar-design.md` | Ctrl+Shift+F with selection prefill, Format JSON on Shift+Alt+F, Ctrl+K removed, the three toggle commands and palette rows, sidebar commands off with notes mode off | 6 |
| `src/editor/scintilla_constants.rs`, `tools/generate-scintilla-constants.ps1`, `src/editor/scintilla.rs` | `SCFIND_REGEXP`, `SCFIND_CXX11REGEX`, `SCFIND_POSIX`, `SCI_GETTARGETEND`; a match's real length; Replace all stops at an empty match | 7 |
| `src/window/find_bar.rs`, `src/window/main_window.rs`, `src/window/search_view.rs` (`open_result` only), `src/window/{commands,menus,command_palette}.rs` | Find-bar toggles, flags, no-match state and tooltip; `show_with`; F3 and Shift+F3; opening a Search result seeds the find bar and selects the first match | 7 |
| `src/window/{sidebar_accessibility,search_view,option_toggles,find_bar,panel,main_window,text_search_host}.rs` | Toggles as check buttons, result names, the search box as an accessible child, the find bar's provider, rate-limited name-change events | 8 |
| `src/bin/fastpad-bench.rs`, `benchmarks/README.md`, `tests/windows/library.rs`, `README.md`, spec §17 | Bench cases (notebook under `target\bench-notes`), end-to-end test, exe size and memory checks, docs, implementation notes | 9 |

## Task Interfaces (the contract every task follows)

The names below are binding. A task may add private helpers, but it must not rename these.

- **Task 1: matcher** (`src/search/matcher.rs`, re-exported from `src/search/mod.rs` as `pub use matcher::{MatchOptions, Matcher, PatternError, SearchOption, escape};`; `pub mod search;` in `src/lib.rs`)
  - `#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)] pub struct MatchOptions { pub case: bool, pub whole_word: bool, pub regex: bool }`. `case: true` means match case. The default is all false.
  - `#[derive(Clone, Copy, Debug, Eq, PartialEq)] pub enum SearchOption { Case, WholeWord, Regex }`, plus `SearchOption::ALL: [SearchOption; 3]` in that order.
  - `impl MatchOptions { pub fn get(self, option: SearchOption) -> bool; pub fn toggled(self, option: SearchOption) -> MatchOptions }`.
  - `#[derive(Clone, Debug, Eq, PartialEq)] pub struct PatternError { pub message: String }`, with `impl Display` printing `message` and `impl std::error::Error`.
    - For a regex syntax error, `message` is the **last** non-empty line of the `regex::Error` text (`error: <description>`), with `error:` stripped, trimmed and capitalised: "Unclosed group".
    - `regex::Error::CompiledTooBig` gives "The pattern is too large."
    - An empty match gives "The pattern matches empty text."
  - `#[derive(Clone, Debug)] pub struct Matcher`, which is `Send + Sync`.
  - `pub fn new(query: &str, options: MatchOptions) -> Result<Matcher, PatternError>`. It fails on an invalid regex, and on any pattern (plain or regex) that matches `""`. An empty plain query is an empty-match error too. In regex mode the pattern is checked alone before it is wrapped as `\b(?:…)\b` for whole word.
  - `pub fn first_in(&self, text: &str) -> Option<Range<usize>>` gives the byte range in `text` of the first match. It honours the per-line rule of spec §6: a match never spans a `\n` unless the regex pattern contains `\n` (as a literal newline, or as the two characters `\` and `n`), in which case the whole text is matched. A trailing `\r` before `\n` is never part of a per-line match. It never returns an empty range.
  - `pub fn find_iter(&self, text: &str) -> Vec<Range<usize>>` gives every non-overlapping, non-empty match, in order, under the same rules.
  - `pub fn options(&self) -> MatchOptions` and `pub fn query(&self) -> &str`.
  - `pub fn escape(text: &str) -> String` is a **free function**, `crate::search::escape`, a thin wrapper over `regex::escape`. It is not an associated function of `Matcher`.
- **Task 2: snippet** (`src/search/snippet.rs`, re-exported as `pub use snippet::{AFTER_CHARS, BEFORE_CHARS, Snippet, cut, first_snippet};`)
  - `#[derive(Clone, Debug, Default, Eq, PartialEq)] pub struct Snippet { pub text: String, pub highlight: Range<usize> }`. `highlight` is a byte range within `text`, always on char boundaries.
  - `pub const BEFORE_CHARS: usize = 40; pub const AFTER_CHARS: usize = 80;`
  - `pub fn cut(line: &str, hit: Range<usize>) -> Snippet`. `hit` is a byte range within `line`. It removes leading white space (never past the hit's start) and keeps at most `BEFORE_CHARS` characters before the hit and `AFTER_CHARS` characters **counted from the hit's start**, with `…` at each cut. A hit longer than `AFTER_CHARS` characters is itself cut and ends with `…`.
  - `pub fn first_snippet(text: &str, matcher: &Matcher) -> Option<Snippet>` finds the first match and cuts the line it starts on, without its `\r`. For a whole-text regex match that spans lines, the highlight ends at the end of that first line. Snippet text is the raw line and can contain tabs.
- **Task 3: worker** (`src/library/text_search.rs`, `pub mod text_search;` in `src/library/mod.rs`)
  - `NoteEntry` gains `pub online_only: bool`: the scan's flag in `load`, `false` from `add_note` (a save), the rescan's flag in `merge_notes`, and kept by `rename_note`.
  - `pub const RESULT_CAP: usize = 500; pub const MAX_NOTE_BYTES: u64 = 4 * 1024 * 1024; pub const BATCH_HITS: usize = 50; pub const BATCH_INTERVAL: Duration = Duration::from_millis(50);`
  - `#[derive(Clone, Debug, Eq, PartialEq)] pub struct SearchNote { pub path: PathBuf, pub size: u64, pub online_only: bool }`, where `path` is relative to the notebook, and `impl From<&NoteEntry> for SearchNote`.
  - `#[derive(Clone, Copy, Debug, Eq, PartialEq)] pub struct Stamp { pub size: u64, pub mtime: u64 }`. `mtime` is FILETIME ticks from the opened file's metadata (`std::os::windows::fs::MetadataExt::last_write_time`).
  - `#[derive(Clone, Debug, Eq, PartialEq)] pub struct TextHit { pub path: PathBuf, pub name: String, pub folder: String, pub snippet: Snippet, pub stamp: Option<Stamp> }`.
    - `name` is the file stem.
    - `folder` is `name_search::folder_of(path)` (now `pub(super)`): the parent path joined with `\`, and `""` at the root.
    - `stamp` is `None` when the text came from an overlay.
  - `#[derive(Clone, Copy, Debug, Eq, PartialEq)] pub enum SkipReason { OnlineOnly, TooLarge, Unreadable, NotText }`, with `SkipReason::ALL: [SkipReason; 4]` in that order and `pub fn index(self) -> usize`.
  - `#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)] pub struct Progress { pub visited: usize, pub total: usize, pub skipped: [usize; 4] }`, with `pub fn skipped_total(&self) -> usize`. `total` is the length of the `notes` slice passed to `run`.
  - `#[derive(Clone, Copy, Debug, Eq, PartialEq)] pub enum RunEnd { Completed, Capped, Cancelled }`.
  - `pub fn hit_cmp(a: &TextHit, b: &TextHit) -> Ordering` orders by `natural_cmp(name)`, then by `natural_cmp(folder)` with `""` first, then by path.
  - `pub fn run(notebook: &Path, notes: &[SearchNote], overlays: &HashMap<PathBuf, String>, matcher: &Matcher, cancel: &AtomicBool, sink: &mut dyn FnMut(Vec<TextHit>, Progress)) -> RunEnd`.
    - It visits `notes` in order. Overlay keys are relative paths as produced by `library::record_path`, compared with note paths as lowercased `to_string_lossy()` (the rule of `library::model::same_path`).
    - **An overlay beats every skip reason:** a dirty tab's text is searched even for an online-only or oversized note, is never counted as skipped, and its hit has `stamp: None`.
    - The sink receives a batch at `BATCH_HITS` hits or when `BATCH_INTERVAL` has elapsed since the last batch (checked after each note). An interval batch can have **no hits** and carry only progress. A final batch goes out at the end even if empty.
    - The sink is never told which batch is the last; the host adds that (Task 4).
    - At `RESULT_CAP` hits it sends the capping batch and returns `Capped`, **only if notes remain**. A 500th hit on the last note is `Completed`.
    - It checks `cancel` (`Ordering::Relaxed`) before each note and before the final batch. Once cancelled, it sends nothing more and returns `Cancelled`.
- **Task 4: host** (`src/window/text_search_host.rs`)
  - `pub(crate) const TEXT_SEARCH_TIMER_ID: usize = 0x4650_5453; pub(crate) const DEBOUNCE_MS: u32 = 150; pub(crate) const MIN_QUERY_CHARS: usize = 2;`
  - `#[derive(Debug)] pub(crate) struct SearchBatch { pub generation: u64, pub hits: Vec<TextHit>, pub progress: Progress, pub end: Option<RunEnd> }` is the boxed `WM_FASTPAD_TEXT_SEARCH_BATCH` payload (`lparam`). Its fields are `pub`, so tests build it.
    - **The worker wraps every sink batch as `end: None`.** After `run` returns `Completed` or `Capped` it posts one more batch with no hits, `end: Some(end)` and the last progress. After `Cancelled` it posts nothing. (It also skips that final post when its cancel flag was set, which only happens when a post failed or the UI cancelled; such a batch would be stale anyway.)
  - `#[derive(Debug, Default)] pub(crate) struct TextSearchHost { generation: u64, cancel: Option<Arc<AtomicBool>>, previous: Option<Narrowing>, running: Option<Running>, list: Option<u64> }` is stored as `App.text_search: TextSearchHost`.
    - Private `Narrowing { query: String, options: MatchOptions, paths: Vec<PathBuf>, list: u64, overlays: u64 }` is recorded only after `RunEnd::Completed`. `list` is `list_mark` of the note list (paths and `online_only` flags, not sizes or times) and `overlays` is `overlay_mark` of the dirty-tab texts.
  - `pub(crate) fn searchable(query: &str) -> bool`: at least `MIN_QUERY_CHARS` characters and not all white space.
  - `pub(crate) fn schedule(hwnd: HWND)`: a keystroke. It cancels any running search and (re)starts the debounce timer.
  - `pub(crate) fn run_now(hwnd: HWND)`: cancels, kills the timer, and starts the search for the view's current query and options. It is used for toggles, Ctrl+Shift+F prefill, library changes and the timer.
    - Order: a query that isn't `searchable` runs nothing; **then the pattern is compiled**, and a `PatternError` calls `search_view::set_pattern_error(hwnd, Some(message))` and runs nothing, even with no notebook or while loading; then it does nothing with no notebook, or while the library isn't loaded (the query runs on `LIBRARY_READY` through `search_view::library_changed`).
    - **Narrowing:** `run_now` uses `previous.paths` as the note list when `!options.regex && !options.whole_word`, the options are equal, `query.contains(&previous.query)`, the dirty-tab texts are unchanged (`overlays`) and the note list is unchanged (`list`). Otherwise it uses the library's full note list.
  - `pub(crate) fn timer(hwnd: HWND)`: the main window's `WM_TIMER` arm for `TEXT_SEARCH_TIMER_ID` calls it, and it calls `run_now`.
  - `pub(crate) fn batch_arrived(hwnd: HWND, lparam: LPARAM)`: takes ownership of the box. A stale generation is dropped. Otherwise it forwards to `search_view::apply_batch` and, on `end == Some(Completed)`, records `Narrowing` from `search_view::result_paths`.
  - `pub(crate) fn cancel(hwnd: HWND)`: sets the flag, **bumps the generation** (so batches a cancelled run already posted are stale) and kills the timer. It is called on window destroy, when a query becomes too short, and by `schedule`, `run_now` and `forget`.
  - `pub(crate) fn forget(hwnd: HWND)`: `cancel` plus dropping the narrowing record and the list mark. Called on a notebook change or close (`search_view::library_changed`) and on notes mode off (`library_host::notes_mode_changed`).
  - `pub(crate) fn notes_reloaded(hwnd: HWND)`: every `library_host::install` (load or rescan) drops the narrowing record and the list mark, so the next library change runs the query again.
  - `pub(crate) fn library_changed(hwnd: HWND)`: the shown Search view's library changed. It re-runs the query **only** when the list mark changed since the last search (a note added, removed or renamed, or an `online_only` flag changed) or after a reload. A save of a listed note runs nothing.
  - `pub(crate) fn dirty_overlays(hwnd: HWND, notebook: &Path) -> HashMap<PathBuf, String>`: the dirty tabs whose path is inside `notebook` (`library::is_inside`), keyed by `library::record_path(notebook, path)`, with the editor text from `main_window::document_text`. It is built on the UI thread just before spawning, with nothing of the App borrowed.
  - Test helpers (`#[cfg(test)]`): `generation(hwnd) -> u64`, `cancel_flag(hwnd) -> Option<Arc<AtomicBool>>`, `test_batch(generation, hits, end) -> LPARAM`.
- **Task 4: `main_window`**: `pub(crate) fn document_text(hwnd: HWND, id: DocumentId) -> Option<String>`, the tab's text as the editor has it (a background tab through `read_inactive_text`; `None` while a file is being populated). The test helper `type_into_search(hwnd, text)` stays, plus `search_for`, `wait_for_search`, `search_generation`, `search_rows`, `search_row`, `search_state`, `search_selected`, `selected_name`, `stray_hit`, `searched_total`.
- **Task 4: search_view data API** (`src/window/search_view.rs`)
  - `SearchView.results: Vec<TextHit>`, kept sorted by `hit_cmp`. `SearchView.options: MatchOptions`. `SearchView.search: SearchState`. `SearchView.query: String`.
  - `#[derive(Clone, Debug, Default, Eq, PartialEq)] pub(crate) enum SearchState { #[default] Idle, TooShort, Running(Progress), Done { progress: Progress, capped: bool }, PatternError(String) }`.
  - Constants `NO_NOTEBOOK`, `NO_MATCH`, `LOADING`, `TOO_SHORT` (`pub(crate)`, the wording above).
  - Free functions: `pub(crate) fn notice_text(notebook_open: bool, loaded: bool, failed: bool) -> Option<&'static str>`, `pub(crate) fn summary_text(state: &SearchState, results: usize) -> Option<(String, bool)>` (the line and whether it is an error), `pub(crate) fn status_text(state: &SearchState) -> Option<String>`.
  - Methods: `SearchView::notice(&self) -> Option<&'static str>`, `SearchView::summary(&self) -> Option<(String, bool)>`, `SearchView::status_line(&self) -> Option<String>`. Private `SearchView::begin(&mut self, query, total)` and `SearchView::apply(&mut self, batch, height) -> bool` (whether anything painted changed) are what the unit tests call.
  - `pub(crate) fn current_query(hwnd: HWND) -> Option<(String, MatchOptions)>`: the box text and options, or `None` if there's no box yet.
  - `pub(crate) fn begin_search(hwnd: HWND, query: &str, total: usize)`: a new query clears the results but remembers the selected path to restore it. The same query run again keeps the results until the first batch replaces them. The state becomes `Running`.
  - `pub(crate) fn apply_batch(hwnd: HWND, batch: SearchBatch)`: no generation check (that is `batch_arrived`'s), so tests call it directly. It inserts each hit with `binary_search_by(hit_cmp)`, handles batches with no hits, keeps the selection by path, updates the state, and invalidates the panel only if something visible changed. The first batch of a re-run replaces the old results.
  - `pub(crate) fn set_pattern_error(hwnd: HWND, error: Option<String>)`.
  - `pub(crate) fn options(hwnd: HWND) -> MatchOptions`.
  - `pub(crate) fn toggle_option(hwnd: HWND, option: SearchOption)`: flips it and calls `text_search_host::run_now`.
  - `pub(crate) fn show_with_query(hwnd: HWND, text: &str)`: sets the box text (escaped first with `crate::search::escape` when regex is on), selects it all and runs at once. It does not show the view. It is used by Ctrl+Shift+F.
  - `pub(crate) fn result_paths(hwnd: HWND) -> Vec<PathBuf>`.
  - `query_changed` (`EN_CHANGE`) calls `text_search_host::schedule` for a searchable query, and otherwise `cancel` plus the `TooShort` or `Idle` state. `library_changed` keeps its notebook-change logic (calling `forget`), and otherwise calls `text_search_host::library_changed` when the view shows, or marks the view stale.
  - Test helpers (`#[cfg(test)]`): `edit_hwnd(hwnd)` (kept), `shown_results(hwnd) -> Vec<(String, String)>` (name, snippet text), `status(hwnd)` (now the notice), `summary(hwnd) -> Option<(String, bool)>`, `search_state(hwnd) -> SearchState`.
- **Task 5: toggles** (`src/window/option_toggles.rs`)
  - `pub(crate) fn toggle_rects(field: RECT, dpi: u32) -> [RECT; 3]`: three square buttons inside the field's right end, in `SearchOption::ALL` order, 22 px each with 2 px gaps and 3 px right padding at 96 DPI, vertically centred.
  - `pub(crate) fn reserved_width(dpi: u32) -> i32`: how much the EDIT must stop short of the field's right edge.
  - `pub(crate) fn paint(hdc: HDC, rects: &[RECT; 3], options: MatchOptions, hover: Option<SearchOption>, palette: &Palette, font: HFONT)`. The glyphs are text "Aa", "ab" (underlined with a 1 px line) and ".*". An on toggle is filled with `palette.selection_background`, the colour FastPad outlines focused fields with (the palette has no accent), and its glyph uses `selection_foreground` (or `editor_foreground` when that is `None`). A hovered off toggle gets `hover_background` and `hover_foreground`. An off toggle's glyph is `muted_foreground`.
  - `pub(crate) fn hit(rects: &[RECT; 3], point: POINT) -> Option<SearchOption>`.
  - `pub(crate) fn alt_key(vk: u32) -> Option<SearchOption>` maps `C`, `W` and `R`.
  - `pub(crate) fn label(option: SearchOption) -> &'static str` and `pub(crate) fn tooltip(option: SearchOption) -> &'static str`.
- **Task 5: elsewhere**
  - `Palette.error_foreground: u32` (`#A1260D` light, `#F48771` dark, the flavor's `red` for Catppuccin, window text in high contrast). The Search view's pattern error and the find bar's no-match outline both use it.
  - `UiFonts.text_bold: HFONT` (Segoe UI bold, 12 px at 96 DPI) and `Tooltip::set_max_width(&self, width: i32)`.
  - `SearchView::summary_rect(client: RECT, dpi: u32) -> RECT` and `SearchView::status_rect(client: RECT, dpi: u32) -> RECT` (associated functions), `SearchView::field_rect(client, dpi)` (existing), `SearchView::list_area(&self, client, dpi)`, `SearchView::destroy_tooltip(&self)`.
  - `pub(crate) fn skipped_tooltip(progress: &Progress) -> String` in `search_view.rs`.
  - Rows are 42 px at 96 DPI (`ROW_AT_96_DPI`), two 18 px line slots; the Search view's scroll thumb becomes 9 px wide as `row_list` scales it from the row height. The snippet is painted with `DT_EXPANDTABS`.
  - `search_view::shown` makes the box (user input), even with no notebook; `layout` makes it only with a notebook open.
- **Task 6: commands**
  - `CommandId::SearchToggleCase = 185`, `SearchToggleWholeWord = 186` and `SearchToggleRegex = 187`, and `pub const fn search_option(self) -> Option<SearchOption>`. They call `search_view::toggle_option`, showing the Search view first if it's hidden.
  - `ShowSearchView` takes the active editor's selection through the private `main_window::single_line_selection(hwnd) -> Option<String>`. If it's non-empty and has no `\n` or `\r`, it shows the view and calls `search_view::show_with_query`.
  - `execute_command` returns early for any `is_sidebar()` command while notes mode is off.
- **Task 7: find bar**
  - `CommandId::FindNext = 188` and `CommandId::FindPrevious = 189` (F3 and Shift+F3).
  - `Editor::search_in_target` returns the match's real end (`SCI_GETTARGETEND`); a negative result (-1 or -2) is `Ok(None)`. `Editor::replace_all` stops at an empty match.
  - `FindBar.options: MatchOptions` and `FindBar::{options, toggle_option, no_match, set_no_match, toggle_rects, wants_tooltip, set_tooltip, toggle_tools}`. `panel_hwnd` is no longer `cfg(test)`.
  - `pub(crate) fn search_flags(options: MatchOptions) -> u32`, `pub(crate) fn scintilla_query(query: &str, options: MatchOptions) -> String` (whole word in regex mode wraps the pattern as `\b(?:…)\b`, because Scintilla's regex search ignores the word flags) and `pub(crate) fn escape_pattern(text: &str) -> String` (ECMAScript escaping for a prefill while regex is on) in `find_bar.rs`, plus `BarClick { Close, Toggle(SearchOption) }` and `PendingText` (`apply(self)`).
  - `pub(crate) fn show(&mut self, mode: FindBarMode, prefill: Option<&str>, colors: Palette) -> Option<PendingText>` and `pub(crate) fn show_with(&mut self, mode: FindBarMode, query: &str, options: MatchOptions, colors: Palette) -> PendingText`. The caller applies the `PendingText` after dropping the App borrow, because `SetWindowTextW` on the field sends `EN_CHANGE` back to the main window.
  - `pub(crate) fn toggle_option(&mut self, option: SearchOption)`.
  - `main_window::open_search_result(hwnd: HWND, relative: &Path, mode: OpenMode, focus_editor: bool)` opens the note with `open_note`, shows the find bar with Search's query and options, and selects the first match from position 0. A click passes `focus_editor: true` and Enter passes `false`. `search_view::open_result` (used by `open_selected` and the click path) calls it instead of `open_note`.
  - `main_window::toggle_find_option(hwnd, option)` (pub(crate)), and `main_window::find_bar_owns` becomes `pub(crate)`.
  - The no-match state outlines the query field in `Palette.error_foreground`, until the query changes or a search finds a match.
- **Task 8: accessibility.** Additions to `sidebar_accessibility.rs`'s item model: `AccessibleItem.window: HWND`, `STATE_CHECKED`, `STATE_READONLY`, `check_item`, `field_item`, `text_item`, and a `get_accChild` that returns a window child's own object. Also `find_bar::{FIND_BAR_ACCESSIBLE, toggle_child}`, `FindBar::accessible_items`, and `search_view::{result_name, announce_lines, Spoken}`. The Search view's accessible children read `SearchView::{notice, summary, status_line}` (Task 4) and `SearchView::{summary_rect, status_rect}` (Task 5) with exactly those signatures.

---

### Task 1: The matcher and the `regex` dependency

**Files:**
- Modify: `Cargo.toml` (`[dependencies]`, after the `pulldown-cmark = { version = "=0.13.4", default-features = false }` line)
- Modify: `Cargo.lock` (Cargo adds `regex` 1.13.1, `regex-automata` 0.4.18, `regex-syntax` 0.8.11 and `aho-corasick` 1.1.5; `memchr` 2.8.3 is already locked)
- Modify: `src/lib.rs` (add `pub mod search;` after `pub mod recovery;`)
- Modify: `LICENSES.md` (the "Rust crates" section lists every locked crate; it gains the four new ones)
- Create: `src/search/mod.rs`
- Create: `src/search/matcher.rs`

**Interfaces:**
- Consumes: `regex` 1.13.1: `RegexBuilder::{new, case_insensitive, unicode, build}`, `Regex::{find_at, find_iter, is_match}`, `regex::escape`, `regex::Error::{Syntax, CompiledTooBig}` (checked in `regex-1.13.1/src/error.rs` and `builders.rs`).
- Produces (`crate::search`, re-exported from `src/search/mod.rs`):
  - `MatchOptions { pub case: bool, pub whole_word: bool, pub regex: bool }` (`Clone, Copy, Debug, Default, Eq, PartialEq`), with `get(self, SearchOption) -> bool` and `toggled(self, SearchOption) -> MatchOptions`
  - `SearchOption { Case, WholeWord, Regex }` and `SearchOption::ALL: [SearchOption; 3]` in that order
  - `PatternError { pub message: String }` (`Clone, Debug, Eq, PartialEq`, `Display` prints `message`, also `std::error::Error`)
  - `Matcher` (`Clone, Debug`, `Send + Sync`): `new(&str, MatchOptions) -> Result<Matcher, PatternError>`, `first_in(&self, &str) -> Option<Range<usize>>`, `find_iter(&self, &str) -> Vec<Range<usize>>`, `options(&self) -> MatchOptions`, `query(&self) -> &str`
  - `escape(text: &str) -> String`

**Decisions this task settles** (the contract leaves them to Task 1):
- **Plain matching** runs a literal regex (`regex::escape` of the query), which uses the crate's SIMD literal search. With match case, it runs on the text as it is.
  - Without match case, an ASCII query also runs directly, as an ASCII-only case-insensitive literal (`unicode(false)`), unless the text contains U+212A KELVIN SIGN. That's the one non-ASCII character whose single-character lowercase is ASCII. The test `only_the_kelvin_sign_folds_into_ascii` checks every `char` to prove it.
  - Otherwise (a non-ASCII query, or a text with the Kelvin sign) the text is folded once per call, one character for one, with `to_lowercase` when that gives a single character. A `Walker` maps folded byte offsets back to the original text. When no character changed its UTF-8 length, the offsets are the same and no walk happens.
  - Nothing allocates per line on the common path. A plain query without `\r` or `\n` is searched over the whole text in one pass. That gives the same matches as per-line search, because such a query can't span a line break or take in a line's `\r`, and a line break is never a word character.
- **Whole word, plain mode:** the characters just before and after the match must not be `char::is_alphanumeric()` or `_`. The check is on the original text. On a failed check, the search goes on from the character after the match's start.
- **Regex mode:** `RegexBuilder` with `case_insensitive(!case)`. The pattern is compiled alone first, which reports syntax errors against what the user typed, and is rejected if it `is_match("")`. With whole word it's then compiled again as `\b(?:…)\b`.
  - Checking the unwrapped pattern matters. `a*` wrapped no longer matches `""`, and `a)|(b` becomes valid once wrapped.
  - Matching is per line unless the pattern contains a newline or the two characters `\n`.
  - Empty matches that get past the check (`\b` matches no `""`, but it does match between characters) are skipped. `first_in` and `find_iter` never return an empty range.
- **`PatternError` text:** `regex::Error`'s `Display` for a syntax error is several lines: `regex parse error:`, the pattern, a caret line, and then `error: <description>` as the **last** line (see `regex-syntax-0.8.11/src/error.rs`, `Formatter::fmt`). The message is that last non-empty line, with `error:` stripped, trimmed and capitalised: "Unclosed group". `CompiledTooBig` becomes "The pattern is too large." An empty match gives "The pattern matches empty text."

- [ ] **Step 1: Add the dependency and the module**

In `Cargo.toml`, under `[dependencies]`, after the `pulldown-cmark` line:

```toml
regex = "=1.13.1"
```

In `src/lib.rs`, after `pub mod recovery;`:

```rust
pub mod search;
```

Create `src/search/mod.rs`:

```rust
//! Matching note text for Search: the matcher. Pure.

pub mod matcher;

pub use matcher::{MatchOptions, Matcher, PatternError, SearchOption, escape};
```

- [ ] **Step 2: Write the failing tests** at the bottom of `src/search/matcher.rs`

```rust
#[cfg(test)]
// Expected matches are byte ranges; a one-range list is not a mistaken `(a..b).collect()`.
#[allow(clippy::single_range_in_vec_init)]
mod tests {
    use super::*;

    fn options(case: bool, whole_word: bool, regex: bool) -> MatchOptions {
        MatchOptions {
            case,
            whole_word,
            regex,
        }
    }

    fn matcher(query: &str, options: MatchOptions) -> Matcher {
        Matcher::new(query, options).unwrap()
    }

    fn error(query: &str, options: MatchOptions) -> String {
        Matcher::new(query, options).unwrap_err().message
    }

    #[test]
    fn every_combination_of_the_three_options_matches_as_described() {
        // Break caught: one option silently ignored in one mode, e.g. match case dropped in regex
        // mode or whole word only applied to plain text.
        let text = "Foo foo_bar food (foo) FOO.";
        let any_case = vec![0..3, 4..7, 12..15, 18..21, 23..26];
        let exact_case = vec![4..7, 12..15, 18..21];
        let whole_words = vec![0..3, 18..21, 23..26];
        let exact_whole_word = vec![18..21];
        for regex in [false, true] {
            let cases = [
                (options(false, false, regex), &any_case),
                (options(true, false, regex), &exact_case),
                (options(false, true, regex), &whole_words),
                (options(true, true, regex), &exact_whole_word),
            ];
            for (options, expected) in cases {
                let found = matcher("foo", options);
                assert_eq!(&found.find_iter(text), expected, "{options:?}");
                assert_eq!(
                    found.first_in(text),
                    expected.first().cloned(),
                    "{options:?}"
                );
            }
        }
        let pattern = matcher(r"fo+d?", options(false, false, true));
        assert_eq!(pattern.first_in("xx FOOOD"), Some(3..8));
        let pattern = matcher(r"fo+d?", options(true, false, true));
        assert_eq!(pattern.first_in("xx FOOOD"), None);
    }

    #[test]
    fn folding_matches_letters_with_a_single_character_lowercase() {
        // Break caught: case-insensitive search that only folds ASCII, so "école" misses "ÉCOLE".
        let any_case = MatchOptions::default();
        let text = "ÉCOLE école ЖУК жук";
        assert_eq!(matcher("école", any_case).find_iter(text), [0..6, 7..13]);
        assert_eq!(matcher("жук", any_case).find_iter(text), [14..20, 21..27]);
        assert_eq!(
            matcher("école", options(true, false, false)).find_iter(text),
            [7..13]
        );
        // `ß` has no one-character lowercase other than itself; it is not "ss".
        assert_eq!(matcher("ss", any_case).first_in("Straße"), None);
        assert_eq!(matcher("STRASSE", any_case).first_in("Straße"), None);
        assert_eq!(matcher("ẞ", any_case).first_in("Straße"), Some(4..6));
    }

    #[test]
    fn folding_maps_offsets_back_to_the_original_text() {
        // Break caught: a highlight shifted by the bytes folding added or removed. The Kelvin sign
        // (3 bytes) folds to `k` (1 byte) and the Ohm sign (3 bytes) to `ω` (2 bytes), so every
        // offset after them differs between the folded and the original text.
        let any_case = MatchOptions::default();
        let text = "\u{212A}\u{212A}\u{212A} \u{2126}hm KELVIN ohm";
        let kkk = matcher("kkk", any_case).first_in(text).unwrap();
        assert_eq!(&text[kkk], "\u{212A}\u{212A}\u{212A}");
        let ohm = matcher("ωHM", any_case).first_in(text).unwrap();
        assert_eq!(&text[ohm], "\u{2126}hm");
        let kelvin = matcher("kelvin", any_case).first_in(text).unwrap();
        assert_eq!(&text[kelvin], "KELVIN");
        let every_k = matcher("k", any_case).find_iter(text);
        assert_eq!(every_k, [0..3, 3..6, 6..9, 16..17]);
        // Whole word walks the same mapping: the first hit is followed by `_`, the second isn't.
        let text = "\u{212A}elvin_x \u{212A}ELVIN.";
        let found = matcher("kelvin", options(false, true, false)).find_iter(text);
        assert_eq!(found.len(), 1);
        assert_eq!(&text[found[0].clone()], "\u{212A}ELVIN");
        // Start and end of the text.
        let text = "Élan … ÉLAN";
        assert_eq!(
            matcher("élan", any_case).find_iter(text),
            [0..5, text.len() - 5..text.len()]
        );
    }

    #[test]
    fn only_the_kelvin_sign_folds_into_ascii() {
        // Break caught: a future Unicode version adding a character whose lowercase is ASCII, which
        // the ASCII fast path would then miss.
        let folding_into_ascii: Vec<char> = (0..=char::MAX as u32)
            .filter_map(char::from_u32)
            .filter(|c| !c.is_ascii() && fold_char(*c).is_ascii())
            .collect();
        assert_eq!(folding_into_ascii, [KELVIN_SIGN]);
    }

    #[test]
    fn whole_word_skips_a_partial_hit_and_finds_the_next() {
        // Break caught: whole word giving up after the first partial hit, so "foobar foo" never
        // finds the "foo" at its end.
        for regex in [false, true] {
            for case in [false, true] {
                let found = matcher("foo", options(case, true, regex));
                assert_eq!(found.first_in("foobar foo"), Some(7..10), "regex {regex}");
                assert_eq!(found.find_iter("foofoo foo xfoo foo"), [7..10, 16..19]);
            }
        }
        let found = matcher("école", options(false, true, false));
        assert_eq!(found.first_in("ÉCOLEs École"), Some(8..14));
    }

    #[test]
    fn whole_word_looks_at_underscores_digits_letters_and_punctuation_on_both_sides() {
        // Break caught: `_` or a digit counted as a word break, so whole word "foo" matched
        // "foo_bar" or "foo2".
        for regex in [false, true] {
            let found = matcher("foo", options(false, true, regex));
            for text in ["foo", "foo.", "(foo)", "a foo, b", "-foo-", "\"foo\""] {
                assert!(found.first_in(text).is_some(), "{text:?}, regex {regex}");
            }
            for text in [
                "foo_bar", "bar_foo", "foo2", "2foo", "éfoo", "fooé", "_foo_",
            ] {
                assert_eq!(found.first_in(text), None, "{text:?}, regex {regex}");
            }
        }
    }

    #[test]
    fn a_regex_error_shows_the_crates_description_with_a_capital() {
        // Break caught: the multi-line "regex parse error:" block with its caret shown under the
        // search box instead of a short message.
        let regex = options(false, false, true);
        assert_eq!(error("(abc", regex), "Unclosed group");
        assert_eq!(error("a)", regex), "Unopened group");
        assert_eq!(error("[a", regex), "Unclosed character class");
        assert_eq!(error("*a", regex), "Repetition operator missing expression");
        assert_eq!(error(r"\p{Nope}", regex), "Unicode property not found");
        assert_eq!(
            error("(a\nb", regex),
            "Unclosed group",
            "a pattern with a newline"
        );
        assert_eq!(
            error("a)|(b", options(false, true, true)),
            "Unopened group",
            "the pattern is checked before it is wrapped for whole word"
        );
        assert_eq!(error(r"(?:\w{100}){200}", regex), TOO_LARGE);
        let shown = Matcher::new("(abc", regex).unwrap_err();
        assert_eq!(shown.to_string(), "Unclosed group");
    }

    #[test]
    fn a_pattern_that_matches_empty_text_is_rejected() {
        // Break caught: `a*` accepted, giving an empty "match" in every note.
        for query in ["", "a*", "(?:)", "x?", "^", "$", "a|"] {
            assert_eq!(
                error(query, options(false, false, true)),
                EMPTY_MATCH,
                "{query:?}"
            );
        }
        assert_eq!(
            error("a*", options(false, true, true)),
            EMPTY_MATCH,
            "whole word does not hide an empty match"
        );
        assert_eq!(error("", MatchOptions::default()), EMPTY_MATCH);
        // `\b` matches no "" but does match empty text between characters: it compiles, and its
        // empty matches are never reported.
        let boundary = matcher(r"\b", options(false, false, true));
        assert_eq!(boundary.first_in("foo bar"), None);
        assert!(boundary.find_iter("foo bar").is_empty());
    }

    #[test]
    fn matches_never_span_lines_or_take_in_a_lines_carriage_return() {
        // Break caught: `foo\s+bar` matching across a line break, or `\s+$` highlighting the `\r`
        // of a CRLF line.
        let regex = options(false, false, true);
        assert_eq!(matcher(r"foo\s*bar", regex).first_in("foo\r\nbar"), None);
        assert_eq!(matcher(r"o$", regex).first_in("foo\r\nbar"), Some(2..3));
        assert_eq!(matcher(r"\s+$", regex).first_in("foo  \r\nx"), Some(3..5));
        assert_eq!(matcher(r"^bar", regex).first_in("foo\r\nbar"), Some(5..8));
        assert_eq!(matcher(r".$", regex).first_in("ab\r\ncd\r"), Some(1..2));
        assert_eq!(matcher(r"\r", regex).first_in("a\r\nb"), None);
        assert_eq!(matcher(r"\r", regex).first_in("a\rb"), Some(1..2));
        let plain = MatchOptions::default();
        assert_eq!(matcher("foo\r", plain).first_in("foo\r\n"), None);
        assert_eq!(matcher("foo\r", plain).first_in("foo\rx"), Some(0..4));
        assert_eq!(matcher("o\nb", plain).first_in("foo\nbar"), None);
        assert_eq!(
            matcher("bar", plain).find_iter("bar\r\nbar\nBAR"),
            [0..3, 5..8, 9..12]
        );
    }

    #[test]
    fn a_regex_that_names_a_newline_is_matched_against_the_whole_text() {
        // Break caught: `foo\nbar` never matching because every line was searched on its own.
        let regex = options(false, false, true);
        let text = "x foo\nbar y";
        assert_eq!(matcher(r"foo\nbar", regex).first_in(text), Some(2..9));
        assert_eq!(matcher("foo\nbar", regex).first_in(text), Some(2..9));
        assert_eq!(
            matcher(r"foo\r\nbar", regex).first_in("foo\r\nbar"),
            Some(0..8)
        );
        assert_eq!(
            matcher(r"o\nb", regex).find_iter("oo\nbb o\nb"),
            [1..4, 6..9]
        );
    }

    #[test]
    fn the_query_is_matched_as_typed_including_its_spaces() {
        let plain = MatchOptions::default();
        assert_eq!(matcher(" foo", plain).first_in("foo  foo"), Some(4..8));
        assert_eq!(matcher(" foo", plain).first_in("foo"), None);
        let found = matcher("a.b", plain);
        assert_eq!(
            found.first_in("axb a.b"),
            Some(4..7),
            "plain text is not a pattern"
        );
        assert_eq!(found.query(), "a.b");
        assert_eq!(found.options(), plain);
    }

    #[test]
    fn escape_makes_text_match_literally_in_regex_mode() {
        assert_eq!(escape("a.b*(c)"), r"a\.b\*\(c\)");
        let found = matcher(&escape("a.b"), options(false, false, true));
        assert_eq!(found.first_in("axb a.b"), Some(4..7));
    }

    #[test]
    fn options_toggle_one_at_a_time() {
        let none = MatchOptions::default();
        assert_eq!(none, options(false, false, false));
        assert_eq!(
            SearchOption::ALL,
            [
                SearchOption::Case,
                SearchOption::WholeWord,
                SearchOption::Regex
            ]
        );
        for option in SearchOption::ALL {
            let on = none.toggled(option);
            assert!(on.get(option));
            let others = SearchOption::ALL.iter().filter(|other| **other != option);
            for other in others {
                assert!(!on.get(*other), "{option:?} also set {other:?}");
            }
            assert_eq!(on.toggled(option), none);
        }
    }

    #[test]
    fn a_matcher_can_be_shared_with_the_search_thread() {
        fn shareable<T: Send + Sync + Clone>() {}
        shareable::<Matcher>();
    }

    #[test]
    fn forty_megabytes_of_text_are_searched_quickly() {
        // Break caught: per-line folding or allocation on the common path (an ASCII query without
        // match case), which would put a 10,000-note search past its 400 ms budget.
        let note = "Plain text with a café, some numbers 12345 and words.\r\n".repeat(75);
        let found = matcher("invoice march", MatchOptions::default());
        let started = std::time::Instant::now();
        for _ in 0..10_000 {
            assert_eq!(found.first_in(&note), None);
        }
        let elapsed = started.elapsed();
        if !cfg!(debug_assertions) {
            assert!(
                elapsed < std::time::Duration::from_millis(100),
                "{elapsed:?}"
            );
        }
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test --lib search::matcher`
Expected: a compile error in `src/search/matcher.rs`, because `Matcher`, `MatchOptions`, `SearchOption`, `escape`, `fold_char`, `KELVIN_SIGN`, `EMPTY_MATCH` and `TOO_LARGE` don't exist yet. Cargo has already locked the four new crates.

If Cargo tries to reach the network for the new crate, add `--offline`. All four crates are in `C:\Users\korn3\.cargo\registry\src\index.crates.io-1949cf8c6b5b557f\`.

- [ ] **Step 4: Write the implementation** at the top of `src/search/matcher.rs`, above the tests

```rust
//! Text matching for the note search: a plain phrase or a regex, with match case and whole word,
//! one line at a time. Pure: no Win32 and no disk.

use regex::{Regex, RegexBuilder};
use std::fmt;
use std::ops::Range;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MatchOptions {
    /// Match case. Off, both sides are folded one character at a time.
    pub case: bool,
    pub whole_word: bool,
    pub regex: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SearchOption {
    Case,
    WholeWord,
    Regex,
}

impl SearchOption {
    /// In the order the toggles are drawn.
    pub const ALL: [SearchOption; 3] = [Self::Case, Self::WholeWord, Self::Regex];
}

impl MatchOptions {
    pub fn get(self, option: SearchOption) -> bool {
        match option {
            SearchOption::Case => self.case,
            SearchOption::WholeWord => self.whole_word,
            SearchOption::Regex => self.regex,
        }
    }

    pub fn toggled(mut self, option: SearchOption) -> MatchOptions {
        match option {
            SearchOption::Case => self.case = !self.case,
            SearchOption::WholeWord => self.whole_word = !self.whole_word,
            SearchOption::Regex => self.regex = !self.regex,
        }
        self
    }
}

/// Why a query can't run, worded for the line under the search box.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PatternError {
    pub message: String,
}

const EMPTY_MATCH: &str = "The pattern matches empty text.";
const TOO_LARGE: &str = "The pattern is too large.";

impl PatternError {
    fn empty_match() -> PatternError {
        PatternError {
            message: EMPTY_MATCH.to_owned(),
        }
    }

    /// The description from the crate's message. A syntax error reads "regex parse error:", the
    /// pattern with a caret under the error, then "error: <description>" on the last line; only
    /// that description is kept, with a capital first letter ("Unclosed group").
    fn from_regex(error: &regex::Error) -> PatternError {
        let message = match error {
            regex::Error::CompiledTooBig(_) => TOO_LARGE.to_owned(),
            other => {
                let text = other.to_string();
                let last = text
                    .lines()
                    .map(str::trim)
                    .rfind(|line| !line.is_empty())
                    .unwrap_or_default();
                capitalized(last.strip_prefix("error:").map_or(last, str::trim))
            }
        };
        PatternError { message }
    }
}

impl fmt::Display for PatternError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for PatternError {}

fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// The one character outside ASCII whose single-character lowercase is ASCII (U+212A → `k`).
/// A test proves no other character does.
const KELVIN_SIGN: char = '\u{212A}';

#[derive(Clone, Debug)]
pub struct Matcher {
    query: String,
    options: MatchOptions,
    engine: Engine,
    /// Whether the text is matched one line at a time. Off, the whole text is one haystack: for a
    /// regex that names `\n`, and for a plain phrase without `\r` or `\n`, which can't span lines
    /// or take in a line's `\r` anyway, so one search over the whole text gives the same matches.
    per_line: bool,
}

#[derive(Clone, Debug)]
enum Engine {
    /// A literal phrase. `direct` runs on the text as it is: always with match case, and without
    /// it for an ASCII phrase on a text without the Kelvin sign, where folding changes nothing but
    /// ASCII letters (it is then ASCII case-insensitive). `folded` runs on the folded text; it is
    /// set only without match case.
    Plain {
        direct: Option<Regex>,
        folded: Option<Regex>,
    },
    Regex(Regex),
}

impl Matcher {
    pub fn new(query: &str, options: MatchOptions) -> Result<Matcher, PatternError> {
        let (engine, per_line) = if options.regex {
            regex_engine(query, options)?
        } else {
            plain_engine(query, options)?
        };
        Ok(Matcher {
            query: query.to_owned(),
            options,
            engine,
            per_line,
        })
    }

    pub fn options(&self) -> MatchOptions {
        self.options
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    /// The byte range of the first match in `text`.
    pub fn first_in(&self, text: &str) -> Option<Range<usize>> {
        let mut first = None;
        self.each_match(text, &mut |range| {
            first = Some(range);
            false
        });
        first
    }

    /// Every match in `text`, in order, none overlapping.
    pub fn find_iter(&self, text: &str) -> Vec<Range<usize>> {
        let mut all = Vec::new();
        self.each_match(text, &mut |range| {
            all.push(range);
            true
        });
        all
    }

    /// Calls `emit` with each match until it returns false.
    fn each_match(&self, text: &str, emit: &mut dyn FnMut(Range<usize>) -> bool) {
        if !self.per_line {
            self.segment_matches(text, 0, emit);
            return;
        }
        let mut start = 0;
        for line in text.split('\n') {
            let body = line.strip_suffix('\r').unwrap_or(line);
            if !self.segment_matches(body, start, emit) {
                return;
            }
            start += line.len() + 1;
        }
    }

    /// The matches in `segment`, which starts at byte `base` of the text. Returns false once
    /// `emit` asks to stop.
    fn segment_matches(
        &self,
        segment: &str,
        base: usize,
        emit: &mut dyn FnMut(Range<usize>) -> bool,
    ) -> bool {
        let mut shifted = |range: Range<usize>| emit(range.start + base..range.end + base);
        match &self.engine {
            Engine::Regex(regex) => {
                for found in regex.find_iter(segment) {
                    // A pattern that passed the empty check can still match empty text at a
                    // position (`\b`); those are never matches.
                    if !found.is_empty() && !shifted(found.range()) {
                        return false;
                    }
                }
                true
            }
            Engine::Plain { direct, folded } => {
                let whole_word = self.options.whole_word;
                if let Some(direct) = direct
                    && (self.options.case || !segment.contains(KELVIN_SIGN))
                {
                    let mut walker = Walker::new(segment, true);
                    return literal_matches(
                        direct,
                        segment,
                        segment,
                        whole_word,
                        &mut walker,
                        &mut shifted,
                    );
                }
                let Some(folded) = folded else {
                    return true;
                };
                let (folded_text, same_lengths) = fold(segment);
                let mut walker = Walker::new(segment, same_lengths);
                literal_matches(
                    folded,
                    &folded_text,
                    segment,
                    whole_word,
                    &mut walker,
                    &mut shifted,
                )
            }
        }
    }
}

/// `regex::escape`, for text that must match as it is in regex mode.
pub fn escape(text: &str) -> String {
    regex::escape(text)
}

fn plain_engine(query: &str, options: MatchOptions) -> Result<(Engine, bool), PatternError> {
    if query.is_empty() {
        return Err(PatternError::empty_match());
    }
    let per_line = query.contains(['\r', '\n']);
    let engine = if options.case {
        Engine::Plain {
            direct: Some(literal(query, false)?),
            folded: None,
        }
    } else {
        let direct = if query.is_ascii() {
            Some(literal(query, true)?)
        } else {
            None
        };
        Engine::Plain {
            direct,
            folded: Some(literal(&fold(query).0, false)?),
        }
    };
    Ok((engine, per_line))
}

fn regex_engine(query: &str, options: MatchOptions) -> Result<(Engine, bool), PatternError> {
    // The pattern is checked alone first: wrapped for whole word, `a)|(b` would compile, and
    // `a*` would no longer match "".
    let inner = build(query, !options.case)?;
    if inner.is_match("") {
        return Err(PatternError::empty_match());
    }
    let regex = if options.whole_word {
        build(&format!(r"\b(?:{query})\b"), !options.case)?
    } else {
        inner
    };
    let per_line = !(query.contains('\n') || query.contains(r"\n"));
    Ok((Engine::Regex(regex), per_line))
}

fn build(pattern: &str, case_insensitive: bool) -> Result<Regex, PatternError> {
    RegexBuilder::new(pattern)
        .case_insensitive(case_insensitive)
        .build()
        .map_err(|error| PatternError::from_regex(&error))
}

/// A regex matching `text` literally. `ascii_case_insensitive` folds ASCII letters only: the
/// crate's Unicode folding would also match `ſ` for `s`, which `to_lowercase` folding doesn't.
fn literal(text: &str, ascii_case_insensitive: bool) -> Result<Regex, PatternError> {
    RegexBuilder::new(&regex::escape(text))
        .case_insensitive(ascii_case_insensitive)
        .unicode(!ascii_case_insensitive)
        .build()
        .map_err(|error| PatternError::from_regex(&error))
}

/// `c` lowercased when that gives one character, else `c` itself.
fn fold_char(c: char) -> char {
    if c.is_ascii() {
        return c.to_ascii_lowercase();
    }
    let mut lower = c.to_lowercase();
    match (lower.next(), lower.next()) {
        (Some(single), None) => single,
        _ => c,
    }
}

/// `text` folded one character for one, and whether every character kept its UTF-8 length (so
/// byte offsets in the folded text are offsets in `text`).
fn fold(text: &str) -> (String, bool) {
    let mut folded = String::with_capacity(text.len());
    let mut same_lengths = true;
    for c in text.chars() {
        let lower = fold_char(c);
        same_lengths &= lower.len_utf8() == c.len_utf8();
        folded.push(lower);
    }
    (folded, same_lengths)
}

/// Maps byte offsets in the folded text back to `original`, walking both forward from the last
/// offset it was moved to. Folding is one character for one, so every folded character boundary
/// is an original one.
struct Walker<'a> {
    original: &'a str,
    identity: bool,
    folded_at: usize,
    original_at: usize,
}

impl<'a> Walker<'a> {
    fn new(original: &'a str, identity: bool) -> Self {
        Walker {
            original,
            identity,
            folded_at: 0,
            original_at: 0,
        }
    }

    fn walk(&self, target: usize) -> (usize, usize) {
        debug_assert!(target >= self.folded_at);
        let (mut folded_at, mut original_at) = (self.folded_at, self.original_at);
        for c in self.original[original_at..].chars() {
            if folded_at >= target {
                break;
            }
            folded_at += fold_char(c).len_utf8();
            original_at += c.len_utf8();
        }
        debug_assert_eq!(folded_at, target);
        (folded_at, original_at)
    }

    /// The original offset of `target`, which becomes the new starting point.
    fn seek(&mut self, target: usize) -> usize {
        if self.identity {
            return target;
        }
        (self.folded_at, self.original_at) = self.walk(target);
        self.original_at
    }

    /// The original offset of `target`, leaving the starting point where it is.
    fn peek(&self, target: usize) -> usize {
        if self.identity {
            target
        } else {
            self.walk(target).1
        }
    }
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Whether neither the character before `range` nor the one after it is a word character.
fn at_word_boundaries(text: &str, range: &Range<usize>) -> bool {
    let before = text[..range.start].chars().next_back();
    let after = text[range.end..].chars().next();
    !before.is_some_and(is_word) && !after.is_some_and(is_word)
}

/// The literal's matches in `haystack` (the text, or its folding), mapped to `original`. With
/// whole word, a match next to a word character is passed over and the search goes on from the
/// character after its start, so "foobar foo" still finds the second "foo".
fn literal_matches(
    regex: &Regex,
    haystack: &str,
    original: &str,
    whole_word: bool,
    walker: &mut Walker<'_>,
    emit: &mut dyn FnMut(Range<usize>) -> bool,
) -> bool {
    let mut from = 0;
    while let Some(found) = regex.find_at(haystack, from) {
        let start = walker.seek(found.start());
        let end = walker.peek(found.end());
        if !whole_word || at_word_boundaries(original, &(start..end)) {
            if !emit(start..end) {
                return false;
            }
            walker.seek(found.end());
            from = found.end();
        } else {
            from = found.start()
                + haystack[found.start()..]
                    .chars()
                    .next()
                    .map_or(1, char::len_utf8);
        }
    }
    true
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --lib search::matcher`
Expected: all 15 tests pass. `only_the_kelvin_sign_folds_into_ascii` walks every `char`, which takes well under a second in a debug build.

`forty_megabytes_of_text_are_searched_quickly` asserts its 100 ms budget only in release builds. This machine measured about 12 ms in release. It isn't part of the per-task run.

Run: `git diff --stat Cargo.lock`
Expected: only additions: the `aho-corasick`, `regex`, `regex-automata` and `regex-syntax` packages, and `"regex"` in `fastpad`'s dependency list. `core.autocrlf` is on and the index holds LF, so Cargo writing LF adds no line-ending noise.

- [ ] **Step 6: List the new crates in `LICENSES.md`**

Replace the sentence under "## Rust crates":

```markdown
FastPad depends directly on windows-sys 0.61.2, serde_json 1.0.151, pulldown-cmark 0.13.4, windows
0.62.2, and windows-numerics 0.3.1. The complete locked dependency closure, with the license
expressions reported by `cargo metadata --locked`, is:
```

with:

```markdown
FastPad depends directly on windows-sys 0.61.2, serde_json 1.0.151, pulldown-cmark 0.13.4, regex
1.13.1, windows 0.62.2, and windows-numerics 0.3.1. The complete locked dependency closure, with the
license expressions reported by `cargo metadata --locked`, is:
```

Change the `memchr` row's role from "`serde_json` dependency (linked)" to "`serde_json` and `regex` dependency (linked)". Then add these rows after the `unicase` row:

```markdown
| `regex` | 1.13.1 | MIT OR Apache-2.0 | Note text search (linked) |
| `regex-automata` | 0.4.18 | MIT OR Apache-2.0 | `regex` dependency (linked) |
| `regex-syntax` | 0.8.11 | MIT OR Apache-2.0 | `regex` dependency (linked) |
| `aho-corasick` | 1.1.5 | Unlicense OR MIT | `regex` dependency (linked) |
```

- [ ] **Step 7: Lint and format**

Run: `cargo clippy --all-targets -- -D warnings` and `cargo fmt --all -- --check`
Expected: clean. The test module's `#[allow(clippy::single_range_in_vec_init)]` is needed: `vec![18..21]` is an expected list of byte ranges, not a mistaken `(18..21).collect()`.

- [ ] **Step 8: Commit**

```bash
git add Cargo.toml Cargo.lock LICENSES.md src/lib.rs src/search/mod.rs src/search/matcher.rs
git commit -m "feat(search): a matcher for plain and regex text with match case and whole word"
```

---

### Task 2: The snippet

**Files:**
- Modify: `src/search/mod.rs` (Task 1)
- Create: `src/search/snippet.rs`

**Interfaces:**
- Consumes: `Matcher::first_in` (Task 1).
- Produces (`crate::search`, re-exported):
  - `Snippet { pub text: String, pub highlight: Range<usize> }` (`Clone, Debug, Default, Eq, PartialEq`); `highlight` is a byte range in `text` on char boundaries
  - `pub const BEFORE_CHARS: usize = 40; pub const AFTER_CHARS: usize = 80;`
  - `cut(line: &str, hit: Range<usize>) -> Snippet`
  - `first_snippet(text: &str, matcher: &Matcher) -> Option<Snippet>`

**Decisions this task settles:**
- **`AFTER_CHARS` is counted from the start of the hit.** The match and the text after it together are at most 80 characters. That is what "a hit longer than `AFTER_CHARS` characters is itself cut" implies, and it keeps every snippet to at most 1 + 40 + 80 + 1 characters. Task 9 records this reading of spec §7 ("80 characters after it") in spec §17.
- **Leading white space is removed, but never past the hit's start.** A search for " foo" keeps the space it matched.
- **Cutting is O(limits), not O(line):** it walks 40 characters back and 80 forward with `char_indices`, and never counts the whole line. A hit range that isn't on char boundaries (it can't come from `Matcher`, but the function is public) is moved back onto them instead of panicking.
- **`first_snippet`** cuts the line the match starts on, without its trailing `\r`.
  - A match that runs over several lines (a regex naming `\n`) is highlighted to the end of that first line.
  - A match that starts with line breaks (`\n\d+`) starts on the line after them.
  - A match of line breaks only shows the line before them, with an empty highlight at its end.

- [ ] **Step 1: Export the module**

Replace the whole of `src/search/mod.rs` with:

```rust
//! Matching note text for Search: the matcher and the snippet shown under each result. Pure.

pub mod matcher;
pub mod snippet;

pub use matcher::{MatchOptions, Matcher, PatternError, SearchOption, escape};
pub use snippet::{AFTER_CHARS, BEFORE_CHARS, Snippet, cut, first_snippet};
```

- [ ] **Step 2: Write the failing tests** at the bottom of `src/search/snippet.rs`

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::MatchOptions;

    fn cut_at(line: &str, needle: &str) -> Snippet {
        let start = line.find(needle).unwrap();
        cut(line, start..start + needle.len())
    }

    fn highlighted(snippet: &Snippet) -> &str {
        &snippet.text[snippet.highlight.clone()]
    }

    #[test]
    fn a_short_line_is_kept_whole() {
        let snippet = cut_at("paid the invoice march 3", "invoice");
        assert_eq!(snippet.text, "paid the invoice march 3");
        assert_eq!(snippet.highlight, 9..16);
        assert_eq!(highlighted(&snippet), "invoice");
    }

    #[test]
    fn leading_white_space_is_removed_but_never_past_the_hit() {
        // Break caught: an indented line showing as a blank-looking row, or a search for " foo"
        // losing the space it matched.
        let snippet = cut_at("\t   - send invoice", "invoice");
        assert_eq!(snippet.text, "- send invoice");
        assert_eq!(highlighted(&snippet), "invoice");
        let snippet = cut("    foo", 2..7);
        assert_eq!(snippet.text, "  foo");
        assert_eq!(snippet.highlight, 0..5);
    }

    #[test]
    fn a_long_line_is_cut_before_and_after_the_hit() {
        let before = "b".repeat(100);
        let after = "a".repeat(200);
        let line = format!("{before}HIT{after}");
        let snippet = cut_at(&line, "HIT");
        let expected = format!("…{}HIT{}…", "b".repeat(40), "a".repeat(77));
        assert_eq!(snippet.text, expected);
        assert_eq!(highlighted(&snippet), "HIT");
        assert_eq!(snippet.highlight.start, '…'.len_utf8() + 40);
    }

    #[test]
    fn exactly_the_limits_are_kept_without_an_ellipsis() {
        // Break caught: an off-by-one that adds `…` though nothing was cut, or drops a character.
        let line = format!("{}HIT{}", "b".repeat(40), "a".repeat(77));
        assert_eq!(cut_at(&line, "HIT").text, line);
        let line = format!("{}HIT{}", "b".repeat(41), "a".repeat(78));
        let snippet = cut_at(&line, "HIT");
        assert!(snippet.text.starts_with('…') && snippet.text.ends_with('…'));
        assert_eq!(snippet.text.chars().count(), 1 + 40 + 80 + 1);
    }

    #[test]
    fn a_hit_at_the_very_start_or_end_of_the_line_cuts_cleanly() {
        let snippet = cut_at("invoice sent", "invoice");
        assert_eq!(
            (snippet.text.as_str(), snippet.highlight.clone()),
            ("invoice sent", 0..7)
        );
        let snippet = cut_at("sent the invoice", "invoice");
        assert_eq!(snippet.highlight, 9..16);
        assert_eq!(snippet.highlight.end, snippet.text.len());
        let line = format!("{}invoice", "x".repeat(60));
        let snippet = cut_at(&line, "invoice");
        assert_eq!(snippet.text, format!("…{}invoice", "x".repeat(40)));
        assert_eq!(snippet.highlight.end, snippet.text.len());
    }

    #[test]
    fn a_hit_longer_than_the_after_limit_is_cut_itself() {
        let line = format!("ab {} cd", "h".repeat(100));
        let start = 3;
        let snippet = cut(&line, start..start + 100);
        assert_eq!(snippet.text, format!("ab {}…", "h".repeat(80)));
        assert_eq!(highlighted(&snippet), "h".repeat(80));
    }

    #[test]
    fn a_cut_never_splits_a_multibyte_character() {
        // Break caught: counting bytes instead of characters, which panics slicing inside `é` or
        // an emoji, or shows 40 bytes (20 characters) of Cyrillic context.
        let line = format!("{}Жук{}", "é😀".repeat(30), "ж".repeat(100));
        let snippet = cut_at(&line, "Жук");
        assert_eq!(highlighted(&snippet), "Жук");
        let before: String = snippet.text[..snippet.highlight.start].chars().collect();
        assert_eq!(before.chars().count(), 1 + 40);
        assert!(before.starts_with('…'));
        let after = &snippet.text[snippet.highlight.end..];
        assert_eq!(after.chars().count(), 77 + 1);
        assert!(after.ends_with('…'));
        // A range that is not on character boundaries is moved back onto them, never panicking.
        let snippet = cut("éé", 1..3);
        assert_eq!(snippet.text, "éé");
        assert_eq!(highlighted(&snippet), "é");
    }

    #[test]
    fn the_first_snippet_is_the_line_of_the_first_match_without_its_carriage_return() {
        let matcher = Matcher::new("invoice", MatchOptions::default()).unwrap();
        let text = "Notes\r\n\r\n    paid the INVOICE\r\nlater invoice\r\n";
        let snippet = first_snippet(text, &matcher).unwrap();
        assert_eq!(snippet.text, "paid the INVOICE");
        assert_eq!(highlighted(&snippet), "INVOICE");
        assert_eq!(first_snippet("nothing here", &matcher), None);
        let at_end = first_snippet("x\ninvoice", &matcher).unwrap();
        assert_eq!(at_end.text, "invoice");
    }

    #[test]
    fn a_folded_match_highlights_the_original_characters() {
        let matcher = Matcher::new("école", MatchOptions::default()).unwrap();
        let snippet = first_snippet("\u{212A} at the ÉCOLE today", &matcher).unwrap();
        assert_eq!(highlighted(&snippet), "ÉCOLE");
    }

    #[test]
    fn a_match_over_several_lines_is_highlighted_to_the_end_of_its_first_line() {
        let regex = MatchOptions {
            regex: true,
            ..MatchOptions::default()
        };
        let matcher = Matcher::new(r"march\r?\n\d+", regex).unwrap();
        let snippet = first_snippet("intro\r\npaid in march\r\n12 days", &matcher).unwrap();
        assert_eq!(snippet.text, "paid in march");
        assert_eq!(highlighted(&snippet), "march");
        // A match that starts with the line break starts on the next line.
        let matcher = Matcher::new(r"\n\d+", regex).unwrap();
        let snippet = first_snippet("paid\n12 days", &matcher).unwrap();
        assert_eq!(snippet.text, "12 days");
        assert_eq!(highlighted(&snippet), "12");
        // A match of line breaks alone shows the line before them, highlighting nothing.
        let matcher = Matcher::new(r"\n\n", regex).unwrap();
        let snippet = first_snippet("a\n\nb", &matcher).unwrap();
        assert_eq!(snippet.text, "a");
        assert_eq!(snippet.highlight, 1..1);
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test --lib search::snippet`
Expected: a compile error, because `Snippet`, `cut` and `first_snippet` don't exist (and `mod.rs` re-exports them).

- [ ] **Step 4: Write the implementation** at the top of `src/search/snippet.rs`

```rust
//! The line shown under a Search result: its first match with a little context either side, cut
//! on character boundaries so painting never measures a long line. Pure.

use super::matcher::Matcher;
use std::ops::Range;

/// At most this many characters are kept before the match.
pub const BEFORE_CHARS: usize = 40;
/// At most this many characters are kept from the start of the match: the match and what follows.
pub const AFTER_CHARS: usize = 80;
const ELLIPSIS: char = '…';

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Snippet {
    pub text: String,
    /// The match, as a byte range within `text`, on character boundaries.
    pub highlight: Range<usize>,
}

/// `line` cut around `hit`, a byte range within it: leading white space removed (never past the
/// hit), at most `BEFORE_CHARS` characters before the hit and `AFTER_CHARS` from its start, with
/// `…` at each cut. A hit longer than `AFTER_CHARS` characters is cut itself and ends with `…`.
/// Costs a walk of those few characters, whatever the line's length.
pub fn cut(line: &str, hit: Range<usize>) -> Snippet {
    let end = floor_boundary(line, hit.end.min(line.len()));
    let start = floor_boundary(line, hit.start.min(end));
    let lead = line.len() - line.trim_start().len();
    let before = &line[lead.min(start)..start];
    let keep_from = before
        .char_indices()
        .rev()
        .nth(BEFORE_CHARS - 1)
        .map_or(0, |(at, _)| at);
    let mut text = String::new();
    if keep_from > 0 {
        text.push(ELLIPSIS);
    }
    text.push_str(&before[keep_from..]);
    let highlight_start = text.len();
    let rest = &line[start..];
    let limit = rest
        .char_indices()
        .nth(AFTER_CHARS)
        .map_or(rest.len(), |(at, _)| at);
    let hit_len = end - start;
    let highlight = if hit_len > limit {
        text.push_str(&rest[..limit]);
        let highlight = highlight_start..text.len();
        text.push(ELLIPSIS);
        highlight
    } else {
        text.push_str(&rest[..hit_len]);
        let highlight = highlight_start..text.len();
        text.push_str(&rest[hit_len..limit]);
        if limit < rest.len() {
            text.push(ELLIPSIS);
        }
        highlight
    };
    Snippet { text, highlight }
}

/// The snippet for `text`'s first match: the line the match starts on, without its `\r`. A match
/// that runs over more lines (a regex naming `\n`) is highlighted to the end of that first line;
/// one that starts with line breaks starts on the line after them.
pub fn first_snippet(text: &str, matcher: &Matcher) -> Option<Snippet> {
    let hit = matcher.first_in(text)?;
    let matched = &text[hit.clone()];
    let breaks = matched.len() - matched.trim_start_matches(['\r', '\n']).len();
    let start = if breaks < matched.len() {
        hit.start + breaks
    } else {
        hit.start
    };
    let line_start = text[..start].rfind('\n').map_or(0, |at| at + 1);
    let line_end = text[start..].find('\n').map_or(text.len(), |at| start + at);
    let line = &text[line_start..line_end];
    let line = line.strip_suffix('\r').unwrap_or(line);
    let local_start = (start - line_start).min(line.len());
    let local_end = (hit.end.min(line_start + line.len()) - line_start).max(local_start);
    Some(cut(line, local_start..local_end))
}

fn floor_boundary(text: &str, mut at: usize) -> usize {
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --lib search::snippet`
Expected: all 10 tests pass.

Run: `cargo clippy --all-targets -- -D warnings` and `cargo fmt --all -- --check`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add src/search/mod.rs src/search/snippet.rs
git commit -m "feat(search): a snippet of the matching line, cut on character boundaries"
```

---

### Task 3: Online-only notes in the note list, and the text search worker

**Files:**
- Modify: `src/library/mod.rs`
  - Add `pub mod text_search;` between `pub mod store;` and `pub mod title;`.
  - `NoteEntry` (lines 46–51): add `online_only`.
  - Every place `NoteEntry` is built. `rg -n "NoteEntry \{" src tests` finds exactly four:
    - `load` (line 227, from the scan's `ScanEntry`)
    - `merge_notes` (line 326, a path FastPad touched while a rescan ran)
    - `LibraryState::add_note` (line 480, a save by FastPad)
    - the test helper `entries` (line 900)
  - `LibraryState::rename_note` (lines 500–515) keeps the flag. `src/bin/fastpad-bench.rs:378` only takes `size_of::<NoteEntry>()` and needs no change.
- Modify: `src/library/name_search.rs` (`fn folder_of`, line 82, becomes `pub(super)`. It is visibility only; no behavior changes.)
- Create: `src/library/text_search.rs`

**Interfaces:**
- Consumes:
  - From Tasks 1 and 2: `Matcher`, `Snippet`, `first_snippet`.
  - `tree::natural_cmp`, `name_search::folder_of`, `file::encoding::decode`, and `scan::ScanEntry.online_only` (set from `FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS | FILE_ATTRIBUTE_OFFLINE`).
- Produces:
  - `NoteEntry.online_only: bool`.
  - Everything in the Task Interfaces list for `src/library/text_search.rs`: `RESULT_CAP`, `MAX_NOTE_BYTES`, `BATCH_HITS`, `BATCH_INTERVAL`, `SearchNote`, `Stamp`, `TextHit`, `SkipReason` (with `ALL` and `pub fn index`), `Progress` (with `skipped_total`), `RunEnd`, `hit_cmp` and `run`.
  - One addition: `impl From<&NoteEntry> for SearchNote`, which Task 4 uses to copy the note list.

**Decisions this task settles:**
- **Which `online_only` value each built `NoteEntry` gets:**
  - The scan's flag in `load`.
  - `false` from `add_note`, which records a save: FastPad just wrote the file, so its data is on this PC. That covers both a new entry and an updated one.
  - The rescan's own flag for a touched path in `merge_notes`, or `false` if the rescan didn't list it.
  - `rename_note` keeps the old entry's flag, because a rename moves no data.
- **An overlay wins over everything.** A dirty tab's text is searched even when the note is online only or over 4 MB, since it's already in memory. It is never counted as skipped, and its hit has `stamp: None`.
- **Reading a note from disk:**
  - An online-only note is skipped before it's opened, and so is one whose scan size is over `MAX_NOTE_BYTES`.
  - Otherwise the note is opened with `std::fs::File::open`, which shares read, write and delete. `Stamp` comes from the opened file's `metadata()`, and the read is capped at `MAX_NOTE_BYTES + 1` with `Read::take`. A file that grew past the limit since the scan is `TooLarge` and is never read in full.
  - An I/O error is `Unreadable`, and an `encoding::decode` error is `NotText`. One byte buffer is reused for every file.
- **Batching:**
  - The clock is read once at the start and once after each note.
  - A batch goes out when it holds `BATCH_HITS` hits, or when `BATCH_INTERVAL` has passed since the last batch. An interval batch may be **empty** and carry only progress.
  - The final batch always goes out, empty or not.
  - The 50 ms rule is tested deterministically through a private `run_with_clock(…, clock: &mut dyn FnMut() -> Instant)`. `run` passes `&mut Instant::now`.
- **The cap:** the note that brings the hits to `RESULT_CAP` sends its batch, and `run` returns `Capped`, **if notes remain**. When the 500th hit is the last note, the search completes normally ("500 notes", not "500+ notes").
- **Cancel** is read before each note and before the final batch. Once it's set, nothing more is sent.
- **Overlay keys** are compared as `path.to_string_lossy().to_lowercase()`. That is `model::same_path`'s rule (`same_path` itself isn't public outside `library`). The keys are computed once per run, and per note only when there are overlays.

**Part A: `NoteEntry.online_only`**

- [ ] **Step 1: Write the failing tests** in `src/library/mod.rs`'s `mod tests`

In the `entries` helper (line 898), add the field:

```rust
    /// `count` index entries that exist only in memory.
    fn entries(count: usize) -> Vec<NoteEntry> {
        (0..count)
            .map(|index| NoteEntry {
                path: PathBuf::from(format!("f{index}.md")),
                size: 0,
                mtime: 0,
                online_only: false,
            })
            .collect()
    }
```

Then add, after the `bare_state` helper and before `a_bulk_change_made_outside_fastpad_costs_no_stat_when_a_rescan_is_merged`:

```rust
    fn mark_online_only(path: &Path) {
        let wide = crate::platform::wide_null(&path.to_string_lossy());
        let marked = unsafe {
            windows_sys::Win32::Storage::FileSystem::SetFileAttributesW(
                wide.as_ptr(),
                windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_OFFLINE,
            )
        };
        assert_ne!(marked, 0, "SetFileAttributesW failed");
    }

    fn online_only(state: &LibraryState, relative: &str) -> bool {
        state
            .notes
            .iter()
            .find(|note| note.path == Path::new(relative))
            .unwrap()
            .online_only
    }

    #[test]
    fn an_online_only_file_is_marked_in_the_note_list() {
        // Break caught: text search opening a OneDrive online-only note, which downloads it.
        let scratch = Scratch::new("online-only");
        let cloud = scratch.folder().join("cloud.md");
        std::fs::write(&cloud, "a").unwrap();
        std::fs::write(scratch.folder().join("here.md"), "b").unwrap();
        mark_online_only(&cloud);
        let state = load(&scratch.folder(), &scratch.local(), 100).unwrap();
        assert!(online_only(&state, "cloud.md"));
        assert!(!online_only(&state, "here.md"));
    }

    #[test]
    fn a_rename_keeps_a_notes_online_only_flag_and_a_save_clears_it() {
        // Break caught: renaming an online-only note in FastPad making the next search download
        // it, or a note FastPad just saved still skipped as online only.
        let scratch = Scratch::new("online-rename");
        let old = scratch.folder().join("old.md");
        std::fs::write(&old, "a").unwrap();
        mark_online_only(&old);
        let mut state = load(&scratch.folder(), &scratch.local(), 100).unwrap();
        let new = scratch.folder().join("new.md");
        std::fs::rename(&old, &new).unwrap();
        state.rename_note(&old, &new);
        assert!(online_only(&state, "new.md"));
        std::fs::write(&new, "saved").unwrap();
        state.add_note(&new);
        assert!(!online_only(&state, "new.md"));
        let added = scratch.folder().join("added.md");
        std::fs::write(&added, "c").unwrap();
        assert!(state.add_note(&added));
        assert!(!online_only(&state, "added.md"));
    }

    #[test]
    fn a_merged_rescan_keeps_the_scans_online_only_flag_for_a_touched_note() {
        let scratch = Scratch::new("online-merge");
        let a = scratch.folder().join("a.md");
        std::fs::write(&a, "a").unwrap();
        let mut previous = bare_state(&scratch, Vec::new(), false);
        previous.add_note(&a);
        let seen_online_only = NoteEntry {
            path: PathBuf::from("a.md"),
            size: 1,
            mtime: 0,
            online_only: true,
        };
        let fresh = bare_state(&scratch, vec![seen_online_only], false);
        let merged = merge_rescan(previous, fresh);
        assert!(online_only(&merged, "a.md"));
    }
```

`FILE_ATTRIBUTE_OFFLINE` can be set with `SetFileAttributesW`, and the scan reads it with `FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS`, so this test needs no OneDrive. A file with the attribute is still deleted normally by `Scratch`'s `Drop`.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib library::tests`
Expected: a compile error, because `NoteEntry` has no field `online_only`.

- [ ] **Step 3: Implement**

In `src/library/mod.rs`, replace the struct:

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NoteEntry {
    pub path: PathBuf,
    pub size: u64,
    pub mtime: u64,
    /// The file's data is only in the cloud (a OneDrive "online only" file): reading it would
    /// download it, so text search skips it. Set by the scan; a save by FastPad clears it.
    pub online_only: bool,
}
```

In `load`, the note list takes the scan's flag:

```rust
    let notes: Vec<NoteEntry> = scan
        .entries
        .iter()
        .map(|entry| NoteEntry {
            path: entry.path.clone(),
            size: entry.size,
            mtime: entry.mtime,
            online_only: entry.online_only,
        })
        .collect();
```

In `merge_notes`, replace the loop body's `retain` and the `push`:

```rust
    for path in touched {
        if !seen.insert(path.to_string_lossy().to_lowercase()) {
            continue;
        }
        // A rename leaves a file online only, and the rescan's own entry says whether it still is.
        let online_only = merged
            .iter()
            .find(|note| same_path(&note.path, path))
            .is_some_and(|note| note.online_only);
        merged.retain(|note| !same_path(&note.path, path));
        let is_note = path
            .extension()
            .is_some_and(|ext| title::is_note_extension(&ext.to_string_lossy()));
        if is_note && let Some(stamp) = store::stamp(&folder.join(path)) {
            merged.push(NoteEntry {
                path: path.clone(),
                size: stamp.size,
                mtime: filetime_ticks(stamp.modified),
                online_only,
            });
        }
    }
```

In `LibraryState::add_note`, replace the `if let … else` at its end:

```rust
        if let Some(existing) = self
            .notes
            .iter_mut()
            .find(|note| same_path(&note.path, &relative))
        {
            existing.size = size;
            existing.mtime = mtime;
            // FastPad just wrote the file, so its data is on this PC.
            existing.online_only = false;
            false
        } else {
            let pinned = self.is_pinned(&relative);
            self.tree.insert_note(&relative, pinned);
            self.notes.push(NoteEntry {
                path: relative,
                size,
                mtime,
                online_only: false,
            });
            true
        }
```

Replace `LibraryState::rename_note`:

```rust
    /// Follows a rename FastPad made: the record, the index and the tree. The record moves first,
    /// so the entry added for the new name finds its pin.
    pub fn rename_note(&mut self, old: &Path, new: &Path) {
        // A rename moves no data: an online-only note stays online only.
        let online_only = strip_folder(&self.folder, old).is_some_and(|stored| {
            self.notes
                .iter()
                .any(|note| note.online_only && same_path(&note.path, &stored))
        });
        let old_stored = record_path(&self.folder, old);
        let new_stored = record_path(&self.folder, new);
        if let Some(record) = self.library.note_by_path(&old_stored) {
            let note = NoteRef {
                id: record.id,
                path: old_stored,
            };
            let _ = self.apply(PendingOp::Relocate {
                note,
                path: new_stored,
            });
        }
        self.remove_note(old);
        let _ = self.add_note(new);
        if online_only
            && let Some(relative) = strip_folder(&self.folder, new)
            && let Some(entry) = self
                .notes
                .iter_mut()
                .find(|note| same_path(&note.path, &relative))
        {
            entry.online_only = true;
        }
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib library::tests`
Expected: all pass, including the three new ones, and `a_truncated_rescan_caps_the_merged_notes_list_at_the_scan_limit` and `a_bulk_change_made_outside_fastpad_costs_no_stat_when_a_rescan_is_merged`, which use `entries`.

Run: `rg -n "NoteEntry \{" src tests`
Expected: the struct, and the four construction sites above, each now with `online_only`.

- [ ] **Step 5: Commit**

```bash
git add src/library/mod.rs
git commit -m "feat(library): the note list marks online-only files; a save clears the mark and a rename keeps it"
```

**Part B: `src/library/text_search.rs`**

- [ ] **Step 6: Wire the module**

In `src/library/mod.rs`, add between `pub mod store;` and `pub mod title;`:

```rust
pub mod text_search;
```

In `src/library/name_search.rs`, change `fn folder_of` (line 82) to:

```rust
/// The folder `path` is in, relative to the notebook and joined with `\`; `""` at the root.
pub(super) fn folder_of(path: &Path) -> String {
```

(The body stays as it is.)

- [ ] **Step 7: Write the failing tests** at the bottom of `src/library/text_search.rs`

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::MatchOptions;
    use std::sync::atomic::Ordering::Relaxed;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(label: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "fastpad-text-search-{label}-{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).unwrap();
            Self(root)
        }

        /// Writes `bytes` at `relative` and returns the note as a scan would list it.
        fn file(&self, relative: &str, bytes: &[u8]) -> SearchNote {
            let path = self.0.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, bytes).unwrap();
            note(relative, bytes.len() as u64)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn note(relative: &str, size: u64) -> SearchNote {
        SearchNote {
            path: PathBuf::from(relative),
            size,
            online_only: false,
        }
    }

    fn find(query: &str) -> Matcher {
        Matcher::new(query, MatchOptions::default()).unwrap()
    }

    type Batches = Vec<(Vec<TextHit>, Progress)>;

    fn collect(
        notebook: &Path,
        notes: &[SearchNote],
        overlays: &HashMap<PathBuf, String>,
        matcher: &Matcher,
        clock: &mut dyn FnMut() -> Instant,
    ) -> (RunEnd, Batches) {
        let mut batches = Vec::new();
        let cancel = AtomicBool::new(false);
        let end = run_with_clock(
            notebook,
            notes,
            overlays,
            matcher,
            &cancel,
            &mut |hits, progress| batches.push((hits, progress)),
            clock,
        );
        (end, batches)
    }

    fn search(
        notebook: &Path,
        notes: &[SearchNote],
        overlays: &HashMap<PathBuf, String>,
        query: &str,
    ) -> (RunEnd, Vec<TextHit>, Progress) {
        let (end, batches) = collect(notebook, notes, overlays, &find(query), &mut frozen_clock());
        let progress = batches.last().map(|(_, progress)| *progress).unwrap();
        let hits = batches.into_iter().flat_map(|(hits, _)| hits).collect();
        (end, hits, progress)
    }

    fn names(hits: &[TextHit]) -> Vec<&str> {
        hits.iter().map(|hit| hit.name.as_str()).collect()
    }

    /// A clock that never moves, so only the hit count sends batches.
    fn frozen_clock() -> impl FnMut() -> Instant {
        let start = Instant::now();
        move || start
    }

    /// A clock that moves `step` further on each read, starting at no time at all.
    fn stepping_clock(step: Duration) -> impl FnMut() -> Instant {
        let start = Instant::now();
        let mut reads = 0;
        move || {
            let now = start + step * reads;
            reads += 1;
            now
        }
    }

    /// `count` overlay-only notes, all containing "needle", so nothing touches the disk.
    fn overlay_notes(count: usize) -> (Vec<SearchNote>, HashMap<PathBuf, String>) {
        let notes: Vec<SearchNote> = (0..count)
            .map(|index| note(&format!("n{index}.md"), 10))
            .collect();
        let overlays = notes
            .iter()
            .map(|note| (note.path.clone(), "a needle here".to_owned()))
            .collect();
        (notes, overlays)
    }

    #[test]
    fn a_hit_carries_its_name_folder_snippet_and_the_stamp_of_what_was_read() {
        // Break caught: a replace (3b) comparing against a stamp the search never took, or a
        // result row showing the extension or a folder with `/`.
        let scratch = Scratch::new("hit");
        let a = scratch.file(
            r"work\q1\Budget plan.md",
            b"Title\r\n\r\n    paid the invoice\r\n",
        );
        let b = scratch.file("todo.txt", b"nothing to see");
        let (end, hits, progress) = search(&scratch.0, &[a, b], &HashMap::new(), "INVOICE");
        assert_eq!(end, RunEnd::Completed);
        assert_eq!(hits.len(), 1);
        let hit = &hits[0];
        assert_eq!(hit.path, PathBuf::from(r"work\q1\Budget plan.md"));
        assert_eq!(hit.name, "Budget plan");
        assert_eq!(hit.folder, r"work\q1");
        assert_eq!(hit.snippet.text, "paid the invoice");
        assert_eq!(&hit.snippet.text[hit.snippet.highlight.clone()], "invoice");
        let metadata = std::fs::metadata(scratch.0.join(&hit.path)).unwrap();
        assert_eq!(
            hit.stamp,
            Some(Stamp {
                size: metadata.len(),
                mtime: metadata.last_write_time(),
            })
        );
        assert_eq!(
            progress,
            Progress {
                visited: 2,
                total: 2,
                skipped: [0; 4],
            }
        );
    }

    #[test]
    fn an_overlay_wins_over_the_disk_both_ways() {
        // Break caught: searching the saved file of a note open with unsaved edits, so a phrase
        // only typed in the editor is missed, or a phrase deleted in the editor still shows.
        let scratch = Scratch::new("overlay");
        let edited = scratch.file("Edited.md", b"the old phrase");
        let untouched = scratch.file("Untouched.md", b"the old phrase and the new phrase");
        let notes = [edited, untouched];
        // The key differs in case from the note list's path; paths compare ignoring case.
        let overlays = HashMap::from([(PathBuf::from("EDITED.md"), "the new phrase".to_owned())]);
        let (_, hits, _) = search(&scratch.0, &notes, &overlays, "new phrase");
        assert_eq!(names(&hits), ["Edited", "Untouched"]);
        assert_eq!(
            hits[0].path,
            PathBuf::from("Edited.md"),
            "the note list's spelling"
        );
        assert_eq!(hits[0].stamp, None, "an overlay has no stamp");
        assert!(hits[1].stamp.is_some());
        let (_, hits, _) = search(&scratch.0, &notes, &overlays, "old phrase");
        assert_eq!(names(&hits), ["Untouched"]);
    }

    #[test]
    fn an_overlay_is_searched_even_for_an_online_only_or_oversized_note() {
        let scratch = Scratch::new("overlay-skips");
        let mut online = note("Online.md", 10);
        online.online_only = true;
        let large = note("Large.md", MAX_NOTE_BYTES + 1);
        let overlays = HashMap::from([
            (PathBuf::from("Online.md"), "typed needle".to_owned()),
            (PathBuf::from("Large.md"), "typed needle".to_owned()),
        ]);
        let (_, hits, progress) = search(&scratch.0, &[online, large], &overlays, "needle");
        assert_eq!(names(&hits), ["Online", "Large"]);
        assert_eq!(progress.skipped_total(), 0);
    }

    #[test]
    fn skipped_notes_are_counted_by_reason_and_never_matched() {
        // Break caught: an online-only note opened (recalling it from the cloud), a huge file read
        // whole, or a binary file's bytes "matching".
        let scratch = Scratch::new("skips");
        let mut online = scratch.file("online.md", b"needle");
        online.online_only = true;
        let mut large = scratch.file("large.md", b"needle");
        large.size = MAX_NOTE_BYTES + 1;
        let missing = note("deleted since the scan.md", 6);
        let binary = scratch.file(
            "binary.txt",
            &[0x80, 0x81, b'n', b'e', b'e', b'd', b'l', b'e'],
        );
        let fine = scratch.file("fine.md", b"needle");
        let notes = [online, large, missing, binary, fine];
        let (end, hits, progress) = search(&scratch.0, &notes, &HashMap::new(), "needle");
        assert_eq!(end, RunEnd::Completed);
        assert_eq!(names(&hits), ["fine"]);
        assert_eq!(progress.visited, 5);
        assert_eq!(progress.total, 5);
        for reason in SkipReason::ALL {
            assert_eq!(progress.skipped[reason.index()], 1, "{reason:?}");
        }
        assert_eq!(progress.skipped_total(), 4);
        assert_eq!(
            SkipReason::ALL.map(SkipReason::index),
            [0, 1, 2, 3],
            "the tooltip's order"
        );
    }

    #[test]
    fn a_note_that_grew_past_the_limit_since_the_scan_is_too_large() {
        let scratch = Scratch::new("grew");
        let mut text = vec![b'x'; MAX_NOTE_BYTES as usize];
        text.extend_from_slice(b" needle");
        let mut grown = scratch.file("grown.md", &text);
        grown.size = 10;
        let (_, hits, progress) = search(&scratch.0, &[grown], &HashMap::new(), "needle");
        assert!(hits.is_empty());
        assert_eq!(progress.skipped[SkipReason::TooLarge.index()], 1);
    }

    #[test]
    fn utf16_and_bom_notes_are_decoded_before_matching() {
        let scratch = Scratch::new("encodings");
        let mut utf16 = vec![0xFF, 0xFE];
        for unit in "Ünïcode needle".encode_utf16() {
            utf16.extend_from_slice(&unit.to_le_bytes());
        }
        let wide = scratch.file("wide.txt", &utf16);
        let bom = scratch.file("bom.md", b"\xEF\xBB\xBFneedle first");
        let (_, hits, _) = search(&scratch.0, &[wide, bom], &HashMap::new(), "needle");
        assert_eq!(names(&hits), ["wide", "bom"]);
        assert_eq!(hits[0].snippet.text, "Ünïcode needle");
        assert_eq!(hits[1].snippet.text, "needle first");
    }

    #[test]
    fn only_the_listed_notes_are_visited() {
        // Break caught: narrowing (the host passing the previous hits) still reading every note.
        let scratch = Scratch::new("listed");
        let a = scratch.file("a.md", b"needle");
        let _b = scratch.file("b.md", b"needle");
        let c = scratch.file("c.md", b"needle");
        let (_, hits, progress) = search(&scratch.0, &[a, c], &HashMap::new(), "needle");
        assert_eq!(names(&hits), ["a", "c"]);
        assert_eq!(progress.total, 2);
    }

    #[test]
    fn batches_go_out_every_fifty_hits_and_a_final_batch_follows() {
        let scratch = Scratch::new("count-batches");
        let (notes, overlays) = overlay_notes(120);
        let (end, batches) = collect(
            &scratch.0,
            &notes,
            &overlays,
            &find("needle"),
            &mut frozen_clock(),
        );
        assert_eq!(end, RunEnd::Completed);
        let sizes: Vec<usize> = batches.iter().map(|(hits, _)| hits.len()).collect();
        assert_eq!(sizes, [50, 50, 20]);
        let visited: Vec<usize> = batches
            .iter()
            .map(|(_, progress)| progress.visited)
            .collect();
        assert_eq!(visited, [50, 100, 120]);
        // The final batch goes out even when it is empty.
        let (notes, overlays) = overlay_notes(50);
        let (_, batches) = collect(
            &scratch.0,
            &notes,
            &overlays,
            &find("needle"),
            &mut frozen_clock(),
        );
        let sizes: Vec<usize> = batches.iter().map(|(hits, _)| hits.len()).collect();
        assert_eq!(sizes, [50, 0]);
    }

    #[test]
    fn a_batch_goes_out_when_fifty_milliseconds_have_passed() {
        // Break caught: a slow notebook with few matches showing nothing, and no progress, until
        // the whole search ends.
        let scratch = Scratch::new("interval-batches");
        let notes: Vec<SearchNote> = ["a", "b", "c", "d", "e"]
            .iter()
            .map(|name| note(&format!("{name}.md"), 10))
            .collect();
        let overlays = HashMap::from([
            (PathBuf::from("a.md"), "needle".to_owned()),
            (PathBuf::from("b.md"), "hay".to_owned()),
            (PathBuf::from("c.md"), "hay".to_owned()),
            (PathBuf::from("d.md"), "hay".to_owned()),
            (PathBuf::from("e.md"), "needle".to_owned()),
        ]);
        // The clock reads 0 at the start and 30, 60, 90, 120, 150 ms after each note: batches go
        // out after the second note (60 ms since the start) and the fourth (60 ms since then).
        let mut clock = stepping_clock(Duration::from_millis(30));
        let (end, batches) = collect(&scratch.0, &notes, &overlays, &find("needle"), &mut clock);
        assert_eq!(end, RunEnd::Completed);
        let shape: Vec<(Vec<&str>, usize)> = batches
            .iter()
            .map(|(hits, progress)| (names(hits), progress.visited))
            .collect();
        assert_eq!(
            shape,
            [(vec!["a"], 2), (vec![], 4), (vec!["e"], 5)],
            "an interval batch can carry only progress"
        );
    }

    #[test]
    fn the_cap_ends_the_search_after_the_capping_batch() {
        let scratch = Scratch::new("cap");
        let (notes, overlays) = overlay_notes(RESULT_CAP + 20);
        let (end, batches) = collect(
            &scratch.0,
            &notes,
            &overlays,
            &find("needle"),
            &mut frozen_clock(),
        );
        assert_eq!(end, RunEnd::Capped);
        let found: usize = batches.iter().map(|(hits, _)| hits.len()).sum();
        assert_eq!(found, RESULT_CAP);
        assert_eq!(
            batches.len(),
            RESULT_CAP / BATCH_HITS,
            "no batch after the cap"
        );
        let last = batches.last().unwrap().1;
        assert_eq!((last.visited, last.total), (RESULT_CAP, RESULT_CAP + 20));
        // Exactly the cap, with nothing left to visit, is a completed search.
        let (notes, overlays) = overlay_notes(RESULT_CAP);
        let (end, _) = collect(
            &scratch.0,
            &notes,
            &overlays,
            &find("needle"),
            &mut frozen_clock(),
        );
        assert_eq!(end, RunEnd::Completed);
    }

    #[test]
    fn a_cancelled_search_stops_at_once_and_sends_nothing_more() {
        // Break caught: typing fast leaving the old search reading the whole notebook, or posting
        // batches after the query that started it was replaced.
        let scratch = Scratch::new("cancel");
        let (notes, overlays) = overlay_notes(200);
        let matcher = find("needle");
        let cancel = AtomicBool::new(true);
        let mut calls = 0;
        let end = run(
            &scratch.0,
            &notes,
            &overlays,
            &matcher,
            &cancel,
            &mut |_, _| {
                calls += 1;
            },
        );
        assert_eq!((end, calls), (RunEnd::Cancelled, 0));

        let cancel = AtomicBool::new(false);
        let mut seen = Vec::new();
        let end = run_with_clock(
            &scratch.0,
            &notes,
            &overlays,
            &matcher,
            &cancel,
            &mut |hits, progress| {
                seen.push((hits.len(), progress.visited));
                cancel.store(true, Relaxed);
            },
            &mut frozen_clock(),
        );
        assert_eq!(end, RunEnd::Cancelled);
        assert_eq!(seen, [(50, 50)]);
    }

    #[test]
    fn hits_sort_by_natural_name_then_folder_with_the_root_first_then_path() {
        let hit = |path: &str| {
            let path = PathBuf::from(path);
            TextHit {
                name: path.file_stem().unwrap().to_string_lossy().into_owned(),
                folder: folder_of(&path),
                path,
                snippet: Snippet::default(),
                stamp: None,
            }
        };
        let mut hits = [
            hit(r"b\Note 10.md"),
            hit(r"a\Note 2.md"),
            hit("Note 2.txt"),
            hit(r"a\Note 2.md.bak"),
            hit("note 2.md"),
            hit("Alpha.md"),
        ];
        hits.sort_by(hit_cmp);
        let order: Vec<String> = hits
            .iter()
            .map(|hit| hit.path.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            order,
            [
                "Alpha.md",
                "Note 2.txt",
                "note 2.md",
                r"a\Note 2.md",
                r"a\Note 2.md.bak",
                r"b\Note 10.md",
            ]
        );
    }
}
```

The tests use the crate's own scratch pattern, a folder under `std::env::temp_dir()` removed on `Drop`, as `scan.rs` and `library/mod.rs` do. The cap, batch and cancel tests serve every note from overlays, so they write no files.

- [ ] **Step 8: Run the tests to verify they fail**

Run: `cargo test --lib library::text_search`
Expected: a compile error, because `SearchNote`, `run`, `run_with_clock`, `TextHit` and the rest don't exist.

- [ ] **Step 9: Write the implementation** at the top of `src/library/text_search.rs`

```rust
//! Text search over a notebook, run on a worker thread: each note's text (from disk, or from a
//! dirty tab's overlay) is matched, and each note's first match goes to a sink in batches with
//! the progress so far. No Win32 and no window.

use super::name_search::folder_of;
use super::tree::natural_cmp;
use crate::file::encoding;
use crate::search::{Matcher, Snippet, first_snippet};
use std::cmp::Ordering;
use std::collections::HashMap;
use std::io::Read;
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

/// A search stops after this many matching notes.
pub const RESULT_CAP: usize = 500;
/// A larger note is skipped as `TooLarge`.
pub const MAX_NOTE_BYTES: u64 = 4 * 1024 * 1024;
/// A batch goes out when it holds this many hits...
pub const BATCH_HITS: usize = 50;
/// ...or when this long has passed since the last one.
pub const BATCH_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchNote {
    /// Relative to the notebook.
    pub path: PathBuf,
    /// The scan's size.
    pub size: u64,
    pub online_only: bool,
}

impl From<&super::NoteEntry> for SearchNote {
    fn from(note: &super::NoteEntry) -> Self {
        SearchNote {
            path: note.path.clone(),
            size: note.size,
            online_only: note.online_only,
        }
    }
}

/// What the search read: the opened file's size and last write time (FILETIME ticks).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Stamp {
    pub size: u64,
    pub mtime: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextHit {
    /// Relative to the notebook, as the note list has it.
    pub path: PathBuf,
    /// The file name without its extension.
    pub name: String,
    /// The parent folder joined with `\`; `""` at the root.
    pub folder: String,
    pub snippet: Snippet,
    /// `None` when the text came from an overlay.
    pub stamp: Option<Stamp>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SkipReason {
    OnlineOnly = 0,
    TooLarge = 1,
    Unreadable = 2,
    NotText = 3,
}

impl SkipReason {
    /// In the order the status line's tooltip lists them.
    pub const ALL: [SkipReason; 4] = [
        Self::OnlineOnly,
        Self::TooLarge,
        Self::Unreadable,
        Self::NotText,
    ];

    /// The index into `Progress::skipped`.
    pub fn index(self) -> usize {
        self as usize
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Progress {
    /// Notes processed so far, skipped ones included.
    pub visited: usize,
    pub total: usize,
    /// Skipped notes by `SkipReason::index`.
    pub skipped: [usize; 4],
}

impl Progress {
    pub fn skipped_total(&self) -> usize {
        self.skipped.iter().sum()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunEnd {
    /// Every note was visited.
    Completed,
    /// `RESULT_CAP` notes matched before the last note.
    Capped,
    Cancelled,
}

/// Natural name order, then the folder (the root first), then the exact path.
pub fn hit_cmp(a: &TextHit, b: &TextHit) -> Ordering {
    natural_cmp(&a.name, &b.name)
        .then_with(|| natural_cmp(&a.folder, &b.folder))
        .then_with(|| a.path.cmp(&b.path))
}

/// Searches `notes` in order and streams each note's first match to `sink`.
///
/// - `overlays` holds dirty tabs' text by relative path (compared ignoring case). An overlay is
///   searched instead of the disk, even for a note that is online only or over the size limit.
/// - A batch goes to `sink` at `BATCH_HITS` hits, or when `BATCH_INTERVAL` has passed since the
///   last batch (checked after each note, so a batch can be empty and carry only progress), and
///   once more at the end even if empty.
/// - At `RESULT_CAP` hits the batch holding the last one is sent and the search ends `Capped`,
///   unless that note was the last, which ends `Completed`.
/// - `cancel` is read before each note and before the final batch. Once it is set, nothing more
///   is sent.
pub fn run(
    notebook: &Path,
    notes: &[SearchNote],
    overlays: &HashMap<PathBuf, String>,
    matcher: &Matcher,
    cancel: &AtomicBool,
    sink: &mut dyn FnMut(Vec<TextHit>, Progress),
) -> RunEnd {
    run_with_clock(
        notebook,
        notes,
        overlays,
        matcher,
        cancel,
        sink,
        &mut Instant::now,
    )
}

/// `run` with the clock passed in, so tests can step time. The clock is read once at the start
/// and once after each note.
fn run_with_clock(
    notebook: &Path,
    notes: &[SearchNote],
    overlays: &HashMap<PathBuf, String>,
    matcher: &Matcher,
    cancel: &AtomicBool,
    sink: &mut dyn FnMut(Vec<TextHit>, Progress),
    clock: &mut dyn FnMut() -> Instant,
) -> RunEnd {
    let cancelled = || cancel.load(std::sync::atomic::Ordering::Relaxed);
    let overlays: Vec<(String, &str)> = overlays
        .iter()
        .map(|(path, text)| (path_key(path), text.as_str()))
        .collect();
    let mut progress = Progress {
        total: notes.len(),
        ..Progress::default()
    };
    let mut batch = Vec::new();
    let mut hits = 0;
    // One buffer for every file read; it is freed when the search ends.
    let mut bytes = Vec::new();
    let mut last_sent = clock();
    for note in notes {
        if cancelled() {
            return RunEnd::Cancelled;
        }
        match search_note(notebook, note, &overlays, matcher, &mut bytes) {
            Searched::Hit(hit) => {
                batch.push(hit);
                hits += 1;
            }
            Searched::NoMatch => {}
            Searched::Skipped(reason) => progress.skipped[reason.index()] += 1,
        }
        progress.visited += 1;
        if hits == RESULT_CAP && progress.visited < progress.total {
            sink(std::mem::take(&mut batch), progress);
            return RunEnd::Capped;
        }
        let now = clock();
        if batch.len() >= BATCH_HITS || now.saturating_duration_since(last_sent) >= BATCH_INTERVAL {
            sink(std::mem::take(&mut batch), progress);
            last_sent = now;
        }
    }
    if cancelled() {
        return RunEnd::Cancelled;
    }
    sink(batch, progress);
    RunEnd::Completed
}

enum Searched {
    Hit(TextHit),
    NoMatch,
    Skipped(SkipReason),
}

/// How overlay keys and note paths are compared: `same_path`'s rule, ignoring case.
fn path_key(path: &Path) -> String {
    path.to_string_lossy().to_lowercase()
}

fn search_note(
    notebook: &Path,
    note: &SearchNote,
    overlays: &[(String, &str)],
    matcher: &Matcher,
    bytes: &mut Vec<u8>,
) -> Searched {
    let overlay = if overlays.is_empty() {
        None
    } else {
        let key = path_key(&note.path);
        overlays
            .iter()
            .find(|(overlay, _)| *overlay == key)
            .map(|(_, text)| *text)
    };
    let (snippet, stamp) = match overlay {
        Some(text) => (first_snippet(text, matcher), None),
        None => match read_note(notebook, note, bytes) {
            Ok((text, stamp)) => (first_snippet(&text, matcher), Some(stamp)),
            Err(reason) => return Searched::Skipped(reason),
        },
    };
    let Some(snippet) = snippet else {
        return Searched::NoMatch;
    };
    Searched::Hit(TextHit {
        path: note.path.clone(),
        name: note
            .path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default(),
        folder: folder_of(&note.path),
        snippet,
        stamp,
    })
}

/// The note's text from disk and the stamp of what was read. An online-only note is never
/// opened, so a search never recalls it; a note over the limit by the scan's size is not opened
/// either, and one that grew past it since is not read past the limit.
fn read_note(
    notebook: &Path,
    note: &SearchNote,
    bytes: &mut Vec<u8>,
) -> Result<(String, Stamp), SkipReason> {
    if note.online_only {
        return Err(SkipReason::OnlineOnly);
    }
    if note.size > MAX_NOTE_BYTES {
        return Err(SkipReason::TooLarge);
    }
    let file =
        std::fs::File::open(notebook.join(&note.path)).map_err(|_| SkipReason::Unreadable)?;
    let metadata = file.metadata().map_err(|_| SkipReason::Unreadable)?;
    if metadata.len() > MAX_NOTE_BYTES {
        return Err(SkipReason::TooLarge);
    }
    bytes.clear();
    file.take(MAX_NOTE_BYTES + 1)
        .read_to_end(bytes)
        .map_err(|_| SkipReason::Unreadable)?;
    if bytes.len() as u64 > MAX_NOTE_BYTES {
        return Err(SkipReason::TooLarge);
    }
    let decoded = encoding::decode(bytes).map_err(|_| SkipReason::NotText)?;
    Ok((
        decoded.text,
        Stamp {
            size: metadata.len(),
            mtime: metadata.last_write_time(),
        },
    ))
}
```

- [ ] **Step 10: Run the tests to verify they pass**

Run: `cargo test --lib -- library::text_search library::name_search`
Expected: the 12 `text_search` tests and the existing `name_search` tests pass. `a_note_that_grew_past_the_limit_since_the_scan_is_too_large` writes one 4 MB file.

Run: `cargo clippy --all-targets -- -D warnings` and `cargo fmt --all -- --check`
Expected: clean.

- [ ] **Step 11: Commit**

```bash
git add src/library/mod.rs src/library/name_search.rs src/library/text_search.rs
git commit -m "feat(library): a text search worker that streams each note's first match in batches"
```

---

### Task 4: The search host and the Search view's data

Results become text hits streamed from a worker: the debounce, the generation, the cancel flag, the batch message, the dirty-tab overlays, narrowing and the library hooks. The rows still paint on one line here; Task 5 draws them on two.

**Files:**
- Create: `src/window/text_search_host.rs`
- Modify:
  - `src/window/mod.rs`: `pub(crate) mod text_search_host;`, and `WM_FASTPAD_TEXT_SEARCH_BATCH` in the `pub use messages::{...}` list
  - `src/window/messages.rs`: `WM_FASTPAD_TEXT_SEARCH_BATCH = WM_APP + 13`, after `WM_FASTPAD_NOTEBOOK_CHECKED`
  - `src/app.rs`: the `App.text_search` field, after `library`, and its value in `App::new`
  - `src/window/main_window.rs`:
    - `wndproc`: the `WM_DESTROY` arm (line 226), a new `WM_TIMER` arm after the `AUTOSAVE_TIMER_ID` arm (line 248), and the batch message in the `_ =>` arm next to `WM_FASTPAD_LIBRARY_READY` (line 640)
    - a new `document_text` after `read_inactive_text` (line 3698)
    - tests: `type_into_search` (line 10956), the four Search view tests after it (lines 10965–11137), and the new tests
  - `src/window/library_host.rs`: `install` (line 429) and `notes_mode_changed` (line 1079)
  - `src/window/search_view.rs`: imports, constants, `status_text`, `SearchView` and its methods, `query_changed`, `library_changed`, `shown`, the test helpers and `mod tests`. It stops using `name_search`, which stays compiled and warning-free because it is `pub` in the library crate and `src/bin/fastpad-bench.rs` (line 403) calls `name_search::search`.

**Interfaces:**
- Consumes:
  - Task 1: `crate::search::{MatchOptions, Matcher, SearchOption}`, `Matcher::new(&str, MatchOptions) -> Result<Matcher, PatternError>`, `PatternError.message`, the free function `crate::search::escape(&str) -> String` (re-exported from `search::matcher`), `MatchOptions::toggled`.
  - Task 2: `crate::search::Snippet { text, highlight }`.
  - Task 3: `crate::library::text_search::{self, Progress, RunEnd, SearchNote, TextHit, hit_cmp, run, RESULT_CAP}`, `Progress::skipped_total`, and `NoteEntry.online_only`.
  - Existing: `library_host::{folder, with_state, load_failed}`, `library::{is_inside, record_path}`, `side_panel::current_view`, `main_window::{app_ptr, window_identity, read_inactive_text}`, `Tabs::{documents, active, document}`.
- Produces (`src/window/text_search_host.rs`):
  - `pub(crate) const TEXT_SEARCH_TIMER_ID: usize = 0x4650_5453;`, `pub(crate) const DEBOUNCE_MS: u32 = 150;`, `pub(crate) const MIN_QUERY_CHARS: usize = 2;`
  - `pub(crate) struct SearchBatch { pub generation: u64, pub hits: Vec<TextHit>, pub progress: Progress, pub end: Option<RunEnd> }`
  - `#[derive(Debug, Default)] pub(crate) struct TextSearchHost { generation: u64, cancel: Option<Arc<AtomicBool>>, previous: Option<Narrowing>, running: Option<Running>, list: Option<u64> }`, stored as `App.text_search`
  - `pub(crate) fn schedule(hwnd: HWND)`, `run_now(hwnd: HWND)`, `timer(hwnd: HWND)`, `batch_arrived(hwnd: HWND, lparam: LPARAM)`, `cancel(hwnd: HWND)`, `forget(hwnd: HWND)`, `library_changed(hwnd: HWND)`, `notes_reloaded(hwnd: HWND)`, `searchable(query: &str) -> bool`, `dirty_overlays(hwnd: HWND, notebook: &Path) -> HashMap<PathBuf, String>`
  - Test helpers: `generation(hwnd) -> u64`, `cancel_flag(hwnd) -> Option<Arc<AtomicBool>>`, `test_batch(generation, hits, end) -> LPARAM`
- Produces (`src/window/messages.rs`): `pub const WM_FASTPAD_TEXT_SEARCH_BATCH: u32 = WM_APP + 13;`
- Produces (`src/window/main_window.rs`): `pub(crate) fn document_text(hwnd: HWND, id: DocumentId) -> Option<String>`
- Produces (`src/window/search_view.rs`):
  - `SearchView.{results: Vec<TextHit>, options: MatchOptions, search: SearchState}`
  - `#[derive(Clone, Debug, Default, Eq, PartialEq)] pub(crate) enum SearchState { #[default] Idle, TooShort, Running(Progress), Done { progress: Progress, capped: bool }, PatternError(String) }`
  - `pub(crate) const TOO_SHORT: &str = "Type at least 2 characters.";`
  - `pub(crate) fn notice_text(notebook_open: bool, loaded: bool, failed: bool) -> Option<&'static str>`: the line shown instead of the results.
  - `pub(crate) fn summary_text(state: &SearchState, results: usize) -> Option<(String, bool)>`: the summary line and whether it is an error.
  - `pub(crate) fn status_text(state: &SearchState) -> Option<String>`: the status line at the bottom.
  - `pub(crate) fn current_query(hwnd) -> Option<(String, MatchOptions)>`, `begin_search(hwnd, query: &str, total: usize)`, `apply_batch(hwnd, batch: SearchBatch)`, `set_pattern_error(hwnd, error: Option<String>)`, `options(hwnd) -> MatchOptions`, `toggle_option(hwnd, option: SearchOption)`, `show_with_query(hwnd, text: &str)`, `result_paths(hwnd) -> Vec<PathBuf>`
  - Methods `SearchView::{notice, summary, status_line}`.
  - Test helpers: `shown_results(hwnd) -> Vec<(String, String)>` (name, snippet text), `status(hwnd)` (the notice, as before), `summary(hwnd)`, `search_state(hwnd)`.

Design decisions this task makes (the Task Interfaces above state them, and Task 9 records them in spec §17):
- `cancel` also bumps the generation, so batches already queued for a cancelled run are dropped too, not only those of an older `run_now`.
- Narrowing also requires `!options.whole_word`, and an unchanged note list (`list`) and dirty-tab text (`overlays`), both recorded in `Narrowing`. `notes_reloaded` (called on every `LIBRARY_READY` install) drops the narrowing record, because a rescan may have seen outside edits.
- A library change re-runs the query only if the list of note paths (or an `online_only` flag) changed since the last run, or if the library was reloaded. A save of a listed note changes neither, so it runs nothing (spec §7).
- `run_now` compiles the pattern before checking for a notebook, so a regex error shows even while loading or with no notebook open.

- [ ] **Step 1: Write the failing pure tests**

At the end of the new `src/window/text_search_host.rs` (Step 3 writes the rest of the file):

```rust
#[cfg(test)]
mod tests {
    use super::{MIN_QUERY_CHARS, Narrowing, list_mark, narrows, overlay_mark, searchable};
    use crate::library::NoteEntry;
    use crate::search::MatchOptions;
    use std::collections::HashMap;
    use std::path::PathBuf;

    #[test]
    fn a_query_runs_from_two_characters_and_never_when_it_is_all_white_space() {
        // Break caught: a one-letter query reading every note in the notebook, a two-byte letter
        // counted as two characters, or a query of spaces running at all.
        assert_eq!(MIN_QUERY_CHARS, 2);
        assert!(!searchable(""));
        assert!(!searchable("a"));
        assert!(!searchable("é"), "one character in two bytes");
        assert!(searchable("ab"));
        assert!(searchable("éa"));
        assert!(searchable(" a"), "a leading space is part of the phrase");
        assert!(!searchable("   "));
        assert!(!searchable("\t\t"));
    }

    fn note(path: &str, online_only: bool) -> NoteEntry {
        NoteEntry {
            path: PathBuf::from(path),
            size: 10,
            mtime: 20,
            online_only,
        }
    }

    #[test]
    fn the_list_mark_follows_the_note_paths_and_not_their_sizes() {
        // Break caught: a save of a listed note re-running the search on the next refresh, or a
        // note added, renamed or made online-only leaving the old results in place.
        let base = list_mark(&[note("a.md", false), note("b.md", false)]);
        assert_eq!(base, list_mark(&[note("a.md", false), note("b.md", false)]));
        assert_ne!(base, list_mark(&[note("a.md", false)]), "a note removed");
        assert_ne!(
            base,
            list_mark(&[note("a.md", false), note("c.md", false)]),
            "a note renamed"
        );
        assert_ne!(
            base,
            list_mark(&[note("a.md", false), note("b.md", true)]),
            "a note went online-only"
        );
        let mut saved = note("b.md", false);
        saved.size = 99;
        saved.mtime = 7;
        assert_eq!(
            base,
            list_mark(&[note("a.md", false), saved]),
            "a save of a listed note is no list change"
        );
    }

    #[test]
    fn the_overlay_mark_changes_with_any_tab_text_and_not_with_order() {
        // Break caught: narrowing kept after an edit in a dirty tab, so a phrase typed there after
        // the last search is never found.
        let mut first = HashMap::new();
        first.insert(PathBuf::from("a.md"), "one".to_owned());
        first.insert(PathBuf::from("b.md"), "two".to_owned());
        let mut second = HashMap::new();
        second.insert(PathBuf::from("b.md"), "two".to_owned());
        second.insert(PathBuf::from("a.md"), "one".to_owned());
        assert_eq!(overlay_mark(&first), overlay_mark(&second));
        second.insert(PathBuf::from("a.md"), "one!".to_owned());
        assert_ne!(overlay_mark(&first), overlay_mark(&second));
        assert_ne!(overlay_mark(&HashMap::new()), overlay_mark(&first));
    }

    #[test]
    fn narrowing_needs_a_longer_plain_query_with_the_same_options_and_tab_texts() {
        // Break caught: a whole-word or regex query narrowed to an earlier query's hits ("xfoo"
        // contains "foo", but "foo" is not a whole word in "xfoo"), a shorter query narrowed, or
        // narrowing across a change of options.
        let plain = MatchOptions::default();
        let previous = Narrowing {
            query: "foo".to_owned(),
            options: plain,
            paths: Vec::new(),
            list: 1,
            overlays: 2,
        };
        assert!(narrows(&previous, "foo", plain, 2), "the same query again");
        assert!(narrows(&previous, "food", plain, 2));
        assert!(narrows(&previous, "a foo", plain, 2));
        assert!(!narrows(&previous, "fo", plain, 2), "a shorter query can match more");
        assert!(!narrows(&previous, "Foo", plain, 2), "contained only after folding");
        let case = MatchOptions {
            case: true,
            ..plain
        };
        assert!(!narrows(&previous, "food", case, 2), "the options changed");
        let whole = MatchOptions {
            whole_word: true,
            ..plain
        };
        let previous_whole = Narrowing {
            options: whole,
            ..previous.clone()
        };
        assert!(!narrows(&previous_whole, "xfoo", whole, 2));
        let regex = MatchOptions {
            regex: true,
            ..plain
        };
        let previous_regex = Narrowing {
            options: regex,
            ..previous.clone()
        };
        assert!(!narrows(&previous_regex, "foo|bar", regex, 2));
        assert!(!narrows(&previous, "food", plain, 3), "a dirty tab's text changed");
    }
}
```

In `src/window/search_view.rs`, replace the whole `mod tests` (lines 1018–1048) with:

```rust
#[cfg(test)]
mod tests {
    use super::{
        LOADING, NO_MATCH, NO_NOTEBOOK, SearchState, SearchView, TOO_SHORT, notice_text,
        placeholder, status_text, summary_text,
    };
    use crate::library::text_search::{Progress, RunEnd, TextHit};
    use crate::search::Snippet;
    use crate::window::notebook_view::LOAD_FAILED;
    use crate::window::text_search_host::SearchBatch;
    use std::path::{Path, PathBuf};

    #[test]
    fn the_notice_explains_an_empty_list() {
        // Break caught: a blank Search view with no notebook open, or "No notes match." while the
        // notebook is still loading.
        assert_eq!(notice_text(false, false, false), Some(NO_NOTEBOOK));
        assert_eq!(notice_text(true, false, false), Some(LOADING));
        assert_eq!(notice_text(true, false, true), Some(LOAD_FAILED));
        assert_eq!(notice_text(true, true, false), None);
    }

    #[test]
    fn the_placeholder_names_the_open_notebook() {
        // Break caught: the box saying "Search" with no hint of which notebook it searches.
        assert_eq!(
            placeholder(Some(Path::new(r"C:\Users\me\Work"))),
            "Search Work"
        );
        assert_eq!(placeholder(None), "Search");
    }

    #[test]
    fn the_summary_counts_notes_and_says_when_nothing_matches() {
        // Break caught: "No notes match." before the search finished, a count without its
        // thousands separator, "1 notes", the cap shown as "500 notes", or a regex error shown as
        // ordinary text.
        let done = |capped| SearchState::Done {
            progress: Progress::default(),
            capped,
        };
        let running = SearchState::Running(Progress::default());
        let line = |text: &str, error| Some((text.to_owned(), error));
        assert_eq!(summary_text(&SearchState::Idle, 0), None);
        assert_eq!(summary_text(&SearchState::TooShort, 0), line(TOO_SHORT, false));
        assert_eq!(summary_text(&running, 0), None, "nothing found yet");
        assert_eq!(summary_text(&running, 1), line("1 note", false));
        assert_eq!(summary_text(&done(false), 0), line(NO_MATCH, false));
        assert_eq!(summary_text(&done(false), 1_234), line("1,234 notes", false));
        assert_eq!(summary_text(&done(true), 500), line("500+ notes", false));
        assert_eq!(
            summary_text(&SearchState::PatternError("Unclosed group".to_owned()), 3),
            line("Unclosed group", true)
        );
    }

    #[test]
    fn the_status_line_shows_progress_and_what_was_skipped() {
        // Break caught: no progress while a big notebook is searched, a status line left up after
        // a clean search, or a skipped count with the wrong grammar.
        let progress = Progress {
            visited: 4_120,
            total: 9_800,
            skipped: [0; 4],
        };
        assert_eq!(
            status_text(&SearchState::Running(progress)).as_deref(),
            Some("Searching\u{2026} 4,120 of 9,800")
        );
        let done = |skipped| SearchState::Done {
            progress: Progress {
                visited: 9,
                total: 9,
                skipped,
            },
            capped: false,
        };
        assert_eq!(status_text(&done([0; 4])), None, "hidden when nothing was skipped");
        assert_eq!(
            status_text(&done([0, 1, 0, 0])).as_deref(),
            Some("1 note wasn't searched")
        );
        assert_eq!(
            status_text(&done([2, 0, 1, 1])).as_deref(),
            Some("4 notes weren't searched")
        );
        assert_eq!(status_text(&SearchState::Idle), None);
        assert_eq!(status_text(&SearchState::PatternError("x".to_owned())), None);
    }

    fn hit(name: &str, folder: &str) -> TextHit {
        let file = format!("{name}.md");
        let path = if folder.is_empty() {
            PathBuf::from(file)
        } else {
            Path::new(folder).join(file)
        };
        TextHit {
            path,
            name: name.to_owned(),
            folder: folder.to_owned(),
            snippet: Snippet {
                text: format!("{name} needle"),
                highlight: name.len() + 1..name.len() + 7,
            },
            stamp: None,
        }
    }

    fn batch(hits: Vec<TextHit>, visited: usize, end: Option<RunEnd>) -> SearchBatch {
        SearchBatch {
            generation: 1,
            hits,
            progress: Progress {
                visited,
                total: 10,
                skipped: [0; 4],
            },
            end,
        }
    }

    fn rows(view: &SearchView) -> Vec<(String, String)> {
        view.results
            .iter()
            .map(|result| (result.name.clone(), result.folder.clone()))
            .collect()
    }

    fn row(name: &str, folder: &str) -> (String, String) {
        (name.to_owned(), folder.to_owned())
    }

    const HEIGHT: i32 = 400;

    #[test]
    fn batches_are_inserted_in_order_and_keep_the_selection_by_path() {
        // Break caught: rows appended in arrival order, or a row arriving above the selection
        // moving it, so Enter opens a different note than the one highlighted.
        let mut view = SearchView::new(std::ptr::null_mut(), 96);
        view.begin("needle", 10);
        assert!(view.apply(batch(vec![hit("m", ""), hit("c", "")], 4, None), HEIGHT));
        assert_eq!(rows(&view), [row("c", ""), row("m", "")]);
        assert_eq!(view.list.selected, Some(0), "the first result is selected");
        view.list.select(1, HEIGHT);
        let more = vec![hit("a", ""), hit("b", "sub"), hit("b", "")];
        assert!(view.apply(batch(more, 8, None), HEIGHT));
        assert_eq!(
            rows(&view),
            [row("a", ""), row("b", ""), row("b", "sub"), row("c", ""), row("m", "")],
            "natural name order, then the root before a folder"
        );
        assert_eq!(view.list.selected, Some(4), "still m");
        assert!(view.apply(batch(Vec::new(), 10, Some(RunEnd::Completed)), HEIGHT));
        assert_eq!(
            view.search,
            SearchState::Done {
                progress: Progress {
                    visited: 10,
                    total: 10,
                    skipped: [0; 4]
                },
                capped: false
            }
        );
    }

    #[test]
    fn a_rerun_keeps_the_results_until_its_first_batch_and_a_new_query_starts_empty() {
        // Break caught: the list blanking on every library change, a new query showing the old
        // query's rows until its own arrive, or the selected note lost across either.
        let mut view = SearchView::new(std::ptr::null_mut(), 96);
        view.begin("needle", 10);
        let all = vec![hit("a", ""), hit("b", ""), hit("c", "")];
        view.apply(batch(all, 10, Some(RunEnd::Completed)), HEIGHT);
        view.list.select(2, HEIGHT);

        view.begin("needle", 10);
        assert_eq!(rows(&view).len(), 3, "kept until the first batch");
        assert!(matches!(view.search, SearchState::Running(_)));
        view.apply(batch(vec![hit("b", ""), hit("c", "")], 5, None), HEIGHT);
        assert_eq!(rows(&view), [row("b", ""), row("c", "")], "the first batch replaces them");
        assert_eq!(view.list.selected, Some(1), "c is still selected");

        view.begin("needles", 10);
        assert!(view.results.is_empty(), "a new query starts empty");
        assert_eq!(view.list.selected, None);
        view.apply(batch(vec![hit("a", ""), hit("c", "")], 10, None), HEIGHT);
        assert_eq!(
            view.list.selected,
            Some(1),
            "the remembered note is selected again when it arrives"
        );
    }

    #[test]
    fn a_selection_the_user_moves_during_a_search_wins_over_the_remembered_one() {
        // Break caught: the remembered note arriving late and snatching the selection from the
        // row the user just moved to.
        let mut view = SearchView::new(std::ptr::null_mut(), 96);
        view.begin("needle", 10);
        let all = vec![hit("a", ""), hit("b", ""), hit("c", "")];
        view.apply(batch(all, 10, Some(RunEnd::Completed)), HEIGHT);
        view.list.select(2, HEIGHT);
        view.begin("needles", 10);
        view.apply(batch(vec![hit("a", ""), hit("b", "")], 5, None), HEIGHT);
        assert_eq!(view.list.selected, Some(0), "c has not arrived: the first row");
        view.list.select(1, HEIGHT);
        view.apply(batch(vec![hit("c", "")], 10, None), HEIGHT);
        assert_eq!(view.list.selected, Some(1), "b, which the user picked");
    }

    #[test]
    fn a_batch_that_changes_nothing_shown_asks_for_no_repaint() {
        // Break caught: an InvalidateRect for every empty batch of a long search.
        let mut view = SearchView::new(std::ptr::null_mut(), 96);
        view.begin("needle", 10);
        assert!(view.apply(batch(vec![hit("a", "")], 3, None), HEIGHT));
        assert!(!view.apply(batch(Vec::new(), 3, None), HEIGHT));
        assert!(view.apply(batch(Vec::new(), 4, None), HEIGHT), "the progress moved");
    }
}
```

- [ ] **Step 2: Write the failing window tests**

In `src/window/main_window.rs`, `mod tests`, replace `type_into_search` and the four tests after it (`the_search_view_matches_names_shows_folders_and_opens_the_preview_tab`, `the_search_query_survives_a_view_switch_but_not_a_notebook_switch`, `a_hidden_search_view_searches_again_only_once_it_shows`, and `search_selected` with `saving_a_listed_note_keeps_the_search_selection_and_does_not_rebuild_the_tree`), lines 10956–11137, with:

```rust
    fn type_into_search(hwnd: HWND, text: &str) {
        let edit = crate::window::search_view::edit_hwnd(hwnd).unwrap();
        let wide = crate::platform::wide_null(text);
        // The Edit sends EN_CHANGE to the panel, which restarts the 150 ms debounce.
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::SetWindowTextW(edit, wide.as_ptr());
        }
    }

    fn search_state(hwnd: HWND) -> crate::window::search_view::SearchState {
        crate::window::search_view::search_state(hwnd)
    }

    fn search_generation(hwnd: HWND) -> u64 {
        crate::window::text_search_host::generation(hwnd)
    }

    /// Waits until a search that began after generation `after` has finished.
    fn wait_for_search(hwnd: HWND, after: u64) {
        pump_until(hwnd, || {
            search_generation(hwnd) != after
                && matches!(
                    search_state(hwnd),
                    crate::window::search_view::SearchState::Done { .. }
                )
        });
    }

    /// Types `text` into the Search box and waits past the debounce for its search to finish.
    fn search_for(hwnd: HWND, text: &str) {
        type_into_search(hwnd, text);
        wait_for_search(hwnd, search_generation(hwnd));
    }

    /// How many notes the finished search visited.
    fn searched_total(hwnd: HWND) -> usize {
        match search_state(hwnd) {
            crate::window::search_view::SearchState::Done { progress, .. } => progress.total,
            other => panic!("the search has not finished: {other:?}"),
        }
    }

    fn search_rows(hwnd: HWND) -> Vec<(String, String)> {
        crate::window::search_view::shown_results(hwnd)
    }

    fn search_row(name: &str, snippet: &str) -> (String, String) {
        (name.to_owned(), snippet.to_owned())
    }

    fn search_selected(hwnd: HWND) -> Option<usize> {
        app_mut(hwnd).sidebar.as_ref().unwrap().search.list.selected
    }

    fn selected_name(hwnd: HWND) -> Option<String> {
        let index = search_selected(hwnd)?;
        search_rows(hwnd).get(index).map(|(name, _)| name.clone())
    }

    fn stray_hit(name: &str) -> crate::library::text_search::TextHit {
        crate::library::text_search::TextHit {
            path: PathBuf::from(format!("{name}.md")),
            name: name.to_owned(),
            folder: String::new(),
            snippet: crate::search::Snippet {
                text: format!("{name} needle"),
                highlight: name.len() + 1..name.len() + 7,
            },
            stamp: None,
        }
    }

    #[test]
    fn the_search_view_finds_note_text_shows_folders_and_opens_the_preview_tab() {
        // Break caught: a search over names instead of text, results without their folder, or
        // Enter opening a normal tab instead of the preview tab.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("search-view");
        scratch.note("Alpha.md", "the alpha plan");
        scratch.note("beta.md", "nothing here");
        scratch.note("gamma.md", "Alphabet soup");
        std::fs::create_dir_all(scratch.folder().join("sub")).unwrap();
        scratch.note(r"sub\notes.md", "  alpha, indented");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        scratch.install(window.hwnd);
        crate::window::side_panel::show_view(window.hwnd, crate::config::SidebarView::Search, true);
        assert_eq!(crate::window::search_view::status(window.hwnd), None);

        search_for(window.hwnd, "alpha");
        assert_eq!(
            search_rows(window.hwnd),
            vec![
                search_row("Alpha", "the alpha plan"),
                search_row("gamma", "Alphabet soup"),
                search_row("notes", "alpha, indented"),
            ]
        );
        let results = &app_mut(window.hwnd).sidebar.as_ref().unwrap().search.results;
        assert_eq!(results[0].folder, "");
        assert_eq!(results[2].folder, "sub");
        assert_eq!(
            crate::window::search_view::summary(window.hwnd),
            Some(("3 notes".to_owned(), false))
        );
        crate::window::search_view::open_selected(window.hwnd, super::OpenMode::Preview, false);
        let active = app_mut(window.hwnd).tabs.active().unwrap();
        assert_eq!(
            active.path.as_deref(),
            Some(scratch.folder().join("Alpha.md").as_path())
        );
        let active_id = active.id;
        assert_eq!(app_mut(window.hwnd).tabs.preview_id(), Some(active_id));

        search_for(window.hwnd, "zzz");
        assert_eq!(
            crate::window::search_view::summary(window.hwnd),
            Some((crate::window::search_view::NO_MATCH.to_owned(), false))
        );
    }

    #[test]
    fn the_search_query_survives_a_view_switch_but_not_a_notebook_switch() {
        // Break caught: the query lost whenever another view is shown, or kept (with results
        // from the old notebook, or its search still reading) after a different notebook opens.
        let _scintilla = load_native_scintilla();
        let first = LibraryScratch::new("search-keep-a");
        first.note("plan.md", "the plan");
        let second = LibraryScratch::new("search-keep-b");
        second.note("other.md", "the plan too");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        first.install(window.hwnd);
        use crate::config::SidebarView;
        crate::window::side_panel::show_view(window.hwnd, SidebarView::Search, true);
        search_for(window.hwnd, "plan");
        crate::window::side_panel::show_view(window.hwnd, SidebarView::Notebook, false);
        crate::window::side_panel::show_view(window.hwnd, SidebarView::Search, false);
        assert_eq!(search_rows(window.hwnd).len(), 1);

        crate::window::text_search_host::run_now(window.hwnd);
        let flag = crate::window::text_search_host::cancel_flag(window.hwnd).unwrap();
        second.install(window.hwnd);
        assert!(flag.load(Ordering::Relaxed), "the notebook change cancelled it");
        assert!(search_rows(window.hwnd).is_empty());
        assert_eq!(
            search_state(window.hwnd),
            crate::window::search_view::SearchState::Idle
        );
        let edit = crate::window::search_view::edit_hwnd(window.hwnd).unwrap();
        assert_eq!(
            unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetWindowTextLengthW(edit) },
            0
        );
        // A late batch of the cancelled search shows nothing.
        pump_posted_messages(window.hwnd);
        assert!(search_rows(window.hwnd).is_empty());
    }

    #[test]
    fn a_hidden_search_view_searches_again_only_once_it_shows() {
        // Break caught: every library refresh re-running a query nobody sees, or the results
        // staying stale when the Search view comes back.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("search-stale");
        scratch.note("plan.md", "plan");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        scratch.install(window.hwnd);
        use crate::config::SidebarView;
        // The box is made when the Search view first shows, not with the sidebar.
        assert!(crate::window::search_view::edit_hwnd(window.hwnd).is_none());
        crate::window::side_panel::show_view(window.hwnd, SidebarView::Search, true);
        assert!(crate::window::search_view::edit_hwnd(window.hwnd).is_some());
        search_for(window.hwnd, "pl");
        assert_eq!(search_rows(window.hwnd).len(), 1);

        crate::window::side_panel::show_view(window.hwnd, SidebarView::Notebook, false);
        let added = scratch.note("planning.md", "planning");
        crate::window::library_host::with_state(window.hwnd, |state| state.add_note(&added));
        let before = search_generation(window.hwnd);
        crate::window::side_panel::refresh(window.hwnd);
        assert_eq!(
            search_generation(window.hwnd),
            before,
            "a hidden Search view is not searched again"
        );
        assert_eq!(search_rows(window.hwnd).len(), 1);
        crate::window::side_panel::show_view(window.hwnd, SidebarView::Search, false);
        wait_for_search(window.hwnd, before);
        assert_eq!(
            search_rows(window.hwnd),
            vec![search_row("plan", "plan"), search_row("planning", "planning")]
        );
    }

    #[test]
    fn saving_a_listed_note_keeps_the_search_selection_and_does_not_rebuild_the_tree() {
        // Break caught: every save (autosave included) rebuilding the sidebar, re-running the
        // search, or snapping the Search selection back to the first result.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("search-save-keeps");
        scratch.note("plan.md", "plan a");
        let planning = scratch.note("planning.md", "plan b");
        scratch.note("plans.md", "plan c");
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        scratch.install(window.hwnd);
        use crate::config::SidebarView;
        crate::window::side_panel::show_view(window.hwnd, SidebarView::Search, true);
        search_for(window.hwnd, "plan");
        let results = search_rows(window.hwnd);
        assert_eq!(results.len(), 3);
        let index = results
            .iter()
            .position(|(name, _)| name == "planning")
            .unwrap();
        assert_ne!(index, 0);
        app_mut(window.hwnd)
            .sidebar
            .as_mut()
            .unwrap()
            .search
            .list
            .selected = Some(index);
        super::open_path(window.hwnd, &planning).unwrap();
        pump_posted_messages(window.hwnd);
        let rebuilds = notebook_view(window.hwnd).rebuilds;
        let searches = search_generation(window.hwnd);

        editor.set_text("edited plan").unwrap();
        assert!(super::save_active_document(window.hwnd));
        assert_eq!(std::fs::read_to_string(&planning).unwrap(), "edited plan");
        assert_eq!(search_selected(window.hwnd), Some(index), "kept by the save");
        assert_eq!(
            notebook_view(window.hwnd).rebuilds,
            rebuilds,
            "a save of a listed note changes no row"
        );
        // A refresh with the same notes runs nothing (spec §7).
        crate::window::side_panel::refresh(window.hwnd);
        assert_eq!(search_generation(window.hwnd), searches, "no search re-ran");

        // The same query run again keeps the selection by path.
        let before = search_generation(window.hwnd);
        crate::window::text_search_host::run_now(window.hwnd);
        wait_for_search(window.hwnd, before);
        assert_eq!(search_selected(window.hwnd), Some(index), "kept by a re-run");
        // A new query selects the same note again once it arrives.
        search_for(window.hwnd, "pla");
        assert_eq!(selected_name(window.hwnd).as_deref(), Some("planning"));
    }

    #[test]
    fn typing_waits_for_the_debounce_and_gives_sorted_results_with_the_selection_kept_by_path() {
        // Break caught: a search per keystroke, results in the order the worker found them, or
        // results arriving above the selected row moving the selection to another note.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("search-debounce");
        scratch.note("c10.md", "needle");
        scratch.note("b.md", "a needle here");
        scratch.note("c9.md", "needle");
        std::fs::create_dir_all(scratch.folder().join("sub")).unwrap();
        scratch.note(r"sub\b.md", "needle too");
        scratch.note("d.md", "no match");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        scratch.install(window.hwnd);
        crate::window::side_panel::show_view(window.hwnd, crate::config::SidebarView::Search, true);

        type_into_search(window.hwnd, "needle");
        // The keystroke only restarted the timer: nothing has run yet.
        assert!(search_rows(window.hwnd).is_empty());
        assert_eq!(
            search_state(window.hwnd),
            crate::window::search_view::SearchState::Idle
        );
        assert!(crate::window::text_search_host::cancel_flag(window.hwnd).is_none());
        wait_for_search(window.hwnd, search_generation(window.hwnd));
        assert_eq!(
            search_rows(window.hwnd),
            vec![
                search_row("b", "a needle here"),
                search_row("b", "needle too"),
                search_row("c9", "needle"),
                search_row("c10", "needle"),
            ]
        );
        let folders = app_mut(window.hwnd)
            .sidebar
            .as_ref()
            .unwrap()
            .search
            .results
            .iter()
            .map(|result| result.folder.clone())
            .collect::<Vec<_>>();
        assert_eq!(folders, ["", "sub", "", ""]);
        assert_eq!(
            crate::window::search_view::summary(window.hwnd),
            Some(("4 notes".to_owned(), false))
        );

        // c9 is selected; a new note that sorts first arrives with the re-run.
        app_mut(window.hwnd)
            .sidebar
            .as_mut()
            .unwrap()
            .search
            .list
            .selected = Some(2);
        let added = scratch.note("a.md", "needle first");
        crate::window::library_host::with_state(window.hwnd, |state| state.add_note(&added));
        let before = search_generation(window.hwnd);
        crate::window::side_panel::refresh(window.hwnd);
        wait_for_search(window.hwnd, before);
        assert_eq!(search_rows(window.hwnd)[0], search_row("a", "needle first"));
        assert_eq!(selected_name(window.hwnd).as_deref(), Some("c9"));
        assert_eq!(search_selected(window.hwnd), Some(3));
    }

    #[test]
    fn a_batch_from_an_older_generation_is_dropped() {
        // Break caught: a slow batch from the previous query landing after the new one began, so
        // the list flickers back to rows the new query never matched.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("search-generation");
        scratch.note("a.md", "needle");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        scratch.install(window.hwnd);
        crate::window::side_panel::show_view(window.hwnd, crate::config::SidebarView::Search, true);
        search_for(window.hwnd, "needle");
        let shown = vec![search_row("a", "needle")];
        assert_eq!(search_rows(window.hwnd), shown);

        let current = search_generation(window.hwnd);
        let stale = crate::window::text_search_host::test_batch(
            current.wrapping_sub(1),
            vec![stray_hit("old")],
            None,
        );
        crate::window::text_search_host::batch_arrived(window.hwnd, stale);
        assert_eq!(search_rows(window.hwnd), shown, "dropped when handled directly");
        let stale = crate::window::text_search_host::test_batch(
            current.wrapping_sub(1),
            vec![stray_hit("older")],
            Some(crate::library::text_search::RunEnd::Completed),
        );
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(
                window.hwnd,
                crate::window::WM_FASTPAD_TEXT_SEARCH_BATCH,
                0,
                stale,
            );
        }
        pump_posted_messages(window.hwnd);
        assert_eq!(search_rows(window.hwnd), shown, "and when it comes through the queue");

        // A keystroke cancels the running search, so its late batches are stale too.
        type_into_search(window.hwnd, "needles");
        let late = crate::window::text_search_host::test_batch(current, vec![stray_hit("late")], None);
        crate::window::text_search_host::batch_arrived(window.hwnd, late);
        assert_eq!(search_rows(window.hwnd), shown);
        // The current generation's batch is the one that shows.
        let now = crate::window::text_search_host::test_batch(
            search_generation(window.hwnd),
            vec![stray_hit("b")],
            None,
        );
        crate::window::text_search_host::batch_arrived(window.hwnd, now);
        assert_eq!(search_rows(window.hwnd).len(), 2);
    }

    #[test]
    fn a_dirty_tab_is_searched_as_the_editor_has_it() {
        // Break caught: the search reading an open note from disk, so a phrase typed only in the
        // editor is missed and a phrase deleted in the editor is still found, for the active tab
        // or a tab in the background; or reading a background tab leaving it in the editor.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("search-dirty");
        let a = scratch.note("a.md", "kept on disk only");
        let b = scratch.note("b.md", "plain b");
        scratch.note("c.md", "plain c");
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        scratch.install(window.hwnd);
        super::open_path(window.hwnd, &a).unwrap();
        pump_posted_messages(window.hwnd);
        editor.set_text("typed in the editor only").unwrap();
        super::open_path(window.hwnd, &b).unwrap();
        pump_posted_messages(window.hwnd);
        editor.set_text("b typed too").unwrap();
        // No autosave may write the edits while the test runs.
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::KillTimer(
                window.hwnd,
                crate::window::library_host::AUTOSAVE_TIMER_ID,
            );
        }
        let dirty = app_mut(window.hwnd)
            .tabs
            .documents()
            .filter(|document| document.dirty)
            .count();
        assert_eq!(dirty, 2);

        let overlays = crate::window::text_search_host::dirty_overlays(window.hwnd, &scratch.folder());
        assert_eq!(overlays.len(), 2);
        assert_eq!(
            overlays.get(std::path::Path::new("a.md")).map(String::as_str),
            Some("typed in the editor only"),
            "a background tab"
        );
        assert_eq!(
            overlays.get(std::path::Path::new("b.md")).map(String::as_str),
            Some("b typed too"),
            "the active tab"
        );
        assert_eq!(editor.text().unwrap(), "b typed too", "the active tab is back");

        crate::window::side_panel::show_view(window.hwnd, crate::config::SidebarView::Search, true);
        search_for(window.hwnd, "editor only");
        assert_eq!(
            search_rows(window.hwnd),
            vec![search_row("a", "typed in the editor only")]
        );
        search_for(window.hwnd, "on disk");
        assert_eq!(
            crate::window::search_view::summary(window.hwnd),
            Some((crate::window::search_view::NO_MATCH.to_owned(), false))
        );
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "kept on disk only");
    }

    #[test]
    fn a_longer_plain_query_searches_only_the_previous_hits() {
        // Break caught: every keystroke re-reading the whole notebook, or narrowing kept after a
        // change of options.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("search-narrow");
        scratch.note("a.md", "needle");
        scratch.note("b.md", "needles");
        scratch.note("c.md", "other");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        scratch.install(window.hwnd);
        crate::window::side_panel::show_view(window.hwnd, crate::config::SidebarView::Search, true);
        search_for(window.hwnd, "need");
        assert_eq!(searched_total(window.hwnd), 3);
        search_for(window.hwnd, "needl");
        assert_eq!(searched_total(window.hwnd), 2, "only the notes \"need\" found");
        assert_eq!(search_rows(window.hwnd).len(), 2);
        let before = search_generation(window.hwnd);
        crate::window::search_view::toggle_option(window.hwnd, crate::search::SearchOption::Case);
        wait_for_search(window.hwnd, before);
        assert_eq!(searched_total(window.hwnd), 3, "new options search everything");
    }

    #[test]
    fn an_invalid_regex_shows_its_error_keeps_the_results_and_runs_nothing() {
        // Break caught: a regex typo blanking the list, running a search anyway, or showing no
        // reason.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("search-bad-regex");
        scratch.note("a.md", "ab here");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        scratch.install(window.hwnd);
        crate::window::side_panel::show_view(window.hwnd, crate::config::SidebarView::Search, true);
        search_for(window.hwnd, "ab");
        let before = search_generation(window.hwnd);
        crate::window::search_view::toggle_option(window.hwnd, crate::search::SearchOption::Regex);
        wait_for_search(window.hwnd, before);
        assert!(crate::window::search_view::options(window.hwnd).regex);
        assert_eq!(search_rows(window.hwnd).len(), 1);

        type_into_search(window.hwnd, "(ab");
        pump_until(window.hwnd, || {
            matches!(
                search_state(window.hwnd),
                crate::window::search_view::SearchState::PatternError(_)
            )
        });
        let (message, error) = crate::window::search_view::summary(window.hwnd).unwrap();
        assert!(error, "shown as an error");
        assert!(!message.is_empty());
        assert_eq!(search_rows(window.hwnd).len(), 1, "the previous results stay");
        assert!(crate::window::text_search_host::cancel_flag(window.hwnd).is_none());

        search_for(window.hwnd, "(ab)");
        assert_eq!(
            crate::window::search_view::summary(window.hwnd),
            Some(("1 note".to_owned(), false))
        );
    }

    #[test]
    fn one_character_says_type_at_least_two_and_clears_the_results() {
        // Break caught: a one-letter query reading the whole notebook, or the last query's rows
        // left under a query they don't match.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("search-short");
        scratch.note("a.md", "ab");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        scratch.install(window.hwnd);
        crate::window::side_panel::show_view(window.hwnd, crate::config::SidebarView::Search, true);
        search_for(window.hwnd, "ab");
        assert_eq!(search_rows(window.hwnd).len(), 1);

        type_into_search(window.hwnd, "a");
        assert_eq!(
            search_state(window.hwnd),
            crate::window::search_view::SearchState::TooShort
        );
        assert!(search_rows(window.hwnd).is_empty());
        assert_eq!(
            crate::window::search_view::summary(window.hwnd),
            Some((crate::window::search_view::TOO_SHORT.to_owned(), false))
        );
        assert!(crate::window::text_search_host::cancel_flag(window.hwnd).is_none());
        type_into_search(window.hwnd, "  ");
        assert_eq!(
            search_state(window.hwnd),
            crate::window::search_view::SearchState::Idle
        );
        assert_eq!(crate::window::search_view::summary(window.hwnd), None);
    }

    #[test]
    fn show_with_query_fills_the_box_escapes_it_for_regex_and_runs_at_once() {
        // Break caught: Ctrl+Shift+F's selection waiting out the debounce, or "1+1" searched as
        // a regex (one or more 1s, then 1) when regex is on.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("search-prefill");
        scratch.note("a.md", "costs 1+1 here");
        scratch.note("b.md", "costs 11 here");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        scratch.install(window.hwnd);
        crate::window::side_panel::show_view(window.hwnd, crate::config::SidebarView::Search, true);
        let before = search_generation(window.hwnd);
        crate::window::search_view::show_with_query(window.hwnd, "1+1");
        assert!(
            matches!(
                search_state(window.hwnd),
                crate::window::search_view::SearchState::Running(_)
                    | crate::window::search_view::SearchState::Done { .. }
            ),
            "running without the debounce"
        );
        wait_for_search(window.hwnd, before);
        assert_eq!(search_rows(window.hwnd), vec![search_row("a", "costs 1+1 here")]);

        let before = search_generation(window.hwnd);
        crate::window::search_view::toggle_option(window.hwnd, crate::search::SearchOption::Regex);
        wait_for_search(window.hwnd, before);
        let before = search_generation(window.hwnd);
        crate::window::search_view::show_with_query(window.hwnd, "1+1");
        wait_for_search(window.hwnd, before);
        let (query, options) = crate::window::search_view::current_query(window.hwnd).unwrap();
        assert!(options.regex);
        assert_eq!(query, r"1\+1");
        assert_eq!(search_rows(window.hwnd), vec![search_row("a", "costs 1+1 here")]);
    }

    #[test]
    fn closing_the_window_cancels_a_running_search() {
        // Break caught: a worker reading a large notebook on after its window closed.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("search-destroy");
        for index in 0..200 {
            scratch.note(&format!("n{index}.md"), "needle");
        }
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        scratch.install(window.hwnd);
        crate::window::side_panel::show_view(window.hwnd, crate::config::SidebarView::Search, true);
        type_into_search(window.hwnd, "needle");
        crate::window::text_search_host::run_now(window.hwnd);
        let flag = crate::window::text_search_host::cancel_flag(window.hwnd).expect("running");
        unsafe {
            DestroyWindow(window.hwnd);
        }
        assert!(flag.load(Ordering::Relaxed));
    }
```

`Ordering` here is the `std::sync::atomic::Ordering` that `mod tests` already imports; `DestroyWindow` and `PathBuf` are already imported there too.

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test --lib -- text_search_host:: search_view:: the_search_view_finds_note_text the_search_query_survives a_hidden_search_view saving_a_listed_note typing_waits_for_the_debounce a_batch_from_an_older_generation_is_dropped a_dirty_tab_is_searched_as_the_editor_has_it a_longer_plain_query an_invalid_regex one_character_says show_with_query_fills closing_the_window_cancels --test-threads=1`
Expected: a compile error, because `text_search_host`, `SearchState`, `notice_text`, `summary_text` and the new test helpers don't exist yet.

- [ ] **Step 4: Add the message, the module and the App field**

In `src/window/messages.rs`, after the `WM_FASTPAD_NOTEBOOK_CHECKED` line, add:

```rust
// Not part of the deferred chain: a text search worker's batch of hits, as a `Box` the receiver
// frees. A post that fails because the window is gone is freed on the worker.
pub const WM_FASTPAD_TEXT_SEARCH_BATCH: u32 = WM_APP + 13;
```

In `src/window/mod.rs`, add after `pub(crate) mod sidebar_accessibility;`:

```rust
pub(crate) mod text_search_host;
```

and in the `pub use messages::{...}` list, replace `WM_FASTPAD_START_IPC,` with `WM_FASTPAD_START_IPC, WM_FASTPAD_TEXT_SEARCH_BATCH,` (then `cargo fmt` wraps it).

In `src/app.rs`, after the `library` field of `App`, add:

```rust
    /// The Search view's text search: its debounce, generation, cancel flag and narrowing record.
    pub(crate) text_search: crate::window::text_search_host::TextSearchHost,
```

and in `App::new`, after `library: crate::window::library_host::LibraryHost::new(process_start),`, add:

```rust
            text_search: Default::default(),
```

- [ ] **Step 5: Write `text_search_host.rs`**

Create `src/window/text_search_host.rs`, above the `mod tests` of Step 1:

```rust
//! Text search scheduling (note-search spec §7): the 150 ms debounce, the generation that makes a
//! late batch stale, the cancel flag, the worker thread and the narrowing record. The worker runs
//! `library::text_search::run` and posts its batches to the main window, which hands the current
//! generation's to the Search view (`search_view::apply_batch`).
//!
//! The timer lives on the main window, as `LIBRARY_WRITE_TIMER_ID` does, not on the panel as spec
//! §7 said: every other timer is the main window's, and the panel goes away with notes mode.

use crate::document::DocumentId;
use crate::library::text_search::{self, Progress, RunEnd, SearchNote, TextHit};
use crate::library::{self, NoteEntry};
use crate::search::{MatchOptions, Matcher};
use crate::window::{library_host, search_view};
use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use windows_sys::Win32::Foundation::{HWND, LPARAM};
use windows_sys::Win32::UI::WindowsAndMessaging::{KillTimer, PostMessageW, SetTimer};

pub(crate) const TEXT_SEARCH_TIMER_ID: usize = 0x4650_5453;
pub(crate) const DEBOUNCE_MS: u32 = 150;
/// The shortest query that runs, in characters (spec §4).
pub(crate) const MIN_QUERY_CHARS: usize = 2;

/// One `WM_FASTPAD_TEXT_SEARCH_BATCH` payload, posted as `Box::into_raw` in `lparam`.
/// `batch_arrived` frees it; a post that fails is freed on the worker.
#[derive(Debug)]
pub(crate) struct SearchBatch {
    pub generation: u64,
    pub hits: Vec<TextHit>,
    pub progress: Progress,
    /// `Some` only on the last batch of a run that was not cancelled.
    pub end: Option<RunEnd>,
}

/// What the last completed search found, so a longer plain query visits only those notes.
#[derive(Clone, Debug)]
struct Narrowing {
    query: String,
    options: MatchOptions,
    paths: Vec<PathBuf>,
    /// The `list_mark` of the note list it searched and the `overlay_mark` of the tab texts it
    /// read. A note added or removed, or an edit in a dirty tab, makes the old hits unsafe.
    list: u64,
    overlays: u64,
}

/// The search that is running, recorded as `Narrowing` if it completes.
#[derive(Clone, Debug)]
struct Running {
    query: String,
    options: MatchOptions,
    list: u64,
    overlays: u64,
}

#[derive(Debug, Default)]
pub(crate) struct TextSearchHost {
    /// Bumped by every start and every cancel: a batch of any other generation is stale.
    generation: u64,
    cancel: Option<Arc<AtomicBool>>,
    previous: Option<Narrowing>,
    running: Option<Running>,
    /// The `list_mark` of the note list the last search started over, so a library change runs
    /// the query again only when a note was added, removed or renamed (spec §7). `None` after a
    /// reload, which always runs it again.
    list: Option<u64>,
}

fn with_host<R>(hwnd: HWND, f: impl FnOnce(&mut TextSearchHost) -> R) -> Option<R> {
    unsafe { super::main_window::app_ptr(hwnd) }.map(|mut app| f(&mut unsafe { app.as_mut() }.text_search))
}

/// Whether `query` runs: at least `MIN_QUERY_CHARS` characters, not all white space.
pub(crate) fn searchable(query: &str) -> bool {
    query.chars().count() >= MIN_QUERY_CHARS && !query.trim().is_empty()
}

/// How the note list compares ignoring case (`library::same_path`).
fn path_key(path: &Path) -> String {
    path.to_string_lossy().to_lowercase()
}

/// A mark of the note list: the paths in order and whether each is online-only. Sizes and times
/// are left out, so FastPad's own save of a listed note is no change.
fn list_mark(notes: &[NoteEntry]) -> u64 {
    let mut hasher = DefaultHasher::new();
    notes.len().hash(&mut hasher);
    for note in notes {
        note.path.hash(&mut hasher);
        note.online_only.hash(&mut hasher);
    }
    hasher.finish()
}

/// A mark of the dirty tabs' texts, the same whatever order the map iterates in.
fn overlay_mark(overlays: &HashMap<PathBuf, String>) -> u64 {
    overlays
        .iter()
        .fold(overlays.len() as u64, |mark, (path, text)| {
            let mut hasher = DefaultHasher::new();
            path_key(path).hash(&mut hasher);
            text.hash(&mut hasher);
            mark.wrapping_add(hasher.finish())
        })
}

/// Whether `query` may visit only `previous`'s hits: a plain query without whole word (a longer
/// whole word can match where the shorter one was no whole word), the same options, containing
/// the previous query, over the same tab texts. The note list is checked by the caller.
fn narrows(previous: &Narrowing, query: &str, options: MatchOptions, overlays: u64) -> bool {
    !options.regex
        && !options.whole_word
        && previous.options == options
        && query.contains(previous.query.as_str())
        && previous.overlays == overlays
}

/// A keystroke in the box: cancels any running search and restarts the debounce.
pub(crate) fn schedule(hwnd: HWND) {
    cancel(hwnd);
    unsafe {
        SetTimer(hwnd, TEXT_SEARCH_TIMER_ID, DEBOUNCE_MS, None);
    }
}

/// `WM_TIMER` for `TEXT_SEARCH_TIMER_ID`: the debounce ended.
pub(crate) fn timer(hwnd: HWND) {
    run_now(hwnd);
}

/// Cancels, kills the timer, and starts the search for the view's query and options now. It does
/// nothing for a query that is too short or all white space, with no notebook, or while the
/// library loads (`LIBRARY_READY` runs it through `library_changed`). A bad pattern shows its
/// error and runs nothing.
pub(crate) fn run_now(hwnd: HWND) {
    cancel(hwnd);
    let Some((query, options)) = search_view::current_query(hwnd) else {
        return;
    };
    if !searchable(&query) {
        return;
    }
    // Compiled first, so a bad pattern shows even while the notebook loads or none is open.
    let matcher = match Matcher::new(&query, options) {
        Ok(matcher) => matcher,
        Err(error) => {
            search_view::set_pattern_error(hwnd, Some(error.message));
            return;
        }
    };
    let Some(notebook) = library_host::folder(hwnd) else {
        return;
    };
    if library_host::with_state(hwnd, |_| ()).is_none() {
        return;
    }
    // Read with nothing of the App borrowed: a background dirty tab is swapped into the editor.
    let overlays = dirty_overlays(hwnd, &notebook);
    let overlays_mark = overlay_mark(&overlays);
    let candidate = with_host(hwnd, |host| host.previous.clone())
        .flatten()
        .filter(|previous| narrows(previous, &query, options, overlays_mark));
    let Some((notes, list)) = library_host::with_state(hwnd, |state| {
        let list = list_mark(&state.notes);
        let keep: Option<HashSet<String>> = candidate
            .filter(|previous| previous.list == list)
            .map(|previous| previous.paths.iter().map(|path| path_key(path)).collect());
        let notes = state
            .notes
            .iter()
            .filter(|note| {
                keep.as_ref()
                    .is_none_or(|keep| keep.contains(&path_key(&note.path)))
            })
            .map(|note| SearchNote {
                path: note.path.clone(),
                size: note.size,
                online_only: note.online_only,
            })
            .collect::<Vec<_>>();
        (notes, list)
    }) else {
        return;
    };
    let total = notes.len();
    let Some((generation, cancel)) = with_host(hwnd, |host| {
        host.generation = host.generation.wrapping_add(1);
        let flag = Arc::new(AtomicBool::new(false));
        host.cancel = Some(Arc::clone(&flag));
        host.list = Some(list);
        host.running = Some(Running {
            query: query.clone(),
            options,
            list,
            overlays: overlays_mark,
        });
        (host.generation, flag)
    }) else {
        return;
    };
    search_view::begin_search(hwnd, &query, total);
    spawn(
        hwnd,
        Job {
            notebook,
            notes,
            overlays,
            matcher,
            generation,
            cancel,
        },
    );
}

/// Everything the worker owns. It is dropped, note copy and overlays included, when it ends.
struct Job {
    notebook: PathBuf,
    notes: Vec<SearchNote>,
    overlays: HashMap<PathBuf, String>,
    matcher: Matcher,
    generation: u64,
    cancel: Arc<AtomicBool>,
}

fn spawn(hwnd: HWND, job: Job) {
    let target = hwnd as isize;
    std::thread::spawn(move || {
        let Job {
            notebook,
            notes,
            overlays,
            matcher,
            generation,
            cancel,
        } = job;
        let post = |batch: SearchBatch| -> bool {
            let payload = Box::into_raw(Box::new(batch));
            let posted = unsafe {
                PostMessageW(
                    target as HWND,
                    crate::window::WM_FASTPAD_TEXT_SEARCH_BATCH,
                    0,
                    payload as isize,
                )
            } != 0;
            if !posted {
                // The window is gone: nothing else will free it.
                drop(unsafe { Box::from_raw(payload) });
            }
            posted
        };
        let mut last = Progress::default();
        let mut sink = |hits: Vec<TextHit>, progress: Progress| {
            last = progress;
            let batch = SearchBatch {
                generation,
                hits,
                progress,
                end: None,
            };
            if !post(batch) {
                cancel.store(true, Ordering::Relaxed);
            }
        };
        let end = text_search::run(&notebook, &notes, &overlays, &matcher, &cancel, &mut sink);
        if end != RunEnd::Cancelled && !cancel.load(Ordering::Relaxed) {
            post(SearchBatch {
                generation,
                hits: Vec::new(),
                progress: last,
                end: Some(end),
            });
        }
    });
}

/// `WM_FASTPAD_TEXT_SEARCH_BATCH`: takes the box back. A batch of any generation but the current
/// one is dropped. A completed search is recorded for narrowing.
pub(crate) fn batch_arrived(hwnd: HWND, lparam: LPARAM) {
    if lparam == 0 {
        return;
    }
    let batch = *unsafe { Box::from_raw(lparam as *mut SearchBatch) };
    if with_host(hwnd, |host| host.generation) != Some(batch.generation) {
        return;
    }
    let end = batch.end;
    search_view::apply_batch(hwnd, batch);
    let Some(end) = end else {
        return;
    };
    let paths = if end == RunEnd::Completed {
        search_view::result_paths(hwnd)
    } else {
        Vec::new()
    };
    with_host(hwnd, |host| {
        host.cancel = None;
        host.previous = match (end, host.running.take()) {
            (RunEnd::Completed, Some(run)) => Some(Narrowing {
                query: run.query,
                options: run.options,
                paths,
                list: run.list,
                overlays: run.overlays,
            }),
            _ => None,
        };
    });
}

/// Stops the running search and the debounce. Batches it already posted are stale from here on.
pub(crate) fn cancel(hwnd: HWND) {
    with_host(hwnd, |host| {
        if let Some(flag) = host.cancel.take() {
            flag.store(true, Ordering::Relaxed);
        }
        host.generation = host.generation.wrapping_add(1);
        host.running = None;
    });
    unsafe {
        KillTimer(hwnd, TEXT_SEARCH_TIMER_ID);
    }
}

/// A notebook change or close, or notes mode off: cancels and forgets what the old notebook's
/// searches found.
pub(crate) fn forget(hwnd: HWND) {
    cancel(hwnd);
    with_host(hwnd, |host| {
        host.previous = None;
        host.list = None;
    });
}

/// The library was loaded or rescanned (`library_host::install`): the next library change runs
/// the query again even if no note was added or removed, since the files may have changed.
pub(crate) fn notes_reloaded(hwnd: HWND) {
    with_host(hwnd, |host| {
        host.previous = None;
        host.list = None;
    });
}

/// The shown Search view's library changed (`search_view::library_changed`, `shown`): runs the
/// query again when the note list moved on since the last search.
pub(crate) fn library_changed(hwnd: HWND) {
    let Some((query, _)) = search_view::current_query(hwnd) else {
        return;
    };
    if !searchable(&query) {
        return;
    }
    let Some(list) = library_host::with_state(hwnd, |state| list_mark(&state.notes)) else {
        return;
    };
    if with_host(hwnd, |host| host.list) == Some(Some(list)) {
        return;
    }
    run_now(hwnd);
}

/// The dirty tabs whose file is inside `notebook`, keyed by the path relative to it, with the
/// editor's text. Clean and untitled tabs are left out: the disk has their text, or they are no
/// note. Call it with nothing of the App borrowed.
pub(crate) fn dirty_overlays(hwnd: HWND, notebook: &Path) -> HashMap<PathBuf, String> {
    let dirty: Vec<(DocumentId, PathBuf)> = unsafe { super::main_window::app_ptr(hwnd) }
        .map(|app| {
            unsafe { app.as_ref() }
                .tabs
                .documents()
                .filter(|document| document.dirty)
                .filter_map(|document| {
                    let path = document.path.as_deref()?;
                    library::is_inside(notebook, path)
                        .then(|| (document.id, library::record_path(notebook, path)))
                })
                .collect()
        })
        .unwrap_or_default();
    let mut overlays = HashMap::with_capacity(dirty.len());
    for (id, relative) in dirty {
        if let Some(text) = super::main_window::document_text(hwnd, id) {
            overlays.insert(relative, text);
        }
    }
    overlays
}

#[cfg(test)]
pub(crate) fn generation(hwnd: HWND) -> u64 {
    with_host(hwnd, |host| host.generation).unwrap_or(0)
}

/// The running search's cancel flag, `None` while nothing runs.
#[cfg(test)]
pub(crate) fn cancel_flag(hwnd: HWND) -> Option<Arc<AtomicBool>> {
    with_host(hwnd, |host| host.cancel.clone()).flatten()
}

/// A boxed batch, as the worker posts it.
#[cfg(test)]
pub(crate) fn test_batch(generation: u64, hits: Vec<TextHit>, end: Option<RunEnd>) -> LPARAM {
    Box::into_raw(Box::new(SearchBatch {
        generation,
        hits,
        progress: Progress::default(),
        end,
    })) as LPARAM
}
```

- [ ] **Step 6: Read a tab's text, and wire the timer, the message and the cancels**

In `src/window/main_window.rs`, after `read_inactive_text` (it ends with `restored?; text }`), add:

```rust
/// The text of tab `id` as the editor has it, for the Search view's overlays. A background tab is
/// swapped into the editor and back (`read_inactive_text`). `None` without an editor or that tab,
/// for a background tab while a file is being populated (the swap would end the population), or
/// when Scintilla can't be read. Call it with nothing of the App borrowed.
pub(crate) fn document_text(hwnd: HWND, id: DocumentId) -> Option<String> {
    let identity = unsafe { window_identity(hwnd) }?;
    let (editor, inactive) = unsafe { app_ptr(hwnd) }.and_then(|app| {
        let app = unsafe { app.as_ref() };
        let editor = app.editor.clone()?;
        let active = app.tabs.active()?;
        let target = app.tabs.document(id)?;
        if target.id == active.id {
            return Some((editor, None));
        }
        if app.populating_file {
            return None;
        }
        Some((editor, Some((target.handle.clone(), active.handle.clone()))))
    })?;
    match inactive {
        None => editor.text().ok(),
        Some((target, active)) => {
            read_inactive_text(hwnd, &identity, &editor, &target, &active).ok()
        }
    }
}
```

In `wndproc`, replace the `WM_DESTROY` arm with:

```rust
        WM_DESTROY => {
            // A running text search stops reading: its posts would fail from here on anyway.
            crate::window::text_search_host::cancel(hwnd);
            unsafe {
                KillTimer(hwnd, crate::recovery::RECOVERY_TIMER_ID);
                KillTimer(hwnd, crate::window::preview_host::PREVIEW_TIMER_ID);
                KillTimer(hwnd, crate::window::library_host::LIBRARY_WRITE_TIMER_ID);
                KillTimer(hwnd, crate::window::library_host::AUTOSAVE_TIMER_ID);
                PostQuitMessage(0);
            }
            0
        }
```

After the `WM_TIMER if wparam == crate::window::library_host::AUTOSAVE_TIMER_ID` arm, add:

```rust
        WM_TIMER if wparam == crate::window::text_search_host::TEXT_SEARCH_TIMER_ID => {
            crate::window::text_search_host::timer(hwnd);
            0
        }
```

In the `_ =>` arm, after the `WM_FASTPAD_NOTEBOOK_CHECKED` block, add:

```rust
            if message == crate::window::WM_FASTPAD_TEXT_SEARCH_BATCH {
                crate::window::text_search_host::batch_arrived(hwnd, lparam);
                return 0;
            }
```

In `src/window/library_host.rs`, in `install`, replace:

```rust
    save_local(
        hwnd,
        LocalWrite {
            wait: false,
            force: rewrite,
        },
    );
    super::side_panel::refresh(hwnd);
```

with:

```rust
    save_local(
        hwnd,
        LocalWrite {
            wait: false,
            force: rewrite,
        },
    );
    // A load or rescan may have seen outside edits: the Search view's query runs again.
    crate::window::text_search_host::notes_reloaded(hwnd);
    super::side_panel::refresh(hwnd);
```

In `notes_mode_changed`, replace the closing block:

```rust
    unsafe {
        KillTimer(hwnd, LIBRARY_WRITE_TIMER_ID);
    }
    host(hwnd, |host| {
        host.state = None;
        host.folder = None;
        host.generation = host.generation.wrapping_add(1);
        host.scanning = false;
    });
}
```

with:

```rust
    unsafe {
        KillTimer(hwnd, LIBRARY_WRITE_TIMER_ID);
    }
    // The Search view goes with the sidebar: its search stops and its record is dropped.
    crate::window::text_search_host::forget(hwnd);
    host(hwnd, |host| {
        host.state = None;
        host.folder = None;
        host.generation = host.generation.wrapping_add(1);
        host.scanning = false;
    });
}
```

- [ ] **Step 7: Switch the Search view to text hits**

In `src/window/search_view.rs`:

Replace the module comment (lines 1–4) with:

```rust
//! The Search view (note-search spec §4): a search box over the text of the open notebook's
//! notes, a summary line, the matching notes with their folders and the first match, and a
//! status line. `text_search_host` runs the search; this view shows its batches. Enter or a click
//! opens a result following the preview-tab rules (sidebar spec §6.4). The box's placeholder is
//! painted the way the find bar paints its placeholder. FastPad has no ComCtl32 v6 manifest, so
//! `EM_SETCUEBANNER` would show nothing.
```

Replace the imports from `use crate::config::SidebarView;` through `use std::path::{Path, PathBuf};` (lines 6–16) with:

```rust
use crate::config::SidebarView;
use crate::library::model::same_path;
use crate::library::text_search::{self, Progress, RunEnd, TextHit, hit_cmp};
use crate::platform::{last_error, wide_null};
use crate::search::{MatchOptions, SearchOption, escape};
use crate::window::library_host;
use crate::window::main_window::OpenMode;
use crate::window::notebook_view::LOAD_FAILED;
use crate::window::palette::Palette;
use crate::window::panel::{create_child, fill, inset, scale, text_height};
use crate::window::row_list::{self, ListKey, RowListState, RowLook, row_foreground};
use crate::window::side_panel::{self, ViewPaint, draw_text, point_of};
use crate::window::text_search_host::{self, SearchBatch};
use std::path::{Path, PathBuf};
```

Replace the constants `RESULT_LIMIT` through `LOADING` (lines 40–43) with:

```rust
pub(crate) const NO_NOTEBOOK: &str = "Open a notebook to search it.";
pub(crate) const NO_MATCH: &str = "No notes match.";
pub(crate) const LOADING: &str = "Loading\u{2026}";
pub(crate) const TOO_SHORT: &str = "Type at least 2 characters.";
```

Replace `status_text` (lines 63–84) with:

```rust
/// The search's progress, as the summary and status lines read it.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) enum SearchState {
    /// Nothing typed, or only white space.
    #[default]
    Idle,
    /// One character: text search starts at two.
    TooShort,
    Running(Progress),
    Done {
        progress: Progress,
        capped: bool,
    },
    /// The pattern can't run. The previous results stay.
    PatternError(String),
}

/// The line shown instead of the results, if any. `failed` is a notebook whose load failed.
pub(crate) fn notice_text(notebook_open: bool, loaded: bool, failed: bool) -> Option<&'static str> {
    if !notebook_open {
        Some(NO_NOTEBOOK)
    } else if failed {
        Some(LOAD_FAILED)
    } else if !loaded {
        Some(LOADING)
    } else {
        None
    }
}

/// The summary line under the box and whether it is an error: "N notes" while results arrive
/// and when the search is done, "No notes match." for a finished search without one.
pub(crate) fn summary_text(state: &SearchState, results: usize) -> Option<(String, bool)> {
    match state {
        SearchState::Idle => None,
        SearchState::TooShort => Some((TOO_SHORT.to_owned(), false)),
        SearchState::PatternError(message) => Some((message.clone(), true)),
        SearchState::Running(_) => (results > 0).then(|| (note_count(results, false), false)),
        SearchState::Done { capped, .. } => {
            let text = if results == 0 {
                NO_MATCH.to_owned()
            } else {
                note_count(results, *capped)
            };
            Some((text, false))
        }
    }
}

/// The status line at the bottom: progress while the search runs, what it skipped once done.
/// `None` hides it.
pub(crate) fn status_text(state: &SearchState) -> Option<String> {
    match state {
        SearchState::Running(progress) => Some(format!(
            "Searching\u{2026} {} of {}",
            thousands(progress.visited),
            thousands(progress.total)
        )),
        SearchState::Done { progress, .. } => match progress.skipped_total() {
            0 => None,
            1 => Some("1 note wasn't searched".to_owned()),
            skipped => Some(format!("{} notes weren't searched", thousands(skipped))),
        },
        _ => None,
    }
}

fn note_count(count: usize, capped: bool) -> String {
    if capped {
        format!("{}+ notes", thousands(text_search::RESULT_CAP))
    } else if count == 1 {
        "1 note".to_owned()
    } else {
        format!("{} notes", thousands(count))
    }
}

/// `value` with a comma between each group of three digits.
fn thousands(value: usize) -> String {
    let digits = value.to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    grouped
}
```

Replace `struct SearchView` and its `new` (lines 106–154) with:

```rust
#[derive(Debug)]
pub(crate) struct SearchView {
    panel: HWND,
    /// The search box, made the first time the Search view shows a notebook (`layout`).
    edit: Option<HWND>,
    /// Making the box failed and was reported; it is not tried again.
    edit_failed: bool,
    brush: HBRUSH,
    colors: Palette,
    notebook: Option<PathBuf>,
    /// The library's state has loaded.
    loaded: bool,
    /// The notebook's load failed (`library_host::load_failed`).
    failed: bool,
    /// The library changed while the view was hidden: `shown` checks whether the query must run
    /// again.
    stale: bool,
    /// The query the results are for: the one the last search began with.
    pub(crate) query: String,
    /// Sorted by `hit_cmp`, at most `text_search::RESULT_CAP`.
    pub(crate) results: Vec<TextHit>,
    /// Match case, whole word and regex, for the session.
    pub(crate) options: MatchOptions,
    pub(crate) search: SearchState,
    /// The same query is running again: its first batch replaces the results.
    replace_on_batch: bool,
    /// The note selected when the running search began, selected again when it arrives.
    restore: Option<PathBuf>,
    /// The note the last batch left selected. A different selection at the next batch means the
    /// user moved it, and `restore` gives way.
    selected_by_batch: Option<PathBuf>,
    pub(crate) list: RowListState,
    placeholder: String,
    /// While the scroll thumb is dragged: how far below its top it was grabbed.
    thumb_grab: Option<i32>,
    /// Bumped whenever the results change (`AccessibleView::accessible_generation`).
    order: u64,
}

impl SearchView {
    /// The view for `panel`. Its search box waits until the view first shows a notebook.
    pub(crate) fn new(panel: HWND, dpi: u32) -> Self {
        let colors = Palette::neutral();
        Self {
            panel,
            edit: None,
            edit_failed: false,
            brush: unsafe { CreateSolidBrush(colors.editor_background) },
            colors,
            notebook: None,
            loaded: false,
            failed: false,
            stale: false,
            query: String::new(),
            results: Vec::new(),
            options: MatchOptions::default(),
            search: SearchState::Idle,
            replace_on_batch: false,
            restore: None,
            selected_by_batch: None,
            list: RowListState::new(scale(ROW_AT_96_DPI, dpi)),
            placeholder: placeholder(None),
            thumb_grab: None,
            order: 0,
        }
    }
```

Replace `set_results` and the `status` method (lines 188–231) with:

```rust
    fn path_at(&self, index: usize) -> Option<PathBuf> {
        self.results.get(index).map(|result| result.path.clone())
    }

    fn position(&self, path: &Path) -> Option<usize> {
        self.results.iter().position(|result| result.path == path)
    }

    /// Empties the list and forgets its selection and scroll.
    fn clear_results(&mut self) {
        if !self.results.is_empty() {
            self.order = self.order.wrapping_add(1);
        }
        self.results.clear();
        self.list.set_count(0);
        self.list.top = 0;
        self.list.selected = None;
        self.replace_on_batch = false;
        self.restore = None;
        self.selected_by_batch = None;
    }

    /// A search for `query` over `total` notes began. The same query again keeps the results
    /// until its first batch; a new one clears them. Either way the selected note is remembered,
    /// to be selected again when it arrives.
    fn begin(&mut self, query: &str, total: usize) {
        let selected = self.list.selected.and_then(|index| self.path_at(index));
        if self.query == query && !self.results.is_empty() {
            self.replace_on_batch = true;
        } else {
            self.clear_results();
            self.query = query.to_owned();
        }
        self.restore = selected;
        self.selected_by_batch = None;
        self.search = SearchState::Running(Progress {
            total,
            ..Progress::default()
        });
    }

    /// Puts `batch`'s hits in sorted place, keeping the selection and the top row by path, for a
    /// list area `height` tall. Reports whether anything painted changed: a row in view, the
    /// selection, the scroll, the summary or the status line.
    fn apply(&mut self, batch: SearchBatch, height: i32) -> bool {
        let lines = (self.summary(), self.status_line());
        let before = (self.list.top, self.list.selected);
        let replacing = std::mem::take(&mut self.replace_on_batch);
        let (selected, top) = if replacing {
            (None, None)
        } else {
            (
                self.list.selected.and_then(|index| self.path_at(index)),
                self.path_at(self.list.top),
            )
        };
        if !replacing && selected != self.selected_by_batch {
            // The user moved the selection since the last batch: it stays where they put it.
            self.restore = None;
        }
        let mut rows_changed = replacing;
        if replacing {
            self.results.clear();
            self.list.top = 0;
            self.list.selected = None;
        }
        let rows_in_view = (height.max(0) as usize).div_ceil(self.list.row_height.max(1) as usize);
        let in_view = self.list.top + rows_in_view;
        let arrived = !batch.hits.is_empty();
        for hit in batch.hits {
            match self.results.binary_search_by(|probe| hit_cmp(probe, &hit)) {
                Ok(index) => {
                    rows_changed |= index < in_view;
                    self.results[index] = hit;
                }
                Err(index) => {
                    rows_changed |= index < in_view;
                    self.results.insert(index, hit);
                }
            }
        }
        if arrived || replacing {
            self.order = self.order.wrapping_add(1);
        }
        self.list.set_count(self.results.len());
        if let Some(index) = top.and_then(|path| self.position(&path)) {
            self.list.top = index;
        }
        let restored = self.restore.as_deref().and_then(|path| self.position(path));
        if restored.is_some() {
            self.restore = None;
        }
        let index = restored
            .or_else(|| selected.and_then(|path| self.position(&path)))
            .or_else(|| (!self.results.is_empty()).then_some(0));
        match index {
            Some(index) => self.list.select(index, height),
            None => self.list.selected = None,
        }
        self.selected_by_batch = self.list.selected.and_then(|index| self.path_at(index));
        if batch.end.is_some() {
            self.restore = None;
        }
        self.search = match batch.end {
            None => SearchState::Running(batch.progress),
            Some(end) => SearchState::Done {
                progress: batch.progress,
                capped: end == RunEnd::Capped,
            },
        };
        rows_changed
            || (self.list.top, self.list.selected) != before
            || (self.summary(), self.status_line()) != lines
    }

    /// The line shown instead of the results.
    pub(crate) fn notice(&self) -> Option<&'static str> {
        notice_text(self.notebook.is_some(), self.loaded, self.failed)
    }

    pub(crate) fn summary(&self) -> Option<(String, bool)> {
        summary_text(&self.search, self.results.len())
    }

    pub(crate) fn status_line(&self) -> Option<String> {
        status_text(&self.search)
    }
```

In `paint` (lines 233–274), replace the block from `if let Some(status) = self.status() {` through its closing `}` (lines 246–263) with:

```rust
            // One line in place of the list: the notice, or the summary while there is no row.
            // Task 5 gives the summary and the status line places of their own.
            let line_text = self.notice().map(str::to_owned).or_else(|| {
                self.results
                    .is_empty()
                    .then(|| self.summary().map(|(text, _)| text))
                    .flatten()
            });
            if let Some(text) = line_text {
                let area = self.list_area(client, dpi);
                let status_line = RECT {
                    left: client.left + pad,
                    top: area.top,
                    right: client.right - pad,
                    bottom: area.top + scale(ROW_AT_96_DPI, dpi),
                };
                draw_text(
                    paint.hdc,
                    &text,
                    status_line,
                    paint.fonts.text,
                    palette.muted_foreground,
                    line | DT_LEFT | DT_END_ELLIPSIS,
                );
                return;
            }
```

Replace `query_changed` and `library_changed` (lines 392–459) with:

```rust
/// `EN_CHANGE` from the box. A query that can run restarts the debounce and nothing else. One
/// that can't (empty, all white space or one character) cancels the search and clears the list.
pub(crate) fn query_changed(hwnd: HWND) {
    let Some(edit) = with_view(hwnd, |view| view.edit).flatten() else {
        return;
    };
    let query = window_text(edit);
    if text_search_host::searchable(&query) {
        text_search_host::schedule(hwnd);
        return;
    }
    text_search_host::cancel(hwnd);
    let state = if query.trim().is_empty() {
        SearchState::Idle
    } else {
        SearchState::TooShort
    };
    let Some(panel) = with_view(hwnd, |view| {
        view.clear_results();
        view.query = query;
        view.search = state;
        view.panel
    }) else {
        return;
    };
    invalidate(panel);
}

/// Part of `side_panel::refresh`. A new notebook cancels the search and clears the query and the
/// results. The same notebook runs the query again if its notes changed, or, while the view is
/// hidden, checks once it shows again.
pub(crate) fn library_changed(hwnd: HWND) {
    let notebook = library_host::folder(hwnd);
    let loaded = library_host::with_state(hwnd, |_| ()).is_some();
    let failed = library_host::load_failed(hwnd);
    let Some((edit, changed)) = with_view(hwnd, |view| {
        let changed = match (&view.notebook, &notebook) {
            (Some(old), Some(new)) => !same_path(old, new),
            (None, None) => false,
            _ => true,
        };
        view.loaded = loaded;
        view.failed = failed;
        if changed {
            view.notebook = notebook.clone();
            view.placeholder = placeholder(notebook.as_deref());
            view.clear_results();
            view.query.clear();
            view.search = SearchState::Idle;
            view.stale = false;
        }
        (view.edit, changed)
    }) else {
        return;
    };
    if changed {
        text_search_host::forget(hwnd);
        // Clearing the box sends EN_CHANGE, which leaves the view idle.
        if let Some(edit) = edit {
            let empty = wide_null("");
            unsafe {
                SetWindowTextW(edit, empty.as_ptr());
                InvalidateRect(edit, std::ptr::null(), 1);
            }
        }
        layout(hwnd);
    } else if side_panel::current_view(hwnd) == SidebarView::Search {
        text_search_host::library_changed(hwnd);
    } else {
        with_view(hwnd, |view| view.stale = true);
    }
}

/// The box's text and the options, or `None` before the box exists.
pub(crate) fn current_query(hwnd: HWND) -> Option<(String, MatchOptions)> {
    let (edit, options) = with_view(hwnd, |view| Some((view.edit?, view.options))).flatten()?;
    // Read with nothing of the App borrowed: WM_GETTEXT goes through the box's subclass.
    Some((window_text(edit), options))
}

/// `text_search_host::run_now` started a search for `query` over `total` notes.
pub(crate) fn begin_search(hwnd: HWND, query: &str, total: usize) {
    if let Some(panel) = with_view(hwnd, |view| {
        view.begin(query, total);
        view.panel
    }) {
        invalidate(panel);
    }
}

/// A batch of the current search (`text_search_host::batch_arrived`). The panel repaints only if
/// something it shows changed.
pub(crate) fn apply_batch(hwnd: HWND, batch: SearchBatch) {
    let Some(panel) = with_view(hwnd, |view| view.panel) else {
        return;
    };
    let (client, dpi) = geometry(panel);
    let changed = with_view(hwnd, |view| {
        let area = view.list_area(client, dpi);
        view.apply(batch, height(area))
    })
    .unwrap_or(false);
    if changed {
        invalidate(panel);
    }
}

/// Shows a pattern's error in place of the summary, keeping the results, or clears it.
pub(crate) fn set_pattern_error(hwnd: HWND, error: Option<String>) {
    let Some(panel) = with_view(hwnd, |view| {
        match error {
            Some(message) => view.search = SearchState::PatternError(message),
            None if matches!(view.search, SearchState::PatternError(_)) => {
                view.search = SearchState::Idle;
            }
            None => {}
        }
        view.panel
    }) else {
        return;
    };
    invalidate(panel);
}

#[cfg_attr(
    not(test),
    expect(dead_code, reason = "only `show_with_query` calls it, until Ctrl+Shift+F does (Task 6)")
)]
pub(crate) fn options(hwnd: HWND) -> MatchOptions {
    with_view(hwnd, |view| view.options).unwrap_or_default()
}

/// Flips `option` and runs the query again at once.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "the toggle buttons and Alt keys call it (Task 5)")
)]
pub(crate) fn toggle_option(hwnd: HWND, option: SearchOption) {
    let Some(panel) = with_view(hwnd, |view| {
        view.options = view.options.toggled(option);
        view.panel
    }) else {
        return;
    };
    invalidate(panel);
    text_search_host::run_now(hwnd);
}

/// Ctrl+Shift+F with a one-line selection: `text` replaces the box's text (escaped first when
/// regex is on), all of it selected, and the search runs at once. The caller shows the view.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "Ctrl+Shift+F calls it (Task 6)")
)]
pub(crate) fn show_with_query(hwnd: HWND, text: &str) {
    let text = if options(hwnd).regex {
        escape(text)
    } else {
        text.to_owned()
    };
    let Some(edit) = ensure_edit(hwnd) else {
        return;
    };
    let wide = wide_null(&text);
    // Setting the text sends EN_CHANGE, which starts the debounce; `run_now` replaces it.
    unsafe {
        SetWindowTextW(edit, wide.as_ptr());
        SendMessageW(edit, EM_SETSEL, 0, -1);
    }
    text_search_host::run_now(hwnd);
}

/// The listed notes' paths, relative to the notebook, in list order.
pub(crate) fn result_paths(hwnd: HWND) -> Vec<PathBuf> {
    with_view(hwnd, |view| {
        view.results
            .iter()
            .map(|result| result.path.clone())
            .collect()
    })
    .unwrap_or_default()
}
```

`options` carries the `expect` too: until Task 6 it is used only by `show_with_query` and the tests, and `show_with_query` itself is dead outside the tests. Task 6 makes `show_with_query` live, which makes `options` live too, so Task 6 removes both attributes.

In `shown` (lines 561–587), replace:

```rust
    if with_view(hwnd, |view| std::mem::take(&mut view.stale)).unwrap_or(false) {
        query_changed(hwnd);
    }
```

with:

```rust
    if with_view(hwnd, |view| std::mem::take(&mut view.stale)).unwrap_or(false) {
        text_search_host::library_changed(hwnd);
    }
```

Replace the test helpers `shown_results` and `status` (lines 937–952) with:

```rust
/// The results listed, as (name, snippet text), for in-process tests.
#[cfg(test)]
pub(crate) fn shown_results(hwnd: HWND) -> Vec<(String, String)> {
    with_view(hwnd, |view| {
        view.results
            .iter()
            .map(|result| (result.name.clone(), result.snippet.text.clone()))
            .collect()
    })
    .unwrap_or_default()
}

/// The notice shown instead of the results.
#[cfg(test)]
pub(crate) fn status(hwnd: HWND) -> Option<&'static str> {
    with_view(hwnd, |view| view.notice()).flatten()
}

#[cfg(test)]
pub(crate) fn summary(hwnd: HWND) -> Option<(String, bool)> {
    with_view(hwnd, |view| view.summary()).flatten()
}

#[cfg(test)]
pub(crate) fn search_state(hwnd: HWND) -> SearchState {
    with_view(hwnd, |view| view.search.clone()).unwrap_or_default()
}
```

The `AccessibleView` impl is unchanged: `TextHit` has the `name`, `folder` and `path` it reads. Task 8 changes its names.

- [ ] **Step 8: Run the tests to verify they pass**

Run: `cargo test --lib -- text_search_host:: search_view:: the_search_view_finds_note_text the_search_query_survives a_hidden_search_view saving_a_listed_note typing_waits_for_the_debounce a_batch_from_an_older_generation_is_dropped a_dirty_tab_is_searched_as_the_editor_has_it a_longer_plain_query an_invalid_regex one_character_says show_with_query_fills closing_the_window_cancels at_startup_both_views with_no_notebook_the_search_view a_failed_load --test-threads=1`
Expected: all pass. `at_startup_both_views…`, `with_no_notebook_the_search_view…` and the failed-load test around line 10200 still read `search_view::status`, which is now the notice.

- [ ] **Step 9: Lint, format, commit**

Run: `cargo fmt --all`, then `cargo fmt --all -- --check` and `cargo clippy --all-targets -- -D warnings`.
Expected: no output from the check, and no warnings.

```bash
git add src/window/text_search_host.rs src/window/mod.rs src/window/messages.rs src/app.rs src/window/main_window.rs src/window/library_host.rs src/window/search_view.rs
git commit -m "feat(search): the Search view searches note text on a worker, with a debounce, generations, dirty-tab overlays and narrowing"
```

---

### Task 5: The toggles, and the Search view's two-line rows and lines

**Files:**
- Create: `src/window/option_toggles.rs`
- Modify:
  - `src/window/mod.rs`: `pub(crate) mod option_toggles;`
  - `src/window/palette.rs`: a new `error_foreground` field on `Palette`, set in `LIGHT`, `DARK`, `catppuccin` and `high_contrast`, and a test
  - `src/window/side_panel.rs`: `UiFonts.text_bold` (in `UiFonts`, its `Default`, `create` and `delete`); `destroy_windows` destroys the Search view's tooltip; `panel_proc` and `route` pass `WM_SYSKEYDOWN` and `WM_SYSCHAR` to the Search view
  - `src/window/tooltip.rs`: `Tooltip::set_max_width`
  - `src/window/search_view.rs`: the placeholder, the constants, `SearchView`'s new fields, `list_area`, `summary_rect`, `status_rect`, `paint`, `draw_row`, the snippet drawing, `layout`, `shown`, `hidden`, `header_hit`, `handle`, `search_edit_proc`, the tooltips, and tests. It drops the `expect` on `toggle_option`.
  - `src/window/main_window.rs`: window tests

**Interfaces:**
- Consumes: Task 1's `MatchOptions::get`, `SearchOption::ALL`; Task 3's `SkipReason::{ALL, index}`, `Progress.skipped`; Task 4's `SearchView.{options, search}`, `toggle_option`, `summary`, `status_line`, `SearchState`, the test helpers `search_for`, `wait_for_search`, `search_generation`, `search_rows`, `search_row`.
- Produces (`src/window/option_toggles.rs`), as the contract states:
  - `pub(crate) fn toggle_rects(field: RECT, dpi: u32) -> [RECT; 3]`, in `SearchOption::ALL` order
  - `pub(crate) fn reserved_width(dpi: u32) -> i32`
  - `pub(crate) fn paint(hdc: HDC, rects: &[RECT; 3], options: MatchOptions, hover: Option<SearchOption>, palette: &Palette, font: HFONT)`
  - `pub(crate) fn hit(rects: &[RECT; 3], point: POINT) -> Option<SearchOption>`
  - `pub(crate) fn alt_key(vk: u32) -> Option<SearchOption>`
  - `pub(crate) fn label(option: SearchOption) -> &'static str`, `pub(crate) fn tooltip(option: SearchOption) -> &'static str`
- Produces elsewhere:
  - `Palette.error_foreground: u32`
  - `UiFonts.text_bold: HFONT` (Segoe UI bold, 12 px at 96 DPI)
  - `Tooltip::set_max_width(&self, width: i32)`
  - `SearchView::{summary_rect, status_rect}(client: RECT, dpi: u32) -> RECT`, `pub(crate)` for Task 8
  - `SearchView::destroy_tooltip(&self)`
  - `search_view::skipped_tooltip(progress: &Progress) -> String`

Colors: the palette has no accent color. FastPad's focused outlines use `selection_background`, so an on toggle is filled with `selection_background`, with its glyph in `selection_foreground`, or `editor_foreground` when that is `None`. A hovered off toggle gets `hover_background` and `hover_foreground`. An off toggle's glyph is `muted_foreground`. The pattern error uses a new `error_foreground`: VS Code's error red, `#A1260D` light and `#F48771` dark, the flavor's `red` for Catppuccin, and the window text color in high contrast.

Row height: the 12 px Segoe UI line is 16 px tall at 96 DPI. The old one-line row was 26 px. A result row is now two 18 px line slots with 3 px above and below: `ROW_AT_96_DPI = 42`, the same for every row, so `row_list` is unchanged. `row_list::thumb_width` scales from the row height, so the Search view's scroll thumb becomes 9 px wide (the other views keep 6 px). The summary, notice and status lines are 22 px (`LINE_AT_96_DPI`).

The box and its toggles now also show with no notebook open, once the user opens the Search view, so the options can be set (spec §4). `shown` makes the box, and `show_view` runs only from user input. `layout`, which runs on the first `WM_SIZE`, still makes it only for a notebook, so a Search view restored at startup makes nothing before the first paint (§14).

- [ ] **Step 1: Write the failing pure tests**

At the end of the new `src/window/option_toggles.rs` (Step 3 writes the rest):

```rust
#[cfg(test)]
mod tests {
    use super::{alt_key, hit, label, paint, reserved_width, toggle_rects, tooltip};
    use crate::search::{MatchOptions, SearchOption};
    use crate::window::palette::Palette;
    use windows_sys::Win32::Foundation::{POINT, RECT};

    const FIELD: RECT = RECT {
        left: 8,
        top: 5,
        right: 252,
        bottom: 33,
    };

    fn edges(rect: RECT) -> (i32, i32, i32, i32) {
        (rect.left, rect.top, rect.right, rect.bottom)
    }

    #[test]
    fn the_toggles_sit_inside_the_right_end_of_the_field_and_scale_with_dpi() {
        // Break caught: toggles drawn over the typed text or off the field's right edge, a box
        // that runs under them, or sizes that ignore a 150% monitor.
        let rects = toggle_rects(FIELD, 96);
        assert_eq!(edges(rects[0]), (179, 8, 201, 30));
        assert_eq!(edges(rects[1]), (203, 8, 225, 30));
        assert_eq!(edges(rects[2]), (227, 8, 249, 30));
        assert!(FIELD.right - reserved_width(96) < rects[0].left, "the box stops short");
        let field = RECT {
            left: 12,
            top: 8,
            right: 378,
            bottom: 51,
        };
        let rects = toggle_rects(field, 144);
        assert_eq!(rects[2].right, 378 - 5);
        assert_eq!(rects[2].right - rects[2].left, 33);
        assert_eq!(rects[1].right, rects[2].left - 3);
        assert_eq!(rects[2].top - field.top, field.bottom - rects[2].bottom, "centered");
        assert!(field.right - reserved_width(144) < rects[0].left);
    }

    #[test]
    fn a_point_hits_the_toggle_under_it_and_nothing_in_the_gaps() {
        let rects = toggle_rects(FIELD, 96);
        assert_eq!(hit(&rects, POINT { x: 179, y: 8 }), Some(SearchOption::Case));
        assert_eq!(hit(&rects, POINT { x: 214, y: 19 }), Some(SearchOption::WholeWord));
        assert_eq!(hit(&rects, POINT { x: 248, y: 29 }), Some(SearchOption::Regex));
        assert_eq!(hit(&rects, POINT { x: 202, y: 19 }), None, "the gap");
        assert_eq!(hit(&rects, POINT { x: 249, y: 19 }), None, "the right padding");
        assert_eq!(hit(&rects, POINT { x: 214, y: 30 }), None, "below");
        assert_eq!(hit(&rects, POINT { x: 100, y: 19 }), None, "over the text");
    }

    #[test]
    fn alt_c_w_and_r_are_the_three_options_and_nothing_else() {
        // Break caught: Alt+F or Alt+E (the menu band's File and Edit) taken by the Search box,
        // or a lowercase character code read as a key.
        assert_eq!(alt_key(u32::from(b'C')), Some(SearchOption::Case));
        assert_eq!(alt_key(u32::from(b'W')), Some(SearchOption::WholeWord));
        assert_eq!(alt_key(u32::from(b'R')), Some(SearchOption::Regex));
        for other in [b'F', b'E', b'S', b'V', b'Z', b'c', b'w'] {
            assert_eq!(alt_key(u32::from(other)), None, "{}", char::from(other));
        }
    }

    #[test]
    fn labels_and_tooltips_name_each_option_and_its_shortcut() {
        assert_eq!(label(SearchOption::Case), "Match case");
        assert_eq!(label(SearchOption::WholeWord), "Match whole word");
        assert_eq!(label(SearchOption::Regex), "Use regular expression");
        assert_eq!(tooltip(SearchOption::Case), "Match case (Alt+C)");
        assert_eq!(tooltip(SearchOption::WholeWord), "Match whole word (Alt+W)");
        assert_eq!(tooltip(SearchOption::Regex), "Use regular expression (Alt+R)");
    }

    #[test]
    fn an_on_toggle_is_filled_a_hovered_one_shaded_and_an_off_one_left_clear() {
        // Break caught: an option that is on looking the same as one that is off.
        use windows_sys::Win32::Graphics::Gdi::{
            CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetPixel,
            ReleaseDC, SelectObject,
        };
        let palette = Palette {
            selection_background: 0x0000_00ff,
            hover_background: 0x0000_ff00,
            ..Palette::neutral()
        };
        let background = 0x0080_8080;
        let rects = toggle_rects(FIELD, 96);
        unsafe {
            let screen = GetDC(std::ptr::null_mut());
            let dc = CreateCompatibleDC(screen);
            let bitmap = CreateCompatibleBitmap(screen, 260, 40);
            let previous = SelectObject(dc, bitmap);
            let all = RECT {
                left: 0,
                top: 0,
                right: 260,
                bottom: 40,
            };
            crate::window::panel::fill(dc, all, background);
            let options = MatchOptions {
                case: true,
                ..MatchOptions::default()
            };
            paint(
                dc,
                &rects,
                options,
                Some(SearchOption::WholeWord),
                &palette,
                std::ptr::null_mut(),
            );
            let corner = |rect: RECT| GetPixel(dc, rect.left + 1, rect.top + 1);
            assert_eq!(corner(rects[0]), palette.selection_background, "on");
            assert_eq!(corner(rects[1]), palette.hover_background, "hovered");
            assert_eq!(corner(rects[2]), background, "off");
            SelectObject(dc, previous);
            DeleteObject(bitmap);
            DeleteDC(dc);
            ReleaseDC(std::ptr::null_mut(), screen);
        }
    }
}
```

In `src/window/palette.rs`, `mod tests`, add:

```rust
    #[test]
    fn the_error_color_stands_apart_from_the_text_and_the_background() {
        // Break caught: a regex error in the Search view that reads like the note count, or
        // vanishes into the background.
        for theme in Theme::ALL {
            let palette = Palette::for_theme(theme, false);
            assert_ne!(palette.error_foreground, palette.editor_background, "{theme:?}");
            assert_ne!(palette.error_foreground, palette.editor_foreground, "{theme:?}");
            assert_ne!(palette.error_foreground, palette.muted_foreground, "{theme:?}");
        }
        assert_eq!(
            Palette::for_theme(Theme::CatppuccinMocha, true).error_foreground,
            unsafe { GetSysColor(COLOR_WINDOWTEXT) },
            "high contrast keeps the system text color"
        );
    }
```

(`mod tests` already imports `Theme`, `GetSysColor` and `COLOR_WINDOWTEXT`.)

In `src/window/search_view.rs`, `mod tests`, replace the `use super::{...}` list with:

```rust
    use super::{
        LOADING, NO_MATCH, NO_NOTEBOOK, ROW_AT_96_DPI, ROW_INSET_AT_96_DPI, ROW_LINE_AT_96_DPI,
        SearchState, SearchView, TOO_SHORT, fit_before, notice_text, placeholder, skipped_tooltip,
        status_text, summary_text,
    };
    use crate::window::panel::scale;
```

replace `the_placeholder_names_the_open_notebook` with:

```rust
    #[test]
    fn the_placeholder_says_it_searches_text_in_the_open_notebook() {
        // Break caught: the box saying "Search" with no hint that it searches the notes' text,
        // or of which notebook.
        assert_eq!(
            placeholder(Some(Path::new(r"C:\Users\me\Work"))),
            "Search text in Work"
        );
        assert_eq!(placeholder(None), "Search text");
    }
```

and add:

```rust
    #[test]
    fn a_long_prefix_is_cut_from_the_start_so_the_match_stays_in_view() {
        // Break caught: in a narrow panel, forty characters before the match pushing it off the
        // row, or a cut inside a multi-byte character.
        let measure = |text: &str| text.chars().count() as i32 * 10;
        assert_eq!(fit_before("abcdef", 100, measure), "abcdef");
        assert_eq!(fit_before("abcdef", 40, measure), "\u{2026}def");
        assert_eq!(fit_before("\u{2026}été ab", 50, measure), "\u{2026}é ab");
        assert_eq!(fit_before("abc", 5, measure), "", "not even the ellipsis fits");
        assert_eq!(fit_before("", 0, measure), "");
    }

    #[test]
    fn the_skipped_tooltip_has_one_line_per_reason_that_skipped_a_note() {
        let progress = Progress {
            visited: 9,
            total: 9,
            skipped: [2, 0, 1, 1_500],
        };
        assert_eq!(
            skipped_tooltip(&progress),
            "2 online only\r\n1 couldn't be read\r\n1,500 not text"
        );
        let large = Progress {
            skipped: [0, 3, 0, 0],
            ..progress
        };
        assert_eq!(skipped_tooltip(&large), "3 larger than 4 MB");
        assert_eq!(skipped_tooltip(&Progress::default()), "");
    }

    #[test]
    fn a_result_row_holds_two_lines_of_the_sidebar_text_at_every_dpi() {
        // Break caught: the snippet line clipped at 150% or 200%, or the bold match taller than
        // its slot.
        use crate::window::titlebar::create_ui_font;
        use windows_sys::Win32::Graphics::Gdi::{
            DeleteObject, FW_BOLD, FW_NORMAL, GetDC, GetTextMetricsW, ReleaseDC, SelectObject,
            TEXTMETRICW,
        };
        for dpi in [96, 120, 144, 192] {
            let mut tallest = 0;
            for weight in [FW_NORMAL, FW_BOLD] {
                let font = create_ui_font(scale(12, dpi), "Segoe UI", weight as i32, false);
                unsafe {
                    let dc = GetDC(std::ptr::null_mut());
                    let previous = SelectObject(dc, font);
                    let mut metrics = TEXTMETRICW::default();
                    assert_ne!(GetTextMetricsW(dc, &mut metrics), 0);
                    tallest = tallest.max(metrics.tmHeight);
                    SelectObject(dc, previous);
                    ReleaseDC(std::ptr::null_mut(), dc);
                    DeleteObject(font);
                }
            }
            assert!(scale(ROW_LINE_AT_96_DPI, dpi) >= tallest, "{dpi}: {tallest}");
            assert!(
                scale(ROW_AT_96_DPI, dpi)
                    >= 2 * scale(ROW_LINE_AT_96_DPI, dpi) + 2 * scale(ROW_INSET_AT_96_DPI, dpi) - 1,
                "{dpi}"
            );
        }
    }
```

(`scale` rounds each term separately, so the sum can be off by one pixel: hence the `- 1`.)

- [ ] **Step 2: Write the failing window tests**

In `src/window/main_window.rs`, `mod tests`, after `closing_the_window_cancels_a_running_search`, add:

```rust
    /// The search field's toggle rectangles in the Search view's panel.
    fn search_toggles(hwnd: HWND) -> (HWND, [RECT; 3]) {
        let panel = sidebar_panel(hwnd);
        let mut client = RECT::default();
        unsafe { GetClientRect(panel, &mut client) };
        let dpi = unsafe { GetDpiForWindow(panel) }.max(96);
        let field = crate::window::search_view::SearchView::field_rect(client, dpi);
        (panel, crate::window::option_toggles::toggle_rects(field, dpi))
    }

    const ALT_DOWN: LPARAM = 1 << 29;

    #[test]
    fn the_toggles_change_by_click_and_by_alt_keys_in_the_box_and_the_results() {
        // Break caught: toggles that paint but ignore clicks, Alt+C/W/R going to the menu band
        // instead of flipping the option, a toggle that flips it without searching again, or a
        // regex error with no line saying so.
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            WM_LBUTTONDOWN, WM_LBUTTONUP, WM_SYSCHAR, WM_SYSKEYDOWN,
        };
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("search-toggles");
        scratch.note("A.md", "Needle");
        scratch.note("b.md", "needle");
        scratch.note("c.md", "needles");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        scratch.install(window.hwnd);
        crate::window::side_panel::show_view(window.hwnd, crate::config::SidebarView::Search, true);
        search_for(window.hwnd, "needle");
        assert_eq!(search_rows(window.hwnd).len(), 3);
        let dpi = unsafe { GetDpiForWindow(sidebar_panel(window.hwnd)) }.max(96);
        assert_eq!(
            app_mut(window.hwnd).sidebar.as_ref().unwrap().search.list.row_height,
            crate::window::panel::scale(42, dpi),
            "two-line rows"
        );
        let options = || crate::window::search_view::options(window.hwnd);

        // A click on Match case.
        let (panel, rects) = search_toggles(window.hwnd);
        let center = |rect: RECT| {
            ((((rect.top + rect.bottom) / 2) as u32) << 16 | ((rect.left + rect.right) / 2) as u32)
                as LPARAM
        };
        let before = search_generation(window.hwnd);
        unsafe {
            SendMessageW(panel, WM_LBUTTONDOWN, 0, center(rects[0]));
            SendMessageW(panel, WM_LBUTTONUP, 0, center(rects[0]));
        }
        assert!(options().case);
        wait_for_search(window.hwnd, before);
        assert_eq!(
            search_rows(window.hwnd),
            vec![search_row("b", "needle"), search_row("c", "needles")]
        );

        // Alt+C in the box turns it off again.
        let edit = crate::window::search_view::edit_hwnd(window.hwnd).unwrap();
        let before = search_generation(window.hwnd);
        unsafe { SendMessageW(edit, WM_SYSKEYDOWN, usize::from(b'C'), ALT_DOWN) };
        assert!(!options().case);
        wait_for_search(window.hwnd, before);
        assert_eq!(search_rows(window.hwnd).len(), 3);
        // The character that follows is swallowed, not handed to the menu band.
        assert_eq!(
            unsafe { SendMessageW(edit, WM_SYSCHAR, usize::from(b'c'), ALT_DOWN) },
            0
        );
        assert!(app_mut(window.hwnd).menu_mode.is_none());

        // Alt+W in the results: whole word drops "needles".
        let before = search_generation(window.hwnd);
        unsafe { SendMessageW(panel, WM_SYSKEYDOWN, usize::from(b'W'), ALT_DOWN) };
        assert!(options().whole_word);
        wait_for_search(window.hwnd, before);
        assert_eq!(
            search_rows(window.hwnd),
            vec![search_row("A", "Needle"), search_row("b", "needle")]
        );
        // Without Alt held (F10 also sends WM_SYSKEYDOWN), nothing flips.
        unsafe { SendMessageW(panel, WM_SYSKEYDOWN, usize::from(b'W'), 0) };
        assert!(options().whole_word);

        // Alt+R; an invalid pattern shows its error in place of the summary and keeps the rows.
        let before = search_generation(window.hwnd);
        unsafe { SendMessageW(edit, WM_SYSKEYDOWN, usize::from(b'R'), ALT_DOWN) };
        assert!(options().regex);
        wait_for_search(window.hwnd, before);
        type_into_search(window.hwnd, "need(le");
        pump_until(window.hwnd, || {
            matches!(
                search_state(window.hwnd),
                crate::window::search_view::SearchState::PatternError(_)
            )
        });
        let (message, error) = crate::window::search_view::summary(window.hwnd).unwrap();
        assert!(error && !message.is_empty(), "{message}");
        assert_eq!(search_rows(window.hwnd).len(), 2, "the previous results stay");
    }

    #[test]
    fn esc_in_the_search_box_clears_it_and_then_returns_to_the_editor() {
        // Break caught: Esc leaving the query in place, or jumping to the editor with text still
        // in the box.
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{SetFocus, VK_ESCAPE};
        use windows_sys::Win32::UI::WindowsAndMessaging::{GetWindowTextLengthW, WM_KEYDOWN};
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("search-escape");
        scratch.note("a.md", "ab");
        let window = shown_window();
        let editor = install_test_editor(&window);
        scratch.install(window.hwnd);
        crate::window::side_panel::show_view(window.hwnd, crate::config::SidebarView::Search, true);
        search_for(window.hwnd, "ab");
        let edit = crate::window::search_view::edit_hwnd(window.hwnd).unwrap();
        unsafe { SetFocus(edit) };

        unsafe { SendMessageW(edit, WM_KEYDOWN, usize::from(VK_ESCAPE), 0) };
        assert_eq!(unsafe { GetWindowTextLengthW(edit) }, 0);
        assert!(search_rows(window.hwnd).is_empty());
        assert_eq!(
            search_state(window.hwnd),
            crate::window::search_view::SearchState::Idle
        );
        assert_eq!(focused(), edit, "the first Esc only clears");

        unsafe { SendMessageW(edit, WM_KEYDOWN, usize::from(VK_ESCAPE), 0) };
        assert_eq!(focused(), editor.hwnd(), "Esc in the empty box goes to the editor");
    }

    #[test]
    fn with_no_notebook_the_search_box_and_its_toggles_still_work() {
        // Break caught: the options impossible to set until a notebook opens.
        use windows_sys::Win32::UI::WindowsAndMessaging::WM_SYSKEYDOWN;
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        crate::window::side_panel::show_view(window.hwnd, crate::config::SidebarView::Search, true);
        assert_eq!(
            crate::window::search_view::status(window.hwnd),
            Some(crate::window::search_view::NO_NOTEBOOK)
        );
        let edit = crate::window::search_view::edit_hwnd(window.hwnd).expect("the box shows");
        unsafe { SendMessageW(edit, WM_SYSKEYDOWN, usize::from(b'C'), ALT_DOWN) };
        assert!(crate::window::search_view::options(window.hwnd).case);
    }
```

`RECT`, `GetClientRect`, `GetDpiForWindow`, `SendMessageW`, `LPARAM` and `focused` are already in scope in `mod tests` (`LPARAM` comes from `windows_sys::Win32::Foundation`; add it to that `use` if it isn't there).

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test --lib -- option_toggles:: the_error_color search_view:: the_toggles_change_by_click esc_in_the_search_box with_no_notebook_the_search_box --test-threads=1`
Expected: a compile error, because `option_toggles`, `error_foreground`, `fit_before`, `skipped_tooltip` and the row constants don't exist yet.

- [ ] **Step 4: Write `option_toggles.rs`**

In `src/window/mod.rs`, add after `pub(crate) mod notebook_view;`:

```rust
pub(crate) mod option_toggles;
```

Create `src/window/option_toggles.rs`, above its `mod tests`:

```rust
//! The match case, whole word and regular expression toggles (note-search spec §4 and §8): three
//! square buttons painted inside the right end of a search field. The Search view and the find
//! bar share their geometry, painting, hit-testing, Alt keys and wording.

use crate::search::{MatchOptions, SearchOption};
use crate::window::palette::Palette;
use crate::window::panel::{fill, scale};
use crate::window::side_panel::draw_text;
use windows_sys::Win32::Foundation::{POINT, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    DT_CENTER, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, GetTextMetricsW, HDC, HFONT, SelectObject,
    TEXTMETRICW,
};

const SIZE_AT_96_DPI: i32 = 22;
const GAP_AT_96_DPI: i32 = 2;
const RIGHT_PADDING_AT_96_DPI: i32 = 3;

/// The three toggles inside `field`'s right end, in `SearchOption::ALL` order: 22 px squares with
/// 2 px gaps and 3 px of padding on the right at 96 DPI, centered vertically.
pub(crate) fn toggle_rects(field: RECT, dpi: u32) -> [RECT; 3] {
    let size = scale(SIZE_AT_96_DPI, dpi);
    let gap = scale(GAP_AT_96_DPI, dpi);
    let top = field.top + (field.bottom - field.top - size) / 2;
    let mut right = field.right - scale(RIGHT_PADDING_AT_96_DPI, dpi);
    let mut rects = [RECT::default(); 3];
    for rect in rects.iter_mut().rev() {
        *rect = RECT {
            left: right - size,
            top,
            right,
            bottom: top + size,
        };
        right -= size + gap;
    }
    rects
}

/// How much narrower than the field the `Edit` inside it must be: the toggles, their padding, and
/// one more gap between the text and the first toggle.
pub(crate) fn reserved_width(dpi: u32) -> i32 {
    3 * scale(SIZE_AT_96_DPI, dpi)
        + 3 * scale(GAP_AT_96_DPI, dpi)
        + scale(RIGHT_PADDING_AT_96_DPI, dpi)
}

fn glyph(option: SearchOption) -> &'static str {
    match option {
        SearchOption::Case => "Aa",
        SearchOption::WholeWord => "ab",
        SearchOption::Regex => ".*",
    }
}

/// Paints the toggles into `rects` (`toggle_rects`) in `font`. An option that is on is filled
/// with the accent FastPad outlines focused fields with (`selection_background`). The hovered one
/// that is off is shaded. "ab" is underlined, as in VS Code.
pub(crate) fn paint(
    hdc: HDC,
    rects: &[RECT; 3],
    options: MatchOptions,
    hover: Option<SearchOption>,
    palette: &Palette,
    font: HFONT,
) {
    for (option, rect) in SearchOption::ALL.into_iter().zip(rects) {
        let color = if options.get(option) {
            unsafe { fill(hdc, *rect, palette.selection_background) };
            palette
                .selection_foreground
                .unwrap_or(palette.editor_foreground)
        } else if hover == Some(option) {
            unsafe { fill(hdc, *rect, palette.hover_background) };
            palette.hover_foreground
        } else {
            palette.muted_foreground
        };
        let width = unsafe {
            draw_text(
                hdc,
                glyph(option),
                *rect,
                font,
                color,
                DT_SINGLELINE | DT_VCENTER | DT_CENTER | DT_NOPREFIX,
            )
        };
        if option == SearchOption::WholeWord {
            unsafe { underline(hdc, *rect, width, font, color) };
        }
    }
}

/// A 1 px line under text `width` wide, centered in `rect` as `DT_CENTER | DT_VCENTER` drew it.
unsafe fn underline(hdc: HDC, rect: RECT, width: i32, font: HFONT, color: u32) {
    let mut metrics = TEXTMETRICW::default();
    let measured = unsafe {
        let previous = (!font.is_null()).then(|| SelectObject(hdc, font));
        let measured = GetTextMetricsW(hdc, &mut metrics) != 0;
        if let Some(previous) = previous {
            SelectObject(hdc, previous);
        }
        measured
    };
    if !measured || width <= 0 {
        return;
    }
    let top = rect.top + (rect.bottom - rect.top - metrics.tmHeight) / 2;
    let y = top + metrics.tmAscent + 1;
    let left = rect.left + (rect.right - rect.left - width) / 2;
    unsafe {
        fill(
            hdc,
            RECT {
                left,
                top: y,
                right: left + width,
                bottom: y + 1,
            },
            color,
        );
    }
}

/// The toggle under `point`, if any.
pub(crate) fn hit(rects: &[RECT; 3], point: POINT) -> Option<SearchOption> {
    SearchOption::ALL
        .into_iter()
        .zip(rects)
        .find(|(_, rect)| {
            point.x >= rect.left
                && point.x < rect.right
                && point.y >= rect.top
                && point.y < rect.bottom
        })
        .map(|(option, _)| option)
}

/// The toggle an Alt+`vk` flips: Alt+C, Alt+W and Alt+R.
pub(crate) fn alt_key(vk: u32) -> Option<SearchOption> {
    match char::from_u32(vk)? {
        'C' => Some(SearchOption::Case),
        'W' => Some(SearchOption::WholeWord),
        'R' => Some(SearchOption::Regex),
        _ => None,
    }
}

/// The toggle's name, as screen readers read it.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "screen readers read it (Task 8)")
)]
pub(crate) fn label(option: SearchOption) -> &'static str {
    match option {
        SearchOption::Case => "Match case",
        SearchOption::WholeWord => "Match whole word",
        SearchOption::Regex => "Use regular expression",
    }
}

/// The toggle's tooltip: its name and its shortcut.
pub(crate) fn tooltip(option: SearchOption) -> &'static str {
    match option {
        SearchOption::Case => "Match case (Alt+C)",
        SearchOption::WholeWord => "Match whole word (Alt+W)",
        SearchOption::Regex => "Use regular expression (Alt+R)",
    }
}
```

`label` has no production caller until Task 8 (accessibility), so it carries `#[cfg_attr(not(test), expect(dead_code, ...))]`. Task 8 removes that attribute when it starts using `label`.

- [ ] **Step 5: Add the error color, the bold font and the tooltip width**

In `src/window/palette.rs`, in `struct Palette`, after `strip_foreground`, add:

```rust
    /// Error text on `editor_background` or the panel, such as a regex error in the Search view.
    pub error_foreground: u32,
```

In `LIGHT`, after `strip_foreground: rgb(32, 32, 32),`, add `error_foreground: rgb(0xA1, 0x26, 0x0D),`. In `DARK`, after `strip_foreground: rgb(212, 212, 212),`, add `error_foreground: rgb(0xF4, 0x87, 0x71),`. In `catppuccin`, after `strip_foreground: flavor.text,`, add `error_foreground: flavor.red,`. In `high_contrast`, after `strip_foreground: color(COLOR_BTNTEXT),`, add `error_foreground: text,`.

In `src/window/side_panel.rs`, add `FW_BOLD` to the `windows_sys::Win32::Graphics::Gdi` import and replace `UiFonts`, its `Default` and its `impl` (lines 58–110) with:

```rust
/// The sidebar's fonts at one DPI. Painting copies them out; `Sidebar` owns and deletes them.
#[derive(Clone, Copy, Debug)]
pub(crate) struct UiFonts {
    /// Row and body text: Segoe UI, 12 px at 96 DPI.
    pub(crate) text: HFONT,
    /// The match in a Search result's snippet: Segoe UI bold, 12 px.
    pub(crate) text_bold: HFONT,
    /// Header titles in small capitals: Segoe UI semibold, 11 px.
    pub(crate) bold: HFONT,
    /// Unsaved rows and notices inside the list: Segoe UI italic, 12 px.
    pub(crate) italic: HFONT,
    /// Row and header-button icons: Segoe MDL2 Assets, 12 px.
    pub(crate) glyph: HFONT,
    /// The activity bar's icons: Segoe MDL2 Assets, 16 px.
    pub(crate) bar_glyph: HFONT,
}

impl Default for UiFonts {
    fn default() -> Self {
        Self {
            text: std::ptr::null_mut(),
            text_bold: std::ptr::null_mut(),
            bold: std::ptr::null_mut(),
            italic: std::ptr::null_mut(),
            glyph: std::ptr::null_mut(),
            bar_glyph: std::ptr::null_mut(),
        }
    }
}

impl UiFonts {
    fn create(dpi: u32) -> Self {
        let normal = FW_NORMAL as i32;
        Self {
            text: create_ui_font(scale(12, dpi), "Segoe UI", normal, false),
            text_bold: create_ui_font(scale(12, dpi), "Segoe UI", FW_BOLD as i32, false),
            bold: create_ui_font(scale(11, dpi), "Segoe UI", FW_SEMIBOLD as i32, false),
            italic: create_ui_font(scale(12, dpi), "Segoe UI", normal, true),
            glyph: create_ui_font(scale(12, dpi), "Segoe MDL2 Assets", normal, false),
            bar_glyph: create_ui_font(scale(16, dpi), "Segoe MDL2 Assets", normal, false),
        }
    }

    fn delete(self) {
        for font in [
            self.text,
            self.text_bold,
            self.bold,
            self.italic,
            self.glyph,
            self.bar_glyph,
        ] {
            if !font.is_null() {
                unsafe { DeleteObject(font) };
            }
        }
    }
}
```

The fonts are made on the first paint that needs them (`Sidebar::fonts`), as before, so the extra font adds nothing before the first paint of a window without a sidebar.

In `destroy_windows`, replace `sidebar.notebook.destroy_tooltip();` with:

```rust
    sidebar.notebook.destroy_tooltip();
    sidebar.search.destroy_tooltip();
```

In `panel_proc`, add `WM_SYSCHAR, WM_SYSKEYDOWN` to the `windows_sys::Win32::UI::WindowsAndMessaging` import, and before the arm `WM_LBUTTONDOWN | WM_MOUSEMOVE | ... | WM_KEYDOWN | WM_CHAR => route(main, panel, message, wparam, lparam),` add:

```rust
        // The Search view's Alt+C, Alt+W and Alt+R, before the menu band sees the letter.
        WM_SYSKEYDOWN | WM_SYSCHAR if current_view(main) == SidebarView::Search => {
            route(main, panel, message, wparam, lparam)
        }
```

and in `route`, replace `WM_KEYDOWN | WM_CHAR => view_key(main, view, panel, message, wparam, lparam),` with:

```rust
        WM_KEYDOWN | WM_CHAR | WM_SYSKEYDOWN | WM_SYSCHAR => {
            view_key(main, view, panel, message, wparam, lparam)
        }
```

In `src/window/tooltip.rs`, add `TTM_SETMAXTIPWIDTH` to the `windows_sys::Win32::UI::Controls` import and, after `relay`, add:

```rust
    /// Lets tips break at `\r\n` and wrap at `width` pixels. Without a maximum width the control
    /// shows every tip on one line.
    pub(crate) fn set_max_width(&self, width: i32) {
        unsafe {
            SendMessageW(self.hwnd, TTM_SETMAXTIPWIDTH, 0, width as LPARAM);
        }
    }
```

- [ ] **Step 6: Draw the Search view's toggles, two-line rows and lines**

In `src/window/search_view.rs`:

Replace the imports from `use crate::config::SidebarView;` through the end of the `windows_sys` imports with:

```rust
use crate::config::SidebarView;
use crate::library::model::same_path;
use crate::library::text_search::{self, Progress, RunEnd, SkipReason, TextHit, hit_cmp};
use crate::platform::{last_error, wide_null};
use crate::search::{MatchOptions, SearchOption, Snippet, escape};
use crate::window::library_host;
use crate::window::main_window::OpenMode;
use crate::window::notebook_view::LOAD_FAILED;
use crate::window::option_toggles;
use crate::window::palette::Palette;
use crate::window::panel::{create_child, fill, inset, scale, text_height};
use crate::window::row_list::{self, ListKey, RowListState, RowLook, row_foreground};
use crate::window::side_panel::{self, UiFonts, ViewPaint, draw_text, point_of};
use crate::window::text_search_host::{self, SearchBatch};
use crate::window::tooltip::Tooltip;
use std::path::{Path, PathBuf};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, DT_CENTER, DT_END_ELLIPSIS, DT_EXPANDTABS, DT_LEFT, DT_NOPREFIX,
    DT_SINGLELINE, DT_VCENTER, DeleteObject, EndPaint, GetTextExtentPoint32W, HBRUSH, HDC, HFONT, InvalidateRect,
    PAINTSTRUCT, SelectObject, SetBkColor, SetTextColor,
};
use windows_sys::Win32::UI::Controls::{
    EM_GETMARGINS, EM_REPLACESEL, EM_SETSEL, EM_UNDO, WM_MOUSELEAVE,
};
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetFocus, GetKeyState, ReleaseCapture, SetCapture, SetFocus, TME_LEAVE, TRACKMOUSEEVENT,
    TrackMouseEvent, VK_CONTROL, VK_DOWN, VK_ESCAPE, VK_NEXT, VK_RETURN, VK_UP,
};
use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DestroyWindow, ES_AUTOHSCROLL, GetClientRect, GetParent, GetWindowTextLengthW, GetWindowTextW,
    MoveWindow, SW_HIDE, SW_SHOWNA, SendMessageW, SetWindowTextW, ShowWindow, WM_CAPTURECHANGED,
    WM_CHAR, WM_CLEAR, WM_CUT, WM_GETFONT, WM_KEYDOWN, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCDESTROY, WM_PAINT, WM_PASTE, WM_SETFONT,
    WM_SETTEXT, WM_SYSCHAR, WM_SYSKEYDOWN, WM_UNDO, WS_CHILD,
};
```

Replace the layout constants (`HEADER_AT_96_DPI` through `SEARCH_HOOK_ID`) with:

```rust
const HEADER_AT_96_DPI: i32 = 38;
/// The summary line, the notice and the status line.
const LINE_AT_96_DPI: i32 = 22;
/// A result: two line slots with a little room above and below. The 12 px Segoe UI line is 16 px
/// tall at 96 DPI; the old one-line row was 26 px.
const ROW_AT_96_DPI: i32 = 42;
const ROW_LINE_AT_96_DPI: i32 = 18;
const ROW_INSET_AT_96_DPI: i32 = 3;
const PADDING_AT_96_DPI: i32 = 12;
const FIELD_MARGIN_AT_96_DPI: i32 = 8;
const FIELD_HEIGHT_AT_96_DPI: i32 = 28;
const FIELD_TEXT_INSET_AT_96_DPI: i32 = 8;
const GLYPH_AT_96_DPI: i32 = 20;
const GAP_AT_96_DPI: i32 = 6;
const DOCUMENT_GLYPH: &str = "\u{E8A5}";
const SEARCH_HOOK_ID: usize = 0x4650_5356;
/// The status line's tooltip. The toggles are tools 0 to 2, in `SearchOption::ALL` order.
const STATUS_TOOL: usize = 3;
const TOOLTIP_WIDTH_AT_96_DPI: i32 = 300;
```

Replace `placeholder` with:

```rust
/// The box's placeholder: "Search text in <notebook>", with the notebook's display name.
pub(crate) fn placeholder(notebook: Option<&Path>) -> String {
    notebook
        .map(|notebook| format!("Search text in {}", library_host::notebook_name(notebook)))
        .unwrap_or_else(|| "Search text".to_owned())
}
```

After `thousands`, add:

```rust
/// The status line's tooltip: one line per reason that skipped a note.
pub(crate) fn skipped_tooltip(progress: &Progress) -> String {
    SkipReason::ALL
        .into_iter()
        .filter_map(|reason| {
            let count = progress.skipped[reason.index()];
            let why = match reason {
                SkipReason::OnlineOnly => "online only",
                SkipReason::TooLarge => "larger than 4 MB",
                SkipReason::Unreadable => "couldn't be read",
                SkipReason::NotText => "not text",
            };
            (count > 0).then(|| format!("{} {why}", thousands(count)))
        })
        .collect::<Vec<_>>()
        .join("\r\n")
}

/// The part of a snippet before its match, cut from the start (with `…`) until it is at most
/// `room` pixels wide, so the match stays in view in a narrow panel. `measure` gives a text's
/// width. Empty when not even `…` and one character fit.
fn fit_before(before: &str, room: i32, measure: impl Fn(&str) -> i32) -> String {
    if measure(before) <= room {
        return before.to_owned();
    }
    let starts = before
        .char_indices()
        .map(|(index, _)| index)
        .skip(1)
        .collect::<Vec<_>>();
    let cut = |start: usize| format!("\u{2026}{}", &before[start..]);
    // A later start is never wider: find the first that fits.
    let first = starts.partition_point(|&start| measure(&cut(start)) > room);
    starts.get(first).map_or_else(String::new, |&start| cut(start))
}

/// `text`'s width in `font`.
fn text_width(hdc: HDC, text: &str, font: HFONT) -> i32 {
    if text.is_empty() {
        return 0;
    }
    let wide: Vec<u16> = text.encode_utf16().collect();
    let mut size = SIZE::default();
    unsafe {
        let previous = (!font.is_null()).then(|| SelectObject(hdc, font));
        let measured = GetTextExtentPoint32W(hdc, wide.as_ptr(), wide.len() as i32, &mut size);
        if let Some(previous) = previous {
            SelectObject(hdc, previous);
        }
        if measured != 0 { size.cx } else { 0 }
    }
}

/// A result's second line: the snippet, its match in bold. The text before the match is cut
/// from its start when the row is too narrow, so the match stays in view. The snippet is the raw
/// line and can hold tabs, so they are expanded (`DT_EXPANDTABS`) rather than drawn as boxes.
unsafe fn draw_snippet(hdc: HDC, snippet: &Snippet, rect: RECT, fonts: UiFonts, color: u32) {
    let line = DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX | DT_LEFT | DT_EXPANDTABS;
    let text = snippet.text.as_str();
    let range = snippet.highlight.clone();
    let (Some(before), Some(matched), Some(after)) = (
        text.get(..range.start),
        text.get(range.clone()),
        text.get(range.end..),
    ) else {
        // Never made by `snippet::cut`, but a bad range draws the text plain, never panics.
        unsafe { draw_text(hdc, text, rect, fonts.text, color, line | DT_END_ELLIPSIS) };
        return;
    };
    let available = (rect.right - rect.left).max(0);
    let room = (available - text_width(hdc, matched, fonts.text_bold)).max(available / 3);
    let before = fit_before(before, room, |part| text_width(hdc, part, fonts.text));
    let mut left = rect.left;
    unsafe {
        left += draw_text(hdc, &before, RECT { left, ..rect }, fonts.text, color, line);
        if left < rect.right {
            left += draw_text(
                hdc,
                matched,
                RECT { left, ..rect },
                fonts.text_bold,
                color,
                line | DT_END_ELLIPSIS,
            );
        }
        if left < rect.right {
            draw_text(
                hdc,
                after,
                RECT { left, ..rect },
                fonts.text,
                color,
                line | DT_END_ELLIPSIS,
            );
        }
    }
}

/// A rectangle as an array, so the tooltip tools can be compared (`RECT` has no `PartialEq`).
const fn edges(rect: RECT) -> [i32; 4] {
    [rect.left, rect.top, rect.right, rect.bottom]
}

const fn rect_of(edges: [i32; 4]) -> RECT {
    RECT {
        left: edges[0],
        top: edges[1],
        right: edges[2],
        bottom: edges[3],
    }
}
```

In `struct SearchView`, after the `order` field, add:

```rust
    /// The toggle under the pointer.
    toggle_hover: Option<SearchOption>,
    /// The toggles' and the status line's tooltip, made when the pointer first moves over the
    /// Search view.
    tooltip: Option<Tooltip>,
    /// The tooltip could not be made; it is not tried again.
    tooltip_failed: bool,
    /// The tools the tooltip has, so a pointer move changes them only when they differ.
    tools_shown: Vec<(usize, [i32; 4], String)>,
```

and in `SearchView::new`, after `order: 0,`, add:

```rust
            toggle_hover: None,
            tooltip: None,
            tooltip_failed: false,
            tools_shown: Vec::new(),
```

Replace `list_area` with:

```rust
    /// Where the results are: under the header and the summary line, above the status line when
    /// it shows.
    pub(crate) fn list_area(&self, client: RECT, dpi: u32) -> RECT {
        let top = (client.top + scale(HEADER_AT_96_DPI, dpi) + scale(LINE_AT_96_DPI, dpi))
            .min(client.bottom);
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

    /// The summary line under the box, where the notice shows too.
    pub(crate) fn summary_rect(client: RECT, dpi: u32) -> RECT {
        let pad = scale(PADDING_AT_96_DPI, dpi);
        let top = (client.top + scale(HEADER_AT_96_DPI, dpi)).min(client.bottom);
        RECT {
            left: client.left + pad,
            top,
            right: client.right - pad,
            bottom: (top + scale(LINE_AT_96_DPI, dpi)).min(client.bottom),
        }
    }

    /// The status line along the bottom.
    pub(crate) fn status_rect(client: RECT, dpi: u32) -> RECT {
        let pad = scale(PADDING_AT_96_DPI, dpi);
        RECT {
            left: client.left + pad,
            top: (client.bottom - scale(LINE_AT_96_DPI, dpi)).max(client.top),
            right: client.right - pad,
            bottom: client.bottom,
        }
    }

    /// The toggle under `point`, while the field shows.
    fn toggle_at(&self, point: POINT, client: RECT, dpi: u32) -> Option<SearchOption> {
        self.edit?;
        option_toggles::hit(
            &option_toggles::toggle_rects(Self::field_rect(client, dpi), dpi),
            point,
        )
    }

    /// The tooltip's tools: the toggles while the field shows, and the status line while it says
    /// notes were skipped. An empty text removes a tool.
    fn tooltip_tools(&self, client: RECT, dpi: u32) -> Vec<(usize, [i32; 4], String)> {
        let rects = option_toggles::toggle_rects(Self::field_rect(client, dpi), dpi);
        let mut tools = SearchOption::ALL
            .into_iter()
            .zip(rects)
            .enumerate()
            .map(|(id, (option, rect))| {
                let text = if self.edit.is_some() {
                    option_toggles::tooltip(option).to_owned()
                } else {
                    String::new()
                };
                (id, edges(rect), text)
            })
            .collect::<Vec<_>>();
        let skipped = match &self.search {
            SearchState::Done { progress, .. } if self.notice().is_none() => {
                skipped_tooltip(progress)
            }
            _ => String::new(),
        };
        tools.push((STATUS_TOOL, edges(Self::status_rect(client, dpi)), skipped));
        tools
    }

    /// Destroys the view's tooltip, if it made one. The popup is owned by the main window, so
    /// destroying the panel does not take it along (`side_panel::destroy_windows` calls this).
    pub(crate) fn destroy_tooltip(&self) {
        if let Some(tooltip) = self.tooltip {
            tooltip.destroy();
        }
    }
```

Replace `paint` and `draw_row` with:

```rust
    pub(crate) fn paint(&self, paint: &ViewPaint) {
        let dpi = paint.dpi;
        let palette = paint.palette;
        let client = paint.client;
        let line = DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX | DT_LEFT | DT_END_ELLIPSIS;
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
            }
            let summary = Self::summary_rect(client, dpi);
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

    /// One result on two lines: the file icon, the name and its folder in dim text, then the
    /// snippet with its match in bold.
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
        let text = RECT {
            left: glyph.right + scale(GAP_AT_96_DPI, dpi),
            right: rect.right - pad,
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
        }
    }
```

Replace `layout` with:

```rust
/// Places the box in the header, short of the toggles, and shows it while the Search view shows,
/// making it the first time the view shows a notebook. Part of `side_panel::layout`, and run
/// whenever the view or the notebook changes. It never makes the box without a notebook: at
/// startup that would come before the first paint. `shown` makes it once the user opens the view.
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
    unsafe {
        if !text_font.is_null() {
            SendMessageW(edit, WM_SETFONT, text_font as WPARAM, 0);
        }
    }
    let field = SearchView::field_rect(client, dpi);
    let text = text_height(edit, text_font).clamp(1, (field.bottom - field.top - 2).max(1));
    let inset_x = scale(FIELD_TEXT_INSET_AT_96_DPI, dpi);
    let top = field.top + (field.bottom - field.top - text) / 2;
    let width = (field.right - field.left - inset_x - option_toggles::reserved_width(dpi)).max(0);
    unsafe {
        MoveWindow(edit, field.left + inset_x, top, width, text, 1);
    }
    with_view(hwnd, |view| {
        view.list.row_height = scale(ROW_AT_96_DPI, dpi)
    });
    if show {
        unsafe {
            ShowWindow(edit, SW_SHOWNA);
        }
    } else {
        hide_box(edit);
    }
}
```

Replace `shown` and `hidden` with:

```rust
/// `side_panel::show_view` switched to Search. The user opened it, so the box is made now even
/// with no notebook: its toggles set the options (spec §4). `focus` puts the caret in the box. A
/// library change while the view was hidden runs the query again now, if the notes changed.
pub(crate) fn shown(hwnd: HWND, focus: bool) {
    ensure_edit(hwnd);
    layout(hwnd);
    if with_view(hwnd, |view| std::mem::take(&mut view.stale)).unwrap_or(false) {
        text_search_host::library_changed(hwnd);
    }
    let Some((panel, edit)) = with_view(hwnd, |view| (view.panel, view.edit)) else {
        return;
    };
    if !focus {
        return;
    }
    unsafe {
        match edit {
            Some(edit) => {
                SetFocus(edit);
                SendMessageW(edit, EM_SETSEL, 0, -1);
            }
            None => {
                SetFocus(panel);
            }
        }
    }
}

/// `side_panel::show_view` switched away from Search. The query stays. The toggles' tooltips go,
/// or they would show over the other view.
pub(crate) fn hidden(hwnd: HWND) {
    let Some((edit, tooltip, tools)) = with_view(hwnd, |view| {
        (view.edit, view.tooltip, std::mem::take(&mut view.tools_shown))
    }) else {
        return;
    };
    if let Some(tooltip) = tooltip {
        for (id, _, _) in tools {
            tooltip.set_tool(id, RECT::default(), "");
        }
    }
    if let Some(edit) = edit {
        hide_box(edit);
    }
}
```

Replace `header_hit` with:

```rust
/// Whether panel point (`x`, `y`) is on the painted search field, which is client area, not
/// window caption. The field shows once the box exists.
pub(crate) fn header_hit(hwnd: HWND, panel: HWND, x: i32, y: i32) -> bool {
    let (client, dpi) = geometry(panel);
    with_view(hwnd, |view| view.edit.is_some()).unwrap_or(false)
        && inside(SearchView::field_rect(client, dpi), POINT { x, y })
}
```

Remove the `#[cfg_attr(not(test), expect(dead_code, ...))]` attribute from `toggle_option`: the toggles call it now.

After `field_pressed`, add:

```rust
/// The toggle an Alt+letter `WM_SYSKEYDOWN` flips. Bit 29 of `lparam` says Alt is down: F10 also
/// arrives as `WM_SYSKEYDOWN`, without it.
fn alt_option(wparam: WPARAM, lparam: LPARAM) -> Option<SearchOption> {
    if lparam & (1 << 29) == 0 {
        return None;
    }
    option_toggles::alt_key(u32::try_from(wparam).ok()?)
}

/// Whether a `WM_SYSCHAR` is a toggle's Alt letter, which must not reach the menu band (it would
/// beep, or open a menu with that mnemonic).
fn is_toggle_char(wparam: WPARAM, lparam: LPARAM) -> bool {
    lparam & (1 << 29) != 0
        && u32::try_from(wparam)
            .ok()
            .and_then(char::from_u32)
            .filter(char::is_ascii_alphabetic)
            .is_some_and(|letter| {
                option_toggles::alt_key(u32::from(letter.to_ascii_uppercase())).is_some()
            })
}

/// Gives the tooltip the tools the view has now, making the tooltip on the first pointer move and
/// handing it that move. Runs with nothing of the App borrowed: creating the control and adding
/// tools send messages.
fn update_tooltips(hwnd: HWND, panel: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) {
    let (client, dpi) = geometry(panel);
    let Some((tools, existing, failed, changed)) = with_view(hwnd, |view| {
        let tools = view.tooltip_tools(client, dpi);
        let changed = tools != view.tools_shown;
        (tools, view.tooltip, view.tooltip_failed, changed)
    }) else {
        return;
    };
    let (tooltip, created) = match existing {
        Some(tooltip) => (tooltip, false),
        None if failed => return,
        None => {
            let created = Tooltip::create(panel);
            let kept = with_view(hwnd, |view| {
                view.tooltip = created;
                view.tooltip_failed = created.is_none();
            });
            match (created, kept) {
                (Some(tooltip), Some(())) => {
                    tooltip.set_max_width(scale(TOOLTIP_WIDTH_AT_96_DPI, dpi));
                    (tooltip, true)
                }
                (Some(tooltip), None) => {
                    tooltip.destroy();
                    return;
                }
                (None, _) => return,
            }
        }
    };
    if changed || created {
        for (id, rect, text) in &tools {
            tooltip.set_tool(*id, rect_of(*rect), text);
        }
        with_view(hwnd, |view| view.tools_shown = tools);
    }
    if created {
        tooltip.relay(message, wparam, lparam);
    }
}
```

Replace `handle` with:

```rust
/// Input for the Search view's field and result list. `None` leaves the message to the panel.
pub(crate) fn handle(
    hwnd: HWND,
    panel: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> Option<LRESULT> {
    let (client, dpi) = geometry(panel);
    match message {
        WM_MOUSEMOVE => {
            let mut track = TRACKMOUSEEVENT {
                cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: panel,
                dwHoverTime: 0,
            };
            unsafe {
                TrackMouseEvent(&mut track);
            }
            let at = point(lparam);
            let changed = with_view(hwnd, |view| {
                if let Some(grab) = view.thumb_grab {
                    let area = view.list_area(client, dpi);
                    return view.list.drag_thumb(grab, at.y - area.top, height(area));
                }
                let toggle = view.toggle_at(at, client, dpi);
                let toggle_changed = std::mem::replace(&mut view.toggle_hover, toggle) != toggle;
                let hover = view.row_under(at, client, dpi);
                let row_changed = view.list.set_hover(hover);
                toggle_changed || row_changed
            })
            .unwrap_or(false);
            if changed {
                invalidate(panel);
            }
            update_tooltips(hwnd, panel, message, wparam, lparam);
            Some(0)
        }
        WM_MOUSELEAVE => {
            let changed = with_view(hwnd, |view| {
                let toggle_changed = view.toggle_hover.take().is_some();
                let row_changed = view.list.set_hover(None);
                toggle_changed || row_changed
            })
            .unwrap_or(false);
            if changed {
                invalidate(panel);
            }
            Some(0)
        }
        WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
            let at = point(lparam);
            if let Some(option) = with_view(hwnd, |view| view.toggle_at(at, client, dpi)).flatten()
            {
                toggle_option(hwnd, option);
                return Some(0);
            }
            if field_pressed(hwnd, panel, at) {
                return Some(0);
            }
            // A press on the scroll thumb drags it, as in the Notebook view.
            let grabbed = message == WM_LBUTTONDOWN
                && with_view(hwnd, |view| {
                    view.thumb_grab = view.thumb_at(at, client, dpi);
                    view.thumb_grab.is_some()
                })
                .unwrap_or(false);
            if grabbed {
                unsafe {
                    SetCapture(panel);
                }
                return Some(0);
            }
            let path = with_view(hwnd, |view| {
                let index = view.row_under(at, client, dpi)?;
                let area = view.list_area(client, dpi);
                view.list.select(index, height(area));
                Some(view.results[index].path.clone())
            })
            .flatten();
            invalidate(panel);
            if let Some(path) = path {
                let mode = if message == WM_LBUTTONDBLCLK {
                    OpenMode::Permanent
                } else {
                    OpenMode::Preview
                };
                // A mouse click moves the focus to the editor (spec §6.4).
                open_result(hwnd, &path, mode, true);
            }
            Some(0)
        }
        WM_LBUTTONUP => {
            if with_view(hwnd, |view| view.thumb_grab.take().is_some()).unwrap_or(false) {
                unsafe {
                    ReleaseCapture();
                }
            }
            Some(0)
        }
        WM_CAPTURECHANGED => {
            with_view(hwnd, |view| view.thumb_grab = None);
            Some(0)
        }
        WM_MOUSEWHEEL => {
            let delta = i32::from((wparam >> 16) as u16 as i16);
            let lines = row_list::wheel_lines();
            let scrolled = with_view(hwnd, |view| {
                let area = view.list_area(client, dpi);
                view.list.wheel(delta, lines, height(area))
            })
            .unwrap_or(false);
            if scrolled {
                invalidate(panel);
            }
            Some(0)
        }
        // Alt+C, Alt+W and Alt+R in the results flip the toggles (spec §4).
        WM_SYSKEYDOWN => {
            let option = alt_option(wparam, lparam)?;
            toggle_option(hwnd, option);
            Some(0)
        }
        WM_SYSCHAR if is_toggle_char(wparam, lparam) => Some(0),
        WM_KEYDOWN => {
            let key = wparam as u16;
            let ctrl = unsafe { GetKeyState(i32::from(VK_CONTROL)) } < 0;
            if key == VK_RETURN {
                if ctrl {
                    open_selected(hwnd, OpenMode::Permanent, true);
                } else {
                    // Enter keeps the focus in the list, so arrows and Enter browse (spec §6.4).
                    open_selected(hwnd, OpenMode::Preview, false);
                }
                return Some(0);
            }
            let at_top = with_view(hwnd, |view| {
                view.list.selected.is_none_or(|index| index == 0)
            })
            .unwrap_or(true);
            if key == VK_UP && at_top {
                if let Some(edit) = with_view(hwnd, |view| view.edit).flatten() {
                    unsafe {
                        SetFocus(edit);
                    }
                }
                return Some(0);
            }
            let movement = ListKey::from_virtual_key(u32::from(key))?;
            with_view(hwnd, |view| {
                let area = view.list_area(client, dpi);
                view.list.move_selection(movement, height(area))
            });
            invalidate(panel);
            Some(0)
        }
        // Typing in the list goes on in the box.
        WM_CHAR if (wparam as u32) >= 0x20 && wparam as u32 != 0x7f => {
            let edit = with_view(hwnd, |view| view.edit).flatten()?;
            unsafe {
                SetFocus(edit);
                SendMessageW(edit, WM_CHAR, wparam, lparam);
            }
            Some(0)
        }
        _ => None,
    }
}
```

Replace `search_edit_proc` with:

```rust
unsafe extern "system" fn search_edit_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _subclass_id: usize,
    _ref_data: usize,
) -> LRESULT {
    if message == WM_NCDESTROY {
        // The last message: drop the hook and let the Edit finish; nothing else is looked up.
        unsafe {
            RemoveWindowSubclass(hwnd, Some(search_edit_proc), SEARCH_HOOK_ID);
            return DefSubclassProc(hwnd, message, wparam, lparam);
        }
    }
    let panel = unsafe { GetParent(hwnd) };
    let main = unsafe { GetParent(panel) };
    // A single-line Edit beeps at Enter and Escape characters; both are handled on key down.
    if message == WM_CHAR && matches!(wparam as u16, 0x0d | 0x1b) {
        return 0;
    }
    // Alt+C, Alt+W and Alt+R flip the toggles before the menu band sees the letter (spec §4).
    if message == WM_SYSKEYDOWN
        && let Some(option) = alt_option(wparam, lparam)
    {
        toggle_option(main, option);
        return 0;
    }
    if message == WM_SYSCHAR && is_toggle_char(wparam, lparam) {
        return 0;
    }
    if message == WM_PAINT
        && unsafe { GetWindowTextLengthW(hwnd) } == 0
        && paint_placeholder(main, hwnd)
    {
        return 0;
    }
    if message == WM_KEYDOWN {
        let ctrl = unsafe { GetKeyState(i32::from(VK_CONTROL)) } < 0;
        match wparam as u16 {
            VK_DOWN | VK_NEXT => {
                enter_results(main, panel);
                return 0;
            }
            VK_RETURN if ctrl => {
                open_selected(main, OpenMode::Permanent, true);
                return 0;
            }
            VK_RETURN => {
                open_selected(main, OpenMode::Preview, false);
                return 0;
            }
            VK_ESCAPE => {
                // Esc clears the box; in an empty box it returns to the editor (spec §4).
                if unsafe { GetWindowTextLengthW(hwnd) } > 0 {
                    let empty = wide_null("");
                    // WM_SETTEXT comes back through this proc, which repaints the placeholder,
                    // and EN_CHANGE clears the results.
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

- [ ] **Step 7: Run the tests to verify they pass**

Run: `cargo test --lib -- option_toggles:: the_error_color search_view:: the_toggles_change_by_click esc_in_the_search_box with_no_notebook_the_search the_search_field_is_client_area the_search_view_finds_note_text typing_waits_for_the_debounce the_panel_shade every_theme_has --test-threads=1`
Expected: all pass. `the_search_field_is_client_area…` still passes: the field is client area whenever the box exists.

- [ ] **Step 8: Lint, format, commit**

Run: `cargo fmt --all`, then `cargo fmt --all -- --check` and `cargo clippy --all-targets -- -D warnings`.
Expected: no output from the check, and no warnings.

```bash
git add src/window/option_toggles.rs src/window/mod.rs src/window/palette.rs src/window/side_panel.rs src/window/tooltip.rs src/window/search_view.rs src/window/main_window.rs
git commit -m "feat(search): match case, whole word and regex toggles in the Search field, two-line results with the match in bold, and summary and status lines"
```

---

### Task 6: Commands (Ctrl+Shift+F, Shift+Alt+F, the three toggle commands)

**Files:**
- Modify:
  - `src/window/commands.rs`
  - `src/window/menus.rs`
  - `src/window/command_palette.rs`
  - `src/window/main_window.rs`
  - `src/window/search_view.rs` (removes the two `expect(dead_code)` attributes Task 4 put on `show_with_query` and `options`)
  - `tests/windows/json_commands.rs` (one comment)
  - `README.md` (the two Ctrl+K and Ctrl+Shift+F mentions and two table cells)
  - `docs/superpowers/specs/2026-09-23-note-sidebar-design.md` (the two Ctrl+K mentions)

**Interfaces:**
- Consumes:
  - `crate::search::SearchOption` (Task 1).
  - `search_view::{toggle_option, show_with_query, current_query, options}` (Task 4). `show_with_query` does the regex escaping; this task never escapes.
  - `side_panel::{show_view, current_view}` and `notes_mode_enabled` (existing).
- Produces:
  - `CommandId::SearchToggleCase = 185`, `SearchToggleWholeWord = 186`, `SearchToggleRegex = 187`, all `is_sidebar()`, none `needs_document()`.
  - `CommandId::search_option(self) -> Option<SearchOption>` (a new `pub const fn`, used by `main_window` and the tests).
  - `main_window::single_line_selection(hwnd) -> Option<String>` (private), which Task 7 reuses in `open_find_bar`.
  - `execute_command` now returns early for any `is_sidebar()` command while notes mode is off.
- Table sizes change by these amounts. Other tasks may have changed the totals, so apply the delta to whatever is there:
  - `COMMANDS` in `commands.rs` grows by 3.
  - `command_palette::ENTRIES` grows by 3.
  - `accelerator_specs()` stays the same length: Ctrl+K goes, Shift+Alt+F comes, and Ctrl+Shift+F changes its command.

- [ ] **Step 1: Write the failing command tests** in `src/window/commands.rs`'s `tests` module

```rust
    #[test]
    fn search_option_commands_have_their_reserved_numbers_and_are_sidebar_commands() {
        // Break caught: a toggle renumbered into another command's range, greyed out while no
        // tab is open, or left enabled with notes mode off, where there is no Search view.
        use crate::search::SearchOption;
        for (value, command, option) in [
            (185, CommandId::SearchToggleCase, SearchOption::Case),
            (186, CommandId::SearchToggleWholeWord, SearchOption::WholeWord),
            (187, CommandId::SearchToggleRegex, SearchOption::Regex),
        ] {
            assert_eq!(CommandId::try_from(value), Ok(command));
            assert!(command.is_sidebar(), "{command:?}");
            assert!(!command.needs_document(), "{command:?}");
            assert_eq!(command.search_option(), Some(option));
        }
        assert_eq!(CommandId::ShowSearchView.search_option(), None);
        assert_eq!(CommandId::Find.search_option(), None);
    }
```

Run: `cargo test --lib -- window::commands --test-threads=1`
Expected: compile error, because `SearchToggleCase` and `search_option` don't exist yet.

- [ ] **Step 2: Add the commands** in `src/window/commands.rs`

1. After `FocusPreviousPane = 184,` in the enum, add:

```rust
    SearchToggleCase = 185,
    SearchToggleWholeWord = 186,
    SearchToggleRegex = 187,
```

2. In `needs_document`'s `matches!` list, after `| Self::FocusPreviousPane`, add:

```rust
                | Self::SearchToggleCase
                | Self::SearchToggleWholeWord
                | Self::SearchToggleRegex
```

3. Replace `is_sidebar` with:

```rust
    /// Commands that act on the sidebar, which exists only in notes mode.
    pub const fn is_sidebar(self) -> bool {
        matches!(
            self,
            Self::ToggleSidebar
                | Self::ShowNotebookView
                | Self::ShowSearchView
                | Self::ShowFavoritesView
                | Self::SearchToggleCase
                | Self::SearchToggleWholeWord
                | Self::SearchToggleRegex
        )
    }

    /// The Search view option a `SearchToggle*` command flips.
    pub const fn search_option(self) -> Option<crate::search::SearchOption> {
        match self {
            Self::SearchToggleCase => Some(crate::search::SearchOption::Case),
            Self::SearchToggleWholeWord => Some(crate::search::SearchOption::WholeWord),
            Self::SearchToggleRegex => Some(crate::search::SearchOption::Regex),
            _ => None,
        }
    }
```

4. In `TryFrom<u16>`, the `COMMANDS` array grows by 3. It is `[CommandId; 76]` on `feat/note-sidebar`, so it becomes `[CommandId; 79]`. After `CommandId::FocusPreviousPane,` add:

```rust
            CommandId::SearchToggleCase,
            CommandId::SearchToggleWholeWord,
            CommandId::SearchToggleRegex,
```

Run: `cargo test --lib -- window::commands --test-threads=1`
Expected: every `window::commands` test passes.

- [ ] **Step 3: Move the shortcuts** in `src/window/menus.rs`

1. In `accelerator_specs()`, replace

```rust
        accelerator(FCONTROL | FSHIFT, b'F', CommandId::FormatJson),
```

with

```rust
        accelerator(FCONTROL | FSHIFT, b'F', CommandId::ShowSearchView),
        accelerator(FSHIFT | FALT, b'F', CommandId::FormatJson),
```

2. Delete the line

```rust
        accelerator(FCONTROL, b'K', CommandId::ShowSearchView),
```

The array's length does not change: one line was added and one deleted.

3. In `tab_zoom_and_direction_shortcuts_are_bound`, change the first `use` to

```rust
        use windows_sys::Win32::UI::WindowsAndMessaging::{FALT, FCONTROL, FSHIFT};
```

and replace the last assertion (the one binding `FCONTROL, b'K'`) with:

```rust
        assert_eq!(
            bound(FCONTROL | FSHIFT, u16::from(b'F')),
            Some(CommandId::ShowSearchView)
        );
        assert_eq!(
            bound(FSHIFT | FALT, u16::from(b'F')),
            Some(CommandId::FormatJson)
        );
        // Break caught: Ctrl+K still showing Search after Search moved to Ctrl+Shift+F.
        assert_eq!(bound(FCONTROL, u16::from(b'K')), None);
        // The option toggles are palette-only (spec §5).
        assert!(
            accelerator_specs()
                .iter()
                .all(|spec| spec.command.search_option().is_none())
        );
```

In the same test, the `ToggleWordWrap` assertion spells `windows_sys::Win32::UI::WindowsAndMessaging::FALT` in full. Change it to `FALT`, now that it is imported.

4. Menu labels: no menu-bar entry names Format JSON or Show search with a shortcut. The overflow menu's "Format JSON" shows no shortcut, like every other overflow entry. The palette spells shortcuts from `accelerator_specs()`, so both palette rows change with the table. Nothing else in `menus.rs` changes.

- [ ] **Step 4: Add the palette rows** in `src/window/command_palette.rs`

1. `ENTRIES` grows by 3. It is `[PaletteEntry; 63]` on `feat/note-sidebar`, so it becomes `[PaletteEntry; 66]`. After `entry("Search: Replace", CommandId::Replace),` add:

```rust
    entry("Search: Toggle match case", CommandId::SearchToggleCase),
    entry("Search: Toggle whole word", CommandId::SearchToggleWholeWord),
    entry(
        "Search: Toggle regular expression",
        CommandId::SearchToggleRegex,
    ),
```

2. In the tests, replace `the_sidebar_commands_are_listed_with_their_shortcuts` with:

```rust
    #[test]
    fn the_sidebar_commands_are_listed_with_their_shortcuts() {
        assert_eq!(labels("sidebar")[0], "View: Toggle sidebar");
        assert_eq!(labels("show search")[0], "View: Show search");
        assert_eq!(
            shortcut_text(CommandId::ToggleSidebar).as_deref(),
            Some("Ctrl+B")
        );
        assert_eq!(
            shortcut_text(CommandId::ShowNotebookView).as_deref(),
            Some("Ctrl+Shift+E")
        );
        // Break caught: the palette row still reading Ctrl+K after the shortcut moved.
        assert_eq!(
            shortcut_text(CommandId::ShowSearchView).as_deref(),
            Some("Ctrl+Shift+F")
        );
        assert_eq!(shortcut_text(CommandId::ShowFavoritesView), None);
    }

    #[test]
    fn the_search_option_rows_are_listed_without_shortcuts() {
        // Break caught: a toggle the palette never offers, or one showing a shortcut it doesn't
        // have (the Alt keys work only inside the Search box and the find bar).
        assert_eq!(labels("toggle match case")[0], "Search: Toggle match case");
        assert_eq!(labels("whole word")[0], "Search: Toggle whole word");
        assert_eq!(
            labels("regular expression")[0],
            "Search: Toggle regular expression"
        );
        for command in [
            CommandId::SearchToggleCase,
            CommandId::SearchToggleWholeWord,
            CommandId::SearchToggleRegex,
        ] {
            assert_eq!(shortcut_text(command), None, "{command:?}");
        }
    }
```

3. In `shortcuts_are_spelled_from_the_accelerator_table`, before `assert_eq!(shortcut_text(CommandId::Copy), None);`, add:

```rust
        // Break caught: Format JSON's row still showing Ctrl+Shift+F, now Search's shortcut.
        assert_eq!(
            shortcut_text(CommandId::FormatJson).as_deref(),
            Some("Shift+Alt+F")
        );
```

`every_command_except_tab_positions_and_the_palette_is_listed_once` needs no change. The three new commands are listed once, as it requires.

Run: `cargo test --lib -- window::menus window::command_palette --test-threads=1`
Expected: all pass.

- [ ] **Step 5: Write the failing window tests** in `src/window/main_window.rs`'s `tests` module

Add them after `a_hidden_search_view_searches_again_only_once_it_shows`. `LibraryScratch`, `pump_until` and `type_into_search` already exist there.

```rust
    #[test]
    fn ctrl_shift_f_takes_a_single_line_selection_and_ignores_a_multi_line_one() {
        // Break caught: Ctrl+Shift+F ignoring the selection, pasting a multi-line one into the
        // box, or clearing the box when nothing is selected.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("search-prefill-selection");
        scratch.note("a.md", "alpha beta");
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        scratch.install(window.hwnd);
        let query = || {
            crate::window::search_view::current_query(window.hwnd).map(|(query, _)| query)
        };
        editor.populate_clean("alpha beta\r\ngamma").unwrap();

        editor.set_selection(6..10).unwrap();
        execute_command(window.hwnd, CommandId::ShowSearchView);
        assert_eq!(
            crate::window::side_panel::current_view(window.hwnd),
            crate::config::SidebarView::Search
        );
        assert_eq!(query().as_deref(), Some("beta"));
        // The prefill searches at once, with no keystroke to start the debounce.
        pump_until(window.hwnd, || {
            crate::window::search_view::shown_results(window.hwnd).len() == 1
        });

        editor.set_selection(6..14).unwrap();
        execute_command(window.hwnd, CommandId::ShowSearchView);
        assert_eq!(query().as_deref(), Some("beta"), "a multi-line selection");

        editor.set_selection(3..3).unwrap();
        execute_command(window.hwnd, CommandId::ShowSearchView);
        assert_eq!(query().as_deref(), Some("beta"), "no selection");
    }

    #[test]
    fn ctrl_shift_f_escapes_the_selection_while_regex_is_on() {
        // Break caught: "a.b" searched as a pattern that also matches "axb".
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("search-prefill-regex");
        scratch.note("a.md", "see a.b here");
        scratch.note("x.md", "see axb here");
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        scratch.install(window.hwnd);
        crate::window::side_panel::show_view(
            window.hwnd,
            crate::config::SidebarView::Search,
            false,
        );
        crate::window::search_view::toggle_option(
            window.hwnd,
            crate::search::SearchOption::Regex,
        );
        editor.populate_clean("see a.b here").unwrap();
        editor.set_selection(4..7).unwrap();

        execute_command(window.hwnd, CommandId::ShowSearchView);

        assert_eq!(
            crate::window::search_view::current_query(window.hwnd).map(|(query, _)| query),
            Some(r"a\.b".to_owned())
        );
        pump_until(window.hwnd, || {
            !crate::window::search_view::shown_results(window.hwnd).is_empty()
        });
        assert_eq!(
            crate::window::search_view::shown_results(window.hwnd),
            vec![("a".to_owned(), "see a.b here".to_owned())]
        );
    }

    #[test]
    fn the_search_toggle_commands_show_search_and_flip_its_options() {
        // Break caught: a palette toggle that flips an option nobody can see, or flips the
        // wrong one.
        use crate::search::MatchOptions;
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("search-toggle-commands");
        scratch.note("a.md", "a");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        scratch.install(window.hwnd);
        crate::window::side_panel::show_view(
            window.hwnd,
            crate::config::SidebarView::Notebook,
            false,
        );
        let options = || crate::window::search_view::options(window.hwnd);

        execute_command(window.hwnd, CommandId::SearchToggleCase);
        assert_eq!(
            crate::window::side_panel::current_view(window.hwnd),
            crate::config::SidebarView::Search
        );
        assert_eq!(
            options(),
            MatchOptions {
                case: true,
                ..MatchOptions::default()
            }
        );
        execute_command(window.hwnd, CommandId::SearchToggleWholeWord);
        execute_command(window.hwnd, CommandId::SearchToggleRegex);
        assert_eq!(
            options(),
            MatchOptions {
                case: true,
                whole_word: true,
                regex: true
            }
        );
        execute_command(window.hwnd, CommandId::SearchToggleCase);
        assert!(!options().case);
    }

    #[test]
    fn with_notes_mode_off_ctrl_shift_f_and_the_search_toggles_do_nothing() {
        // Break caught: a sidebar command reaching code that assumes a sidebar, or reading and
        // changing editor state with notes mode off.
        let _scintilla = load_native_scintilla();
        let mut app = make_app();
        app.settings.notes_mode = false;
        let window = ProductionWindow::new(app);
        let editor = install_test_editor(&window);
        editor.populate_clean("alpha beta").unwrap();
        editor.set_selection(0..5).unwrap();

        for command in [
            CommandId::ShowSearchView,
            CommandId::SearchToggleCase,
            CommandId::SearchToggleWholeWord,
            CommandId::SearchToggleRegex,
        ] {
            execute_command(window.hwnd, command);
        }

        assert!(app_mut(window.hwnd).sidebar.is_none());
        assert_eq!(
            crate::window::side_panel::current_view(window.hwnd),
            crate::config::SidebarView::Hidden
        );
        assert_eq!(editor.selection().unwrap(), 0..5);
        assert!(notices(window.hwnd).is_empty());
    }

    #[test]
    fn shift_alt_f_formats_json_and_ctrl_shift_f_no_longer_does() {
        // Break caught: Format JSON left on Ctrl+Shift+F, where it would rewrite a JSON file
        // the user only meant to search from, or not reachable from any shortcut.
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
            GetKeyboardState, SetKeyboardState, VK_CONTROL, VK_MENU, VK_SHIFT,
        };
        use windows_sys::Win32::UI::WindowsAndMessaging::{MSG, WM_KEYDOWN, WM_SYSKEYDOWN};
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        let identity = unsafe { super::window_identity(window.hwnd).unwrap() };
        // Presses F with the given modifiers held, through the accelerator table.
        let press_f = |ctrl: bool, shift: bool, alt: bool| {
            let mut keys = [0u8; 256];
            unsafe { GetKeyboardState(keys.as_mut_ptr()) };
            let original = keys;
            keys[VK_CONTROL as usize] = if ctrl { 0x80 } else { 0 };
            keys[VK_SHIFT as usize] = if shift { 0x80 } else { 0 };
            keys[VK_MENU as usize] = if alt { 0x80 } else { 0 };
            unsafe { SetKeyboardState(keys.as_ptr()) };
            let message = MSG {
                hwnd: editor.hwnd(),
                message: if alt { WM_SYSKEYDOWN } else { WM_KEYDOWN },
                wParam: usize::from(b'F'),
                // Bit 29, the context code, is set while Alt is down.
                lParam: if alt { 1 << 29 } else { 0 },
                ..Default::default()
            };
            let translated =
                unsafe { super::translate_accelerator(window.hwnd, &identity, &message) };
            unsafe { SetKeyboardState(original.as_ptr()) };
            translated
        };
        editor.populate_clean("{\"a\":1}").unwrap();

        assert!(press_f(true, true, false));
        pump_posted_messages(window.hwnd);
        assert_eq!(editor.text().unwrap(), "{\"a\":1}", "Ctrl+Shift+F leaves JSON alone");

        assert!(press_f(false, true, true));
        pump_posted_messages(window.hwnd);
        assert_eq!(editor.text().unwrap(), "{\n  \"a\": 1\n}");
    }
```

`translate_accelerator` delivers the `WM_COMMAND` with `SendMessage` semantics (`TranslateAcceleratorW` sends it), so the command has run when it returns. `pump_posted_messages` only drains the deferred language work that some commands post.

In `ctrl_b_toggles_the_sidebar_back_to_the_last_view_and_saves_each_change`, replace

```rust
        assert!(press(b'K', false));
        assert_eq!(view(), SidebarView::Search);
```

with

```rust
        // Break caught: Ctrl+K still bound after Search moved to Ctrl+Shift+F.
        assert!(!press(b'K', false));
        assert_eq!(view(), SidebarView::Hidden);
        assert!(press(b'F', true));
        assert_eq!(view(), SidebarView::Search);
```

Run: `cargo test --lib -- window::main_window::tests::ctrl_shift_f window::main_window::tests::the_search_toggle_commands window::main_window::tests::with_notes_mode_off_ctrl_shift_f window::main_window::tests::shift_alt_f window::main_window::tests::ctrl_b_toggles --test-threads=1`
Expected: the new tests fail. Ctrl+Shift+F only shows the view, and the toggle commands reach `App::execute`, which ignores them. `ctrl_b_toggles…` passes, because Step 3 already moved the accelerators.

- [ ] **Step 6: Implement the commands** in `src/window/main_window.rs`

1. In `execute_command_with_note`, after the `needs_document` early return (`if command.needs_document() && tab_count(hwnd) == 0 && tree_note.is_none() { return; }`), add:

```rust
    // Sidebar commands do nothing with notes mode off: there is no sidebar to act on (spec §5).
    if command.is_sidebar() && !notes_mode_enabled(hwnd) {
        return;
    }
```

2. Replace the `CommandId::ShowSearchView` arm with:

```rust
        CommandId::ShowSearchView => show_search_view(hwnd),
        CommandId::SearchToggleCase
        | CommandId::SearchToggleWholeWord
        | CommandId::SearchToggleRegex => {
            if let Some(option) = command.search_option() {
                toggle_search_option(hwnd, option);
            }
        }
```

3. Add these functions after `open_find_bar`:

```rust
/// The active editor's selection as a query, when it is non-empty and on one line. A multi-line
/// selection can't be shown in a one-line box, so it is left alone rather than cut.
fn single_line_selection(hwnd: HWND) -> Option<String> {
    let editor =
        unsafe { app_ptr(hwnd) }.and_then(|app| unsafe { app.as_ref() }.editor.clone())?;
    let text = editor.selected_text().ok()?;
    (!text.is_empty() && !text.contains(['\n', '\r'])).then_some(text)
}

/// Ctrl+Shift+F (spec §5) shows Search and focuses its box. A single-line selection in the
/// active editor replaces the box's text and searches at once. `show_with_query` escapes it
/// while regex is on.
fn show_search_view(hwnd: HWND) {
    // Read before the box takes the focus.
    let prefill = single_line_selection(hwnd);
    crate::window::side_panel::show_view(hwnd, crate::config::SidebarView::Search, true);
    if let Some(text) = prefill {
        crate::window::search_view::show_with_query(hwnd, &text);
    }
}

/// A `SearchToggle*` palette command shows the Search view first if it is hidden, so the
/// option it flips is visible.
fn toggle_search_option(hwnd: HWND, option: crate::search::SearchOption) {
    if crate::window::side_panel::current_view(hwnd) != crate::config::SidebarView::Search {
        crate::window::side_panel::show_view(hwnd, crate::config::SidebarView::Search, true);
    }
    crate::window::search_view::toggle_option(hwnd, option);
}
```

4. In `open_find_bar`, replace the `prefill` block (the comment and the `let prefill = unsafe { app_ptr(hwnd) }.and_then(…);` statement) with:

```rust
    let prefill = single_line_selection(hwnd);
```

5. In `src/window/search_view.rs`, remove the `#[cfg_attr(not(test), expect(dead_code, …))]` attribute above `show_with_query` and the one above `options`. `show_search_view` now calls `show_with_query`, which calls `options`, so both are live outside the tests, and a kept `expect` would fail clippy as an unfulfilled expectation.

Run: the Step 5 command.
Expected: all pass.

- [ ] **Step 7: Update the comments and docs that name Ctrl+K or Ctrl+Shift+F**

1. `src/window/search_view.rs` needs no comment change: Task 5 rewrote `shown`'s doc comment, which named Ctrl+K, without a shortcut. The `git grep` below confirms it.
2. `tests/windows/json_commands.rs`: in `format_json_reformats_with_two_spaces_and_is_undone_in_one_step`, change the comment's `Ctrl+Shift+F's Format JSON command` to `Shift+Alt+F's Format JSON command`.
3. `README.md`:
   - In "Notes and notebooks", replace
     `- **Ctrl+K** searches note names. Star a notebook to keep it in **Favorites**, and switch`
     with
     `- **Ctrl+Shift+F** searches your notes. Star a notebook to keep it in **Favorites**, and switch`.
     Task 9 expands this bullet.
   - In "JSON you can trust", replace `**Format JSON** (Ctrl+Shift+F)` with `**Format JSON** (Shift+Alt+F)`.
   - In the shortcuts table, change the cell `` `Ctrl+Shift+F` `` in the Format JSON row to `` `Shift+Alt+F` ``, and the cell `` `Ctrl+K` `` in the Search notes row to `` `Ctrl+Shift+F` ``.
4. `docs/superpowers/specs/2026-09-23-note-sidebar-design.md`:
   - Replace `Ctrl+K shows Search and focuses its box.` with `Ctrl+K showed Search and focused its box; the note search spec (2026-09-24, §5) moves this to Ctrl+Shift+F and removes Ctrl+K.`
   - Replace `  - View: Show search (Ctrl+K).` with `  - View: Show search (Ctrl+K; Ctrl+Shift+F since the note search spec, §5).`

Check that nothing else still binds or documents Ctrl+K:

Run: `git grep -n "Ctrl+K" -- src tests README.md docs/superpowers/specs`
Expected: only the two sidebar-spec lines just edited, which now also name Ctrl+Shift+F.

- [ ] **Step 8: Lint, format, commit**

Run: `cargo clippy --all-targets -- -D warnings`, then `cargo fmt --all -- --check`
Expected: no warnings, and no formatting diff.

```
git add src/window/commands.rs src/window/menus.rs src/window/command_palette.rs src/window/main_window.rs src/window/search_view.rs tests/windows/json_commands.rs README.md docs/superpowers/specs/2026-09-23-note-sidebar-design.md
git commit -m "feat(search): Ctrl+Shift+F shows Search with the selection, Format JSON moves to Shift+Alt+F, Ctrl+K is gone, and the palette toggles the search options"
```

---

### Task 7: Find bar (toggles, flags, F3, opening a Search result)

**Files:**
- Modify:
  - `src/editor/scintilla_constants.rs`
  - `tools/generate-scintilla-constants.ps1`
  - `src/editor/scintilla.rs`
  - `src/window/find_bar.rs`
  - `src/window/main_window.rs`
  - `src/window/search_view.rs` (only `open_result`)
  - `src/window/commands.rs`, `src/window/menus.rs`, `src/window/command_palette.rs` (F3 and Shift+F3)

**Findings this task rests on (checked against the sources in `native\src`):**
- `SCFIND_REGEXP` (`0x00200000`), `SCFIND_POSIX` (`0x00400000`) and `SCFIND_CXX11REGEX` (`0x00800000`) exist in Scintilla 5.6.6's `Scintilla.iface`. `scintilla_constants.rs` has only `SCFIND_NONE`, `SCFIND_WHOLEWORD` and `SCFIND_MATCHCASE`.
- **C++11 regex is compiled in.** `Document.cxx` guards it with `#ifndef NO_CXX11_REGEX`. `scintilla.mak` defines `NO_CXX11_REGEX` only when the nmake macro of that name is set, and `tools/build-native.ps1` runs `nmake /nologo -f scintilla.mak` without it. Step 9's `\d{2}` test pins this against the real DLL: `{n}` quantifiers exist only in the C++11 dialect. If that assertion ever fails, the DLL was built without C++11 regex. `search_flags` then falls back to `SCFIND_REGEXP | SCFIND_POSIX`, and the spec §17 note says so.
- **A regex error is a miss.** `Editor::SearchInTarget` catches `RegexError`, sets `errorStatus = Status::RegEx` and returns `-1`. Older builds return `-2`. `search_in_target` already treats every negative result as `None`. The sticky status is never read (`SCI_GETSTATUS` is unused). So an invalid pattern is simply "no match": no error, no notice, no panic. Step 3 pins `-2` as well.
- **A regex match is as long as the text it matched.** `search_in_target` currently returns `start..start + needle.len()`, which is wrong for regex. It must read `SCI_GETTARGETEND` (2193) after a hit.
- **Scintilla ignores the word flags for regex.** `BuiltinRegex::FindText` leaves its `word` and `wordStart` parameters unnamed. Whole word in regex mode therefore wraps the pattern as `\b(?:…)\b`, as the Search view does (spec §6).
- **There is no F3 today.** No accelerator, command or key handler exists. Spec §8 assumes F3 and Shift+F3, so this task adds `FindNext = 188` and `FindPrevious = 189`. Global Constraints reserve 188 and 189 for them, after the toggles' 185–187; 3b starts at 190.
- **There is no "no match" state today.** A miss leaves the selection as it was, and nothing shows it. This task adds one: the query field's outline turns `Palette.error_foreground` (the error color Task 5 added for the Search view's pattern error) until the query changes or a search finds something.
- **`scintilla_constants.rs` is out of sync with its generator.** `SCI_GETANCHOR`, `SCI_GETLINE` and `SCI_LINELENGTH` were added by hand. Regenerating now would drop them, so the new constants are also hand-added, and all six names go into the generator's list.

**Interfaces:**
- Consumes:
  - `MatchOptions`, `SearchOption` (Task 1).
  - `option_toggles::{toggle_rects, reserved_width, paint, hit, alt_key, tooltip}` and `Palette.error_foreground` (Task 5).
  - `search_view::current_query` and `text_search_host::run_now` (Task 4).
  - `single_line_selection` (Task 6).
- Produces:
  - `FindBar.options: MatchOptions` and `FindBar::{options, no_match, set_no_match, toggle_option, show_with, toggle_rects, panel_hwnd}`. `panel_hwnd` is no longer `cfg(test)`.
  - `find_bar::{search_flags, scintilla_query, escape_pattern, BarClick, PendingText}`.
  - `main_window::{open_search_result, toggle_find_option, find_bar_owns}`. `find_bar_owns` becomes `pub(crate)` for Task 8.
  - `CommandId::FindNext = 188`, `CommandId::FindPrevious = 189`.
- **Signature notes against the contract:**
  - `show_with(&mut self, mode, query, options, colors)` keeps its parameters but returns a `PendingText`. The caller applies it after dropping the `App` borrow, because `SetWindowTextW` on the field sends `EN_CHANGE` back to the main window (App-borrow rule).
  - `FindBar::show` changes the same way, to `-> Option<PendingText>`.
- Table sizes grow by these amounts, applied to whatever the table holds after Task 6:
  - `COMMANDS` by 2.
  - `accelerator_specs()` by 2.
  - `ENTRIES` by 2.

- [ ] **Step 1: Add the Scintilla constants**

1. In `tools/generate-scintilla-constants.ps1`'s `$RequiredNames`, change the last line `"SCI_VISIBLEFROMDOCLINE", "SC_UPDATE_V_SCROLL", "SCI_GOTOPOS", "SCI_DOCUMENTEND"` to:

```powershell
    "SCI_VISIBLEFROMDOCLINE", "SC_UPDATE_V_SCROLL", "SCI_GOTOPOS", "SCI_DOCUMENTEND",
    "SCI_GETANCHOR", "SCI_GETLINE", "SCI_LINELENGTH",
    "SCFIND_REGEXP", "SCFIND_CXX11REGEX", "SCFIND_POSIX", "SCI_GETTARGETEND"
```

   The first three are the hand-added names already in the file; listing them keeps a future regeneration from dropping them.

2. In `src/editor/scintilla_constants.rs`, add in sorted position (`SCFIND_*` after the `SCE_*` block, `SCI_GETTARGETEND` between `SCI_GETTABWIDTH` and `SCI_GETTEXT`):

```rust
pub const SCFIND_CXX11REGEX: u32 = 0x00800000;
```
before `pub const SCFIND_MATCHCASE: u32 = 0x4;`;

```rust
pub const SCFIND_POSIX: u32 = 0x00400000;
pub const SCFIND_REGEXP: u32 = 0x00200000;
```
between `pub const SCFIND_NONE: u32 = 0x0;` and `pub const SCFIND_WHOLEWORD: u32 = 0x2;`;

```rust
pub const SCI_GETTARGETEND: u32 = 2193;
```
between `pub const SCI_GETTABWIDTH: u32 = 2121;` and `pub const SCI_GETTEXT: u32 = 2182;`.

`SCFIND_POSIX` is added only for the fallback that the findings above describe. Nothing uses it yet, and an unused `pub const` in a `pub mod` doesn't warn.

- [ ] **Step 2: Write the failing editor tests** in `src/editor/scintilla.rs`'s `tests` module

1. Add `SCI_GETTARGETEND` to the first `use crate::editor::scintilla_constants::{…}` list in `mod tests`.

2. In `TestDirectState`, add two fields after `search_needle`:

```rust
        /// Where the last hit's target ends: its position plus the needle's length, as a plain
        /// search reports it, unless `target_ends` scripts another end (a regex match).
        last_target_end: isize,
        target_ends: VecDeque<isize>,
```

3. In `impl TestDirectHarness`, add:

```rust
        /// Scripts `SCI_GETTARGETEND` for the next hit, as a regex match of another length would.
        fn push_target_end(&self, end: isize) {
            self.state.lock().unwrap().target_ends.push_back(end);
        }
```

4. In `test_direct`, replace the `SCI_SEARCHINTARGET` arm with the following, and add the `SCI_GETTARGETEND` arm after it:

```rust
            SCI_SEARCHINTARGET => {
                let bytes = unsafe { std::slice::from_raw_parts(lparam as *const u8, wparam) };
                state.search_needle = Some(bytes.to_vec());
                let found = state.responses.pop_front().unwrap_or(-1);
                if found >= 0 {
                    state.last_target_end = found + wparam as isize;
                }
                found
            }
            SCI_GETTARGETEND => {
                let scripted = state.target_ends.pop_front();
                scripted.unwrap_or(state.last_target_end)
            }
```

5. In `search_in_target_sets_range_and_flags_before_searching`, the expected messages become:

```rust
            vec![
                SCI_SETTARGETRANGE,
                SCI_SETSEARCHFLAGS,
                SCI_SEARCHINTARGET,
                SCI_GETTARGETEND
            ]
```

6. Add:

```rust
    #[test]
    fn search_in_target_reports_the_length_scintilla_matched() {
        // Break caught: a regex hit reported as long as the pattern, so `\d+` over "12345"
        // selects three characters, or a find-next that starts inside the previous match.
        let harness = TestDirectHarness::new();
        harness.push_response(2);
        harness.push_target_end(7);
        let editor = Editor::test_fixture(test_direct, harness.direct_ptr());

        assert_eq!(
            editor.search_in_target(r"\d+", 0..10, 0).unwrap(),
            Some(2..7)
        );
    }

    #[test]
    fn a_pattern_scintilla_cannot_compile_is_a_miss_not_an_error() {
        // Break caught: Scintilla's -2 (a bad regex in some versions) turned into an error or a
        // bogus range, which the find bar would report or panic on.
        let harness = TestDirectHarness::new();
        harness.push_response(-2);
        let editor = Editor::test_fixture(test_direct, harness.direct_ptr());

        assert_eq!(editor.search_in_target("(", 0..10, 0).unwrap(), None);
    }

    #[test]
    fn replace_all_stops_at_an_empty_match() {
        // Break caught: a regex that can match empty text (`x*`) replacing at the same
        // position forever, or inserting the replacement between every character.
        let harness = TestDirectHarness::new();
        harness.push_response(0); // SCI_BEGINUNDOACTION
        harness.push_response(5); // SCI_GETLENGTH
        harness.push_response(0); // SCI_SEARCHINTARGET: an empty match at 0
        harness.push_target_end(0);
        harness.push_response(0); // SCI_ENDUNDOACTION
        let editor = Editor::test_fixture(test_direct, harness.direct_ptr());

        assert_eq!(editor.replace_all("x*", "y", 0).unwrap(), 0);
        assert!(harness.replace_bytes().is_empty());
        assert_eq!(harness.event_log(), vec!["begin", "end"]);
    }
```

Run: `cargo test --lib -- editor::scintilla --test-threads=1`
Expected: `search_in_target_reports_the_length_scintilla_matched`, `replace_all_stops_at_an_empty_match` and the edited `search_in_target_sets_range_and_flags_before_searching` fail. `a_pattern_scintilla_cannot_compile_is_a_miss_not_an_error` passes already.

- [ ] **Step 3: Read the match end, and stop replace-all at an empty match** in `src/editor/scintilla.rs`

1. Add `SCI_GETTARGETEND` to the top-level `use crate::editor::scintilla_constants::{…}` list.

2. Replace the Windows `search_in_target` with:

```rust
    /// Searches `range` for `needle` with `search_flags`. A backward search passes a range whose
    /// start is after its end. The match's end is Scintilla's own target end, so a regular
    /// expression's match is as long as the text it matched, not as long as the pattern.
    /// Scintilla reports a pattern it can't compile as a miss (-1, or -2 in some versions), and
    /// so does this: `Ok(None)`, never an error.
    #[cfg(windows)]
    pub fn search_in_target(
        &self,
        needle: &str,
        range: Range<usize>,
        search_flags: u32,
    ) -> Result<Option<Range<usize>>> {
        let needle = CString::new(needle).map_err(|_| {
            FastPadError::Invariant("Scintilla search text may not contain NUL bytes")
        })?;
        self.endpoint
            .send_direct_checked(SCI_SETTARGETRANGE, range.start, range.end as isize)?;
        self.endpoint
            .send_direct_checked(SCI_SETSEARCHFLAGS, search_flags as usize, 0)?;
        let found = self.endpoint.send_direct_checked(
            SCI_SEARCHINTARGET,
            needle.as_bytes().len(),
            needle.as_ptr() as isize,
        )?;
        if found < 0 {
            return Ok(None);
        }
        let end = self
            .endpoint
            .send_direct_checked(SCI_GETTARGETEND, 0, 0)?
            .max(found);
        Ok(Some(found as usize..end as usize))
    }
```

3. In the Windows `replace_all`, replace

```rust
                let Some(found) = self.search_in_target(query, position..length, search_flags)?
                else {
                    break;
                };
```

with

```rust
                let Some(found) = self.search_in_target(query, position..length, search_flags)?
                else {
                    break;
                };
                // A regex that matched empty text would match at the same place forever.
                if found.is_empty() {
                    break;
                }
```

Run: `cargo test --lib -- editor::scintilla --test-threads=1`
Expected: all pass.

- [ ] **Step 4: Write the failing find-bar unit tests** in `src/window/find_bar.rs`'s `tests` module

1. Extend the stub. Add `SCI_GETTARGETEND` to the `use crate::editor::scintilla_constants::{…}` list in `mod tests`. Replace `TargetLog` and `target_range_stub` with:

```rust
    #[derive(Default)]
    struct TargetLog {
        responses: VecDeque<isize>,
        ranges: Vec<(usize, isize)>,
        /// Scripted target ends; otherwise a hit ends one needle-length after it starts.
        ends: VecDeque<isize>,
        last_end: isize,
    }

    unsafe extern "C" fn target_range_stub(
        direct_ptr: isize,
        message: u32,
        wparam: usize,
        lparam: isize,
    ) -> isize {
        let shared = unsafe { &*(direct_ptr as *const Mutex<TargetLog>) };
        let mut log = shared.lock().unwrap();
        match message {
            SCI_SETTARGETRANGE => {
                log.ranges.push((wparam, lparam));
                0
            }
            SCI_SETSEARCHFLAGS => 0,
            SCI_SEARCHINTARGET => {
                let found = log.responses.pop_front().unwrap_or(-1);
                if found >= 0 {
                    log.last_end = found + wparam as isize;
                }
                found
            }
            SCI_GETTARGETEND => {
                let scripted = log.ends.pop_front();
                scripted.unwrap_or(log.last_end)
            }
            _ => 0,
        }
    }
```

2. Add:

```rust
    #[test]
    fn options_map_to_scintilla_flags_and_a_whole_word_regex_is_wrapped() {
        // Break caught: the toggles changing nothing, whole word silently ignored in regex mode
        // (Scintilla's regex search drops the word flags), or plain text wrapped as a pattern.
        use super::{scintilla_query, search_flags};
        use crate::editor::scintilla_constants::{
            SCFIND_CXX11REGEX, SCFIND_MATCHCASE, SCFIND_REGEXP, SCFIND_WHOLEWORD,
        };
        use crate::search::MatchOptions;
        let plain = MatchOptions::default();
        let case = MatchOptions {
            case: true,
            ..plain
        };
        let word = MatchOptions {
            whole_word: true,
            ..plain
        };
        let regex = MatchOptions {
            regex: true,
            ..plain
        };
        let all = MatchOptions {
            case: true,
            whole_word: true,
            regex: true,
        };
        assert_eq!(search_flags(plain), 0);
        assert_eq!(search_flags(case), SCFIND_MATCHCASE);
        assert_eq!(search_flags(word), SCFIND_WHOLEWORD);
        assert_eq!(search_flags(regex), SCFIND_REGEXP | SCFIND_CXX11REGEX);
        assert_eq!(
            search_flags(all),
            SCFIND_MATCHCASE | SCFIND_REGEXP | SCFIND_CXX11REGEX
        );
        assert_eq!(scintilla_query("a|b", all), r"\b(?:a|b)\b");
        assert_eq!(scintilla_query("a|b", regex), "a|b");
        assert_eq!(scintilla_query("a|b", word), "a|b");
    }

    #[test]
    fn a_prefilled_selection_is_escaped_for_ecmascript() {
        // Break caught: a selected "a.b" matching "axb" with regex on, or an escape ECMAScript
        // rejects (`\#`, `\-`) making every prefill an invalid pattern.
        use super::escape_pattern;
        assert_eq!(
            escape_pattern(r"a.b*(c)[d]{2}^$|?+\"),
            r"a\.b\*\(c\)\[d\]\{2\}\^\$\|\?\+\\"
        );
        assert_eq!(escape_pattern("plain words #1 & -2"), "plain words #1 & -2");
    }

    #[test]
    fn the_query_field_leaves_room_for_the_three_toggles() {
        // Break caught: typed text running under the toggles, or toggles outside the field.
        use super::{FindBarMode, bar_layout};
        use crate::window::option_toggles::toggle_rects;
        for dpi in [96, 144] {
            for mode in [FindBarMode::Find, FindBarMode::Replace] {
                let query = bar_layout(800, dpi, 16, mode).query;
                let rects = toggle_rects(query.field, dpi);
                assert!(query.edit.right <= rects[0].left, "{dpi} {mode:?}");
                assert!(rects[2].right <= query.field.right, "{dpi} {mode:?}");
                for rect in rects {
                    assert!(rect.top >= query.field.top && rect.bottom <= query.field.bottom);
                }
            }
        }
    }

    #[test]
    fn a_regex_error_or_an_empty_match_is_no_match() {
        // Break caught: a bad pattern reported as an error, or an empty match "found" at the
        // caret forever, so F3 never moves.
        let log = Mutex::new(TargetLog {
            responses: VecDeque::from([-2_isize, -2, 4, 4]),
            ends: VecDeque::from([4_isize, 4]),
            ..TargetLog::default()
        });
        let editor =
            Editor::test_fixture(target_range_stub, &log as *const Mutex<TargetLog> as isize);

        let mut bad = SearchState::new("(", SearchDirection::Forward, 0);
        assert_eq!(bad.next_editor_match(&editor, 0, 11).unwrap(), None);

        let mut empty = SearchState::new("x*", SearchDirection::Forward, 4);
        assert_eq!(empty.next_editor_match(&editor, 0, 11).unwrap(), None);
    }
```

Run: `cargo test --lib -- window::find_bar --test-threads=1`
Expected: compile errors, because `search_flags`, `scintilla_query` and `escape_pattern` don't exist yet.

- [ ] **Step 5: Implement the pure find-bar pieces** in `src/window/find_bar.rs`

1. In `SearchState::next_editor_match`, drop empty matches. Replace the body after the empty-query check with:

```rust
        let bounds = self.scintilla_bounds(doc_len);
        // An empty match (a regex like `x*`) would be found at the caret again and again.
        let found = editor
            .search_in_target(&self.query, bounds, flags)?
            .filter(|found| !found.is_empty());
        if let Some(found) = found {
            self.record_match(found.clone());
            return Ok(Some(found));
        }
        if self.wrapped {
            return Ok(None);
        }
        self.record_miss(doc_len);
        let bounds = self.scintilla_bounds(doc_len);
        let found = editor
            .search_in_target(&self.query, bounds, flags)?
            .filter(|found| !found.is_empty());
        if let Some(found) = &found {
            self.record_match(found.clone());
        }
        Ok(found)
```

2. After the `SearchState` impl, before the `// --- Window integration` line, add:

```rust
use crate::editor::scintilla_constants::{
    SCFIND_CXX11REGEX, SCFIND_MATCHCASE, SCFIND_NONE, SCFIND_REGEXP, SCFIND_WHOLEWORD,
};
use crate::search::{MatchOptions, SearchOption};

/// The Scintilla search flags for `options` (spec §8). Whole word in regex mode is carried by
/// the pattern instead (`scintilla_query`): Scintilla's regex search ignores the word flags.
pub(crate) fn search_flags(options: MatchOptions) -> u32 {
    let mut flags = SCFIND_NONE;
    if options.case {
        flags |= SCFIND_MATCHCASE;
    }
    if options.regex {
        flags |= SCFIND_REGEXP | SCFIND_CXX11REGEX;
    } else if options.whole_word {
        flags |= SCFIND_WHOLEWORD;
    }
    flags
}

/// What Scintilla searches for: the query, wrapped as `\b(?:…)\b` for a whole-word regex, as
/// the Search view wraps it (spec §6).
pub(crate) fn scintilla_query(query: &str, options: MatchOptions) -> String {
    if options.regex && options.whole_word {
        format!(r"\b(?:{query})\b")
    } else {
        query.to_owned()
    }
}

/// `text` as a pattern that matches it literally in Scintilla's ECMAScript regex, for a
/// selection prefilled while regex is on. Only ECMAScript's syntax characters are escaped,
/// because there an identity escape of anything else (`\#`, `\-`) is an error.
pub(crate) fn escape_pattern(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(
            c,
            '\\' | '^' | '$' | '.' | '|' | '?' | '*' | '+' | '(' | ')' | '[' | ']' | '{' | '}'
        ) {
            escaped.push('\\');
        }
        escaped.push(c);
    }
    escaped
}
```

3. Give the query field room for the toggles. Replace `field_layouts` with:

```rust
/// The fields across `width` (the right padding included), starting `top` pixels down the bar.
/// The query field's right end holds the three option toggles (spec §8).
fn field_layouts(
    width: i32,
    dpi: u32,
    text_height: i32,
    mode: FindBarMode,
    top: i32,
) -> (FieldLayout, Option<FieldLayout>) {
    let padding = scale(PADDING_AT_96_DPI, dpi);
    let field_height = scale(FIELD_HEIGHT_AT_96_DPI, dpi);
    let inset_x = scale(FIELD_TEXT_INSET_AT_96_DPI, dpi);
    let text_height = text_height.clamp(1, (field_height - 2).max(1));
    let toggles = option_toggles::reserved_width(dpi);
    // `reserve` is how far the Edit stops short of the field's right edge.
    let field = |left: i32, right: i32, reserve: i32| {
        let field = RECT {
            left,
            top,
            right: right.max(left),
            bottom: top + field_height,
        };
        let edit_top = top + (field_height - text_height) / 2;
        FieldLayout {
            field,
            edit: RECT {
                left: left + inset_x,
                top: edit_top,
                right: (field.right - reserve).max(left + inset_x),
                bottom: edit_top + text_height,
            },
        }
    };
    match mode {
        FindBarMode::Find => (field(padding, width - padding, toggles), None),
        FindBarMode::Replace => {
            let half = (width - 3 * padding) / 2;
            (
                field(padding, padding + half, toggles),
                // An odd leftover pixel stays at the right edge so both fields match.
                Some(field(
                    2 * padding + half,
                    2 * padding + 2 * half,
                    inset_x,
                )),
            )
        }
    }
}
```

4. Extend the window-integration `use` lines:
   - Add `use crate::window::option_toggles;` and `use crate::window::tooltip::Tooltip;`.
   - Add `POINT` to the `windows_sys::Win32::Foundation` import.
   - Add `WM_SYSCHAR, WM_SYSKEYDOWN` to the `WindowsAndMessaging` import.

Run: `cargo test --lib -- window::find_bar --test-threads=1`
Expected: the four new tests pass. `find_fields_center_their_text_and_replace_mode_splits_the_bar_without_overlap` still passes, because the fields' outer rectangles didn't move.

- [ ] **Step 6: Give `FindBar` its options, no-match state, toggles and tooltip** in `src/window/find_bar.rs`

1. Add to `struct FindBar`, after `close_hovered`:

```rust
    /// Match case, whole word and regex (spec §8). They last for the session, and opening a
    /// Search result replaces them with Search's.
    options: MatchOptions,
    /// The last search found nothing. Cleared when the query changes or a search finds a match.
    no_match: Cell<bool>,
    hovered_toggle: Cell<Option<SearchOption>>,
    /// The toggles' tooltip, made the first time the pointer moves over the bar.
    tooltip: Cell<Option<Tooltip>>,
    tooltip_failed: Cell<bool>,
```

   Initialize them in `create`:

```rust
            options: MatchOptions::default(),
            no_match: Cell::new(false),
            hovered_toggle: Cell::new(None),
            tooltip: Cell::new(None),
            tooltip_failed: Cell::new(false),
```

2. Add these types before `impl FindBar`:

```rust
/// What a click released on the bar hit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BarClick {
    Close,
    Toggle(SearchOption),
}

/// Text to put in the query field once nothing of the `App` is borrowed. Setting it sends
/// `EN_CHANGE`, and the main window handles that by borrowing the find bar again.
#[must_use = "the text only reaches the field when applied"]
pub(crate) struct PendingText {
    edit: HWND,
    text: String,
}

impl PendingText {
    pub(crate) fn apply(self) {
        set_control_text(self.edit, &self.text);
    }
}
```

3. Replace `show` with the following, and add `show_with` after it:

```rust
    /// Shows the bar in `mode`. The caller applies the returned prefill once it holds no `App`
    /// borrow.
    pub(crate) fn show(
        &mut self,
        mode: FindBarMode,
        prefill: Option<&str>,
        colors: Palette,
    ) -> Option<PendingText> {
        self.set_colors(colors);
        self.mode = mode;
        self.visible = true;
        self.no_match.set(false);
        unsafe {
            ShowWindow(
                self.replace_edit,
                if mode == FindBarMode::Replace {
                    SW_SHOWNA
                } else {
                    SW_HIDE
                },
            );
        }
        prefill.map(|text| PendingText {
            edit: self.query_edit,
            text: text.to_owned(),
        })
    }

    /// Shows the bar with `query` and `options`, as opening a Search result does (spec §8).
    pub(crate) fn show_with(
        &mut self,
        mode: FindBarMode,
        query: &str,
        options: MatchOptions,
        colors: Palette,
    ) -> PendingText {
        self.options = options;
        let _ = self.show(mode, None, colors);
        PendingText {
            edit: self.query_edit,
            text: query.to_owned(),
        }
    }

    pub(crate) fn options(&self) -> MatchOptions {
        self.options
    }

    pub(crate) fn toggle_option(&mut self, option: SearchOption) {
        self.options = self.options.toggled(option);
        self.no_match.set(false);
        unsafe {
            InvalidateRect(self.panel, std::ptr::null(), 0);
        }
    }

    #[cfg_attr(not(test), allow(dead_code, reason = "read by the window tests"))]
    pub(crate) fn no_match(&self) -> bool {
        self.no_match.get()
    }

    /// Shows or clears the no-match outline on the query field.
    pub(crate) fn set_no_match(&self, no_match: bool) {
        if self.no_match.replace(no_match) != no_match {
            unsafe {
                InvalidateRect(self.panel, std::ptr::null(), 0);
            }
        }
    }

    /// The three toggles, in bar coordinates, in `SearchOption::ALL` order.
    pub(crate) fn toggle_rects(&self) -> [RECT; 3] {
        let (layout, dpi) = self.current_layout();
        option_toggles::toggle_rects(layout.query.field, dpi)
    }

    /// Whether the pointer's first move over the bar should make the toggles' tooltip.
    pub(crate) fn wants_tooltip(&self) -> bool {
        self.tooltip.get().is_none() && !self.tooltip_failed.get()
    }

    /// Keeps the tooltip made for the bar. `None` means it couldn't be made, and that is not
    /// tried again.
    pub(crate) fn set_tooltip(&self, tooltip: Option<Tooltip>) {
        self.tooltip.set(tooltip);
        self.tooltip_failed.set(tooltip.is_none());
    }

    /// The tooltip and each toggle's tool, to set with nothing of the `App` borrowed.
    pub(crate) fn toggle_tools(&self) -> Option<(Tooltip, [(RECT, &'static str); 3])> {
        let tooltip = self.tooltip.get()?;
        let rects = self.toggle_rects();
        Some((
            tooltip,
            std::array::from_fn(|index| {
                (rects[index], option_toggles::tooltip(SearchOption::ALL[index]))
            }),
        ))
    }
```

4. Replace `current_layout` with a version that also returns the DPI:

```rust
    fn current_layout(&self) -> (BarLayout, u32) {
        let mut client = RECT::default();
        unsafe {
            GetClientRect(self.panel, &mut client);
        }
        let dpi = unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForWindow(self.panel) }.max(96);
        (bar_layout(client.right, dpi, 0, self.mode), dpi)
    }
```

5. Replace `pointer` with:

```rust
    /// `WM_MOUSEMOVE`, `WM_MOUSELEAVE` and `WM_LBUTTONUP` on the bar: tracks hovering over the
    /// close button and the toggles, and reports a click released on one of them.
    pub(crate) fn pointer(&self, message: u32, lparam: LPARAM) -> Option<BarClick> {
        let (layout, dpi) = self.current_layout();
        let point = POINT {
            x: (lparam as u32 & 0xffff) as u16 as i16 as i32,
            y: ((lparam as u32 >> 16) & 0xffff) as u16 as i16 as i32,
        };
        let leaving = message == WM_MOUSELEAVE;
        let over_close = !leaving
            && point.x >= layout.close.left
            && point.x < layout.close.right
            && point.y >= layout.close.top
            && point.y < layout.close.bottom;
        let over_toggle = if leaving {
            None
        } else {
            option_toggles::hit(&option_toggles::toggle_rects(layout.query.field, dpi), point)
        };
        if message == WM_MOUSEMOVE {
            let mut track = TRACKMOUSEEVENT {
                cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: self.panel,
                dwHoverTime: 0,
            };
            unsafe {
                TrackMouseEvent(&mut track);
            }
        }
        if self.close_hovered.replace(over_close) != over_close {
            unsafe {
                InvalidateRect(self.panel, &layout.close, 0);
            }
        }
        if self.hovered_toggle.replace(over_toggle) != over_toggle {
            unsafe {
                InvalidateRect(self.panel, &layout.query.field, 0);
            }
        }
        if message != WM_LBUTTONUP {
            return None;
        }
        if over_close {
            Some(BarClick::Close)
        } else {
            over_toggle.map(BarClick::Toggle)
        }
    }
```

6. In `layout`, after the `SetWindowPos`/`InvalidateRect` block, keep the tooltip's tools on the toggles:

```rust
        if let Some((tooltip, tools)) = self.toggle_tools() {
            for (index, (rect, text)) in tools.into_iter().enumerate() {
                tooltip.set_tool(index, rect, text);
            }
        }
```

7. Change `paint_panel`'s signature to `pub(crate) fn paint_panel(&self, panel: HWND, glyph_font: HFONT, text_font: HFONT)`. Its `let BarLayout { query, replace, close } = bar_layout(client.right, dpi, 0, self.mode);` stays as it is: it calls `bar_layout` directly, not `current_layout`. Then:
   - Replace the `outline` computation with:

```rust
                let outline = if edit == self.query_edit && self.no_match.get() {
                    // A miss is outlined in the Search view's error color.
                    colors.error_foreground
                } else if focus == edit {
                    colors.selection_background
                } else {
                    colors.pressed_background
                };
```

   - After the `for (layout, edit) in …` loop, paint the toggles:

```rust
            if !text_font.is_null() {
                option_toggles::paint(
                    dc,
                    &option_toggles::toggle_rects(query.field, dpi),
                    self.options,
                    self.hovered_toggle.get(),
                    &colors,
                    text_font,
                );
            }
```

8. Remove `#[cfg(test)]` from `panel_hwnd`. It is now used by `main_window::toggle_find_option`, and by Task 8's events.

9. In `impl Drop for FindBar`, add before the brush's `DeleteObject`:

```rust
            // The tooltip is owned by the main window, not by the bar.
            if let Some(tooltip) = self.tooltip.get() {
                tooltip.destroy();
            }
```

10. In `find_field_proc`, directly after the `WM_CHAR` Enter/Escape swallow, add:

```rust
    // Alt+C, Alt+W and Alt+R flip the options while a field has the focus (spec §8). The key
    // down flips; its WM_SYSCHAR is swallowed, so the menu band never sees the letter.
    if message == WM_SYSKEYDOWN
        && let Some(option) = option_toggles::alt_key(wparam as u32)
    {
        super::main_window::toggle_find_option(hook.parent, option);
        return 0;
    }
    if message == WM_SYSCHAR
        && u8::try_from(wparam)
            .ok()
            .and_then(|c| option_toggles::alt_key(u32::from(c.to_ascii_uppercase())))
            .is_some()
    {
        return 0;
    }
```

- [ ] **Step 7: Add F3 and Shift+F3**

1. `src/window/commands.rs`:
   - After `SearchToggleRegex = 187,`, add `FindNext = 188,` and `FindPrevious = 189,`.
   - `COMMANDS` grows by 2. Add `CommandId::FindNext,` and `CommandId::FindPrevious,` at its end.
   - Leave `needs_document` as it is: both need a document.
   - Add the test:

```rust
    #[test]
    fn find_next_and_previous_have_stable_values_and_need_a_document() {
        assert_eq!(CommandId::try_from(188), Ok(CommandId::FindNext));
        assert_eq!(CommandId::try_from(189), Ok(CommandId::FindPrevious));
        assert!(CommandId::FindNext.needs_document());
        assert!(!CommandId::FindNext.is_sidebar());
    }
```

2. `src/window/menus.rs`:
   - Add `VK_F3` to the `KeyboardAndMouse` import.
   - `accelerator_specs()` grows by 2. Change its return type (`[AcceleratorSpec; 49]` after Task 6) to `[AcceleratorSpec; 51]`, and after `accelerator(FCONTROL, b'H', CommandId::Replace),` add:

```rust
        virtual_key(0, VK_F3, CommandId::FindNext),
        virtual_key(FSHIFT, VK_F3, CommandId::FindPrevious),
```

   - Replace the Search menu with:

```rust
            let search = create_popup(&[
                MenuEntry::command("&Find\tCtrl+F", CommandId::Find),
                MenuEntry::command("Find &next\tF3", CommandId::FindNext),
                MenuEntry::command("Find pre&vious\tShift+F3", CommandId::FindPrevious),
                MenuEntry::command("&Replace\tCtrl+H", CommandId::Replace),
            ])?;
```

   - In `shortcut_and_menu_commands_share_command_ids`, change `assert_eq!(specs.len(), 49);` to `assert_eq!(specs.len(), 51);`. The table grew by 2; if another task changed the total, add 2 to it.
   - In `tab_zoom_and_direction_shortcuts_are_bound`, add `VK_F3` to its `KeyboardAndMouse` `use` and append:

```rust
        // Break caught: F3 unbound, so opening a Search result can't step on (spec §8).
        assert_eq!(bound(0, VK_F3), Some(CommandId::FindNext));
        assert_eq!(bound(FSHIFT, VK_F3), Some(CommandId::FindPrevious));
```

3. `src/window/command_palette.rs`:
   - `ENTRIES` grows by 2 (66 after Task 6, so 68). After `entry("Search: Find", CommandId::Find),` add:

```rust
    entry("Search: Find next", CommandId::FindNext),
    entry("Search: Find previous", CommandId::FindPrevious),
```

   - In `shortcut_text`, spell function keys. `VK_F3` is `0x72`, which the current fallback would print as `r`. Replace the `use` line and the `match` with:

```rust
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        VK_F1, VK_F24, VK_OEM_MINUS, VK_OEM_PLUS,
    };
    match spec.key {
        VK_TAB => text.push_str("Tab"),
        VK_OEM_PLUS => text.push('+'),
        VK_OEM_MINUS => text.push('-'),
        key @ VK_F1..=VK_F24 => text.push_str(&format!("F{}", key - VK_F1 + 1)),
        key => text.push(char::from_u32(u32::from(key))?),
    }
```

   - In `shortcuts_are_spelled_from_the_accelerator_table`, add:

```rust
        assert_eq!(shortcut_text(CommandId::FindNext).as_deref(), Some("F3"));
        assert_eq!(
            shortcut_text(CommandId::FindPrevious).as_deref(),
            Some("Shift+F3")
        );
```

Run: `cargo test --lib -- window::commands window::menus window::command_palette --test-threads=1`
Expected: all pass, `the_search_option_rows_are_listed_without_shortcuts` included.

- [ ] **Step 8: Wire the find bar in `src/window/main_window.rs`**

1. `paint_panel`: change `bar.paint_panel(panel, glyph_font);` to `bar.paint_panel(panel, glyph_font, text_font);`.

2. Replace `open_find_bar` with the following, and add `ensure_find_bar` after it:

```rust
fn open_find_bar(hwnd: HWND, mode: find_bar::FindBarMode) {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    // The two bars share the band above the editor; only one shows at a time.
    crate::window::library_host::close_name_box(hwnd);
    if !identity.is_live_for(hwnd) || !ensure_find_bar(hwnd) {
        return;
    }
    // With regex on, a selection is escaped so it matches only itself.
    let regex = unsafe { app_ptr(hwnd) }
        .and_then(|app| Some(unsafe { app.as_ref() }.find_bar.as_ref()?.options().regex))
        .unwrap_or(false);
    let prefill = single_line_selection(hwnd).map(|text| {
        if regex {
            find_bar::escape_pattern(&text)
        } else {
            text
        }
    });
    let colors = title_chrome(hwnd).0;
    let pending = unsafe { app_ptr(hwnd) }.and_then(|mut app| {
        let bar = unsafe { app.as_mut() }.find_bar.as_mut()?;
        Some(bar.show(mode, prefill.as_deref(), colors))
    });
    let Some(pending) = pending else {
        return;
    };
    // Applied with nothing borrowed: the field's EN_CHANGE borrows the bar again.
    if let Some(pending) = pending {
        pending.apply();
    }
    if !identity.is_live_for(hwnd) {
        return;
    }
    layout_editor_and_find_bar(hwnd);
    if let Some(app) = unsafe { app_ptr(hwnd) }
        && let Some(bar) = unsafe { app.as_ref() }.find_bar.as_ref()
    {
        bar.focus_query();
    }
}

/// Makes the find bar the first time it's needed, with nothing of the `App` borrowed, because
/// creating its controls sends messages. Returns false when there's no bar and none could be
/// made.
fn ensure_find_bar(hwnd: HWND) -> bool {
    let Some(exists) = unsafe { app_ptr(hwnd) }.map(|app| unsafe { app.as_ref() }.find_bar.is_some())
    else {
        return false;
    };
    if exists {
        return true;
    }
    let Ok(bar) = find_bar::FindBar::create(hwnd) else {
        return false;
    };
    unsafe { app_ptr(hwnd) }.is_some_and(|mut app| {
        unsafe { app.as_mut() }.find_bar = Some(bar);
        true
    })
}
```

3. Replace `panel_pointer` with the following, and add `ensure_find_tooltip` and `toggle_find_option` after it:

```rust
/// Mouse input on a panel: hovering over and clicking the find bar's close button and toggles.
pub(crate) fn panel_pointer(hwnd: HWND, panel: HWND, message: u32, lparam: LPARAM) {
    if message == windows_sys::Win32::UI::WindowsAndMessaging::WM_MOUSEMOVE {
        ensure_find_tooltip(hwnd, panel);
    }
    let click = unsafe { app_ptr(hwnd) }.and_then(|app| {
        unsafe { app.as_ref() }
            .find_bar
            .as_ref()
            .filter(|bar| bar.owns(panel))
            .and_then(|bar| bar.pointer(message, lparam))
    });
    match click {
        Some(find_bar::BarClick::Close) => close_find_bar(hwnd),
        Some(find_bar::BarClick::Toggle(option)) => toggle_find_option(hwnd, option),
        None => {}
    }
}

/// The first pointer move over the find bar makes its toggles' tooltip. Nothing before that
/// needs it.
fn ensure_find_tooltip(hwnd: HWND, panel: HWND) {
    let wanted = unsafe { app_ptr(hwnd) }.is_some_and(|app| {
        unsafe { app.as_ref() }
            .find_bar
            .as_ref()
            .is_some_and(|bar| bar.owns(panel) && bar.wants_tooltip())
    });
    if !wanted {
        return;
    }
    // Made with nothing of the App borrowed: creating the control sends messages.
    let tooltip = crate::window::tooltip::Tooltip::create(panel);
    let tools = unsafe { app_ptr(hwnd) }.and_then(|app| {
        let bar = unsafe { app.as_ref() }.find_bar.as_ref()?;
        bar.set_tooltip(tooltip);
        Some(bar.toggle_tools())
    });
    match (tooltip, tools) {
        (Some(_), Some(Some((tooltip, tools)))) => {
            for (index, (rect, text)) in tools.into_iter().enumerate() {
                tooltip.set_tool(index, rect, text);
            }
        }
        (Some(tooltip), None) => tooltip.destroy(),
        _ => {}
    }
}

/// Flips a find bar option: a toggle click, or Alt+C, Alt+W or Alt+R in its fields.
pub(crate) fn toggle_find_option(hwnd: HWND, option: crate::search::SearchOption) {
    if let Some(mut app) = unsafe { app_ptr(hwnd) }
        && let Some(bar) = unsafe { app.as_mut() }.find_bar.as_mut()
    {
        bar.toggle_option(option);
    }
}
```

4. Make `find_bar_owns` `pub(crate)`.

5. In the window procedure, add this arm directly after the palette's `EN_CHANGE` arm (`… command_palette_owns(hwnd, lparam as HWND) => { refilter_command_palette(hwnd); 0 }`):

```rust
        // Typing in the find bar clears its no-match outline until the next search.
        WM_COMMAND
            if lparam != 0
                && ((wparam >> 16) & 0xffff) as u32 == EN_CHANGE
                && find_bar_owns(hwnd, lparam as HWND) =>
        {
            set_find_no_match(hwnd, false);
            0
        }
```

6. Replace `navigate_to_match`, `replace_current` and `replace_all_matches` with the following, and add `select_match` and `set_find_no_match`:

```rust
fn navigate_to_match(hwnd: HWND, backward: bool) {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    let Some((editor, query, options)) = (unsafe { app_ptr(hwnd) }).and_then(|app| {
        let app = unsafe { app.as_ref() };
        let editor = app.editor.clone()?;
        let bar = app.find_bar.as_ref()?;
        Some((editor, bar.query_text(), bar.options()))
    }) else {
        return;
    };
    if query.is_empty() {
        return;
    }
    let Ok(selection) = editor.selection() else {
        return;
    };
    let (origin, direction) = if backward {
        (selection.start, find_bar::SearchDirection::Backward)
    } else {
        (selection.end, find_bar::SearchDirection::Forward)
    };
    select_match(hwnd, &identity, &editor, &query, options, origin, direction);
}

/// Selects the next match of `query` under `options` from `origin`, wrapping once, and scrolls
/// it into view. When there is none, the selection stays and the find bar shows its no-match
/// state. A pattern Scintilla can't compile counts as no match.
fn select_match(
    hwnd: HWND,
    identity: &WindowIdentity,
    editor: &Editor,
    query: &str,
    options: crate::search::MatchOptions,
    origin: usize,
    direction: find_bar::SearchDirection,
) {
    let Ok(doc_len) = editor.length() else {
        return;
    };
    let pattern = find_bar::scintilla_query(query, options);
    let mut state = find_bar::SearchState::new(&pattern, direction, origin);
    let found = state
        .next_editor_match(editor, find_bar::search_flags(options), doc_len)
        .ok()
        .flatten();
    if !identity.is_live_for(hwnd) {
        return;
    }
    if let Some(found) = found.clone() {
        let _ = editor.set_selection(found);
        editor.scroll_caret_into_view();
    }
    set_find_no_match(hwnd, found.is_none());
}

fn set_find_no_match(hwnd: HWND, no_match: bool) {
    if let Some(app) = unsafe { app_ptr(hwnd) }
        && let Some(bar) = unsafe { app.as_ref() }.find_bar.as_ref()
    {
        bar.set_no_match(no_match);
    }
}

pub(crate) fn replace_current(hwnd: HWND) {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    let Some((editor, query, replacement, options)) = (unsafe { app_ptr(hwnd) }).and_then(|app| {
        let app = unsafe { app.as_ref() };
        let editor = app.editor.clone()?;
        let bar = app.find_bar.as_ref()?;
        Some((editor, bar.query_text(), bar.replace_text(), bar.options()))
    }) else {
        return;
    };
    if query.is_empty() {
        return;
    }
    // Only replace when the selection is exactly a match under the options (a case-insensitive
    // "CAT" for "cat", or a regex's match); otherwise this Enter just moves to the next match,
    // as in a bare Find field. The replacement is literal text, also in regex mode.
    let pattern = find_bar::scintilla_query(&query, options);
    if let Ok(selection) = editor.selection()
        && !selection.is_empty()
        && editor
            .search_in_target(&pattern, selection.clone(), find_bar::search_flags(options))
            .ok()
            .flatten()
            == Some(selection.clone())
    {
        let _ = editor.replace_target(selection, &replacement);
        if !identity.is_live_for(hwnd) {
            return;
        }
    }
    find_next(hwnd);
}

pub(crate) fn replace_all_matches(hwnd: HWND) {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    let Some((editor, query, replacement, options)) = (unsafe { app_ptr(hwnd) }).and_then(|app| {
        let app = unsafe { app.as_ref() };
        let editor = app.editor.clone()?;
        let bar = app.find_bar.as_ref()?;
        Some((editor, bar.query_text(), bar.replace_text(), bar.options()))
    }) else {
        return;
    };
    if query.is_empty() {
        return;
    }
    let pattern = find_bar::scintilla_query(&query, options);
    let replaced = editor
        .replace_all(&pattern, &replacement, find_bar::search_flags(options))
        .unwrap_or(0);
    if identity.is_live_for(hwnd) {
        editor.scroll_caret_into_view();
        set_find_no_match(hwnd, replaced == 0);
    }
}
```

7. F3 in `execute_command_with_note`: after the `CommandId::Find` arm, add:

```rust
        CommandId::FindNext => find_again(hwnd, false),
        CommandId::FindPrevious => find_again(hwnd, true),
```

   and after `find_previous`, add:

```rust
/// F3 and Shift+F3 step through the find bar's query, even while the bar is closed. With no
/// query yet, they open the bar.
fn find_again(hwnd: HWND, backward: bool) {
    let has_query = unsafe { app_ptr(hwnd) }.is_some_and(|app| {
        unsafe { app.as_ref() }
            .find_bar
            .as_ref()
            .is_some_and(|bar| !bar.query_text().is_empty())
    });
    if has_query {
        navigate_to_match(hwnd, backward);
    } else {
        open_find_bar(hwnd, find_bar::FindBarMode::Find);
    }
}
```

8. Add `open_search_result` after `open_note`:

```rust
/// Opens a Search result (spec §8). The note opens as `open_note` opens it. The find bar then
/// opens in Find mode with the Search query and options, and selects the first match from the
/// start of the note. The find bar searches the live text: a phrase gone since the search
/// leaves the note open and the bar in its no-match state. `focus_editor` then moves the focus
/// to the editor, so F3 and Shift+F3 step on from the selected match. A note that can't be
/// opened (moved or deleted since the search) gets a notice, and the search runs again.
pub(crate) fn open_search_result(
    hwnd: HWND,
    relative: &std::path::Path,
    mode: OpenMode,
    focus_editor: bool,
) {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    let Some(folder) = crate::window::library_host::folder(hwnd) else {
        return;
    };
    let path = folder.join(relative);
    // Read before opening, which can move the focus but never changes the query.
    let search = crate::window::search_view::current_query(hwnd);
    if let Err(error) = open_note(hwnd, &path, mode, false) {
        push_notice(
            hwnd,
            format!("FastPad could not open {}: {error}", path.display()),
        );
        crate::window::text_search_host::run_now(hwnd);
        return;
    }
    if !identity.is_live_for(hwnd) {
        return;
    }
    if let Some((query, options)) = search.filter(|(query, _)| !query.is_empty()) {
        seed_find_bar(hwnd, &identity, &query, options);
    }
    if focus_editor && identity.is_live_for(hwnd) {
        focus_content(hwnd);
    }
}

/// Shows the find bar with `query` and `options` and selects the first match from position 0.
fn seed_find_bar(
    hwnd: HWND,
    identity: &WindowIdentity,
    query: &str,
    options: crate::search::MatchOptions,
) {
    crate::window::library_host::close_name_box(hwnd);
    if !identity.is_live_for(hwnd) || !ensure_find_bar(hwnd) {
        return;
    }
    let colors = title_chrome(hwnd).0;
    let pending = unsafe { app_ptr(hwnd) }.and_then(|mut app| {
        let bar = unsafe { app.as_mut() }.find_bar.as_mut()?;
        Some(bar.show_with(find_bar::FindBarMode::Find, query, options, colors))
    });
    let Some(pending) = pending else {
        return;
    };
    // Applied with nothing borrowed: the field's EN_CHANGE borrows the bar again.
    pending.apply();
    if !identity.is_live_for(hwnd) {
        return;
    }
    layout_editor_and_find_bar(hwnd);
    let Some(editor) = unsafe { app_ptr(hwnd) }.and_then(|app| unsafe { app.as_ref() }.editor.clone())
    else {
        return;
    };
    let _ = editor.set_selection(0..0);
    select_match(
        hwnd,
        identity,
        &editor,
        query,
        options,
        0,
        find_bar::SearchDirection::Forward,
    );
}
```

9. `src/window/search_view.rs`: `open_selected` (Enter, `focus_editor: false`) and Task 5's click path in `handle` (`focus_editor: true`) both call `open_result`. Replace `open_result`'s body with:

```rust
fn open_result(hwnd: HWND, relative: &Path, mode: OpenMode, focus_editor: bool) {
    super::main_window::open_search_result(hwnd, relative, mode, focus_editor);
}
```

- [ ] **Step 9: Write the window tests** in `src/window/main_window.rs`'s `tests` module

```rust
    fn set_find_query(hwnd: HWND, text: &str) {
        let edit = app_mut(hwnd).find_bar.as_ref().unwrap().query_hwnd();
        let wide = crate::platform::wide_null(text);
        // Sends EN_CHANGE, handled with nothing of the App borrowed here.
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::SetWindowTextW(edit, wide.as_ptr());
        }
    }

    #[test]
    fn the_find_bar_passes_its_options_to_scintilla_and_a_bad_regex_is_a_miss() {
        // Break caught: toggles that change nothing, whole word matching inside foo_bar, the
        // basic regex dialect instead of C++11 (no `{2}`), or an invalid pattern reported as an
        // error or leaving no trace.
        use crate::search::SearchOption;
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        editor
            .populate_clean("Foo foo foobar foo_bar foo. a1 b22")
            .unwrap();
        execute_command(window.hwnd, CommandId::Find);
        let bar = || app_mut(window.hwnd).find_bar.as_ref().unwrap();

        set_find_query(window.hwnd, "foo");
        super::toggle_find_option(window.hwnd, SearchOption::Case);
        editor.set_selection(0..0).unwrap();
        super::find_next(window.hwnd);
        assert_eq!(editor.selection().unwrap(), 4..7, "match case skips Foo");

        super::toggle_find_option(window.hwnd, SearchOption::Case);
        super::toggle_find_option(window.hwnd, SearchOption::WholeWord);
        editor.set_selection(7..7).unwrap();
        super::find_next(window.hwnd);
        assert_eq!(
            editor.selection().unwrap(),
            23..26,
            "whole word skips foobar and foo_bar"
        );

        // `{2}` exists only in Scintilla's C++11 (ECMAScript) regex. If this fails, the DLL was
        // built with NO_CXX11_REGEX (see the Task 7 findings).
        super::toggle_find_option(window.hwnd, SearchOption::WholeWord);
        super::toggle_find_option(window.hwnd, SearchOption::Regex);
        set_find_query(window.hwnd, r"b\d{2}");
        editor.set_selection(0..0).unwrap();
        super::find_next(window.hwnd);
        assert_eq!(editor.selection().unwrap(), 31..34);
        assert!(!bar().no_match());

        set_find_query(window.hwnd, "(");
        super::find_next(window.hwnd);
        assert_eq!(editor.selection().unwrap(), 31..34, "the selection stays");
        assert!(bar().no_match());
        assert!(notices(window.hwnd).is_empty());
        set_find_query(window.hwnd, "a1");
        assert!(!bar().no_match(), "typing clears the no-match state");
    }

    #[test]
    fn alt_keys_and_clicks_flip_the_find_bar_toggles() {
        // Break caught: Alt+C opening a menu instead of flipping match case, a toggle click that
        // does nothing, or the letter reaching the menu band after the flip.
        use crate::search::MatchOptions;
        use windows_sys::Win32::UI::WindowsAndMessaging::{WM_LBUTTONUP, WM_SYSCHAR, WM_SYSKEYDOWN};
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        execute_command(window.hwnd, CommandId::Find);
        let bar = || app_mut(window.hwnd).find_bar.as_ref().unwrap();
        let (query, panel) = (bar().query_hwnd(), bar().panel_hwnd());
        let alt = 1 << 29;

        unsafe { SendMessageW(query, WM_SYSKEYDOWN, usize::from(b'C'), alt) };
        assert!(bar().options().case);
        unsafe { SendMessageW(query, WM_SYSCHAR, usize::from(b'c'), alt) };
        assert_eq!(app_mut(window.hwnd).menu_mode, None);
        unsafe {
            SendMessageW(query, WM_SYSKEYDOWN, usize::from(b'W'), alt);
            SendMessageW(query, WM_SYSKEYDOWN, usize::from(b'R'), alt);
        }
        assert_eq!(
            bar().options(),
            MatchOptions {
                case: true,
                whole_word: true,
                regex: true
            }
        );

        let rect = bar().toggle_rects()[0];
        let x = (rect.left + rect.right) / 2;
        let y = (rect.top + rect.bottom) / 2;
        let point = ((y as u32) << 16 | (x as u32 & 0xffff)) as super::LPARAM;
        super::panel_pointer(window.hwnd, panel, WM_LBUTTONUP, point);
        assert!(!bar().options().case, "a click on Aa turns match case off");
    }

    #[test]
    fn replace_current_replaces_a_selection_that_matches_under_the_options() {
        // Break caught: Enter in Replace comparing the selection to the query byte for byte, so
        // a case-insensitive "CAT" is skipped instead of replaced.
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        editor.populate_clean("CAT cat").unwrap();
        execute_command(window.hwnd, CommandId::Replace);
        set_find_query(window.hwnd, "cat");
        let replace = app_mut(window.hwnd).find_bar.as_ref().unwrap().replace_hwnd();
        let dog = crate::platform::wide_null("dog");
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::SetWindowTextW(replace, dog.as_ptr());
        }
        editor.set_selection(0..3).unwrap();

        super::replace_current(window.hwnd);

        assert_eq!(editor.text().unwrap(), "dog cat");
        assert_eq!(editor.selection().unwrap(), 4..7);
    }

    #[test]
    fn opening_a_result_seeds_the_find_bar_with_search_options_and_f3_steps_on() {
        // Break caught: the find bar keeping its own options (so match case is lost), the first
        // match not selected, or F3 and Shift+F3 not reaching the next and previous matches.
        use crate::search::{MatchOptions, SearchOption};
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("result-seed");
        scratch.note("a.md", "beta Beta beta Beta");
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        scratch.install(window.hwnd);
        crate::window::side_panel::show_view(
            window.hwnd,
            crate::config::SidebarView::Search,
            true,
        );
        crate::window::search_view::toggle_option(window.hwnd, SearchOption::Case);
        type_into_search(window.hwnd, "Beta");
        pump_until(window.hwnd, || {
            crate::window::search_view::shown_results(window.hwnd).len() == 1
        });

        crate::window::search_view::open_selected(window.hwnd, super::OpenMode::Preview, true);

        assert_eq!(editor.text().unwrap(), "beta Beta beta Beta");
        assert_eq!(editor.selection().unwrap(), 5..9);
        let bar = app_mut(window.hwnd).find_bar.as_ref().unwrap();
        assert!(bar.is_visible());
        assert_eq!(bar.query_text(), "Beta");
        assert_eq!(
            bar.options(),
            MatchOptions {
                case: true,
                ..MatchOptions::default()
            }
        );
        assert!(!bar.no_match());
        execute_command(window.hwnd, CommandId::FindNext);
        assert_eq!(editor.selection().unwrap(), 15..19);
        execute_command(window.hwnd, CommandId::FindPrevious);
        assert_eq!(editor.selection().unwrap(), 5..9);
    }

    #[test]
    fn opening_a_result_whose_text_changed_shows_no_match() {
        // Break caught (review focus 5): a stale result opening nothing, panicking on its
        // snippet, selecting text that no longer matches, or reporting the miss as an error.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("result-stale");
        let note = scratch.note("a.md", "alpha beta");
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        scratch.install(window.hwnd);
        crate::window::side_panel::show_view(
            window.hwnd,
            crate::config::SidebarView::Search,
            true,
        );
        type_into_search(window.hwnd, "beta");
        pump_until(window.hwnd, || {
            crate::window::search_view::shown_results(window.hwnd).len() == 1
        });
        std::fs::write(&note, "alpha gamma").unwrap();
        let before = notices(window.hwnd).len();

        crate::window::search_view::open_selected(window.hwnd, super::OpenMode::Preview, false);

        assert_eq!(editor.text().unwrap(), "alpha gamma");
        assert_eq!(editor.selection().unwrap(), 0..0);
        let bar = app_mut(window.hwnd).find_bar.as_ref().unwrap();
        assert!(bar.is_visible());
        assert_eq!(bar.query_text(), "beta");
        assert!(bar.no_match());
        assert_eq!(notices(window.hwnd).len(), before);
    }
```

`SendMessageW` is already imported in the tests module. If it isn't there, spell it as `windows_sys::Win32::UI::WindowsAndMessaging::SendMessageW`.

Run: `cargo test --lib -- window::main_window::tests::the_find_bar window::main_window::tests::alt_keys window::main_window::tests::replace_current window::main_window::tests::opening_a_result window::main_window::tests::the_find_bar_panel window::main_window::tests::ctrl_shift_f --test-threads=1`
Expected: all pass. That includes the existing `the_find_bar_panel_reserves_its_band_above_the_editor_and_follows_theme_changes`, whose close-button click still goes through `panel_pointer`.

Then the source-linked find-bar tests, which include `src/lib.rs`:

Run: `cargo build`, then `cargo test --test editing -- --test-threads=1`
Expected: all pass. The default options are all off, which gives flags 0, as before.

- [ ] **Step 10: Lint, format, commit**

Run: `cargo clippy --all-targets -- -D warnings`, then `cargo fmt --all -- --check`
Expected: clean.

```
git add src/editor/scintilla_constants.rs tools/generate-scintilla-constants.ps1 src/editor/scintilla.rs src/window/find_bar.rs src/window/main_window.rs src/window/search_view.rs src/window/commands.rs src/window/menus.rs src/window/command_palette.rs
git commit -m "feat(find): match case, whole word and regex toggles in the find bar, F3 and Shift+F3, a no-match state, and opening a Search result selects its first match"
```

---

### Task 8: Accessibility

**Files:**
- Modify:
  - `src/window/sidebar_accessibility.rs`
  - `src/window/search_view.rs`
  - `src/window/option_toggles.rs` (removes `label`'s `expect(dead_code)`)
  - `src/window/find_bar.rs`
  - `src/window/panel.rs`
  - `src/window/main_window.rs` (`toggle_find_option`, tests)
  - `src/window/text_search_host.rs` (one call in `batch_arrived`)

**Interfaces:**
- Consumes these from Tasks 4 and 5, with their exact signatures:
  - `SearchView::notice(&self) -> Option<&'static str>` (Task 4): the notice painted in the summary line's place ("Open a notebook to search it.", "Loading…", the load failure). While it shows, `paint` draws no summary and no status line.
  - `SearchView::summary(&self) -> Option<(String, bool)>` (Task 4): the summary line or the pattern error, and whether it is an error. `None` when the line is empty.
  - `SearchView::status_line(&self) -> Option<String>` (Task 4): `None` while the line is hidden.
  - `SearchView::summary_rect(client: RECT, dpi: u32) -> RECT` and `SearchView::status_rect(client: RECT, dpi: u32) -> RECT` (Task 5), associated functions.
  - `SearchView::field_rect(client, dpi)` (existing) and the toggles, which Task 5 paints whenever the box exists. The box is visible whenever the Search view shows, so the toggles show exactly when the box shows.
- Also consumes:
  - From Task 4: `SearchView.{options, results, search, edit, placeholder}`, `row_under`, `text_search_host::batch_arrived`.
  - From Task 7: `FindBar::{toggle_rects, options, panel_hwnd}`.
  - From Task 5: `option_toggles::{toggle_rects, label}`.
- Adds to `sidebar_accessibility.rs`'s item model, as the contract allows:
  - `AccessibleItem.window: HWND` (a native control shown as a child; null otherwise).
  - `STATE_CHECKED`.
  - `check_item`, `field_item`, `text_item`.
  - `get_accChild` returns a window child's own object.
  - `default_action` covers check buttons and text.
- Adds `find_bar::FIND_BAR_ACCESSIBLE` (an `AccessibleSource`), `find_bar::toggle_child(option) -> usize` and `search_view::result_name(&TextHit) -> String`.

**The Search view's children, in order:**
1. The search box (a field child, backed by the real `Edit`).
2. The three toggles.
3. The summary line, while it shows.
4. The status line, while it shows.
5. One list item per result.

The status line comes before the results here, though it is painted after them. That keeps its child ID fixed while results stream in, so its name-change events always point at it.

- [ ] **Step 1: Write the failing item-model tests** in `src/window/sidebar_accessibility.rs`'s `tests` module

```rust
    #[test]
    fn toggles_are_check_buttons_fields_are_text_and_lines_are_static_text() {
        // Break caught: a toggle read as a push button with no checked state, or a status line
        // offering "Open" as its default action.
        let on = check_item("Match case", true, ROW);
        assert_eq!(on.role, ROLE_SYSTEM_CHECKBUTTON);
        assert_ne!(on.state & STATE_CHECKED, 0);
        assert_eq!(default_action(&on), "Uncheck");
        let off = check_item("Match case", false, ROW);
        assert_eq!(off.state & STATE_CHECKED, 0);
        assert_eq!(default_action(&off), "Check");

        let field = field_item("Find", "abc".to_owned(), true, ROW, std::ptr::null_mut());
        assert_eq!(field.role, ROLE_SYSTEM_TEXT);
        assert_eq!(field.value, "abc");
        assert_ne!(field.state & STATE_FOCUSED, 0);
        assert_eq!(default_action(&field), "");

        let line = text_item("3 notes", ROW);
        assert_eq!(line.role, ROLE_SYSTEM_STATICTEXT);
        assert_eq!(default_action(&line), "");
        assert_eq!(default_action(&button_item("Close", false, false, ROW)), "Press");
    }
```

Run: `cargo test --lib -- window::sidebar_accessibility --test-threads=1`
Expected: compile errors, because the new names don't exist yet.

- [ ] **Step 2: Extend the item model** in `src/window/sidebar_accessibility.rs`

1. Imports:
   - Add `AccessibleObjectFromWindow, ROLE_SYSTEM_CHECKBUTTON, ROLE_SYSTEM_STATICTEXT, ROLE_SYSTEM_TEXT` to the `windows_sys::Win32::UI::Accessibility` import.
   - Add `OBJID_WINDOW` to the `WindowsAndMessaging` import.
   - Add `IID_IDISPATCH` if it isn't there yet. It is already imported from `accessibility`.

2. After `STATE_SELECTED`, add:

```rust
pub(crate) const STATE_CHECKED: u32 = 0x0000_0010;
pub(crate) const STATE_READONLY: u32 = 0x0000_0040;
```

3. In `AccessibleItem`, change `value`'s doc comment and add `window`:

```rust
    /// An outline item's level (0 for the root's children), as tree views report it, or a
    /// field's text. Empty otherwise.
    pub value: String,
    /// A native control shown as this child (the Search box, a find field). Its own MSAA object
    /// is the child's full object. Null for a painted child.
    pub window: HWND,
```

   Add `window: std::ptr::null_mut(),` to the three existing literals, in `button_item`, `list_item` and `tree_item`.

4. After `button_item`, add:

```rust
/// A painted option toggle: a check button, checked while its option is on (spec §10). The
/// toggles take no keyboard focus: Alt+C, Alt+W and Alt+R flip them from the field.
pub(crate) fn check_item(name: &str, checked: bool, rect: RECT) -> AccessibleItem {
    AccessibleItem {
        name: name.to_owned(),
        role: ROLE_SYSTEM_CHECKBUTTON,
        state: if checked { STATE_CHECKED } else { 0 },
        rect,
        value: String::new(),
        window: std::ptr::null_mut(),
    }
}

/// A native `Edit` shown as a child of a painted window, with its text as the value.
pub(crate) fn field_item(
    name: &str,
    text: String,
    focused: bool,
    rect: RECT,
    window: HWND,
) -> AccessibleItem {
    AccessibleItem {
        name: name.to_owned(),
        role: ROLE_SYSTEM_TEXT,
        state: STATE_FOCUSABLE | if focused { STATE_FOCUSED } else { 0 },
        rect,
        value: text,
        window,
    }
}

/// A painted line of text, such as the Search view's summary or status line.
pub(crate) fn text_item(name: &str, rect: RECT) -> AccessibleItem {
    AccessibleItem {
        name: name.to_owned(),
        role: ROLE_SYSTEM_STATICTEXT,
        state: STATE_READONLY,
        rect,
        value: String::new(),
        window: std::ptr::null_mut(),
    }
}
```

5. Replace `default_action` with:

```rust
pub(crate) fn default_action(item: &AccessibleItem) -> &'static str {
    match item.role {
        ROLE_SYSTEM_PUSHBUTTON => "Press",
        ROLE_SYSTEM_CHECKBUTTON if item.state & STATE_CHECKED != 0 => "Uncheck",
        ROLE_SYSTEM_CHECKBUTTON => "Check",
        ROLE_SYSTEM_TEXT | ROLE_SYSTEM_STATICTEXT => "",
        _ if item.state & STATE_EXPANDED != 0 => "Collapse",
        _ if item.state & STATE_COLLAPSED != 0 => "Expand",
        _ => "Open",
    }
}
```

6. Replace the `child` vtable function with:

```rust
unsafe extern "system" fn child(
    this: *mut c_void,
    child: RawVariant,
    output: *mut *mut c_void,
) -> HRESULT {
    if output.is_null() {
        return E_INVALIDARG;
    }
    unsafe { *output = std::ptr::null_mut() };
    match target(unsafe { provider(this) }, &child) {
        // A native control's own object, so a screen reader reads and edits it as the control
        // itself. Asked for from any thread, as clients do.
        Some(Some(found)) if !found.window.is_null() => {
            let result = unsafe {
                AccessibleObjectFromWindow(
                    found.window,
                    OBJID_WINDOW as u32,
                    &IID_IDISPATCH,
                    output,
                )
            };
            if result >= 0 && !unsafe { *output }.is_null() {
                S_OK
            } else {
                unsafe { *output = std::ptr::null_mut() };
                S_FALSE
            }
        }
        Some(Some(_)) => S_FALSE,
        _ => E_INVALIDARG,
    }
}
```

Run: `cargo test --lib -- window::sidebar_accessibility --test-threads=1`
Expected: all pass.

- [ ] **Step 3: The Search view's children** in `src/window/search_view.rs`

1. Imports:
   - Add `use crate::window::sidebar_accessibility::{self, AccessibleItem};`. Task 5 already imports `SearchOption`, `option_toggles` and `TextHit`.
   - Add `GetWindowLongPtrW, GWL_STYLE, WS_VISIBLE, EVENT_OBJECT_NAMECHANGE, EVENT_OBJECT_STATECHANGE` to the `WindowsAndMessaging` import.

   In `src/window/option_toggles.rs`, remove the `#[cfg_attr(not(test), expect(dead_code, …))]` attribute above `label`: this task's toggles are its first production callers.

2. Add before `impl SearchView`:

```rust
/// How often the summary and status lines may announce a change while a search runs (spec §10).
const ANNOUNCE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);

/// What screen readers last heard the summary and status lines say, and when.
#[derive(Debug, Default)]
pub(crate) struct Spoken {
    summary: String,
    status: String,
    at: Option<std::time::Instant>,
}

/// One of the Search view's MSAA children (see the module's accessibility impl).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SearchChild {
    Box,
    Toggle(SearchOption),
    Summary,
    Status,
    Result(usize),
}

/// A result's accessible name (spec §10): "<name>, <folder>: <snippet>". A note at the
/// notebook's root has no folder, so it reads "<name>: <snippet>".
pub(crate) fn result_name(hit: &TextHit) -> String {
    if hit.folder.is_empty() {
        format!("{}: {}", hit.name, hit.snippet.text)
    } else {
        format!("{}, {}: {}", hit.name, hit.folder, hit.snippet.text)
    }
}
```

3. Add `pub(crate) spoken: Spoken,` to `struct SearchView`, and `spoken: Spoken::default(),` to `SearchView::new`.

4. Add to `impl SearchView`:

```rust
    /// Whether the box shows. Read from its style: `IsWindowVisible` would also ask its
    /// ancestors, and a hidden test window hides everything.
    fn box_shown(&self) -> bool {
        self.edit.is_some_and(|edit| {
            (unsafe { GetWindowLongPtrW(edit, GWL_STYLE) }) as u32 & WS_VISIBLE != 0
        })
    }

    /// The summary line's text (or the notice painted in its place) and the status line's, as
    /// `paint` draws them: while a notice shows, there is no status line.
    fn shown_lines(&self) -> (Option<String>, Option<String>) {
        match self.notice() {
            Some(notice) => (Some(notice.to_owned()), None),
            None => (self.summary().map(|(text, _)| text), self.status_line()),
        }
    }

    /// The children before the results.
    fn head_children(&self) -> Vec<SearchChild> {
        let mut head = Vec::with_capacity(6);
        if self.box_shown() {
            head.push(SearchChild::Box);
            head.extend(SearchOption::ALL.map(SearchChild::Toggle));
        }
        let (summary, status) = self.shown_lines();
        if summary.is_some() {
            head.push(SearchChild::Summary);
        }
        if status.is_some() {
            head.push(SearchChild::Status);
        }
        head
    }

    fn child_at(&self, index: usize) -> Option<SearchChild> {
        let head = self.head_children();
        match head.get(index) {
            Some(child) => Some(*child),
            None => {
                let result = index - head.len();
                (result < self.results.len()).then_some(SearchChild::Result(result))
            }
        }
    }

    fn child_index(&self, child: SearchChild) -> Option<usize> {
        let head = self.head_children();
        match child {
            SearchChild::Result(index) => {
                (index < self.results.len()).then_some(head.len() + index)
            }
            _ => head.iter().position(|shown| *shown == child),
        }
    }
```

5. Replace the whole `impl crate::window::sidebar_accessibility::AccessibleView for SearchView` block with:

```rust
impl sidebar_accessibility::AccessibleView for SearchView {
    /// The box, the three toggles, the summary and status lines while they show, then the
    /// results. The status line comes before the results so its child ID stays put while
    /// results stream in.
    fn accessible_count(&self, _client: RECT, _dpi: u32) -> usize {
        self.head_children().len() + self.results.len()
    }

    fn accessible_item(
        &self,
        index: usize,
        client: RECT,
        dpi: u32,
        focused: bool,
    ) -> Option<AccessibleItem> {
        let field = SearchView::field_rect(client, dpi);
        Some(match self.child_at(index)? {
            SearchChild::Box => {
                let edit = self.edit?;
                sidebar_accessibility::field_item(
                    &self.placeholder,
                    window_text(edit),
                    unsafe { GetFocus() } == edit,
                    field,
                    edit,
                )
            }
            SearchChild::Toggle(option) => {
                let position = SearchOption::ALL.iter().position(|o| *o == option)?;
                sidebar_accessibility::check_item(
                    option_toggles::label(option),
                    self.options.get(option),
                    option_toggles::toggle_rects(field, dpi)[position],
                )
            }
            SearchChild::Summary => sidebar_accessibility::text_item(
                &self.shown_lines().0.unwrap_or_default(),
                SearchView::summary_rect(client, dpi),
            ),
            SearchChild::Status => sidebar_accessibility::text_item(
                &self.shown_lines().1.unwrap_or_default(),
                SearchView::status_rect(client, dpi),
            ),
            SearchChild::Result(row) => {
                let hit = self.results.get(row)?;
                let (rect, visible) =
                    sidebar_accessibility::row_rect(self.list_area(client, dpi), &self.list, row);
                sidebar_accessibility::list_item(
                    &result_name(hit),
                    self.list.selected == Some(row),
                    focused,
                    rect,
                    visible,
                )
            }
        })
    }

    fn accessible_hit(&self, point: POINT, client: RECT, dpi: u32) -> Option<usize> {
        let field = SearchView::field_rect(client, dpi);
        let (summary, status) = self.shown_lines();
        let child = if self.box_shown() && inside(field, point) {
            option_toggles::hit(&option_toggles::toggle_rects(field, dpi), point)
                .map_or(SearchChild::Box, SearchChild::Toggle)
        } else if summary.is_some() && inside(SearchView::summary_rect(client, dpi), point) {
            SearchChild::Summary
        } else if status.is_some() && inside(SearchView::status_rect(client, dpi), point) {
            SearchChild::Status
        } else {
            SearchChild::Result(self.row_under(point, client, dpi)?)
        };
        self.child_index(child)
    }

    fn accessible_current(&self, _client: RECT, _dpi: u32) -> Option<usize> {
        self.child_index(SearchChild::Result(self.list.selected?))
    }

    fn accessible_select(&mut self, index: usize, client: RECT, dpi: u32) {
        if let Some(SearchChild::Result(row)) = self.child_at(index) {
            let area = self.list_area(client, dpi);
            self.list.select(row, area.bottom - area.top);
        }
    }

    fn accessible_identity(&self, index: usize, _client: RECT, _dpi: u32) -> Option<u64> {
        Some(match self.child_at(index)? {
            SearchChild::Box => sidebar_accessibility::identity_of(&"search box"),
            SearchChild::Toggle(option) => {
                sidebar_accessibility::identity_of(&("toggle", option_toggles::label(option)))
            }
            SearchChild::Summary => sidebar_accessibility::identity_of(&"summary"),
            SearchChild::Status => sidebar_accessibility::identity_of(&"status"),
            SearchChild::Result(row) => {
                sidebar_accessibility::identity_of(&self.results.get(row)?.path)
            }
        })
    }

    fn accessible_generation(&self) -> u64 {
        self.order
    }
}
```

The panel's default action is a click on the child's center (`side_panel::accessible_activate`). On a toggle, that is the click Task 5 handles. On the box, it is a press on the field, which focuses the box (`field_pressed`).

- [ ] **Step 4: Raise the events** in `src/window/search_view.rs` and `src/window/text_search_host.rs`

1. Add to `search_view.rs`:

```rust
/// Raises `EVENT_OBJECT_NAMECHANGE` for the summary and status lines whose text changed, at
/// most once a second while a search runs (spec §10). `settled` (the search finished, failed or
/// can't run) always speaks, so the limit never swallows the final count. A line that went away
/// has no child left to name; the panel's reorder event covers it.
pub(crate) fn announce_lines(hwnd: HWND, settled: bool) {
    if side_panel::current_view(hwnd) != SidebarView::Search {
        return;
    }
    let Some((panel, changed)) = with_view(hwnd, |view| {
        let (summary, status) = view.shown_lines();
        let (summary, status) = (summary.unwrap_or_default(), status.unwrap_or_default());
        let summary_changed = summary != view.spoken.summary;
        let status_changed = status != view.spoken.status;
        if !summary_changed && !status_changed {
            return None;
        }
        let running = matches!(view.search, SearchState::Running(_));
        let recent = view
            .spoken
            .at
            .is_some_and(|at| at.elapsed() < ANNOUNCE_INTERVAL);
        if running && !settled && recent {
            return None;
        }
        let mut changed = Vec::with_capacity(2);
        if summary_changed && let Some(index) = view.child_index(SearchChild::Summary) {
            changed.push(index);
        }
        if status_changed && let Some(index) = view.child_index(SearchChild::Status) {
            changed.push(index);
        }
        view.spoken = Spoken {
            summary,
            status,
            at: Some(std::time::Instant::now()),
        };
        Some((view.panel, changed))
    })
    .flatten() else {
        return;
    };
    for index in changed {
        sidebar_accessibility::notify(EVENT_OBJECT_NAMECHANGE, panel, Some(index));
    }
}

/// Tells screen readers a toggle's checked state changed.
fn announce_toggle(hwnd: HWND, option: SearchOption) {
    if side_panel::current_view(hwnd) != SidebarView::Search {
        return;
    }
    if let Some((panel, index)) = with_view(hwnd, |view| {
        Some((view.panel, view.child_index(SearchChild::Toggle(option))?))
    })
    .flatten()
    {
        sidebar_accessibility::notify(EVENT_OBJECT_STATECHANGE, panel, Some(index));
    }
}
```

2. Call them. Each call is the function's last statement, after every `with_view` borrow has ended, and outside any borrow:
   - `begin_search`: `announce_lines(hwnd, false);`
   - `apply_batch`: `announce_lines(hwnd, batch_end.is_some());`. Copy `let batch_end = batch.end;` at the top of the function, before `batch` moves.
   - `set_pattern_error`: `announce_lines(hwnd, true);`
   - `query_changed`, on the path that sets `SearchState::TooShort` or clears the query: `announce_lines(hwnd, true);`
   - `toggle_option`: `announce_toggle(hwnd, option);`, placed before its `text_search_host::run_now` call. The run then announces its own lines.

3. In `text_search_host::batch_arrived`, the call that forwards the batch (`search_view::apply_batch(hwnd, batch)`) becomes

```rust
        crate::window::side_panel::with_accessible_events(hwnd, || {
            crate::window::search_view::apply_batch(hwnd, batch)
        });
```

   New rows then raise the panel's `EVENT_OBJECT_REORDER`, as a keystroke inside `panel_proc` already does. It costs two `AccessibleMark` reads per batch, which touch only the current child, well within the 2 ms batch budget.

- [ ] **Step 5: The find bar's children** in `src/window/find_bar.rs`, `src/window/panel.rs` and `src/window/main_window.rs`

1. `find_bar.rs`: add the imports `use crate::window::sidebar_accessibility::{self, AccessibleItem, AccessibleSource};` and `ROLE_SYSTEM_TOOLBAR` from `windows_sys::Win32::UI::Accessibility`. Then add to `impl FindBar`:

```rust
    /// The bar's MSAA children, in order: the Find field, the three toggles, the Replace field
    /// in Replace mode, and the close button.
    pub(crate) fn accessible_items(&self) -> Vec<AccessibleItem> {
        let (layout, dpi) = self.current_layout();
        let focus = unsafe { GetFocus() };
        let mut items = vec![sidebar_accessibility::field_item(
            "Find",
            self.query_text(),
            focus == self.query_edit,
            layout.query.field,
            self.query_edit,
        )];
        let rects = option_toggles::toggle_rects(layout.query.field, dpi);
        for (option, rect) in SearchOption::ALL.into_iter().zip(rects) {
            items.push(sidebar_accessibility::check_item(
                option_toggles::label(option),
                self.options.get(option),
                rect,
            ));
        }
        if let Some(replace) = layout.replace {
            items.push(sidebar_accessibility::field_item(
                "Replace",
                self.replace_text(),
                focus == self.replace_edit,
                replace.field,
                self.replace_edit,
            ));
        }
        items.push(sidebar_accessibility::button_item(
            "Close",
            false,
            false,
            layout.close,
        ));
        items
    }
```

   and after `impl Drop for FindBar`:

```rust
/// The 0-based MSAA child of `option`'s toggle: right after the Find field.
pub(crate) fn toggle_child(option: SearchOption) -> usize {
    1 + SearchOption::ALL
        .iter()
        .position(|shown| *shown == option)
        .unwrap_or(0)
}

/// Runs `f` on the find bar whose panel is `panel`, on the window's own thread.
fn with_bar<R>(panel: HWND, f: impl FnOnce(&FindBar) -> R) -> Option<R> {
    let main = unsafe { GetParent(panel) };
    let app = unsafe { super::main_window::app_ptr(main) }?;
    let bar = unsafe { app.as_ref() }
        .find_bar
        .as_ref()
        .filter(|bar| bar.panel == panel)?;
    Some(f(bar))
}

fn accessible_container(panel: HWND) -> (String, u32) {
    let replace = with_bar(panel, |bar| bar.mode == FindBarMode::Replace).unwrap_or(false);
    let name = if replace { "Find and replace" } else { "Find" };
    (name.to_owned(), ROLE_SYSTEM_TOOLBAR)
}

fn accessible_count(panel: HWND) -> usize {
    with_bar(panel, |bar| bar.accessible_items().len()).unwrap_or(0)
}

fn accessible_item(panel: HWND, index: usize) -> Option<AccessibleItem> {
    with_bar(panel, |bar| bar.accessible_items().into_iter().nth(index)).flatten()
}

/// The toggles sit inside the Find field, so the last child under the point wins.
fn accessible_hit(panel: HWND, point: POINT) -> Option<usize> {
    with_bar(panel, |bar| {
        bar.accessible_items().iter().rposition(|item| {
            point.x >= item.rect.left
                && point.x < item.rect.right
                && point.y >= item.rect.top
                && point.y < item.rect.bottom
        })
    })
    .flatten()
}

fn accessible_current(_panel: HWND) -> Option<usize> {
    None
}

fn accessible_select(_panel: HWND, _index: usize) {}

/// A field's default action focuses it. A toggle's or the close button's is a click on its
/// center, which `main_window::panel_pointer` handles as the mouse's.
fn accessible_activate(panel: HWND, index: usize) {
    let Some(item) = accessible_item(panel, index) else {
        return;
    };
    if item.window.is_null() {
        sidebar_accessibility::click_item(panel, item.rect);
    } else {
        unsafe {
            SetFocus(item.window);
        }
    }
}

fn accessible_identity(_panel: HWND, index: usize) -> Option<u64> {
    Some(index as u64)
}

fn accessible_generation(_panel: HWND) -> u64 {
    0
}

pub(crate) static FIND_BAR_ACCESSIBLE: AccessibleSource = AccessibleSource {
    container: accessible_container,
    count: accessible_count,
    item: accessible_item,
    hit: accessible_hit,
    current: accessible_current,
    select: accessible_select,
    activate: accessible_activate,
    identity: accessible_identity,
    generation: accessible_generation,
};
```

2. `panel.rs`: add `OBJID_CLIENT, WM_GETOBJECT` to the `WindowsAndMessaging` import, and these arms to `panel_proc` before the `_ =>` arm. Only the find bar's panel answers. The palette and the name box keep the system's default object, which lists their native controls.

```rust
        WM_GETOBJECT
            if lparam as i32 == OBJID_CLIENT && super::main_window::find_bar_owns(main, hwnd) =>
        unsafe {
            super::sidebar_accessibility::object_result(
                hwnd,
                &super::find_bar::FIND_BAR_ACCESSIBLE,
                wparam,
            )
        },
        super::sidebar_accessibility::WM_FASTPAD_SIDEBAR_ACCESSIBLE => unsafe {
            super::sidebar_accessibility::answer(hwnd, lparam)
        },
        super::sidebar_accessibility::WM_FASTPAD_SIDEBAR_ACTION
            if super::main_window::find_bar_owns(main, hwnd) =>
        {
            super::sidebar_accessibility::run_action(
                hwnd,
                &super::find_bar::FIND_BAR_ACCESSIBLE,
                wparam,
                lparam,
            );
            0
        }
```

   `find_bar_owns` borrows the `App` only long enough to compare handles, and releases it before `object_result` runs, as `object_result`'s contract requires.

3. `main_window.rs`: replace `toggle_find_option` with:

```rust
/// Flips a find bar option (a toggle click, or Alt+C, Alt+W or Alt+R in its fields) and tells
/// screen readers the check button's state changed.
pub(crate) fn toggle_find_option(hwnd: HWND, option: crate::search::SearchOption) {
    let panel = unsafe { app_ptr(hwnd) }.and_then(|mut app| {
        let bar = unsafe { app.as_mut() }.find_bar.as_mut()?;
        bar.toggle_option(option);
        Some(bar.panel_hwnd())
    });
    if let Some(panel) = panel {
        crate::window::sidebar_accessibility::notify(
            windows_sys::Win32::UI::WindowsAndMessaging::EVENT_OBJECT_STATECHANGE,
            panel,
            Some(find_bar::toggle_child(option)),
        );
    }
}
```

- [ ] **Step 6: Write the window tests** in `src/window/main_window.rs`'s `tests` module

```rust
    #[test]
    fn the_search_view_exposes_its_box_toggles_summary_and_results() {
        // Break caught (spec §10): the search box missing from the panel's children (the
        // sidebar PR's known limitation), toggles read as push buttons or without their checked
        // state, or results named without their snippet.
        use crate::window::sidebar_accessibility::{
            SIDEBAR_VTABLE, STATE_CHECKED, create_for_test, take_raised,
        };
        use windows_sys::Win32::UI::Accessibility::{
            ROLE_SYSTEM_CHECKBUTTON, ROLE_SYSTEM_LISTITEM, ROLE_SYSTEM_STATICTEXT,
            ROLE_SYSTEM_TEXT,
        };
        use windows_sys::Win32::UI::WindowsAndMessaging::EVENT_OBJECT_STATECHANGE;
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("search-msaa");
        scratch.note("a.md", "one beta");
        std::fs::create_dir_all(scratch.folder().join("sub")).unwrap();
        scratch.note(r"sub\b.md", "beta two");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        scratch.install(window.hwnd);
        crate::window::side_panel::show_view(
            window.hwnd,
            crate::config::SidebarView::Search,
            true,
        );
        type_into_search(window.hwnd, "beta");
        pump_until(window.hwnd, || {
            crate::window::search_view::shown_results(window.hwnd).len() == 2
        });
        let panel = sidebar_panel(window.hwnd);
        let items = || {
            (0..crate::window::side_panel::accessible_item_count(panel))
                .filter_map(|index| crate::window::side_panel::accessible_item(panel, index))
                .collect::<Vec<_>>()
        };

        let shown = items();
        let edit = crate::window::search_view::edit_hwnd(window.hwnd).unwrap();
        assert_eq!(shown[0].role, ROLE_SYSTEM_TEXT, "{shown:?}");
        assert_eq!(shown[0].window, edit);
        assert_eq!(shown[0].value, "beta");
        let toggles = shown[1..4].iter().map(|item| item.name.as_str()).collect::<Vec<_>>();
        assert_eq!(
            toggles,
            ["Match case", "Match whole word", "Use regular expression"]
        );
        assert!(shown[1..4].iter().all(|item| item.role == ROLE_SYSTEM_CHECKBUTTON
            && item.state & STATE_CHECKED == 0));
        assert_eq!(shown[4].role, ROLE_SYSTEM_STATICTEXT);
        assert_eq!(shown[4].name, "2 notes");
        let results = shown
            .iter()
            .filter(|item| item.role == ROLE_SYSTEM_LISTITEM)
            .map(|item| item.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(results, ["a: one beta", "b, sub: beta two"]);

        take_raised();
        crate::window::search_view::toggle_option(
            window.hwnd,
            crate::search::SearchOption::Case,
        );
        assert_ne!(items()[1].state & STATE_CHECKED, 0);
        assert!(
            take_raised().contains(&(panel as usize, EVENT_OBJECT_STATECHANGE, 2)),
            "the Match case child (ID 2) raises a state change"
        );

        // The box's full object is the Edit's own.
        let com = unsafe {
            windows_sys::Win32::System::Com::CoInitializeEx(
                std::ptr::null(),
                windows_sys::Win32::System::Com::COINIT_APARTMENTTHREADED as u32,
            )
        };
        let provider = create_for_test(panel, &crate::window::side_panel::PANEL_ACCESSIBLE);
        use crate::window::accessibility::{RawVariant, VariantValue};
        unsafe {
            let mut object = std::ptr::null_mut();
            let result = (SIDEBAR_VTABLE.get_acc_child)(provider, RawVariant::integer(1), &mut object);
            assert_eq!(result, 0, "S_OK");
            assert!(!object.is_null());
            let vtable = *(object as *const *const crate::window::accessibility::AccessibleVtable);
            ((*vtable).release)(object);
            let mut none = std::ptr::null_mut();
            assert_eq!(
                (SIDEBAR_VTABLE.get_acc_child)(provider, RawVariant::integer(2), &mut none),
                windows_sys::Win32::Foundation::S_FALSE
            );
            (SIDEBAR_VTABLE.release)(provider);
        }
        if com >= 0 {
            unsafe { windows_sys::Win32::System::Com::CoUninitialize() };
        }
    }

    #[test]
    fn the_summary_speaks_at_most_once_a_second_while_a_search_runs() {
        // Break caught: a name change per batch (up to 20 a second) flooding the screen reader,
        // or the final count swallowed by the limit.
        use crate::library::text_search::{Progress, RunEnd, TextHit};
        use crate::window::sidebar_accessibility::take_raised;
        use crate::window::text_search_host::SearchBatch;
        use windows_sys::Win32::UI::WindowsAndMessaging::EVENT_OBJECT_NAMECHANGE;
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("search-speak");
        scratch.note("a.md", "a");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        scratch.install(window.hwnd);
        crate::window::side_panel::show_view(
            window.hwnd,
            crate::config::SidebarView::Search,
            false,
        );
        let panel = sidebar_panel(window.hwnd);
        let hit = |name: &str| TextHit {
            path: std::path::PathBuf::from(format!("{name}.md")),
            name: name.to_owned(),
            folder: String::new(),
            snippet: crate::search::Snippet {
                text: "beta".to_owned(),
                highlight: 0..4,
            },
            stamp: None,
        };
        let progress = |visited| Progress {
            visited,
            total: 100,
            skipped: [0; 4],
        };
        let name_changes = || {
            take_raised()
                .into_iter()
                .filter(|&(hwnd, event, _)| hwnd == panel as usize && event == EVENT_OBJECT_NAMECHANGE)
                .count()
        };
        take_raised();

        crate::window::search_view::begin_search(window.hwnd, "beta", 100);
        for (visited, name) in [(10, "a"), (20, "b"), (30, "c")] {
            crate::window::search_view::apply_batch(
                window.hwnd,
                SearchBatch {
                    generation: 0,
                    hits: vec![hit(name)],
                    progress: progress(visited),
                    end: None,
                },
            );
        }
        assert!(name_changes() <= 2, "one announcement (summary and status) at most");

        crate::window::search_view::apply_batch(
            window.hwnd,
            SearchBatch {
                generation: 0,
                hits: Vec::new(),
                progress: progress(100),
                end: Some(RunEnd::Completed),
            },
        );
        assert!(name_changes() >= 1, "the finished search is announced");
    }

    #[test]
    fn the_find_bar_exposes_its_fields_toggles_and_close_button() {
        // Break caught: the find bar's toggles invisible to screen readers, or a default action
        // that doesn't flip them.
        use crate::window::find_bar::FIND_BAR_ACCESSIBLE;
        use crate::window::sidebar_accessibility::{STATE_CHECKED, take_raised};
        use windows_sys::Win32::UI::Accessibility::{
            ROLE_SYSTEM_CHECKBUTTON, ROLE_SYSTEM_PUSHBUTTON, ROLE_SYSTEM_TEXT,
        };
        use windows_sys::Win32::UI::WindowsAndMessaging::{EVENT_OBJECT_STATECHANGE, WM_SYSKEYDOWN};
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        execute_command(window.hwnd, CommandId::Find);
        let (panel, query) = {
            let bar = app_mut(window.hwnd).find_bar.as_ref().unwrap();
            (bar.panel_hwnd(), bar.query_hwnd())
        };
        let items = || {
            (0..(FIND_BAR_ACCESSIBLE.count)(panel))
                .filter_map(|index| (FIND_BAR_ACCESSIBLE.item)(panel, index))
                .collect::<Vec<_>>()
        };

        let shown = items();
        let roles = shown.iter().map(|item| item.role).collect::<Vec<_>>();
        assert_eq!(
            roles,
            [
                ROLE_SYSTEM_TEXT,
                ROLE_SYSTEM_CHECKBUTTON,
                ROLE_SYSTEM_CHECKBUTTON,
                ROLE_SYSTEM_CHECKBUTTON,
                ROLE_SYSTEM_PUSHBUTTON
            ]
        );
        assert_eq!(shown[0].window, query);
        assert_eq!(shown[2].name, "Match whole word");

        take_raised();
        unsafe { SendMessageW(query, WM_SYSKEYDOWN, usize::from(b'W'), 1 << 29) };
        assert_ne!(items()[2].state & STATE_CHECKED, 0);
        assert!(take_raised().contains(&(panel as usize, EVENT_OBJECT_STATECHANGE, 3)));

        // The default action clicks the toggle, as the mouse does.
        (FIND_BAR_ACCESSIBLE.activate)(panel, 1);
        assert!(app_mut(window.hwnd).find_bar.as_ref().unwrap().options().case);

        execute_command(window.hwnd, CommandId::Replace);
        assert_eq!((FIND_BAR_ACCESSIBLE.count)(panel), 6);
        assert_eq!(items()[4].name, "Replace");
    }
```

Also add to `src/window/search_view.rs`'s `tests` module:

```rust
    #[test]
    fn a_result_reads_its_name_folder_and_snippet() {
        // Break caught: a screen reader hearing only the note name, with no hint of why it
        // matched, or a stray ", " for a note at the root.
        use super::result_name;
        use crate::library::text_search::TextHit;
        let hit = |folder: &str| TextHit {
            path: std::path::PathBuf::from("q1.md"),
            name: "Q1 budget".to_owned(),
            folder: folder.to_owned(),
            snippet: crate::search::Snippet {
                text: "\u{2026}paid the invoice march 3\u{2026}".to_owned(),
                highlight: 12..25,
            },
            stamp: None,
        };
        assert_eq!(
            result_name(&hit("work")),
            "Q1 budget, work: \u{2026}paid the invoice march 3\u{2026}"
        );
        assert_eq!(
            result_name(&hit("")),
            "Q1 budget: \u{2026}paid the invoice march 3\u{2026}"
        );
    }
```

Run: `cargo test --lib -- window::sidebar_accessibility window::search_view::tests::a_result_reads window::main_window::tests::the_search_view_exposes window::main_window::tests::the_summary_speaks window::main_window::tests::the_find_bar_exposes window::main_window::tests::pinning_a_note --test-threads=1`
Expected: all pass. That includes the existing `pinning_a_note_that_is_not_first_raises_reorder_and_state_change`, whose Notebook view children didn't change.

Then run the sidebar e2e tests, which read the panel's children out of process:

Run: `cargo build`, then `cargo test --test library -- --test-threads=1`
Expected: all pass. `tree_rows` filters by `ROLE_SYSTEM_OUTLINEITEM`, so the Search view's new children don't disturb it.

- [ ] **Step 7: Lint, format, commit**

Run: `cargo clippy --all-targets -- -D warnings`, then `cargo fmt --all -- --check`
Expected: clean.

```
git add src/window/sidebar_accessibility.rs src/window/search_view.rs src/window/option_toggles.rs src/window/find_bar.rs src/window/panel.rs src/window/main_window.rs src/window/text_search_host.rs
git commit -m "feat(a11y): search options are check buttons in Search and the find bar, results read their snippet, the search box is a panel child, and the summary and status lines announce changes at most once a second"
```

---

### Task 9: Bench, end-to-end test, docs

**Files:**
- Modify:
  - `src/bin/fastpad-bench.rs`
  - `benchmarks/README.md`
  - `tests/windows/library.rs`
  - `README.md`
  - `docs/superpowers/specs/2026-09-24-note-search-design.md` (§17)

**Interfaces:**
- Consumes these public items. `fastpad-bench` is a separate binary, so everything it uses must be `pub`:
  - `fastpad::library::text_search::{run, SearchNote, TextHit, Progress, hit_cmp, BATCH_HITS}` (Task 3).
  - `fastpad::search::{Matcher, MatchOptions, Snippet}` (Tasks 1 and 2).
  - `fastpad::library::NoteEntry.online_only` (Task 3).
- The e2e test consumes Task 8's result name format ("<name>: <snippet>" at the root) and the `ShowSearchView` and `FindNext` commands.
- Produces:
  - `library-scan` output lines `text_search_first_batch_ms`, `text_search_full_ms` and `text_search_batch_ui_ms`, gated by `--enforce-reference` at 50, 400 and 2 ms.
  - The e2e test.
  - The size and memory numbers.
  - The README and spec updates.

**Machine hygiene:**
- Every real-exe test runs against a scratch `LOCALAPPDATA` and a scratch notes folder, and `Scratch::new` refuses to run while any FastPad runs in the session.
- The bench writes its text notebook to a scratch folder under the build's `target` directory (`target\bench-notes\text-search-<pid>`, git-ignored), not under `%TEMP%`, and removes it on exit, early `?` returns included. Files freshly written under `%TEMP%` are scanned by the antivirus: Task 3 measured about 750 ms to read 10,000 of them warm there, which says nothing about the search.
- Before any manual run of the live app against your own profile, copy `%LOCALAPPDATA%\FastPad\fastpad.ini` and `folders.ini` aside, and restore them afterwards.

- [ ] **Step 1: Add the text-search timings** to `src/bin/fastpad-bench.rs`

1. Add these constants after `NAME_SEARCH_REFERENCE_MS`:

```rust
/// The note-search spec's §14 text search targets on the reference machine, over
/// `TEXT_SEARCH_NOTES` notes of about 4 KB each with a warm OS cache.
const TEXT_SEARCH_NOTES: usize = 10_000;
const TEXT_SEARCH_NOTE_BYTES: usize = 4_096;
/// Every this-many-th note mentions the invoice: 200 hits, so the full search visits every note
/// without reaching the 500-note cap.
const TEXT_SEARCH_RARE_EVERY: usize = 50;
const TEXT_SEARCH_FIRST_BATCH_REFERENCE_MS: f64 = 50.0;
const TEXT_SEARCH_FULL_REFERENCE_MS: f64 = 400.0;
const TEXT_SEARCH_BATCH_UI_REFERENCE_MS: f64 = 2.0;
```

2. Add after `create_library_fixture`:

```rust
/// Note `index` of the text-search fixture: about `TEXT_SEARCH_NOTE_BYTES` of prose, every line
/// holding "lazy dog". One note in `TEXT_SEARCH_RARE_EVERY` also mentions an invoice, on its
/// last line, so a search for it reads the whole note first.
fn text_search_note(index: usize) -> String {
    let mut text = format!("# Note {index}\r\n\r\n");
    while text.len() < TEXT_SEARCH_NOTE_BYTES - 64 {
        text.push_str("The quick brown fox jumps over the lazy dog.\r\n");
    }
    if index % TEXT_SEARCH_RARE_EVERY == 0 {
        text.push_str(&format!("Paid the invoice march {index}.\r\n"));
    }
    text
}

/// Writes `TEXT_SEARCH_NOTES` fixture notes into `folder`, 500 per subfolder.
fn create_text_search_fixture(folder: &Path) -> Result<(), String> {
    for index in 0..TEXT_SEARCH_NOTES {
        let path = folder.join(format!(r"batch{}\note{index}.md", index / 500));
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
        }
        std::fs::write(&path, text_search_note(index))
            .map_err(|error| format!("could not write {}: {error}", path.display()))?;
    }
    Ok(())
}

/// The scratch folder for the text-search notebook: `bench-notes\text-search-<pid>` in the build's
/// `target` directory, which git ignores. Not `%TEMP%`: the antivirus scans files freshly written
/// there, and reading them would time the scanner, not the search. `fastpad-bench.exe` runs from
/// `target\<profile>`, so `target` is the nearest ancestor of that name, or else the exe folder's
/// parent.
fn text_search_scratch_root() -> Result<PathBuf, String> {
    let exe = std::env::current_exe()
        .map_err(|error| format!("could not find the bench executable: {error}"))?;
    let target = exe
        .ancestors()
        .skip(1)
        .find(|dir| {
            dir.file_name()
                .is_some_and(|name| name.eq_ignore_ascii_case("target"))
        })
        .or_else(|| exe.parent().and_then(Path::parent))
        .ok_or_else(|| format!("{} has no parent folder", exe.display()))?;
    Ok(target
        .join("bench-notes")
        .join(format!("text-search-{}", std::process::id())))
}

/// Times the note-search spec's §14 text search over a notebook generated in a scratch folder.
/// Returns, in milliseconds:
/// - the first batch for a phrase in every note;
/// - the whole search for a phrase in one note in fifty;
/// - the UI thread's sorted insert of one 50-hit batch into 450 shown results.
///
/// The `InvalidateRect` that follows a batch is not part of it: it only queues a paint.
fn text_search_timings() -> Result<(f64, f64, f64), String> {
    use fastpad::library::text_search::{self, BATCH_HITS, Progress, SearchNote, TextHit};
    use fastpad::search::{MatchOptions, Matcher, Snippet};
    use std::sync::atomic::{AtomicBool, Ordering};

    // Removed on drop, with the notebook and its library.ini, on every return.
    let root = ScratchDir(text_search_scratch_root()?);
    let _ = std::fs::remove_dir_all(&root.0);
    let notebook = root.0.join("notes");
    std::fs::create_dir_all(&notebook)
        .map_err(|error| format!("could not create {}: {error}", notebook.display()))?;
    create_text_search_fixture(&notebook)?;
    let local = root.0.join("library.ini");
    let state = fastpad::library::load(&notebook, &local, fastpad::library::now_unix())
        .map_err(|error| format!("could not load {}: {error}", notebook.display()))?;
    let notes = state
        .notes
        .iter()
        .map(|note| SearchNote {
            path: note.path.clone(),
            size: note.size,
            online_only: note.online_only,
        })
        .collect::<Vec<_>>();
    let overlays = std::collections::HashMap::new();
    let common = Matcher::new("lazy dog", MatchOptions::default())
        .map_err(|error| error.to_string())?;
    let rare = Matcher::new("invoice march", MatchOptions::default())
        .map_err(|error| error.to_string())?;
    // One untimed pass warms the OS cache, as §14 measures.
    text_search::run(
        &notebook,
        &notes,
        &overlays,
        &rare,
        &AtomicBool::new(false),
        &mut |_: Vec<TextHit>, _: Progress| {},
    );

    // The first batch fills at 50 hits, after 50 notes; the sink then cancels the rest.
    let first_batch_ms = median_ms(|| {
        let cancel = AtomicBool::new(false);
        text_search::run(
            &notebook,
            &notes,
            &overlays,
            &common,
            &cancel,
            &mut |hits: Vec<TextHit>, _: Progress| {
                std::hint::black_box(hits);
                cancel.store(true, Ordering::Relaxed);
            },
        );
    });
    let full_ms = median_ms(|| {
        std::hint::black_box(text_search::run(
            &notebook,
            &notes,
            &overlays,
            &rare,
            &AtomicBool::new(false),
            &mut |hits: Vec<TextHit>, _: Progress| {
                std::hint::black_box(hits);
            },
        ));
    });

    let hit = |index: usize| TextHit {
        path: PathBuf::from(format!(r"batch{}\note{index}.md", index / 500)),
        name: format!("note{index}"),
        folder: format!("batch{}", index / 500),
        snippet: Snippet {
            text: format!("Paid the invoice march {index}."),
            highlight: 9..22,
        },
        stamp: None,
    };
    let mut shown = (0..450).map(|i| hit(i * 20)).collect::<Vec<_>>();
    shown.sort_by(text_search::hit_cmp);
    let batch = (0..BATCH_HITS).map(|i| hit(i * 20 + 10)).collect::<Vec<_>>();
    let mut times = Vec::with_capacity(LIBRARY_SCAN_WARM_LOADS);
    for _ in 0..LIBRARY_SCAN_WARM_LOADS {
        let mut results = shown.clone();
        let incoming = batch.clone();
        let started = std::time::Instant::now();
        for hit in incoming {
            let at = results
                .binary_search_by(|probe| text_search::hit_cmp(probe, &hit))
                .unwrap_or_else(|at| at);
            results.insert(at, hit);
        }
        times.push(started.elapsed().as_secs_f64() * 1_000.0);
        std::hint::black_box(results);
    }
    times.sort_by(f64::total_cmp);
    Ok((first_batch_ms, full_ms, times[times.len() / 2]))
}

/// A scratch folder removed on drop, including on an early `?` return.
struct ScratchDir(PathBuf);

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
```

3. In `run_library_scan`:
   - After the `name_search_ms` block, add:

```rust
    let (text_search_first_batch_ms, text_search_full_ms, text_search_batch_ui_ms) =
        text_search_timings()?;
```

   - After `println!("name_search_ms={name_search_ms:.2}");`, add:

```rust
    println!("text_search_first_batch_ms={text_search_first_batch_ms:.2}");
    println!("text_search_full_ms={text_search_full_ms:.2}");
    println!("text_search_batch_ui_ms={text_search_batch_ui_ms:.3}");
```

   - In the `failures` array, after the name-search row, add:

```rust
            (
                "text search, first batch",
                text_search_first_batch_ms,
                TEXT_SEARCH_FIRST_BATCH_REFERENCE_MS,
            ),
            (
                "text search, whole notebook",
                text_search_full_ms,
                TEXT_SEARCH_FULL_REFERENCE_MS,
            ),
            (
                "text search, one batch on the UI thread",
                text_search_batch_ui_ms,
                TEXT_SEARCH_BATCH_UI_REFERENCE_MS,
            ),
```

4. Add to the bench's `tests` module:

```rust
    #[test]
    fn text_search_notes_are_about_4_kb_and_one_in_fifty_mentions_the_invoice() {
        // Break caught: a fixture that measures 200-byte notes, or one where every note (or no
        // note) matches the rare phrase, so the full search is capped or finds nothing.
        for index in [0, 1, 49, 50, 9_999] {
            let text = super::text_search_note(index);
            assert!(
                (4_000..=4_200).contains(&text.len()),
                "{index}: {}",
                text.len()
            );
            assert!(text.contains("lazy dog"));
            assert_eq!(text.contains("invoice march"), index % 50 == 0, "{index}");
        }
    }
```

Run: `cargo test --bin fastpad-bench`
Expected: all pass.

- [ ] **Step 2: Run the bench once by hand and record the numbers**

```
cargo build --release --bin fastpad --bin fastpad-bench
target\release\fastpad-bench.exe library-scan %TEMP%\fastpad-10k --count 10000
```

This run is without `--enforce-reference`: it records the numbers. The three text-search gates (50, 400 and 2 ms) apply only when `--enforce-reference` is passed, on the reference machine.

Targets on the reference i5-4590:
- `text_search_first_batch_ms` under 50.
- `text_search_full_ms` under 400.
- `text_search_batch_ui_ms` under 2.
- The sidebar's `tree_*` and `name_search_ms` lines within their gates, as before.

The text notebook (about 40 MB) is written under `target\bench-notes` and removed on each run, which adds a few seconds.

A text-search target that is missed does **not** fail this plan:
- Record the measured numbers in spec §17 (Step 7) and in the PR description, with the machine they were measured on.
- For `text_search_full_ms` at or above 400, §17 also says that spec §14 names this as the point where an index would be justified. Don't add one in 3a.

- [ ] **Step 3: Document the bench** in `benchmarks/README.md`. Append:

```markdown
## Note search

`library-scan` also generates a second notebook (10,000 notes of about 4 KB, 500 per folder) in
a scratch folder under the build's `target` directory (`target\bench-notes`, git-ignored), not
under `%TEMP%`, whose antivirus scanning of fresh files would dominate the timings. It warms the
OS cache with one untimed search and times the text search of the note search spec (§14), as the
median of five runs each:
- `text_search_first_batch_ms`: from starting the worker to its first batch, for a phrase in
  every note. The target is under 50 ms.
- `text_search_full_ms`: the whole search for a phrase in one note in fifty (200 hits, below
  the 500-note cap), so every note is read. The target is under 400 ms.
- `text_search_batch_ui_ms`: the UI thread's part of one batch, inserting 50 hits into 450
  shown results by binary search. The `InvalidateRect` that follows only queues a paint and
  isn't timed. The target is under 2 ms.

`--enforce-reference` fails the run when any of them reaches its target; without it the numbers
are only printed. A full search at or above 400 ms on the reference machine is the point where §14
says an index would be justified.

The note search build's idle private working set, with the Search view open on a 10,000-note
notebook and nothing typed, may grow by at most 0.5 MB over the `feat/note-sidebar` build with
the same notebook. The `regex` crate may add at most 1.5 MB to the release exe.
```

- [ ] **Step 4: Add the end-to-end test** to `tests/windows/library.rs`

1. Imports:
   - Add `ROLE_SYSTEM_CHECKBUTTON, ROLE_SYSTEM_TEXT` to the `windows_sys::Win32::UI::Accessibility` import.
   - Change `use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_RETURN;` to `use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_F3, VK_RETURN};`.
   - Add `SendMessageW` to the `WindowsAndMessaging` import.

2. In `AccessibleVtable`, change `get_acc_child: usize,` to:

```rust
    get_acc_child: unsafe extern "system" fn(*mut c_void, VARIANT, *mut *mut c_void) -> HRESULT,
```

3. In `impl Accessible`, add:

```rust
    /// Whether child `child` has a full object of its own (a native control), releasing it.
    fn has_child_object(&self, child: i32) -> bool {
        let mut object = std::ptr::null_mut();
        let result =
            unsafe { (self.vtable().get_acc_child)(self.0, child_variant(child), &mut object) };
        if result < 0 || object.is_null() {
            return false;
        }
        let vtable = unsafe { &**(object as *const *const AccessibleVtable) };
        unsafe { (vtable.release)(object) };
        true
    }
```

4. After `write_folders`, add:

```rust
/// The editor's selection, as Scintilla byte positions.
fn selection(editor: HWND) -> (isize, isize) {
    use fastpad::editor::scintilla_constants::{SCI_GETSELECTIONEND, SCI_GETSELECTIONSTART};
    unsafe {
        (
            SendMessageW(editor, SCI_GETSELECTIONSTART, 0, 0),
            SendMessageW(editor, SCI_GETSELECTIONEND, 0, 0),
        )
    }
}
```

5. Add the test:

```rust
#[test]
fn searching_the_notebook_opens_a_result_at_its_first_match_and_f3_steps_on() {
    // Break caught: the Search view not searching note text in the real exe, the box or
    // toggles missing from what a screen reader sees, a result opening without the find bar
    // seeded, the first match not selected, or F3 not reaching the next match.
    let _lock = LIBRARY_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _dpi = DpiContext::per_monitor_v2();
    let _com = ComApartment::initialize();
    let data = Scratch::new("text-search");
    data.note("a.md", "alpha\r\nthe invoice march is paid\r\ninvoice again\r\n");
    data.note("b.md", "nothing to see");
    let mut process =
        FastPadProcess::spawn_with_local_app_data([data.folder()], &data.root).unwrap();
    let hwnd = process.wait_for_main_window(WAIT).unwrap();
    let editor = find_child_by_class(hwnd, "Scintilla").unwrap();
    wait_for_library(&data);
    let panel = find_child_by_class(hwnd, SIDE_PANEL_CLASS).unwrap();

    // Ctrl+Shift+F's command; the in-process tests pin the accelerator itself.
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
    let result = "a: the invoice march is paid";
    wait_until("the result", || panel_lists(panel, result));
    assert!(!panel_lists(panel, "b: nothing to see"));

    let accessible = Accessible::from_window(panel).unwrap();
    let children = accessible.children();
    assert!(
        children.iter().any(|(id, name)| {
            name == "Match case" && accessible.role(*id) == Some(ROLE_SYSTEM_CHECKBUTTON)
        }),
        "{children:?}"
    );
    let (box_id, _) = children
        .iter()
        .find(|(id, _)| accessible.role(*id) == Some(ROLE_SYSTEM_TEXT))
        .expect("the search box is one of the panel's children");
    assert!(accessible.has_child_object(*box_id));
    drop(accessible);

    click_child(panel, result);
    // "alpha\r\n" is 7 bytes and "the " 4 more: the first "invoice" is 11..18.
    wait_until("a to open at its first match", || selection(editor) == (11, 18));
    assert!(scintilla_text(editor).is_ok_and(|text| text.starts_with("alpha\r\n")));
    unsafe {
        PostMessageW(editor, WM_KEYDOWN, VK_F3 as usize, 0);
    }
    // The next line starts at 7 + 27 = 34.
    wait_until("F3 to reach the next match", || selection(editor) == (34, 41));
    close(process, hwnd);
}
```

Run: `cargo build`, then `cargo test --test library -- --test-threads=1`
Expected: all pass, the new test included. A panic "close every FastPad window in this session…" means a FastPad is running. Close it and run again.

- [ ] **Step 5: Measure the exe size and the idle memory against `feat/note-sidebar`**

Build the baseline in a separate worktree. The worktree has no native build output, so copy it over:

```
git worktree add ..\FastPad-note-sidebar feat/note-sidebar
Copy-Item -Recurse native\out ..\FastPad-note-sidebar\native\out
cd ..\FastPad-note-sidebar
cargo build --release --features release-package --bin fastpad --bin fastpad-bench
(Get-Item target\release\fastpad.exe).Length
cd ..\FastPad
cargo build --release --features release-package --bin fastpad --bin fastpad-bench
(Get-Item target\release\fastpad.exe).Length
```

Expected: this branch's exe is at most 1,572,864 bytes (1.5 MB) larger. Note both sizes and the difference.

Idle memory, with the same 10,000-note notebook from Step 2. Both builds read the same `library.ini` version, and nothing here changes a file format. Run the two builds back to back, in pairs, because the machine drifts:

```
..\FastPad-note-sidebar\target\release\fastpad-bench.exe --runs 30 --warmup 5 --notes-folder %TEMP%\fastpad-10k --sidebar-view search --output benchmarks\sidebar-search-open.jsonl
target\release\fastpad-bench.exe --runs 30 --warmup 5 --notes-folder %TEMP%\fastpad-10k --sidebar-view search --output benchmarks\note-search-open.jsonl
target\release\fastpad-bench.exe compare benchmarks\sidebar-search-open.jsonl benchmarks\note-search-open.jsonl
```

Expected:
- `compare` reports no regressed milestone. Nothing new runs before first paint or first input.
- The printed `idle_private_working_set_bytes` p50 of the second run is at most 524,288 bytes (0.5 MB) above the first.

Each launch uses its own scratch profile, so nothing here reads or writes the real `fastpad.ini`.

Afterwards:
- Remove the worktree with `git worktree remove ..\FastPad-note-sidebar`.
- Delete `%TEMP%\fastpad-10k` and the `benchmarks\*.jsonl` files from this step. They are not committed.
- Paste all outputs into the PR description.

- [ ] **Step 6: Update `README.md`**

1. In "Notes and notebooks", replace the bullet Task 6 left, `- **Ctrl+Shift+F** searches your notes. Star a notebook to keep it in **Favorites**, and switch` together with its continuation line `  between notebooks from there.`, with:

```markdown
- **Ctrl+Shift+F** searches the text of every note in the notebook as you type, with match
  case (**Alt+C**), whole word (**Alt+W**) and regular expression (**Alt+R**) toggles. Select
  a word first and it becomes the search. Opening a result puts your search in the find bar,
  so **F3** steps through every match in that note. Nothing is indexed: the notes are read
  when you search, and results stream in as they're found.
- Star a notebook to keep it in **Favorites**, and switch between notebooks from there.
```

2. In the features list, replace `- **Find and replace**, zoom, word wrap, line numbers, and left-to-right or right-to-left text.` with:

```markdown
- **Find and replace**, with match case, whole word and regular expressions (Alt+C, Alt+W,
  Alt+R) and F3 / Shift+F3, plus zoom, word wrap, line numbers, and left-to-right or
  right-to-left text.
```

3. In the shortcuts table, after the `| Move note to notebook | …` row, add:

```markdown
| Find next / previous | `F3` / `Shift+F3` | | Match case / whole word / regex | `Alt+C` / `Alt+W` / `Alt+R` |
```

   After Task 6, the Search notes cell already reads `` `Ctrl+Shift+F` `` and the Format JSON cell `` `Shift+Alt+F` ``. Check both.

- [ ] **Step 7: Write the implementation notes** in `docs/superpowers/specs/2026-09-24-note-search-design.md`. Replace §17's placeholder line with the following. Fill each `…` with the numbers measured in Steps 2 and 5; they are measurements, not text to invent now.

```markdown
- **The debounce timer lives on the main window** (a deviation from §7, which said "on the
  panel, with the timer ID in `ids.rs`"). `TEXT_SEARCH_TIMER_ID` (`0x4650_5453`) is defined in
  `text_search_host.rs` and handled in the main window's `WM_TIMER`, like
  `LIBRARY_WRITE_TIMER_ID`. `ids.rs` holds library IDs, and every other timer is the main
  window's.
- **F3 and Shift+F3 are new commands,** `FindNext` (188) and `FindPrevious` (189). §8 assumed
  they existed; nothing bound F3 before. With no query yet, they open the find bar. They're in
  the Search menu and the palette.
- **The find bar gained a no-match state.** §8 speaks of its "existing" one, but a miss used to
  leave no trace. Now the query field's outline turns `Palette.error_foreground`, the error
  color added for the Search view's pattern error, until the query changes or a search finds
  something. An invalid regex shows it too. Scintilla reports a bad pattern as -1 (-2 in some versions), which is a miss,
  never an error or a notice.
- **Whole word in the find bar's regex mode wraps the pattern** as `\b(?:…)\b` instead of
  passing `SCFIND_WHOLEWORD`, which Scintilla's regex search ignores. This matches Search.
- **The find bar ignores empty regex matches.** A pattern like `x*` would match at the caret
  forever, so an empty match counts as no match, and Replace all stops at one. Search rejects
  such patterns outright (§6).
- **Scintilla is built with C++11 regex** (`NO_CXX11_REGEX` is never set by
  `tools/build-native.ps1`). An in-process test pins it with `\d{2}`.
- **A regex match's length comes from Scintilla** (`SCI_GETTARGETEND`), not from the pattern's
  length, so find next, replace and Replace all handle regex matches correctly. Replace in the
  find bar inserts its text literally, in regex mode too (no `$1` or `\1`).
- **Enter on a result keeps the focus in the list** (sidebar spec §6.4); a click moves it to the
  editor (`open_search_result`'s `focus_editor`: true for a click, false for Enter). §8's "focus goes to the editor" holds for the click. Either way the first match is
  selected and F3 continues from it.
- **A selection prefilled into the find bar while regex is on** is escaped for ECMAScript
  (`find_bar::escape_pattern`), not with `regex::escape`, whose `\#` and `\-` ECMAScript rejects.
- **Accessibility:** the Search view's children are the box (its full object is the `Edit`'s
  own), the three toggles, the summary line, the status line, then the results. The status line
  comes before the results so its child ID stays fixed while they stream in. The find bar's
  panel now has a provider too: the Find field, the toggles, the Replace field in Replace mode,
  and Close.
- **`text_search_batch_ui_ms` times the sorted insert only.** It inserts 50 hits into 450 by
  binary search. The `InvalidateRect` after it only queues a paint.
- **Format JSON has no menu-bar entry with a shortcut.** The palette row shows Shift+Alt+F from
  the accelerator table; the overflow menu shows no shortcuts for any entry.
- **A regex error's message** is the last line of the `regex` crate's error text
  (`error: <description>`), with `error:` stripped and a capital first letter: "Unclosed group".
  A pattern too large to compile reads "The pattern is too large.", a message §4 doesn't list.
- **The snippet's 80 characters "after" the match are counted from the match's start** (§7): the
  match and what follows it together are at most 80 characters, so a match longer than that is
  itself cut with `…`.
- **`name_search::folder_of` became `pub(super)`** (§9 says nothing else changes in
  `name_search`). It is a visibility change only: text search reuses it so a result's folder
  follows the same rule as a name match's.
- **The Search box is made when the user opens the Search view** (`search_view::shown`), even with
  no notebook, so the toggles work then (§4). `layout`, which also runs on the first `WM_SIZE`,
  makes it only with a notebook open, so a Search view restored at startup still makes nothing
  before the first paint (§14).
- **Narrowing is stricter than §7:** it also needs whole word off (a longer whole word can match
  where the shorter one was part of a word), and an unchanged note list and dirty-tab text. A
  library load or rescan drops the record.
- **A library change runs the query again only when the note paths or their `online_only` flags
  changed, or after a load or rescan.** `side_panel::refresh` also runs for pins, favorites and
  expansions, and a save of a listed note changes neither (§7).
- **Cancelling bumps the generation too,** so batches a cancelled search already posted are
  dropped, not only those of an older search.
- **A dirty tab's text is searched even when its note is online only or over 4 MB,** since it is
  already in memory. It is never counted as skipped.
- **The cap ends a search as "500+ notes" only when notes remain** after the 500th hit. When the
  500th hit is the last note, it is a completed search ("500 notes").
- **Measured** (reference machine, warm cache, notebook under `target\bench-notes`):
  `text_search_first_batch_ms` …, `text_search_full_ms` …, `text_search_batch_ui_ms` …. If a
  target was missed, say which, on which machine, and, for the full search, that §14 names this
  as the point where an index would be justified; none was added in 3a. Release exe with `release-package`:
  … bytes on `feat/note-sidebar`, … bytes here (+… bytes, budget 1,572,864). Idle private
  working set with the Search view open on a 10,000-note notebook: +… bytes over
  `feat/note-sidebar` (budget 524,288). `compare` found no regressed milestone.
```

- [ ] **Step 8: Lint, format, commit**

Run: `cargo clippy --all-targets -- -D warnings`, then `cargo fmt --all -- --check`
Expected: clean.

```
git add src/bin/fastpad-bench.rs benchmarks/README.md tests/windows/library.rs README.md docs/superpowers/specs/2026-09-24-note-search-design.md
git commit -m "test(search): end-to-end text search in the real exe, text search bench cases, README and implementation notes with the size and memory checks"
```

