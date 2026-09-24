# Note Sidebar Design

**Status:** Approved design
**Sub-project:** 2 of 4 (sidebar and note list), stacked on the note library (`feat/note-library`, PR #9)
**Builds on:** `docs/superpowers/specs/2026-09-23-note-library-design.md` ("the library spec")

## 1. Purpose

The library spec gave FastPad an open folder, a metadata store and a first-save flow, with only palette commands to organize anything. This sub-project adds a UI: a VS Code-style activity bar that never expands, and one side panel beside it.

While designing it, the user changed the organizing model, and the product spec's notebooks-as-labels, tags and note favorites were dropped:

- **A notebook is a folder.** Opening a notebook opens a folder, and one is open at a time, or none. This is the library spec's "one open folder", now named a notebook in the UI.
- **Inside a notebook, the folder hierarchy is the structure.** Notes are files, subfolders are shown as a tree, and the only extra metadata is **pins**.
- **Favorites and Recent are lists of notebooks, never notes.**

The rule that notes and files are the same thing still holds, more strongly than before: a note's place in a notebook is its place on disk.

## 2. Goals and non-goals

### Goals

- A permanent, narrow activity bar with three views: **Notebook**, **Search** and **Favorites**, plus **Settings** at the bottom.
- A resizable side panel showing one view at a time, which closes when its active icon is clicked again.
- The Notebook view: the open notebook's folder tree, with an icon and a name per row and no other metadata. A pin button appears on hover, and pinned notes sort first.
- Clicking a note opens it in a preview tab that is replaced in place. Opening or editing a note never moves it in the tree.
- Favorite notebooks, recent notebooks, and a "no notebook open" state.
- Search matches note names in the open notebook. Full-text search is sub-project 3 and uses the same panel.
- Everything works from the keyboard and is exposed to screen readers.
- No cost to startup or first input.

### Non-goals

- Tags, note favorites, notebook colors and notebooks-as-labels. The library spec's parts for these are removed (§3).
- Dragging notes, in the tree or onto notebooks. Moving is done with "Move to notebook…" and in Explorer.
- Creating, renaming or deleting folders in the tree. Explorer does that, and rescans pick it up.
- Searching note text (sub-project 3) and links (sub-project 4).
- More than one notebook open at a time.

## 3. Changes to the library spec

The stacked branch makes these changes to what PR #9 built. The library spec gets a note at the top pointing here.

| Library spec | Now |
|---|---|
| §6.2: `notebook=` and `tag=` lines; `note=` lines carry a notebook, tags and flags `f/p/d` | `library.ini` version 2 keeps pins only (§5.1). |
| §6.1: files outside the folder can be organized with an absolute-path record | Only notes inside the open notebook can be pinned. There are no absolute-path records. |
| §6.3: `recent=` note lines in the per-PC library file | Removed. Recent is now a list of notebooks (§4.3). |
| §8.5: temporary organizing commands | Only "Note: Toggle pin" stays. "Note: Move to notebook…" now moves the file (§6.6). The favorite, tag and notebook commands and their pickers are removed. |
| §5: "folder" in the UI | The UI says **notebook**: "Open notebook…" (still Ctrl+Shift+O), "Close notebook", "Favorite notebooks". The code and files may keep "folder". |
| §13: sidebar folder switcher, unsaved-note entries, favorite shortcut | Delivered here as the Favorites view (§7), unsaved rows at the top of the tree (§6.2) and "Toggle favorite notebook" (§9). There is no note favorite shortcut. |

Unchanged from the library spec: scanning and its skip rules, reconciliation (which now protects pins), the inline name box, autosave, rename and delete to the Recycle Bin, the disk-change guard, and notes mode.

## 4. Notebooks

### 4.1 Opening, switching, closing

- **Opening:** Open notebook… (Ctrl+Shift+O, File menu, the Favorites view, or the "no notebook" panel) opens a folder exactly as the library spec §5 describes. That covers the command line, IPC and dropping a folder.
- **Switching:** opening another notebook first autosaves the current one's dirty notes (library spec §14).
- **Closing:** "Close notebook" (the Notebook view's "…" menu and the palette) autosaves dirty notes, flushes pending metadata, and unloads the library. Open tabs stay open, as plain files. With no notebook open:
  - Ctrl+N creates an untitled tab and Ctrl+S shows the Save As dialog, the same as with notes mode off. There is no autosave and no name box.
  - The Notebook view shows the no-notebook state (§6.8), and the Search view says "Open a notebook to search it."
  - The activity bar stays.

### 4.2 Startup

- If the last session ended with a notebook open, it is reopened as in the library spec §5 and §14 (decided on the worker, with a notice if it's missing and a fallback to `Documents\FastPad`).
- If the last session ended with no notebook open, none is opened. The fallback does not apply.
- On first run, `Documents\FastPad` opens, as before.

### 4.3 Favorites and recent (`folders.ini`)

`folders.ini` gains two kinds of line:

```
version=1
open=none
folder=<absolute path>
favorite=<absolute path>
```

- **`folder=`** is the recent list, as before: most recent first, at most 10. Closing a notebook keeps it in the list.
- **`favorite=`** lines are the favorite notebooks, at most 50. They are shown sorted by name (§7). Favoriting doesn't change the recent list.
- **`open=none`** is present only when the last session ended with no notebook open. Opening a notebook removes it.
- Paths are normalized as in the library spec §14. Unknown keys are ignored.
- **A notebook's display name is its folder's name.** Two favorites with the same folder name show their parent folder in dim text after the name.
- Favorites and recent entries are never checked for existence when listed, so an offline drive can't stall anything. Opening one that is missing shows the notice "<path> is not available" and changes nothing. There is no fallback here, unlike at startup.

## 5. Metadata

### 5.1 `library.ini` version 2

```
version=2
note=<id>|<flags>|<size>|<hash>|<path>
```

- `<flags>` is `p` (pinned), `d` (deleted, library spec §8.4), both, or `-`. A note with no flags has no record, so unpinning a note deletes its record at the next flush.
- `<path>` is relative to the notebook and is always the last field, unescaped, as before.
- Any other version, or none, makes the file unreadable, handled as the library spec §6.2 and §7.6 describe: it's never overwritten, and pinning is off with a notice. There is no migration (§16): a version 1 file is unreadable, like any other unrecognized version.

### 5.2 Per-PC library file

The library spec §6.3 file drops `recent=` lines. It gains one line per expanded subfolder:

```
expanded=<path relative to the notebook>
```

This file is written with the library spec §14 mechanism (a copy of the state on a one-off writer thread). Unknown lines are ignored.

## 6. The Notebook view

### 6.1 The tree

- **Contents:** every note the scan found, arranged by folder. A folder appears only if it contains a note, at any depth. The scan's skip rules and its 10,000-note limit still apply, and a truncated scan shows a last row, "Showing the first 10,000 notes".
- **Rows:** a note row is a file icon and the note's name (the filename without its extension, library spec §8.1). A folder row is a chevron, a folder icon and the folder's name. Rows are indented by depth.
- **Order within a folder:**
  1. Pinned notes, by name.
  2. Subfolders, by name.
  3. Other notes, by name.

  "By name" is a natural, case-insensitive order ("Note 2" before "Note 10"), with ties broken by extension, then by exact name. There are no section headers.
- **Stable positions.** Opening, selecting or editing a note never reorders anything. Only these do: a rename, a pin change, a note created or moved, or a rescan that finds changes.
- **Expansion:** the root is always expanded, and other folders start collapsed. Expansion is remembered per PC (§5.2). When the active tab is a note in the open notebook, its row is selected and its parent folders expand. This happens on every tab switch, without moving keyboard focus.
- **Truncated names** show the full name in a tooltip.

### 6.2 Unsaved notes

Untitled tabs (Ctrl+N) appear first at the root, in italics, labeled from their first line (library spec §8.1). Selecting one switches to its tab. Once it's saved it moves to its place in the tree.

### 6.3 Pins

- A pin button appears at the right end of a row on mouse hover, and on the selected row.
- Clicking it pins or unpins the note. Pinned rows always show it, filled.
- The same toggle is in the row's context menu and in "Note: Toggle pin", which works on the selected row when the panel has focus and on the active tab otherwise.
- The first pin creates `.fastpad\` (library spec §6.1). Pinning is refused, with the library spec's notices, while the notebook is loading or `library.ini` is unreadable.

### 6.4 Opening notes: the preview tab

- A single click, or Enter, opens a note in the **preview tab**. There is at most one preview tab, and its label is in italics.
  - If the note is already open in a tab, FastPad switches to that tab instead.
  - Otherwise the preview tab's document is replaced in place, keeping its position in the tab strip. If there is no preview tab yet, one is added where a new tab would go.
- **The preview tab becomes a normal tab** on a double-click of the row, Ctrl+Enter, "Open in new tab", a double-click on the tab, the first edit, or a save.
- A preview tab is never dirty, since the first edit promotes it, so replacing it never discards changes.
- Session restore brings the preview tab back as a normal tab.
- Focus: a mouse click moves focus to the editor. Enter keeps focus in the tree, so arrow keys and Enter can browse. Ctrl+Enter opens a normal tab and moves focus to the editor.

### 6.5 Header

- **Left:** the notebook's name in small capitals, with its path in a tooltip. The empty part of the header is a window drag area.
- **Right:**
  - A star (Add to / Remove from favorites).
  - **New note**, the same as Ctrl+N (§6.7).
  - **"…"** with Reveal in Explorer and Close notebook.

### 6.6 Context menus

Opened by right-click, Shift+F10 or the context-menu key.

- **Note:**
  - Open in new tab.
  - Pin / Unpin.
  - Move to notebook… (Ctrl+Shift+M).
  - Rename… (F2, the library spec's name box).
  - Reveal in Explorer.
  - Delete… (Del, the library spec's Recycle Bin flow).
- **Folder:**
  - New note here.
  - Reveal in Explorer.
- **Unsaved note:**
  - Close tab.

**Move to notebook…** opens a picker listing favorite notebooks, then recent ones (not the open one), then "Browse…". The chosen notebook's root is the destination.

- The file is moved with `rename_no_replace` (a copy and delete across volumes). A name clash, or any failure, leaves everything unchanged and shows a notice.
- The note's pin record is removed, since pins belong to a notebook.
- An open tab follows the move and becomes a plain file outside the notebook, with no autosave. This is the library spec's rule for files outside the open folder.

### 6.7 Where new notes are saved

A new note remembers a destination folder when it is created: the folder of the selected row (a folder row's own folder, or a note row's parent), or the notebook root when nothing is selected. "New note here" uses the clicked folder. The inline name box (library spec §8.2) saves into that folder. If the folder has gone, it falls back to the root.

### 6.8 No notebook open

The panel says "Open a notebook to see its notes.", shows an **Open notebook…** button, and lists **RECENT** notebooks (`folder=` entries, §4.3) with their paths in tooltips. Clicking one opens it. This is the only place the recent list appears in the sidebar. Ctrl+Shift+O, then Recent, still works as in the library spec.

### 6.9 Empty notebook

"No notes in <name> yet." with a New note button.

## 7. The Favorites view

- **Header:** FAVORITES, with an Open notebook… button.
- **Rows:** a folder icon and the notebook's name, sorted by name, with the open notebook highlighted.
  - Clicking a row opens that notebook and shows the Notebook view.
  - Hovering, or selecting, shows a filled star that removes the favorite.
  - Context menu: Open, Remove from favorites, Reveal in Explorer.
- **With no favorites:** "Star a notebook to keep it here."
- **Footer row:** "Open notebook…".

## 8. The Search view

- A search box with the placeholder "Search <notebook>", focused when the view opens.
- Results update as you type, matching the query anywhere in note names (case-insensitive). Names that start with the query come first, then the rest, each group by name.
- Each result row is the file icon, the name, and the note's folder relative to the notebook in dim text. This is the only place a path is shown, because results lose the tree's context.
- Enter or a click opens the note, following the preview-tab rules (§6.4). Down arrow moves from the box into the results.
- The query stays until the notebook changes. With nothing typed, the view is empty. When nothing matches: "No notes match."
- Sub-project 3 adds text matches below the name matches.

## 9. Layout, activity bar and settings

- **Columns:** `[activity bar | side panel | editor area]`. The tab strip and caption buttons move to the top of the editor area, and the activity bar and panel run the full window height. The find bar, name box, preview split and status bar stay inside the editor area.
- **Activity bar:**
  - 44 px wide at 96 DPI, scaled for other DPIs.
  - Icons with accessible names and tooltips: Notebook ("Notebook: <name>"), Search, Favorites, and Settings at the bottom.
  - The active view has a 2 px accent bar and full-strength color. The others are muted.
  - The strip above the first icon is a window drag area.
- **Side panel:**
  - 260 px by default, resizable from 180 to 480 px by dragging its right edge. A double-click on the edge resets it.
  - The editor area keeps at least 320 px, so the panel gives way first on narrow windows.
- **Opening and closing views:**
  - Clicking an inactive icon shows its view. Clicking the active icon closes the panel.
  - Ctrl+B reopens the last view or closes the panel.
  - Ctrl+Shift+E shows the Notebook view and moves focus into the tree. Ctrl+K showed Search and focused its box; the note search spec (2026-09-24, §5) moves this to Ctrl+Shift+F and removes Ctrl+K.
  - There is no shortcut for Favorites.
- **Settings** opens the command palette filtered to FastPad's settings commands.
- **New `fastpad.ini` keys,** saved when a drag ends or the view changes, through `change_setting`:
  - `sidebar_view`: `notebook`, `search`, `favorites` or `none`. The default is `notebook`.
  - `sidebar_width`: an integer in 96-DPI pixels. The default is 260.
- **Notes mode off:** no activity bar and no panel. The layout is exactly today's.
- **Theme:** colors come from `Palette`. The activity bar uses `strip_background`, the panel a shade between strip and editor, the selection `selection_background`, and the hover a light tint. High contrast uses system colors, as the rest of FastPad does.

### New and changed commands

- **New, in the palette:**
  - View: Toggle sidebar (Ctrl+B).
  - View: Show notebook (Ctrl+Shift+E).
  - View: Show search (Ctrl+K; Ctrl+Shift+F since the note search spec, §5).
  - View: Show favorites.
  - Notebook: Close.
  - Notebook: Toggle favorite.
  - Note: Reveal in Explorer.
- **Kept:**
  - Note: Toggle pin.
  - Note: Rename (F2 while the tree has focus).
  - Note: Delete.
  - Note: Move to notebook… (now Ctrl+Shift+M, and it moves the file).
- **Removed:** Note: Toggle favorite, Note: Add tag…, Note: Remove tag…, Notebook: New/Rename/Change color/Delete, Tag: Rename…, Tag: Remove from all notes….
- `CommandId` numbers of removed commands are retired, not reused.

## 10. Keyboard and accessibility

- **Tab order:** activity bar, panel, editor. F6 cycles between them, and Esc in the panel returns focus to the editor.
- **Activity bar:** Up and Down move between icons, and Enter or Space activates one.
- **Tree:**
  - Up and Down move the selection. Home, End, PageUp and PageDown work.
  - Right expands a folder or moves to its first child. Left collapses a folder or moves to its parent.
  - Enter opens (preview tab), Ctrl+Enter opens in a normal tab.
  - Typing letters jumps to the next row whose name starts with them (type-ahead, 1 s reset).
  - F2 renames and Del deletes. Shift+F10 or the context-menu key opens the context menu.
- **Screen readers (MSAA):** each part answers `WM_GETOBJECT` with a provider following `accessibility.rs`.
  - **Activity bar:** push buttons with a pressed state.
  - **Tree:** an outline with outline items. Each item has an expanded or collapsed state, and a selected and focused state. The name is the note's name, with ", pinned" or ", unsaved" added.
  - **Search results:** a list.
  - FastPad raises `EVENT_OBJECT_FOCUS` and `EVENT_OBJECT_SELECTION` on selection changes, and `EVENT_OBJECT_STATECHANGE` on expand, collapse and pin changes.
- **Not by color alone:** pinned has a filled icon, the preview tab uses italics, and the active view has an accent bar and pressed state.
- There are no animations.

## 11. Components

- **`src/library/tree.rs`** (pure, no Win32):
  - Builds the ordered tree from the scan's note list, the pin set and the unsaved-tab labels.
  - Flattens it into visible rows for a given expansion set.
  - Answers "row of path", "parent of row" and type-ahead queries.
  - Supports incremental updates on create, rename, delete, pin and move without a full rebuild.
- **`src/library/name_search.rs`** (pure): name matching and ranking for §8, over a cached lower-case copy of names.
- **`src/window/activity_bar.rs`:** a GDI-painted child window with its buttons, hit-testing, tooltips and MSAA provider.
- **`src/window/side_panel.rs`:** the panel's child window, which hosts the three views, header, divider drag and context menus (`TrackPopupMenuEx`).
- **`src/window/row_list.rs`:** a shared virtual row list for the tree, search results and favorites. It handles scrolling, selection, keyboard handling, hover, hit-testing, painting visible rows only, and the MSAA children.
- **`src/window/library_host.rs`:** keeps its role. It drops the removed organizing commands, adds pin, favorites, close and move, and feeds tree updates to the panel.
- **`src/window/tabs.rs` / `document.rs`:** the preview-tab flag and in-place replacement.
- **`main_window.rs`:** the layout slot in `layout_editor_and_find_bar`, and the titlebar offset so tabs start at the editor area.
- **`config`:** the two new keys. **`folders.ini`:** the `favorite=` and `open=none` lines.
- **Library model:** removes `Notebook`, `Tag`, and the favorite, notebook and tag fields of `NoteRecord`. It adds version 2 reading and writing.

## 12. Performance

- **Startup and first input:**
  - The activity bar and an empty panel frame paint in the first frame, from `fastpad.ini` values that are already loaded.
  - The panel reads nothing from disk before first input. Rows appear when `LIBRARY_READY` arrives, and until then the panel shows "Loading…".
  - Warm startup time must stay within noise of the current branch (the `benchmarks/README.md` gate).
- **The tree** is built on the scan worker and posted with `LIBRARY_READY`. Its target is under 20 ms for 10,000 notes on the reference i5-4590.
- **UI thread:**
  - Painting touches only visible rows.
  - Expanding a folder with 5,000 notes takes under 16 ms.
  - A search keystroke over 10,000 names takes under 5 ms.
  - None of these touch the disk.
- **Memory:** idle working set with a 10,000-note notebook grows by at most 1 MB over the library spec's measurement.

## 13. Error handling

- **Metadata can't be read or written:** the tree still shows every note, and pinning is off with the library spec's notices. Note content is always reachable (product spec §15).
- **A pin fails to save:** the library spec's flush retry applies, and the pin shows as set because the operation is pending.
- **A move, rename or delete fails:** nothing changes, and a notice names the file.
- **A favorite or recent notebook is missing:** a notice, and nothing changes (§4.3).
- **The tree is stale** after changes made in Explorer: the library spec's activation rescan updates it, and selection and expansion are kept by path.

## 14. Testing

- **Unit tests (pure):**
  - Tree order: pinned, folders, notes; natural order; ties.
  - Stable positions across open and edit.
  - Incremental updates, compared with a full rebuild.
  - Flattening with expansion, parent and child navigation, type-ahead.
  - Name-search ranking.
  - `library.ini` version 2 round trip and unknown versions.
  - `folders.ini` `favorite=` and `open=none` parsing and writing, the 50-favorite cap, normalization and display-name clashes.
  - Layout math: widths, clamps, the editor minimum, DPI.
  - Preview-tab rules: replace in place, promote on edit, double-click or save, switch to an already-open note.
- **Window-level tests (in-process, scratch profile):**
  - Ctrl+B toggles the panel, and it persists.
  - The active icon closes the panel.
  - Keyboard navigation and MSAA child counts and states.
  - The context menu command routing.
- **Real-exe tests** (`tests/windows/library.rs`, scratch profile and notes folder):
  - A click opens a preview tab, a second click replaces it in place, and a double-click keeps it.
  - Pinning from the tree writes a version 2 record, and it survives a restart.
  - Favoriting and switching notebooks from the Favorites view.
  - Close notebook leaves `open=none`, and the next start opens nothing.
  - Move to notebook moves the file, and its tab follows.
  - With notes mode off there's no activity bar.
- **Bench:**
  - Startup time with and without the sidebar.
  - `library-scan` extended to report tree build time.
  - Idle working set with 10,000 notes.
- **Manual:** a screen reader walk-through of the activity bar, tree and search, plus high-contrast mode and 150% and 200% DPI.

## 15. Open items for later sub-projects

- Text search in the Search view (sub-project 3).
- Links and backlinks (sub-project 4), which may bring back a use for note-level metadata.
- Drag and drop in the tree, and folder management in the tree.

## 16. Implementation notes

- **No backward compatibility (user decision):** version 1 files are not migrated. `library.ini`
  is version 2 only; a file with any other version, or none, is unreadable and never overwritten
  (§5.1), the same as a damaged file.
- **The search box's placeholder is painted, not a cue banner.** FastPad has no ComCtl32 v6
  manifest, so `EM_SETCUEBANNER` shows nothing. The Search view paints "Search <notebook>" the
  way the find bar paints "Find".
- **Settings lists tab width too.** The Settings button's palette holds the font size, word
  wrap, line numbers, theme, session restore, notes mode, folder autosave and tab width
  commands.
- **F6 is two commands,** `FocusNextPane` (183) and `FocusPreviousPane` (184), bound in the
  accelerator table and kept out of the palette.
- **MSAA reads the panel one child at a time** (`accessible_item_count` and `accessible_item`),
  so a 10,000-row tree never builds 10,000 names per call. Queries from screen-reader threads
  are sent to the UI thread. A default action is a click on the child's center, after scrolling
  it into view, so it does exactly what the mouse does.
- **Outline items report their level** as the MSAA value, as tree views do.
- **`fastpad.ini` is read before the window exists.** `bootstrap::run` reads it once, so the
  activity bar and panel paint their saved view and width in the first frame, and
  `WM_FASTPAD_LOAD_SETTINGS` only applies it and reports its warnings. It re-lays the sidebar
  out only when what it applies differs from what the first frame used. The read itself costs
  about 0.15 ms, so the plan's fallback (reading it after first paint) was not needed.
- **Only the bar and the panel windows are made before first paint.** The window's first
  `WM_SIZE`, when `bootstrap::run` shows it, lays them out; nothing else does before then. The
  activity bar's tooltip (and with it `InitCommonControlsEx`) is made the first time the pointer
  moves over the bar, and that move is relayed to it. The search box is made the first time the
  Search view shows a notebook. Measured with Task 14's method (`--runs 30 --warmup 5`, a 10,000
  note notebook, `feat/note-library` on its own version 1 fixture, three back-to-back pairs), the
  sidebar build with the Notebook view open showed no `compare` regression, and its p50s were
  about 0.5 to 1.0 ms later for first input and 0.6 to 2.0 ms later for first paint. Before this
  they were about 3 ms and 2.5 ms later. What remains is making the two windows and laying out
  and painting them in the first frame.
- **The tree stores each folder's note names in one buffer.** A note is 8 bytes (a range and
  its pin), and the tree is built straight from the library's note list, not from a copy of it.
  A second spelling of one note is dropped per folder, after sorting, keeping the spelling given
  first. With these changes the idle working set with a 10,000-note notebook grew by 0.56 to
  0.65 MB over `feat/note-library` (it was 1.5 MB).
- **Search runs over the library's own note list** and keeps no copy of it. A library change
  while the Search view is hidden only marks its results stale; the query runs again when the
  view shows.
- **A favorite opens asynchronously.** The Favorites view shows the Notebook view once the
  worker has found the folder, so a missing favorite changes nothing, the view included.
- **Name search lowercases on each call, with no cached lower-case copy** (a deviation from
  §11). The binding requirement is §12's 5 ms per keystroke, and the `library-scan` bench's
  `name_search_ms` gate (under 5 ms) enforces it; a cache would cost memory for no measured gain.
- **The panel's accessible name uses the activity bar's wording,** "Notebook: <name>", from one
  shared source, so the bar and the panel never disagree.
- **Loading a notebook also reveals and selects the active tab's note** in the tree, as a tab
  switch does (§6.1), so the first view after `LIBRARY_READY` already shows where you are.
- **A move onto a path another tab already holds is refused** with a notice, like a rename onto
  it, because that tab's autosave would otherwise re-create or overwrite the moved note (§13).
- **A late answer about a notebook is dropped** when the user has opened, closed or checked
  another notebook since, so a slow drive can never undo a later choice.
