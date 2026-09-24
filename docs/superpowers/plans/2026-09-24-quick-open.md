# Quick Open Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Open a note by typing part of its name (Ctrl+P, VS Code style, with a `:<line>` suffix), and close tabs with Ctrl+W and a middle-click.

**Architecture:**
- **Pure code:** `src/library/quick_open.rs` (new, replaces `name_search.rs`) scores every note of `LibraryState.notes` against the query's terms: letters in order, contiguous and word-start bonuses, name matches before folder-path matches, at most 50 rows, and the hit positions as char indices. `split_line` takes the `:<digits>` suffix off. `Tabs` gains an in-memory activation order (`Vec<DocumentId>`, most recent first) and a close for a clean background tab.
- **Window code:** the command palette gets `PickerKind::QuickOpen`. `main_window` builds its rows on each keystroke (the matcher's results, the notes open in tabs when nothing is typed, `Go to line <n>`, or the disabled `No notebook is open`), opens the pick with `open_note(..., OpenMode::Permanent, true)` and then `Editor::go_to_line`. The rows are owner-drawn with bold hits and a muted folder, and the empty field paints the hint `Go to note by name`. Ctrl+W is a new accelerator for `CloseTab`, except in the palette's field, where it closes the palette. `WM_MBUTTONDOWN`/`WM_MBUTTONUP` on a tab close it through `close_tab_at`.

**Tech Stack:** Rust 2024, `windows-sys` 0.61, Scintilla (`SCI_GOTOLINE`), GDI owner-draw, MSAA through the list box's strings. No new crates.

**Spec:** `docs/superpowers/specs/2026-09-24-quick-open-design.md` (binding). Deviations and decisions go into its new §9, written in Task 5.

**Branch:** `feat/quick-open`, stacked on `feat/note-replace` (PR #12). One PR.

## Global Constraints

- **Latency:** nothing new runs before first paint or first input; the palette window is still created on first use. No note-file I/O (metadata included) on the UI thread: matching runs over `LibraryState.notes` in memory, and a pick is resolved against that same list.
- **Budget:** the `library-scan` bench's `quick_open_ms` (query `nt 12`, limit 50, over the bench's notes) stays under the 5.0 ms reference.
- **App-borrow rule:** while a `&mut` from `with_view`, `library_host::with_state`, `with_host` or `app_ptr(...).as_mut()` is held, never call `SetFocus`, `SetCapture`, `CreateWindowExW`, `UpdateWindow`, `SetWindowTextW` on a child, `GetWindowTextW`, `SendMessageW` to another window, `modal::*`, or a document swap. Take the values, end the borrow, then act. The code below does this (`open_quick_open` creates the palette with nothing borrowed; `quick_open_rows` only reads).
- **Tests never touch the real profile** (`%LOCALAPPDATA%\FastPad`) or the real Documents: window tests use `LibraryScratch` / `RecoveryScratch` in `main_window.rs`'s tests and `ProductionWindow::new(make_app())`, whose library host has no data dir under `cfg(test)`.
- **Test runs:** window tests need `-- --test-threads=1`. Run only the targeted tests named in each task, plus `cargo clippy --all-targets -- -D warnings`. The full suite runs once, at the final review. In a worktree, copy the `native/out` DLLs first.
- **No backward compatibility:** `name_search` and `NameMatch` are deleted, not kept as shims.
- **Commits:** conventional messages, no attribution lines.
- **Searching:** never search the whole disk (no `find /`, no recursive listing of `C:\` or `D:\`). Crate sources are in `C:\Users\korn3\.cargo\registry\src\`.
- **Style:** match the surrounding code, its comment density and test naming; tests start with a `// Break caught:` comment.
- **Table sizes (verified against the current code):** `COMMANDS` 82 → 83, `ENTRIES` 69 → 70, `accelerator_specs` 52 → 54 (53 after Task 3). `CommandId::QuickOpen = 191`.
- **Wording (verbatim):** palette row and File menu item `Go to note…` (U+2026); hint `Go to note by name`; disabled row `No notebook is open`; line row `Go to line <n>`; a row's screen-reader text `<name>, in <folder>` (just `<name>` at the root).

## Review Focus

1. **Queries that are not plain words:** only spaces (`"   "` lists the open tabs, never all notes and never nothing), a trailing colon (`meet:` keeps listing `meet`'s matches), a colon inside the text (`a:b` is matched as text and finds nothing, with no panic) and a line too big for `u32`. Pinned in Task 1 (`a_trailing_colon_and_digits_is_a_line`) and Task 4 (`ctrl_p_again_keeps_the_query_and_a_query_of_spaces_lists_the_open_tabs`).
2. **Non-ASCII names, where char indices and byte offsets diverge:** `Über Straße` typed as `st` must bold `St` (chars 5–6, bytes 6–7), and `ÜB` must match `Über` ignoring case. Pinned in Task 1 (`hits_are_char_positions_in_the_name_and_the_folder`) and Task 5 (`hit_runs_cut_at_char_positions_not_bytes`).
3. **A middle-click that isn't a click on one tab:** pressed on one tab and released on another, a release with no press, or the tab under the pointer changing between press and release. None of them closes anything. Pinned in Task 3 (`a_middle_click_closes_a_clean_background_tab_and_keeps_the_active_one`).
4. **Keys aimed at the palette:** Ctrl+P with the picker already open keeps the typed query and rows; Ctrl+W typed in the palette's field closes the palette, never the tab behind it (the accelerator table sees the key before the field's hook). Pinned in Task 4 (`ctrl_p_again_keeps_the_query_and_a_query_of_spaces_lists_the_open_tabs`) and Task 3 (`ctrl_w_closes_the_active_tab_and_in_the_palette_field_closes_the_palette`).
5. **The world changing under an open picker:** a tab closed while its row is listed still opens from that row (from disk, as a new tab), and a note removed from the library between listing and Enter opens nothing and shows the open-failure notice. Pinned in Task 4 (`a_tab_closed_while_the_picker_is_open_still_opens_from_its_row`, `a_note_removed_from_the_library_after_listing_reports_it_and_opens_nothing`).

## File Map

| File | Responsibility | Task |
|---|---|---|
| `src/library/quick_open.rs` (new), `src/library/name_search.rs` (deleted), `src/library/mod.rs`, `src/library/text_search.rs`, `src/bin/fastpad-bench.rs`, `benchmarks/README.md` | `QuickMatch`, `QuickMatch::plain`, `search`, `split_line`, `folder_of` (moved); bench `quick_open_ms` | 1 |
| `src/window/tabs.rs`, `src/window/main_window.rs` | `Tabs` activation order (`activation_order`, `reset_activation_order`), updated by activate/push/close/replace; restore resets it | 2 |
| `src/window/tabs.rs`, `src/window/menus.rs`, `src/window/command_palette.rs`, `src/window/main_window.rs`, `src/app.rs` | `Tabs::close_clean_background`; Ctrl+W accelerator and File menu shortcut text; Ctrl+W in the palette field closes the palette; `close_tab_at`, `close_background_document`, `WM_MBUTTONDOWN`/`WM_MBUTTONUP`, `App.middle_press` | 3 |
| `src/window/commands.rs`, `src/window/menus.rs`, `src/window/command_palette.rs`, `src/window/main_window.rs`, `src/window/library_host.rs`, `src/editor/scintilla.rs`, `src/editor/scintilla_constants.rs`, `src/window/tabs.rs` | `CommandId::QuickOpen`, the palette row, the File menu item, Ctrl+P, `PickerKind::QuickOpen`, the new `PickerRow`/`PickerChoice` variants, `quick_open_rows`, `open_quick_open`, `open_quick_open_choice`, `go_to_line`, `Editor::go_to_line` | 4 |
| `src/window/command_palette.rs`, `src/window/main_window.rs`, `README.md`, the spec (§9) | Owner-draw rows (bold hits, muted folder), the painted hint, list-box strings for screen readers; docs | 5 |

---

### Task 1: `quick_open` matcher and `split_line`, replacing `name_search`

**Files:**
- Create: `src/library/quick_open.rs`
- Delete: `src/library/name_search.rs`
- Modify: `src/library/mod.rs` (the `pub mod` list), `src/library/text_search.rs` (line 6), `src/bin/fastpad-bench.rs` (lines 335, 412–416, 426, 443), `benchmarks/README.md` (line 80)

**Interfaces:**
- Consumes: `super::tree::natural_cmp(&str, &str) -> Ordering`.
- Produces:
  - `#[derive(Clone, Debug, Eq, PartialEq)] pub struct QuickMatch { pub path: PathBuf, pub name: String, pub folder: String, pub name_hits: Vec<usize>, pub folder_hits: Vec<usize> }` — hits are ascending char indices into `name` and `folder`.
  - `impl QuickMatch { pub fn plain(path: &Path) -> Option<Self> }` — a row with no hits.
  - `pub fn search<'a, P>(notes: impl IntoIterator<Item = &'a P>, query: &str, limit: usize) -> Vec<QuickMatch> where P: AsRef<Path> + ?Sized + 'a`
  - `pub fn split_line(query: &str) -> (&str, Option<u32>)`
  - `pub(super) fn folder_of(path: &Path) -> String` (moved unchanged from `name_search`).

**Decisions this task settles:**
- **Case folding** maps each char to the first char of its lowercase form (`İ` → `i`), so a char never becomes two and every position stays a char index of the original text.
- **Targets:** the name (the file stem) alone; then, only for a term that fails on the name and only when the note has a folder, `folder\name`. A term that matches the name is a name match (+20). The `\` joining folder and name is never reported as a hit.
- **Best alignment** is a dynamic program over (term letter, target char): `score[j][i]` is the best score of the first `j + 1` letters with letter `j` on char `i`; the predecessor is either the char right before (`+5`) or the best one at least two before. Scores are ≥ 1 per matched char, so 0 means "no alignment". Ties keep the leftmost end.
- **`split_line`:** trailing spaces are ignored; `a:` drops the colon (`("a", None)`), so the list doesn't empty while the number is being typed; `a:x` is text; a number too large for `u32` is `u32::MAX` (the last line).

- [ ] **Step 1: Write the failing tests**

Create `src/library/quick_open.rs` with only the tests for now:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn paths(names: &[&str]) -> Vec<PathBuf> {
        names.iter().map(PathBuf::from).collect()
    }

    fn names(matches: &[QuickMatch]) -> Vec<&str> {
        matches.iter().map(|found| found.name.as_str()).collect()
    }

    #[test]
    fn a_term_matches_when_its_letters_appear_in_order_ignoring_case() {
        // Break caught: a substring-only matcher ("nt" finding nothing in "note"), letters taken
        // out of order ("tn" finding "note"), or the extension searched.
        let notes = paths(&["note.md", "tone.md", "Readme"]);
        assert_eq!(names(&search(&notes, "nt", 50)), ["note"]);
        assert_eq!(names(&search(&notes, "NT", 50)), ["note"]);
        assert_eq!(names(&search(&notes, "tn", 50)), ["tone"]);
        assert_eq!(names(&search(&notes, "rdm", 50)), ["Readme"]);
        assert!(
            search(&notes, "md", 50).is_empty(),
            "the extension is not searched"
        );
    }

    #[test]
    fn every_term_must_match_in_the_name_or_the_folder_path() {
        // Break caught: terms ORed (every note with "al" listed), a term that only matches the
        // folder dropping the note, or a query of spaces listing every note.
        let notes = paths(&[
            "alpha beta.md",
            "alpha.md",
            "beta.md",
            r"alpha\gamma beta.md",
        ]);
        let found = search(&notes, "al be", 50);
        assert_eq!(names(&found), ["alpha beta", "gamma beta"]);
        assert_eq!(found[1].folder, "alpha");
        assert!(search(&notes, "   ", 50).is_empty());
        assert!(search(&notes, "al zz", 50).is_empty());
        assert!(search(&notes, "al", 0).is_empty());
    }

    #[test]
    fn a_name_match_comes_before_a_higher_scoring_folder_match() {
        // Break caught: "meet" listing every note of a folder called "meet" above the note whose
        // own name holds the letters.
        let notes = paths(&[r"meet\zz.md", "xmxexet.md"]);
        assert_eq!(names(&search(&notes, "meet", 50)), ["xmxexet", "zz"]);
    }

    #[test]
    fn contiguous_and_word_start_letters_beat_scattered_ones() {
        // Break caught: plain in-order matching that ranks "xaxbxc" level with "abc" typed as a
        // word, so the note the user meant sinks under the noise.
        let notes = paths(&["xaxbxc.md", "xxabcxx.md", "zz-a-b-c.md"]);
        assert_eq!(
            names(&search(&notes, "abc", 50)),
            ["zz-a-b-c", "xxabcxx", "xaxbxc"]
        );
        let camel = paths(&["xaxbxc.md", "zzAxBxCx.md"]);
        assert_eq!(names(&search(&camel, "abc", 50)), ["zzAxBxCx", "xaxbxc"]);
    }

    #[test]
    fn hits_are_char_positions_in_the_name_and_the_folder() {
        // Break caught (review focus 2): byte offsets reported as char positions, so "Über
        // Straße" bolds the wrong letters; a folder match's hits left in the name; or the `\`
        // joining folder and name counted as a hit.
        let notes = paths(&["Über Straße.md"]);
        let found = search(&notes, "st", 50);
        assert_eq!(found[0].name_hits, [5, 6]);
        assert!(found[0].folder_hits.is_empty());
        assert_eq!(search(&notes, "ÜB", 50)[0].name_hits, [0, 1]);

        let notes = paths(&[r"Work\2026\meeting notes.md"]);
        let found = search(&notes, "w26 mn", 50);
        assert_eq!(found[0].folder, r"Work\2026");
        assert_eq!(found[0].folder_hits, [0, 5, 8]);
        assert_eq!(found[0].name_hits, [0, 8]);
    }

    #[test]
    fn at_most_the_limit_best_matches_are_kept() {
        // Break caught: a one-letter query listing all 300 notes, or the cap keeping arbitrary
        // notes instead of the best (the shorter names, then natural order).
        let many: Vec<PathBuf> = (0..300)
            .map(|index| PathBuf::from(format!("n{index}.md")))
            .collect();
        let found = search(&many, "n", 50);
        assert_eq!(found.len(), 50);
        assert_eq!(found[0].name, "n0");
        assert_eq!(found[9].name, "n9");
        assert_eq!(found[10].name, "n10");
        assert_eq!(found[49].name, "n49");
    }

    #[test]
    fn a_trailing_colon_and_digits_is_a_line() {
        // Break caught (review focus 1): "meet:42" matched as text (nothing found), a colon
        // further in cutting the text short, ":12" offering notes instead of the line, a
        // trailing colon emptying the list while the number is typed, or a huge number panicking.
        assert_eq!(split_line("a:12"), ("a", Some(12)));
        assert_eq!(split_line(":12"), ("", Some(12)));
        assert_eq!(split_line("a:"), ("a", None));
        assert_eq!(split_line("a:x"), ("a:x", None));
        assert_eq!(split_line("12"), ("12", None));
        assert_eq!(split_line("a:b:3"), ("a:b", Some(3)));
        assert_eq!(split_line("meet:42  "), ("meet", Some(42)));
        assert_eq!(split_line("a:99999999999"), ("a", Some(u32::MAX)));
        assert_eq!(split_line("   "), ("", None));
        assert!(search(&paths(&["a.md", "b.md"]), split_line("a:b").0, 50).is_empty());
    }

    #[test]
    fn plain_rows_name_the_note_and_its_folder_with_nothing_matched() {
        // Break caught: the open-tab rows shown before anything is typed losing their folder, or
        // carrying stale highlights.
        let plain = QuickMatch::plain(Path::new(r"a\b\x.md")).unwrap();
        assert_eq!((plain.name.as_str(), plain.folder.as_str()), ("x", r"a\b"));
        assert!(plain.name_hits.is_empty() && plain.folder_hits.is_empty());
        assert_eq!(QuickMatch::plain(Path::new("x.md")).unwrap().folder, "");
    }

    #[test]
    fn ten_thousand_notes_match_quickly() {
        // Break caught: a keystroke that allocates or sorts per note so heavily that typing in
        // Ctrl+P lags on a big notebook (spec §3.6).
        let notes: Vec<PathBuf> = (0..10_000)
            .map(|index| PathBuf::from(format!(r"batch{}\note{index}.md", index / 500)))
            .collect();
        let started = std::time::Instant::now();
        let found = search(&notes, "nt 12", 50);
        let elapsed = started.elapsed();
        assert_eq!(found.len(), 50);
        assert_eq!(found[0].name, "note12");
        assert_eq!(found[0].name_hits, [0, 2, 4, 5]);
        if !cfg!(debug_assertions) {
            assert!(
                elapsed < std::time::Duration::from_millis(20),
                "{elapsed:?}"
            );
        }
    }
}
```

In `src/library/mod.rs`, replace `pub mod name_search;` with nothing and add `pub mod quick_open;` right after `pub mod ops;`. In `src/library/text_search.rs` line 6, replace `use super::name_search::folder_of;` with `use super::quick_open::folder_of;`. Delete `src/library/name_search.rs` (`git rm src/library/name_search.rs`).

- [ ] **Step 2: Run the tests and see them fail**

Run: `cargo test --lib library::quick_open`
Expected: compile errors, `cannot find function 'search' in this scope`, `cannot find type 'QuickMatch'`, `cannot find function 'folder_of' in module 'super::quick_open'`.

- [ ] **Step 3: Implement**

Put this above the tests in `src/library/quick_open.rs`:

```rust
//! Quick open (Ctrl+P, quick-open spec §3.3–3.4): fuzzy matching of note names and folders the
//! way VS Code's Ctrl+P matches files, and the `:<line>` suffix. Pure: no Win32 and no disk.

use super::tree::natural_cmp;
use std::cmp::Ordering;
use std::path::{Path, PathBuf};

/// Every matched char.
const PER_CHAR: u32 = 1;
/// A char right after the previous matched one.
const CONTIGUOUS: u32 = 5;
/// A char that starts a word.
const WORD_START: u32 = 8;
/// The target's first char, on top of its word start.
const FIRST: u32 = 4;
/// Each term matched in the name rather than in the folder path.
const NAME_MATCH: u32 = 20;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QuickMatch {
    /// As given: relative to the notebook.
    pub path: PathBuf,
    /// The file name without its extension, as the tree shows it.
    pub name: String,
    /// The folder the note is in, relative to the notebook and joined with `\`; `""` at the root.
    pub folder: String,
    /// The matched chars of `name`, as ascending char indices (not byte offsets).
    pub name_hits: Vec<usize>,
    /// The matched chars of `folder`, as ascending char indices.
    pub folder_hits: Vec<usize>,
}

impl QuickMatch {
    /// `path` as a row with nothing matched: how the open tabs are listed before anything is
    /// typed. `None` for a path with no file name.
    pub fn plain(path: &Path) -> Option<Self> {
        Some(Self {
            path: path.to_path_buf(),
            name: path.file_stem()?.to_string_lossy().into_owned(),
            folder: folder_of(path),
            name_hits: Vec::new(),
            folder_hits: Vec::new(),
        })
    }
}

/// Splits a trailing `:<digits>` off `query`, trailing spaces ignored: the text to match and
/// the 1-based line. A trailing `:` with no digits yet is dropped too, so the list doesn't empty
/// while the number is typed; anything else after the last `:` is text (`a:x`). A line too
/// large for `u32` is `u32::MAX`, which goes to the last line.
pub fn split_line(query: &str) -> (&str, Option<u32>) {
    let query = query.trim_end();
    let Some(colon) = query.rfind(':') else {
        return (query, None);
    };
    let digits = &query[colon + 1..];
    if digits.is_empty() {
        return (&query[..colon], None);
    }
    if !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return (query, None);
    }
    (&query[..colon], Some(digits.parse().unwrap_or(u32::MAX)))
}

/// One char of a match target.
#[derive(Clone, Copy)]
struct Unit {
    /// The char folded to one lowercase char (`fold`).
    folded: char,
    /// Whether a word starts here (spec §3.3).
    start: bool,
}

/// `c` in lowercase as one char: the first char of its lowercase form, so a char never becomes
/// two (`İ` lowercases to `i` plus a combining dot) and positions stay char indices.
fn fold(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

/// `chars` as match targets. A word starts at the first char, after a space, `-`, `_`, `.`,
/// `\` or `/`, and at an uppercase letter after a lowercase one.
fn units(chars: impl IntoIterator<Item = char>, out: &mut Vec<Unit>) {
    out.clear();
    let mut previous: Option<char> = None;
    for c in chars {
        let start = previous.is_none_or(|previous| {
            matches!(previous, ' ' | '-' | '_' | '.' | '\\' | '/')
                || (c.is_uppercase() && previous.is_lowercase())
        });
        out.push(Unit {
            folded: fold(c),
            start,
        });
        previous = Some(c);
    }
}

/// The best score of `term`'s letters, in order, in `target`, or 0 when they don't all appear.
/// `score[j * n + i]` is the best score of the first `j + 1` letters with letter `j` on char `i`
/// (0: impossible), and `from[j * n + i]` the char letter `j - 1` sits on then. With
/// `positions`, also records where each letter of the best alignment landed.
fn align(
    term: &[char],
    target: &[Unit],
    score: &mut Vec<u32>,
    from: &mut Vec<u32>,
    positions: Option<&mut Vec<usize>>,
) -> u32 {
    let (letters, n) = (term.len(), target.len());
    if letters == 0 || letters > n {
        return 0;
    }
    score.clear();
    score.resize(letters * n, 0);
    from.clear();
    from.resize(letters * n, 0);
    for (j, &letter) in term.iter().enumerate() {
        // The best score of the previous letter on a char at least two before `i`, and where.
        let (mut before, mut before_at) = (0, 0);
        for (i, unit) in target.iter().enumerate() {
            if j > 0 && i >= 2 {
                let candidate = score[(j - 1) * n + i - 2];
                if candidate > before {
                    (before, before_at) = (candidate, i - 2);
                }
            }
            if unit.folded != letter {
                continue;
            }
            let own = PER_CHAR
                + if unit.start { WORD_START } else { 0 }
                + if i == 0 { FIRST } else { 0 };
            if j == 0 {
                score[i] = own;
                continue;
            }
            let adjacent = if i >= 1 { score[(j - 1) * n + i - 1] } else { 0 };
            let (previous, at) = if adjacent > 0 && adjacent + CONTIGUOUS >= before {
                (adjacent + CONTIGUOUS, i - 1)
            } else {
                (before, before_at)
            };
            if previous > 0 {
                score[j * n + i] = previous + own;
                from[j * n + i] = at as u32;
            }
        }
    }
    let (mut end, mut best) = (0, 0);
    for (i, &value) in score[(letters - 1) * n..].iter().enumerate() {
        if value > best {
            (end, best) = (i, value);
        }
    }
    if best > 0
        && let Some(positions) = positions
    {
        positions.clear();
        positions.resize(letters, 0);
        let mut at = end;
        for (j, slot) in positions.iter_mut().enumerate().rev() {
            *slot = at;
            if j > 0 {
                at = from[j * n + at] as usize;
            }
        }
    }
    best
}

/// Buffers reused from note to note, so a keystroke doesn't allocate per note for matching.
#[derive(Default)]
struct Scratch {
    name: Vec<char>,
    folder: Vec<char>,
    name_units: Vec<Unit>,
    path_units: Vec<Unit>,
    score: Vec<u32>,
    from: Vec<u32>,
    positions: Vec<usize>,
}

impl Scratch {
    /// Whether every term matched in the name, and the summed score; `None` when a term matches
    /// neither the name nor `folder\name`. Leaves the note's name and folder chars in `name` and
    /// `folder`. With `hits`, also collects the name's and the folder's matched chars.
    fn score(
        &mut self,
        terms: &[Vec<char>],
        path: &Path,
        mut hits: Option<(&mut Vec<usize>, &mut Vec<usize>)>,
    ) -> Option<(bool, u32)> {
        let stem = path.file_stem()?;
        self.name.clear();
        self.name.extend(stem.to_string_lossy().chars());
        self.folder.clear();
        if let Some(parent) = path.parent() {
            for (index, component) in parent.components().enumerate() {
                if index > 0 {
                    self.folder.push('\\');
                }
                self.folder
                    .extend(component.as_os_str().to_string_lossy().chars());
            }
        }
        units(self.name.iter().copied(), &mut self.name_units);
        let keep = hits.is_some();
        let mut path_ready = false;
        let (mut all_name, mut total) = (true, 0);
        for term in terms {
            let positions = keep.then_some(&mut self.positions);
            let in_name = align(
                term,
                &self.name_units,
                &mut self.score,
                &mut self.from,
                positions,
            );
            if in_name > 0 {
                total += in_name + NAME_MATCH;
                if let Some((name_hits, _)) = hits.as_mut() {
                    name_hits.extend_from_slice(&self.positions);
                }
                continue;
            }
            if self.folder.is_empty() {
                return None;
            }
            if !path_ready {
                units(
                    self.folder
                        .iter()
                        .copied()
                        .chain(std::iter::once('\\'))
                        .chain(self.name.iter().copied()),
                    &mut self.path_units,
                );
                path_ready = true;
            }
            let positions = keep.then_some(&mut self.positions);
            let in_path = align(
                term,
                &self.path_units,
                &mut self.score,
                &mut self.from,
                positions,
            );
            if in_path == 0 {
                return None;
            }
            all_name = false;
            total += in_path;
            if let Some((name_hits, folder_hits)) = hits.as_mut() {
                let split = self.folder.len();
                for &position in &self.positions {
                    match position.cmp(&split) {
                        Ordering::Less => folder_hits.push(position),
                        // The `\` joining the folder to the name is in neither.
                        Ordering::Equal => {}
                        Ordering::Greater => name_hits.push(position - split - 1),
                    }
                }
            }
        }
        if let Some((name_hits, folder_hits)) = hits {
            for list in [name_hits, folder_hits] {
                list.sort_unstable();
                list.dedup();
            }
        }
        Some((all_name, total))
    }
}

struct Candidate<'a> {
    /// Every term matched in the name.
    all_name: bool,
    score: u32,
    /// The name's length in chars.
    name_len: usize,
    name: String,
    folder: String,
    path: &'a Path,
}

/// Name matches first, then the higher score, the shorter name, natural name order, natural
/// folder order (the root first) and the exact path (spec §3.3).
fn rank(a: &Candidate<'_>, b: &Candidate<'_>) -> Ordering {
    b.all_name
        .cmp(&a.all_name)
        .then_with(|| b.score.cmp(&a.score))
        .then_with(|| a.name_len.cmp(&b.name_len))
        .then_with(|| natural_cmp(&a.name, &b.name))
        .then_with(|| natural_cmp(&a.folder, &b.folder))
        .then_with(|| a.path.cmp(b.path))
}

/// The notes matching every space-separated term of `query`, best first, at most `limit`, each
/// with its matched chars. `query` has no line suffix here: `split_line` took it off.
pub fn search<'a, P>(
    notes: impl IntoIterator<Item = &'a P>,
    query: &str,
    limit: usize,
) -> Vec<QuickMatch>
where
    P: AsRef<Path> + ?Sized + 'a,
{
    let terms: Vec<Vec<char>> = query
        .split_whitespace()
        .map(|term| term.chars().map(fold).collect())
        .collect();
    if terms.is_empty() || limit == 0 {
        return Vec::new();
    }
    let mut scratch = Scratch::default();
    let mut found: Vec<Candidate<'_>> = Vec::new();
    for path in notes {
        let path = path.as_ref();
        let Some((all_name, score)) = scratch.score(&terms, path, None) else {
            continue;
        };
        found.push(Candidate {
            all_name,
            score,
            name_len: scratch.name.len(),
            name: scratch.name.iter().collect(),
            folder: scratch.folder.iter().collect(),
            path,
        });
    }
    // Only the best `limit` need a full sort: one letter can match every note.
    if found.len() > limit {
        found.select_nth_unstable_by(limit, rank);
        found.truncate(limit);
    }
    found.sort_by(rank);
    found
        .into_iter()
        .map(|candidate| {
            let (mut name_hits, mut folder_hits) = (Vec::new(), Vec::new());
            // The same alignment again, this time keeping where each letter landed.
            let _ = scratch.score(
                &terms,
                candidate.path,
                Some((&mut name_hits, &mut folder_hits)),
            );
            QuickMatch {
                path: candidate.path.to_path_buf(),
                name: candidate.name,
                folder: candidate.folder,
                name_hits,
                folder_hits,
            }
        })
        .collect()
}

/// The folder `path` is in, relative to the notebook and joined with `\`; `""` at the root.
pub(super) fn folder_of(path: &Path) -> String {
    path.parent()
        .map(|parent| {
            parent
                .components()
                .map(|component| component.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("\\")
        })
        .unwrap_or_default()
}
```

In `src/bin/fastpad-bench.rs`:
- Replace line 335 `const NAME_SEARCH_REFERENCE_MS: f64 = 5.0;` with:

```rust
/// The quick-open spec's §3.6 budget for one Ctrl+P keystroke.
const QUICK_OPEN_REFERENCE_MS: f64 = 5.0;
```

- Replace

```rust
    let name_search_ms = median_ms(|| {
        std::hint::black_box(fastpad::library::name_search::search(
            &paths, "note 12", 500,
        ));
    });
```

with

```rust
    let quick_open_ms = median_ms(|| {
        std::hint::black_box(fastpad::library::quick_open::search(&paths, "nt 12", 50));
    });
```

- Replace `    println!("name_search_ms={name_search_ms:.2}");` with `    println!("quick_open_ms={quick_open_ms:.2}");`.
- Replace `            ("name search", name_search_ms, NAME_SEARCH_REFERENCE_MS),` with `            ("quick open", quick_open_ms, QUICK_OPEN_REFERENCE_MS),`.

In `benchmarks/README.md`, replace line 80 `- \`name_search_ms\`: one Search-view keystroke over every name. The target is under 5 ms.` with:

```markdown
- `quick_open_ms`: one Ctrl+P keystroke (`nt 12`) scored over every note. The target is under
  5 ms.
```

- [ ] **Step 4: Run the tests and see them pass**

Run: `cargo test --lib library::quick_open`
Expected: 9 passed.
Run: `cargo test --lib library::text_search`
Expected: all pass (only the import moved).

- [ ] **Step 5: Bench**

Run (PowerShell):
```powershell
$bench = Join-Path $env:TEMP "fastpad-quick-open-bench"
Remove-Item -Recurse -Force $bench -ErrorAction SilentlyContinue
cargo run --release --bin fastpad-bench -- library-scan $bench --count 10000 --enforce-reference
Remove-Item -Recurse -Force $bench
```
Expected: a `quick_open_ms=` line well under 5.00, and exit code 0.

- [ ] **Step 6: Clippy**

Run: `cargo fmt; cargo clippy --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 7: Commit**

```bash
git add src/library/quick_open.rs src/library/mod.rs src/library/text_search.rs src/bin/fastpad-bench.rs benchmarks/README.md
git status --short src/library   # name_search.rs shows as deleted (D), staged by Step 1's git rm
git commit -m "feat(library): quick_open fuzzy note matcher with char-index hits and a :line suffix, replacing name_search; bench quick_open_ms under 5 ms"
```

---

### Task 2: `Tabs` activation order

**Files:**
- Modify: `src/window/tabs.rs` (`struct Tabs`, `new`, `with_document`, `from_documents`, `replace_active_untitled`, `activate`, `push`, `close_active`, `select_after_removal`, `close_reviewed`, `replace_preview`, `clear_for_shutdown`; two new methods; `mod tests`)
- Modify: `src/window/main_window.rs` (`finish_session_restore`; one test)

**Interfaces:**
- Consumes: `DocumentId` (`Copy`, `Eq`).
- Produces:
  - `pub(crate) fn activation_order(&self) -> &[DocumentId]` — every tab, most recently activated first; the active tab is always first when there is one. Marked `#[cfg_attr(not(test), expect(dead_code, ...))]` until Task 4 reads it.
  - `pub(crate) fn reset_activation_order(&mut self)` — the active tab first, then the strip order.
  - Private `fn touch(&mut self, id: DocumentId)`; `select_after_removal(&mut self, removed: usize, closed: DocumentId)` (new parameter).

**Decisions this task settles:**
- The tab that takes a closed active tab's place becomes active, so it moves to the front. This keeps "the active tab is first", which Task 4's empty-query selection relies on.
- A document replaced in place (the preview, a reused untitled tab) leaves the order and its replacement takes the front.

- [ ] **Step 1: Write the failing tests**

In `src/window/tabs.rs`'s `mod tests`, add after `use std::fs;`:

```rust
    use crate::document::CloseDecision;

    fn order(tabs: &Tabs) -> Vec<u64> {
        tabs.activation_order().iter().map(|id| id.0).collect()
    }
```

and these tests at the end of the module:

```rust
    #[test]
    fn activating_or_opening_a_tab_moves_it_to_the_front_of_the_activation_order() {
        // Break caught: Ctrl+P listing tabs in strip order, so Ctrl+P then Enter doesn't go back
        // to the previous note, or a new tab missing from the list.
        let mut tabs = Tabs::with_document(document(1));
        tabs.push(document(2)).unwrap();
        tabs.push(document(3)).unwrap();
        assert_eq!(order(&tabs), [3, 2, 1]);
        tabs.activate(DocumentId(1)).unwrap();
        assert_eq!(order(&tabs), [1, 3, 2]);
        tabs.activate_index(2).unwrap();
        assert_eq!(order(&tabs), [3, 1, 2]);
        tabs.activate(DocumentId(3)).unwrap();
        assert_eq!(order(&tabs), [3, 1, 2], "the active tab stays first");
    }

    #[test]
    fn a_closed_tab_leaves_the_order_and_the_tab_taking_its_place_comes_first() {
        // Break caught: Ctrl+P offering a closed tab's dead document, or leaving the tab now on
        // screen second, so Ctrl+P then Enter re-selects the tab already shown.
        let mut tabs = Tabs::with_document(document(1));
        tabs.push(document(2)).unwrap();
        tabs.push(document(3)).unwrap();
        tabs.activate(DocumentId(2)).unwrap();
        assert_eq!(order(&tabs), [2, 3, 1]);
        tabs.close_active(CloseDecision::Discard).unwrap();
        assert_eq!(tabs.active().unwrap().id, DocumentId(3));
        assert_eq!(order(&tabs), [3, 1]);
        let review = tabs.active_close_review().unwrap();
        tabs.close_reviewed(review, CloseDecision::Discard).unwrap();
        assert_eq!(order(&tabs), [1]);
    }

    #[test]
    fn a_tab_replaced_in_place_takes_the_front_and_the_old_document_leaves_the_order() {
        // Break caught: a replaced preview (or reused untitled tab) still listed by Ctrl+P under
        // its old document, or the note now in it missing.
        let mut tabs = Tabs::with_document(document(1));
        tabs.push(preview(2)).unwrap();
        tabs.push(document(3)).unwrap();
        tabs.replace_preview(preview(4)).unwrap();
        assert_eq!(order(&tabs), [4, 3, 1]);
        tabs.activate(DocumentId(1)).unwrap();
        tabs.replace_active_untitled(document(5)).unwrap();
        assert_eq!(order(&tabs), [5, 4, 3]);
        tabs.clear_for_shutdown();
        assert!(order(&tabs).is_empty());
    }

    #[test]
    fn restored_tabs_restart_the_order_from_the_strip_with_the_active_tab_first() {
        // Break caught: after a session restore, the tabs listed last-restored first (each one
        // entered at the front as it opened).
        let mut tabs = Tabs::with_document(document(1));
        tabs.push(document(2)).unwrap();
        tabs.push(document(3)).unwrap();
        tabs.push(document(4)).unwrap();
        tabs.activate(DocumentId(3)).unwrap();
        tabs.reset_activation_order();
        assert_eq!(order(&tabs), [3, 1, 2, 4]);
        let from = Tabs::from_documents([document(5), document(6)]).unwrap();
        assert_eq!(order(&from), [5, 6]);
    }
```

In `src/window/main_window.rs`'s tests, right after `fn session_restore_reopens_files_and_unsaved_text_in_order`, add:

```rust
    #[test]
    fn restored_tabs_enter_the_activation_order_in_strip_order_with_the_active_tab_first() {
        // Break caught: the restore wiring missing, so Ctrl+P after a restart lists the tabs in
        // the reverse order they reopened in, not the saved active tab first.
        let _scintilla = load_native_scintilla();
        let scratch = RecoveryScratch::new("session-activation-order");
        let files = ["one.txt", "two.txt", "three.txt"].map(|name| {
            let path = scratch.path().join(name);
            std::fs::write(&path, name).unwrap();
            path
        });
        write_session(
            &scratch,
            files
                .iter()
                .map(|path| SessionEntry::new(SessionSource::File(path.clone())))
                .collect(),
            1,
        );
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        enable_session(window.hwnd, &scratch);

        run_session_restore(window.hwnd);

        let app = app_mut(window.hwnd);
        let paths = app
            .tabs
            .activation_order()
            .iter()
            .map(|&id| app.tabs.document(id).unwrap().path.clone().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            paths,
            [files[1].clone(), files[0].clone(), files[2].clone()]
        );
    }
```

- [ ] **Step 2: Run the tests and see them fail**

Run: `cargo test --lib window::tabs`
Expected: compile error, `no method named 'activation_order' found for reference '&Tabs'`.

- [ ] **Step 3: Implement**

In `src/window/tabs.rs`:

Replace the struct:

```rust
#[derive(Debug)]
pub struct Tabs {
    documents: Vec<Document>,
    /// Every tab, the most recently activated first; the active tab is always first
    /// (quick-open spec §3.2). Kept in memory only, never saved.
    recent: Vec<DocumentId>,
    selection: TabSelection,
    view: TabView,
}
```

Replace `new`, `with_document` and `from_documents` with:

```rust
    pub fn new() -> Self {
        let documents = Vec::new();
        Self {
            view: TabView::new(&documents),
            recent: Vec::new(),
            documents,
            selection: TabSelection::new(0),
        }
    }

    pub fn with_document(document: Document) -> Self {
        let documents = vec![document];
        Self {
            view: TabView::new(&documents),
            recent: documents.iter().map(|document| document.id).collect(),
            documents,
            selection: TabSelection::new(0),
        }
    }

    pub fn from_documents(
        documents: impl IntoIterator<Item = Document>,
    ) -> Result<Self, DuplicateDocumentPath> {
        let documents = documents.into_iter().collect::<Vec<_>>();
        validate_unique_paths(&documents)?;
        Ok(Self {
            view: TabView::new(&documents),
            recent: documents.iter().map(|document| document.id).collect(),
            documents,
            selection: TabSelection::new(0),
        })
    }
```

Replace `replace_active_untitled` with:

```rust
    pub(crate) fn replace_active_untitled(&mut self, document: Document) -> Option<Document> {
        let index = self.active_index();
        let active = self.documents.get_mut(index)?;
        let old = std::mem::replace(active, document);
        let id = active.id;
        self.recent.retain(|recent| *recent != old.id);
        self.touch(id);
        self.view.update(&self.documents);
        Some(old)
    }
```

Replace `activate` with:

```rust
    pub fn activate(&mut self, id: DocumentId) -> Result<(), UnknownDocument> {
        let index = self
            .documents
            .iter()
            .position(|document| document.id == id)
            .ok_or(UnknownDocument(id))?;
        self.selection.select(index, self.documents.len());
        self.touch(id);
        Ok(())
    }

    /// Moves `id` to the front of the activation order.
    fn touch(&mut self, id: DocumentId) {
        self.recent.retain(|recent| *recent != id);
        self.recent.insert(0, id);
    }

    /// The tabs, the most recently activated first; the active tab leads (spec §3.2).
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "the quick-open picker reads it (quick-open plan, Task 4)")
    )]
    pub(crate) fn activation_order(&self) -> &[DocumentId] {
        &self.recent
    }

    /// Restarts the activation order from the strip, the active tab first: how restored tabs
    /// enter it once a session restore has reopened them all (spec §3.2).
    pub(crate) fn reset_activation_order(&mut self) {
        let active = self.active().map(|document| document.id);
        self.recent = active
            .into_iter()
            .chain(
                self.documents
                    .iter()
                    .map(|document| document.id)
                    .filter(|id| Some(*id) != active),
            )
            .collect();
    }
```

In `push`, replace

```rust
        self.documents.push(document);
        let index = self.documents.len() - 1;
        self.selection.select(index, self.documents.len());
        self.view.update(&self.documents);
        Ok(())
```

with

```rust
        let id = document.id;
        self.documents.push(document);
        let index = self.documents.len() - 1;
        self.selection.select(index, self.documents.len());
        self.touch(id);
        self.view.update(&self.documents);
        Ok(())
```

In `close_active`, replace

```rust
        let closed = self.documents.remove(index);
        self.select_after_removal(index);
        Ok(closed)
    }

    /// Keeps the successor of a removed tab selected (or its predecessor at the end of the strip).
    fn select_after_removal(&mut self, removed: usize) {
        let active = removed.min(self.documents.len().saturating_sub(1));
        self.selection.active.store(active, Ordering::Release);
        self.view.update(&self.documents);
    }
```

with

```rust
        let closed = self.documents.remove(index);
        self.select_after_removal(index, closed.id);
        Ok(closed)
    }

    /// Keeps the successor of a removed tab selected (or its predecessor at the end of the strip).
    /// `closed` leaves the activation order and the tab now selected takes its front.
    fn select_after_removal(&mut self, removed: usize, closed: DocumentId) {
        let active = removed.min(self.documents.len().saturating_sub(1));
        self.selection.active.store(active, Ordering::Release);
        self.recent.retain(|recent| *recent != closed);
        if let Some(id) = self.documents.get(active).map(|document| document.id) {
            self.touch(id);
        }
        self.view.update(&self.documents);
    }
```

In `close_reviewed`, replace

```rust
        let closed = self.documents.remove(index);
        self.select_after_removal(index);
        Ok(closed)
```

with

```rust
        let closed = self.documents.remove(index);
        self.select_after_removal(index, closed.id);
        Ok(closed)
```

In `replace_preview`, replace

```rust
            Some(index) if !self.documents[index].dirty => {
                let old = std::mem::replace(&mut self.documents[index], document);
                self.selection.select(index, self.documents.len());
                self.view.update(&self.documents);
                Some(old)
            }
```

with

```rust
            Some(index) if !self.documents[index].dirty => {
                let id = document.id;
                let old = std::mem::replace(&mut self.documents[index], document);
                self.selection.select(index, self.documents.len());
                self.recent.retain(|recent| *recent != old.id);
                self.touch(id);
                self.view.update(&self.documents);
                Some(old)
            }
```

Replace `clear_for_shutdown` with:

```rust
    pub fn clear_for_shutdown(&mut self) {
        self.documents.clear();
        self.recent.clear();
        self.selection.active.store(0, Ordering::Release);
        self.view.update(&self.documents);
    }
```

In `src/window/main_window.rs`'s `finish_session_restore`, replace

```rust
    if !identity.is_live_for(hwnd) {
        return;
    }
    if restore.failed > 0 {
```

with

```rust
    if !identity.is_live_for(hwnd) {
        return;
    }
    // Each restored tab entered the activation order at the front as it opened. Restart it from
    // the strip, the active tab first (quick-open spec §3.2).
    if let Some(mut app) = unsafe { app_ptr(hwnd) } {
        unsafe { app.as_mut() }.tabs.reset_activation_order();
    }
    if restore.failed > 0 {
```

- [ ] **Step 4: Run the tests and see them pass**

Run: `cargo test --lib window::tabs`
Expected: all pass, including the 4 new tests.
Run: `cargo test --lib document::tests`
Expected: all pass (they drive `close_active` / `close_reviewed`).
Run: `cargo test --lib window::main_window::tests::restored_tabs_enter_the_activation_order -- --test-threads=1`
Expected: 1 passed.

- [ ] **Step 5: Clippy**

Run: `cargo fmt; cargo clippy --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 6: Commit**

```bash
git add src/window/tabs.rs src/window/main_window.rs
git commit -m "feat(tabs): an in-memory activation order, most recent first, kept by activate, push, close and in-place replacement; a session restore restarts it from the strip with the active tab first"
```

---

### Task 3: Ctrl+W and middle-click close

**Files:**
- Modify: `src/window/tabs.rs` (new method and its test)
- Modify: `src/window/menus.rs` (`accelerator_specs`, File menu, tests)
- Modify: `src/window/command_palette.rs` (`palette_control_proc`, tests)
- Modify: `src/window/main_window.rs` (window procedure arms, `translate_accelerator`, new functions, tests)
- Modify: `src/app.rs` (`App.middle_press`)

**Interfaces:**
- Consumes: `CloseReview { pub id, pub generation }`, `CloseReviewError`, `activate_document_by_id`, `execute_command`, `client_title_target`, `HitTarget::{Tab, CloseTab}`, `crate::recovery::{snapshots_removed_on_close, remove_snapshot_files}`, `refresh_tabs`, `command_palette_owns`, `close_command_palette`.
- Produces:
  - `pub(crate) fn Tabs::close_clean_background(&mut self, review: CloseReview) -> Result<Document, CloseReviewError>`
  - `fn close_tab_at(hwnd: HWND, index: usize)` (main_window, private)
  - `fn close_background_document(hwnd: HWND, identity: &WindowIdentity, review: crate::window::tabs::CloseReview)`
  - `fn tab_id_at(hwnd: HWND, index: usize) -> Option<DocumentId>`
  - `fn palette_keeps_key(hwnd: HWND, message: &windows_sys::Win32::UI::WindowsAndMessaging::MSG) -> bool`
  - `App.middle_press: Option<(usize, crate::document::DocumentId)>`
  - Accelerator `FCONTROL, 'W'` → `CloseTab` (53 entries).

**Decisions this task settles:**
- `TranslateAcceleratorW` runs in the message loop before the palette field's subclass sees `WM_KEYDOWN`, so `translate_accelerator` leaves Ctrl+W alone while the key is aimed at a palette control; the field's hook then closes the palette. The field also swallows the `WM_CHAR` 0x17 that Ctrl+W produces.
- The middle press remembers the tab's index and its document. The release closes only over the same index while it still shows that document. `WM_MOUSELEAVE` (the pointer left the client area, or moved onto a child such as the editor) forgets the press, so a stale press can't close a tab later.
- Tabs answer `HTCLIENT` (confirmed in `titlebar::nonclient_hit_test`: `Tab(_)` and `CloseTab(_)` map to `HTCLIENT`), so the buttons arrive as client `WM_MBUTTON*`. The caption and the logo square are nonclient: nothing changes there.

- [ ] **Step 1: Write the failing tests**

In `src/window/tabs.rs`'s tests, extend the imports with `use super::{CloseReview, CloseReviewError};` and add:

```rust
    #[test]
    fn a_clean_background_tab_closes_where_it_is_and_the_active_tab_stays() {
        // Break caught: a middle-click on another tab switching to it first (the editor flashes),
        // closing the active tab instead, the active index left pointing one tab too far right,
        // or a dirty tab closed without its prompt.
        let mut tabs = Tabs::with_document(document(1));
        tabs.push(document(2)).unwrap();
        tabs.push(document(3)).unwrap();
        let review = |tabs: &Tabs, id: u64| CloseReview {
            id: DocumentId(id),
            generation: tabs.document(DocumentId(id)).unwrap().generation,
        };
        let first = review(&tabs, 1);
        let closed = tabs.close_clean_background(first).unwrap();
        assert_eq!(closed.id, DocumentId(1));
        assert_eq!(tabs.active().unwrap().id, DocumentId(3));
        assert_eq!(tabs.active_index(), 1);
        assert_eq!(order(&tabs), [3, 2]);
        assert_eq!(tabs.view().snapshot().tabs.len(), 2);

        let active = tabs.active_close_review().unwrap();
        assert_eq!(
            tabs.close_clean_background(active),
            Err(CloseReviewError::Stale)
        );
        tabs.document_mut(DocumentId(2)).unwrap().dirty = true;
        let dirty = review(&tabs, 2);
        assert_eq!(
            tabs.close_clean_background(dirty),
            Err(CloseReviewError::Unsaved)
        );
        let stale = CloseReview {
            generation: dirty.generation + 1,
            ..dirty
        };
        assert_eq!(
            tabs.close_clean_background(stale),
            Err(CloseReviewError::Stale)
        );
        assert_eq!(tabs.len(), 2);
    }
```

In `src/window/menus.rs`'s tests, change `assert_eq!(specs.len(), 52);` to `assert_eq!(specs.len(), 53);` and add:

```rust
    #[test]
    fn ctrl_w_closes_the_tab() {
        // Break caught: Ctrl+W unbound, or bound to Close all tabs (quick-open spec §4).
        use windows_sys::Win32::UI::WindowsAndMessaging::FCONTROL;
        let bound = accelerator_specs()
            .into_iter()
            .find(|spec| spec.modifiers == FCONTROL && spec.key == u16::from(b'W'))
            .map(|spec| spec.command);
        assert_eq!(bound, Some(CommandId::CloseTab));
    }
```

In `src/window/command_palette.rs`'s tests, add:

```rust
    #[test]
    fn close_tab_is_listed_with_ctrl_w() {
        // Break caught: the palette's Close tab row still showing no shortcut after Ctrl+W.
        assert_eq!(labels("close tab")[0], "File: Close tab");
        assert_eq!(
            shortcut_text(CommandId::CloseTab).as_deref(),
            Some("Ctrl+W")
        );
    }
```

In `src/window/main_window.rs`'s tests, after `fn a_picker_lists_its_items_and_enter_reports_the_choice`, add:

```rust
    #[test]
    fn ctrl_w_closes_the_active_tab_and_in_the_palette_field_closes_the_palette() {
        // Break caught (review focus 4): Ctrl+W dead, closing a background tab, or, typed in the
        // palette's query field, closing the tab behind the palette (the accelerator table sees
        // the key before the field's hook).
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
            GetKeyboardState, SetKeyboardState, VK_CONTROL,
        };
        use windows_sys::Win32::UI::WindowsAndMessaging::{MSG, WM_KEYDOWN};
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        let identity = unsafe { super::window_identity(window.hwnd).unwrap() };
        execute_command(window.hwnd, CommandId::New);
        execute_command(window.hwnd, CommandId::New);
        let ids = || {
            app_mut(window.hwnd)
                .tabs
                .documents()
                .map(|document| document.id)
                .collect::<Vec<_>>()
        };
        let &[first, second, _] = &ids()[..] else {
            panic!("three tabs")
        };
        let mut keys = [0u8; 256];
        unsafe { GetKeyboardState(keys.as_mut_ptr()) };
        let original = keys;
        keys[VK_CONTROL as usize] = 0x80;
        unsafe { SetKeyboardState(keys.as_ptr()) };
        let ctrl_w = |target: HWND| MSG {
            hwnd: target,
            message: WM_KEYDOWN,
            wParam: usize::from(b'W'),
            ..Default::default()
        };

        let closed =
            unsafe { super::translate_accelerator(window.hwnd, &identity, &ctrl_w(editor.hwnd())) };
        execute_command(window.hwnd, CommandId::CommandPalette);
        let query = with_command_palette(window.hwnd, |palette| palette.query_hwnd()).unwrap();
        let in_palette =
            unsafe { super::translate_accelerator(window.hwnd, &identity, &ctrl_w(query)) };
        unsafe { SendMessageW(query, WM_KEYDOWN, usize::from(b'W'), 0) };
        unsafe { SetKeyboardState(original.as_ptr()) };

        assert!(closed, "Ctrl+W was not translated");
        assert!(!in_palette, "the palette's field keeps Ctrl+W");
        assert!(!with_command_palette(window.hwnd, |palette| palette.is_visible()).unwrap());
        assert_eq!(ids(), [first, second], "only the active tab closed");
    }

    #[test]
    fn a_middle_click_closes_a_clean_background_tab_and_keeps_the_active_one() {
        // Break caught (review focus 3): a middle-click switching to the tab it closes, closing
        // the active tab instead, a press on one tab and a release on another closing either, a
        // release with no press closing anything, or a press kept after the pointer left.
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            HTCLIENT, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_NCHITTEST,
        };
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        execute_command(window.hwnd, CommandId::New);
        execute_command(window.hwnd, CommandId::New);
        let ids = || {
            app_mut(window.hwnd)
                .tabs
                .documents()
                .map(|document| document.id)
                .collect::<Vec<_>>()
        };
        let &[first, second, third] = &ids()[..] else {
            panic!("three tabs")
        };
        let center = |index: usize| super::title_layout(window.hwnd).tab(index).center();
        let send = |message: u32, index: usize| {
            let point = center(index);
            unsafe { SendMessageW(window.hwnd, message, 0, client_lparam(point.x, point.y)) };
        };
        // Tabs answer HTCLIENT, so the middle button arrives as client WM_MBUTTON* (spec §5).
        let tab = center(0);
        assert_eq!(
            unsafe {
                SendMessageW(
                    window.hwnd,
                    WM_NCHITTEST,
                    0,
                    screen_lparam(window.hwnd, tab.x, tab.y),
                )
            },
            HTCLIENT as isize
        );

        send(WM_MBUTTONDOWN, 0);
        send(WM_MBUTTONUP, 1);
        assert_eq!(ids(), [first, second, third], "released over another tab");
        send(WM_MBUTTONUP, 0);
        assert_eq!(ids(), [first, second, third], "a release with no press");
        send(WM_MBUTTONDOWN, 0);
        unsafe {
            SendMessageW(
                window.hwnd,
                windows_sys::Win32::UI::Controls::WM_MOUSELEAVE,
                0,
                0,
            )
        };
        send(WM_MBUTTONUP, 0);
        assert_eq!(ids(), [first, second, third], "the pointer left in between");

        send(WM_MBUTTONDOWN, 0);
        send(WM_MBUTTONUP, 0);
        assert_eq!(ids(), [second, third]);
        assert_eq!(app_mut(window.hwnd).tabs.active().unwrap().id, third);
        assert_eq!(app_mut(window.hwnd).tabs.active_index(), 1);
    }

    #[test]
    fn a_middle_click_on_a_dirty_background_tab_shows_it_and_asks_first() {
        // Break caught: the save prompt asking about a tab that isn't on screen, a dirty tab
        // closed without asking, or Cancel putting the previously active tab back.
        use windows_sys::Win32::UI::WindowsAndMessaging::{WM_MBUTTONDOWN, WM_MBUTTONUP};
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        editor.set_text("unsaved").unwrap();
        let dirty = app_mut(window.hwnd).tabs.active().unwrap().id;
        assert!(app_mut(window.hwnd).tabs.active().unwrap().dirty);
        execute_command(window.hwnd, CommandId::New);
        let asked = std::rc::Rc::new(std::cell::Cell::new(false));
        let seen = asked.clone();
        answer_next_close_prompt(move |hwnd| {
            assert_eq!(
                app_mut(hwnd).tabs.active().unwrap().id,
                dirty,
                "the prompt's tab is on screen"
            );
            seen.set(true);
            CloseDecision::Cancel
        });
        let center = super::title_layout(window.hwnd).tab(0).center();

        unsafe {
            SendMessageW(window.hwnd, WM_MBUTTONDOWN, 0, client_lparam(center.x, center.y));
            SendMessageW(window.hwnd, WM_MBUTTONUP, 0, client_lparam(center.x, center.y));
        }

        assert!(asked.get(), "no prompt");
        assert_eq!(super::tab_count(window.hwnd), 2);
        assert_eq!(
            app_mut(window.hwnd).tabs.active().unwrap().id,
            dirty,
            "after Cancel it stays active"
        );
        answer_next_close_prompt(|_| CloseDecision::Discard);
        super::close_tab_at(window.hwnd, 0);
        assert_eq!(super::tab_count(window.hwnd), 1);
    }
```

- [ ] **Step 2: Run the tests and see them fail**

Run: `cargo test --lib window::tabs::tests::a_clean_background_tab`
Expected: compile error, `no method named 'close_clean_background'` (the other new tests fail to compile the same way: `cannot find function 'close_tab_at'`).

- [ ] **Step 3: Implement**

In `src/window/tabs.rs`, add after `close_reviewed`:

```rust
    /// Closes the clean tab `review` names without activating it (quick-open spec §5): the
    /// active tab stays active and keeps its place in the activation order. `Stale` when that
    /// tab has gone, is the active one or changed since `review`; `Unsaved` when it has unsaved
    /// edits, which only the usual reviewed close may discard.
    pub(crate) fn close_clean_background(
        &mut self,
        review: CloseReview,
    ) -> Result<Document, CloseReviewError> {
        let Some(index) = self
            .documents
            .iter()
            .position(|document| document.id == review.id)
        else {
            return Err(CloseReviewError::Stale);
        };
        let active = self.active_index();
        if index == active || self.documents[index].generation != review.generation {
            return Err(CloseReviewError::Stale);
        }
        if self.documents[index].dirty {
            return Err(CloseReviewError::Unsaved);
        }
        let closed = self.documents.remove(index);
        if index < active {
            self.selection.active.store(active - 1, Ordering::Release);
        }
        self.recent.retain(|recent| *recent != closed.id);
        self.view.update(&self.documents);
        Ok(closed)
    }
```

In `src/window/menus.rs`:
- Change `pub const fn accelerator_specs() -> [AcceleratorSpec; 52] {` to `[AcceleratorSpec; 53]`.
- After `        accelerator(FCONTROL | FSHIFT, b'S', CommandId::SaveAs),` add `        accelerator(FCONTROL, b'W', CommandId::CloseTab),`.
- In the File popup, replace `                MenuEntry::command("&Close tab", CommandId::CloseTab),` with `                MenuEntry::command("&Close tab\tCtrl+W", CommandId::CloseTab),`.

In `src/app.rs`, after

```rust
    /// While the tab scroll thumb is dragged: where along the thumb the pointer grabbed it.
    pub(crate) tab_thumb_grab: Option<i32>,
```

add

```rust
    /// Between a middle-button press on a tab and its release: the tab's strip index and the
    /// document it showed then (quick-open spec §5).
    pub(crate) middle_press: Option<(usize, crate::document::DocumentId)>,
```

and after `            tab_thumb_grab: None,` add `            middle_press: None,`.

In `src/window/command_palette.rs`'s `palette_control_proc`, replace

```rust
            if key == VK_ESCAPE {
                super::main_window::close_command_palette(parent, true);
                return 0;
            }
        }
        // A single-line Edit beeps at Enter and Escape characters; both were handled on key down.
        (PaletteControl::Query, WM_CHAR) if matches!(wparam as u16, 0x0d | 0x1b) => return 0,
```

with

```rust
            if key == VK_ESCAPE {
                super::main_window::close_command_palette(parent, true);
                return 0;
            }
            // Ctrl+W closes the palette here, not the tab behind it (quick-open spec §4);
            // `main_window::translate_accelerator` leaves the key to this hook.
            if key == u16::from(b'W')
                && unsafe { GetKeyState(i32::from(VK_CONTROL)) } < 0
                && unsafe { GetKeyState(i32::from(VK_MENU)) } >= 0
            {
                super::main_window::close_command_palette(parent, true);
                return 0;
            }
        }
        // A single-line Edit beeps at Enter, Escape and Ctrl+W characters; all three were
        // handled on key down.
        (PaletteControl::Query, WM_CHAR) if matches!(wparam as u16, 0x0d | 0x1b | 0x17) => {
            return 0;
        }
```

In `src/window/main_window.rs`:

Add `WM_MBUTTONDOWN, WM_MBUTTONUP,` to the `windows_sys::Win32::UI::WindowsAndMessaging::{...}` import list at the top (next to `WM_LBUTTONUP`; `cargo fmt` reflows it).

Replace the `WM_MOUSELEAVE` arm

```rust
        WM_MOUSELEAVE => {
            update_title_pointer(hwnd, |pointer| pointer.leave(false));
            crate::window::preview_host::button_hover(hwnd, None);
            0
        }
```

with

```rust
        WM_MOUSELEAVE => {
            update_title_pointer(hwnd, |pointer| pointer.leave(false));
            crate::window::preview_host::button_hover(hwnd, None);
            // A middle press whose release never reaches the strip must not close a tab later.
            if let Some(mut app) = unsafe { app_ptr(hwnd) } {
                unsafe { app.as_mut() }.middle_press = None;
            }
            0
        }
```

Insert before

```rust
        WM_NOTIFY => {
            handle_editor_notification(hwnd, lparam);
```

the arms

```rust
        // A middle-click closes the tab under the pointer (quick-open spec §5). Tabs answer
        // HTCLIENT, so the button arrives here; the caption and the logo square are nonclient
        // and keep the system's behavior.
        WM_MBUTTONDOWN => {
            let press = match client_title_target(hwnd, lparam) {
                Some(HitTarget::Tab(index) | HitTarget::CloseTab(index)) => {
                    tab_id_at(hwnd, index).map(|id| (index, id))
                }
                _ => None,
            };
            if let Some(mut app) = unsafe { app_ptr(hwnd) } {
                unsafe { app.as_mut() }.middle_press = press;
            }
            0
        }
        WM_MBUTTONUP => {
            let press = unsafe { app_ptr(hwnd) }
                .and_then(|mut app| unsafe { app.as_mut() }.middle_press.take());
            // Only over the pressed tab, and only while it still shows the same document.
            if let Some((index, id)) = press
                && let Some(HitTarget::Tab(released) | HitTarget::CloseTab(released)) =
                    client_title_target(hwnd, lparam)
                && released == index
                && tab_id_at(hwnd, index) == Some(id)
            {
                close_tab_at(hwnd, index);
            }
            0
        }
```

After `fn close_active_document` (right before `/// Closes \`id\` without asking`), add:

```rust
/// Closes the tab at strip `index` (quick-open spec §5). A clean tab that isn't the active one
/// closes where it is, and the active tab stays. Any other tab is activated first, so a save
/// prompt asks about the tab on screen, and is then closed as Close tab closes it.
fn close_tab_at(hwnd: HWND, index: usize) {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    let target = unsafe { app_ptr(hwnd) }.and_then(|app| {
        let tabs = &unsafe { app.as_ref() }.tabs;
        let document = tabs.documents().nth(index)?;
        let background = tabs.active().is_some_and(|active| active.id != document.id);
        Some((
            crate::window::tabs::CloseReview {
                id: document.id,
                generation: document.generation,
            },
            background && !document.dirty,
        ))
    });
    let Some((review, clean_background)) = target else {
        return;
    };
    if clean_background {
        close_background_document(hwnd, &identity, review);
    } else if activate_document_by_id(hwnd, review.id) && identity.is_live_for(hwnd) {
        execute_command(hwnd, CommandId::CloseTab);
    }
}

/// The document shown by tab `index` of the strip.
fn tab_id_at(hwnd: HWND, index: usize) -> Option<DocumentId> {
    unsafe { app_ptr(hwnd) }.and_then(|app| {
        unsafe { app.as_ref() }
            .tabs
            .documents()
            .nth(index)
            .map(|document| document.id)
    })
}
```

After `fn close_reviewed_document`, add:

```rust
/// `close_reviewed_document` for a clean tab that isn't active: it closes where it is, its
/// recovery snapshots go, and the editor keeps showing the active tab (no document swap).
fn close_background_document(
    hwnd: HWND,
    identity: &WindowIdentity,
    review: crate::window::tabs::CloseReview,
) {
    let closed = unsafe { app_ptr(hwnd) }.and_then(|mut app| {
        let app = unsafe { app.as_mut() };
        let closed = app.tabs.close_clean_background(review).ok()?;
        let snapshots = app
            .recovery_root
            .as_deref()
            .map(|root| crate::recovery::snapshots_removed_on_close(root, &closed, true))
            .unwrap_or_default();
        Some((closed, snapshots))
    });
    let Some((closed, snapshots)) = closed else {
        return;
    };
    drop(closed);
    crate::recovery::remove_snapshot_files(&snapshots);
    if identity.is_live_for(hwnd) {
        refresh_tabs(hwnd);
    }
}
```

In `translate_accelerator`, replace

```rust
    let accelerator = unsafe { app_ptr(hwnd) }.and_then(|app| {
        unsafe { app.as_ref() }
            .accelerators
```

with

```rust
    // Ctrl+W in the palette's field closes the palette, not a tab (quick-open spec §4). The
    // table would turn it into Close tab before the field's hook saw the key.
    if palette_keeps_key(hwnd, message) {
        return false;
    }
    let accelerator = unsafe { app_ptr(hwnd) }.and_then(|app| {
        unsafe { app.as_ref() }
            .accelerators
```

and after `fn translate_accelerator`, add:

```rust
/// Ctrl+W (without Alt) aimed at one of the command palette's controls.
fn palette_keeps_key(hwnd: HWND, message: &windows_sys::Win32::UI::WindowsAndMessaging::MSG) -> bool {
    message.message == WM_KEYDOWN
        && message.wParam == usize::from(b'W')
        && unsafe { GetKeyState(VK_CONTROL as i32) } < 0
        && unsafe { GetKeyState(VK_MENU as i32) } >= 0
        && command_palette_owns(hwnd, message.hwnd)
}
```

- [ ] **Step 4: Run the tests and see them pass**

Run: `cargo test --lib window::tabs`
Expected: all pass.
Run: `cargo test --lib window::menus`
Expected: all pass.
Run: `cargo test --lib window::command_palette`
Expected: all pass.
Run: `cargo test --lib window::main_window::tests::ctrl_w_ -- --test-threads=1`
Expected: 1 passed.
Run: `cargo test --lib window::main_window::tests::a_middle_click -- --test-threads=1`
Expected: 2 passed.

- [ ] **Step 5: Clippy**

Run: `cargo fmt; cargo clippy --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 6: Commit**

```bash
git add src/window/tabs.rs src/window/menus.rs src/window/command_palette.rs src/window/main_window.rs src/app.rs
git commit -m "feat(tabs): Ctrl+W closes the tab (in the palette's field it closes the palette), and a middle-click closes the tab under the pointer: a clean background tab where it is, any other after showing it so its prompt is on screen"
```

---

### Task 4: `CommandId::QuickOpen` and the `QuickOpen` picker

**Files:**
- Modify: `src/window/commands.rs` (enum, `needs_document`, `COMMANDS`, tests)
- Modify: `src/window/menus.rs` (`accelerator_specs`, File menu, tests)
- Modify: `src/window/command_palette.rs` (`ENTRIES`, constants, `PickerKind`, `PickerRow`, `PickerChoice`, `picker_row_label`, `CommandPalette` fields and methods, tests)
- Modify: `src/window/main_window.rs` (`execute_command_with_note`, `refilter_command_palette`, `run_command_palette_selection`, new functions, tests)
- Modify: `src/window/library_host.rs` (`picked`)
- Modify: `src/window/tabs.rs` (remove the `cfg_attr` on `activation_order`)
- Modify: `src/editor/scintilla_constants.rs`, `src/editor/scintilla.rs` (`SCI_GOTOLINE`, `Editor::go_to_line`)

**Interfaces:**
- Consumes: Task 1's `quick_open::{search, split_line, QuickMatch, QuickMatch::plain}`; Task 2's `Tabs::activation_order`; Task 3's `close_tab_at` (a test); `library_host::{folder, with_state}`; `crate::library::{record_path, path_key}`; `open_note`, `OpenMode::Permanent`, `report_open_failure`, `capture_palette_focus`, `title_chrome`, `with_command_palette`.
- Produces:
  - `CommandId::QuickOpen = 191`, not `needs_document`, not `is_sidebar`.
  - `pub(crate) const QUICK_OPEN_ROWS: usize = 50;`, `pub(crate) const NO_NOTEBOOK: &str = "No notebook is open";`
  - `PickerKind::QuickOpen`
  - `PickerRow::Note { found: QuickMatch, line: Option<u32> }`, `PickerRow::GoToLine(u32)`, `PickerRow::Notice(&'static str)`
  - `PickerChoice::Note { path: PathBuf, line: Option<u32> }`, `PickerChoice::GoToLine(u32)` (a `Notice` row gives no choice)
  - `CommandPalette::set_picker_rows(&mut self, rows: Vec<PickerRow>, selected: Option<usize>)` (new second parameter); `#[cfg(test)] shown_picker_rows(&self) -> &[PickerRow]`, `#[cfg(test)] selected_row(&self) -> Option<usize>`
  - `pub(crate) fn open_quick_open(hwnd: HWND)`, `fn quick_open_rows(hwnd: HWND, query: &str) -> (Vec<command_palette::PickerRow>, Option<usize>)`, `pub(crate) fn open_quick_open_choice(hwnd: HWND, relative: &std::path::Path, line: Option<u32>)`, `pub(crate) fn go_to_line(hwnd: HWND, line: u32)`
  - `Editor::go_to_line(&self, line: usize) -> Result<()>` (0-based); `SCI_GOTOLINE: u32 = 2024`.

**Decisions this task settles:**
- **Row order of precedence:** a query that is only `:<n>` shows `Go to line <n>` even with no notebook open (it needs none); otherwise no notebook shows `No notebook is open`; otherwise typed text shows the matcher's rows; otherwise the open tabs' notes.
- **Empty-query selection:** the second row when the first row is the active tab's note and there are two or more; otherwise the first.
- **Enter with nothing to pick** (the notice row, or no rows) leaves the picker open.
- **A note gone from the library at Enter** is reported with `report_open_failure(path, Io(NotFound, "the note is no longer in the notebook"))` and nothing opens.
- The palette window is created with nothing borrowed, as the App-borrow rule asks (the older `show_command_palette` / `open_picker` are not refactored here).
- The palette row is `Go to note…` with no category prefix (spec §3.1), placed after `File: Open recent notebook...`.

- [ ] **Step 1: Write the failing tests**

In `src/window/commands.rs`'s tests, in `replace_in_notes_is_190_a_sidebar_command_and_needs_no_document`, delete the line `        assert_eq!(CommandId::try_from(191), Err(()));` and add the test:

```rust
    #[test]
    fn quick_open_is_191_and_needs_no_document() {
        // Break caught: Ctrl+P renumbered onto another command, greyed out while no tab is open
        // (when opening a note matters most), or hidden with notes mode off.
        assert_eq!(CommandId::QuickOpen as u16, 191);
        assert_eq!(CommandId::try_from(191), Ok(CommandId::QuickOpen));
        assert!(!CommandId::QuickOpen.needs_document());
        assert!(!CommandId::QuickOpen.is_sidebar());
        assert_eq!(CommandId::try_from(192), Err(()));
    }
```

In `src/window/menus.rs`'s tests, change `assert_eq!(specs.len(), 53);` to `assert_eq!(specs.len(), 54);` and add:

```rust
    #[test]
    fn ctrl_p_opens_quick_open_and_ctrl_shift_p_stays_the_palette() {
        // Break caught: Ctrl+P unbound, or taking Ctrl+Shift+P from the command palette.
        use windows_sys::Win32::UI::WindowsAndMessaging::{FCONTROL, FSHIFT};
        let bound = |modifiers: u8| {
            accelerator_specs()
                .into_iter()
                .find(|spec| spec.modifiers == modifiers && spec.key == u16::from(b'P'))
                .map(|spec| spec.command)
        };
        assert_eq!(bound(FCONTROL), Some(CommandId::QuickOpen));
        assert_eq!(bound(FCONTROL | FSHIFT), Some(CommandId::CommandPalette));
    }
```

In `src/window/command_palette.rs`'s tests, add:

```rust
    #[test]
    fn go_to_note_is_listed_once_with_ctrl_p() {
        // Break caught: Ctrl+P working but the palette never offering it, or its row showing no
        // shortcut (quick-open spec §3.1).
        assert_eq!(labels("go to note")[0], "Go to note\u{2026}");
        assert_eq!(
            shortcut_text(CommandId::QuickOpen).as_deref(),
            Some("Ctrl+P")
        );
        assert_eq!(ENTRIES.len(), 70);
    }
```

In `src/window/main_window.rs`'s tests, after `fn a_middle_click_on_a_dirty_background_tab_shows_it_and_asks_first`, add the helpers and tests:

```rust
    /// The quick-open rows as their names, or the row itself for a non-note row.
    fn quick_open_names(hwnd: HWND) -> Vec<String> {
        with_command_palette(hwnd, |palette| {
            palette
                .shown_picker_rows()
                .iter()
                .map(|row| match row {
                    crate::window::command_palette::PickerRow::Note { found, .. } => {
                        found.name.clone()
                    }
                    other => format!("{other:?}"),
                })
                .collect()
        })
        .unwrap()
    }

    fn type_query(hwnd: HWND, text: &str) {
        use windows_sys::Win32::UI::WindowsAndMessaging::SetWindowTextW;
        let query = with_command_palette(hwnd, |palette| palette.query_hwnd()).unwrap();
        let typed = crate::platform::wide_null(text);
        unsafe { SetWindowTextW(query, typed.as_ptr()) };
    }

    fn press_enter_in_palette(hwnd: HWND) {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_RETURN;
        use windows_sys::Win32::UI::WindowsAndMessaging::WM_KEYDOWN;
        let query = with_command_palette(hwnd, |palette| palette.query_hwnd()).unwrap();
        unsafe { SendMessageW(query, WM_KEYDOWN, VK_RETURN as usize, 0) };
    }

    fn palette_visible(hwnd: HWND) -> bool {
        with_command_palette(hwnd, |palette| palette.is_visible()).unwrap_or(false)
    }

    fn active_path(hwnd: HWND) -> Option<std::path::PathBuf> {
        app_mut(hwnd).tabs.active().and_then(|document| document.path.clone())
    }

    #[test]
    fn ctrl_p_then_enter_switches_to_the_previous_note() {
        // Break caught: Ctrl+P dead in the running app, the open tabs listed in strip order, a
        // file outside the notebook or an unopened note listed, or the selection on the current
        // note, so Enter does nothing.
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
            GetKeyboardState, SetKeyboardState, VK_CONTROL,
        };
        use windows_sys::Win32::UI::WindowsAndMessaging::{MSG, WM_KEYDOWN};
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("quick-open-previous");
        let a = scratch.note("a.md", "a");
        let b = scratch.note("b.md", "b");
        scratch.note("c.md", "c");
        let outside = scratch.root.join("outside.txt");
        std::fs::write(&outside, "outside").unwrap();
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        scratch.install(window.hwnd);
        let identity = unsafe { super::window_identity(window.hwnd).unwrap() };
        super::open_path(window.hwnd, &a).unwrap();
        super::open_path(window.hwnd, &outside).unwrap();
        super::open_note(window.hwnd, &b, super::OpenMode::Permanent, false).unwrap();

        let mut keys = [0u8; 256];
        unsafe { GetKeyboardState(keys.as_mut_ptr()) };
        let original = keys;
        keys[VK_CONTROL as usize] = 0x80;
        unsafe { SetKeyboardState(keys.as_ptr()) };
        let message = MSG {
            hwnd: editor.hwnd(),
            message: WM_KEYDOWN,
            wParam: usize::from(b'P'),
            ..Default::default()
        };
        let translated = unsafe { super::translate_accelerator(window.hwnd, &identity, &message) };
        unsafe { SetKeyboardState(original.as_ptr()) };

        assert!(translated, "Ctrl+P was not translated");
        assert!(palette_visible(window.hwnd));
        assert_eq!(
            with_command_palette(window.hwnd, |p| p.picker().map(|p| p.kind)).flatten(),
            Some(crate::window::command_palette::PickerKind::QuickOpen)
        );
        assert_eq!(
            with_command_palette(window.hwnd, |p| p.query_text()).unwrap(),
            ""
        );
        assert_eq!(quick_open_names(window.hwnd), ["b", "a"]);
        assert_eq!(
            with_command_palette(window.hwnd, |p| p.selected_row()).flatten(),
            Some(1)
        );
        press_enter_in_palette(window.hwnd);
        assert!(!palette_visible(window.hwnd));
        assert_eq!(active_path(window.hwnd), Some(a));
    }

    #[test]
    fn typing_a_name_then_enter_opens_a_closed_note_as_a_normal_tab() {
        // Break caught: typed letters never reaching the matcher, a folder-only match dropped,
        // or the pick opening in the preview tab the next sidebar click replaces.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("quick-open-type");
        let alpha = scratch.note("alpha.md", "a");
        scratch.note("beta.md", "b");
        std::fs::create_dir_all(scratch.folder().join("work")).unwrap();
        let gamma = scratch.note(r"work\gamma notes.md", "g");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        scratch.install(window.hwnd);
        super::open_path(window.hwnd, &alpha).unwrap();

        execute_command(window.hwnd, CommandId::QuickOpen);
        type_query(window.hwnd, "wk gmn");
        assert_eq!(quick_open_names(window.hwnd), ["gamma notes"]);
        assert_eq!(
            with_command_palette(window.hwnd, |p| p.selected_row()).flatten(),
            Some(0)
        );
        press_enter_in_palette(window.hwnd);

        assert_eq!(active_path(window.hwnd), Some(gamma));
        assert_eq!(super::tab_count(window.hwnd), 2);
        assert_eq!(app_mut(window.hwnd).tabs.preview_id(), None);
    }

    #[test]
    fn a_line_suffix_puts_the_caret_on_that_line_and_colon_digits_alone_moves_the_current_tab() {
        // Break caught: "lines:3" matched as text, the line applied 0-based (caret on line 4),
        // a line past the end ignored, or ":2" offering notes instead of moving the caret.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("quick-open-line");
        scratch.note("lines.md", "one\r\ntwo\r\nthree\r\nfour");
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        scratch.install(window.hwnd);
        let caret_line = || {
            editor
                .line_from_position(editor.selection().unwrap().start)
                .unwrap()
        };

        execute_command(window.hwnd, CommandId::QuickOpen);
        type_query(window.hwnd, "lines:3");
        assert_eq!(quick_open_names(window.hwnd), ["lines"]);
        press_enter_in_palette(window.hwnd);
        assert_eq!(caret_line(), 2);

        execute_command(window.hwnd, CommandId::QuickOpen);
        type_query(window.hwnd, ":2");
        assert_eq!(
            with_command_palette(window.hwnd, |p| p.shown_picker_rows().to_vec()).unwrap(),
            [crate::window::command_palette::PickerRow::GoToLine(2)]
        );
        press_enter_in_palette(window.hwnd);
        assert_eq!(caret_line(), 1);

        execute_command(window.hwnd, CommandId::QuickOpen);
        type_query(window.hwnd, "lines:99");
        press_enter_in_palette(window.hwnd);
        assert_eq!(caret_line(), 3, "past the end goes to the last line");
    }

    #[test]
    fn with_no_notebook_open_the_picker_shows_one_row_that_cannot_be_picked() {
        // Break caught: an empty list that looks broken, Enter closing the picker or opening
        // something, or ":5" refused although it needs no notebook.
        use crate::window::command_palette::{NO_NOTEBOOK, PickerRow};
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        assert!(crate::window::library_host::folder(window.hwnd).is_none());

        execute_command(window.hwnd, CommandId::QuickOpen);
        let rows = || with_command_palette(window.hwnd, |p| p.shown_picker_rows().to_vec()).unwrap();
        assert_eq!(rows(), [PickerRow::Notice(NO_NOTEBOOK)]);
        assert_eq!(
            with_command_palette(window.hwnd, |p| p.selected_row()).flatten(),
            None
        );
        press_enter_in_palette(window.hwnd);
        assert!(palette_visible(window.hwnd), "Enter does nothing");
        assert_eq!(super::tab_count(window.hwnd), 1);

        type_query(window.hwnd, ":5");
        assert_eq!(rows(), [PickerRow::GoToLine(5)]);
    }

    #[test]
    fn ctrl_p_again_keeps_the_query_and_a_query_of_spaces_lists_the_open_tabs() {
        // Break caught (review focus 1 and 4): a second Ctrl+P clearing what was typed, or a
        // query of spaces listing every note, or none.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("quick-open-again");
        let a = scratch.note("alpha.md", "a");
        let b = scratch.note("beta.md", "b");
        scratch.note("gamma.md", "g");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        scratch.install(window.hwnd);
        super::open_path(window.hwnd, &a).unwrap();
        super::open_path(window.hwnd, &b).unwrap();

        execute_command(window.hwnd, CommandId::QuickOpen);
        type_query(window.hwnd, "gam");
        execute_command(window.hwnd, CommandId::QuickOpen);
        assert_eq!(
            with_command_palette(window.hwnd, |p| p.query_text()).unwrap(),
            "gam"
        );
        assert_eq!(quick_open_names(window.hwnd), ["gamma"]);

        type_query(window.hwnd, "   ");
        assert_eq!(quick_open_names(window.hwnd), ["beta", "alpha"]);
        assert_eq!(
            with_command_palette(window.hwnd, |p| p.selected_row()).flatten(),
            Some(1)
        );
    }

    #[test]
    fn a_tab_closed_while_the_picker_is_open_still_opens_from_its_row() {
        // Break caught (review focus 5): a row naming a tab that closed under the open picker
        // switching to a dead document, or doing nothing.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("quick-open-closed-tab");
        let a = scratch.note("a.md", "a");
        let b = scratch.note("b.md", "b");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        scratch.install(window.hwnd);
        super::open_path(window.hwnd, &a).unwrap();
        super::open_path(window.hwnd, &b).unwrap();

        execute_command(window.hwnd, CommandId::QuickOpen);
        assert_eq!(quick_open_names(window.hwnd), ["b", "a"]);
        super::close_tab_at(window.hwnd, 0);
        assert_eq!(tab_paths(window.hwnd), [Some(b.clone())]);
        assert!(palette_visible(window.hwnd));
        press_enter_in_palette(window.hwnd);

        assert_eq!(tab_paths(window.hwnd), [Some(b), Some(a.clone())]);
        assert_eq!(active_path(window.hwnd), Some(a));
    }

    #[test]
    fn a_note_removed_from_the_library_after_listing_reports_it_and_opens_nothing() {
        // Break caught (review focus 5): a note deleted after the list was shown opening an
        // empty tab, failing silently, or crashing the pick.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("quick-open-removed");
        scratch.note("alpha.md", "a");
        let gamma = scratch.note("gamma.md", "g");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        scratch.install(window.hwnd);
        let tabs_before = super::tab_count(window.hwnd);

        execute_command(window.hwnd, CommandId::QuickOpen);
        type_query(window.hwnd, "gam");
        assert_eq!(quick_open_names(window.hwnd), ["gamma"]);
        crate::window::library_host::with_state(window.hwnd, |state| state.remove_note(&gamma));
        std::fs::remove_file(&gamma).unwrap();
        press_enter_in_palette(window.hwnd);

        assert_eq!(super::tab_count(window.hwnd), tabs_before);
        assert!(
            notices(window.hwnd)
                .iter()
                .any(|n| n.contains("could not open") && n.contains("no longer in the notebook")),
            "{:?}",
            notices(window.hwnd)
        );
    }
```

- [ ] **Step 2: Run the tests and see them fail**

Run: `cargo test --lib window::commands`
Expected: compile error, `no variant or associated item named 'QuickOpen' found for enum 'CommandId'`.

- [ ] **Step 3: Implement**

**`src/editor/scintilla_constants.rs`:** before `pub const SCI_GOTOPOS: u32 = 2025;` add `pub const SCI_GOTOLINE: u32 = 2024;`.

**`src/editor/scintilla.rs`:** add `SCI_GOTOLINE` to the `#[cfg(windows)] use crate::editor::scintilla_constants::{SC_ELEMENT_CARET_LINE_BACK, ...}` import group, and after `pub fn scroll_caret_into_view(&self)` add:

```rust
    /// Moves the caret to the start of 0-based `line`, removing any selection, and scrolls it
    /// into view. A line past the end goes to the last line (Scintilla clamps it).
    #[cfg(windows)]
    pub fn go_to_line(&self, line: usize) -> Result<()> {
        self.endpoint.send_direct_checked(SCI_GOTOLINE, line, 0)?;
        self.scroll_caret_into_view();
        Ok(())
    }

    #[cfg(not(windows))]
    pub fn go_to_line(&self, _line: usize) -> Result<()> {
        Err(FastPadError::Invariant(
            "Scintilla editor is only supported on Windows",
        ))
    }
```

**`src/window/commands.rs`:**
- After `    ReplaceInNotes = 190,` add `    QuickOpen = 191,`.
- In `needs_document`, after `                | Self::ReplaceInNotes` add `                | Self::QuickOpen`.
- Change `const COMMANDS: [CommandId; 82] = [` to `[CommandId; 83]` and after `            CommandId::ReplaceInNotes,` add `            CommandId::QuickOpen,`.

**`src/window/menus.rs`:**
- Change `[AcceleratorSpec; 53]` to `[AcceleratorSpec; 54]`.
- Before `        accelerator(FCONTROL | FSHIFT, b'P', CommandId::CommandPalette),` add `        accelerator(FCONTROL, b'P', CommandId::QuickOpen),`.
- In the File popup, after `                MenuEntry::command("Open &Notebook...\tCtrl+Shift+O", CommandId::OpenFolder),` add `                MenuEntry::command("&Go to note\u{2026}\tCtrl+P", CommandId::QuickOpen),`.

**`src/window/tabs.rs`:** remove the `#[cfg_attr(not(test), expect(dead_code, ...))]` attribute above `activation_order`.

**`src/window/command_palette.rs`:**

Add `use crate::library::quick_open::QuickMatch;` and `use std::path::PathBuf;` to the imports.

Change `pub(crate) const ENTRIES: [PaletteEntry; 69] = [` to `[PaletteEntry; 70]`, and after `    entry("File: Open recent notebook...", CommandId::OpenRecentFolder),` add `    entry("Go to note\u{2026}", CommandId::QuickOpen),`.

Replace

```rust
/// What a picker is choosing; decides what `library_host::picked` does with the choice.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PickerKind {
    RecentFolder,
    MoveToNotebook,
}
```

with

```rust
/// What a picker is choosing; decides what `library_host::picked` does with the choice.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PickerKind {
    RecentFolder,
    MoveToNotebook,
    /// Ctrl+P (quick-open spec §3): notes by name. Its rows come from `main_window`, not from
    /// `Picker::items`.
    QuickOpen,
}

/// The most rows quick open lists (spec §3.3).
pub(crate) const QUICK_OPEN_ROWS: usize = 50;
/// Quick open's one row while no notebook is open; it can't be picked (spec §3.1).
pub(crate) const NO_NOTEBOOK: &str = "No notebook is open";
```

Replace

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PickerRow {
    Item(usize),
    Create(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PickerChoice {
    Item(usize),
    Create(String),
}
```

with

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PickerRow {
    Item(usize),
    Create(String),
    /// A quick-open note, and the line the query's `:<n>` names.
    Note { found: QuickMatch, line: Option<u32> },
    /// A quick-open query that is only `:<n>`: that line of the current tab.
    GoToLine(u32),
    /// A row that can't be picked, such as `NO_NOTEBOOK`.
    Notice(&'static str),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PickerChoice {
    Item(usize),
    Create(String),
    /// A quick-open note, relative to the notebook.
    Note { path: PathBuf, line: Option<u32> },
    GoToLine(u32),
}
```

In `picker_row_label`, replace

```rust
        PickerRow::Create(name) => {
            format!(
                "{} \u{201c}{name}\u{201d}",
                picker.create.unwrap_or("Create")
            )
        }
    }
```

with

```rust
        PickerRow::Create(name) => {
            format!(
                "{} \u{201c}{name}\u{201d}",
                picker.create.unwrap_or("Create")
            )
        }
        PickerRow::Note { found, .. } if found.folder.is_empty() => found.name.clone(),
        PickerRow::Note { found, .. } => format!("{}, in {}", found.name, found.folder),
        PickerRow::GoToLine(line) => format!("Go to line {line}"),
        PickerRow::Notice(text) => (*text).to_owned(),
    }
```

In `struct CommandPalette`, after the `picker_rows: Vec<PickerRow>,` field add:

```rust
    /// The row `fill_list` selects in picker mode; `None` selects nothing.
    picker_selected: Option<usize>,
```

and in `create`'s `Ok(Self { ... })`, after `            picker_rows: Vec::new(),` add `            picker_selected: None,`.

In `mark_hidden`, after `        self.picker_rows = Vec::new();` add `        self.picker_selected = None;`. In `set_picker`, after `        self.picker_rows = Vec::new();` add `        self.picker_selected = None;`.

Replace `set_picker_rows` with:

```rust
    /// Records the picker rows to list and the one to select; `fill_list` then puts them in the
    /// list box.
    pub(crate) fn set_picker_rows(&mut self, rows: Vec<PickerRow>, selected: Option<usize>) {
        self.picker_rows = rows;
        self.picker_selected = selected;
    }
```

In `fill_list`, replace

```rust
        let count = if self.picker.is_some() {
            let empty = wide_null("");
            for _ in 0..self.picker_rows.len() {
                unsafe {
                    SendMessageW(self.list, LB_ADDSTRING, 0, empty.as_ptr() as LPARAM);
                }
            }
            self.picker_rows.len()
        } else {
            for entry in &self.shown {
                let label = wide_null(entry.label);
                unsafe {
                    SendMessageW(self.list, LB_ADDSTRING, 0, label.as_ptr() as LPARAM);
                }
            }
            self.shown.len()
        };
        if count > 0 {
            unsafe {
                SendMessageW(self.list, LB_SETCURSEL, 0, 0);
            }
        }
```

with

```rust
        let (count, selected) = if self.picker.is_some() {
            let empty = wide_null("");
            for _ in 0..self.picker_rows.len() {
                unsafe {
                    SendMessageW(self.list, LB_ADDSTRING, 0, empty.as_ptr() as LPARAM);
                }
            }
            (self.picker_rows.len(), self.picker_selected)
        } else {
            for entry in &self.shown {
                let label = wide_null(entry.label);
                unsafe {
                    SendMessageW(self.list, LB_ADDSTRING, 0, label.as_ptr() as LPARAM);
                }
            }
            (self.shown.len(), Some(0))
        };
        if let Some(selected) = selected.filter(|&selected| selected < count) {
            unsafe {
                SendMessageW(self.list, LB_SETCURSEL, selected, 0);
            }
        }
```

In `selected_choice`, replace

```rust
        Some(match row {
            PickerRow::Item(index) => PickerChoice::Item(*index),
            PickerRow::Create(name) => PickerChoice::Create(name.clone()),
        })
```

with

```rust
        Some(match row {
            PickerRow::Item(index) => PickerChoice::Item(*index),
            PickerRow::Create(name) => PickerChoice::Create(name.clone()),
            PickerRow::Note { found, line } => PickerChoice::Note {
                path: found.path.clone(),
                line: *line,
            },
            PickerRow::GoToLine(line) => PickerChoice::GoToLine(*line),
            PickerRow::Notice(_) => return None,
        })
```

After `pub(crate) fn shown(&self) -> &[PaletteEntry]` (inside `impl CommandPalette`) add:

```rust
    #[cfg(test)]
    pub(crate) fn shown_picker_rows(&self) -> &[PickerRow] {
        &self.picker_rows
    }

    #[cfg(test)]
    pub(crate) fn selected_row(&self) -> Option<usize> {
        usize::try_from(unsafe { SendMessageW(self.list, LB_GETCURSEL, 0, 0) }).ok()
    }
```

**`src/window/library_host.rs`**, in `picked`, replace

```rust
            move_note_to(hwnd, &note, &destination);
        }
        _ => {}
```

with

```rust
            move_note_to(hwnd, &note, &destination);
        }
        (PickerKind::QuickOpen, PickerChoice::Note { path, line }) => {
            super::main_window::open_quick_open_choice(hwnd, &path, line);
        }
        (PickerKind::QuickOpen, PickerChoice::GoToLine(line)) => {
            super::main_window::go_to_line(hwnd, line);
        }
        _ => {}
```

**`src/window/main_window.rs`:**

In `execute_command_with_note`, after `        CommandId::CommandPalette => open_command_palette(hwnd),` add `        CommandId::QuickOpen => open_quick_open(hwnd),`.

After `fn open_picker`, add:

```rust
/// Ctrl+P, the palette row and File → Go to note… (quick-open spec §3.1): the palette in the
/// `QuickOpen` picker with an empty query. While that picker already shows, nothing changes.
pub(crate) fn open_quick_open(hwnd: HWND) {
    let (visible, showing) = with_command_palette(hwnd, |palette| {
        let quick_open = palette
            .picker()
            .is_some_and(|picker| picker.kind == command_palette::PickerKind::QuickOpen);
        (palette.is_visible(), palette.is_visible() && quick_open)
    })
    .unwrap_or((false, false));
    if showing {
        return;
    }
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    // Still made on first use (spec §3.6), and with nothing borrowed: it creates windows.
    let missing = unsafe { app_ptr(hwnd) }
        .is_some_and(|app| unsafe { app.as_ref() }.command_palette.is_none());
    if missing {
        let Ok(created) = CommandPalette::create(hwnd) else {
            return;
        };
        if !identity.is_live_for(hwnd) {
            return;
        }
        if let Some(mut app) = unsafe { app_ptr(hwnd) } {
            let app = unsafe { app.as_mut() };
            if app.command_palette.is_none() {
                app.command_palette = Some(created);
            }
        }
    }
    // Switching from the open command list keeps the focus the palette first took from.
    if !visible {
        capture_palette_focus(hwnd);
    }
    let colors = title_chrome(hwnd).0;
    let shown = unsafe { app_ptr(hwnd) }.and_then(|mut app| {
        let palette = unsafe { app.as_mut() }.command_palette.as_mut()?;
        palette.mark_shown(colors);
        palette.set_subset(None);
        palette.set_picker(Some(command_palette::Picker {
            kind: command_palette::PickerKind::QuickOpen,
            items: Vec::new(),
            create: None,
        }));
        Some(())
    });
    if shown.is_none() {
        return;
    }
    // Always from an empty query. Clearing sends EN_CHANGE, which lists the rows.
    with_command_palette(hwnd, CommandPalette::clear_query);
    if !identity.is_live_for(hwnd) {
        return;
    }
    refilter_command_palette(hwnd);
    with_command_palette(hwnd, CommandPalette::focus_query);
}

/// The quick-open rows for `query` and the row to select (spec §3.1–3.4). Only reads what is
/// in memory: the tabs and `LibraryState.notes`.
fn quick_open_rows(hwnd: HWND, query: &str) -> (Vec<command_palette::PickerRow>, Option<usize>) {
    use crate::library::quick_open::{self, QuickMatch};
    use command_palette::PickerRow;
    let (text, line) = quick_open::split_line(query);
    let typed = !text.trim().is_empty();
    // `:<n>` alone needs no notebook: it moves the current tab's caret.
    if let Some(line) = line.filter(|_| !typed) {
        return (vec![PickerRow::GoToLine(line)], Some(0));
    }
    let Some(folder) = crate::window::library_host::folder(hwnd) else {
        return (vec![PickerRow::Notice(command_palette::NO_NOTEBOOK)], None);
    };
    if typed {
        let rows = crate::window::library_host::with_state(hwnd, |state| {
            quick_open::search(
                state.notes.iter().map(|note| &note.path),
                text,
                command_palette::QUICK_OPEN_ROWS,
            )
        })
        .unwrap_or_default()
        .into_iter()
        .map(|found| PickerRow::Note { found, line })
        .collect::<Vec<_>>();
        let selected = (!rows.is_empty()).then_some(0);
        return (rows, selected);
    }
    // Nothing typed: the notes open in tabs, the most recently used first (spec §3.2). Tabs
    // outside the notebook are left out here, untitled ones by having no path.
    let open = unsafe { app_ptr(hwnd) }
        .map(|app| {
            let tabs = &unsafe { app.as_ref() }.tabs;
            let active = tabs.active().map(|document| document.id);
            tabs.activation_order()
                .iter()
                .filter_map(|&id| {
                    let path = tabs.document(id)?.path.as_deref()?;
                    let relative = crate::library::record_path(&folder, path);
                    (!relative.is_absolute())
                        .then(|| (crate::library::path_key(&relative), Some(id) == active))
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if open.is_empty() {
        return (Vec::new(), None);
    }
    // Then only the notes the library lists, spelled as it spells them.
    let (rows, first_active) = crate::window::library_host::with_state(hwnd, |state| {
        let wanted = open
            .iter()
            .map(|(key, _)| key.as_str())
            .collect::<std::collections::HashSet<_>>();
        let mut listed = std::collections::HashMap::new();
        for note in &state.notes {
            let key = crate::library::path_key(&note.path);
            if wanted.contains(key.as_str()) {
                listed.insert(key, note.path.clone());
            }
        }
        let mut rows = Vec::new();
        let mut first_active = false;
        for (key, active) in &open {
            let Some(path) = listed.get(key) else {
                continue;
            };
            let Some(found) = QuickMatch::plain(path) else {
                continue;
            };
            if rows.is_empty() {
                first_active = *active;
            }
            rows.push(PickerRow::Note { found, line: None });
        }
        (rows, first_active)
    })
    .unwrap_or_default();
    // The current note leads, so the selection starts on the one before it: Ctrl+P then Enter
    // goes back to the previous note.
    let selected = match rows.len() {
        0 => None,
        1 => Some(0),
        _ if first_active => Some(1),
        _ => Some(0),
    };
    (rows, selected)
}

/// Opens a quick-open pick (spec §3.5). `relative` is resolved again against the notebook's
/// notes, since it may have left the library since the list was shown; then it opens as a
/// normal tab (an open one is switched to) with the focus in the editor, and `line` applies.
pub(crate) fn open_quick_open_choice(hwnd: HWND, relative: &std::path::Path, line: Option<u32>) {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    let Some(folder) = crate::window::library_host::folder(hwnd) else {
        return;
    };
    let path = folder.join(relative);
    let key = crate::library::path_key(relative);
    let listed = crate::window::library_host::with_state(hwnd, |state| {
        state
            .notes
            .iter()
            .any(|note| crate::library::path_key(&note.path) == key)
    })
    .unwrap_or(false);
    if !listed {
        report_open_failure(
            hwnd,
            &path,
            &crate::FastPadError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "the note is no longer in the notebook",
            )),
        );
        return;
    }
    if let Err(error) = open_note(hwnd, &path, OpenMode::Permanent, true) {
        report_open_failure(hwnd, &path, &error);
        return;
    }
    if let Some(line) = line
        && identity.is_live_for(hwnd)
    {
        go_to_line(hwnd, line);
    }
}

/// Moves the active tab's caret to the start of 1-based `line`, the last line when past the end,
/// and scrolls it into view (spec §3.4). Does nothing with no tab open.
pub(crate) fn go_to_line(hwnd: HWND, line: u32) {
    if tab_count(hwnd) == 0 {
        return;
    }
    let Some(editor) =
        unsafe { app_ptr(hwnd) }.and_then(|app| unsafe { app.as_ref() }.editor.clone())
    else {
        return;
    };
    let _ = editor.go_to_line(line.saturating_sub(1) as usize);
}
```

In `refilter_command_palette`, replace

```rust
    if is_picker {
        if let Some(mut app) = unsafe { app_ptr(hwnd) }
            && let Some(palette) = unsafe { app.as_mut() }.command_palette.as_mut()
            && let Some(picker) = palette.picker()
        {
            let rows = command_palette::picker_rows(picker, &query);
            palette.set_picker_rows(rows);
        }
    } else {
```

with

```rust
    if is_picker {
        let quick_open = with_command_palette(hwnd, |palette| {
            palette
                .picker()
                .is_some_and(|picker| picker.kind == command_palette::PickerKind::QuickOpen)
        })
        .unwrap_or(false);
        // Built before the palette is borrowed: the rows read the tabs and the library.
        let quick_rows = quick_open.then(|| quick_open_rows(hwnd, &query));
        if let Some(mut app) = unsafe { app_ptr(hwnd) }
            && let Some(palette) = unsafe { app.as_mut() }.command_palette.as_mut()
        {
            match quick_rows {
                Some((rows, selected)) => palette.set_picker_rows(rows, selected),
                None => {
                    if let Some(picker) = palette.picker() {
                        let rows = command_palette::picker_rows(picker, &query);
                        let selected = (!rows.is_empty()).then_some(0);
                        palette.set_picker_rows(rows, selected);
                    }
                }
            }
        }
    } else {
```

In `run_command_palette_selection`, replace

```rust
    .flatten();
    let command = if pick.is_none() {
```

with

```rust
    .flatten();
    // A quick-open row that can't be picked ("No notebook is open"), or no row at all, leaves
    // the picker open (spec §3.1).
    if matches!(pick, Some((command_palette::PickerKind::QuickOpen, None))) {
        return;
    }
    let command = if pick.is_none() {
```

- [ ] **Step 4: Run the tests and see them pass**

Run: `cargo test --lib window::commands`
Expected: all pass.
Run: `cargo test --lib window::menus`
Expected: all pass.
Run: `cargo test --lib window::command_palette`
Expected: all pass (including `every_command_except_tab_positions_and_the_palette_is_listed_once`, which now covers 191).
Run: `cargo test --lib window::main_window::tests::a_picker_lists_its_items -- --test-threads=1`
Expected: 1 passed (the old pickers still select their first row).
Run: `cargo test --lib window::main_window::tests::ctrl_p_ -- --test-threads=1`
Expected: 2 passed (`ctrl_p_then_enter_switches_to_the_previous_note`, `ctrl_p_again_keeps_the_query_and_a_query_of_spaces_lists_the_open_tabs`).
Run each of these with `cargo test --lib window::main_window::tests::<name> -- --test-threads=1`: `typing_a_name_then_enter_opens_a_closed_note_as_a_normal_tab`, `a_line_suffix_puts_the_caret_on_that_line`, `with_no_notebook_open_the_picker_shows_one_row`, `a_tab_closed_while_the_picker_is_open`, `a_note_removed_from_the_library_after_listing`.
Expected: each 1 passed.

- [ ] **Step 5: Clippy**

Run: `cargo fmt; cargo clippy --all-targets -- -D warnings`
Expected: no warnings (the `expect(dead_code)` on `activation_order` is gone and the method is read).

- [ ] **Step 6: Commit**

```bash
git add src/window/commands.rs src/window/menus.rs src/window/command_palette.rs src/window/main_window.rs src/window/library_host.rs src/window/tabs.rs src/editor/scintilla.rs src/editor/scintilla_constants.rs
git commit -m "feat(palette): Ctrl+P quick open (command 191, the \"Go to note…\" row and File menu item): fuzzy note matches, the open tabs most recent first with the previous note selected, name:42 and :42 to a line, and a disabled \"No notebook is open\" row"
```

---

### Task 5: Owner-drawn rows, the hint, screen-reader text and docs

**Files:**
- Modify: `src/window/command_palette.rs` (imports, `CommandPalette` field, `create`, `fill_list`, `draw_item`, new methods and free functions, `Drop`, `palette_control_proc`, tests)
- Modify: `src/window/main_window.rs` (`paint_palette_placeholder`, `open_quick_open` and `show_command_palette` endings, a test)
- Modify: `README.md`, `docs/superpowers/specs/2026-09-24-quick-open-design.md` (new §9)

**Interfaces:**
- Consumes: Task 4's `PickerRow::{Note, GoToLine, Notice}`, `PickerKind::QuickOpen`, `NO_NOTEBOOK`, `picker_row_label`.
- Produces:
  - `pub(crate) const QUICK_OPEN_PLACEHOLDER: &str = "Go to note by name";`
  - `fn hit_runs<'a>(text: &'a str, hits: &[usize]) -> Vec<(&'a str, bool)>`
  - `fn draw_runs(dc: HDC, rect: &mut RECT, text: &str, hits: &[usize], regular: HFONT, bold: HFONT, color: u32)`
  - `CommandPalette::placeholder(&self) -> Option<&'static str>`, `CommandPalette::paint_placeholder(&self, edit: HWND) -> bool`, private `bold_font(&self, base: HFONT) -> HFONT`, private `draw_quick_open_row(&self, item: &DRAWITEMSTRUCT, row: &PickerRow)`; `#[cfg(test)] list_text(&self, index: usize) -> String`, `#[cfg(test)] has_bold_font(&self) -> bool`
  - `pub(crate) fn main_window::paint_palette_placeholder(hwnd: HWND, edit: HWND) -> bool`

**Decisions this task settles:**
- **The hint is painted, not a cue banner:** FastPad has no ComCtl32 v6 manifest, so `EM_SETCUEBANNER` would show nothing (the Search view and find bar paint theirs). The field's hook paints `QUICK_OPEN_PLACEHOLDER` in the muted color while the field is empty and the picker is `QuickOpen`, and repaints when the field turns empty or non-empty.
- **Screen readers:** every picker row's list-box string is now its `picker_row_label` (it was `""`), so MSAA reads quick-open rows as `meeting notes, in Work\2026`, the notice as `No notebook is open`, and the older pickers' rows as their text too.
- **The bold font** is made from the list's font (or `DEFAULT_GUI_FONT` when it has none) with `FW_BOLD`, cached until the list's font changes, and deleted on drop.

- [ ] **Step 1: Write the failing tests**

In `src/window/command_palette.rs`'s tests, extend the `use super::{...}` list with `hit_runs` and add:

```rust
    #[test]
    fn hit_runs_cut_at_char_positions_not_bytes() {
        // Break caught (review focus 2): hits used as byte offsets, so "Über Straße" bolds "S"
        // and "t" one char late, or a run split inside a multi-byte char (a panic).
        assert_eq!(
            hit_runs("Über Straße", &[5, 6]),
            [("Über ", false), ("St", true), ("raße", false)]
        );
        assert_eq!(hit_runs("abc", &[0, 1, 2]), [("abc", true)]);
        assert_eq!(hit_runs("abc", &[]), [("abc", false)]);
        assert!(hit_runs("", &[]).is_empty());
    }
```

In `src/window/main_window.rs`'s tests, after `fn a_note_removed_from_the_library_after_listing_reports_it_and_opens_nothing`, add:

```rust
    #[test]
    fn quick_open_rows_carry_their_text_for_screen_readers_and_draw_their_hits_in_bold() {
        // Break caught: rows a screen reader reads as blank, the notice read as anything else,
        // hits drawn in the regular font, or the hint missing from the empty field (no ComCtl32
        // v6 manifest, so EM_SETCUEBANNER shows nothing) or left behind in command mode.
        use windows_sys::Win32::Graphics::Gdi::{CreateCompatibleDC, DeleteDC};
        use windows_sys::Win32::UI::Controls::DRAWITEMSTRUCT;
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        let palette = || app_mut(window.hwnd).command_palette.as_ref().unwrap();

        execute_command(window.hwnd, CommandId::QuickOpen);
        assert_eq!(
            palette().list_text(0),
            crate::window::command_palette::NO_NOTEBOOK
        );
        assert_eq!(palette().placeholder(), Some("Go to note by name"));
        let query = palette().query_hwnd();
        assert!(palette().paint_placeholder(query));
        execute_command(window.hwnd, CommandId::CommandPalette);
        assert_eq!(palette().placeholder(), None);
        assert!(!palette().paint_placeholder(query));

        let scratch = LibraryScratch::new("quick-open-draw");
        std::fs::create_dir_all(scratch.folder().join("work")).unwrap();
        scratch.note(r"work\gamma notes.md", "g");
        scratch.install(window.hwnd);
        execute_command(window.hwnd, CommandId::QuickOpen);
        type_query(window.hwnd, "wk gmn");
        assert_eq!(palette().list_text(0), r"gamma notes, in work");

        let dc = unsafe { CreateCompatibleDC(std::ptr::null_mut()) };
        let item = DRAWITEMSTRUCT {
            itemID: 0,
            hDC: dc,
            rcItem: RECT {
                left: 0,
                top: 0,
                right: 400,
                bottom: 26,
            },
            ..Default::default()
        };
        palette().draw_item(&item);
        unsafe { DeleteDC(dc) };
        assert!(palette().has_bold_font());
    }
```

- [ ] **Step 2: Run the tests and see them fail**

Run: `cargo test --lib window::command_palette::tests::hit_runs`
Expected: compile error, `unresolved import 'super::hit_runs'` (and in main_window, `no method named 'list_text'`).

- [ ] **Step 3: Implement**

**`src/window/command_palette.rs`:**

Imports: add `use std::cell::Cell;`; add `CreateFontIndirectW, DEFAULT_GUI_FONT, FW_BOLD, GetObjectW, GetStockObject, LOGFONTW` to the `windows_sys::Win32::Graphics::Gdi` list; add `EM_GETMARGINS, EM_REPLACESEL, EM_UNDO` to the `windows_sys::Win32::UI::Controls` list; add `WM_CLEAR, WM_CUT, WM_PAINT, WM_PASTE, WM_SETTEXT, WM_UNDO` to the `windows_sys::Win32::UI::WindowsAndMessaging` list.

After `pub(crate) const NO_NOTEBOOK: &str = "No notebook is open";` add:

```rust
/// What quick open's empty field shows (spec §3.1).
pub(crate) const QUICK_OPEN_PLACEHOLDER: &str = "Go to note by name";
```

In `struct CommandPalette`, after `list_brush: HBRUSH,` add:

```rust
    /// The list's font when the bold one was made, and the bold one; null until a quick-open
    /// row is first drawn.
    bold: Cell<(HFONT, HFONT)>,
```

and in `create`'s `Ok(Self { ... })`, after `            list_brush: unsafe { CreateSolidBrush(colors.strip_background) },` add `            bold: Cell::new((std::ptr::null_mut(), std::ptr::null_mut())),`.

In `fill_list`, replace

```rust
        let (count, selected) = if self.picker.is_some() {
            let empty = wide_null("");
            for _ in 0..self.picker_rows.len() {
                unsafe {
                    SendMessageW(self.list, LB_ADDSTRING, 0, empty.as_ptr() as LPARAM);
                }
            }
            (self.picker_rows.len(), self.picker_selected)
```

with

```rust
        let (count, selected) = if let Some(picker) = &self.picker {
            // Owner-drawn, but the strings are what a screen reader reads (spec §3.7).
            for row in &self.picker_rows {
                let label = wide_null(&picker_row_label(picker, row));
                unsafe {
                    SendMessageW(self.list, LB_ADDSTRING, 0, label.as_ptr() as LPARAM);
                }
            }
            (self.picker_rows.len(), self.picker_selected)
```

At the start of `draw_item`, replace

```rust
    pub(crate) fn draw_item(&self, item: &DRAWITEMSTRUCT) {
        let index = usize::try_from(item.itemID).ok();
```

with

```rust
    pub(crate) fn draw_item(&self, item: &DRAWITEMSTRUCT) {
        let index = usize::try_from(item.itemID).ok();
        let quick_open = self
            .picker
            .as_ref()
            .is_some_and(|picker| picker.kind == PickerKind::QuickOpen);
        if quick_open {
            if let Some(row) = index.and_then(|index| self.picker_rows.get(index)) {
                self.draw_quick_open_row(item, row);
            }
            return;
        }
```

After `draw_item` add:

```rust
    /// A quick-open row (spec §3.3): the name with its matched letters in bold, then the folder
    /// in the muted color, its matched letters bold too. The notice row can't be picked, so it is
    /// muted and never drawn selected.
    fn draw_quick_open_row(&self, item: &DRAWITEMSTRUCT, row: &PickerRow) {
        let notice = matches!(row, PickerRow::Notice(_));
        let selected = item.itemState & ODS_SELECTED != 0 && !notice;
        let colors = self.colors;
        let (background, foreground, muted) = if selected {
            (
                colors.hover_background,
                colors.hover_foreground,
                colors.hover_foreground,
            )
        } else {
            (
                colors.strip_background,
                colors.editor_foreground,
                colors.muted_foreground,
            )
        };
        let dc = item.hDC;
        let padding = self.layout.map_or(8, |layout| layout.edit.left - 1);
        let mut text = RECT {
            left: item.rcItem.left + padding,
            right: item.rcItem.right - padding,
            ..item.rcItem
        };
        let font = unsafe { SendMessageW(self.list, WM_GETFONT, 0, 0) } as HFONT;
        let bold = self.bold_font(font);
        unsafe {
            fill(dc, item.rcItem, background);
            SetBkMode(dc, TRANSPARENT as i32);
        }
        let previous = (!font.is_null()).then(|| unsafe { SelectObject(dc, font) });
        match row {
            PickerRow::Note { found, .. } => {
                draw_runs(dc, &mut text, &found.name, &found.name_hits, font, bold, foreground);
                if !found.folder.is_empty() {
                    text.left += padding;
                    draw_runs(
                        dc,
                        &mut text,
                        &found.folder,
                        &found.folder_hits,
                        font,
                        bold,
                        muted,
                    );
                }
            }
            PickerRow::Notice(label) => draw_runs(dc, &mut text, label, &[], font, bold, muted),
            other => {
                let label = self
                    .picker
                    .as_ref()
                    .map(|picker| picker_row_label(picker, other))
                    .unwrap_or_default();
                draw_runs(dc, &mut text, &label, &[], font, bold, foreground);
            }
        }
        if let Some(previous) = previous {
            unsafe {
                SelectObject(dc, previous);
            }
        }
    }

    /// `base` in bold, made on first use and again when the list's font changes (a DPI change).
    /// A list with no font yet uses the default GUI font's metrics.
    fn bold_font(&self, base: HFONT) -> HFONT {
        let (made_for, bold) = self.bold.get();
        if made_for == base && !bold.is_null() {
            return bold;
        }
        if !bold.is_null() {
            unsafe {
                DeleteObject(bold);
            }
        }
        let source = if base.is_null() {
            unsafe { GetStockObject(DEFAULT_GUI_FONT) }
        } else {
            base
        };
        let mut font = LOGFONTW::default();
        let read = unsafe {
            GetObjectW(
                source,
                std::mem::size_of::<LOGFONTW>() as i32,
                (&mut font as *mut LOGFONTW).cast(),
            )
        };
        let bold = if read == 0 {
            std::ptr::null_mut()
        } else {
            font.lfWeight = FW_BOLD as i32;
            unsafe { CreateFontIndirectW(&font) }
        };
        self.bold.set((base, bold));
        bold
    }

    /// What the empty query field shows: the quick-open hint, nothing in other modes.
    pub(crate) fn placeholder(&self) -> Option<&'static str> {
        self.picker
            .as_ref()
            .filter(|picker| picker.kind == PickerKind::QuickOpen)
            .map(|_| QUICK_OPEN_PLACEHOLDER)
    }

    /// `WM_PAINT` for the empty query field while it has a placeholder: the hint in the muted
    /// color where typed text starts. False for any other control or mode, which paints normally.
    pub(crate) fn paint_placeholder(&self, edit: HWND) -> bool {
        if edit != self.query_edit {
            return false;
        }
        let Some(placeholder) = self.placeholder() else {
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
            fill(dc, client, self.colors.editor_background);
            let font = SendMessageW(edit, WM_GETFONT, 0, 0);
            let previous = (font != 0).then(|| SelectObject(dc, font as _));
            // Typed text starts after the Edit's left margin (the low word).
            client.left += (SendMessageW(edit, EM_GETMARGINS, 0, 0) & 0xffff) as i32;
            SetBkMode(dc, TRANSPARENT as i32);
            SetTextColor(dc, self.colors.muted_foreground);
            let mut text = placeholder.encode_utf16().collect::<Vec<_>>();
            DrawTextW(
                dc,
                text.as_mut_ptr(),
                text.len() as i32,
                &mut client,
                DT_SINGLELINE | DT_NOPREFIX | DT_END_ELLIPSIS,
            );
            if let Some(previous) = previous {
                SelectObject(dc, previous);
            }
            EndPaint(edit, &paint);
        }
        true
    }

    #[cfg(test)]
    pub(crate) fn list_text(&self, index: usize) -> String {
        use windows_sys::Win32::UI::WindowsAndMessaging::{LB_GETTEXT, LB_GETTEXTLEN};
        let length = unsafe { SendMessageW(self.list, LB_GETTEXTLEN, index, 0) };
        let Ok(length) = usize::try_from(length) else {
            return String::new();
        };
        let mut buffer = vec![0u16; length + 1];
        let copied =
            unsafe { SendMessageW(self.list, LB_GETTEXT, index, buffer.as_mut_ptr() as LPARAM) };
        buffer.truncate(usize::try_from(copied).unwrap_or(0));
        String::from_utf16_lossy(&buffer)
    }

    #[cfg(test)]
    pub(crate) fn has_bold_font(&self) -> bool {
        !self.bold.get().1.is_null()
    }
```

After `impl CommandPalette { ... }` (before `impl Drop for CommandPalette`), add:

```rust
/// `text` cut into runs of chars that are all hits or all not, in order. `hits` are ascending
/// char indices (quick_open's), never byte offsets.
fn hit_runs<'a>(text: &'a str, hits: &[usize]) -> Vec<(&'a str, bool)> {
    let mut runs = Vec::new();
    let mut start = 0;
    let mut current = None;
    for (index, (byte, _)) in text.char_indices().enumerate() {
        let hit = hits.binary_search(&index).is_ok();
        match current {
            Some(previous) if previous == hit => {}
            Some(previous) => {
                runs.push((&text[start..byte], previous));
                start = byte;
                current = Some(hit);
            }
            None => current = Some(hit),
        }
    }
    if let Some(last) = current {
        runs.push((&text[start..], last));
    }
    runs
}

/// Draws `text` from `rect.left` in `color`, the chars at `hits` in `bold` and the rest in
/// `regular`, and moves `rect.left` past what it drew. The run that reaches `rect.right` ends in
/// an ellipsis, and nothing is drawn after it.
fn draw_runs(
    dc: HDC,
    rect: &mut RECT,
    text: &str,
    hits: &[usize],
    regular: HFONT,
    bold: HFONT,
    color: u32,
) {
    let flags = DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX | DT_LEFT;
    unsafe {
        SetTextColor(dc, color);
    }
    for (run, hit) in hit_runs(text, hits) {
        if rect.left >= rect.right {
            return;
        }
        let font = if hit && !bold.is_null() { bold } else { regular };
        let mut wide = run.encode_utf16().collect::<Vec<_>>();
        let mut measured = *rect;
        unsafe {
            if !font.is_null() {
                SelectObject(dc, font);
            }
            DrawTextW(
                dc,
                wide.as_mut_ptr(),
                wide.len() as i32,
                &mut measured,
                flags | DT_CALCRECT,
            );
            DrawTextW(
                dc,
                wide.as_mut_ptr(),
                wide.len() as i32,
                &mut *rect,
                flags | DT_END_ELLIPSIS,
            );
        }
        rect.left += measured.right - measured.left;
    }
}
```

Replace `impl Drop for CommandPalette` with:

```rust
impl Drop for CommandPalette {
    fn drop(&mut self) {
        let (_, bold) = self.bold.get();
        unsafe {
            DeleteObject(self.field_brush);
            DeleteObject(self.list_brush);
            if !bold.is_null() {
                DeleteObject(bold);
            }
        }
    }
}
```

In `palette_control_proc`, replace

```rust
        (PaletteControl::List, WM_LBUTTONDOWN | WM_LBUTTONDBLCLK) => {
            if super::main_window::select_command_palette_row(parent, lparam) {
                super::main_window::run_command_palette_selection(parent);
            }
            return 0;
        }
        _ => {}
    }
    unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
}
```

with

```rust
        (PaletteControl::List, WM_LBUTTONDOWN | WM_LBUTTONDBLCLK) => {
            if super::main_window::select_command_palette_row(parent, lparam) {
                super::main_window::run_command_palette_selection(parent);
            }
            return 0;
        }
        // The empty field shows quick open's hint (EM_SETCUEBANNER needs ComCtl32 v6).
        (PaletteControl::Query, WM_PAINT)
            if unsafe { GetWindowTextLengthW(hwnd) } == 0
                && super::main_window::paint_palette_placeholder(parent, hwnd) =>
        {
            return 0;
        }
        _ => {}
    }
    // The Edit repaints only the text it changes; the hint must go (or come back) whole.
    let edits_text = hook.control == PaletteControl::Query
        && matches!(
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

**`src/window/main_window.rs`:**

After `pub(crate) fn paint_find_placeholder`, add:

```rust
pub(crate) fn paint_palette_placeholder(hwnd: HWND, edit: HWND) -> bool {
    with_command_palette(hwnd, |palette| palette.paint_placeholder(edit)).unwrap_or(false)
}
```

At the end of `open_quick_open`, replace

```rust
    refilter_command_palette(hwnd);
    with_command_palette(hwnd, CommandPalette::focus_query);
}

/// The quick-open rows for `query`
```

with

```rust
    refilter_command_palette(hwnd);
    with_command_palette(hwnd, CommandPalette::focus_query);
    // The field may have been empty already: the hint appears without an edit to trigger it.
    with_command_palette(hwnd, CommandPalette::invalidate);
}

/// The quick-open rows for `query`
```

At the end of `show_command_palette`, replace

```rust
    refilter_command_palette(hwnd);
    with_command_palette(hwnd, CommandPalette::focus_query);
}

/// Opens the palette in picker mode
```

with

```rust
    refilter_command_palette(hwnd);
    with_command_palette(hwnd, CommandPalette::focus_query);
    // Leaving quick open with an empty field: the hint must go.
    with_command_palette(hwnd, CommandPalette::invalidate);
}

/// Opens the palette in picker mode
```

**`README.md`:**

After the bullet that starts `- **Ctrl+Shift+H** opens a replace field` (ending `...in the find bar's Replace too.`), add:

```markdown
- **Ctrl+P** opens a note by typing part of its name or folder, its letters in order, as in
  VS Code. With nothing typed it lists the notes open in tabs, the most recent first, so
  Ctrl+P then Enter goes back to the previous note. Add `:42` to open a note at line 42, or
  type `:42` alone to go to that line in the current tab.
```

In `### Tabs, done right`, replace `the empty tab bar for a new tab, scroll the wheel over the tabs to browse them. Open a file from` with `the empty tab bar for a new tab, scroll the wheel over the tabs to browse them, and close one with **Ctrl+W** or a middle-click. Open a file from`.

In the shortcut table, replace `| Replace in the selected result | \`Ctrl+Shift+1\` | | | |` with:

```markdown
| Replace in the selected result | `Ctrl+Shift+1` | | Go to note | `Ctrl+P` |
| Close tab | `Ctrl+W` or middle-click | | | |
```

**Spec:** append to `docs/superpowers/specs/2026-09-24-quick-open-design.md`:

```markdown

## 9. Implementation notes

- **Case folding** maps each char to the first char of its lowercase form (`İ` → `i`), so hits stay char indices of the original name and folder, as §6 asks.
- **`split_line`:** `a:` drops the trailing colon (`a` is still matched, so the list doesn't empty while the number is typed); `a:x` is text; a line too large for `u32` goes to the last line.
- **Targets:** a term is tried on the name first; only when that fails, and only for a note with a folder, on `folder\name`. The joining `\` is never a hit.
- **`:<n>` alone** shows `Go to line <n>` even with no notebook open, since it needs none; every other query shows `No notebook is open` then.
- **Enter with nothing to pick** (the notice row, or no rows) leaves the picker open.
- **Selection before anything is typed:** the second row only when the first is the active tab's note.
- **Activation order:** when the active tab closes, the tab that takes its place becomes active and moves to the front; a document replaced in place (the preview, a reused untitled tab) leaves the order and its replacement takes the front. A session restore restarts the order from the strip, the active tab first, once every tab has reopened.
- **A note gone from the library at Enter** shows `FastPad could not open <path>: the note is no longer in the notebook`, through `report_open_failure`.
- **The hint** is painted by the field's hook, like the find bar's: FastPad has no ComCtl32 v6 manifest, so `EM_SETCUEBANNER` would show nothing.
- **Screen readers:** every picker's list-box strings now carry the row text (they were empty), so the older pickers' rows are read too.
- **Ctrl+W in the palette:** `TranslateAcceleratorW` runs before the field's hook, so `translate_accelerator` leaves Ctrl+W alone for the palette's controls; the hook closes the palette and swallows the 0x17 character.
- **Middle-click:** the press remembers the tab's index and document; the release closes only over the same index while it shows that document, and `WM_MOUSELEAVE` forgets the press. Tabs answer `HTCLIENT`, confirmed in `titlebar::nonclient_hit_test`.
- **Placement:** the palette row `Go to note…` has no category prefix and follows `File: Open recent notebook...`; the File menu item is `&Go to note…` with `Ctrl+P`, and File → Close tab now shows `Ctrl+W`. `QuickOpen` is not a sidebar command, so it is listed with notes mode off.
```

- [ ] **Step 4: Run the tests and see them pass**

Run: `cargo test --lib window::command_palette`
Expected: all pass.
Run: `cargo test --lib window::main_window::tests::quick_open_rows_carry_their_text -- --test-threads=1`
Expected: 1 passed.
Run: `cargo test --lib window::main_window::tests::a_picker_lists_its_items -- --test-threads=1`
Expected: 1 passed (the older picker still works with real strings).

- [ ] **Step 5: Clippy**

Run: `cargo fmt; cargo clippy --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 6: Commit**

```bash
git add src/window/command_palette.rs src/window/main_window.rs README.md docs/superpowers/specs/2026-09-24-quick-open-design.md
git commit -m "feat(palette): quick-open rows drawn with bold matched letters and a muted folder, the painted \"Go to note by name\" hint, row text for screen readers in every picker; README shortcuts and the spec's implementation notes"
```

---

## Final review

- [ ] Run the full suite once: `cargo test -- --test-threads=1`. Expected: all pass.
- [ ] Run `cargo clippy --all-targets -- -D warnings`. Expected: no warnings.
- [ ] Re-run the bench from Task 1, Step 5. Expected: `quick_open_ms` under 5.00.
- [ ] Before any live check of the real exe, back up `%LOCALAPPDATA%\FastPad\fastpad.ini` and restore it afterwards.
