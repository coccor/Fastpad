# Open Editors and copying files into the notebook: design

- Status: approved in conversation on 2026-09-25.
- Branch: `feat/open-editors`, based on `main`.
- It removes "dropping files or folders from Explorer into a tree folder" from the tree drag spec's out-of-scope list (§9).

## 1. Goal

- **Add any file to the notebook the way VS Code does it.** A file opened from anywhere shows up in an Open Editors list at the top of the Notebook panel. Dragging it from there onto the tree copies it into the folder under the pointer. Dragging a file or folder from Windows Explorer onto the tree does the same.
- **The notebook becomes a collapsible root under Open Editors**, like a workspace folder in VS Code's Explorer.
- **Nothing is lost:** a copy never overwrites without asking, and a replaced item goes to the Recycle Bin.
- **Nothing is slower:** nothing new runs before first paint, and copying runs on a worker thread.

## 2. Decisions

| Question | Decision |
|---|---|
| Copy or move | Copy, as VS Code does. The original stays where it is, and its tab stays on it. |
| Name clashes | Ask, per item: "X already exists in <folder>. Replace it?" OK replaces, Cancel skips that item. |
| Which files | All of them. A file the tree doesn't list is copied and a notice says it isn't shown. |
| How an open document gets in | By dragging its row from Open Editors onto the tree. There is no palette command. |
| Tree drags | Unchanged: they still move. Only Open Editors rows and Explorer drops copy. |
| Untitled tabs | They leave the tree and show only in Open Editors. They can't be dragged. |
| Root row buttons | Star, New note, New folder and "…" move from the panel header to the root row and are always visible. |

## 3. The panel

```
▾ OPEN EDITORS  3
    📄 todo.md                    ✕
  ● 📄 draft.txt
    📄 Untitled 2
▾ MY NOTEBOOK        ★  📄+ 📁+ …
    ▸ projects
      📄 ideas.md
```

### 3.1 Layout

- Both sections are painted into the existing sidebar panel window. The notebook header band (`HEADER_HEIGHT`, `header_layout`) goes away.
- From the top:
  1. **The Open Editors header row:** chevron, "OPEN EDITORS" and the tab count.
  2. **Open Editors rows**, when the section is expanded: at most 9 rows are visible. Past that the section scrolls on its own, with its own scroll bar.
  3. **The notebook root row:** chevron, the notebook's name, and the four buttons on the right.
  4. **The tree**, when the root is expanded: it fills the rest of the panel and keeps its own scroll.
- The empty, loading and failed states draw under the root row, where the tree would be, exactly as they draw under the header today.
- With no notebook open, the root row reads "NO NOTEBOOK" and has no buttons, and the no-notebook state (message, "Open notebook…" button, RECENT list) draws under it.
- Row height is the tree's `ROW_HEIGHT`. Header rows use the same height and the tree's header text style.

### 3.2 Open Editors rows

- One row per tab, in tab-strip order: file-type icon (from the current icon set) and name. An untitled tab uses its tab label (`unsaved_label`).
- A dirty tab shows ● at the right. A clean tab shows ✕ there on hover, and on the selected row.
- The active tab's row is drawn as selected.
- The tooltip is the file's full path, or the label for an untitled tab.
- **Click** a row: switch to that tab (`activate_document_by_id`). The focus stays in the panel, like a click on a tree row.
- **Click ✕ or middle-click** a row: close that tab through the same path as the tab strip (`close_tab_at`), with its save prompt.
- The list follows the tabs: opening, closing, reordering, renaming, saving and switching update it. It is rebuilt from `tabs.documents()` when the tab strip is refreshed (`refresh_tabs`).
- No context menu and no reordering by drag (§9).

### 3.3 Collapsing

- The Open Editors chevron (or a click on its header row) collapses and expands the section. The state is the new setting `open_editors_expanded` (default `true`), saved like `file_icons`.
- The root row's chevron (or a click on the name) collapses and expands the tree. The state is saved per notebook in `LocalState`, next to the expanded folders. Default: expanded.
- A click on a root row button does what the header button does today and doesn't collapse anything.

### 3.4 Untitled tabs leave the tree

- The tree no longer shows unsaved rows: `RowKind::Unsaved`, `unsaved_entries` and the code that places them go away. Their place is Open Editors.
- The inline "new note" draft row stays in the tree.
- A first save that the inline name box runs still works: the name box belongs to the draft row, not to unsaved rows.

### 3.5 Keyboard

- The arrow keys move one selection through the whole panel, in this order: the Open Editors header, its rows, the root row, then the tree rows. Home and End go to the first and last row. Page Up and Page Down move within the section that holds the selection.
- **Left and Right on a header row** collapse and expand it. Left on a tree row at the top level goes to the root row.
- **Enter on an Open Editors row** switches to that tab and moves the focus to the editor. **Enter on the root row** toggles it.
- **F2, Del, Shift+F10 and Apps** do nothing on Open Editors rows and header rows. So Del never deletes a file when the selection is on a tab row.
- Palette and accelerator commands that act on "the selected tree row while the panel has focus" (sidebar spec §6.3) act on the active tab when the selection is on an Open Editors row or a header row.

## 4. Copying into the tree

### 4.1 Sources and targets

| Dropped on… | From an Open Editors row | From Windows Explorer |
|---|---|---|
| A folder row | Copy into that folder. | Copy into that folder. |
| A note row | Copy into the folder that holds it. | Same. |
| The root row (expanded or collapsed), or empty space below the tree | Copy into the notebook root. | Same. |
| Open Editors (header or rows) | No target. | Opens, as a drop on the window does today: files as tabs, the last folder as the notebook. |
| No notebook open | — | Opens, as today. |

- An Explorer drop can hold several files and folders. Folders are copied with everything inside them.
- A tree drag (a note or folder row) still moves, as in the tree drag spec. Nothing about it changes.

### 4.2 Refused targets

A target is refused (no highlight, the "no" cursor, and an Explorer drag gets `DROPEFFECT_NONE`) when:

- the dragged Open Editors row is an untitled tab (it can't be dragged at all: the press doesn't arm a drag);
- the destination is the item's own path: its own folder for a file, or itself for a folder;
- a dropped folder would land inside itself or one of its subfolders.

An Explorer drop with several items is refused only if every item is refused. Otherwise the refused items are skipped on drop, and a notice names them.

### 4.3 The gesture

- **From Open Editors:** the tree drag's in-window gesture: press, then move past the system drag distance, then mouse capture on the panel. It reuses the drag label, the folder highlight, the 700 ms auto-expand, the edge auto-scroll, and the Esc, right-press and `WM_CAPTURECHANGED` cancels.
- **From Explorer:** a new OLE `IDropTarget` registered on the sidebar panel window.
  - `DragEnter` accepts only data that offers `CF_HDROP`.
  - `DragOver` converts the screen point to panel coordinates and runs the same hover logic as the in-window drag. So the highlight, auto-expand and auto-scroll are the same.
  - It answers `DROPEFFECT_COPY` over an accepted target, and `DROPEFFECT_NONE` elsewhere, except over Open Editors (§4.1), where it answers `DROPEFFECT_COPY` and opens.
  - `DragLeave` and `Drop` clear the highlight.
  - No drag label is shown for an Explorer drag: Explorer draws its own drag image.
- The drag source becomes one type with two cases: `Tree(RowKind)`, which moves, and `Tab(DocumentId, PathBuf)`, which copies. An Explorer drag has a list of paths as its source and never becomes a `Drag`: it only borrows the hover logic.
- The drop target's COM plumbing comes from `editor::file_drop` (vtable, `CF_HDROP` reading), shared rather than copied. The panel's target has no inner target and decides the effect per point.

### 4.4 Name clashes

- Before copying, the drop is planned on the UI thread (a pure function): for each item, its destination path, or why it is refused.
- For each item whose destination already exists, one confirmation: "<name> already exists in <folder>. Replace it?" (`modal::confirm`, OK and Cancel). A folder clashing with a file, or the reverse, asks the same way.
- OK: the existing item goes to the Recycle Bin (the same helper Delete uses), then the copy is made. If the Recycle Bin step fails, that item isn't copied and the notice says so.
- Cancel: that item is skipped. The rest of the drop carries on.

### 4.5 The copy

- The planned copies run on a worker thread, in order. The UI doesn't wait for them.
- A file is copied with `CopyFileExW` and `COPY_FILE_FAIL_IF_EXISTS`. A folder is created, then walked recursively in the same way. A new `platform::files::copy_file_no_replace` wraps the call.
- A failure stops the item it happened in (the rest of that folder isn't copied) and the worker moves on to the next item.
- When the worker finishes, it posts the result to the window:
  - a copied note file is added to the index (`add_note`), and a copied folder triggers a rescan (`request_rescan`);
  - when the drop was a single item, its row is selected, as after a tree drag;
  - the notices in §4.6 go through the notice bar.
- A copy never changes the dragged tab: it stays on the original file.

### 4.6 Notices

- **Files the tree doesn't list:** "photo.png was copied but isn't shown: the notebook lists text notes only." With several: "3 files were copied but aren't shown: …"
- **A dirty tab:** the file on disk is copied. "Copied the saved version of draft.txt. Your unsaved changes are still in its tab."
- **A failure:** "<name> could not be copied: <reason>." For a folder, it says how many files were copied before the failure.
- **Skipped refused items** in an Explorer drop: "<name> was not copied: it is already there." or "…: a folder can't be copied into itself."

### 4.7 Open tabs of replaced items

- A replaced note that is open and clean reloads (`reload_clean_document`).
- One that is open and dirty is left alone. The existing disk-stamp check pauses its autosave and says so, as for any file changed outside FastPad.

## 5. Failures

- The source vanished or can't be read: that item fails with its notice. The rest carry on.
- The destination folder vanished before the copy: the item fails with its notice, and a rescan follows.
- The window closes while copying: the worker finishes the file it is on and stops. Nothing is posted to a dead window.
- A second drop while a copy is still running is queued behind it on the same worker.

## 6. Latency

- Nothing new runs before first paint or first input. The Open Editors rows are built from the tab list that already exists.
- The OLE drop target is one `RegisterDragDrop` call on the panel window, made after first paint.
- Hover work during an Explorer drag happens in memory, the same as the tree drag.
- All copying and Recycle Bin work runs on the worker. Only the clash prompts and the plan run on the UI thread, and the plan does one `exists` check per top-level item.

## 7. Screen readers

- Open Editors rows are exposed the way tree rows are. The name is "<name>, open editor", plus ", modified" when dirty. The header rows are exposed as expandable items with their expanded state.
- The root row is exposed as an expandable item named after the notebook.
- After a copy of a single item, its row is selected with the same events a click raises.
- Notices go through the notice bar.
- The drag is mouse-only. A keyboard way to copy a file into the notebook is out of scope (§9).

## 8. Testing

- **The layout** (pure): the section rectangles for expanded and collapsed Open Editors, with 0, 1, 9 and 20 tabs; a collapsed root; the no-notebook state under the root row.
- **The hit test** (pure): each part of the Open Editors rows (row, ✕), both header rows, the root row's buttons, and the tree rows below.
- **The drop plan** (pure, `window::tree_copy`):
  - destinations for a folder row, a note row, the root row and empty space;
  - refusals: an untitled tab, a copy onto itself, a folder into itself or a subfolder;
  - clashes found, and a mixed Explorer drop where some items are refused.
- **The copy**, in temporary folders only (never the real profile or Documents): a file, a nested folder, a clash replaced (the old item is in the Recycle Bin, or its stand-in in tests), a clash skipped, a failure part way through a folder, a non-note file with its notice.
- **The OLE target:** its vtable called with a fake data object offering `CF_HDROP`: the effect over a folder, over Open Editors, and over a refused target; a drop copies.
- **Window tests**, with mouse messages to the panel:
  - Open Editors follows opening, closing, switching and saving tabs;
  - a click switches tabs, ✕ and middle-click close, and a dirty tab's close still prompts;
  - collapsing both sections, and the states survive a restart;
  - the arrow keys cross from Open Editors into the tree, and Del on a tab row deletes nothing;
  - dragging a tab row onto a folder copies the file, selects the new row and leaves the tab on the original;
  - a clash prompts, OK replaces and Cancel skips (`answer_next_confirm`);
  - an untitled row doesn't start a drag;
  - untitled tabs no longer appear in the tree.
- **Docs:** the tree drag spec §9 no longer lists Explorer drops and points here. The README describes Open Editors and copying into the notebook.

## 9. Out of scope

- A palette command to copy the active tab into the notebook.
- Reordering tabs by dragging Open Editors rows, and a context menu on them.
- Dragging tabs from the tab strip onto the tree.
- Moving with Shift held, and moving from Explorer.
- Dragging notes out of FastPad to Explorer or other apps.
- A progress indicator for large copies.
- Keeping both on a clash (`name (2).md`).
