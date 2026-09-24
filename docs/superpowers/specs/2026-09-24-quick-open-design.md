# Quick open (Ctrl+P), Ctrl+W and middle-click close: design

**Branch:** `feat/quick-open`, stacked on `feat/note-replace` (PR #12). One PR.
**Follows:** `2026-09-24-note-search-design.md`, which moved finding a note by name out of the Search view and into Ctrl+P.
**Followed by:** links and backlinks (sub-project 4).

## 1. Goal

- Open a note by typing part of its name, the way VS Code's Ctrl+P opens a file.
- Close tabs the way VS Code does, with Ctrl+W and a middle-click.

## 2. Decisions

| Question | Decision |
|---|---|
| Where the picker lives | In the command palette, as a new picker kind. No new window. |
| Matching | Fuzzy, as in VS Code: the typed letters appear in order in the name or folder path. Replaces `library::name_search`. |
| Before anything is typed | The notes open in tabs, most recently used first. That order is in memory only, and no history is saved. |
| History of closed notes | None for now. |
| `name:42` and `:42` | Supported. The note opens at line 42. |
| Ctrl+W | The existing Close tab command. |
| Middle-click | Closes the tab under the pointer. It switches to that tab first only when it has unsaved changes. |

## 3. Ctrl+P

### 3.1 Opening

- **Ctrl+P**, the palette row **"Go to note…"** (the shortcut is shown beside it) and the File menu item **"Go to note…"** all open the palette in the `QuickOpen` picker.
  - They run the new command `CommandId::QuickOpen`.
- The query field is empty. When the field is empty, its hint (cue banner) reads `Go to note by name`.
- Pressing Ctrl+P while the picker is open does nothing new; the picker stays as it is.
- Escape or a click outside closes the picker, as it closes every palette mode.
- **With no notebook open,** the list has one row that can't be picked: `No notebook is open`. Enter does nothing.

### 3.2 Nothing typed

- The rows are the notes open in tabs, most recently used first.
- **What counts:** only tabs whose file is a note in the current notebook. Untitled tabs and files outside the notebook are left out.
- **The order** comes from a new activation list on `Tabs`. It is kept in memory and never saved.
  - Activating a tab moves it to the front.
  - A new tab enters at the front, because opening a tab activates it.
  - A closed tab leaves the list.
  - At startup, restored tabs enter in strip order with the active tab at the front.
- **Selection:** the current note is the first row, and the selection starts on the second row when there is one. So Ctrl+P then Enter switches to the previous note.
- With no note open in a tab, the list is empty.

### 3.3 Typing

- **What's searched:** every note in `LibraryState.notes`, the list already in memory. No disk access.
- **Each note is matched against** its name (the file stem, as the tree shows it) and its folder path relative to the notebook, joined with `\`.
- **Spaces** split the query into terms. Every term must match (AND), each scored on its own and summed.
- A term matches when its letters appear in order, ignoring case, either:
  - in the name alone, which is a name match; or
  - in `folder\name`, which is a path match.
- **Scoring,** for each character matched:
  - +1 per character;
  - +5 when it follows the previous matched character directly;
  - +8 when it starts a word: first character, after a space, `-`, `_`, `.`, `\` or `/`, or an uppercase letter after a lowercase one;
  - +4 more when it's the target's first character.

  A term's score is the best alignment's score. A name match gets +20 per term.
- **Ranking:**
  1. Notes where every term is a name match come before notes with a path match.
  2. Then higher score.
  3. Then the shorter name.
  4. Then natural name order, then natural folder order (the root first), then path.
- At most 50 rows are shown.
- **Rows:**
  - The name comes first, with matched letters in bold.
  - The folder follows in the muted colour, also with matched letters in bold. A note at the root shows no folder.
  - The picker's rows are drawn owner-draw, reusing the palette's row style.
- The selection starts on the first row.

### 3.4 Line suffix

- A query ending in `:<digits>` goes to that line. The suffix is removed before matching.
  - `meet:42` shows the notes matching `meet`. Picking one opens it and moves the caret to the start of line 42.
  - A line past the end goes to the last line.
- A query that is only `:<digits>` shows one row, `Go to line <n>`. Picking it moves the current tab's caret, and does nothing when no tab is open.
- **The go-to-line step** uses Scintilla's `SCI_GOTOLINE` (the line is 1-based in the query and 0-based to Scintilla) and scrolls the caret into view.

### 3.5 Picking

- Enter or a double-click opens the selected row with `open_note(hwnd, path, OpenMode::Permanent, true)`. A note already open in a tab is switched to.
- The line suffix is then applied.
- **A note that has left the library:**
  - A note can vanish between listing and picking; the pick resolves the path again against `LibraryState.notes`.
  - If the note is gone, the open error shows through the existing open-failure notice.

### 3.6 Latency

- Matching runs on the UI thread on each keystroke, over paths already in memory.
- **Budget:** the `library-scan` bench's `name_search_ms` case becomes `quick_open_ms`. It runs the new matcher over the bench's notes with the query `nt 12` and must stay under 5 ms.
- Nothing new runs before first paint. The palette window is still created on first use.
- `Tabs`' activation list is a `Vec<DocumentId>`, updated in `activate`, `push` and the close paths.

### 3.7 Screen readers

- The list is the palette's existing list box, so each row is read as its name and then its folder (`meeting notes, in Work\2026`).
- The disabled `No notebook is open` row is read as that text.

## 4. Ctrl+W

- Add the accelerator `FCONTROL, 'W'` → `CommandId::CloseTab`.
- It behaves exactly like File → Close tab, save prompt included.
- It works from the editor, the sidebar and the Search view, like every accelerator today.
- **In the palette's query field,** the palette's key hook gets Ctrl+W first, and Ctrl+W there closes the palette, not a tab.

## 5. Middle-click

- `WM_MBUTTONDOWN` on a `Tab(i)` or `CloseTab(i)` hit target remembers `i`.
- `WM_MBUTTONUP` over the same tab closes tab `i`; releasing anywhere else does nothing.
- **A clean tab** closes without becoming active. The active tab stays active, unless it is the one being closed.
- **A tab with unsaved changes** is activated first and then closed with the usual prompt, so the prompt's tab is on screen. After Cancel, it stays active.
- **Mechanism:** a new `close_tab_at(hwnd, index)` in `main_window.rs`.
  - It closes a clean background tab through the same path as a reviewed close.
  - Otherwise it activates the tab and runs `CommandId::CloseTab`.
- **Hit testing:** tabs answer `HTCLIENT`, so the messages arrive as client-area `WM_MBUTTON*`. The implementation confirms this.
- **Caption:** middle-click on the caption and the activity bar's logo square keeps the system behaviour.

## 6. Tables and names

- **New:** `CommandId::QuickOpen`.
- **Table sizes:** COMMANDS +1, ENTRIES +1 (`"Go to note…"`), accelerators +2 (Ctrl+P and Ctrl+W).
- **New:** `PickerKind::QuickOpen`.
- **New module `src/library/quick_open.rs`,** replacing `name_search.rs`. It is pure: no Win32, no disk.
  - `pub fn search(notes, query, limit) -> Vec<QuickMatch>`
  - `QuickMatch { path, name, folder, name_hits: Vec<usize>, folder_hits: Vec<usize> }`. The hits are char indices into `name` and `folder`.
  - `pub fn split_line(query) -> (&str, Option<u32>)`.
- `folder_of` moves into `quick_open.rs`, and `text_search.rs` imports it from there.
- `README.md` lists Ctrl+P, Ctrl+W and middle-click.

## 7. Testing

- **Unit tests in `quick_open`:**
  - letters in order;
  - AND across terms;
  - a name match before a path match;
  - contiguous and word-start matches before scattered ones;
  - the hits' positions;
  - the 50 cap;
  - `split_line` on `a:12`, `:12`, `a:`, `a:x` and `12`.
- **`Tabs` activation order:** activate, push, close and startup order.
- **Tables:** the accelerator for Ctrl+P and Ctrl+W, the palette row and the command lookup.
- **Window tests,** with a scratch profile and notebook and `--test-threads=1`:
  - Ctrl+P then Enter switches to the previous note;
  - typing then Enter opens a closed note;
  - `name:3` puts the caret on line 3;
  - Ctrl+W closes the active tab;
  - a middle-click on a clean background tab closes it and keeps the active tab;
  - a middle-click on a dirty background tab activates it and prompts;
  - with no notebook open, the picker shows its one disabled row.
- **Bench:** `quick_open_ms` under 5 ms.
- **Checks:** clippy and the targeted tests while working, then the full suite once at the end.

## 8. Out of scope

- A saved history of opened notes, and a "recently opened" section.
- Matching files outside the notebook.
- Symbol search (`@`) and commands (`>`) typed into the picker.
- Keeping Ctrl held while pressing P repeatedly to step through the list.

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
