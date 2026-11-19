# Notebook tree multi-select

## Intent

Select several notes and folders in the Notebook tree and move or delete them at once, instead of
one item at a time.

## Goals

- Ctrl+click toggles a row in or out of the selection; Shift+click selects the range from the
  anchor to the clicked row; a plain click selects just that row (today's behaviour).
- Keyboard: Shift+Up/Down/Home/End/PageUp/PageDown extend the range; Ctrl+Space toggles the cursor
  row; Ctrl+A selects all visible note and folder rows of the cursor's notebook.
- Every selected row paints as selected; the cursor (focused) row keeps its focus look.
- Drag a selection: a drop moves the whole group into the folder (same notebook), across to
  another notebook, or copies it with Ctrl, using the same drop rules as one row.
- Delete (Del key or context menu) on a selection: one confirm for the group, every item to the
  Recycle Bin, their tabs closed.
- "Move to notebook…" on a selection: one picker, the whole group moved.
- Escape clears the selection back to the cursor row.

## Non-goals

- Multi-select in Open Editors, Search, Tags, Favorites or any other list.
- A selection spanning notebooks (decided: one notebook only).
- Rename, Pin, Open, Reveal on several items. With more than one item selected the context menu
  offers only Move to notebook… and Delete…; F2 and Enter act on the cursor row alone.

## Anti-goals

- A single click or drag of one row must behave exactly as today; nothing in existing tests may
  need to change for the one-row case.
- A rebuild of the tree (file watcher, expand/collapse, notebook load) must never silently move
  the selection onto different files.

## Constraints

- One notebook: Ctrl/Shift+click on a row of another notebook starts a new selection there.
  Root rows, Draft and Placeholder rows are never part of a multi-selection.
- Effective items: when a folder and something inside it are both selected, the inner item is
  dropped from the operation (it moves or is deleted with its folder).
- A drop is refused (no-drop cursor) if the target folder refuses any effective item under the
  existing `tree_drag::accepts_here` / `source_accepts` rules, e.g. a folder dropped into itself.
- `cross_move_host::refuse_item_if_moving` still applies per item.

## Approaches

- **A. A selection set on the Notebook tree view only (picked).** The view keeps its cursor
  (`list.selected`) plus a set of selected items keyed by `(NotebookId, RowKind)` and an anchor.
  Moves and deletes loop over the existing one-item code with one confirm up front; a failed item
  doesn't stop the rest. Small, and reuses well-tested move/delete paths.
- **B. Multi-select in the shared `RowListState`.** Gives every list multi-select; nobody asked
  for it elsewhere, and it churns Open Editors and Search code and tests.
- **C. All-or-nothing group moves with rollback.** Undo can itself fail (the `stuck` case already
  exists); a lot of machinery for a rare case, and deletes are recoverable from the Recycle Bin.

Picked A for its size and reuse.

## Design

### Selection state (`notebook_view.rs`)

- `selection: Vec<RowKind>` (ordered as in the tree), `selection_notebook: Option<NotebookId>`,
  `anchor: Option<RowKind>`. Empty selection means "just the cursor row", so all current one-row
  code paths are unchanged.
- Keyed by `RowKind` (paths), not row indices. After a rebuild, entries whose rows no longer
  exist are dropped (deleted or moved away); entries under a collapsed folder stay selected but
  hidden. This mirrors how `follow_selection` keeps the cursor.
- A plain click, a click in another notebook, or Escape clears it.

### Input (`notebook_view/input.rs`)

- `left_down` reads `MK_CONTROL` / `MK_SHIFT`. Ctrl+click toggles without activating (no tab
  opens); Shift+click selects the range without activating. Plain click is unchanged.
- A press on a row that is already in a multi-selection does not clear it, so the drag can carry
  the group; the release without a drag then collapses the selection to that row (Explorer's
  behaviour).
- Right-click on a row inside the selection keeps the selection; on a row outside it, selects
  just that row (as today).

### Painting (`notebook_view/paint.rs`)

- `RowLook.selected` is true for every row in the selection; `focused` stays the cursor row.

### Drag (`tree_drag.rs`, `notebook_view/drag.rs`)

- New `DragSource::Rows(Vec<RowKind>)` for two or more effective items; one item stays
  `DragSource::Row`. Drag label: "N items".
- `drop_row` for several rows:
  - Move here: `tree_move::drop_into_of` per item, in order.
  - Move across: `cross_move_host::move_items(hwnd, from, items, dest)` (already takes a Vec).
  - Copy: `copy_host::copy_into_of(hwnd, dest, paths, folder, None)` (already takes a Vec).
- Dropping a group onto an editor group (open in group) is not supported; that drop is refused.

### Delete (`library_host/note_actions.rs`)

- `delete_items(hwnd, notebook, items)`: one confirm, "Move 5 items to the Recycle Bin?", with
  "and discard unsaved changes in 2 open tabs" when any affected tab is dirty. Then each item goes
  through the existing per-note/per-folder body of `delete_file` / `delete_folder_of` without its
  own prompt (split those functions into confirm + perform).

### Move to notebook…

- The picker opens once; the chosen destination goes to `cross_move_host::move_items` with all
  effective items.

### Failures

- Inside one notebook (drop into a folder, delete): each item's failure is collected; after the
  batch one notice names what didn't happen, e.g. "2 of 5 items were not moved: a.md (name
  taken), b.md (open tab could not follow)." Items that succeeded stay done.
- Across notebooks (drop on another notebook, Move to notebook…): *builder's call, approved with
  the doc* — keep `cross_move_host::move_items` as it is: one plan, one question, all-or-nothing
  ("… already exists in …. Nothing was moved."). No rework of the cross-move worker. A group with
  more than one folder going to another drive is refused after the drop by the existing
  "move one folder at a time to another drive" notice, not during the drag.

### Interaction details (builder's calls)

- **Press on a row already in a multi-selection:** nothing activates on the press (no preview
  tab, no folder toggle, no pin toggle). If the press becomes a drag, the drag carries the
  selection. If it's released without a drag, the selection collapses to that row and the row
  activates then, as a plain click would. One-row behaviour is unchanged.
- **Ctrl+press:** on an unselected row, toggles it in at press time and can start a drag of the
  resulting selection. On a selected row, the toggle-out is deferred to release, so Ctrl+dragging
  a selection copies it (Explorer's behaviour). **Shift+press** selects the range at press time
  and can start a drag of it.
- **Escape:** with a multi-selection, the first Escape clears it and keeps focus in the tree; with
  none, Escape returns focus to the editor as today (`side_panel.rs`).
- **Plain cursor keys** (Up/Down/Home/End/PageUp/PageDown without Shift) clear the selection
  and move the cursor, as in Explorer. So the cursor is always inside a non-empty selection, and
  Del, the context-menu key and the menu act on the selection.
- **After a group operation:** the selection clears; the cursor goes to the last moved item at
  its new path (a move) or to the row in place of the first deleted item (a delete), the way the
  one-item paths already place it.

### Accessibility (`notebook_view/accessibility.rs`)

- Each selected row reports selected; the cursor row stays the focused/current item. A
  screen-reader announcement "N items selected" after Ctrl/Shift selection changes, through the
  existing announcement path if one exists.

## Left to the builder

- Exact shape of the split between confirm and perform in the delete functions.
- Whether the selection lives in a small struct of its own (for unit tests) — recommended.
- Notice wording beyond the example above.
- Test placement: pure selection logic as unit tests next to the struct; end-to-end move/delete
  of a group in `main_window/tests/` alongside `folder_rename_and_tree_move.rs` and
  `note_operations.rs`.
