# Moving notes and folders by dragging them in the Notebook tree: design

- Status: approved in conversation on 2026-09-25.
- Branch: `feat/tree-drag-move`, stacked on `feat/icon-sets` (PR #17).
- It removes "drag and drop" from the notebook folders spec's out-of-scope list (§8).

## 1. Goal

- **Move by dragging:** drag a note or a folder onto another folder, or onto the notebook root, like VS Code's explorer.
- **Nothing is lost:** a move never overwrites. Open tabs, pins and expanded folders follow the moved item.
- **Nothing is slower:** nothing new runs at startup, and a drop makes one disk call.

## 2. Decisions

| Question | Decision |
|---|---|
| What can be dragged where | Notes and folders, within the Notebook tree only. Dropping from Explorer and dragging out of FastPad are out of scope (§9). |
| Confirmation | None. The move happens on drop, and dragging it back undoes it. |
| Name clashes | Refused with a notice. Nothing is moved and nothing is renamed. |
| Mechanism | An in-window drag using mouse capture on the tree panel. No OLE `DoDragDrop`, no COM. |
| Keyboard | Nothing new. The palette's "Move to notebook" stays as it is. No "Move to folder…" command. |

## 3. The gesture

### 3.1 Starting a drag

- A press on a note row or a folder row arms a drag. The press does what it does today: it selects the row, and if a name field is open it commits the edit first.
- The drag starts once the pointer moves past the system drag distance (`SM_CXDRAG` by `SM_CYDRAG`) from the press point while the left button is held. The tree captures the mouse then.
- A release before that distance is an ordinary click, unchanged.
- These rows can't be dragged: the draft row, an unsaved row, and the "truncated" row. A press on a chevron, a pin, a header button or the scroll thumb keeps its current meaning and never arms a drag.
- A drag never starts while a name field is still open. If committing the edit leaves the field open (for example, the name is refused), the press doesn't arm a drag.

### 3.2 Drop targets

The row under the pointer gives the target folder:

| Under the pointer | Target |
|---|---|
| A folder row | That folder. |
| A note row | The folder that holds that note (the root for a top-level note). |
| Empty space below the last row | The notebook root. |
| The tree header | The notebook root. |
| Anything outside the tree panel | No target. |

A target is **refused** when:

- it is the dragged item's current folder, because nothing would change;
- the dragged item is a folder and the target is that folder or a folder inside it.

While dragging:

- An accepted target is highlighted: the target folder's row and the rows of its visible children, or the whole list for the root. The highlight uses the theme's selection colour at reduced strength. In high contrast it uses the system highlight colour as an outline.
- The dragged row stays drawn where it is, dimmed.
- The cursor is the normal arrow over an accepted target and the "no" cursor (`IDC_NO`) over a refused target or no target.

### 3.3 While dragging

- **Auto-expand:** holding over a collapsed folder row for 700 ms expands it. It stays expanded after the drag, like a click on its chevron.
- **Auto-scroll:** holding within one row's height of the list's top or bottom edge scrolls the list one row every 50 ms, faster the closer the pointer is to the edge. It stops when the list can't scroll further.
- These run on a timer on the tree panel that exists only during a drag. It is killed when the drag ends.
- **Cancelling:** Esc, a right-button press, or losing the capture (switching apps, a modal dialog) cancels the drag. Nothing moves and the highlight is removed.
- **The tree changes mid-drag:** if the tree is rebuilt (a rescan, a notebook switch, the sidebar hiding) and the dragged item's row is gone, the drag is cancelled. If the target row is gone, the target is found again at the next mouse move.

### 3.4 Dropping

- Releasing the left button over an accepted target moves the item into the target folder, keeping its name.
- Releasing over a refused target or no target does nothing.
- After a move:
  - the tree refreshes from memory;
  - the target folder and the folders above it are expanded;
  - the moved row is selected and scrolled into view.

## 4. The move

A move is a rename to a new parent folder. It reuses the rename commit path from inline naming (`inline_name::commit_rename_note` and `commit_rename_folder`), split so the destination path can come from either a typed name or a drop:

1. **Check in memory:**
   - The source row still exists.
   - The destination isn't the source's own folder.
   - A folder isn't going into itself or a descendant.
   - The name isn't taken in the target folder, among the tree's rows.
2. **One disk call:** `platform::files::rename_no_replace(source, target_folder / name)`.
3. **Tabs follow:**
   - A moved note's open tab is rebound to the new path.
   - For a moved folder, every open tab under it is rebound.
   - Unsaved edits stay in their tabs.
4. **State follows:**
   - Pins and expanded folders are rewritten under the new path (`state.rename_note` / `state.rename_folder`).
   - The per-document save folders are re-rooted (`reroot_save_folders`).
   - State is written with `schedule_write` and `save_local_soon`, as rename does.
5. **Refresh:** `side_panel::refresh`, then the row is selected as in §3.4.

## 5. Failures

In every failure case, nothing on disk is overwritten.

| Case | What happens |
|---|---|
| The name is taken in the target folder (in memory, or the disk call reports it exists) | Nothing moves. Notice: `<name> already exists in <folder>. Nothing was moved.` The root is named after the notebook. |
| The source has vanished (deleted outside FastPad since the last scan) | Nothing moves. Notice: `<name> no longer exists.` The tree is refreshed from a rescan. |
| Windows refuses the move (access denied, in use, another drive through a junction) | Nothing moves. Notice: `Couldn't move <name>: <system message>` |
| An open tab can't follow | The item is moved back with a no-replace rename, as rename does today. The notice says the tab couldn't follow. |
| Moving back also fails | The item stays in its new place. The existing `rename_undo_failed_notice` says which tabs are still open at the old path. |

All notices use the existing notice bar (`main_window::push_notice`).

## 6. Latency

- Nothing new runs before first paint or first input. The drag state is a field that is empty until a press arms a drag.
- All the work during a drag happens in memory: the hit test against the row list, the target checks, the highlight and the cursor.
- The drag timer exists only during a drag.
- The only disk work is the one no-replace rename on drop, which you start, as rename and delete already do. A vanished source adds the rescan that already follows any failed rename.

## 7. Screen readers

- The drag is mouse-only. The keyboard way to move a note stays "Move to notebook".
- After a drop, the moved row is selected with the same focus and selection events a click raises, so a screen reader reads its new place.
- A failure notice goes through the notice bar, like other notices.
- Accessible names don't change.

## 8. Testing

- **Drop targets** (pure, no window):
  - a folder row, a note row (its folder), a top-level note (the root), empty space and the header (the root);
  - refused targets: the current folder, a folder onto itself, a folder into a descendant;
  - rows that can't be dragged: drafts, unsaved rows and the truncated row.
- **The move**, in temporary notebook folders only (never the real profile or Documents):
  - a note and a folder move, and the names stay the same;
  - an open tab follows, including an unsaved one, and every tab under a moved folder follows;
  - pins and expanded folders follow;
  - a taken name is refused with nothing changed on disk;
  - a vanished source gives its notice;
  - when a tab can't follow, the item is moved back, and when that fails, the notice lists the stuck tabs.
- **Window tests**, sending mouse messages to the tree panel:
  - a press and a move shorter than the drag distance stay a click;
  - moving past it starts a drag and captures the mouse;
  - a release over a folder moves the item and selects its row;
  - a release over a refused target moves nothing;
  - Esc, a right-button press and `WM_CAPTURECHANGED` cancel with nothing moved;
  - the cursor is `IDC_NO` over a refused target;
  - the timer auto-expands a collapsed folder and auto-scrolls near an edge, and it's killed when the drag ends;
  - a drag can't start while a refused name edit stays open.
- **Docs:** the notebook folders spec §8 no longer lists drag and drop, and points here. The README describes dragging in the tree.

## 9. Out of scope

- Dropping files or folders from Explorer into a tree folder. A drop on the window still opens the file or notebook, as today.
- Dragging notes out of FastPad (to Explorer or other apps).
- Copying on drag (Ctrl+drag).
- Dragging several rows at once.
- A "Move to folder…" command.
- Undoing a move from the palette. You undo it by dragging the item back.

## 10. Decisions made while implementing

- **The press still clicks.** A press on a row selects it and opens the note or toggles the folder, as before; the drag is armed after that. The dragged item is followed by its path, since toggling a folder moves the rows below it.
- **No capture until the drag starts.** A release the tree never sees (over another window before the drag distance) is noticed at the next mouse move without the button, which disarms.
- **Highlight colours:** the band uses the theme's inactive-selection colour; in high contrast, a 1 px (scaled) outline in the system highlight colour.
- **Shared move code:** the inline renames and the drop use the same moves (`window::tree_move`), so a rename and a move fail, undo and report stuck tabs the same way. A move whose undo failed names the new path in the existing "could not undo renaming" notice.
- **A right press that cancels a drag** swallows its release, so no context menu opens.
