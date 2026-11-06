# Multi-root notebooks, phase 3: moving between notebooks

**Branch:** `feat/multi-root-move`, stacked on phase 2 (`feat/multi-root-search`, #97). One PR.

**Follows:** the phase 1 spec (`2026-11-05-multi-root-notebooks-design.md`, §4.5 and §5.3) and the phase 2 spec (`2026-11-06-multi-root-search-design.md`).

## 1. Goal and decisions

Phase 1 refuses a tree drop onto another notebook ("Moving between notebooks isn't supported yet"). Move to notebook… (`Ctrl+Shift+M`) moves only a single note, only into the other notebook's root, drops its pin and ignores links.

Phase 3 lets notes and folders move between open notebooks, by drag and by keyboard. Open tabs follow, both notebooks update at once, and a failure never loses a file.

| Question | Decision |
|---|---|
| What a drop onto another notebook does | Always moves, on the same drive or another. Ctrl held on the drop copies instead, in one notebook or across (§3.1). |
| Wikilinks that would break | Counted before the move. If any would break, FastPad asks first. Nothing is rewritten (§3.2). |
| Images a moved note embeds from outside what moves | The question warns about them and offers to bring them along (§3.2, §4.2). |
| A folder going to another drive | Copied on a worker, checked, then the original is recycled. Progress and Cancel are shown on the row. A failure removes the partial copy and leaves the original (§3.3). |
| Keyboard | Move to notebook… also takes folders, and adds a step to pick a folder in the destination (§3.4). |
| Name clash at the destination | The whole move is refused; nothing moves. No Replace for moves. A Ctrl copy keeps its existing Replace question. |
| Pins and expanded folders | Move with the items. |
| Wikilinks across notebooks | Still never resolve across notebooks (phase 1 rule). |

**Out of scope:** rewriting wikilinks into cross-notebook paths; merging folders; moving notebook roots; several windows.

## 2. Architecture

One planner, two ways of carrying it out.

### 2.1 `library/cross_move.rs` (pure)

No window and no disk: the caller passes in note lists, indexes, resolvers and an `exists` check.

```text
plan(items, dest_folder, source: Side, dest: Option<Side>, exists) -> MovePlan
```

- `items`: notes and folders, relative to the source notebook. `dest_folder`: relative to the destination (empty is the root). `Side` carries a notebook's note list, its index (`None` while not built) and its resolver. `dest` is `None` for a notebook that isn't open.
- `MovePlan`:
  - `moves: Vec<(PathBuf, PathBuf)>`: each item's source and destination path. A folder is one entry; the notes inside it are listed in `notes: Vec<(PathBuf, PathBuf)>` for the hand-off.
  - `clashes: Vec<String>`: item names already taken at the destination (by `exists` on the destination path, ignoring case).
  - `breaking: Option<usize>`: `None` when the source index isn't built. Otherwise the count of link uses that stop reaching the note they reach now:
    - from a note that stays to a note that moves;
    - from a note that moves to a note that stays;
    - from a note that moves, which in the destination would resolve to a different note (only when `dest` is open with an index; otherwise this kind is not counted).
    - Links between two notes that move together are not counted. Image links are never counted here.
  - `images: Vec<Image>`: each image a moved note embeds (`LinkKind::Image`, and `![[x.png]]` wiki embeds of a non-note file), resolved against the source folder on disk, that is outside every moving folder. `Image { source, users_only_moving: bool }`: `true` when no note that stays embeds it.
- `question(plan, item_label, source_name, dest_name) -> Option<Question>`: the wording and buttons of §3.2, or `None` when nothing breaks and there are no images.
- `image_destination(dest_attachments, name, taken) -> PathBuf`: the destination path, adding " (2)", " (3)" … before the extension while taken.
- `rewrite_image_links(text, moves: &[(old_link_target, new_link_target)]) -> String`: rewrites only the link targets, in Markdown image links and `![[…]]` embeds, leaving every other byte as it was.

### 2.2 `window/cross_move_host.rs`

`move_items(hwnd, from: NotebookId, items: Vec<RowKind>, dest: Destination)`, where `Destination` is an open notebook and folder, or a closed notebook's folder (Move to notebook's recent and Browse… entries).

1. **Check:** both notebooks loaded; the source isn't running another move ("Wait for the move of Projects to finish.").
2. **Prepare:** every dirty tab under the items is saved with the existing `save_before_move` guard. A refusal there stops the move.
3. **Plan and ask:** `cross_move::plan`, then a clash notice or `question`. Cancel changes nothing.
4. **Carry out:**
   - **Same drive** (the volume of the source and destination folders match, `files::same_volume`): `rename_no_replace` for each item, tabs follow, and the existing undo applies when a tab can't follow. This is the logic of `tree_move::move_note` and `move_folder`, lifted out to take an absolute destination.
   - **Another drive, notes only:** `files::move_file` per note, as Move to notebook does now, including its "copied but the original could not be removed" case.
   - **Another drive, a folder:** a job on the window's `MoveWorker` (§3.3).
5. **Hand off** (§4.1), the same for every case.

Callers:

- **Tree drop on a folder of another root:** `move_items`. With Ctrl held: the existing `copy_host::copy_into_of`.
- **Ctrl held on a drop inside one notebook:** `copy_into_of` too.
- **Move to notebook…:** `move_items` after the notebook and folder steps (§3.4).
- `note_actions::move_note_to` is replaced by this path.

Removed: `tree_drag::Reason::OtherNotebook`, `drag_label::OTHER_NOTEBOOK_HINT`, and the phase 1 test asserting the refusal (rewritten to assert the move).

### 2.3 `MoveWorker`

A queued worker like `CopyWorker`, one per window, started on its first job. A job copies a folder with `files::copy_tree` (with a cancel flag and a per-file progress callback that posts at most every 100 ms), checks the copy, then recycles the source, and posts `WM_FASTPAD_MOVE_DONE` with the outcome. Dropping the worker (the window closing) cancels after the file in hand, removes the partial copy and joins.

## 3. Behaviour

### 3.1 Dragging

- A drag of a note or folder over a folder of another notebook highlights that folder, as in one notebook, with the same auto-expand and edge scroll.
- The drag label reads "Move to *Personal › projects*" ("Move to *Personal*" at the root). With Ctrl held it reads "Copy to …" and the cursor is the copy cursor. Pressing or releasing Ctrl updates both at once, without a mouse move.
- Refusals inside a notebook are unchanged (onto its own folder, a folder into itself or below itself). Roots still can't be dragged.

### 3.2 The question

Asked only when something would break. `item` is the note's file name, the folder's name, or "*N* items".

| Situation | Text | Buttons |
|---|---|---|
| Links only | "Move *item* to *Personal*? *N* links in *Work* will stop working." | Move / Cancel |
| Images only | "Move *item* to *Personal*? It uses *M* images from *Work*'s attachments folder." | Move with images / Move without images / Cancel |
| Both | Both sentences, links first. | Move with images / Move without images / Cancel |
| Source index not built | "Move *item* to *Personal*? *Work* is still being indexed, so FastPad can't tell which links will stop working." | Move / Cancel |

"1 link" and "1 image" in the singular. The `confirmed` test hook learns to answer a three-button question.

### 3.3 A folder going to another drive

- The folder's row in the source tree shows dim "Moving to *Personal*… *k* of *n*" and a Cancel button where a root shows Retry. UI Automation names the row "*Projects*, moving to *Personal*, *k* of *n*" and exposes Cancel as a button.
- While it runs, that row and everything under it can't be dragged, renamed, deleted or moved again. A second move out of the same notebook is refused with "Wait for the move of *Projects* to finish."
- Tabs inside the folder stay editable. Autosave and Ctrl+S hold their writes (the tab stays dirty) until the move ends; they then write to the new path, or to the old one after a cancel or failure.
- The check, before recycling: every file the copy read exists at the destination with the same size, and every source file's size and modified time match what was read when it was copied. A mismatch fails the move with "*Projects* changed while it was being moved. Nothing was moved."
- Closing the window asks "A move to *Personal* is still running. Cancel it and close?" (Cancel move and close / Keep running). Closing the source notebook asks the same about the notebook. Closing the destination notebook cancels the move. A late `WM_FASTPAD_MOVE_DONE` for a closed notebook is dropped.

| Outcome | Notice |
|---|---|
| Moved | "Moved *Projects* (*34* notes) to *Personal › projects*." For one note: "Moved *Plan.md* to *Personal › projects*." |
| Cancelled | "Move of *Projects* cancelled. Nothing was moved." The partial copy is removed. |
| Copy or check failed | "Couldn't move *Projects*: *error*. Nothing was moved." The partial copy is removed. |
| Copied, original not recycled | "*Projects* was copied to *Personal*, but the original in *Work* couldn't be removed." Both notebooks list what is on their disk. |

### 3.4 Move to notebook…

- Works on a note or a folder row (the tree's context menu and `Ctrl+Shift+M`, which takes the selected row, then the active tab's note).
- Step 1 is the existing notebook picker (other open notebooks, then favorites, then recent, then Browse…).
- Step 2, for an open notebook only: a picker of its folders, the root first ("*Personal* (root)"), then every folder by relative path in natural order. A closed notebook skips step 2 and the item goes to its root.
- Then `move_items`. For a closed destination, `plan` gets no destination side: clashes are checked on disk, and links are counted on the source side only.

## 4. Data

### 4.1 Hand-off

After the files are in place, on the UI thread:

- **Watchers:** before any write, both notebooks' `begin_own_writes` take the source and destination paths, including images.
- **Source notebook:** each moved note's shared record goes through `PendingOp::Drop`; `remove_note` and the folder entries go; pins and expanded-folder entries under the moved paths are taken out; the index is asked to rebuild.
- **Destination notebook** (when open): `add_note` per note; the taken pins and expanded entries come back re-rooted at the new paths; `request_index_of`.
- **Tabs:** rebound to the new paths (same drive: during step 4; another drive: here). Save folders under a moved folder are re-rooted (`reroot_save_folders`). A preview tab that followed becomes a normal tab, as `move_note_to` does today.
- **Tree:** the destination's folders above the new location expand and the moved row is selected; the focus stays where it is.
- **Views:** one `side_panel::refresh`. Search, Tags and Quick Open (phase 2) follow both notebooks' note lists without new code.
- `notes_moved_of` (the Update renamed links offer) is not called across notebooks.

### 4.2 Images (Move with images)

- After the notes arrive: an image with `users_only_moving` moves, any other is copied, into the destination's attachments folder for that note (`capture::attachments_folder`: `notebook.ini`, then Obsidian's setting, then `attachments`), named by `image_destination`.
- Each moved note's file is rewritten with `rewrite_image_links`, as FastPad's own write; a clean open tab reloads (no tab under the move is dirty: step 2 saved them).
- An image that fails to move or copy keeps its old link, and the notice adds "*K* images couldn't be brought along." An image copied when it should have moved, because its original couldn't be removed, stays in the source too.
- Move without images, or a cancelled question, touches no image.

## 5. Errors

| Failure | Effect |
|---|---|
| A name is taken at the destination | "*X* already exists in *Personal › projects*. Nothing was moved." |
| A dirty tab can't be saved, or changed on disk | The existing `save_before_move` notice; nothing moves. |
| A tab can't follow (same drive) | The move is undone: "Couldn't move *X*: another tab already has that file open." |
| Source gone | "*X* no longer exists." and a rescan, as `tree_move` does. |
| Destination drive full or offline mid-copy | A failed copy: the partial copy is removed and the original stays. |
| A notebook closes mid-move | §3.3. |

## 6. Testing

TDD throughout. Window tests in `--test in_process`, loading Scintilla.

- **`cross_move` (pure):** destination paths for notes and folders; a clash refuses the whole move; each breaking-link kind counts and links within the moving set don't; image links never count as links; images used only by moving notes move and shared ones copy; images inside a moving folder are left out; the attachments rule; " (2)" naming; every §3.2 wording and button set, singular and plural; no question when nothing breaks; the indexing wording; `rewrite_image_links` changes only the targets.
- **`tree_drag`:** another notebook's folder accepts a row; Ctrl makes it a copy; the label reads Move or Copy and updates on Ctrl alone.
- **Same drive:** a note and a folder (with an expanded subfolder) move: disk, both note lists, the tab follows, the pin moves, the row is selected, the notice. Ctrl copies across notebooks and inside one. The question: Cancel, Move, Move with images (a sole-use image moves, a shared one copies, the clean tab shows the rewritten link), Move without images. A dirty tab is saved first; a changed-on-disk tab blocks. A clash refuses. A tab that can't follow undoes.
- **Another drive** (a test hook makes `same_volume` answer false): the folder goes to the worker; progress on the row; drag, rename, delete and a second move are refused; success recycles through the existing test hook and hands off; Cancel removes the partial copy and tabs write to the old path; a failed copy (fail-next hook), a changed file and a failed recycle each give their notice and leave the disk and lists in step; closing the destination cancels; closing the source asks; a late result for a closed notebook is dropped; a tab edited during the move saves to the new path.
- **Move to notebook:** folder rows are taken; step 2 lists the root then folders; a recent destination skips step 2; the old `move_note_to` tests go through the planner (the pin now moves; the copied-but-not-removed notice stays).
- **Gate:** fmt, clippy with `-D warnings`, the full test suite; a manual try with two notebooks on two drives before merging.
