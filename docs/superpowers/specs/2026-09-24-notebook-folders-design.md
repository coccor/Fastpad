# Notebook folders and file-type icons: design

**Branch:** `feat/notebook-folders`, stacked on `feat/quick-open` (PR #13). One PR.
**Principle:** notes are files (see the notebook specs). The disk is the only source of truth for which folders exist.

## 1. Goal

- Create, rename and delete folders from the Notebook view, and see empty folders in the tree.
- Show a coloured icon per note type in the tree, as VS Code does.

## 2. Decisions

| Question | Decision |
|---|---|
| Where the folder list comes from | The scan records every folder it walks. No separate "kept folders" list. |
| Which files the tree lists | Unchanged: only the 14 note types. |
| Folder commands | New folder, Rename (F2), Delete (Del). There is no moving of notes or folders. |
| Where a new folder's name is typed | The existing name box above the editor, the one used for rename. |
| Icon style | A coloured glyph (or short label) per type, drawn with fonts, so it stays crisp at any DPI. Colours come from the theme. |
| High contrast | Every icon uses the muted system colour, as today. |

## 3. Folders in the library

### 3.1 Scan

- `scan::scan` also records each folder it walks, relative to the notebook, in a new `Scan::folders: Vec<PathBuf>`.
- The root isn't listed. Folders the scan skips aren't listed either: dot-folders, `node_modules`, `target`, `bin`, `obj`, hidden or system folders, and reparse points.
- **Cap:** `FOLDER_LIMIT = 10_000`. Past it, further folders aren't listed, but their notes still count toward the note cap as today. A folder that holds listed notes always gets a row: the tree adds a folder for every note's parent path, as it does today.
- The scan already runs on a worker thread. It does no extra I/O, because the directory handles it opens to walk are the same ones.

### 3.2 State and tree

- **`LibraryState` gains `folders: Vec<PathBuf>`:** the scan's folders, relative and in the spelling on disk.
  - A rescan replaces it.
  - FastPad's own folder operations update it in place (§4).
- **`NoteTree::build(notes, folders, pinned)`** also creates a folder node for every listed folder, so an empty folder gets a row.
  - Folders sort as today: folders first, then natural name order.
- **`NoteTree::remove_note`** no longer prunes a folder chain that becomes empty. Folders leave the tree only through:
  - `NoteTree::remove_folder`;
  - a folder rename;
  - a rebuild in which the folder is missing (the folder was deleted outside FastPad and a rescan ran).

  The test `removing_the_last_note_in_a_folder_chain_removes_the_chain` changes accordingly. The folder rows stay while the folders exist on disk.
- **New incremental methods:**
  - `NoteTree::insert_folder(relative)`: adds any missing ancestors too.
  - `NoteTree::remove_folder(relative)`: removes the folder and everything under it.
  - `NoteTree::rename_folder(old, new)`: moves the subtree and keeps its notes and pins.
- **Matching `LibraryState` methods** keep `folders`, `notes`, the tree and the expanded list in step:
  - `add_folder`;
  - `remove_folder`: drops the notes under the folder;
  - `rename_folder`: rewrites the paths of the notes under it.
- **Library records** (`library.ini`) for notes under a renamed folder are relocated with the existing `PendingOp::Relocate`, one per note, so their IDs and pins survive. Notes under a deleted folder get `PendingOp::SetDeleted` and `local.set_missing`, the same as a deleted note.
- **Expanded state** (`LocalState.expanded`) entries under a renamed folder are rewritten. Entries under a deleted folder are dropped.

## 4. Folder commands

### 4.1 New folder

- **Entry points:**
  - A new header button **"New folder"**, drawn with the glyph `\u{E8F4}` (NewFolder) to the right of the "+" New note button. Its tooltip and accessible name are `New folder`.
  - **"New folder here"** on a folder row's context menu.
  - **"Notebook: New folder…"** in the palette.
  - All three run `CommandId::NoteNewFolder`.
- **Where it goes:**
  - from a folder's context menu: that folder;
  - otherwise, the selected row's folder (the folder itself for a folder row, the note's folder for a note row);
  - otherwise, the notebook root.
- **With no notebook open,** the command is unavailable: it is left out of the palette and the header isn't shown.
- **Naming:**
  - The name box opens with the new purpose `NamePurpose::NewFolder(parent)`. The text is `New folder`, or `New folder 2`, 3 and so on when that name is taken, selected in full. The suffix reads `in <parent>`, or `in <notebook name>` at the root.
  - Enter creates the folder with `std::fs::create_dir`, and Escape cancels. Nothing is created before Enter.
- **Name rules:**
  - `title::sanitize_stem` applies, with no extension handling.
  - An existing folder or file with the same name, ignoring case, is refused with the name box's existing error style: `A folder or file named "<name>" already exists`. The box stays open.
  - A name that sanitizes to nothing is refused the same way: `Type a folder name`.
- **After creation:**
  - The folder is added through `LibraryState::add_folder`.
  - Its parent is expanded.
  - The new row is selected and scrolled into view.
  - The keyboard focus returns to the tree.
- **Errors:** a failed `create_dir` shows its error on the name box and keeps the box open.

### 4.2 Rename folder

- **Entry points:** F2 or **"Rename…"** on a folder row. This is `CommandId::NoteRename`, which already handles notes; a folder row routes to the folder rename.
- The name box opens with `NamePurpose::RenameFolder(relative)`. The text is the current name, selected in full, and the suffix is `in <parent>`.
- **Enter:**
  1. Nothing changes if the name is the same, including its case.
  2. A case-only change is allowed.
  3. Otherwise a sibling with that name, ignoring case, is refused with the same message as §4.1.
  4. The rename itself uses `platform::files::rename_no_replace(old, new)`.
- **Open tabs** on notes under the folder are rebound with `Tabs::rebind_path`.
  - If any rebind fails, every rebind done so far is reverted, the folder is renamed back on disk, and the error shows on the name box.
  - Otherwise `LibraryState::rename_folder` runs, the Relocate ops are recorded and `schedule_write` runs.
- **Afterwards:** the renamed row stays selected, and the side panel and tab labels refresh.
- **A preview tab or dirty tab under the folder** is rebound the same way. A rename never touches a file's contents.

### 4.3 Delete folder

- **Entry points:** Del or **"Delete…"** on a folder row. This is `CommandId::NoteDelete`, routed to the folder delete for a folder row.
- **Confirmation** through the existing `confirmed` modal:
  - An empty folder: `Move the folder "<name>" to the Recycle Bin?`
  - A folder with notes: `Move "<name>" and its <n> note(s) to the Recycle Bin?`
  - When any note under it is open with unsaved changes, the confirmation adds a second line: `<k> open note(s) have unsaved changes, which will be lost.`
  - `<n>` counts the listed notes under the folder, from memory.
- **On Yes:**
  1. `platform::files::recycle(hwnd, folder)`, which already works on folders.
  2. Open tabs under the folder close without a prompt.
  3. `LibraryState::remove_folder` runs, the notes' records are marked deleted, and `schedule_write` runs.
  4. The selection moves to the next row, or the previous one at the end.
- **A failure** is reported through the existing delete-failure notice. Nothing in the library changes.

### 4.4 Threads and latency

- The folder commands do their single disk call on the UI thread, like today's note rename and delete: one call per user action, never while typing, never before first paint.
- Note counts, tab lookups and path rewrites use data already in memory.

### 4.5 Name box

- `NamePurpose` gains `NewFolder(PathBuf)` and `RenameFolder(PathBuf)`. Both hold paths relative to the notebook.
- `close_stale_name_box` closes a folder name box when:
  - the notebook changes or closes;
  - a rescan removes the folder;
  - (for `NewFolder`) a rescan removes the parent.

  Switching tabs doesn't close it, because a folder box doesn't belong to a tab.
- The Browse button is hidden for folder purposes.

## 5. File-type icons

### 5.1 Table

| Extensions | Icon | Colour (Catppuccin role) |
|---|---|---|
| `md`, `markdown` | glyph `\u{E8A5}` (Document) | `blue` |
| `json` | label `{}` in the bold UI font | `yellow` |
| `yaml`, `yml`, `toml`, `ini`, `cfg`, `conf` | glyph `\u{E713}` (Settings) | `peach` |
| `csv` | glyph `\u{E80A}` (grid) | `green` |
| `xml` | glyph `\u{E943}` (Code) | `maroon` |
| `txt`, `text`, `log`, and anything else | glyph `\u{E8A5}` (Document) | `overlay2` |
| folder | glyph `\u{E8B7}` (Folder) | `yellow` |

- The pinned glyph and the unsaved row keep their current look.
- **Unsaved tabs** in the tree keep the muted document glyph.

The icon sets spec (`2026-09-25-icon-sets-design.md`) keeps these as the Minimal set; Material Icon Theme is the default.

### 5.2 Colours

- **Source:**
  - Catppuccin themes use their own flavour.
  - The Light theme uses Latte's colours.
  - The Dark theme uses Mocha's.
- **Where they live:** a new `FileIcons` palette of the six colours, built next to `Palette` in `src/window/palette.rs` as compiled static data per theme.
- **High contrast** uses `muted_foreground` for every icon.
- **Selected rows:** a selected or hovered row keeps the icon's colour. That colour is a mid-tone, readable on the selection background.

The icon sets spec (`2026-09-25-icon-sets-design.md`) keeps these as the Minimal set; Material Icon Theme is the default.

### 5.3 Code

- A pure `minimal_icon(kind: NoteKind) -> FileIcon { text: &'static str, font: IconFont, color: IconColor }` in a new `src/window/file_icons.rs`, with unit tests for every row of the table and for mixed-case extensions. (The icon sets spec, `2026-09-25-icon-sets-design.md`, later adds the Material set alongside it; `minimal_icon` is the Minimal set's production API, and `draw_tree_row` falls back to it whenever Material can't draw.)
- `draw_tree_row` uses it for note rows and folder rows.
- The note's extension comes from its path. The tree's rows keep the extension-stripped name for display, and `RowKind::Note(path)` still has the full path.

### 5.4 Screen readers

- A note row's accessible name adds its type: `budget.csv, CSV`, or `meeting notes.md, Markdown`.
  - The type names: `Markdown`, `JSON`, `YAML`, `TOML`, `INI`, `config`, `CSV`, `XML`, `text`, `log`.
  - `cfg` and `conf` are `config`.
- The existing `, pinned` and `, unsaved` suffixes come after the type.
- Folder rows are unchanged.

## 6. Tables and names

- **New command:** `CommandId::NoteNewFolder = 192`. It is not `needs_document` and not `is_sidebar`.
  - Palette entry: `Notebook: New folder…`.
  - Folder context menu: `New folder here`.
- **Table sizes:** COMMANDS +1, palette ENTRIES +1. The accelerators are unchanged: F2 and Del are handled by the tree today and route to the folder variants for folder rows.
- **New module** `src/window/file_icons.rs`.
- **New `NamePurpose` variants:** `NewFolder`, `RenameFolder`.
- **New methods:**
  - `NoteTree::{insert_folder, remove_folder, rename_folder}`;
  - `LibraryState::{add_folder, remove_folder, rename_folder}`;
  - `Scan::folders`.
- `README.md` gains a line on folder commands.

## 7. Testing

- **Scan** (scratch folders):
  - empty folders are listed;
  - skipped and hidden folders are not;
  - nested folders are listed;
  - the folder cap.
- **Tree:**
  - an empty folder has a row;
  - removing the last note keeps the folder row;
  - `insert_folder` creates its ancestors;
  - `remove_folder` removes a subtree;
  - `rename_folder` keeps notes and pins;
  - folders still sort first.
- **LibraryState:**
  - `rename_folder` rewrites note paths and expanded entries;
  - `remove_folder` drops the folder's notes.
- **Window tests** (scratch profile and notebook, `--test-threads=1`):
  - New folder from the header, and from a folder's menu, creates the folder on disk and selects its row;
  - a name that is taken is refused and the box stays open;
  - renaming a folder with an open note rebinds the tab and keeps the note's pin;
  - deleting a folder recycles it, closes its tabs and removes its rows;
  - an empty folder made on disk appears after a rescan.
- **Icons:** `file_icon` covers every row of the table. The accessible name includes the type.
- **Bench:** the existing `library-scan` numbers (cold, warm, tree build) must not regress by more than 10%.

## 8. Out of scope

- A "Move to folder" command. Dragging in the tree moves notes and folders (tree drag spec, `2026-09-25-tree-drag-move-design.md`).
- Listing files other than notes.
- Icon themes, or user-chosen colours.
- Pinning folders.

## 9. Implementation notes

- **Recycling a folder** uses a new `platform::files::recycle_folder`: `recycle` takes only a file (a directory is refused, note-library spec §14), so the folder variant checks for a directory and shares its `SHFileOperationW` call.
- **Wording:** the confirmations and the name-taken error use typographic quotes (“ ”), like the note Delete confirmation. Counts read naturally: `1 note` / `2 notes`, `1 open note has` / `2 open notes have`.
- **Names the scan hides are refused:** a dot-folder, `node_modules`, `target`, `bin` or `obj` would vanish at the next rescan (§3.1), so New folder and Rename refuse them with `FastPad hides folders named “<name>”. Choose another name.`
- **Name clashes** are checked in memory against the tree's folders and the listed notes. Any other file, or a folder the scan skips, with that name makes `create_dir` or `MoveFileExW` refuse, and the same message shows. `New folder N` counts listed folders only.
- **Rescans racing folder commands:** each folder change is also kept as a `FolderChange` until the next rescan starts, and replayed onto that rescan's folders, notes and tree when it is merged, before the touched notes are re-checked. A rescan that listed the folders before the change cannot bring the old row back. The replay is idempotent and touches no disk; records come from the pending `Relocate`/`SetDeleted` operations, and expanded folders from the live state.
- **`touched` entries** under a renamed folder are rewritten, and those under a deleted folder dropped, so a merge makes no stat for them.
- **A folder name box closes** when its folder (or a new folder's parent) is no longer in the tree, checked after every load or rescan, after a notebook switch and after a folder delete. The check reads the tree, so a folder past `FOLDER_LIMIT` that holds notes still counts.
- **Tab rebinds** go through `Tabs::rebind_path`, whose collision check canonicalizes the open tabs' paths, as a note rename's does. A failed rebind undoes the ones before it, renames the folder back and shows today's `Another tab already has that file open.`
- **Focus:** Enter in a folder box moves the focus to the tree, on the new or renamed row; Escape returns it to the editor, as for the note name box. The folder name is selected in full, dots included (`v1.2`).
- **The suffix** names the parent's own name (`in inner`, not `in outer\inner`), or the notebook at the root.
- **The header** reads, left to right: the star, New note, New folder, "…".
- **The empty state** ("No notes in <notebook> yet.") shows only when the notebook has no notes and no folders.
- **Rename and Delete from the palette** act on the row that had the focus when the palette opened: a focused folder row, else a focused note row, else the active tab's note. The palette gives the focus back to the panel before the command runs, so a focused folder row is what the command sees. F2, Del and the folder menu act on folders too.
- **A rename that cannot be undone stands:** when a tab cannot follow a folder rename and renaming the folder back fails, the disk is the truth. The tabs that moved keep their new paths, the library follows the new name, the box closes, and a notice names the tab left behind: `FastPad could not undo renaming “<old>” to “<new>”. “<note>” is still open at its old path.`
- **After a delete** the selection goes to the row that takes the folder's place: the next row after the folder's subtree, else the row before it. That row's kind is remembered before the delete and selected after the refresh, because closing the folder's tabs moves the selection to the new active note first and can expand folders above, shifting the indexes. Only when that row is gone does the selection fall back to the folder's old index.
- **Closing the folder's tabs** marks them autosave-paused first: each close activates the tab it closes, which autosaves the tab being left, and a dirty doomed tab whose file is already recycled would otherwise report `changed on disk`.
- **Untitled tabs** whose first save was to go at or under a renamed folder ("New note here") follow the rename, on success and when the rename stands after a failed undo.
- **Folder paths are checked** before anything destructive: `tree::is_plain_relative_folder` (one or more names, nothing else) guards New folder, Rename folder (old and new), Delete folder and `LibraryState::rename_folder`/`remove_folder`. An empty path would name the notebook root, where `remove_folder` would take every note, and an absolute or `..` path would escape the notebook.
- **Icon contrast:** the weakest pair is Latte yellow on the Light theme's selection, about 1.7:1. A test keeps every icon colour at 1.5:1 or more on the selection, inactive selection, hover and panel backgrounds of every theme. In high contrast an icon takes the row's muted colour, which on a focused selected row is the selection text colour, as the other glyphs do.
- **Palette placement:** `Notebook: New folder…` follows `Notebook: Toggle favorite` and is listed only while a notebook is open (or loading).
