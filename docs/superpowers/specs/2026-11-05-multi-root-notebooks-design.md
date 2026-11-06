# Multi-root notebooks, phase 1: design

**Branch:** `feat/multi-root-notebooks`. One PR for phase 1.
**Principle:** notes are files. Each open notebook is a folder on disk, and the disk is the only source of truth.

## 1. Goal

Open several notebooks at once in one window. Each open notebook is its own top-level root in the Notebook sidebar tree, in the style of VS Code's multi-root workspaces.

The work is split into three phases. Each phase gets its own spec, plan and PR:

| Phase | Scope |
|---|---|
| **1 (this spec)** | Per-notebook load, watcher and index. Open, add, close and restore notebooks. The multi-root tree. Per-file routing of autosave, pins, naming, links and tree operations to the right notebook. Commands that act on a notebook pick one by context, then fall back to the default notebook. |
| 2 | Search and replace, Quick Open and Tags across all open notebooks, with results labelled by notebook. Remove the phase 1 shims (§3.5). |
| 3 | Drag and move notes and folders between open notebooks. |

**Out of scope for all phases:** several windows with different sets of open notebooks. FastPad stays single-instance with one window.

## 2. Decisions

| Question | Decision |
|---|---|
| Shape | One window, several roots in one Notebook tree. |
| What opening a notebook does | It **adds** a root. Opening one that is already open reveals and selects its root. |
| Nested notebooks | Refused. A folder inside an open notebook, or one that contains an open notebook, is not opened. A notice names the conflicting notebook. |
| Closing one notebook | Its dirty notes are saved and its pins flushed first, as switching does today. Its open tabs stay open as ordinary files. |
| Which notebook a command acts on | The selected tree row's notebook, then the active tab's notebook, then the default notebook (§5.1). |
| Quick capture | Always uses the default notebook, or the `capture_target` in that notebook's `notebook.ini`. |
| Search, Quick Open and Tags in phase 1 | They cover the target notebook only (§5.1), and say which one. Phase 2 widens them to all open notebooks. |
| Wikilinks and backlinks | Always resolve inside the note's own notebook, in every phase. |
| Drops between roots in phase 1 | Rejected. Phase 3 turns them into moves. |
| Single notebook open | Uses the same layout as several. There is no special case. |
| Persistence | `open=` and `default=` lines in `folders.ini`. Older files and older versions keep working (§6). |

## 3. Data model and lifecycle

### 3.1 Notebook slots

`LibraryHost` gains `notebooks: Vec<NotebookSlot>`, kept in sidebar order, and `default_notebook: Option<NotebookId>`.

A `NotebookSlot` owns every field that is per-notebook today:

- `id: NotebookId` and `folder: PathBuf`;
- `state: Option<LibraryState>`;
- load bookkeeping: `generation`, `scanning`, `rescan_requested`, `local_written_during_scan`, `load_failed` and `notified`;
- watcher bookkeeping: `watcher`, `watch_generation` and `watch_failed`;
- the index: `index`, `index_generation`, `index_loaded`, `index_running` and `index_again`;
- `renamed_outside` and `renamed_outside_told`;
- `notebook` and `notebook_text` (the notebook's `notebook.ini`).

The following stay on `LibraryHost`, because they belong to the window:

- `data_dir` and `ids`;
- the `folders` cache and `folders_edited`;
- `check_request`;
- the `shown_*` picker state;
- `copy_worker`;
- `own_writes`, which is already keyed by absolute path;
- `changed_only` and `expansion_revision`.

### 3.2 Identity and routing

- **`NotebookId`** is a `u32` counter handed out when a notebook opens. It is never reused within a session. Ids are not persisted.
- **Worker results are matched to their slot by a generation value unique across the window.** These are `Loaded`, `IndexReady`, watcher `FileChanges` and `CopyDone`. No two slots, and no two kinds of work, ever share a generation, so the generation names the slot as carrying its id would. The handler looks up the slot by generation and drops the result in either case:
  - no slot holds that generation any more, because the notebook was closed;
  - the slot's generation for that kind of work has moved on.
- **`notebook_of(hwnd, path) -> Option<NotebookId>`** is the only way code decides which open notebook owns a file. It returns the slot whose folder contains `path`, using `library::is_inside`. §3.3 rules out nesting, so at most one slot can match.
- **`with_notebook(hwnd, id, f)`** gives `f` the slot's `LibraryState`, and does nothing when the slot is missing or not yet loaded. Changes to one notebook go through it.

### 3.3 Opening

Every way of opening a notebook adds a root:

- the folder picker;
- the recent-notebooks picker;
- Favorites;
- the no-notebook panel;
- a folder dropped on the window;
- the command line;
- an IPC `OpenFolder` request.

Opening a folder `F` goes through these steps in order:

1. **Notes mode is off:** show the existing `NOTES_MODE_OFF` notice and stop.
2. **`F` is already open** (`same_path`): reveal and select its root, and change nothing else.
3. **`F` is inside an open notebook `N`, or contains one:** show "*F* is inside the open notebook *N*" (or "contains the open notebook *N*") and change nothing.
4. **Otherwise:**
   - append a new slot;
   - push `F` onto the recent list (`update_folders`);
   - start the slot's load.
   - Its watcher and index start once the first scan lands, exactly as they do today.
   - If no default notebook is set, the new notebook becomes the default.

A listed notebook is still checked on a worker first (`check_listed_notebook`), as today.

### 3.4 Closing

**Close notebook** (on the root's `…` menu or in the palette, §5.3) runs on one slot:

1. **Save before leaving.** Run `save_before_leaving` on that slot only:
   - autosave its dirty notes;
   - flush its pins.
   - If the pin write fails, the notebook stays open. A retry is scheduled, and a notice explains why.
2. Stop the slot's watcher.
3. Remove the slot.
4. Bump `check_request`.
5. Close any name box that targets that notebook.
6. Tabs from the notebook stay open as ordinary files.
7. **If it was the default:** the first remaining root becomes the default, or none if no roots remain.
8. **If no roots remain:** record `open=none`, as today's close does.

**Window close and flushing.** These run over every slot:

- the debounced write timer (`LIBRARY_WRITE_TIMER_ID`);
- `try_flush`;
- `flush_before_close`;
- the activation-time rescan of slots without a watcher.

A failure in one slot is reported for that slot, and does not stop the others from flushing.

### 3.5 Migration shims

`folder(hwnd)` and `with_state(hwnd, f)` remain for now as shims over the **default** notebook, so the migration can go one area at a time.

Phase 1 must move every call site whose answer depends on a particular file onto `notebook_of` and `with_notebook`:

- autosave;
- pins;
- first-save naming;
- inline naming;
- tree operations;
- copies;
- attachments;
- links;
- the breadcrumb;
- external changes.

Call sites that act on the target notebook use `target_notebook` (§5.1). Phase 2 deletes the shims.

## 4. The multi-root Notebook tree

### 4.1 Rows

- **View rows become `(NotebookId, RowKind)`.**
  - `RowKind` gains `Root`, plus a `Placeholder(Placeholder)` for a root that is not ready.
  - Folder and note paths stay relative to their own root, so `library::tree` and `NoteTree` do not change.
- **`snapshot` builds the list root by root.** Each root contributes:
  - its root row;
  - when the root is expanded, its tree, or one placeholder row.
- **`RebuildKey` becomes a list of per-root keys**:

  ```
  (id, loaded, failed, expansion, root_expanded)
  ```

  plus `changed_only`.
- **`Mode`** is reduced to two cases:
  - no notebook open (`NoNotebook`, the existing panel);
  - `Tree`.

### 4.2 Root rows

The fixed header area in `notebook_layout.rs` goes away. Each root is a row in the scrolling list:

- **Label.** The notebook's name, in the style of today's header (bold, as the folder is spelled).
- **Chevron.** Collapses or expands the root. The state is stored in that notebook's local `root_collapsed`, so it survives restarts.
- **Default marker.** The default notebook shows a small marker after its name. The tooltip says "Default notebook".
- **Buttons.** When the row is hovered or selected, it shows today's header buttons:
  - New note;
  - New folder;
  - Refresh;
  - Collapse folders;
  - `…`.
- **The `…` menu** keeps today's entries, and adds:
  - **Close notebook**;
  - **Set as default notebook** (not shown on the default);
  - **Add to Favorites** / **Remove from Favorites**.

### 4.3 Placeholders

When a root is expanded but not ready, a single row under it says one of:

- *Loading…*
- *Couldn't load. Retry*, where Retry reloads that slot only;
- *No notes yet*.

These replace today's `Loading`, `Failed` and `Empty` modes.

### 4.4 Keyboard, type-ahead and accessibility

Root rows are ordinary rows for keyboard and type-ahead:

| Key | On a root row |
|---|---|
| Up / Down | Move between rows, as on any row. |
| Left | Collapse the root. If it is already collapsed, stay on it. |
| Right | Expand the root, then move to its first child. |
| Enter | Toggle the root. |
| `F2` and `Del` | Do nothing: a root cannot be renamed or deleted from the tree. |
| Type-ahead | Matches root names too. |

**UI Automation.**

- Each root is a top-level tree item, named after its notebook, with "(default)" added for the default.
- Its folders and notes are its descendants.
- A placeholder row is exposed as a plain text item.

### 4.5 Drag and drop

- **Within one root:** unchanged.
- **Onto a row of a different root:**
  - the cursor shows "no drop";
  - the drop hint says "Moving between notebooks isn't supported yet".
  - Phase 3 replaces this.
- **External file drops** (copy into the notebook, through `panel_drop` and `copy_host`) go to the notebook of the row or root under the cursor.
  - `CopyDone` already carries the root, and it now carries the slot id too.

### 4.6 Open Editors

The Pinned part lists pins from every open notebook, in sidebar order. When more than one notebook is open, each pinned entry shows its notebook's name as dim secondary text.

## 5. Routing, targets and commands

### 5.1 The target notebook

`target_notebook(hwnd) -> Option<NotebookId>` picks the first of these that applies:

1. **The sidebar's notebook.** If the Notebook view is shown and has focus or a selected row, use that row's notebook.
2. **The active tab's notebook.** Otherwise, use `notebook_of(active tab path)`.
3. **The default notebook.**

`target_notebook` is used by:

- New note and New folder (from the palette or shortcuts);
- Daily note: today, previous and next;
- New note from template;
- Edit notebook settings;
- Toggle folder autosave;
- Show changed notes;
- Update renamed links;
- Close notebook and Favorite this notebook (palette versions).

The buttons on a root row act on that root.

### 5.2 Per-file decisions

These use the file's own notebook (`notebook_of`), never the target notebook:

- whether autosave applies, and the per-notebook autosave switch;
- pins;
- first-save naming (`saving.rs`, `naming.rs`);
- attachments pasted into a note;
- wikilink completion and following;
- backlinks;
- link updates on rename;
- the editor toolbar breadcrumb;
- external-change handling.

**Untitled notes.** An untitled note saves into the notebook it was created in, since `Document.save_folder` is already absolute. An untitled tab created with no notebook context uses the default notebook.

### 5.3 Commands

- **Open notebook… (`Ctrl+Shift+O`)** adds a root (§3.3). The recent picker and Favorites do the same.
- **Close notebook** closes the target notebook.
- **Move note to notebook (`Ctrl+Shift+M`).** The picker lists:
  1. the other open notebooks;
  2. then recent ones;
  3. then *Browse…*.

  When the destination is open, its `LibraryState` is updated directly, using the same `add_note` path a save uses. It does not wait for the destination's watcher.
- **Search notes, Quick Open and Tags** cover the target notebook only in phase 1. Each names its notebook:
  - the Search view's header reads "Searching *Work*";
  - Quick Open's placeholder reads "Search notes in *Work*";
  - the Tags view header names the notebook.
- **Quick capture** uses the default notebook, or that notebook's `capture_target`. The capture box shows the target notebook's name.

### 5.4 Labels

| Where | Shows |
|---|---|
| Activity-bar tooltip and side-panel label, one notebook open | "Notebook: *name*". |
| Activity-bar tooltip and side-panel label, several open | "*N* notebooks". |
| Editor toolbar breadcrumb | The active note's own notebook. |
| Favorites | Every open notebook is highlighted. Clicking an open one reveals its root. |

## 6. Persistence and startup

### 6.1 `folders.ini`

Existing lines keep their meaning:

- `folder=` lines are the recent list, most recent first;
- `favorite=` lines are the favorites;
- `open=none` means the last session ended with no notebook open.

New lines:

- **`open=<path>`:** one line per open notebook, in sidebar order.
- **`default=<path>`:** the default notebook. It must also appear on an `open=` line, or it is ignored.

**Unavailable roots.** A remembered notebook that is missing at startup (an unplugged drive) stays on its `open=` line, in its place, until it opens again (it is then written in sidebar order) or until the user closes the last open notebook, which records `open=none`. Opening a folder nested with an unavailable notebook (inside it or containing it) drops the unavailable one: the explicit open wins, so `open=` never lists both.

**Reading.**

| What the file contains | What startup opens |
|---|---|
| `open=none` | Nothing, as today. |
| One or more `open=<path>` lines | Those notebooks, in order. The default is `default=`, or the first one when `default=` is missing or invalid. |
| Neither (a file from an older version) | `folder[0]`, as today, and it becomes the default. |

**Compatibility with older versions.** An older FastPad already ignores `open=<path>`, so it opens the most recent notebook. Going back to an older version loses the extra roots and nothing else.

FastPad never writes `open=none` together with `open=<path>` lines. If a hand-edited file has both, `open=none` wins.

**Writing.** Opening or closing roots, and changing the default, rewrite `open=` and `default=` through `update_folders`, which re-reads the file first as today.

### 6.2 Startup

- **Which roots.** The startup worker checks every remembered open root on its own thread, so an offline drive cannot stall the others.
- **Loading.** Each root that exists gets a slot and starts its own load. Roots keep their order from `open=`.
- **Missing roots.**
  - One notice lists every missing root ("D:\Notes is not available").
  - A missing root stays on its `open=` line, so it comes back when the drive does.
  - A missing default hands the default to the first root that did open.
- **Fallback.** `Documents\FastPad` is used only when nothing was remembered and `open=none` is absent, as today.
- **Command line and IPC.** A folder named on the command line or through IPC is added after the restored roots. If it is already open, its root is revealed instead.

## 7. Errors

Every failure is scoped to its notebook:

| Failure | Effect |
|---|---|
| A load fails. | That root shows *Couldn't load. Retry*. The other roots are unaffected. |
| The watcher fails. | That root falls back to rescanning on activation, until it is reopened. |
| A pin write fails while closing. | That notebook stays open, and a notice explains why. |
| A flush fails for one notebook on window close. | It is reported, and the other notebooks still flush. |
| A worker result arrives for a closed notebook. | It is dropped. |

## 8. Testing

TDD throughout.

**Helpers.** `LibraryScratch`, `library(hwnd)`, `notebook_window` and `row_of` learn to install *N* notebooks, and default to one. The existing window tests should then pass with mechanical updates. Tests that inspect rows, `Mode` or the fixed header are updated for root rows.

**New tests:**

- **Opening and closing**
  - Opening a second notebook adds a root, and leaves the first one loaded with its watcher and index running.
  - Opening an already-open notebook reveals it, and adds nothing.
  - A nested folder is refused in both directions (inside an open notebook, or containing one), with the right notice.
  - Closing one notebook keeps its tabs open, leaves the other roots untouched, and hands the default on.
  - Closing a notebook whose pin write fails keeps it open.
- **Worker results**
  - A late `Loaded`, `IndexReady`, watcher batch or `CopyDone` for a closed notebook is dropped.
  - A late result for a stale generation is dropped.
- **Per-file routing**
  - Autosave, pins, first-save naming and attachments each go to the notebook that owns the file.
  - `target_notebook` resolves in order: sidebar selection, then the active tab, then the default.
- **Commands**
  - Quick capture writes to the default notebook's inbox.
  - Move to notebook updates an open destination directly.
- **Tree**
  - A drop onto another root is rejected.
  - External file drops land in the root under the cursor.
  - Keyboard navigation and UI Automation see one top-level item per root.
- **Persistence**
  - `folders.ini` round-trips `open=` and `default=`.
  - Legacy files (with no `open=` lines) open `folder[0]`.
  - `open=none` still opens nothing.
  - Missing roots are kept in `open=`.
- **Failure isolation**
  - A failed load in one root shows Retry there, and the other roots keep working.
