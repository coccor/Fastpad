# Note Search Design

**Status:** Approved design, pending review of this document
**Sub-project:** 3 of the notebook feature, delivered as two stacked PRs:
- **3a, text search** (`feat/note-search`, stacked on `feat/note-sidebar`, PR #10)
- **3b, replace across notes** (`feat/note-replace`, stacked on 3a)

**Builds on:** `docs/superpowers/specs/2026-09-23-note-sidebar-design.md` ("the sidebar spec"), which built on `2026-09-23-note-library-design.md`.

**Followed by:** Ctrl+P quick open by note name (its own small PR, stacked on 3b), then links and backlinks (sub-project 4).

## 1. Purpose

The sidebar's Search view matches note names. This sub-project turns it into a VS Code-style search of the text of every note in the open notebook:
- **3a** adds the search, with the match case, whole word and regular expression options, shared with the editor's find bar.
- **3b** adds replacing across notes.

Finding a note by name moves to Ctrl+P, a separate PR, so the Search view no longer matches names at all.

The main uses:
- **Finding the note:** you type a phrase you remember, see which notes contain it, and open one.
- **Finding the place:** opening a result puts the query in the find bar, so F3 steps through every match in that note.

## 2. Goals and non-goals

### Goals
- Search the text of every note in the open notebook as you type. Results stream in, and typing never blocks.
- Show one result row per matching note, with a snippet of the first matching line.
- Offer match case, whole word and regex toggles, in both the Search view and the find bar.
- Ctrl+Shift+F shows Search. A single-line selection in the editor fills the box.
- Opening a result hands the query and its options to the find bar and selects the first match.
- **3b:** Ctrl+Shift+H replaces across notes. Open tabs change in the editor, where Ctrl+Z undoes it. Closed notes are written in place, with a conflict check and a confirmation first.
- No index, no cache and no memory kept between searches. Nothing is added to startup.

### Non-goals
- Name matching in the Search view. That becomes Ctrl+P, a separate PR, which reuses `library::name_search`.
- Include and exclude globs, or searching outside the open notebook.
- A persistent or in-memory text index. §14 records when one would be justified.
- Listing every matching line per note. The find bar covers stepping through matches.
- Preserve-case replace and multi-line replace previews.
- Saving the search options to `fastpad.ini`. They last for the session.

## 3. Decisions made during design

| Question | Decision |
|---|---|
| What is a result? | One row per note: the name, its folder, and a snippet of the first match (option C). Opening it seeds the find bar. |
| Multi-word queries | Exact phrase: a substring, or a regex when regex is on. There is no "all the words anywhere" mode. |
| How text is searched | The files are read for each query on a worker thread. There is no index. |
| Name matches in Search | Removed. Ctrl+P (a separate PR) finds notes by name. |
| Options | Match case, whole word and regex, like VS Code, in both Search and the find bar. |
| Replace | Ctrl+Shift+H, a separate PR (3b) stacked on 3a. One spec covers both. |
| Search shortcut | Ctrl+Shift+F. Format JSON moves to Shift+Alt+F. Ctrl+K is removed. |
| Selection on Ctrl+Shift+F | A single-line selection replaces the box's text. |

## 3a — Text search

## 4. The Search view

```
┌ SEARCH ─────────────────────────┐
│ [invoice march      Aa ab .* ]  │
│ 3 notes                         │
│ 📄 Q1 budget   work             │
│    …paid the invoice march 3…   │
│ 📄 Todo                         │
│    …send invoice March to Ana   │
│ Searching… 4,120 of 9,800       │
└─────────────────────────────────┘
```

- **The search box**
  - Its placeholder is "Search text in <notebook>". It is focused when the view opens.
  - Three toggle buttons sit inside the right end of the field:
    - **Match case** (`Aa`, Alt+C)
    - **Match whole word** (`ab` underlined, Alt+W)
    - **Use regular expression** (`.*`, Alt+R)
  - Each toggle shows its state with the accent color and a filled background. Each has a tooltip that includes its shortcut, and each is exposed to screen readers as a check button.
  - The Alt shortcuts work only while focus is in the box or the results. They are handled before the menu band's mnemonics.
- **Summary line:** under the box, it reads "N notes" (or "500+ notes" at the cap, §7). With nothing typed it's empty. When the search finishes with no matches it reads "No notes match."
- **Pattern errors:** an invalid regex shows its error in the error color instead of the summary, for example "Unclosed group". The previous results stay and nothing runs. A pattern that can match empty text is rejected with "The pattern matches empty text."
- **Result rows** are two lines tall and all the same height, so `row_list` is unchanged.
  - Line 1 is the file icon, the note name, and its folder relative to the notebook in dim text (empty at the root).
  - Line 2 is the snippet, with the first match on that line drawn in bold.
- **Order:** results are in natural name order, then by folder with the root first, then by path, the same key as `name_search`'s tie-breaks. They are placed in sorted order as they arrive. The selection and scroll position are kept by path, so rows arriving never change what Enter opens.
- **Status line** at the bottom of the list area:
  - While the search runs: "Searching… N of M", where N counts notes processed, including skipped ones.
  - When it finishes with skipped notes: "N notes weren't searched". Its tooltip lists the counts by reason (online only, larger than 4 MB, couldn't be read, not text).
  - When it finishes with nothing skipped, the line is hidden.
- **Keys:**
  - Down arrow moves from the box into the results.
  - Enter or a click opens the note, following the preview-tab rules (sidebar spec §6.4).
  - Esc in the box clears it. If it's already empty, Esc returns focus to the editor.
- **The query:**
  - It stays until the notebook changes or you clear it.
  - Text search starts at 2 characters. With 1, the view shows "Type at least 2 characters."
  - The regex rule is the same: at least 2 characters of pattern.
- **With no notebook open:** "Open a notebook to search it." The box and the toggles still work, so the options can be set.

## 5. Keyboard entry points and commands

- **Ctrl+Shift+F** (`ShowSearchView`) shows the sidebar and the Search view, focuses the box and selects its text.
  - If the active editor has a non-empty selection within one line, that text replaces the box's text and the search runs at once, with no debounce. With regex on, the text is escaped (`regex::escape`) first.
  - A selection spanning lines is ignored, and the box keeps its text.
- **Format JSON** moves from Ctrl+Shift+F to **Shift+Alt+F**. The menu label and palette row change to match.
- **Ctrl+K no longer shows Search.** Its accelerator is removed.
- **Palette:** "View: Show search" shows Ctrl+Shift+F. New palette rows cover the options, "Search: Toggle match case", "Search: Toggle whole word" and "Search: Toggle regular expression", with no accelerators. They toggle the Search view's options.
- **With notes mode off,** Ctrl+Shift+F and Ctrl+Shift+H do nothing, as with the other sidebar commands (`CommandId::is_sidebar()`).

## 6. Matching

- **The phrase** is the query as typed, including its spaces. It is not trimmed, so a search for "` foo`" finds a leading space. A query that is entirely white space doesn't run.
- **Plain mode**
  - It searches for the substring.
  - Without match case, both sides are folded one character at a time with `char::to_lowercase` when that gives a single character, and to themselves otherwise. Match offsets then map back to the original text one character for one. The folding is Unicode-aware for characters that fold to a single character (É/é, Ж/ж). Special cases like `ß` aren't treated as `ss`.
- **Whole word**
  - A match counts only if the characters just before and just after it are not word characters. Word characters are alphanumerics and `_`, by `char::is_alphanumeric`.
  - In regex mode, the pattern is wrapped as `\b(?:…)\b`.
- **Regex mode**
  - It uses the `regex` crate, pinned as `=1.13.1` (the version already in the cargo registry), default features. The pattern is compiled with `RegexBuilder` and `case_insensitive(!case)`, using the crate's default size limit.
  - The crate runs in linear time, so there is no catastrophic backtracking.
- **Lines**
  - Matching is per line: lines are split at `\n`, and a trailing `\r` is removed first. A match never spans lines.
  - In regex mode, a pattern containing `\n` (literally, or the escape `\n`) is matched against the whole text instead.
- **Empty matches** are rejected when the pattern is compiled, by testing whether it matches `""`.
- **The same `Matcher`** is used by the Search worker, by the find bar's regex mode (§8) and by 3b's replace.

## 7. Search execution

**The worker loop.** A search runs on a new `std::thread` per query and stops at the first of:
- it has visited every note;
- the cancel flag is set;
- 500 notes have matched (the cap).

For each note, in the library's note-list order:

1. Skip the note, counting it by reason, if it is:
   - `online_only` (it is never opened, so a search never recalls it);
   - larger than **4 MB** (4,194,304 bytes, by the scan's size);
   - not decodable by `file::encoding::decode` ("not text");
   - unreadable, for any other I/O error.
2. **Get the text:**
   - If the path is in the overlay map (see Overlays below), use that text.
   - Otherwise `std::fs::read` the note and `encoding::decode` it.
3. **Find the first match.**
   - If there is one, build the snippet: the matching line, cut to at most **40 characters before** the match and **80 characters after it**, measured in characters. An ellipsis `…` marks each cut, and leading white space is removed.
   - The result is `TextHit { path, name, folder, snippet, highlight: Range<usize> }`, where `highlight` is a byte range within `snippet`.
4. **Send a batch** to the UI when it holds 50 hits, or when 50 ms have passed since the last one, and when the search finishes. Each batch also carries the progress counts (visited, total, skipped by reason).
5. Check the cancel flag between notes.

**Overlays.**
- When a search starts, the UI thread copies the text of every **dirty** tab whose path is inside the notebook, into `HashMap<PathBuf, String>` keyed by the path relative to the notebook.
- Clean tabs are not copied, because the file on disk is the same text.
- An untitled tab doesn't belong to the notebook and isn't searched.

**The note list.**
- It is copied at the start as `Vec<SearchNote { path, size, online_only }>`, about 1 MB for 10,000 notes.
- The worker owns it and frees it at the end.
- Nothing is kept between searches.

**Narrowing.** In plain mode with the same options, if the new query contains the previous query, and the previous search finished without being cancelled or capped, the new search visits only the previous hits.

**Scheduling: `src/window/text_search_host.rs`, owned by the window.**
- It owns the current search: a generation number, an `Arc<AtomicBool>` cancel flag, and the 150 ms debounce timer (`SetTimer` on the panel, with the timer ID in `ids.rs`).
- **Starting a search:**
  - A keystroke in the box, or a toggle change, cancels the running search and restarts the debounce.
  - When the timer fires, the host builds the overlays and the note copy, bumps the generation, and spawns the worker.
  - A toggle change, or text arriving from Ctrl+Shift+F, starts the search at once, with no debounce.
- **The worker posts** `WM_FASTPAD_TEXT_SEARCH_BATCH = WM_APP + 13` with a boxed `SearchBatch { generation, hits, progress, done }`.
  - The handler frees the box, and drops the batch if its generation isn't the current one.
  - If posting fails because the window is gone, the worker frees the box itself, as `spawn_load` does.
- **Cancellation:** a new query, a notebook change or close, and window destruction all set the cancel flag.
- **A library change** (`LIBRARY_READY`, or a save that adds or removes a note):
  - If the Search view is showing a query, it runs again.
  - If the view is hidden, the results are marked stale and run again when the view shows, as the sidebar spec's §16 note says.
  - A save of a note that is already listed doesn't re-run the search.
- **UI-thread work per batch:** a binary-search insert into the sorted result `Vec`, then one `InvalidateRect` if a visible row, the summary or the status changed. There is no disk access, and no step that grows with the notebook.

## 8. The find bar

- It gets the same three toggles, drawn and exposed the same way as in the Search box, with Alt+C, Alt+W and Alt+R while focus is in the find bar.
- **Plain mode** searches with Scintilla, with these flags wherever `search_flags` is used: find next and previous, replace, and replace all in the editor.

  | Option | Scintilla flag |
  |---|---|
  | Match case | `SCFIND_MATCHCASE` |
  | Whole word | `SCFIND_WHOLEWORD` |

- **Regex mode** uses the same `Matcher` as Search (§6), built from the find bar's query and options, over the document's UTF-8 text. The text is borrowed from Scintilla with `SCI_GETCHARACTERPOINTER`, so nothing is copied, and the `Matcher`'s byte offsets are Scintilla positions.
  - **Find next** selects the first match that starts at or after the selection's end, and wraps once to the first match in the note.
  - **Find previous** selects the last match that ends at or before the selection's start, and wraps once to the last match.
  - **Replace** replaces the selection only if it is exactly the match found where it starts.
  - **Replace all** replaces every match, from the last backwards, as one undo action. The replacement is literal text (no `$1`).
  - With one engine, a regex Search result always opens to the match Search showed: the same dialect, case folding, whole word and lines.
- **Pattern errors:** a pattern that doesn't compile, or that matches empty text, shows the find bar's no-match state, as Search shows its pattern error (§6).
- **Opening a Search result:**
  - The find bar opens in Find mode with the Search query and **Search's options**, which then become the find bar's options.
  - It selects the first match at or after the start of the document, and scrolls it into view.
  - Focus goes to the editor, so F3 and Shift+F3 step through the matches straight away.
  - This works the same for preview and normal tabs.
- **Options between the two:** each surface keeps its own options for the session. Only opening a result copies Search's options into the find bar.

## 9. Code layout (3a)

- **`src/search/mod.rs`, `src/search/matcher.rs`** (new, pure)
  - `MatchOptions { case: bool, whole_word: bool, regex: bool }`
  - `Matcher::new(&str, MatchOptions) -> Result<Matcher, PatternError>`
  - `Matcher::first_in(&str) -> Option<Range<usize>>`
  - `Matcher::find_iter`, used by 3b
  - `PatternError` implements `Display` with the message shown under the box.
- **`src/search/snippet.rs`** (new, pure): cutting the line and computing the highlight range.
- **`src/library/text_search.rs`** (new, no Win32): `SearchNote`, `TextHit`, `SearchBatch`, `Progress`, `SkipReason`, and `run(notebook, notes, overlays, matcher, narrow, cancel, sink)`. It is tested over a scratch folder.
- **`src/window/text_search_host.rs`** (new): the debounce, generation, spawning, the batch message and overlays. This keeps `library_host.rs` from growing past its current 2,200 lines.
- **`src/window/search_view.rs`**
  - It stops using `name_search`.
  - It gets two-line rows, the toggle buttons in the field, the summary line, the pattern error and the status line.
  - Its toggle buttons and summary are added to the panel's accessible children.
- **`src/window/find_bar.rs`**: the toggle buttons, the plain-mode flags, the regex-mode search over the document, and `show_with(query, options)`.
- **`src/window/menus.rs` and `commands.rs`**: Ctrl+Shift+F, Shift+Alt+F, Ctrl+K removed, and the three palette rows.
- **`Cargo.toml`**: `regex = "=1.13.1"`.
- **`src/library/name_search.rs`** stays for Ctrl+P. It keeps its unit tests and the `library-scan` bench's `name_search_ms` case, which keep it compiled and warning-free; nothing else changes in it.

## 10. Accessibility (3a)

- The toggles are check buttons (`ROLE_SYSTEM_CHECKBUTTON`, with the `STATE_SYSTEM_CHECKED` state), named "Match case", "Match whole word" and "Use regular expression".
- Each result's accessible name is "<name>, <folder>: <snippet>".
- The summary line and the status line send `EVENT_OBJECT_NAMECHANGE` when their text changes, at most once per second while a search runs.
- This also closes the sidebar PR's known limitation: the search box itself becomes one of the panel's accessible children.

## 3b — Replace across notes

## 11. The replace UI

- **Ctrl+Shift+H** (`ReplaceInNotes`) shows Search with the replace field open, and focuses the replace field. A chevron button left of the search box opens and closes the replace field, as in VS Code. Ctrl+Shift+F leaves the field as it is.
- **The replace field** has the placeholder "Replace". In regex mode, `$1`, `${name}` and `$$` expand, using `regex::Captures::expand`. In plain mode the text is literal.
- **Replace all** is a button at the right of the replace field, and Ctrl+Alt+Enter while focus is in either field. It replaces in every result. It's disabled while a search is running, and when there are no results.
- **Per-row replace:** hovering over or selecting a result shows a replace button on the row. It replaces in that note only, and the row then disappears.
- **Confirmation** (`modal::confirm`):
  - Replace all asks "Replace N matches in M notes with "<text>"?". If any of those notes aren't open, the prompt adds "Notes that aren't open are saved and can't be undone."
  - A per-row replace asks only if the note isn't open.
  - N is counted exactly with `find_iter` on the worker before the prompt. It's a quick second pass over the result notes only.
- **After replacing,** the query runs again, so the results show what still matches.

## 12. How replace writes

- **Open tabs** (clean or dirty):
  - The replacement is applied in the editor as one undo action (`SCI_BEGINUNDOACTION` and `SCI_ENDUNDOACTION`), by replacing each match's range from the end of the document backwards.
  - The ranges come from `Matcher` on the tab's current text, so they are the same as Search's.
  - The tab becomes dirty and is **not saved**. Autosave handles it as it would any edit.
- **Closed notes:**
  - They are written on a worker, one at a time.
  - Read the bytes and check the stamp (size and last-write time) against what the search read. The search worker records each hit's stamp in `TextHit`.
  - Decode the text, replace every match, and encode it back with the file's original `Encoding`, including its BOM. Line endings are kept because the text was never normalised.
  - Write with `file::saver::save_atomic`.
  - **A stamp mismatch** skips the note and counts it as "changed since the search".
  - **A write failure** skips the note and counts it with its error.
  - **An online-only note** is never a hit, so it is never written.
- **The report** afterwards is a notification: "Replaced N matches in M notes." It also names any notes that were skipped, for example "2 notes were skipped because they changed since the search", with the note names in the details.
- **The library** hears about the writes as it does about any external change, through its rescan and reconcile. The writes are also marked FastPad's own, so they don't show as outside changes to tabs.

## 13. Code layout (3b)

- **`src/library/text_replace.rs`** (new, no Win32): `plan_replacements` counts matches, and `apply(notebook, targets, matcher, replacement, cancel) -> ReplaceReport` writes the files.
- **`src/window/text_search_host.rs`**: running the replace, applying it to open tabs, and the report.
- **`src/window/search_view.rs`**: the chevron, the replace field, the Replace all button and the per-row button.
- **`commands.rs`, `menus.rs`**: `ReplaceInNotes` (Ctrl+Shift+H), and the palette row "Search: Replace in notes".

## 14. Performance

- **Startup:** there is no change. The search box, toggles and host state are created only when the Search view first shows a notebook, as today. `regex` is compiled only when a regex search runs.
- **Typing:** a keystroke costs only the EDIT's own work and a timer reset. Nothing runs until the 150 ms debounce ends.
- **Search**, with 10,000 notes of about 4 KB each and a warm OS cache, on the reference i5-4590:
  - the first batch posted in **under 50 ms**;
  - the whole search in **under 400 ms**.
  - A cold cache or a network share is slower. Streaming keeps the first results early.
- **UI thread:** handling one batch takes **under 2 ms**.
- **Memory:** idle working set growth is **at most 0.5 MB** over the sidebar branch. During a search, memory is bounded by the note copy (about 1 MB), one file's text, the overlays, and 500 hits.
- **Exe size:** the `regex` crate may add at most **1.5 MB** to the release exe. It is measured with the size bench and recorded in §16.
- **When an index would be justified:** if the warm full search exceeds 400 ms on the reference machine for 10,000 notes, or users hit the cold case often. The worker interface (`run` with a sink) allows an index behind it later.
- **Bench:** the `library-scan` bench gains `text_search_first_batch_ms`, `text_search_full_ms` and `text_search_batch_ui_ms`, over a generated 10,000-note notebook.

## 15. Errors and edge cases

- **The notebook is still loading:** the query waits and runs on `LIBRARY_READY`. The view shows "Loading…".
- **A notebook change or close:** the search is cancelled, the results clear, and the query clears (sidebar spec §8).
- **A tab gets dirty after the search started:** that search used the start-time text. The next keystroke or library change runs the search again. A result opened by then still lands correctly, because the find bar searches the live text.
- **A note is deleted or renamed between search and open:** opening follows the existing missing-file path, which shows a notice and runs the search again.
- **Very long lines:** the snippet is cut before painting, so painting never measures a long line.
- **Many matches in one note** cost nothing extra: only the first match is found in 3a. 3b's count pass is limited to the result notes.
- **FastPad closes during a search or replace:**
  - The cancel flag is set when the window is destroyed.
  - A replace in progress finishes the file it is writing. `save_atomic` writes a temporary file and renames it, so a file is never half-written. It then stops.
- **Regex with a `\n`** is matched on the whole text. The snippet is the first line of the match.

## 16. Testing

- **Unit tests (pure):**
  - `Matcher` for all eight option combinations.
  - Folding for Unicode letters with a single-character lowercase.
  - Whole word with `_`, digits and punctuation at both ends.
  - Regex errors, and rejecting patterns that match empty text.
  - Per-line matching, and whole-text matching for patterns with `\n`.
  - Mapping offsets back after folding.
  - Snippet cutting at both ends, leading white space, and multi-byte characters at the cut.
- **`text_search::run` over a scratch folder:**
  - Overlays take priority over the disk.
  - Online-only, oversized, binary and unreadable files are counted.
  - Cancelling stops the search promptly.
  - Narrowing visits only the previous hits.
  - The 500 cap marks the search as capped.
  - Batches go out at 50 hits and at the 50 ms mark.
  - Stamps are recorded.
- **Window tests (`--test-threads=1`):**
  - Ctrl+Shift+F with a single-line selection, a multi-line selection, and none.
  - Typing and the debounce give sorted results. The selection is kept by path while batches arrive.
  - Toggles by click, by Alt+C, Alt+W and Alt+R, and from the palette.
  - The pattern-error line.
  - Opening a result gives the find bar the query and options, selects the first match, and F3 moves on.
  - Format JSON on Shift+Alt+F. Ctrl+K no longer does anything.
  - With notes mode off, Ctrl+Shift+F does nothing.
- **End-to-end tests** run the real exe with a scratch profile and a scratch notes folder: search, open, F3.
- **3b:**
  - Replace in an open tab, then undo it with a single Ctrl+Z.
  - A closed note keeps its encoding and BOM (UTF-8, UTF-8 with BOM, UTF-16 LE) and its CRLF or LF line endings.
  - A stamp mismatch skips the note and reports it.
  - Declining the confirmation writes nothing.
  - `$1` expansion.
  - The per-row replace.
  - A replace-all end-to-end test.
- **Manual:**
  - Narrator on the toggles, the results and the status line.
  - High contrast.
  - 150% and 200% scaling.
- **All tests** use scratch profiles and scratch folders. None touch the real `%LOCALAPPDATA%\FastPad` or Documents.

## 17. Implementation notes

- **The debounce timer lives on the main window** (a deviation from §7, which said "on the
  panel, with the timer ID in `ids.rs`"). `TEXT_SEARCH_TIMER_ID` (`0x4650_5453`) is defined in
  `text_search_host.rs` and handled in the main window's `WM_TIMER`, like
  `LIBRARY_WRITE_TIMER_ID`. `ids.rs` holds library IDs, and every other timer is the main
  window's.
- **The debounce waits out modal loops and file population.** When the timer fires inside a
  nested modal loop or while a file is being populated, it does nothing and stays armed, so it
  tries again at the next tick: reading the dirty tabs swaps editor documents, which neither may
  see.
- **Plain search compiles a literal `Regex`** of the escaped query rather than using a substring
  search. Without match case it is a case-insensitive literal for an ASCII query, and otherwise
  a literal of the case-folded query matched against case-folded text.
- **Whole-text regex mode keeps `^` and `$` per line.** A regex that names a newline (a line
  break or `\n` in the pattern) is matched over the whole text in one pass, not line by line,
  and is compiled with `multi_line(true)`, so `^` and `$` still anchor at each line's start and
  end inside it rather than only at the start and end of the note.
- **`text_search::run` has no `narrow` parameter.** Narrowing passes the narrowed slice of notes
  (the previous hits plus the notes the previous search skipped, which are carried over through
  `run_noting_skipped`), and a batch carries `SearchBatch.end: Option<RunEnd>`, set on the last.
- **Narrowing is stricter than §7:** it is disabled for whole word (a longer whole word can
  match where the shorter one was part of a word) and for regex, and it also needs the same
  options and unchanged dirty-tab text. Its record includes every note's size and modification
  time, so it is dropped on any library load or rescan and on any save in the notebook
  (FastPad's own included); the next query is then a full search.
- **A library change runs the query again only when the note paths or their `online_only` flags
  changed, or after a load or rescan.** `side_panel::refresh` also runs for pins, favorites and
  expansions, and a save of a listed note changes neither (§7). The re-run is deferred through
  the debounce timer rather than started inline, so no editor document is swapped inside an
  install, save or notebook flow; it costs at most 150 ms more after a library change.
- **The UI thread's cost at search start grows with the dirty tabs and the notebook** (accepted):
  it copies the note list and each dirty tab's text for the worker before starting it. §7 allows
  it; very large dirty tabs make it noticeable.
- **Cancelling bumps the generation too,** so batches a cancelled search already posted are
  dropped, not only those of an older search.
- **A re-run keeps the old results until the first batch that carries hits or the end.** An
  interval batch with only progress in it updates the status line but never blanks the list,
  so a re-run (after a library change, say) doesn't flash an empty view.
- **A dirty tab's text is searched even when its note is online only or over 4 MB,** since it is
  already in memory. It is never counted as skipped.
- **The cap ends a search as "500+ notes" only when notes remain** after the 500th hit. When the
  500th hit is the last note, it is a completed search ("500 notes").
- **A regex error's message** is the last line of the `regex` crate's error text
  (`error: <description>`), with `error:` stripped and a capital first letter: "Unclosed group".
  A pattern too large to compile reads "The pattern is too large.", a message §4 doesn't list.
- **The snippet's 80 characters "after" the match are counted from the match's start** (§7): the
  match and what follows it together are at most 80 characters, so a match longer than that is
  itself cut with `…`.
- **`name_search::folder_of` became `pub(super)`** (§9 says nothing else changes in
  `name_search`). It is a visibility change only: text search reuses it so a result's folder
  follows the same rule as a name match's.
- **The Search box is made when the user first shows the Search view** (`search_view::shown`),
  even with no notebook, so the toggles work then (§4). `layout`, which also runs on the first
  `WM_SIZE`, makes it only with a notebook open, so a Search view restored at startup still makes
  nothing before the first paint (§14).
- **Visuals:** a toggle that is on is filled with `Palette.selection_background`, since the
  palette has no accent color. `Palette.error_foreground` is new, for the pattern error and the
  find bar's no-match outline. The bold snippet font is created lazily, on the first snippet
  paint, not in `UiFonts::create`. A Search result row is 42 px tall at 96 DPI (two 18 px line
  slots), and since `row_list::thumb_width` scales from the row height, the Search view's scroll
  thumb becomes 9 px wide (the other views keep 6 px).
- **F3 and Shift+F3 are new commands,** `FindNext` (188) and `FindPrevious` (189). §8 assumed
  they existed; nothing bound F3 before. With no query yet, they open the find bar. They're in
  the Search menu and the palette. 3b's commands start at 190.
- **The find bar gained a no-match state.** §8 speaks of its "existing" one, but a miss used to
  leave no trace. Now the query field's outline turns `Palette.error_foreground` until the query
  changes or a search finds something. A regex that doesn't compile or matches empty text
  shows it too; neither is an error or a notice.
- **The find bar's regex mode reads the document in place.** `Editor::with_document_text`
  runs a closure on the text from `SCI_GETCHARACTERPOINTER`, which closes Scintilla's gap once
  (a move of at most the document's size) and then costs nothing until the next edit. It checks
  the code page is UTF-8 and the bytes are valid UTF-8; every FastPad document is UTF-8, so
  nothing is converted, and a document that failed the check would show as no match. The
  `Matcher` is compiled on each find next, not kept between presses.
- **Find next in regex mode is `Matcher::find_at`,** which starts at the selection's end but
  judges word edges and anchors in the whole line, so `\bfoo` never matches inside "xfoo" when
  the caret sits after the x. **Find previous is `Matcher::last_before`,** the last of
  `find_iter`'s matches that ends by the selection's start, tried from that line backwards.
- **Measured:** a regex find next that misses in a 1 MB note (the worst case: the text after
  the caret, then all of it again after the wrap, with the pattern compiled) takes about 3 ms in
  a release build (`a_regex_find_next_in_a_megabyte_note_takes_well_under_a_frame`, which
  asserts under 8 ms in release).
- **Plain mode keeps Scintilla's own case folding and word test.** Both are Unicode-aware in a
  UTF-8 document, so they agree with Search's plain matching on ordinary text; Search's
  one-character folding (§6) may still differ from Scintilla's at rare characters.
- **Replace in the find bar inserts its text literally,** in both modes (no `$1` or `\1`). A
  plain match's length is Scintilla's `SCI_GETTARGETEND`; a regex match's is the `Matcher`'s.
- **A click on a result moves the focus to the editor; Enter keeps it in the list** (sidebar
  spec §6.4; `open_search_result`'s `focus_editor` is true for a click, false for Enter). §8's
  "focus goes to the editor" holds for the click. Either way the first match is selected and F3
  continues from it.
- **Opening a result seeds the find bar from what the results ran with**: the view's `query`
  and `run_options` (the options when that search began), not the box's current text or
  toggles. A query or option edited after the search, still inside its debounce, never leaks
  into the find bar, so F3 steps through the same matches the result showed.
- **A selection prefilled into the find bar while regex is on** is escaped with
  `search::escape` (`regex::escape`), as Search's prefill is.
- **The find bar's tooltip is updated under a shared App borrow** (`TTM_*` sends in
  `FindBar::layout`), an accepted exception to the App-borrow rule: the tooltip is a same-thread
  control that never calls back into the main window.
- **Accessibility:** the Search view's children are the box (its full object is the `Edit`'s
  own), the three toggles (check buttons), the summary line, the status line, then the results.
  The status line comes before the results so its child ID stays fixed while they stream in. A
  result reads its snippet; at the root, where there is no folder to add, it is named
  "<name>: <snippet>". The summary and status lines announce a change at most once a second. The
  find bar's panel now has a provider too: the Find field, the toggles, the Replace field in
  Replace mode, and Close.
- **`text_search_batch_ui_ms` times the sorted insert only.** It inserts 50 hits into 450 by
  binary search, calling `hit_cmp` directly, since the bench can't reach `search_view`. The
  `InvalidateRect` after it only queues a paint.
- **Format JSON has no menu-bar entry with a shortcut.** The palette row shows Shift+Alt+F from
  the accelerator table; the overflow menu shows no shortcuts for any entry.
- **Measured** on the reference machine itself (Intel Core i5-4590 at 3.3 GHz, 8 GB, Windows 10
  22H2), warm cache, notebook under `target\bench-notes`, the second of two runs:
  `text_search_first_batch_ms` 5.57, `text_search_full_ms` 1133.18,
  `text_search_batch_ui_ms` 0.119. The first batch and the UI-thread batch meet their targets
  (50 and 2 ms). **The full search misses its 400 ms target by almost three times.** The first
  batch reads 50 notes in about the same time per note (0.11 ms) as the full search's 10,000, so
  the cost looks like opening and reading each file (the file system and the antivirus filter on
  every open), not matching. §14 names this as the point where an index would be justified; none
  was added in 3a. Release exe with `release-package`: 1,565,696 bytes on `feat/note-sidebar`
  (4c9ba2e), 2,833,408 bytes here (+1,267,712 bytes, budget 1,572,864). Idle private working
  set with the Search view open on a 10,000-note notebook: +86,016 bytes over
  `feat/note-sidebar` in each of two back-to-back pairs of 30 runs (p50 4,390,912 there,
  4,476,928 here; budget 524,288). `compare` found no regressed milestone in either pair.
