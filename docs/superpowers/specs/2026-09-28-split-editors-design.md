# Split editors (editor groups): design

- Status: approved in conversation on 2026-09-28.
- Branch: `feat/split-editors`, based on `main` (0.2.0).
- Delivered as three stacked PRs (§10).

## 1. Goal

- **Split the editor area the way VS Code does.** The editor area becomes a grid of editor groups. Each group has its own tab strip, active tab, find bar, preview and image view. Groups nest into rows and columns, and draggable sashes separate them.
- **The same document can be shown in several groups.** The text, undo history and dirty flag are shared. Each group keeps its own caret and scroll.
- **Tabs move by drag and drop.** They can be reordered within a strip or moved to another strip. Dropping on an editor's edge splits it. VS Code's drop overlay shows where a tab will land.
- **The layout survives a restart** through the session.
- **Nothing gets slower.** One group costs what the single editor costs today. Nothing new runs before first paint.

## 2. Decisions

| Question | Decision |
|---|---|
| Scope | Core splits plus drag and drop. Not in scope: "open preview to the side" as its own group, layout presets (2 columns, 2×2 grid), maximize or join groups (§11). |
| Same document in two groups | Yes. Ctrl+\\ opens the active document in the new group, as VS Code does. |
| Where tab strips go | Every group has its own strip. A group whose top edge is the top of the editor area shows its strip in the title bar row, directly above its own column; any other group shows its strip at its own top. With one group this is 0.2.0's layout. There is no window-title text in the title bar, so the file name appears once, on its tab (§4.1). |
| Find bar | One per group, with its own query and toggles. |
| Markdown/SVG preview | Stays as today (side or full, inside the editor area) but is per group. |
| Preview (italic) tab | One per group. |
| Opening a file already open in another group | A new view of it in the active group (VS Code's default). |
| Ctrl+1..9 | Focus group N. Tab selection moves to Alt+1..9. |
| Closing a group's last tab | Closes the group, unless it is the only one. |
| Architecture | Each group is its own child window. Documents live in a shared store, and tabs are views onto them (§3). |
| Session v1 (0.2.0) | Still read, as one group. 0.2.0 has shipped, so upgrading users keep their tabs (§8). |

## 3. Data model

### 3.1 Document store

- `DocumentStore` (new, `src/window/document_store.rs`) holds each open `Document` exactly once, keyed by `DocumentId`. `Tabs` no longer owns documents.
- **The document host.** The store owns a hidden message-only Scintilla window. It creates every Scintilla document (`SCI_CREATEDOCUMENT`) and lives as long as the app.
  - `EditorDocument` keeps an `Rc` to the host's endpoint, not to the editor that created it.
  - `Editor::use_document` accepts a document when the document and the editor share the same host. That check replaces the current `Rc::ptr_eq` against the editor's own endpoint (`editor/scintilla.rs`, `use_document`).
  - Scintilla documents are global to the DLL and reference-counted (`SCI_ADDREFDOCUMENT` / `SCI_RELEASEDOCUMENT`), so any group's editor can show any of them. Destroying a group's editor releases only that view's reference.
- **Inactive documents.** Their text is read and changed through the host editor, not by swapping them into the visible editor. This removes `with_inactive_document` and its callers' swap (`document_text`, `replace_in_document`, `reload_clean_document`, snapshots).
- **Document-level logic** walks the store, not the tabs: autosave, recovery snapshots, disk-conflict checks, library renames and moves, labels, and the close-window prompts.

### 3.2 Views and groups

- `EditorTab { document: DocumentId, view_state: ViewState }`. A view is identified by its group and its document: a group never has two views of one document (§5.3), so there is no `ViewId`.
  - `ViewState` holds the caret, anchor, first visible line and horizontal scroll offset.
  - It is saved whenever the tab stops being shown in its group's editor, when another tab is activated or the tab moves group, and it is restored when the tab is shown again. Today switching tabs loses the caret and scroll; after this change it keeps them.
- `EditorGroup` (data part) holds:
  - its tabs in strip order;
  - the active index;
  - which of its tabs, if any, is the preview tab.
- The preview flag stays on `Document`. A document that gets a second view becomes a normal tab, so a preview tab always has exactly one view and each group has at most one preview tab. The first edit clears the flag, as today.
- **Reference counting of views.** A document is removed from the store when its last view closes.
  - Closing a view of a dirty document that has another view **does not prompt**.
  - Closing its last view prompts as today.
- **Accessibility.** Each strip has its own accessibility provider (MSAA, §10) built from its group's `TabView`/`TabSelection`.

### 3.3 App state

`App.tabs` stays the one façade over the documents. It holds:

- the `DocumentStore`;
- one `GroupTabs` per group (its views, selection and `TabView`), keyed by `GroupId`, so an id never refers to a different group after one is removed;
- the active group;
- `recent`, a `Vec<(GroupId, DocumentId)>`: the global activation order for Ctrl+P, kept in memory only as today.

Every existing `Tabs` method acts on the active group; new methods name a group. The group windows live in `App.groups`, joined to their data by `GroupId`, and `App.layout` is the `SplitTree` (§4.3). `App::editor()`, `find_bar()` and `active_group()` return the active group's.

### 3.4 Scintilla notifications

- Every editor that shows a document sends `SCN_MODIFIED` and the save-point notifications for it.
- **Document-level effects** run once per change: the dirty flag, generation, label watch and preview-tab promotion.
  - Each document has one *reporting editor*: the editor of the lowest-numbered group whose active view shows that document. Notifications about that document from any other editor are ignored for document-level effects.
  - A group's editor only ever shows its active view's document, so "the editors showing document D" is exactly "the groups whose active view is D".
  - When no group shows the document (it is inactive everywhere), its changes come only through the host editor (replace across notes, reload from disk). The host editor sends no notifications anyone handles, so the code making that change applies the document-level effects itself, as a store call (`DocumentStore::note_text_change`).
  - The reporting editor is recomputed whenever a group's active view changes.
- **View-level effects** (caret position, status bar, scroll sync with the preview) run only for the active group's editor.
- `handle_editor_notification` maps `hwndFrom` to its group instead of comparing it to the single editor.

## 4. Windows and layout

### 4.1 Title bar

Changed after the first look at PR 1 (2026-09-28): a separate title-bar row repeated the file name and cost a row of height, so the tabs stay in the title bar.

- **Top groups' strips are the title bar.** A group whose top edge is the editor area's top (a *top group*) extends up into the title bar row, and its strip is drawn there, over its own column. In a row split every group is a top group, so Split Right adds no row. A group below another one draws its strip at its own top.
- **What the main window keeps:** the title bar row over the sidebar and the caption buttons at the right end. The rightmost top group's strip stops before them.
- **No "…" buttons** (changed after the second look, 2026-09-28). The app menu's items are in the Alt menu band (Format JSON moved to Edit); the strip's menu is on right-click, and Close all tabs is also in File. Later group commands (Split Right, Split Down, Close Group) go in the menus, the strip's right-click menu, the palette and their shortcuts.
- **No window-title text is drawn.** The window's caption text (taskbar, Alt+Tab) is still `<name> - FastPad`, following the active group's active tab.
- **Caption behaviour, as in 0.2.0:**
  - Empty space in a title-row strip drags the window: the group answers `WM_NCHITTEST` with `HTTRANSPARENT` there, and the main window answers `HTCAPTION`.
  - Double-clicking that space opens New in that group, and right-clicking it opens that group's tab-strip menu. Double-clicking the title bar over the sidebar maximizes.
- **The menu band** (Alt) keeps its place directly below the title row. While it shows, top groups keep their strip in the title row and lay out their find bar and content below the band; the group paints its part of the band and passes clicks on it to the main window.
- The tab strip, the preview buttons and the tab overflow leave `titlebar.rs`. Its `HitTarget::Tab`/`CloseTab`/preview variants and their paint code move to the group strip.

### 4.2 The group window

`EditorGroup` is a child window class (`src/window/editor_group.rs`). The strip's paint and hit-testing live in `src/window/group_strip.rs`. From top to bottom the group contains:

1. **The tab strip.** It is painted by the group window and is as tall as today's tabs. For a top group it sits in the title bar row (§4.1). It keeps today's features:
   - the look, and the close buttons;
   - middle-click to close;
   - wheel scrolling and the scroll thumb;
   - the italic preview tab and double-click to promote it;
   - double-click on empty space for New;
   - right-click on empty space for the tab-strip menu.

   The strip holds only tabs. The Markdown/SVG **Preview Side / Preview Full** buttons float at the top-right of the content area (§4.1).
2. **The find bar.** It is the existing `FindBar`, one instance per group, hidden until used. It searches its own group's editor.
3. **The content area.** It holds the group's own Scintilla, its own `PreviewHost` (side or full mode, ratio and divider) and its own `ImageHost`. `preview_host::layout` and `image_host::layout` take the group's content rectangle in place of the window's. The divider drag and editor↔preview scroll sync move with them.

The group lays out its own children on `WM_SIZE`. Most of `layout_editor_and_find_bar` moves into the group.

**Active group.**
- A group becomes active when anything in it is clicked, when its editor, find bar or preview gets focus, or through Ctrl+1..9, F6 or a drop.
- With two or more groups, each group's active tab has a 2 px bar along its top edge: the editor text colour in the active group, the muted colour in the others. With one group there is no bar, as in 0.2.0.
- The status bar, window title, Open Editors highlight and command routing all follow the active group.

### 4.3 Split tree

`src/window/split_tree.rs` is pure logic.

- `Node = Leaf(GroupId) | Branch { axis: Row | Column, children: Vec<(Node, ratio)> }`. The ratios of a branch's children sum to 1.
- **Splitting** a leaf in direction D:
  - If the parent's axis matches D, insert a sibling next to the leaf. The leaf's share is halved between the leaf and the new group, and the other siblings keep their share.
  - Otherwise, replace the leaf with a new branch on D's axis holding the leaf and the new group at 0.5 each.
  - Left and up place the new group before the leaf; right and down place it after.
- **Removing** a leaf gives its share to its remaining siblings in proportion to their shares. A branch with one child left is replaced by that child. If that child is a branch on the same axis as its new parent, it is flattened into the parent.
- **`layout(rect, dpi)`** returns each group's rectangle and each sash's rectangle, together with its branch and child index. The sash is 4 px at 96 DPI, scaled.
- **Minimum group size** is 160×100 px at 96 DPI, scaled.
  - A sash drag is clamped so that no group on either side goes below it.
  - A split is refused when the new pair would not fit. A notice says "Not enough room to split".
- **Group numbering** for Ctrl+1..8 and the Open Editors headers is the leaf order in a depth-first walk: left to right in a row, top to bottom in a column.

### 4.4 Sashes and the main window

- The main window's layout places the sidebar and the command palette and name box. It then gives the rest, above the status bar, to `split_tree.layout` and moves each group window.
- **The main window owns the sashes.**
  - Hovering one shows the resize cursor.
  - A drag uses `SetCapture`, as the preview divider does, and updates the ratios live.
  - A double-click equalizes that branch's children.
  - Resizing the window keeps the ratios.
- **There is always at least one group.**
  - When the last tab of the only group closes, that group stays and shows today's empty state.
  - When the last tab of any other group closes, that group is destroyed, removed from the tree, and the group next to it becomes active.

## 5. Commands and behaviour

### 5.1 New and changed commands

Each command is in the menu band (a new **View ▸ Editor Layout** submenu), the command palette and the accelerator table. Win32 accelerators have no chords, so VS Code's Ctrl+K Ctrl+\\ is not available.

| Command | Key | Behaviour |
|---|---|---|
| Split Right | Ctrl+\\ | New group to the right of the active one, showing a new view of the active document. With no tab open, the new group is empty. |
| Split Down | Ctrl+Shift+\\ | The same, below. |
| Focus Group 1..8 | Ctrl+1..8 | Focuses group N. If group N doesn't exist, the active document is split into a new group to the right of the last group, as in VS Code. |
| Focus Last Group | Ctrl+9 | |
| Select Tab 1..8, Last Tab | Alt+1..9 | Within the active group. Moved from Ctrl+1..9, keeping the numpad variants. |
| Move Tab to Next Group | Ctrl+Alt+Right | Moves the active view to the next group in numbering order. If there is none, a group is created on the right. |
| Move Tab to Previous Group | Ctrl+Alt+Left | Moves the active view to the previous group. Does nothing in group 1. |
| Close Group | File menu, the strip's right-click menu, palette | Closes each tab with the usual prompts. Cancelling a prompt stops the command and keeps the group. |

These commands take the next free `CommandId` numbers.

Existing commands:
- **F6 / Shift+F6** cycle activity bar → panel → group 1 → group 2 → …. Only the parts that are visible take part.
- **Ctrl+Tab / Ctrl+Shift+Tab** cycle the active group's strip in order, as today.
- **Close all tabs** closes the active group's tabs. With one group that is every tab, as today; the group closes with its last tab when it isn't the only one.
- **Zoom** applies to every group's editor, so the zoom level stays one setting.
- **Ctrl+W** and middle-click close one view.

### 5.2 Routing

- `execute_command` targets the active group. That covers the edit commands, find and replace, save, save as, close, the preview toggles, zoom, and the "needs a document" and "needs text" guards.
- A command started from a group's strip or content (its context menu or preview buttons) first makes that group active. Every command therefore keeps one routing path.

### 5.3 Opening files

This applies to files opened from the tree, a Ctrl+P file result, an Explorer drop that misses every group, the command line and a forwarded single-instance open.

- The file opens in the active group.
- If the active group already has a view of it, that view is activated.
- If only another group has it, a new view is added to the active group.
- A single click in the tree replaces the active group's preview tab, if it has one. Other groups' preview tabs are left alone.

### 5.4 Ctrl+P

- With an empty query, it lists open views across all groups, most recent first (`recent_views`).
- When more than one group exists, each row shows its group ("Group 2").
- Picking a row focuses that view in its own group. It does not create a copy.
- A typed file result opens as in §5.3.

### 5.5 Open Editors

- With one group it is the same flat list as today.
- With two or more groups, the list shows `Group 1`, `Group 2`, … header rows, each followed by its views in strip order. The headers can't be dragged or selected.
- A click focuses that view in its group, and ✕ closes that view.
- A document open in two groups appears under each group.

### 5.6 File operations and closing

- **Rename, move and delete** from the tree, and external changes on disk, act on the store's document.
  - Every view of the document updates its title.
  - Deleting a file closes all its views.
- **The dirty-close prompt** appears once per document, when its last view closes.
- **Closing the window** prompts per document, as today.
- **The status bar and window title** show the active group's active view. The caret position comes from that group's editor.

## 6. Drag and drop

It is an in-window `SetCapture` drag, the same pattern as `tree_drag`, not OLE. The pure decisions live in `src/window/group_drop.rs`.

- **Starting.** A left press on a tab followed by movement past `SM_CXDRAG`/`SM_CYDRAG`. The existing `drag_label` popup follows the pointer, showing the tab's icon and name. Esc or a right-click cancels. A press and release without that movement is still a click.
- **The overlay.** It is a layered, click-through popup, the same technique as `drag_label`. It tints the rectangle the tab would end up in with a translucent accent: the whole group for a middle drop, or the future half for an edge drop. On a strip it shows an accent insertion bar instead.

### 6.1 Targets

| Pointer over | Drop does |
|---|---|
| Its own strip | Reorders the tab to the insertion point. Tab reordering is new. |
| Another group's strip | Moves the view to that position in that group. With **Ctrl** held, it copies: a new view of the same document, and the source view stays. |
| A group's content area, middle | Moves (or with Ctrl copies) the view into that group, appended at the end and activated. |
| A group's content area, outer third on the left, right, top or bottom | Splits that group in that direction and puts the view in the new group. |
| Its own group's edge, when the group has only that one tab and no Ctrl | Nothing (the no-drop cursor). |
| A notebook folder in the sidebar tree | Copies the file there, today's `copy_tab_into` behaviour for Open Editors rows, now also from a strip. |
| Anywhere else | Nothing (the no-drop cursor). |

The zone is chosen by the pointer's position in the content rectangle. The outer third is measured against the rectangle's width for left and right, and its height for top and bottom. Where the corner regions overlap, the nearer edge wins, and on a tie the horizontal edge wins.

### 6.2 Rules

- If the target group already has a view of the document, the drop activates that view instead of adding one. A move still removes the source view.
- Moving a group's last tab out closes the source group after the drop completes.
- Dropping a tab on its own current position does nothing.
- **Open Editors rows** can be dragged onto group strips and content areas with the same targets and rules.
- **Explorer (OLE) file drops** open the files in the group under the pointer, or in the active group if the pointer isn't over one. They have no edge zones and no overlay.

## 7. Menus and the tab-strip context menu

- The tab-strip menu is today's `menus::show_tab_strip_menu`, plus Split Right, Split Down and Close Group. It acts on the group that was right-clicked.
- The tab context menu (a right-click on a tab) is new. It makes that view active in its group, then offers Close tab, Close all tabs, Split Right, Split Down and Move to Next Group.

## 8. Session

`session.ini`, version 2:

```
version=2
layout=row(1:0.5,column(2:0.6,3:0.4):0.5)
active_group=2
group=1|active=0
file=<caret>|<anchor>|<firstline>|<path>
snapshot=<caret>|<anchor>|<firstline>|<recovery id>
group=2|active=1
file=...
```

- **`layout`** is the split tree.
  - `row(...)` and `column(...)` are branches, and each child is written as `<child>:<ratio>`.
  - A bare number is a leaf, numbered by its `group=` line.
  - Ratios are written with up to four decimals and normalized on read.
- Each `group=` line starts a group, and the view lines that follow belong to it in strip order.
  - **Every view** records its caret, anchor and first line, not just the active one.
  - `file=` and `snapshot=` have the same format as in v1.
- **A document open in several groups** is written under each group and restored once. Its views are matched by path, or by recovery ID for dirty untitled documents, and they share the restored document.
- The preview mode is not saved (unchanged).
- **Restore** parses the layout first. It then creates the group windows and fills them through the existing step-by-step restore (`begin_session_restore` / `restore_session_step` / `finish_session_restore`), extended to a list of (group, entry).
  - Every view's `ViewState` is applied, not only the active one's.
  - A `layout` line that fails to parse, or that doesn't name exactly the groups present, is dropped. All views then go into one group in file order.
  - Files that no longer exist are skipped, as today. A group left with no views is removed from the tree, which collapses.
  - `active_group` and each `active=` fall back to the first group and the first view when out of range.
- **Version 1** files (0.2.0) are read as a single group. Their entry lines are unchanged, so the reader handles `version=1` with an implicit `group=1` and `active=`. The file is always written as version 2.
- `recent_views` is reset after restore, as the MRU is today.

## 9. Errors

- **Creating a group's window or Scintilla fails** during a split, a drop or a restore:
  - on a split or drop, a notification shows the error and the layout and tabs are unchanged;
  - during a restore, that group's views go into the first group.
- **A drop that can't complete** (for example, the target group went away) leaves the source view where it was.
- **The host editor can't be created** at start-up: this is fatal in the same way today's editor creation failure is.
- **No room to split:** the split is refused with a notice (§4.3).

## 10. Delivery

Three PRs, stacked on `feat/split-editors`:

1. **One group** (`feat/split-editors`).
   - `DocumentStore` and the host editor, `EditorTab` and `ViewState`, and the `EditorGroup` window with its strip, find bar, preview and image view.
   - The tab strip leaves the title bar, notifications are routed by group, and session v2 is written with a single group (the v1 reader is kept).
   - There are no splits yet. The only visible change is that caret and scroll are kept per tab: the one group is a top group, so its tabs are where 0.2.0 draws them (§4.1).
   - Plan-time amendments (docs/superpowers/plans/2026-09-28-split-editors-1-one-group.md):
     - The tab strip's accessibility is MSAA (`IAccessible`), not UIA. The group window hosts the tab list and the preview buttons; the main window's provider keeps the caption buttons.
     - There are no "…" buttons: the app menu's items are in the menu band and the strip's menu is on right-click (§4.1).
     - `App.tabs` stays in PR 1 as a façade over the `DocumentStore` and the one group's views. The editor, find bar, preview and image objects stay `App` fields; PR 2 moves them into the group when it has several.
     - Double-click for New and right-click for the tab-strip menu are on the group strip's empty space; double-clicking the title bar over the sidebar maximizes.
     - After the first look at PR 1 the strip went back into the title bar row (§4.1): the group window reaches up into it, so the strip still belongs to the group.
     - Split Right is a View menu entry, a strip right-click entry and Ctrl+\\ in PR 2; the strip has no action cluster.
     - The preview (italic) flag stays on `Document` in PR 1; with one group it behaves the same as a per-view flag. PR 2 moves it to the view.
2. **Splits** (`feat/split-editors-grid`).
   - The split tree, sashes, the §5.1 commands, several views of one document, per-group find and preview, Open Editors group headers, group-aware Ctrl+P, and multi-group sessions.
   - PR 2 plan-time amendments (docs/superpowers/plans/2026-09-28-split-editors-2-splits.md):
     1. `Tabs` stays the façade (§3.3): the `DocumentStore`, one `GroupTabs` per group and the active group. The windows live in `App.groups`, joined by `GroupId`.
     2. A view is identified by (group, document); there is no `ViewId` (§3.2).
     3. The preview flag stays on `Document`; a second view promotes the document to a normal tab (§3.2).
     4. Ctrl+Tab keeps strip order (§5.1).
     5. "Not enough room to split" is a notice, not a status-bar hint (§4.3, §9).
     6. Focus arriving in a group activates it: the editor reports `SCN_FOCUSIN`; the preview view, the image view and the find fields post `WM_FASTPAD_CONTENT_FOCUSED` with their window, resolved with `IsChild`.
     7. The active-tab accent is a 2 px top bar, only with two or more groups (§4.2).
     8. The new command ids are 197–210; the palette completeness test covers `100..300`.
     9. Focus Group 1–8 and Focus Last Group are not palette entries, like Select Tab 1–9.
     10. Close all tabs acts on the active group (§5.1).
     11. Zoom applies to every group's editor (§5.1).
     12. The tab context menu is new (§7).
     13. The shared Direct2D graphics move from `PreviewHost` to `App.graphics`.
     14. A sash double-click is timed with `GetDoubleClickTime`, as the tab strip does: the main window has no `CS_DBLCLKS`.
     15. Session group numbers are the groups' positions in layout order, from 1.
3. **Drag and drop** (`feat/split-editors-dnd`).
   - Tab drags, reordering, moves and copies between groups, edge splits, the overlay, Open Editors row drags, and Explorer drops per group.

## 11. Out of scope

- Opening a Markdown or SVG preview as its own group ("open preview to the side").
- Layout presets (two columns, 2×2 grid, and so on), maximize a group, join groups, "close other groups".
- Edge-split zones for Explorer (OLE) drops.
- Dragging tree rows into editor groups.
- Floating or separate-window groups.
- Saving the preview mode in the session.

## 12. Testing

- **Pure unit tests:**
  - `split_tree`: split in all four directions with the same and a different axis, removal and collapse and flatten, ratio normalization, layout at 96/144/192 DPI, sash clamping at the minimum size, refusing a split without room, and group numbering;
  - `group_drop`: zone hit-testing including corners and ties, move and copy, the already-open-there rule, the last-tab no-op, and reordering within a strip;
  - session v2: parse and format round-trip, corrupt and mismatched layouts, a document shared across groups, and v1 read as one group;
  - `DocumentStore`: view reference counting, and that only the last view prompts.
- **Window tests** in `main_window` tests (`--test-threads=1`):
  - split and close group;
  - commands routed to the active group;
  - an edit in one group showing in the other group's view of the same document, and the dirty flag shared;
  - per-group find bars;
  - per-group preview;
  - Open Editors headers;
  - Ctrl+1..9 and Alt+1..9;
  - Ctrl+Alt+Left/Right;
  - caret and scroll kept per tab;
  - a session round trip with three groups.
- **Drag tests** send synthetic mouse messages to strips and content zones, as the tree-drag tests do.
- **E2E:** split, restart, and check the restored layout.
- **Manual checks before merge:** Narrator on multiple strips, high contrast, 150–200% DPI (sashes, overlay, strips), and a real Explorer drop onto a second group.
