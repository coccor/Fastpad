# Inline naming in the Notebook tree: design

**Branch:** `feat/inline-naming`, stacked on `fix/sidebar-rough-edges` (PR #15). One PR.
**Follows:** `2026-09-24-notebook-folders-design.md`, whose New folder, Rename folder and note rename use the name bar above the editor.
**Followed by:** swappable icon sets, with Material Icon Theme as the first set.

## 1. Goal

- Name notes and folders the way VS Code's explorer does: in an edit field in the tree, at the place where the item will be.
- New note creates the file first, from the name you type, instead of opening an untitled tab.

## 2. Decisions

| Question | Decision |
|---|---|
| Where the name is typed | In an edit field in the tree, for New note, New folder and both renames. |
| Rename started outside the tree | The sidebar shows the Notebook view (opening if hidden), scrolls to the row and edits it there. |
| New note with no extension typed | `.md`. A typed note extension (`data.json`) is kept. |
| Focus leaves the field | Commits, as in VS Code. Switching to another app doesn't count. |
| Ctrl+N | Unchanged: a plain untitled tab. |
| The name bar above the editor | Kept only for the first save of an untitled tab (Ctrl+S), and for renaming a file outside the open notebook. |
| Implementation | A real Win32 `EDIT` control, a child of the side panel, placed over the row's name. |

## 3. Starting an edit

### 3.1 New note

- **Started from:**
  - the Notebook header's "+" button;
  - "New note here" on a folder's context menu;
  - the palette row "Notebook: New note…".

  All three run the new command `CommandId::NoteNew` (193).
- **The target folder** is the one "New note here" was chosen on, else the folder of the tree's selected row, else the notebook root. These are the same rules untitled tabs use for their first save today.
- **The draft row:**
  - The target folder expands, and a draft row appears as its first child. For the notebook root it's the first row of the tree, below any unsaved rows.
  - The draft row is indented one level below its folder.
  - Its icon is the note icon for the extension typed so far: Markdown until a note extension is typed.
  - The field starts empty.
- **With no notebook open,** `NoteNew` is hidden from the palette, and its menu entries and the "+" button are absent, as `NoteNewFolder` is today.

### 3.2 New folder

- Started as today: the header's New folder button, "New folder here", and the palette's "Notebook: New folder…" (`NoteNewFolder`).
- It uses the same target folder rules and draft row as a new note, with the folder icon.
- The field starts empty. The name box's "New folder" prefill goes away.

### 3.3 Rename

- **Started from:**
  - F2 on a note or folder row;
  - Rename… on a row's context menu;
  - the palette's "Note: Rename…" (`NoteRename`), which acts on the focused row as today, else on the active tab's note.
- **The field covers the row's name.** The chevron, icon and pin stay drawn beside it.
- **Prefill and selection:**
  - A note shows its full file name, with the name before the last `.` selected, as in VS Code. A name that starts with its only `.` is selected in full.
  - A folder's whole name is selected.
- **A note that isn't open** is renamed without opening a tab. Today F2 opens the note as a normal tab first; that goes away.
- **Revealing the row.** When `NoteRename` acts on the active tab's note and the sidebar is hidden or shows another view, the sidebar switches to the Notebook view first. The row's folders expand, and the row is scrolled into view and selected before the field appears.
- **A file outside the open notebook** has no row, so `NoteRename` on it keeps using the name bar, as today.

### 3.4 One edit at a time

- Starting an edit while another is open commits the open one first, as if focus had left it (§5.3), then starts the new one.
- Starting an edit also closes the find bar and any open name bar, as the name bar does today.
- **The field takes keyboard focus.** The row being renamed is selected, and a draft row is scrolled into view.

## 4. Names

### 4.1 New note

- The typed text is split with `title::split_typed_name(text, "md")`:
  - no note extension typed means `.md`, so `todo` is `todo.md` and `v1.2 plan` is `v1.2 plan.md`;
  - a typed note extension is kept, so `data.json` is JSON.
- The stem is cleaned as `sanitize_stem` cleans it: forbidden characters are removed, trailing dots and spaces are trimmed, and device names such as `CON` are neutralised.
- A name that is empty, or cleans to nothing, is not an error. Enter or leaving the field cancels (§5).

### 4.2 New folder

- Cleaned with `title::folder_name`, as today. Nothing left of it cancels.
- A name the scan would hide (`.git`, `node_modules`, a dot-folder…) is refused, as today.

### 4.3 Rename

- **A note** follows `title::split_rename`, as today:
  - the current extension is kept unless another note extension is typed;
  - a change of letter case only is a rename, not a clash.
- **A folder** follows today's folder rename rules.
- An unchanged name cancels without a message.

### 4.4 Live checks

- While you type, the name is checked on each change, against what is already in memory. There is no disk access.
- **The checks:**
  - The name is taken by a note or folder listed in the target folder, ignoring case. A rename's own row doesn't count.
  - For a folder, the name is one the scan would hide.
- **A problem shows below the field,** drawn in the palette's error colour over the row beneath it, as VS Code does. When the field is on the last visible row, the message goes above it.
  - Taken: `<name> already exists here.`
  - Hidden folder: the name box's current wording for that case.
- Enter is refused while a problem shows. The field stays, with the message.
- Files the tree doesn't list (non-note files, hidden folders) aren't known in memory. For those, the commit's own disk call catches the clash (§5.2).

## 5. Ending an edit

### 5.1 Keys in the field

- **Enter** commits.
- **Esc** cancels. The row being renamed stays selected, the draft row goes, and focus returns to the tree.
- Ctrl+A selects all. Ctrl+C, Ctrl+X, Ctrl+V, Ctrl+Z, Del, Home, End, Ctrl+Left and Ctrl+Right, and Ctrl+Backspace work in the field as in any edit field.
  - Accelerators that would take those keys (Del isn't one) leave them to the field while it has focus, as `palette_keeps_key` does for the palette.
  - Other accelerators, such as Ctrl+P and Ctrl+S, run as usual. When they move focus, the edit commits (§5.3).
- **Tab** is swallowed; it does nothing.

### 5.2 Commit

- The commit makes the single disk call you start:
  - create the empty note file, never over an existing file;
  - create the folder;
  - or rename, never over an existing file.
- **Success:**
  - **New note:** the file is created empty and added to the library. It opens as a normal tab with focus in the editor.
  - **New folder:** the folder is added to the tree. Its row is selected, and focus stays in the tree.
  - **Rename:** the library records, pins and expanded folders follow, as today. Open tabs are rebound, dirty and preview tabs included, and with a tab left behind handled as today. The row stays selected, and focus stays in the tree.
- **Failure** (a file already has the name, access denied, the folder is gone…): the message shows under the field as in §4.4, and the field stays.
  - The wording is today's: `<name> already exists. Try <free name>.`, or `FastPad could not … : <error>`.
- **Empty or unchanged names** cancel without a message.

### 5.3 Focus leaves the field

- **Focus moving to another window of this FastPad** commits: a tree row, the editor, a tab, the palette, a menu.
- **Focus leaving FastPad,** by switching apps or when the main window is deactivated, doesn't commit. The field stays, and focus goes back to it when the window is reactivated.
- **A click on another tree row** commits first. The click then does what it does today.
- **A commit caused by focus leaving:**
  - with an empty or unchanged name, cancels;
  - with a problem showing, or when the disk call fails, closes the field and shows the reason in the usual notice.

### 5.4 Cancelled by the tree changing

- The tree is rebuilt while the field is open: after a rescan, a folder change or a tab change. The draft row is put back, and the field is placed on its row again.
- The edit is cancelled, with no message, when:
  - its target folder is no longer in the tree;
  - the row being renamed is no longer in the tree;
  - the notebook closes or changes;
  - the sidebar is hidden or switches to another view.
- **Scrolling** moves the field with its row. The field is clipped to the list area and never draws over the header. A field scrolled fully out of view is hidden but keeps editing, and it shows again when its row returns.

## 6. Screen readers

- **The field's accessible name:**
  - `New note name, in <folder>` (`in <notebook>` at the root);
  - `New folder name, in <folder>`;
  - `Rename <file name>`.
- A problem that appears or changes under the field is raised as the field's description, and announced with `EVENT_OBJECT_DESCRIPTIONCHANGE`.
- The draft row isn't an item in the tree's MSAA list. The field is its own focusable control.

## 7. Latency

- Nothing new runs before first paint. The field is created on first use.
- Live checks read only the tree in memory.
- Each commit makes one disk call on the UI thread, which you start, as note rename and delete already do.

## 8. Tables and names

- **New:** `CommandId::NoteNew` (193), with the palette row `Notebook: New note…` placed before `Notebook: New folder…`. It is listed only with a notebook open, like `NoteNewFolder`.
- **Changed:**
  - The header's "+" and "New note here" run `NoteNew` instead of `CommandId::New`.
  - `CommandId::New` (Ctrl+N, File → New tab) is unchanged.
- **New module `src/window/inline_name.rs`.** It holds the field (create, place, show and hide, hook, accessible name), the draft or rename state, the live checks and the commit paths.
- **Name bar:**
  - `NamePurpose` keeps `FirstSave` and `RenameNote`. `RenameNote` is now used only for files outside the open notebook.
  - `NewFolder` and `RenameFolder` are removed.
  - The folder submit logic moves to `inline_name.rs`. There's no backward compatibility.
- **Pure name logic** is testable without Win32: the split, the selection range, and the live checks against a list of sibling names.
- `README.md` describes naming in the tree.

## 9. Testing

- **Unit tests:**
  - the new-note split (`todo`, `data.json`, `v1.2 plan`, `CON`, empty);
  - the rename selection range (`a.md`, `.gitignore`, `archive.tar.gz`, a folder);
  - the live checks: taken ignoring case, the renamed row itself, hidden folder names.
- **Window tests,** with a scratch profile and notebook and `--test-threads=1`:
  - "+", "New note here" and the palette each show a draft row in the right folder with the field focused;
  - Enter creates `todo.md` and opens it with focus in the editor;
  - New folder creates the folder, selects it and keeps focus in the tree;
  - F2 and Rename… on a note that isn't open rename it without opening a tab;
  - rename of an open, dirty or preview tab rebinds it;
  - folder rename rebinds its tabs, as today's tests do;
  - `NoteRename` from the palette with the sidebar hidden reveals the row and edits it;
  - `NoteRename` on a file outside the notebook uses the name bar;
  - a taken name shows the message, and Enter leaves the field open;
  - a disk clash with an unlisted file shows the message after Enter;
  - Esc cancels, and the draft row goes;
  - focus moving to the editor commits;
  - focus moving to the editor with a taken name closes the field with a notice;
  - app deactivation doesn't commit;
  - a rescan keeps the field, and one that removes the target cancels it;
  - the field's accessible name;
  - Ctrl+A and Ctrl+Z in the field act on the field.
- The folders tests that used the name bar move to the field.
- **Checks:** clippy and the targeted tests while working, then the full suite once at the end.

## 10. Out of scope

- Typing a path with `\` or `/` to create nested folders. Separators are cleaned out, as today.
- Moving notes or folders by drag and drop.
- Inline naming in the Search and Favorites views.
- Changing the first-save name bar or its Browse… button.

## 11. Decisions made while implementing

- **No row, no field.** `NoteRename` uses the name bar for any file the tree has no row for:
  outside the open notebook (§3.3), but also a non-note file inside it (`script.py`), a note not
  listed yet, or a window with no sidebar (notes mode off).
- **The Notebook view shows for every edit.** New note and New folder from the palette switch
  the sidebar to the Notebook view first, opening it if hidden, as `NoteRename` does (§3.3); a
  draft needs its row on screen.
- **An empty notebook** shows the tree while a draft is open, with the draft as its only row,
  and goes back to its empty state when the draft ends.
- **The empty state's New note button** drafts a note in the tree too (`CommandId::NoteNew`), at
  the user's request, so every New note in the Notebook view names its file first; only Ctrl+N
  and File → New tab open an untitled tab.
- **Wording.** The live check says `<name> already exists here.` for notes and folders alike. A
  clash found by the disk call keeps each kind's current wording: `<name> already exists. Try
  <free name>.` for a note, `A folder or file named “<name>” already exists` for a folder.
- **A folder name that cleans to nothing cancels**, like an empty note name (§4.1, §5.2):
  "Type a folder name" goes with the name box's New folder.
- **Focus.** A commit caused by focus leaving moves no focus: a new note opens without taking
  the focus from where it went, and a rename leaves it there too. Only Enter puts the focus in
  the editor (new note) or keeps it in the tree (folder, renames).
- **A click in the tree while editing** commits first, then acts on the row it landed on, found
  by what it shows: the rows may have moved (the draft gone, the renamed row sorted elsewhere).
  A click on the draft row or on the row just renamed does nothing more.
- **A modal prompt** that takes the focus from the field holds the commit until it ends.
- **Switching views wins.** Showing another view or hiding the sidebar cancels, even when the
  palette took the focus first and a focus-leave commit is still queued.
- **Keys.** The Edit control has no Ctrl+A or Ctrl+Backspace of its own: the field's hook
  handles both. Of the field's keys only Ctrl+Z is an accelerator; the field keeps it.
- **Screen readers.** The field's accessible name and its problem (as the description) are set
  through `IAccPropServices` dynamic annotation, so the system's own `Edit` proxy reports them.
- **A problem shown after Enter** stays until the text changes; Enter again with the same text
  is refused like any other problem.
- **A vanished row is not the active tab.** When `Note: Rename…` acts on a row that left the
  notebook before Enter, it renames nothing, unless that row is the active tab's own note, which
  then uses the name bar.
- **A note rename that cannot be undone stands.** If a renamed note's tab cannot follow and
  putting the file back also fails, the new name stands, the library follows the disk, and a
  notice names the tab left on its old path, as a folder rename does.
- **Scrolling keeps the edit.** A press on the tree's scroll thumb, left or right, neither
  commits nor moves the focus: the field keeps editing while the list scrolls.
- **A field scrolled out of view** is shrunk and marked hidden rather than hidden with
  `ShowWindow`, which would take its focus away.
