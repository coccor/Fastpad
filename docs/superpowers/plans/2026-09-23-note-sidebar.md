# Note Sidebar Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the note library's label notebooks, tags and note favorites with a pins-only model where a notebook is a folder, and give FastPad a VS Code-style activity bar (Notebook, Search, Favorites, Settings) with one resizable side panel: a folder tree with pins, name search, and favorite notebooks.

**Architecture:**
- **Pure library code (no Win32):** `src/library/` shrinks to pins only and gains `tree.rs`, which builds and incrementally maintains the ordered folder tree on the scan worker, and `name_search.rs`.
- **Window code:** a new `src/window/side_panel.rs` owns two painted child windows, the activity bar (`activity_bar.rs`) and the panel. The panel paints its header and a virtual row list (`row_list.rs`) for the current view (`notebook_view.rs`, `favorites_view.rs`, `search_view.rs`).
- **Layout:** `main_window::layout_editor_and_find_bar` gives the sidebar the left edge, and everything else (tabs, menu band, find bar, name box, editor, preview, status bar) is laid out to its right.
- **Opening notes:** tabs gain a preview mode.

**Tech Stack:** Rust 2024, `windows-sys` 0.61 (already enabled: `Win32_UI_Controls` for tooltips, `Win32_UI_Accessibility`, `Win32_UI_Shell`, `Win32_Graphics_Gdi`), and Scintilla. Icons are Segoe MDL2 Assets glyphs, the font the title bar already uses. No new crates.

**Spec:** `docs/superpowers/specs/2026-09-23-note-sidebar-design.md` (it builds on `docs/superpowers/specs/2026-09-23-note-library-design.md`).

**Branch:** `feat/note-sidebar`, stacked on `feat/note-library` (PR #9). The PR targets `feat/note-library`.

## Global Constraints

- **Startup:** the one new cost before first paint is reading `fastpad.ini`: `bootstrap::run` reads it before the window is created (it used to be read in the deferred `WM_FASTPAD_LOAD_SETTINGS`), so the activity bar and an empty panel frame paint their saved view and width in the first frame and nothing moves afterwards. `WM_FASTPAD_LOAD_SETTINGS` applies those settings and reports their warnings without reading the file again. Task 14's startup bench gate must show warm startup within noise of `feat/note-library`; if it doesn't, Task 14 moves the read back and the sidebar is reconciled in `apply_loaded_settings`. Nothing else new runs before first paint or first input. The panel reads no files before `WM_FASTPAD_LIBRARY_READY`, and shows "Loading…" until then.
- **UI thread:** the tree is built on the scan worker. The UI thread never touches the disk for the sidebar, except for user-initiated file operations (move, rename, delete, reveal) and user-initiated changes to `folders.ini` (close, favorite). The sidebar's notebook lists come from a cache of `folders.ini` filled by the startup step, which already read it.
- **Dependencies:** none new. Only the `editor` module sends `SCI_*` messages.
- **`library.ini`:** version 2 is `version=2` plus `note=<id>|<flags>|<size>|<hash>|<path>`. Flags are `p`, `d`, both, or `-`. Version 1 is read and migrated, keeping `p` and `d`, dropping notebooks, tags, `f` and absolute-path records. Any other version is never overwritten.
- **`folders.ini`:** `version=1`, `open=none` (only when the last session ended with no notebook), `folder=` (recent, at most 10), `favorite=` (at most 50).
- **Per-PC library file:** drops `recent=` and gains `expanded=<relative path>`.
- **`fastpad.ini`:** `sidebar_view` (`notebook`, `search`, `favorites` or `none`; default `notebook`) and `sidebar_width` (96-DPI pixels; default 260; valid range 180–480).
- **Sizes at 96 DPI** (scale with `panel::scale`): activity bar 44 px; panel 260 px default, 180–480 px range; the editor area keeps at least 320 px; rows 26 px; the header 38 px.
- **Tree order in each folder:** pinned notes, then subfolders, then other notes. Each group is in natural, case-insensitive order ("Note 2" before "Note 10"), with ties broken by extension, then by exact name. There are no section headers. Opening, selecting or editing a note never reorders anything.
- **Wording:** the UI says "notebook", never "folder", for the open library: "Open notebook…" (Ctrl+Shift+O), "Close notebook", "Favorite notebooks". Code and file names may keep "folder".
- **Shortcuts:** Ctrl+B toggles the sidebar, Ctrl+Shift+E shows Notebook, Ctrl+K shows Search, Ctrl+Shift+M moves a note to another notebook. F2, Del, Shift+F10 and Apps work while the tree has focus. F6 cycles focus between the activity bar, the panel and the editor.
- **`CommandId`:** removed variants retire their numbers. Every variant from `NoteTogglePin` on gets an explicit discriminant (`NoteTogglePin = 164`, `NoteMoveToNotebook = 165`, `NoteRename = 174`, `NoteDelete = 175`), and new commands start at 176: `ToggleSidebar` 176, `ShowNotebookView` 177, `ShowSearchView` 178, `ShowFavoritesView` 179, `CloseNotebook` 180, `ToggleNotebookFavorite` 181, `NoteRevealInExplorer` 182, `FocusNextPane` 183, `FocusPreviousPane` 184. New window messages: `WM_FASTPAD_NOTEBOOK_CHECKED = WM_APP + 12`, `WM_FASTPAD_SIDEBAR_ACCESSIBLE = WM_APP + 0x60`, `WM_FASTPAD_SIDEBAR_ACTION = WM_APP + 0x61`.
- **Notes mode off:** no activity bar and no panel, and the layout and behavior are exactly what `feat/note-library` has.
- **Commits:** git commit messages carry no attribution lines. Never commit `native/out` or `target/`.
- **Tests:**
  - Compile with `cargo clippy --all-targets -- -D warnings` and check formatting with `cargo fmt --all -- --check`. Run only each task's targeted tests. The full suite runs once, at the final review.
  - A test command with more than one filter puts the filters after `--` (`cargo test --lib -- a b --test-threads=1`).
  - In-process window tests and `tests/windows/*` need `-- --test-threads=1`.
  - Tests never touch the real profile or the real Documents folder. In-process tests already resolve to a scratch profile under cfg(test), and real-exe tests use a scratch `LOCALAPPDATA` and notes folder.
  - Never search the whole disk. Crate sources are in `C:\Users\korn3\.cargo\registry\src\`.

## Review Focus

1. **Notebooks with thousands of notes in one folder, or very deep nesting.** Expanding, scrolling and keyboard paging stay instant, and painting touches only visible rows. Pinned in Task 3 (`ten_thousand_notes_in_one_folder_flatten_quickly`) and Task 7 (`paging_through_ten_thousand_rows_never_leaves_the_list`).
2. **A rescan or outside rename while a row is selected or a folder is expanded.** Selection and expansion follow by path, a vanished selection moves to the nearest row, and nothing panics on a stale index. Pinned in Task 10 (`a_rescan_keeps_selection_and_expansion_by_path`).
3. **The window is narrowed below activity bar + panel + 320 px, or the DPI changes.** The panel shrinks first, never below 0, the editor keeps 320 px when possible, and the saved width is not overwritten by the squeeze. Pinned in Task 6 (`a_narrow_window_squeezes_the_panel_without_saving_it`).
4. **Clicking notes quickly while the preview tab is the active, unedited tab, then typing.** The preview is replaced in place, never dirty, and the first keystroke promotes it, so text is never lost to a replacement. Pinned in Task 9 (`the_first_edit_promotes_the_preview_so_a_later_click_opens_a_new_preview`).
5. **The left window edge and title-strip dragging now that child windows sit under the title row.** Resizing from the left border, dragging the window by the empty panel header or activity-bar top, and double-click to maximize still work. Pinned in Task 6 (`the_sidebar_top_strip_and_panel_header_are_caption`).

## File Map

| File | Responsibility | Task |
|---|---|---|
| `src/library/{model,ids,ops,store,mod,reconcile}.rs` | Pins-only model, `library.ini` v2 with v1 migration; the `reconcile.rs` test helper | 1 |
| `src/window/{commands,command_palette,library_host,name_box,main_window}.rs` | Remove favorite, tag and notebook commands, pickers, name-box purposes (`RenameNotebook`, `RenameTag`), `NameError` and `close_name_box_unless_error`; `execute_command`'s organize arm; the in-process tests (4 deleted, 1 replaced by 2 pin tests) | 1 |
| `tests/windows/library.rs`, `src/bin/fastpad-bench.rs` | `NoteToggleFavorite` becomes `NoteTogglePin` (`\|f\|` becomes `\|p\|`), `PendingOp::SetFavorite` becomes `SetPinned` | 1 |
| `src/library/local.rs`, `src/window/library_host.rs` | `expanded=`, `favorite=`, `open=none`, display names; drop recent notes; `Startup.closed`, `Loaded.closed` | 2 |
| `src/library/{reconcile,mod}.rs`, `src/window/main_window.rs` (tests), `tests/windows/library.rs`, `src/bin/fastpad-bench.rs` | The `rename_path` loop; `RecentFolders { folders }` literals gain `..Default::default()`; `a_metadata_flush_leaves_the_local_file_alone_and_a_recent_change_writes_it` rewritten | 2 |
| `src/library/tree.rs` (new), `src/library/mod.rs` | `NoteTree`, row flattening, navigation helpers; `LibraryState.tree` | 3 |
| `src/library/name_search.rs` (new) | Name matching and ranking | 4 |
| `src/config/{persisted,defaults,mod}.rs` | `sidebar_view`, `sidebar_width`, `clamp_sidebar_width` | 5 |
| `src/window/{side_panel,activity_bar,tooltip}.rs` (new) | Sidebar state, the view seam (`PanelView`, `ViewPaint`, `draw_text`, `UiFonts`), activity bar, empty panel, resize, tooltips | 6 |
| `src/window/{main_window,titlebar,menus,commands,command_palette}.rs`, `src/app.rs`, `src/bootstrap.rs` | Layout slot, `ui_fonts`, `create_ui_font`, `strip_height`, title-strip offset and caption rect, view commands and menu item, `fastpad.ini` read before the window exists | 6 |
| `src/window/{find_bar,name_box,menu_band,accessibility,palette}.rs` | A left edge for everything right of the sidebar (`layout`, `apply_layout`, `heading_rects`, `paint`, `native_layout`); `Palette::panel_background()` | 6 |
| `src/window/row_list.rs` (new) | Virtual row list: state, keyboard, hit-testing, scrolling, painting helper | 7 |
| `src/window/{library_host,commands,menus,command_palette,main_window,messages,mod}.rs` | Open, close and favorite notebooks, explicit-open availability, `folders.ini` cache, wording, sidebar refresh hooks | 8 |
| `src/document.rs`, `src/window/{tabs,titlebar,main_window,library_host}.rs`, `src/app.rs` | Preview tab, `open_note`, italic tab label | 9 |
| `src/window/notebook_view.rs` (new), `side_panel.rs`, `row_list.rs`, `library_host.rs`, `menus.rs` | The Notebook view and its routing through Task 6's seam | 10 |
| `src/window/notebook_view.rs`, `library_host.rs`, `commands.rs`, `command_palette.rs`, `menus.rs`, `main_window.rs`, `src/platform/{files,shell,mod}.rs`, `src/document.rs` | Context menus, move to notebook, reveal, new-note destination | 11 |
| `src/window/{favorites_view,search_view}.rs` (new), `side_panel.rs`, `library_host.rs`, `command_palette.rs`, `main_window.rs`, `activity_bar.rs` | Favorites view, Search view, Settings button | 12 |
| `src/window/sidebar_accessibility.rs` (new), `side_panel.rs`, `activity_bar.rs`, the three views, `main_window.rs`, `commands.rs`, `menus.rs`, `command_palette.rs` | MSAA providers, win events, F6/Esc focus cycle | 13 |
| `tests/windows/library.rs`, `src/bin/fastpad-bench.rs`, `benchmarks/README.md`, `README.md`, the sidebar spec | End-to-end tests, bench, docs | 14 |

## Task Interfaces (the contract every task follows)

The names below are binding. A task may add private helpers, but it must not rename these.

- **Task 1 — pins-only library:**
  - `NoteRecord { id: NoteId, pinned: bool, deleted: bool, size: u64, hash: u64, path: PathBuf }`.
  - `Library { notes: Vec<NoteRecord> }`, with `NoteRecord::has_metadata(&self) -> bool` returning `self.pinned`.
  - `PendingOp::{SetPinned { note, value }, Relocate { note, path }, SetFingerprint { note, size, hash }, SetDeleted { note, value }, Drop { id }}`.
  - `store::parse(source: &str) -> Option<Library>` accepts versions 1 and 2. `store::encode` writes version 2.
  - `LibraryState::is_pinned(&self, path: &Path) -> bool`.
  - `library_host::toggle_pin(hwnd: HWND, path: &Path)`, which checks `ready_library` first and refuses a path outside the open notebook with the notice "Only notes in the open notebook can be pinned.".
  - `NotebookId`, `TagId`, `Notebook`, `Tag` and `NotebookColor` are deleted. `PickerKind` keeps only `RecentFolder`. `NamePurpose::RenameNotebook` and `RenameTag`, `NameError` and `close_name_box_unless_error` go. `LibraryError` keeps only `NotFound`.
  - `CommandId::{NoteTogglePin = 164, NoteMoveToNotebook = 165, NoteRename = 174, NoteDelete = 175}`; 163 and 166–173 are retired. `NoteMoveToNotebook` has no palette row until Task 11, and Task 1 exempts it in `every_command_except_tab_positions_and_the_palette_is_listed_once`.
- **Task 2 — local files:**
  - `LocalState.expanded: Vec<PathBuf>`, `LocalState::set_expanded(&mut self, path: &Path, expanded: bool)`, `LocalState::is_expanded(&self, path: &Path) -> bool`.
  - `Conveniences { autosave: bool, expanded: Vec<PathBuf>, missing: Vec<(u64, NoteId)> }`.
  - `RecentFolders { folders: Vec<PathBuf>, favorites: Vec<PathBuf>, closed: bool }` (`Default`), with:
    - `toggle_favorite(&mut self, folder: &Path) -> bool` (returns whether it's now a favorite; at the 50-favorite cap it adds nothing and returns false);
    - `is_favorite(&self, folder: &Path) -> bool`;
    - `set_closed(&mut self, closed: bool)`.
    `push` clears `closed`.
  - `local::display_names(folders: &[PathBuf]) -> Vec<(String, Option<String>)>`: the folder name, plus the dim parent-folder hint only on a name clash.
  - `library_host::resolve_startup` opens nothing when `closed` is set and the command line named no folder. `Startup` gains `closed`, and `Loaded` gains a private `closed: bool`. With `open=none` and no command-line folder, `open_library_step` returns without a worker, the host's folder stays `None`, and no notice is shown.
- **Task 3 — tree:** in `src/library/tree.rs`:
  - `UnsavedEntry { key: u64, label: String }`, where `key` is the tab's `DocumentId.0`.
  - `RowKind::{Unsaved(u64), Folder(PathBuf), Note(PathBuf)}` and `TreeRow { kind: RowKind, depth: u16, name: String, pinned: bool, expanded: bool }`. Both derive `Clone, Debug, Eq, PartialEq` and have public fields.
  - `NoteTree::build(notes: &[PathBuf], pinned: &[PathBuf]) -> NoteTree`, where paths are relative to the notebook. `NoteTree` derives only `Debug` and `Default`, and drops iteratively (no recursion).
  - `NoteTree` methods: `insert_note(&mut self, path: &Path, pinned: bool)`, `remove_note(&mut self, path: &Path)`, `rename_note(&mut self, old: &Path, new: &Path)`, `set_pinned(&mut self, path: &Path, pinned: bool)`, `note_count(&self) -> usize`, `rows(&self, expanded: &dyn Fn(&Path) -> bool, unsaved: &[UnsavedEntry]) -> Vec<TreeRow>`.
  - Free functions: `natural_cmp(a: &str, b: &str) -> std::cmp::Ordering`, `row_index(rows: &[TreeRow], kind: &RowKind) -> Option<usize>` (paths compared ignoring case), `parent_index(rows: &[TreeRow], index: usize) -> Option<usize>`, `type_ahead(rows: &[TreeRow], from: usize, prefix: &str) -> Option<usize>`, `ancestors(path: &Path) -> Vec<PathBuf>` (`a\b\c.md` gives `a` and `a\b`: outermost first, without the root).
  - `LibraryState.tree: NoteTree` is built on the worker in `load`. `merge_rescan`, which runs on the UI thread, updates it in place and never rebuilds it. `add_note`, `remove_note`, `rename_note` and `apply(SetPinned)` keep it current, and `flush` syncs pins when it re-reads `library.ini`.
- **Task 4 — name search:** `name_search::NameMatch { path: PathBuf, name: String, folder: String }` and `name_search::search(notes: &[PathBuf], query: &str, limit: usize) -> Vec<NameMatch>` (lowercases on each call, no cache; `select_nth_unstable_by` for the top `limit`).
- **Task 5 — settings:**
  - `config::SidebarView::{Notebook, Search, Favorites, Hidden}` (`Clone, Copy, Debug, Default, Eq, PartialEq`), with ini tokens `notebook`, `search`, `favorites` and `none`, and `SidebarView::token(self) -> &'static str`.
  - `Settings.sidebar_view: SidebarView` and `Settings.sidebar_width: u16`, with matching `SettingsDelta` fields. A width outside the range is clamped with no warning; a bad token gives a warning.
  - `DEFAULT_SIDEBAR_WIDTH = 260`, `MIN_SIDEBAR_WIDTH = 180`, `MAX_SIDEBAR_WIDTH = 480` as `u16` in `crate::config::defaults` (now `pub mod`), and `clamp_sidebar_width(width: u16) -> u16`, all re-exported from `crate::config`.
- **Task 6 — shell:**
  - `App.sidebar: Option<side_panel::Sidebar>`, created with the main window when `notes_mode` is on, and destroyed or recreated when notes mode changes. `Sidebar` has the crate-visible fields `bar: HWND`, `panel: HWND`, `tooltip: Option<Tooltip>` and `bar_state: BarState`, and `Sidebar::fonts(&mut self, dpi) -> UiFonts`. Later tasks add `notebook` (10), `favorites` and `search` (12) and `bar_focus` (13).
  - `bootstrap::run` reads `fastpad.ini` (`config::load()`) before the window is created and keeps its warnings in `App.preloaded_settings_warnings`, which `WM_FASTPAD_LOAD_SETTINGS` reports instead of reading the file again.
  - In `side_panel`:
    - `create(hwnd: HWND) -> crate::Result<Sidebar>`;
    - `left_edge(hwnd: HWND) -> i32` (activity bar plus the open panel, in device pixels; 0 with notes mode off);
    - `layout(hwnd: HWND, client: RECT, dpi: u32)`;
    - `show_view(hwnd: HWND, view: SidebarView, focus: bool)`;
    - `toggle(hwnd: HWND)`;
    - `refresh(hwnd: HWND)`, which re-reads library state and repaints;
    - `active_tab_changed(hwnd: HWND)`, called at the end of `main_window::refresh_tabs`;
    - `notes_mode_changed(hwnd: HWND, enabled: bool)`;
    - `current_view(hwnd: HWND) -> SidebarView`;
    - `windows(hwnd: HWND) -> Option<(HWND, HWND)>` (bar, panel);
    - `sidebar_widths(client_width, dpi, view_open, width_96) -> (i32, i32)` and `drag_width_96(panel_px, dpi) -> u16`;
    - `paint_buffered`, `point_of`, `register_child_class`, `with_bar_state`, and `ACTIVITY_WIDTH_96`, `EDITOR_MIN_WIDTH_96`, `HEADER_HEIGHT_96`, `GRIP_WIDTH_96`.
  - The view seam, in `side_panel`:
    - `ViewPaint { hdc: HDC, client: RECT, palette: Palette, background: u32, fonts: UiFonts, dpi: u32, focused: bool }` (`Clone, Copy`), built per `WM_PAINT` by `view_paint(main: HWND, panel: HWND, hdc: HDC, client: RECT) -> ViewPaint`;
    - `unsafe fn draw_text(hdc: HDC, text: &str, rect: RECT, font: HFONT, color: u32, flags: DRAW_TEXT_FORMAT) -> i32` (returns the width drawn; empty text draws nothing);
    - `PanelView::{Notebook, Search, Favorites}` with the private dispatch `paint_view(main, view, &ViewPaint)`, `view_mouse(main, view, panel, message, wparam, lparam) -> Option<LRESULT>`, `view_key(main, view, panel, message, wparam, lparam) -> Option<LRESULT>` and `header_is_caption(main, view, panel, x, y) -> bool`. Tasks 10 and 12 replace their arms; `None` falls through to `DefWindowProcW`.
  - The one font API: `titlebar::create_ui_font(pixel_height: i32, face: &str, weight: i32, italic: bool) -> HFONT` (the private `create_font` wraps it), `side_panel::UiFonts { text, bold, italic, glyph, bar_glyph }` (`Clone, Copy, Debug, Default`), cached per DPI on the `Sidebar`, and `main_window::ui_fonts(hwnd: HWND) -> UiFonts` (null handles without a sidebar; call it with nothing of the App borrowed).
  - `TitleBarLayout::calculate_with_offset(client: Size, dpi: u32, tab_count: usize, scroll: i32, preview_buttons: bool, left: i32) -> Self`, `TitleBarLayout.sidebar: Rect` (from 0 to `left`, hit-tested as caption; `drag_region` is unchanged) and `titlebar::strip_height(dpi: u32) -> i32`.
  - `activity_bar::{ActivityButton, BarState, button_at, activate}`, `activity_bar::button_rects(client: RECT, dpi: u32) -> [RECT; 4]` (Notebook, Search, Favorites, Settings), and the Settings click handler `activity_bar::open_settings(main)`, which Task 12 repoints. Class names `"FastPadActivityBar"` and `"FastPadSidePanel"`.
  - `Palette::panel_background(&self) -> u32`, the panel's fill.
  - `CommandId::{ToggleSidebar = 176, ShowNotebookView = 177, ShowSearchView = 178, ShowFavoritesView = 179}`, `CommandId::is_sidebar()`, `menus::set_sidebar_enabled`, and the View menu entry "Side&bar\tCtrl+B".
  - `main_window::{change_setting, open_command_palette}` become `pub(crate)`.
  - `tooltip::Tooltip::create(owner: HWND) -> Option<Tooltip>` and `Tooltip::set_tool(&self, id: usize, rect: RECT, text: &str)`. An empty `text` removes the tool (`TTM_DELTOOLW` only).
- **Task 7 — row list:** in `src/window/row_list.rs`:
  - `RowListState { count: usize, selected: Option<usize>, hover: Option<usize>, top: usize, row_height: i32 }` (plus a private wheel remainder).
  - `ListKey::{Up, Down, Home, End, PageUp, PageDown}` and `ListKey::from_virtual_key(key: u32) -> Option<ListKey>`.
  - Methods:
    - `new(row_height: i32) -> Self`;
    - `set_count(&mut self, count: usize)`;
    - `visible_rows(&self, height: i32) -> usize`;
    - `row_at(&self, y: i32) -> Option<usize>`;
    - `row_top(&self, index: usize) -> Option<i32>`;
    - `move_selection(&mut self, key: ListKey, height: i32) -> bool`;
    - `select(&mut self, index: usize, height: i32)`;
    - `ensure_visible(&mut self, index: usize, height: i32)`;
    - `scroll_lines(&mut self, lines: i32, height: i32) -> bool`;
    - `wheel(&mut self, delta: i32, lines_per_notch: u32, height: i32) -> bool`;
    - `set_hover(&mut self, hover: Option<usize>) -> bool`;
    - `thumb(&self, height: i32) -> Option<(i32, i32)>`, `thumb_hit(&self, x: i32, y: i32, width: i32, height: i32) -> Option<i32>`;
    - `drag_thumb(&mut self, grab_offset: i32, y: i32, height: i32) -> bool`.
  - Every `height` is the list area's height and every `y` is relative to the list's top.
  - `row_list::paint(hdc: HDC, area: RECT, state: &RowListState, palette: &Palette, focused: bool, draw_row: &mut dyn FnMut(HDC, usize, RECT, RowLook))`, a safe `fn` that fills the selected and hovered rows' backgrounds, calls `draw_row` for the rows in view only, and paints the scroll thumb. `RowLook { selected: bool, hover: bool, focused: bool }`, and `row_list::thumb_width(row_height: i32) -> i32`.
- **Task 8 — notebook lifecycle:** in `library_host`:
  - `close_notebook(hwnd)` and `toggle_notebook_favorite(hwnd)`; at the cap it shows "You can keep up to 50 favorite notebooks.";
  - `remove_favorite(hwnd, folder: &Path)`;
  - `favorites(hwnd) -> Vec<PathBuf>` and `recent_notebooks(hwnd) -> Vec<PathBuf>`, answered from a cache of `folders.ini` (filled by `open_library_step` and the startup worker, replaced by every user change), never from the disk;
  - `is_favorite(hwnd) -> bool`;
  - `open_listed_notebook(hwnd, folder: &Path)`: an explicit open, checked on a worker (`WM_FASTPAD_NOTEBOOK_CHECKED = WM_APP + 12`, handled by `notebook_checked(hwnd, lparam)`). A missing folder shows the notice "<path> is not available." and changes nothing, and there's no fallback. The private `check_listed_notebook(hwnd, folder, show_notebook: Option<bool>)` does the work.
  - `CommandId::{CloseNotebook = 180, ToggleNotebookFavorite = 181}`.
  - Every place the library changes calls `side_panel::refresh(hwnd)`.
  - With no notebook open, Save As suggests "Untitled.txt" with no starting folder.
- **Task 9 — preview tab:**
  - `Document.preview: bool`.
  - `Tabs::preview_id(&self) -> Option<DocumentId>`, `Tabs::replace_preview(&mut self, document: Document) -> Option<Document>` (in place, or pushed when there is no preview), `Tabs::promote(&mut self, id: DocumentId) -> bool`, and `Tabs::note_active_text_change` returns whether it promoted.
  - `main_window::OpenMode::{Preview, Permanent}` and `main_window::open_note(hwnd: HWND, path: &Path, mode: OpenMode, focus_editor: bool) -> crate::Result<()>`.
  - The first text change, a save, and a double-click on the tab promote (the tab double-click is detected with `GetMessageTime`, `App.last_tab_click`). `TitleFontHandles::italic() -> HFONT`, made with `create_ui_font`, draws the preview label (`TitlePaint.preview_tab`).
- **Task 10 — Notebook view:**
  - `notebook_view::NotebookView` (rows, `RowListState`, hover pin, header buttons), `NotebookView::new(panel)`, and the accessors `rows()`, `list()`, `list_mut()`, `list_area(client, dpi) -> RECT`, `buttons(client, dpi) -> Vec<(String, RECT)>` and `recent_rows(client, dpi) -> Vec<(String, RECT)>`.
  - `notebook_view::{rebuild, active_tab_changed, paint(hwnd, &ViewPaint), handle(hwnd, message, wparam, lparam) -> Option<LRESULT>, header_hit, activate, key_down, header_clicked}`, `Mode`, `HeaderButton` and `Activation`. The panel's `WM_LBUTTONDBLCLK` on a note opens it permanently, which promotes its preview.
  - `library_host::set_expanded(hwnd, path: &Path, expanded: bool)` and `library_host::expanded(hwnd) -> Vec<PathBuf>`.
  - `Sidebar.notebook: NotebookView`. The Notebook arms of `paint_view`, `view_mouse`, `view_key` and `header_is_caption` route to `notebook_view`, and `side_panel::{refresh, active_tab_changed, show_view}` rebuild its rows.
  - `menus::track_popup`, `menus::MenuEntry` and `MenuEntry::command` become `pub(crate)`.
- **Task 11 — context menus and moves:**
  - `library_host::move_to_notebook(hwnd, path: &Path)` (picker `PickerKind::MoveToNotebook`: favorites, recent, "Browse…").
  - `platform::files::move_file(from: &Path, to: &Path) -> crate::Result<()>` (no replace; copy allowed across volumes).
  - `platform::shell::reveal_in_explorer(path: &Path) -> crate::Result<()>` (`ShellExecuteW` `explorer.exe /select`, recorded under cfg(test)) and `library_host::reveal(hwnd, path)`, which reports a failure as a notice.
  - `Document.save_folder: Option<PathBuf>`, `library_host::new_note_in(hwnd, folder: Option<PathBuf>)` and `first_save_folder(hwnd)`.
  - Row context menus read `track_popup`'s command locally: "Open in new tab" is `CommandId::Open` and "New note here" is `CommandId::New`.
  - `CommandId::{NoteRevealInExplorer = 182}`, Ctrl+Shift+M for `NoteMoveToNotebook`, and the palette entries "Note: Move to notebook..." and "Note: Reveal in Explorer". Task 1's exemption for `NoteMoveToNotebook` is removed.
- **Task 12 — Favorites, Search, Settings:**
  - `favorites_view::FavoritesView` and `search_view::SearchView` (it owns a native EDIT child, with a painted placeholder since there is no `EM_SETCUEBANNER`). `Sidebar.favorites` and `Sidebar.search`.
  - The Favorites row menu follows Task 11's pattern: `OpenFolder`, `ToggleNotebookFavorite` and `NoteRevealInExplorer`, read locally.
  - `library_host::open_listed_notebook_in_view(hwnd, folder: &Path, focus: bool)`: shows the Notebook view once the notebook has opened.
  - `command_palette::SETTINGS_COMMANDS: &[CommandId]` (settings plus `TabWidth2/4/8`) and `main_window::open_settings_palette(hwnd)`, which shows only those entries. `activity_bar::open_settings` calls it.
- **Task 13 — accessibility and focus:**
  - `sidebar_accessibility::AccessibleItem { name: String, role: u32, state: u32, rect: RECT, value: String }`, with the providers answering `WM_GETOBJECT(OBJID_CLIENT)` on the activity bar and panel windows. Queries from other threads go to the UI thread as `WM_FASTPAD_SIDEBAR_ACCESSIBLE = WM_APP + 0x60`, and actions are posted as `WM_FASTPAD_SIDEBAR_ACTION = WM_APP + 0x61`.
  - `side_panel::accessible_item_count(panel: HWND) -> usize` and `side_panel::accessible_item(panel: HWND, index: usize) -> Option<AccessibleItem>`, so a 10,000-row tree is never listed whole.
  - `activity_bar::accessible_items(bar: HWND) -> Vec<AccessibleItem>` (always 4).
  - `Sidebar.bar_focus: usize`, `side_panel::{bar_focus, set_bar_focus}`.
  - `main_window::cycle_focus(hwnd, backwards: bool)` and `return_focus_to_editor(hwnd)`, and `CommandId::{FocusNextPane = 183, FocusPreviousPane = 184}` (F6 and Shift+F6, accelerators only, exempted in the palette completeness test).
- **Task 14:** tests, bench and docs only. `LIBRARY_SCAN_FAVORITES` becomes `LIBRARY_SCAN_PINS`, and the startup bench gate checks Task 6's early `fastpad.ini` read.

**Table sizes:** every task that adds or removes commands says how much `COMMANDS`, `ENTRIES` and `accelerator_specs` grow or shrink, never a total. In order: Task 1 removes 9 commands and 10 palette entries; Task 6 adds 4 commands, 4 palette entries and 3 accelerators; Task 8 adds 2 commands and 2 palette entries; Task 11 adds 1 command, 1 accelerator (Ctrl+Shift+M) and 2 palette entries; Task 13 adds 2 commands and 2 accelerators (F6, Shift+F6).

---

### Task 1: Pins-only library and `library.ini` version 2

**Files:**
- Modify: `src/library/model.rs` (full rewrite), `src/library/store.rs` (full rewrite), `src/library/ops.rs` (full rewrite)
- Modify: `src/library/ids.rs` (drop `NotebookId` and `TagId`), `src/library/mod.rs` (`is_pinned`, doc, tests), `src/library/reconcile.rs` (test helper)
- Modify: `src/window/commands.rs`, `src/window/command_palette.rs`, `src/window/name_box.rs`, `src/window/library_host.rs`
- Modify: `src/window/main_window.rs` (`execute_command` and the library tests: they name the removed commands, so the crate does not compile without this)
- Modify: `tests/windows/library.rs` and `src/bin/fastpad-bench.rs` (they use `NoteToggleFavorite` and `PendingOp::SetFavorite`; `cargo clippy --all-targets` compiles them)

**Interfaces:**
- Consumes: nothing new.
- Produces:
  - `NoteRecord { id: NoteId, pinned: bool, deleted: bool, size: u64, hash: u64, path: PathBuf }`, with `NoteRecord::has_metadata(&self) -> bool` returning `self.pinned`
  - `Library { notes: Vec<NoteRecord> }`, `LibraryError::NotFound` (the only variant left)
  - `PendingOp::{SetPinned { note, value }, Relocate { note, path }, SetFingerprint { note, size, hash }, SetDeleted { note, value }, Drop { id }}`
  - `store::parse(source: &str) -> Option<Library>` (versions 1 and 2), `store::encode(&Library) -> String` (version 2)
  - `LibraryState::is_pinned(&self, path: &Path) -> bool`
  - `library_host::toggle_pin(hwnd: HWND, path: &Path)`
  - `CommandId::{NoteTogglePin = 164, NoteMoveToNotebook = 165, NoteRename = 174, NoteDelete = 175}`; 163 and 166–173 are retired
  - `PickerKind::RecentFolder` (the only variant), `NamePurpose::{FirstSave, RenameNote}`
  - Deleted: `NotebookId`, `TagId`, `Notebook`, `Tag`, `NotebookColor`, `NameError`, `normalize_name`, `normalize_tag_name`, `same_name`, `library_host::organize`, `library_host::Target`

- [ ] **Step 1: Write the failing tests for the store**

Replace the `mod tests` block at the bottom of `src/library/store.rs` with:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::ids::NoteId;
    use crate::library::model::NoteRecord;

    fn sample() -> Library {
        let mut pinned = NoteRecord::new(NoteId(0xa), PathBuf::from(r"sub\a b|c.md"));
        pinned.pinned = true;
        pinned.size = 12;
        pinned.hash = 0xfeed;
        let mut deleted = NoteRecord::new(NoteId(0xb), PathBuf::from("b.md"));
        deleted.deleted = true;
        let mut both = NoteRecord::new(NoteId(0xc), PathBuf::from("c.md"));
        both.pinned = true;
        both.deleted = true;
        Library {
            notes: vec![pinned, deleted, both],
        }
    }

    #[test]
    fn a_library_round_trips_through_its_text_form() {
        // Break caught: a path with `|` or a flag combination not surviving a write and read.
        let text = encode(&sample());
        assert!(text.starts_with("version=2\r\n"));
        assert!(text.contains(
            "note=0000000000000000000000000000000a|p|12|000000000000feed|sub\\a b|c.md\r\n"
        ));
        assert!(text.contains("note=0000000000000000000000000000000b|d|0|0000000000000000|b.md\r\n"));
        assert!(text.contains("note=0000000000000000000000000000000c|pd|0|0000000000000000|c.md\r\n"));
        assert_eq!(parse(&text), Some(sample()));
    }

    #[test]
    fn a_version_one_file_keeps_its_pins_and_deleted_flags_and_drops_the_rest() {
        // Break caught: a file from the first note-library builds read as unreadable (turning
        // pinning off), or its favorites, notebooks and tags surviving into a version 2 write.
        let text = "version=1\r\n\
            notebook=00000000000000000000000000000001|0|teal|100|200|Work\r\n\
            tag=00000000000000000000000000000002|idea\r\n\
            note=0000000000000000000000000000000a|00000000000000000000000000000001|fp|\
            00000000000000000000000000000002|12|000000000000feed|sub\\a b|c.md\r\n\
            note=0000000000000000000000000000000b|-|fd|-|0|0000000000000000|b.md\r\n\
            note=0000000000000000000000000000000c|-|f|-|0|0000000000000000|c.md\r\n";
        let library = parse(text).unwrap();
        let flags: Vec<_> = library
            .notes
            .iter()
            .map(|note| (note.id, note.pinned, note.deleted))
            .collect();
        assert_eq!(
            flags,
            [
                (NoteId(0xa), true, false),
                (NoteId(0xb), false, true),
                (NoteId(0xc), false, false)
            ]
        );
        assert_eq!(library.notes[0].path, PathBuf::from(r"sub\a b|c.md"));
        assert_eq!((library.notes[0].size, library.notes[0].hash), (12, 0xfeed));
        let rewritten = encode(&library);
        assert!(rewritten.starts_with("version=2\r\n"));
        assert!(!rewritten.contains("notebook=") && !rewritten.contains("tag="));
        assert!(rewritten.contains(
            "note=0000000000000000000000000000000a|p|12|000000000000feed|sub\\a b|c.md\r\n"
        ));
    }

    #[test]
    fn only_version_one_and_two_files_are_readable() {
        // Break caught: a newer FastPad's file (or a damaged one) read as an empty library and
        // then overwritten, destroying every pin.
        assert_eq!(parse("note=x\r\n"), None);
        assert_eq!(parse("version=3\r\n"), None);
        assert_eq!(parse("version=1\r\n"), Some(Library::default()));
        assert_eq!(parse("\u{feff}version=2\r\n"), Some(Library::default()));
    }

    #[test]
    fn malformed_lines_unknown_keys_and_unknown_flags_are_tolerated() {
        let text = "version=2\n\
            future=1\n\
            note=00000000000000000000000000000007|zq|5|0000000000000001|a.md\n\
            note=short\n\
            note=00000000000000000000000000000008|p|x|0000000000000001|b.md\n";
        let library = parse(text).unwrap();
        assert_eq!(library.notes.len(), 1);
        let note = &library.notes[0];
        assert!(!note.pinned && !note.deleted, "unknown flags are ignored");
        assert_eq!((note.size, note.hash), (5, 1));
    }

    #[test]
    fn absolute_path_records_are_dropped_on_read_and_never_written() {
        // Break caught: a version 1 record for a file outside the folder surviving into version
        // 2, which has no way to say which drive it meant.
        let text = "version=1\r\n\
            note=0000000000000000000000000000000a|-|p|-|0|0000000000000000|C:\\elsewhere\\log.txt\r\n\
            note=0000000000000000000000000000000b|-|p|-|0|0000000000000000|\\rooted.md\r\n\
            note=0000000000000000000000000000000c|-|p|-|0|0000000000000000|..\\up.md\r\n\
            note=0000000000000000000000000000000d|-|p|-|0|0000000000000000|kept.md\r\n";
        let library = parse(text).unwrap();
        let kept: Vec<_> = library.notes.iter().map(|note| note.id).collect();
        assert_eq!(kept, [NoteId(0xd)]);
        let mut outside = NoteRecord::new(NoteId(0xe), PathBuf::from(r"D:\x.md"));
        outside.pinned = true;
        assert_eq!(
            encode(&Library {
                notes: vec![outside]
            }),
            "version=2\r\n"
        );
    }

    #[test]
    fn read_distinguishes_absent_loaded_and_unreadable_and_write_returns_the_stamp() {
        let dir = std::env::temp_dir().join(format!("fastpad-store-io-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = library_file(&dir);
        assert_eq!(path, dir.join(".fastpad").join("library.ini"));
        assert!(matches!(read(&path), ReadOutcome::Absent));
        let stamp = write(&path, &sample()).unwrap();
        assert_eq!(super::stamp(&path), Some(stamp));
        match read(&path) {
            ReadOutcome::Loaded(library, read_stamp) => {
                assert_eq!(library, sample());
                assert_eq!(read_stamp, stamp);
            }
            _ => panic!("expected a loaded library"),
        }
        // Break caught: reading a version 1 file rewriting it before anything changed.
        std::fs::write(
            &path,
            "version=1\r\nnote=0000000000000000000000000000000a|-|p|-|0|0000000000000000|a.md\r\n",
        )
        .unwrap();
        let before = std::fs::read(&path).unwrap();
        assert!(matches!(read(&path), ReadOutcome::Loaded(..)));
        assert_eq!(std::fs::read(&path).unwrap(), before, "reading never rewrites");
        std::fs::write(&path, "version=9\r\n").unwrap();
        assert!(matches!(read(&path), ReadOutcome::Unreadable));
        std::fs::write(&path, [0xff, 0xfe, 0x00]).unwrap();
        assert!(matches!(read(&path), ReadOutcome::Unreadable));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_held_open_by_another_process_reads_as_busy_not_unreadable() {
        // Break caught: a sharing violation while OneDrive syncs library.ini being taken for a
        // damaged file, which turns pinning off for the whole session.
        use std::os::windows::fs::OpenOptionsExt;
        let dir = std::env::temp_dir().join(format!("fastpad-store-busy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = library_file(&dir);
        write(&path, &sample()).unwrap();
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&path)
            .unwrap();
        assert!(matches!(read(&path), ReadOutcome::Busy));
        drop(lock);
        assert!(matches!(read(&path), ReadOutcome::Loaded(..)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_read_that_races_a_write_is_retried_and_never_pairs_old_bytes_with_a_new_stamp() {
        // Break caught: the stamp taken after the bytes, so a sync landing mid-read left FastPad
        // holding the old library under the new file's stamp, and the next flush wrote over the
        // synced change without re-reading it.
        let dir = std::env::temp_dir().join(format!("fastpad-store-race-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = library_file(&dir);
        write(&path, &Library::default()).unwrap();
        let mut calls = 0;
        let outcome = read_via(&path, |path| {
            calls += 1;
            let bytes = std::fs::read(path);
            if calls == 1 {
                std::thread::sleep(std::time::Duration::from_millis(20));
                write(path, &sample()).unwrap();
            }
            bytes
        });
        match outcome {
            ReadOutcome::Loaded(library, stamp) => {
                assert_eq!(calls, 2);
                assert_eq!(library, sample());
                assert_eq!(Some(stamp), super::stamp(&path));
            }
            _ => panic!("expected the retried read to load"),
        }
        let always_changing = read_via(&path, |path| {
            let bytes = std::fs::read(path);
            std::fs::write(path, format!("{}x", std::fs::read_to_string(path).unwrap())).unwrap();
            bytes
        });
        assert!(matches!(always_changing, ReadOutcome::Busy));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib library::store -- --test-threads=1`
Expected: a compile error (`NoteRecord` still has `notebook`, `favorite` and `tags`; `Library` still has `notebooks` and `tags`).

- [ ] **Step 3: Rewrite the model**

Replace the whole of `src/library/model.rs` with:

```rust
//! Note records, the only metadata a notebook keeps: a pin and a deleted flag per note, attached
//! to its file by path, stable ID and content fingerprint.

use super::ids::NoteId;
use std::path::{Path, PathBuf};

/// One note's metadata. `path` is relative to the notebook.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NoteRecord {
    pub id: NoteId,
    pub pinned: bool,
    /// Sent to the Recycle Bin by FastPad: kept, hidden, until the purge or a restore.
    pub deleted: bool,
    pub size: u64,
    pub hash: u64,
    pub path: PathBuf,
}

impl NoteRecord {
    pub fn new(id: NoteId, path: PathBuf) -> Self {
        Self {
            id,
            pinned: false,
            deleted: false,
            size: 0,
            hash: 0,
            path,
        }
    }

    /// Whether the record is worth keeping. Only a pin is; the deleted flag alone is not.
    pub fn has_metadata(&self) -> bool {
        self.pinned
    }
}

/// Names a note for an operation: by stable ID first, then by path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NoteRef {
    pub id: NoteId,
    pub path: PathBuf,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LibraryError {
    NotFound,
}

impl std::fmt::Display for LibraryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => formatter.write_str("That item no longer exists."),
        }
    }
}

/// NTFS compares names ignoring case, so the library does too.
pub fn same_path(left: &Path, right: &Path) -> bool {
    left.as_os_str().to_string_lossy().to_lowercase()
        == right.as_os_str().to_string_lossy().to_lowercase()
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Library {
    pub notes: Vec<NoteRecord>,
}

impl Library {
    pub fn note(&self, id: NoteId) -> Option<&NoteRecord> {
        self.notes.iter().find(|note| note.id == id)
    }

    pub fn note_by_path(&self, path: &Path) -> Option<&NoteRecord> {
        self.notes.iter().find(|note| same_path(&note.path, path))
    }

    /// An existing record, by ID and then by path. Never creates one.
    pub fn find_note_mut(&mut self, target: &NoteRef) -> Option<&mut NoteRecord> {
        let index = self
            .notes
            .iter()
            .position(|note| note.id == target.id)
            .or_else(|| {
                self.notes
                    .iter()
                    .position(|note| same_path(&note.path, &target.path))
            })?;
        self.notes.get_mut(index)
    }

    /// The record for `target`, created with its ID and path when none exists.
    pub fn resolve_note(&mut self, target: &NoteRef) -> &mut NoteRecord {
        let index = self
            .notes
            .iter()
            .position(|note| note.id == target.id)
            .or_else(|| {
                self.notes
                    .iter()
                    .position(|note| same_path(&note.path, &target.path))
            });
        let index = index.unwrap_or_else(|| {
            self.notes
                .push(NoteRecord::new(target.id, target.path.clone()));
            self.notes.len() - 1
        });
        &mut self.notes[index]
    }

    /// Drops records that carry no pin. Run before every write.
    pub fn prune(&mut self) {
        self.notes.retain(NoteRecord::has_metadata);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_finds_by_id_then_by_path_ignoring_case_then_creates() {
        let mut library = Library::default();
        library.resolve_note(&NoteRef {
            id: NoteId(1),
            path: "Plan.md".into(),
        });
        let by_path = library.resolve_note(&NoteRef {
            id: NoteId(2),
            path: "plan.MD".into(),
        });
        assert_eq!(by_path.id, NoteId(1));
        let by_id = library.resolve_note(&NoteRef {
            id: NoteId(1),
            path: "other.md".into(),
        });
        assert_eq!(by_id.path, PathBuf::from("Plan.md"));
        library.resolve_note(&NoteRef {
            id: NoteId(3),
            path: "new.md".into(),
        });
        assert_eq!(library.notes.len(), 2);
    }

    #[test]
    fn prune_keeps_pinned_records_and_drops_the_rest() {
        // Break caught: an unpinned note's record staying in library.ini forever, or a pinned
        // note that was deleted losing its pin before the purge decides.
        let mut library = Library::default();
        for (id, path, pinned, deleted) in [
            (1, "a.md", true, false),
            (2, "b.md", false, false),
            (3, "c.md", true, true),
            (4, "d.md", false, true),
        ] {
            let record = library.resolve_note(&NoteRef {
                id: NoteId(id),
                path: path.into(),
            });
            record.pinned = pinned;
            record.deleted = deleted;
        }
        library.prune();
        let kept: Vec<_> = library.notes.iter().map(|note| note.id).collect();
        assert_eq!(kept, [NoteId(1), NoteId(3)]);
    }

    #[test]
    fn paths_compare_ignoring_case_like_ntfs() {
        assert!(same_path(
            Path::new(r"Sub\Plan.md"),
            Path::new(r"sub\plan.MD")
        ));
        assert!(!same_path(Path::new("a.md"), Path::new("b.md")));
    }
}
```

In `src/library/ids.rs`, delete the two lines `library_id!(NotebookId);` and `library_id!(TagId);`, and in `ids_round_trip_as_32_hex_digits_and_reject_anything_else` replace the last assertion with:

```rust
        assert_eq!(
            NoteId::parse_hex(&"f".repeat(32)),
            Some(NoteId(u128::MAX))
        );
```

- [ ] **Step 4: Rewrite the store**

Replace everything in `src/library/store.rs` above `#[cfg(test)] mod tests` with:

```rust
//! `.fastpad\library.ini`: the pins that travel with the notebook.
//!
//! ```text
//! version=2
//! note=<id>|<flags>|<size>|<hash>|<path>
//! ```
//!
//! `<flags>` is `p` (pinned), `d` (deleted), both, or `-`. The path is relative to the notebook,
//! last and unescaped, so splitting a `note` line on its first four `|` characters is
//! unambiguous.
//!
//! Version 1 (the first note-library builds) is still read: its `notebook=` and `tag=` lines are
//! dropped, and each `note=<id>|<notebook>|<flags>|<tags>|<size>|<hash>|<path>` line keeps only
//! its `p` and `d` flags. It becomes version 2 at the next flush that has something to write;
//! reading alone never rewrites it. Records whose path is not a plain relative path (version 1
//! allowed absolute ones for files outside the folder) are dropped on read and never written. Any
//! other version, or none, makes the whole file unreadable, and an unreadable file is never
//! overwritten.

use super::ids::NoteId;
use super::model::{Library, NoteRecord};
use crate::Result;
use std::path::{Component, Path, PathBuf};

const VERSION: &str = "2";
const VERSION_1: &str = "1";
pub const LIBRARY_DIR: &str = ".fastpad";
const LIBRARY_FILE: &str = "library.ini";

pub fn library_file(folder: &Path) -> PathBuf {
    folder.join(LIBRARY_DIR).join(LIBRARY_FILE)
}

/// Size and last-write time (nanoseconds since the Unix epoch), used to notice outside changes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileStamp {
    pub size: u64,
    pub modified: u64,
}

#[cfg(test)]
thread_local! {
    static STATS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// How many stamps this thread has taken, so tests can prove a path does no disk access.
#[cfg(test)]
pub fn stats_taken() -> usize {
    STATS.with(std::cell::Cell::get)
}

pub fn stamp(path: &Path) -> Option<FileStamp> {
    #[cfg(test)]
    STATS.with(|stats| stats.set(stats.get() + 1));
    let metadata = std::fs::metadata(path).ok()?;
    let modified = metadata
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_nanos() as u64;
    Some(FileStamp {
        size: metadata.len(),
        modified,
    })
}

/// Whether a record path is a plain path inside the notebook: only normal components.
fn is_notebook_path(path: &Path) -> bool {
    path.components()
        .all(|component| matches!(component, Component::Normal(_)))
}

pub fn encode(library: &Library) -> String {
    let mut output = format!("version={VERSION}\r\n");
    for note in library
        .notes
        .iter()
        .filter(|note| is_notebook_path(&note.path))
    {
        let flags = match (note.pinned, note.deleted) {
            (true, true) => "pd",
            (true, false) => "p",
            (false, true) => "d",
            (false, false) => "-",
        };
        output.push_str(&format!(
            "note={}|{flags}|{}|{:016x}|{}\r\n",
            note.id.to_hex(),
            note.size,
            note.hash,
            note.path.to_string_lossy()
        ));
    }
    output
}

pub fn parse(source: &str) -> Option<Library> {
    let source = source.strip_prefix('\u{feff}').unwrap_or(source);
    let mut version = None;
    let mut lines = Vec::new();
    for line in source.lines() {
        match line.split_once('=') {
            Some(("version", value)) => version = Some(value),
            Some(("note", value)) => lines.push(value),
            _ => {}
        }
    }
    let parse_line: fn(&str) -> Option<NoteRecord> = match version? {
        VERSION => parse_note,
        VERSION_1 => parse_note_v1,
        _ => return None,
    };
    let mut library = Library {
        notes: lines
            .into_iter()
            .filter_map(parse_line)
            .filter(|note| is_notebook_path(&note.path))
            .collect(),
    };
    dedupe_by(&mut library.notes, |note| note.id.0);
    Some(library)
}

fn dedupe_by<T>(items: &mut Vec<T>, key: impl Fn(&T) -> u128) {
    let mut seen = std::collections::HashSet::new();
    items.retain(|item| seen.insert(key(item)));
}

/// `<id>|<flags>|<size>|<hash>|<path>`.
fn parse_note(value: &str) -> Option<NoteRecord> {
    let mut fields = value.splitn(5, '|');
    let id = NoteId::parse_hex(fields.next()?)?;
    let flags = fields.next()?;
    finish_note(id, flags, fields)
}

/// `<id>|<notebook>|<flags>|<tags>|<size>|<hash>|<path>`: the notebook and tags are dropped.
fn parse_note_v1(value: &str) -> Option<NoteRecord> {
    let mut fields = value.splitn(7, '|');
    let id = NoteId::parse_hex(fields.next()?)?;
    let _notebook = fields.next()?;
    let flags = fields.next()?;
    let _tags = fields.next()?;
    finish_note(id, flags, fields)
}

/// The size, hash and path that end a `note` line in both versions. Only `p` and `d` flags count.
fn finish_note<'a>(
    id: NoteId,
    flags: &str,
    mut fields: impl Iterator<Item = &'a str>,
) -> Option<NoteRecord> {
    let size = fields.next()?.parse().ok()?;
    let hash = u64::from_str_radix(fields.next()?, 16).ok()?;
    let path = fields.next()?;
    if path.is_empty() {
        return None;
    }
    let mut note = NoteRecord::new(id, PathBuf::from(path));
    note.pinned = flags.contains('p');
    note.deleted = flags.contains('d');
    note.size = size;
    note.hash = hash;
    Some(note)
}

pub enum ReadOutcome {
    Absent,
    Loaded(Library, FileStamp),
    /// Read, but damaged or from a newer FastPad: never overwritten.
    Unreadable,
    /// Could not be read right now (a sharing violation while OneDrive syncs it, or it kept
    /// changing during the read). Nothing is known about its contents; try again later.
    Busy,
}

/// How many times a read that raced a write is retried before it is reported as busy.
const READ_ATTEMPTS: usize = 3;

pub fn read(path: &Path) -> ReadOutcome {
    read_via(path, |path| std::fs::read(path))
}

fn read_via(
    path: &Path,
    mut read_bytes: impl FnMut(&Path) -> std::io::Result<Vec<u8>>,
) -> ReadOutcome {
    for _ in 0..READ_ATTEMPTS {
        // A stamp on both sides of the read proves the bytes belong to that stamp.
        let before = stamp(path);
        let bytes = match read_bytes(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return ReadOutcome::Absent;
            }
            Err(_) => return ReadOutcome::Busy,
        };
        let after = stamp(path);
        let Some(stamp) = after.filter(|_| before == after) else {
            continue;
        };
        return match std::str::from_utf8(&bytes).ok().and_then(parse) {
            Some(library) => ReadOutcome::Loaded(library, stamp),
            None => ReadOutcome::Unreadable,
        };
    }
    ReadOutcome::Busy
}

pub fn write(path: &Path, library: &Library) -> Result<FileStamp> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    crate::file::saver::save_atomic(path, encode(library).as_bytes())?;
    stamp(path).ok_or(crate::FastPadError::Invariant(
        "library.ini could not be read back after it was written",
    ))
}
```

`ids::escape` and `ids::unescape` stay (public, tested, and free for later name fields); nothing in the store uses them now.

- [ ] **Step 5: Rewrite the operations**

Replace the whole of `src/library/ops.rs` with:

```rust
//! Every change the UI makes to a library is a `PendingOp`, applied at once to the live library
//! and kept until the next successful write. When `library.ini` changed on disk in between
//! (sync, another instance), the file is re-read and the pending operations are replayed on top,
//! so both sides' changes survive.

use super::ids::NoteId;
use super::model::{Library, LibraryError, NoteRef};
use std::path::PathBuf;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PendingOp {
    SetPinned { note: NoteRef, value: bool },
    Relocate { note: NoteRef, path: PathBuf },
    SetFingerprint { note: NoteRef, size: u64, hash: u64 },
    SetDeleted { note: NoteRef, value: bool },
    Drop { id: NoteId },
}

pub fn apply(library: &mut Library, op: &PendingOp) -> Result<(), LibraryError> {
    match op {
        PendingOp::SetPinned { note, value } => {
            library.resolve_note(note).pinned = *value;
            Ok(())
        }
        PendingOp::Relocate { note, path } => {
            library
                .find_note_mut(note)
                .ok_or(LibraryError::NotFound)?
                .path = path.clone();
            Ok(())
        }
        PendingOp::SetFingerprint { note, size, hash } => {
            let record = library.find_note_mut(note).ok_or(LibraryError::NotFound)?;
            record.size = *size;
            record.hash = *hash;
            Ok(())
        }
        PendingOp::SetDeleted { note, value } => {
            library
                .find_note_mut(note)
                .ok_or(LibraryError::NotFound)?
                .deleted = *value;
            Ok(())
        }
        PendingOp::Drop { id } => {
            library.notes.retain(|note| note.id != *id);
            Ok(())
        }
    }
}

/// Applies every operation in order. Operations that no longer apply are dropped and counted.
pub fn replay(library: &mut Library, ops: &[PendingOp]) -> usize {
    ops.iter().filter(|op| apply(library, op).is_err()).count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::ids::NoteId;

    fn note(id: u128, path: &str) -> NoteRef {
        NoteRef {
            id: NoteId(id),
            path: path.into(),
        }
    }

    #[test]
    fn two_diverged_copies_merge_by_replay_without_losing_either_side() {
        // Break caught: a sync from another PC overwriting this PC's pin, or the reverse.
        let mut other_pc = Library::default();
        apply(
            &mut other_pc,
            &PendingOp::SetPinned {
                note: note(7, "a.md"),
                value: true,
            },
        )
        .unwrap();
        let ours = vec![PendingOp::SetPinned {
            note: note(8, "b.md"),
            value: true,
        }];
        let mut merged = other_pc.clone();
        assert_eq!(replay(&mut merged, &ours), 0);
        assert!(merged.note(NoteId(7)).unwrap().pinned);
        assert!(merged.note(NoteId(8)).unwrap().pinned);
    }

    #[test]
    fn replaying_an_already_applied_log_changes_nothing() {
        let ops = vec![
            PendingOp::SetPinned {
                note: note(7, "a.md"),
                value: true,
            },
            PendingOp::SetFingerprint {
                note: note(7, "a.md"),
                size: 3,
                hash: 9,
            },
            PendingOp::SetDeleted {
                note: note(7, "a.md"),
                value: true,
            },
        ];
        let mut once = Library::default();
        replay(&mut once, &ops);
        let mut twice = once.clone();
        assert_eq!(replay(&mut twice, &ops), 0);
        assert_eq!(once, twice);
    }

    #[test]
    fn operations_whose_target_was_removed_are_dropped() {
        // Break caught: a relocation or fingerprint for a record the other PC purged creating a
        // bare record that points at nothing.
        let mut library = Library::default();
        let ops = vec![
            PendingOp::Relocate {
                note: note(8, "gone.md"),
                path: "moved.md".into(),
            },
            PendingOp::SetFingerprint {
                note: note(8, "gone.md"),
                size: 1,
                hash: 2,
            },
            PendingOp::SetDeleted {
                note: note(8, "gone.md"),
                value: true,
            },
        ];
        assert_eq!(replay(&mut library, &ops), 3);
        assert!(library.notes.is_empty());
    }

    #[test]
    fn relocation_fingerprints_and_deletion_flags_need_an_existing_record() {
        let mut library = Library::default();
        apply(
            &mut library,
            &PendingOp::SetPinned {
                note: note(7, "a.md"),
                value: true,
            },
        )
        .unwrap();
        apply(
            &mut library,
            &PendingOp::Relocate {
                note: note(7, "a.md"),
                path: "b.md".into(),
            },
        )
        .unwrap();
        apply(
            &mut library,
            &PendingOp::SetFingerprint {
                note: note(7, "b.md"),
                size: 3,
                hash: 9,
            },
        )
        .unwrap();
        apply(
            &mut library,
            &PendingOp::SetDeleted {
                note: note(7, "b.md"),
                value: true,
            },
        )
        .unwrap();
        let record = library.note(NoteId(7)).unwrap();
        assert_eq!(record.path, std::path::PathBuf::from("b.md"));
        assert_eq!((record.size, record.hash, record.deleted), (3, 9, true));
        apply(&mut library, &PendingOp::Drop { id: NoteId(7) }).unwrap();
        assert!(library.notes.is_empty());
    }
}
```

In `src/library/reconcile.rs`, the `record` test helper becomes:

```rust
    fn record(id: u128, path: &str, size: u64, hash: u64) -> NoteRecord {
        let mut record = NoteRecord::new(NoteId(id), path.into());
        record.pinned = true;
        record.size = size;
        record.hash = hash;
        record
    }
```

and the first comment of `a_rename_outside_fastpad_is_followed_by_file_id` becomes `// Break caught: renaming a note in Explorer losing its pin.`. The absolute-path filter in `reconcile` and `records_outside_the_folder_are_left_alone` stay: the store no longer produces such records, and the filter keeps a hand-built one harmless.

- [ ] **Step 6: `LibraryState::is_pinned` and the library tests**

In `src/library/mod.rs`, replace the module doc (lines 1–3) with:

```rust
//! The note library: a notebook (a folder) of plain text files seen as notes, plus sparse pins
//! kept in `.fastpad\library.ini` and attached to files by path, file ID and content fingerprint.
//! Nothing here touches a window.
```

Add to `impl LibraryState`, after `record_for`:

```rust
    /// Whether the note at `path` (absolute, or as records store it) is pinned. A note FastPad
    /// sent to the Recycle Bin is not.
    pub fn is_pinned(&self, path: &Path) -> bool {
        self.record_for(path)
            .is_some_and(|record| record.pinned && !record.deleted)
    }
```

In the `mod tests` block of `src/library/mod.rs`:

- Replace `use crate::library::ids::{NotebookId, TagId};` with `use crate::library::ids::NoteId;`.
- Replace `organizing_creates_the_library_file_and_a_reload_sees_it` with:

```rust
    #[test]
    fn pinning_creates_the_library_file_and_a_reload_sees_it() {
        let scratch = Scratch::new("organize");
        let note = scratch.folder().join("a.md");
        std::fs::write(&note, "a").unwrap();
        let mut ids = IdSource::new(1, 2);
        let mut state = load(&scratch.folder(), &scratch.local(), 100).unwrap();
        let target = state.note_ref(&mut ids, &note);
        state
            .apply(PendingOp::SetPinned {
                note: target,
                value: true,
            })
            .unwrap();
        assert!(state.is_pinned(&note));
        assert_eq!(flush(&mut state).unwrap(), Flushed::Wrote);
        assert!(state.pending.is_empty());
        let reloaded = load(&scratch.folder(), &scratch.local(), 101).unwrap();
        assert!(reloaded.is_pinned(&note));
        assert_eq!(
            reloaded.record_for(&note).unwrap().path,
            PathBuf::from("a.md")
        );
    }
```

- Replace `a_file_changed_on_disk_is_merged_not_overwritten` with:

```rust
    #[test]
    fn a_file_changed_on_disk_is_merged_not_overwritten() {
        // Break caught: this PC's debounced write replacing the pin another PC just synced.
        let scratch = Scratch::new("merge");
        let [a, b, c] = ["a.md", "b.md", "c.md"].map(|name| {
            let path = scratch.folder().join(name);
            std::fs::write(&path, name).unwrap();
            path
        });
        let mut ids = IdSource::new(1, 2);
        let mut state = load(&scratch.folder(), &scratch.local(), 100).unwrap();
        let target = state.note_ref(&mut ids, &a);
        state
            .apply(PendingOp::SetPinned {
                note: target,
                value: true,
            })
            .unwrap();
        flush(&mut state).unwrap();

        // "Another PC" pins b.md directly in the file.
        let path = store::library_file(&scratch.folder());
        let mut other = match store::read(&path) {
            store::ReadOutcome::Loaded(library, _) => library,
            _ => panic!("expected a library"),
        };
        other
            .resolve_note(&NoteRef {
                id: NoteId(77),
                path: "b.md".into(),
            })
            .pinned = true;
        std::thread::sleep(std::time::Duration::from_millis(20));
        store::write(&path, &other).unwrap();

        let target = state.note_ref(&mut ids, &c);
        state
            .apply(PendingOp::SetPinned {
                note: target,
                value: true,
            })
            .unwrap();
        flush(&mut state).unwrap();
        let final_state = load(&scratch.folder(), &scratch.local(), 102).unwrap();
        assert!(final_state.is_pinned(&a));
        assert!(final_state.is_pinned(&b), "the synced pin survives");
        assert!(final_state.is_pinned(&c));
    }
```

- In `an_unreadable_library_file_is_never_overwritten`, `flush_refuses_to_overwrite_a_library_file_replaced_by_an_unreadable_one`, `a_rescan_that_cannot_read_the_library_file_keeps_the_live_library_and_is_not_unreadable` and `the_index_follows_saves_renames_and_deletes`, replace every `PendingOp::SetFavorite {` with `PendingOp::SetPinned {`, and in the third replace `assert!(merged.record_for(&a).unwrap().favorite);` with `assert!(merged.is_pinned(&a));`. Its comment's "replacing the live notebooks with an empty library" becomes "replacing the live pins with an empty library".
- Replace `a_rescan_does_not_revert_a_favorite_already_flushed_while_it_ran` with:

```rust
    #[test]
    fn a_rescan_does_not_revert_a_pin_already_flushed_while_it_ran() {
        // Break caught: merge_rescan replaying an empty pending list onto the rescan's own
        // (stale) snapshot of the library and installing the rescan's stamp, silently reverting
        // a change the live library had already flushed to disk before the merge happened.
        let scratch = Scratch::new("rescan-flush");
        let a = scratch.folder().join("a.md");
        std::fs::write(&a, "a").unwrap();
        let mut ids = IdSource::new(1, 2);
        let mut previous = load(&scratch.folder(), &scratch.local(), 100).unwrap();
        let fresh = load(&scratch.folder(), &scratch.local(), 101).unwrap();
        let target = previous.note_ref(&mut ids, &a);
        previous
            .apply(PendingOp::SetPinned {
                note: target,
                value: true,
            })
            .unwrap();
        assert_eq!(flush(&mut previous).unwrap(), Flushed::Wrote);

        let mut merged = merge_rescan(previous, fresh);
        assert!(merged.is_pinned(&a), "the flushed pin is still visible");
        assert!(
            flush(&mut merged).unwrap() == Flushed::Nothing,
            "nothing pending: the flush is a no-op"
        );
        let reloaded = load(&scratch.folder(), &scratch.local(), 102).unwrap();
        assert!(reloaded.is_pinned(&a), "the flush did not revert it");
    }
```

- Replace `a_rescan_absorbs_a_change_made_outside_fastpad_while_it_was_inactive` with:

```rust
    #[test]
    fn a_rescan_absorbs_a_change_made_outside_fastpad_while_it_was_inactive() {
        // Break caught: treating any stamp mismatch as "the live library flushed during the
        // scan" and keeping a stale library forever when the file actually changed from
        // outside (another PC's sync) while this FastPad made no local changes of its own.
        let scratch = Scratch::new("rescan-outside-sync");
        let a = scratch.folder().join("a.md");
        std::fs::write(&a, "a").unwrap();
        let b = scratch.folder().join("b.md");
        std::fs::write(&b, "b").unwrap();
        let mut ids = IdSource::new(1, 2);

        // Pin once, so `library.ini` exists and `previous` loads with a real stamp.
        let mut setup = load(&scratch.folder(), &scratch.local(), 100).unwrap();
        let target = setup.note_ref(&mut ids, &a);
        setup
            .apply(PendingOp::SetPinned {
                note: target,
                value: true,
            })
            .unwrap();
        assert_eq!(flush(&mut setup).unwrap(), Flushed::Wrote);

        let previous = load(&scratch.folder(), &scratch.local(), 101).unwrap();
        assert!(previous.stamp.is_some());

        // Another PC syncs a pin directly into the file while this FastPad is inactive.
        let path = store::library_file(&scratch.folder());
        let mut outside = match store::read(&path) {
            store::ReadOutcome::Loaded(library, _) => library,
            _ => panic!("expected a library"),
        };
        outside
            .resolve_note(&NoteRef {
                id: NoteId(77),
                path: "b.md".into(),
            })
            .pinned = true;
        std::thread::sleep(std::time::Duration::from_millis(20));
        store::write(&path, &outside).unwrap();

        let fresh = load(&scratch.folder(), &scratch.local(), 102).unwrap();
        assert_ne!(fresh.stamp, previous.stamp);
        let current = store::stamp(&path);

        let merged = merge_rescan(previous, fresh);
        assert!(merged.is_pinned(&b), "the outside change is visible");
        assert!(merged.is_pinned(&a), "the earlier pin is still there");
        assert_eq!(merged.stamp, current);
    }
```

- Replace `a_flush_that_meets_a_busy_library_file_keeps_its_operations_for_a_retry` with:

```rust
    #[test]
    fn a_flush_that_meets_a_busy_library_file_keeps_its_operations_for_a_retry() {
        // Break caught: a flush that could not re-read a synced library.ini dropping the pending
        // operations or marking the library unreadable.
        use std::os::windows::fs::OpenOptionsExt;
        let scratch = Scratch::new("busy-flush");
        let a = scratch.folder().join("a.md");
        std::fs::write(&a, "a").unwrap();
        let b = scratch.folder().join("b.md");
        std::fs::write(&b, "b").unwrap();
        let mut ids = IdSource::new(1, 2);
        let mut state = load(&scratch.folder(), &scratch.local(), 100).unwrap();
        let target = state.note_ref(&mut ids, &a);
        state
            .apply(PendingOp::SetPinned {
                note: target,
                value: true,
            })
            .unwrap();
        // Another PC's sync creates the file, so the flush must re-read it, while it is held open.
        let path = store::library_file(&scratch.folder());
        let mut other = Library::default();
        other
            .resolve_note(&NoteRef {
                id: NoteId(77),
                path: "b.md".into(),
            })
            .pinned = true;
        store::write(&path, &other).unwrap();
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&path)
            .unwrap();
        let busy = flush(&mut state).unwrap();
        drop(lock);
        assert_eq!(busy, Flushed::Busy);
        assert_eq!(state.pending.len(), 1);
        assert_eq!(state.metadata, Metadata::Ready);

        assert_eq!(flush(&mut state).unwrap(), Flushed::Wrote);
        let reloaded = load(&scratch.folder(), &scratch.local(), 101).unwrap();
        assert!(reloaded.is_pinned(&b));
        assert!(reloaded.is_pinned(&a));
    }
```

- Add:

```rust
    #[test]
    fn a_version_one_library_loads_its_pins_and_is_rewritten_only_when_something_changes() {
        // Break caught: a PR #9 notebook losing its pins on the first start of this build, or
        // opening it rewriting library.ini before the user changed anything.
        let scratch = Scratch::new("v1-migrate");
        let a = scratch.folder().join("a.md");
        std::fs::write(&a, "a").unwrap();
        let b = scratch.folder().join("b.md");
        std::fs::write(&b, "b").unwrap();
        let path = store::library_file(&scratch.folder());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        // The size and hash match a.md, so the load has no fingerprint to correct.
        let v1 = format!(
            "version=1\r\ntag={}|idea\r\nnote={}|-|fp|-|1|{:016x}|a.md\r\n",
            NoteId(9).to_hex(),
            NoteId(5).to_hex(),
            ids::fnv1a(b"a")
        );
        std::fs::write(&path, &v1).unwrap();

        let mut state = load(&scratch.folder(), &scratch.local(), 100).unwrap();
        assert_eq!(state.metadata, Metadata::Ready);
        assert!(state.is_pinned(&a));
        assert_eq!(flush(&mut state).unwrap(), Flushed::Nothing);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), v1, "reading alone never rewrites");

        let mut ids = IdSource::new(1, 2);
        let target = state.note_ref(&mut ids, &b);
        state
            .apply(PendingOp::SetPinned {
                note: target,
                value: true,
            })
            .unwrap();
        assert_eq!(flush(&mut state).unwrap(), Flushed::Wrote);
        let written = std::fs::read_to_string(&path).unwrap();
        assert!(written.starts_with("version=2\r\n"), "{written:?}");
        assert!(!written.contains("tag="), "{written:?}");
        assert!(written.contains(&format!("note={}|p|1|", NoteId(5).to_hex())), "{written:?}");
        assert!(written.ends_with("|b.md\r\n"), "{written:?}");
    }
```

- [ ] **Step 7: Run the library tests to verify they pass**

Run: `cargo test --lib library:: -- --test-threads=1`
Expected: every test under `library::` passes, including the new `a_version_one_file_keeps_its_pins_and_deleted_flags_and_drops_the_rest`, `only_version_one_and_two_files_are_readable`, `absolute_path_records_are_dropped_on_read_and_never_written`, `prune_keeps_pinned_records_and_drops_the_rest` and `a_version_one_library_loads_its_pins_and_is_rewritten_only_when_something_changes`. (The window crate does not compile yet, so run this after Step 10 if the lib build stops on `window::`.)

- [ ] **Step 8: Commands and the palette**

In `src/window/commands.rs`, the enum's tail from `NoteKeepMine` becomes:

```rust
    NoteReloadFromDisk,
    NoteKeepMine,
    // 163 and 166-173 were the favorite, tag and notebook commands: retired, never reused.
    NoteTogglePin = 164,
    NoteMoveToNotebook = 165,
    NoteRename = 174,
    NoteDelete = 175,
}
```

`needs_document` drops its last six alternatives, so its `matches!` ends with `| Self::ToggleFolderAutosave`. In `try_from`, the `COMMANDS` table shrinks by 9 (the nine retired commands; update its length literal), and its tail after `CommandId::NoteKeepMine,` is:

```rust
            CommandId::NoteTogglePin,
            CommandId::NoteMoveToNotebook,
            CommandId::NoteRename,
            CommandId::NoteDelete,
        ];
```

In `native_command_values_are_stable_and_round_trip`, replace everything from `assert_eq!(CommandId::try_from(163), Ok(CommandId::NoteToggleFavorite));` to the end of the test with:

```rust
        // Break caught: a removed command's number reused, so a stale shortcut or a test
        // posting 163 runs something else.
        for retired in [163_u16, 166, 167, 168, 169, 170, 171, 172, 173] {
            assert!(CommandId::try_from(retired).is_err(), "{retired}");
        }
        assert_eq!(CommandId::try_from(164), Ok(CommandId::NoteTogglePin));
        assert_eq!(CommandId::try_from(165), Ok(CommandId::NoteMoveToNotebook));
        assert_eq!(CommandId::try_from(174), Ok(CommandId::NoteRename));
        assert_eq!(CommandId::try_from(175), Ok(CommandId::NoteDelete));
        assert!(CommandId::NoteTogglePin.needs_document());
        assert!(CommandId::NoteMoveToNotebook.needs_document());
        assert!(CommandId::NoteRename.needs_document());
        assert!(CommandId::NoteDelete.needs_document());
    }
```

In `src/window/command_palette.rs`, the `ENTRIES` table shrinks by 10 (update its length literal), and the rows from `entry("Note: Reload from disk", ...)` to the `Tag: Remove from all notes...` entry become:

```rust
    entry("Note: Reload from disk", CommandId::NoteReloadFromDisk),
    entry("Note: Keep my version", CommandId::NoteKeepMine),
    entry("Note: Toggle pin", CommandId::NoteTogglePin),
    entry("Note: Rename...", CommandId::NoteRename),
    entry("Note: Delete", CommandId::NoteDelete),
```

`PickerKind` becomes:

```rust
/// What a picker is choosing; decides what `library_host::picked` does with the choice.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PickerKind {
    RecentFolder,
}
```

In the palette tests, `every_command_except_tab_positions_and_the_palette_is_listed_once` computes `expected` as:

```rust
            // Moving a note's file has no palette row until it moves the file.
            let expected = usize::from(
                command.tab_index().is_none()
                    && command != CommandId::CommandPalette
                    && command != CommandId::MarkdownPreviewCycle
                    && command != CommandId::NoteMoveToNotebook,
            );
```

and the `picker` helper and its test become:

```rust
    fn picker(create: Option<&'static str>) -> Picker {
        Picker {
            kind: PickerKind::RecentFolder,
            items: vec!["idea".into(), "reference".into(), "todo".into()],
            create,
        }
    }

    #[test]
    fn picker_rows_filter_items_like_commands_and_offer_to_create_a_new_name() {
        // Break caught: typing a new name leaving nothing to press Enter on, or offering to
        // create a name that already exists under another case.
        let with_create = picker(Some("Create"));
        assert_eq!(
            picker_rows(&with_create, ""),
            vec![PickerRow::Item(0), PickerRow::Item(1), PickerRow::Item(2)]
        );
        assert_eq!(
            picker_rows(&with_create, "ref"),
            vec![PickerRow::Item(1), PickerRow::Create("ref".into())]
        );
        assert_eq!(picker_rows(&with_create, "TODO"), vec![PickerRow::Item(2)]);
        assert_eq!(picker_rows(&picker(None), "zzz"), vec![]);
        assert_eq!(
            picker_row_label(&with_create, &PickerRow::Create("urgent".into())),
            "Create \u{201c}urgent\u{201d}"
        );
        assert_eq!(picker_row_label(&with_create, &PickerRow::Item(0)), "idea");
    }
```

In `src/window/name_box.rs`, the module doc's first sentence ends "shown above the editor for a first save and for renaming a note.", the `use crate::library::ids::{NotebookId, TagId};` line goes, and:

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum NamePurpose {
    FirstSave(DocumentId),
    RenameNote(DocumentId),
}

impl NamePurpose {
    /// The tab this purpose acts on: the box closes when that tab goes away.
    pub(crate) fn document(&self) -> Option<DocumentId> {
        match self {
            Self::FirstSave(id) | Self::RenameNote(id) => Some(*id),
        }
    }
}
```

- [ ] **Step 9: `library_host`: pins only**

In `src/window/library_host.rs`:

The module doc becomes:

```rust
//! Window wiring for the note library: the deferred load, rescans, debounced metadata writes,
//! folder commands, first-save naming, autosave, and pins.
```

The imports at the top become:

```rust
use super::main_window::{app_ptr, push_notice, window_identity};
use crate::library::model::LibraryError;
use crate::library::ops::PendingOp;
use crate::library::title;
use crate::library::{self, LibraryState, Metadata, ids::IdSource};
use crate::window::command_palette::{Picker, PickerChoice, PickerKind};
use crate::window::name_box::{NameBox, NamePurpose};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{HWND, LPARAM};
use windows_sys::Win32::UI::WindowsAndMessaging::{KillTimer, PostMessageW, SetTimer};
```

`LibraryHost` loses `shown_targets` and `pending_target` (and `LibraryHost::new` their initializers); `ids` is documented `/// IDs for new note records.`; the `Target` enum is deleted.

In `install`, the unreadable notice becomes `push_notice(hwnd, READ_ONLY.to_owned());`, and `READ_ONLY` becomes:

```rust
const READ_ONLY: &str = "This folder's .fastpad\\library.ini is damaged or from a newer FastPad, so pins are read-only.";
```

In `flush_reporting`, the error notice becomes `format!("FastPad could not save this folder's pins: {error}")`. In `open_folder`, the comment above `let failure` becomes `// Switching would drop the unsaved pins, so a failed write keeps the old folder.`, and the notice becomes `format!("FastPad kept this folder open because it could not save its pins: {error}")`.

`name_box_submit` becomes the function below, and `close_name_box_unless_error` and `submit_new_notebook` are deleted:

```rust
/// Enter or Save in the name box.
pub(crate) fn name_box_submit(hwnd: HWND) {
    let Some((purpose, text)) = name_box_state(hwnd) else {
        return;
    };
    match purpose {
        NamePurpose::FirstSave(id) => submit_first_save(hwnd, id, &text),
        NamePurpose::RenameNote(_) if !ready_library(hwnd) => close_name_box(hwnd),
        NamePurpose::RenameNote(id) => submit_rename(hwnd, id, &text),
    }
}
```

Delete `type Row`, `notebook_rows`, `tag_rows`, `note_tags`, `note_tag_rows`, `move_rows`, `add_tag_rows`, `picker`, `shown_row`, `shown_notebook`, `shown_tag`, `toggle_flag` and `organize`. Add, after `confirmed`:

```rust
/// Pins or unpins `path`. Only a note inside the open notebook can be pinned: version 2 of
/// `library.ini` has no records for files outside it.
pub(crate) fn toggle_pin(hwnd: HWND, path: &Path) {
    if !ready_library(hwnd) {
        return;
    }
    if !folder(hwnd).is_some_and(|folder| library::is_inside(&folder, path)) {
        push_notice(
            hwnd,
            "Only notes in the open notebook can be pinned.".to_owned(),
        );
        return;
    }
    let mut now_on = false;
    let result = apply_op(hwnd, |state, ids| {
        now_on = !state.is_pinned(path);
        Some(PendingOp::SetPinned {
            note: state.note_ref(ids, path),
            value: now_on,
        })
    });
    if result.is_err() {
        report(hwnd, result);
        return;
    }
    push_notice(
        hwnd,
        if now_on { "Pinned." } else { "Unpinned." }.to_owned(),
    );
}
```

`picked` becomes:

```rust
/// A picker row was chosen. The recent-folder picker is the only one; its row opens the folder
/// that row showed, even if `folders.ini` changed meanwhile.
pub(crate) fn picked(hwnd: HWND, kind: PickerKind, choice: PickerChoice) {
    #[cfg(test)]
    LAST_PICK.with(|last| *last.borrow_mut() = Some((kind, choice.clone())));
    let (PickerKind::RecentFolder, PickerChoice::Item(index)) = (kind, choice) else {
        return;
    };
    let shown = host(hwnd, |host| std::mem::take(&mut host.shown_recent_folders));
    if let Some(folder) = shown.unwrap_or_default().get(index) {
        open_folder(hwnd, folder);
    }
}
```

- [ ] **Step 10: Route the commands and rewrite the window tests**

In `execute_command` (`src/window/main_window.rs`), replace the `CommandId::NoteToggleFavorite | ... | CommandId::TagRemoveEverywhere => crate::window::library_host::organize(hwnd, command),` arm with:

```rust
        CommandId::NoteTogglePin => {
            if let Some(path) = crate::window::library_host::active_file(hwnd) {
                crate::window::library_host::toggle_pin(hwnd, &path);
            }
        }
        // No palette row or shortcut reaches this until moving a note's file is wired in.
        CommandId::NoteMoveToNotebook => {}
```

In the `mod tests` block of `src/window/main_window.rs`:

- `a_stale_ready_message_is_dropped_and_writes_are_flushed_on_demand`: `PendingOp::SetFavorite {` becomes `PendingOp::SetPinned {`.
- Delete the line `use crate::window::command_palette::{PickerChoice, PickerKind};` above `fn library(hwnd: HWND)`.
- Delete `duplicate_notebook_names_show_an_inline_error_and_keep_the_box_open`, `deleting_a_notebook_asks_first_and_moves_its_notes_to_notes`, `recoloring_renaming_and_removing_a_tag_act_on_the_picked_rows` and `a_pick_resolves_against_the_rows_the_picker_showed` (`a_recent_folder_pick_opens_the_row_that_was_shown` keeps the shown-rows guarantee).
- Replace `favorite_pin_notebook_and_tags_apply_to_the_active_file_and_persist` with:

```rust
    #[test]
    fn toggling_a_pin_applies_to_the_active_note_and_persists() {
        // Break caught: the pin command changing only memory, recording the wrong file, or an
        // unpin leaving a record behind.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("pin");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        let path = open_note(&window, &scratch, "a.md", "a");
        let hwnd = window.hwnd;

        execute_command(hwnd, CommandId::NoteTogglePin);
        assert!(app_mut(hwnd).library.state.as_ref().unwrap().is_pinned(&path));
        assert!(notices(hwnd).iter().any(|n| n == "Pinned."));
        crate::window::library_host::flush_now(hwnd);
        let ini =
            std::fs::read_to_string(crate::library::store::library_file(&scratch.folder()))
                .unwrap();
        assert!(ini.starts_with("version=2\r\n"), "{ini:?}");
        assert!(ini.contains("|p|") && ini.ends_with("|a.md\r\n"), "{ini:?}");
        let reloaded =
            crate::library::load(&scratch.folder(), &scratch.root.join("x.ini"), 0).unwrap();
        assert!(reloaded.is_pinned(&path));

        execute_command(hwnd, CommandId::NoteTogglePin);
        crate::window::library_host::flush_now(hwnd);
        assert!(notices(hwnd).iter().any(|n| n == "Unpinned."));
        assert!(library(hwnd).notes.is_empty(), "an unpinned note keeps no record");
    }

    #[test]
    fn pinning_a_file_outside_the_open_notebook_is_refused() {
        // Break caught: a pin on a file outside the notebook writing an absolute-path record
        // that version 2 of library.ini cannot hold.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("pin-outside");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        scratch.install(window.hwnd);
        let outside = scratch.root.join("outside.md");
        std::fs::write(&outside, "x").unwrap();
        super::open_path(window.hwnd, &outside).unwrap();
        execute_command(window.hwnd, CommandId::NoteTogglePin);
        assert!(
            notices(window.hwnd)
                .iter()
                .any(|n| n == "Only notes in the open notebook can be pinned.")
        );
        let state = app_mut(window.hwnd).library.state.as_ref().unwrap();
        assert!(state.pending.is_empty());
        assert!(state.library.notes.is_empty());
    }
```

- `an_untitled_tab_must_be_saved_before_it_can_be_organized`: `CommandId::NoteToggleFavorite` becomes `CommandId::NoteTogglePin` (the notice stays "Save this note first to organize it.", which rename and delete share).
- `an_unreadable_library_disables_organizing_with_an_explanation` is renamed `an_unreadable_library_disables_pinning_with_an_explanation`, and its `CommandId::NoteToggleFavorite` becomes `CommandId::NoteTogglePin`.
- `renaming_a_note_renames_its_file_and_keeps_its_metadata`: the first comment becomes `// Break caught: a rename losing the note's pin, or the tab still pointing at the old path.`, `CommandId::NoteToggleFavorite` becomes `CommandId::NoteTogglePin`, and the last line becomes `assert!(state.is_pinned(&new));`.
- `deleting_a_note_asks_then_recycles_it_and_keeps_its_record_hidden`, `a_note_moved_and_changed_outside_fastpad_follows_but_does_not_autosave_over_the_change` and `a_metadata_flush_leaves_the_local_file_alone_and_a_recent_change_writes_it`: `CommandId::NoteToggleFavorite` becomes `CommandId::NoteTogglePin`.
- `a_note_renamed_outside_fastpad_moves_its_open_tab_and_autosave_resumes`: `CommandId::NoteToggleFavorite` becomes `CommandId::NoteTogglePin`, and the last line becomes `assert!(state.record_for(&new).unwrap().pinned);`.
- `a_library_file_held_open_by_a_sync_keeps_the_operations_and_retries_without_a_notice`: `CommandId::NoteToggleFavorite` becomes `CommandId::NoteTogglePin`, the last line becomes `assert!(reloaded.is_pinned(&a));`, and its comment's "the pending notebooks and tags" becomes "the pending pins".
- The comments in `a_failed_flush_keeps_the_current_folder_open` and `turning_notes_mode_off_says_so_when_metadata_cannot_be_written_and_closes_the_name_box` say "pins" where they say "notebooks and tags".

In `tests/windows/library.rs`:

- Both `command(hwnd, CommandId::NoteToggleFavorite);` lines become `command(hwnd, CommandId::NoteTogglePin);`.
- In the Explorer-rename test, `read(&data.library_ini()).contains("|f|-|")` becomes `read(&data.library_ini()).contains("|p|")`, and the last assertion becomes:

```rust
    assert!(
        renamed.contains("|p|"),
        "the pin must follow the rename: {renamed:?}"
    );
```

- The comment at "user's notebooks and tags" says "user's pins".

In `src/bin/fastpad-bench.rs` (`create_library_fixture`), `PendingOp::SetFavorite {` becomes `PendingOp::SetPinned {`, and its doc says "with pinned records spread across them".

- [ ] **Step 11: Compile, format and run the targeted tests**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: no warnings or errors.

Run: `cargo fmt --all -- --check`
Expected: no diff.

Run: `cargo test --lib -- --test-threads=1 library:: window::commands window::command_palette`
Expected: all pass.

Run: `cargo test --lib -- --test-threads=1 toggling_a_pin pinning_a_file_outside an_untitled_tab_must_be_saved an_unreadable_library_disables_pinning renaming_a_note_renames deleting_a_note_asks a_note_renamed_outside a_library_file_held_open a_stale_ready_message a_recent_folder_pick`
Expected: all 10 pass.

- [ ] **Step 12: Commit**

```bash
git add src/library src/window/commands.rs src/window/command_palette.rs src/window/name_box.rs src/window/library_host.rs src/window/main_window.rs tests/windows/library.rs src/bin/fastpad-bench.rs
git commit -m "feat(library): pins only, library.ini version 2 with version 1 migration"
```

---

### Task 2: Local files: expanded folders, favorite notebooks, `open=none`

**Files:**
- Modify: `src/library/local.rs` (full rewrite)
- Modify: `src/library/mod.rs` (`merge_rescan`, `rename_note`, doc comments, tests), `src/library/reconcile.rs` (no recent-list rename)
- Modify: `src/window/library_host.rs` (`Startup`, `resolve_startup`, `open_library_step`, `spawn_load`, `Loaded`, `library_ready`, `document_loaded`, doc comments; a new `mod tests`)
- Modify: `src/window/main_window.rs` (tests), `tests/windows/library.rs` and `src/bin/fastpad-bench.rs` (`RecentFolders` literals)

**Interfaces:**
- Consumes: Task 1's pins-only `LibraryState`.
- Produces:
  - `LocalState { folder, autosave, expanded: Vec<PathBuf>, missing, files }`, with `set_expanded(&mut self, path: &Path, expanded: bool)` and `is_expanded(&self, path: &Path) -> bool`; `recent`, `note_opened`, `rename_path`, `merge_recent` and `RECENT_LIMIT` are deleted
  - `Conveniences { autosave: bool, expanded: Vec<PathBuf>, missing: Vec<(u64, NoteId)> }`
  - `RecentFolders { folders: Vec<PathBuf>, favorites: Vec<PathBuf>, closed: bool }`, with `toggle_favorite(&mut self, folder: &Path) -> bool`, `is_favorite(&self, folder: &Path) -> bool`, `set_closed(&mut self, closed: bool)`; `push` clears `closed`
  - `pub const FAVORITE_LIMIT: usize = 50`
  - `local::display_names(folders: &[PathBuf]) -> Vec<(String, Option<String>)>`
  - `library_host::resolve_startup` opens nothing when `closed` is set and the command line named no folder

- [ ] **Step 1: Write the failing tests**

Replace the `mod tests` block of `src/library/local.rs` with:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> LocalState {
        let mut state = LocalState::new(PathBuf::from(r"D:\Notes"));
        state.autosave = false;
        state.expanded = vec![PathBuf::from("sub"), PathBuf::from(r"sub\a|x")];
        state.missing = vec![(5, NoteId(9))];
        state.files = vec![CachedFile {
            volume: 0x1234,
            file_id: 77,
            mtime: 133_000,
            size: 42,
            path: PathBuf::from(r"sub\a|x\n.md"),
        }];
        state
    }

    #[test]
    fn local_state_round_trips_and_rejects_another_folders_file() {
        // Break caught: two folders whose keys collide sharing one cache, or `|` in a path
        // breaking the scan cache or the expanded folders.
        let text = sample().encode();
        assert!(text.contains("expanded=sub\\a|x\r\n"), "{text:?}");
        assert_eq!(
            LocalState::parse(&text, Path::new(r"D:\Notes")),
            Some(sample())
        );
        assert!(
            LocalState::parse(&text, Path::new(r"d:\notes")).is_some(),
            "case is ignored"
        );
        assert_eq!(LocalState::parse(&text, Path::new(r"D:\Other")), None);
        assert_eq!(
            LocalState::parse("version=2\r\n", Path::new(r"D:\Notes")),
            None
        );
    }

    #[test]
    fn an_older_local_file_with_recent_notes_still_loads_and_drops_them() {
        // Break caught: a PR #9 build's local file (with `recent=` lines) failing to parse, which
        // throws away the scan cache and the folder's autosave switch.
        let older = "version=1\r\nfolder=D:\\Notes\r\nautosave=false\r\nrecent=20|b.md\r\n\
                     missing=5|00000000000000000000000000000009\r\n";
        let state = LocalState::parse(older, Path::new(r"D:\Notes")).unwrap();
        assert!(!state.autosave);
        assert_eq!(state.missing, [(5, NoteId(9))]);
        assert!(!state.encode().contains("recent="));
    }

    #[test]
    fn expanded_folders_are_kept_once_ignoring_case_and_collapse_away() {
        // Break caught: expanding a folder twice writing two lines, or collapsing `Sub` leaving
        // `sub` expanded.
        let mut state = LocalState::new(PathBuf::from(r"D:\Notes"));
        state.set_expanded(Path::new(r"Sub\Deep"), true);
        state.set_expanded(Path::new(r"sub\deep"), true);
        assert_eq!(state.expanded, [PathBuf::from(r"Sub\Deep")]);
        assert!(state.is_expanded(Path::new(r"SUB\DEEP")));
        assert!(!state.is_expanded(Path::new("Sub")));
        let before = state.conveniences();
        state.set_expanded(Path::new(r"sub\deep"), true);
        assert_eq!(state.conveniences(), before, "no change, no rewrite");
        state.set_expanded(Path::new(r"SUB\deep"), false);
        assert!(state.expanded.is_empty());
    }

    #[test]
    fn missing_times_are_recorded_once_and_cleared() {
        let mut state = LocalState::new(PathBuf::from(r"D:\Notes"));
        state.set_missing(NoteId(1), 100);
        state.set_missing(NoteId(1), 200);
        assert_eq!(state.missing_since(NoteId(1)), Some(100));
        state.clear_missing(NoteId(1));
        assert_eq!(state.missing_since(NoteId(1)), None);
    }

    #[test]
    fn folder_keys_ignore_case_and_name_the_local_file() {
        let data = Path::new(r"C:\Users\u\AppData\Local\FastPad");
        assert_eq!(
            folder_key(Path::new(r"D:\Notes")),
            folder_key(Path::new(r"d:\NOTES"))
        );
        assert_eq!(folder_key(Path::new(r"D:\Notes")).len(), 16);
        assert_eq!(
            local_file(data, Path::new(r"D:\Notes")),
            data.join("libraries")
                .join(format!("{}.ini", folder_key(Path::new(r"D:\Notes"))))
        );
    }

    #[test]
    fn recent_folders_dedupe_ignoring_case_put_the_newest_first_and_cap_at_ten() {
        let mut folders = RecentFolders::default();
        for index in 0..12 {
            folders.push(PathBuf::from(format!(r"D:\F{index}")));
        }
        folders.push(PathBuf::from(r"d:\f5"));
        assert_eq!(folders.folders.len(), FOLDER_LIMIT);
        assert_eq!(folders.folders[0], PathBuf::from(r"d:\f5"));
        assert_eq!(RecentFolders::parse(&folders.encode()), folders);
        assert_eq!(
            RecentFolders::parse("version=7\r\nfolder=D:\\x\r\n"),
            RecentFolders::default()
        );
    }

    #[test]
    fn folders_ini_round_trips_recent_favorites_and_open_none() {
        // Break caught: favorites or a closed notebook lost on restart, or a PR #9 build's
        // folders.ini (no favorite= or open= lines) failing to load.
        let mut folders = RecentFolders::default();
        folders.push(PathBuf::from(r"D:\Work"));
        assert!(folders.toggle_favorite(Path::new(r"E:\Recipes")));
        folders.set_closed(true);
        let text = folders.encode();
        assert_eq!(
            text,
            "version=1\r\nopen=none\r\nfolder=D:\\Work\r\nfavorite=E:\\Recipes\r\n"
        );
        assert_eq!(RecentFolders::parse(&text), folders);
        let older = RecentFolders::parse("version=1\r\nfolder=D:\\Work\r\nfuture=x\r\n");
        assert_eq!(older.folders, [PathBuf::from(r"D:\Work")]);
        assert!(older.favorites.is_empty() && !older.closed);
        assert!(!RecentFolders::parse("version=1\r\nopen=D:\\Work\r\n").closed);
    }

    #[test]
    fn opening_a_notebook_clears_open_none_and_favoriting_leaves_the_recent_list_alone() {
        // Break caught: a favorite star reordering the recent list, or a notebook opened after
        // a close still reading as closed at the next start.
        let mut folders = RecentFolders::default();
        folders.set_closed(true);
        assert!(folders.toggle_favorite(Path::new(r"D:\Notes")));
        assert!(folders.folders.is_empty(), "favoriting is not opening");
        assert!(folders.closed);
        folders.push(PathBuf::from(r"D:\Notes"));
        assert!(!folders.closed);
    }

    #[test]
    fn favorites_toggle_ignoring_case_and_spelling_and_cap_at_fifty() {
        let mut folders = RecentFolders::default();
        assert!(folders.toggle_favorite(Path::new(r"D:\Notes\")));
        assert_eq!(folders.favorites, [PathBuf::from(r"D:\Notes")]);
        assert!(folders.is_favorite(Path::new(r"d:\notes")));
        assert!(
            !folders.toggle_favorite(Path::new(r"d:\NOTES")),
            "a second toggle removes it"
        );
        assert!(folders.favorites.is_empty());
        for index in 0..FAVORITE_LIMIT + 3 {
            folders.toggle_favorite(&PathBuf::from(format!(r"D:\F{index}")));
        }
        assert_eq!(folders.favorites.len(), FAVORITE_LIMIT);
        assert!(!folders.is_favorite(Path::new(r"D:\F50")), "the 51st is refused");
        let many: String = (0..60).map(|i| format!("favorite=D:\\F{i}\r\n")).collect();
        let parsed = RecentFolders::parse(&format!("version=1\r\n{many}"));
        assert_eq!(parsed.favorites.len(), FAVORITE_LIMIT);
    }

    #[test]
    fn display_names_are_folder_names_with_a_parent_hint_only_on_a_clash() {
        // Break caught: two favorites both reading "Notes" with no way to tell them apart, or
        // every row carrying a path it does not need.
        let folders = [
            r"D:\Work\Notes",
            r"E:\Home\notes",
            r"D:\Recipes",
            r"D:\A\Plans",
            r"E:\A\Plans",
            r"D:\",
        ]
        .map(PathBuf::from);
        assert_eq!(
            display_names(&folders),
            [
                ("Notes".to_owned(), Some("Work".to_owned())),
                ("notes".to_owned(), Some("Home".to_owned())),
                ("Recipes".to_owned(), None),
                ("Plans".to_owned(), Some(r"D:\A".to_owned())),
                ("Plans".to_owned(), Some(r"E:\A".to_owned())),
                (r"D:\".to_owned(), None),
            ]
        );
    }

    #[test]
    fn missing_or_damaged_local_files_read_as_fresh_state() {
        let dir = std::env::temp_dir().join(format!("fastpad-local-io-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let folder = Path::new(r"D:\Notes");
        let path = local_file(&dir, folder);
        assert_eq!(read(&path, folder), LocalState::new(folder.to_path_buf()));
        write(&path, &sample()).unwrap();
        assert_eq!(read(&path, folder), sample());
        std::fs::write(&path, [0xff, 0x00]).unwrap();
        assert_eq!(read(&path, folder), LocalState::new(folder.to_path_buf()));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib library::local -- --test-threads=1`
Expected: a compile error (`expanded`, `set_expanded`, `favorites`, `toggle_favorite`, `display_names` and `FAVORITE_LIMIT` do not exist).

- [ ] **Step 3: Rewrite `local.rs`**

Replace everything in `src/library/local.rs` above `#[cfg(test)] mod tests` with:

```rust
//! Per-PC state that must not travel with the notebook: the scan cache (file IDs only mean
//! something on one volume), which subfolders are expanded in the sidebar, when records went
//! missing, the notebook's autosave switch, and `folders.ini` (recent and favorite notebooks).
//! Losing any of it is harmless, so reads never fail.

use super::ids::{NoteId, fnv1a};
use super::model::same_path;
use crate::Result;
use std::path::{Path, PathBuf};

const VERSION: &str = "1";
pub const FOLDER_LIMIT: usize = 10;
pub const FAVORITE_LIMIT: usize = 50;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CachedFile {
    pub volume: u32,
    pub file_id: u64,
    pub mtime: u64,
    pub size: u64,
    pub path: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalState {
    pub folder: PathBuf,
    pub autosave: bool,
    /// Subfolders expanded in the Notebook view, relative to the notebook. The root is always
    /// expanded and never listed.
    pub expanded: Vec<PathBuf>,
    pub missing: Vec<(u64, NoteId)>,
    pub files: Vec<CachedFile>,
}

/// Everything in the local file except the scan cache: what the UI thread changes.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Conveniences {
    pub autosave: bool,
    pub expanded: Vec<PathBuf>,
    pub missing: Vec<(u64, NoteId)>,
}

impl LocalState {
    pub fn conveniences(&self) -> Conveniences {
        Conveniences {
            autosave: self.autosave,
            expanded: self.expanded.clone(),
            missing: self.missing.clone(),
        }
    }

    pub fn new(folder: PathBuf) -> Self {
        Self {
            folder,
            autosave: true,
            expanded: Vec::new(),
            missing: Vec::new(),
            files: Vec::new(),
        }
    }

    pub fn encode(&self) -> String {
        let mut output = format!(
            "version={VERSION}\r\nfolder={}\r\nautosave={}\r\n",
            self.folder.to_string_lossy(),
            self.autosave
        );
        for path in &self.expanded {
            output.push_str(&format!("expanded={}\r\n", path.to_string_lossy()));
        }
        for (time, id) in &self.missing {
            output.push_str(&format!("missing={time}|{}\r\n", id.to_hex()));
        }
        for file in &self.files {
            output.push_str(&format!(
                "file={}|{}|{}|{}|{}\r\n",
                file.volume,
                file.file_id,
                file.mtime,
                file.size,
                file.path.to_string_lossy()
            ));
        }
        output
    }

    /// `None` for anything but a version-1 file written for `folder`. Unknown lines (the
    /// `recent=` lines older builds wrote) are ignored.
    pub fn parse(source: &str, folder: &Path) -> Option<Self> {
        let source = source.strip_prefix('\u{feff}').unwrap_or(source);
        let mut version = None;
        let mut stored_folder = None;
        let mut state = Self::new(folder.to_path_buf());
        for line in source.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            match key {
                "version" => version = Some(value),
                "folder" => stored_folder = Some(PathBuf::from(value)),
                "autosave" => state.autosave = value != "false",
                "expanded" if !value.is_empty() => state.expanded.push(PathBuf::from(value)),
                "missing" => {
                    if let Some((time, id)) = value.split_once('|')
                        && let (Ok(time), Some(id)) = (time.parse(), NoteId::parse_hex(id))
                    {
                        state.missing.push((time, id));
                    }
                }
                "file" => state.files.extend(parse_file(value)),
                _ => {}
            }
        }
        let same_folder = stored_folder.is_some_and(|stored| {
            same_path(
                &super::normalize_folder(&stored),
                &super::normalize_folder(folder),
            )
        });
        if version != Some(VERSION) || !same_folder {
            return None;
        }
        state.folder = folder.to_path_buf();
        Some(state)
    }

    pub fn is_expanded(&self, path: &Path) -> bool {
        self.expanded.iter().any(|existing| same_path(existing, path))
    }

    /// Records a subfolder as expanded or collapsed. Setting what is already set changes
    /// nothing, so it causes no write.
    pub fn set_expanded(&mut self, path: &Path, expanded: bool) {
        let present = self.is_expanded(path);
        if expanded && !present {
            self.expanded.push(path.to_path_buf());
        } else if !expanded && present {
            self.expanded.retain(|existing| !same_path(existing, path));
        }
    }

    pub fn missing_since(&self, id: NoteId) -> Option<u64> {
        self.missing
            .iter()
            .find(|(_, missing)| *missing == id)
            .map(|(time, _)| *time)
    }

    pub fn set_missing(&mut self, id: NoteId, now: u64) {
        if self.missing_since(id).is_none() {
            self.missing.push((now, id));
        }
    }

    pub fn clear_missing(&mut self, id: NoteId) {
        self.missing.retain(|(_, missing)| *missing != id);
    }
}

fn parse_file(value: &str) -> Option<CachedFile> {
    let mut fields = value.splitn(5, '|');
    let volume = fields.next()?.parse().ok()?;
    let file_id = fields.next()?.parse().ok()?;
    let mtime = fields.next()?.parse().ok()?;
    let size = fields.next()?.parse().ok()?;
    let path = fields.next()?;
    if path.is_empty() {
        return None;
    }
    Some(CachedFile {
        volume,
        file_id,
        mtime,
        size,
        path: PathBuf::from(path),
    })
}

/// The same key for every spelling of one folder (letter case, a trailing separator).
pub fn folder_key(folder: &Path) -> String {
    let folder = super::normalize_folder(folder);
    format!(
        "{:016x}",
        fnv1a(folder.to_string_lossy().to_lowercase().as_bytes())
    )
}

pub fn local_file(data_dir: &Path, folder: &Path) -> PathBuf {
    data_dir
        .join("libraries")
        .join(format!("{}.ini", folder_key(folder)))
}

pub fn read(path: &Path, folder: &Path) -> LocalState {
    read_with_source(path, folder).0
}

/// The state, and the file's text when it could be read, so a writer can skip an unchanged file.
pub fn read_with_source(path: &Path, folder: &Path) -> (LocalState, Option<String>) {
    let source = std::fs::read_to_string(path).ok();
    let state = source
        .as_deref()
        .and_then(|source| LocalState::parse(source, folder))
        .unwrap_or_else(|| LocalState::new(folder.to_path_buf()));
    (state, source)
}

pub fn write(path: &Path, state: &LocalState) -> Result<()> {
    write_text(path, &state.encode())
}

fn write_text(path: &Path, text: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    crate::file::saver::save_atomic(path, text.as_bytes())
}

static NEXT_WRITE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
/// Per local file, the newest write that landed. Writes come from the scan worker and from
/// one-off writer threads, so an older one finishing last must not win.
static LANDED: std::sync::Mutex<Vec<(PathBuf, u64)>> = std::sync::Mutex::new(Vec::new());

/// A number for a write about to be handed off: a later number means newer content.
pub fn next_write() -> u64 {
    NEXT_WRITE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// Writes `text` unless a write numbered after `order` already landed for `path`. Writes to one
/// path are serialized.
pub fn write_text_in_order(path: &Path, text: &str, order: u64) -> Result<()> {
    let mut landed = LANDED.lock().unwrap_or_else(|error| error.into_inner());
    let index = match landed.iter().position(|(p, _)| same_path(p, path)) {
        Some(index) => index,
        None => {
            landed.push((path.to_path_buf(), 0));
            landed.len() - 1
        }
    };
    if landed[index].1 > order {
        return Ok(());
    }
    write_text(path, text)?;
    landed[index].1 = order;
    Ok(())
}

pub fn write_in_order(path: &Path, state: &LocalState, order: u64) -> Result<()> {
    write_text_in_order(path, &state.encode(), order)
}

/// `folders.ini`: recent notebooks (most recent first; the first opens at startup), favorite
/// notebooks, and whether the last session ended with no notebook open. Unknown keys are ignored,
/// so an older build reading this file loses nothing it understands.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RecentFolders {
    pub folders: Vec<PathBuf>,
    /// In the order they were added; the sidebar sorts them by name.
    pub favorites: Vec<PathBuf>,
    /// `open=none`: the last session closed its notebook, so startup opens none.
    pub closed: bool,
}

impl RecentFolders {
    pub fn parse(source: &str) -> Self {
        let source = source.strip_prefix('\u{feff}').unwrap_or(source);
        let mut version = None;
        let mut parsed = Self::default();
        for line in source.lines() {
            match line.split_once('=') {
                Some(("version", value)) => version = Some(value),
                Some(("open", "none")) => parsed.closed = true,
                Some(("folder", value)) if !value.is_empty() => {
                    parsed.folders.push(PathBuf::from(value));
                }
                Some(("favorite", value)) if !value.is_empty() => {
                    parsed.favorites.push(PathBuf::from(value));
                }
                _ => {}
            }
        }
        if version != Some(VERSION) {
            return Self::default();
        }
        parsed.folders.truncate(FOLDER_LIMIT);
        parsed.favorites.truncate(FAVORITE_LIMIT);
        parsed
    }

    pub fn encode(&self) -> String {
        let mut output = format!("version={VERSION}\r\n");
        if self.closed {
            output.push_str("open=none\r\n");
        }
        for folder in &self.folders {
            output.push_str(&format!("folder={}\r\n", folder.to_string_lossy()));
        }
        for favorite in &self.favorites {
            output.push_str(&format!("favorite={}\r\n", favorite.to_string_lossy()));
        }
        output
    }

    /// A notebook was opened: it moves to the front of the recent list, and the next start
    /// opens it.
    pub fn push(&mut self, folder: PathBuf) {
        let folder = super::normalize_folder(&folder);
        self.folders
            .retain(|existing| !same_path(&super::normalize_folder(existing), &folder));
        self.folders.insert(0, folder);
        self.folders.truncate(FOLDER_LIMIT);
        self.closed = false;
    }

    /// Adds `folder` to the favorites, or removes it when it is one, and says whether it is a
    /// favorite now. At `FAVORITE_LIMIT`, a new favorite is refused and this returns false. The
    /// recent list is left alone.
    pub fn toggle_favorite(&mut self, folder: &Path) -> bool {
        let folder = super::normalize_folder(folder);
        let before = self.favorites.len();
        self.favorites
            .retain(|existing| !same_path(&super::normalize_folder(existing), &folder));
        if self.favorites.len() != before || self.favorites.len() >= FAVORITE_LIMIT {
            return false;
        }
        self.favorites.push(folder);
        true
    }

    pub fn is_favorite(&self, folder: &Path) -> bool {
        let folder = super::normalize_folder(folder);
        self.favorites
            .iter()
            .any(|existing| same_path(&super::normalize_folder(existing), &folder))
    }

    pub fn set_closed(&mut self, closed: bool) {
        self.closed = closed;
    }
}

/// What the sidebar calls each notebook: its folder's name, plus a dim hint only when another
/// entry has the same name (ignoring case). The hint is the parent folder's name, or the parent's
/// whole path when those clash too. Touches no disk.
pub fn display_names(folders: &[PathBuf]) -> Vec<(String, Option<String>)> {
    fn name(folder: &Path) -> String {
        folder.file_name().map_or_else(
            || folder.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        )
    }
    let names: Vec<String> = folders.iter().map(|folder| name(folder)).collect();
    let keys: Vec<String> = names.iter().map(|name| name.to_lowercase()).collect();
    let parent_keys: Vec<Option<String>> = folders
        .iter()
        .map(|folder| folder.parent().map(|parent| name(parent).to_lowercase()))
        .collect();
    folders
        .iter()
        .enumerate()
        .map(|(index, folder)| {
            let clashes: Vec<usize> = (0..folders.len())
                .filter(|&other| other != index && keys[other] == keys[index])
                .collect();
            if clashes.is_empty() {
                return (names[index].clone(), None);
            }
            let parent_clashes = clashes
                .iter()
                .any(|&other| parent_keys[other] == parent_keys[index]);
            let hint = folder.parent().map(|parent| {
                if parent_clashes {
                    parent.display().to_string()
                } else {
                    name(parent)
                }
            });
            (names[index].clone(), hint)
        })
        .collect()
}

pub fn folders_file(data_dir: &Path) -> PathBuf {
    data_dir.join("folders.ini")
}

pub fn read_folders(path: &Path) -> RecentFolders {
    std::fs::read_to_string(path)
        .map(|source| RecentFolders::parse(&source))
        .unwrap_or_default()
}

pub fn write_folders(path: &Path, folders: &RecentFolders) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    crate::file::saver::save_atomic(path, folders.encode().as_bytes())
}
```

- [ ] **Step 4: Follow in the library state and reconciliation**

In `src/library/mod.rs`:

- The doc on `LibraryState.written_local` becomes:

```rust
    /// The expanded folders, autosave switch and missing times as the local file last written
    /// holds them, so the UI thread rewrites that file only when one of them changed.
    pub written_local: local::Conveniences,
```

- The doc on `take_local_changes` begins `/// The per-PC state to write, when its expanded folders, autosave switch or missing times changed`.
- In `merge_rescan`, replace `fresh.local.merge_recent(&previous_local);` and the `fresh.local.autosave = previous_local.autosave;` line after it with:

```rust
    // The UI thread owns these conveniences: what it set while the rescan ran wins.
    fresh.local.autosave = previous_local.autosave;
    fresh.local.expanded = previous_local.expanded;
```

- `rename_note` becomes:

```rust
    /// Follows a rename FastPad made: the index and any record.
    pub fn rename_note(&mut self, old: &Path, new: &Path) {
        let old_stored = record_path(&self.folder, old);
        let new_stored = record_path(&self.folder, new);
        self.remove_note(old);
        self.add_note(new);
        if let Some(record) = self.library.note_by_path(&old_stored) {
            let note = NoteRef {
                id: record.id,
                path: old_stored,
            };
            let _ = self.apply(PendingOp::Relocate {
                note,
                path: new_stored,
            });
        }
    }
```

In `src/library/reconcile.rs`, delete the loop

```rust
    for (old, new) in &result.relocated {
        local.rename_path(old, new);
    }
```

In the `mod tests` block of `src/library/mod.rs`:

- `a_rescan_keeps_changes_made_while_it_ran`: replace `previous.local.note_opened(Path::new("a.md"), 150);` with `previous.local.set_expanded(Path::new("sub"), true);`, and `assert_eq!(merged.local.recent[0].0, 150);` with `assert!(merged.local.is_expanded(Path::new("SUB")));`.
- `the_load_writes_the_local_file_and_only_convenience_changes_need_another_write`: replace its last four lines, from `state.local.note_opened(Path::new("a.md"), 150);`, with:

```rust
        state.local.set_expanded(Path::new("sub"), true);
        let changed = take_local_changes(&mut state).expect("an expansion change is written");
        assert_eq!(changed.expanded, [PathBuf::from("sub")]);
        assert!(take_local_changes(&mut state).is_none());
```

- [ ] **Step 5: Startup honors `open=none`; opening a note writes nothing local**

In `src/window/library_host.rs`:

`Startup` becomes:

```rust
/// The startup candidates, checked on the worker so an offline drive cannot stall the UI thread.
struct Startup {
    /// A path named on the command line, which may be a file.
    launch: Option<PathBuf>,
    /// The most recent folder from `folders.ini`, unless the last session closed its notebook.
    remembered: Option<PathBuf>,
    /// `open=none`: the last session ended with no notebook open.
    closed: bool,
    data: PathBuf,
}
```

`Loaded` gains a field after `notice`:

```rust
    /// The last session closed its notebook and the command line named no folder: nothing was
    /// opened, on purpose.
    closed: bool,
```

and `test_ready_payload` sets `closed: false`.

`resolve_startup` becomes:

```rust
/// On the worker: the folder to open at startup, a directory named on the command line, else
/// nothing when the last session closed its notebook, else the most recent folder if it still
/// exists, else `Documents\FastPad`; and a notice when the most recent folder is gone.
fn resolve_startup(startup: Startup) -> (Option<PathBuf>, Option<String>) {
    if let Some(launch) = startup.launch
        && library::folder_exists(&launch)
    {
        remember_folder(&startup.data, &launch);
        return (Some(launch), None);
    }
    if startup.closed {
        return (None, None);
    }
    let mut notice = None;
    if let Some(remembered) = startup.remembered {
        if library::folder_exists(&remembered) {
            return (Some(remembered), None);
        }
        notice = Some(format!(
            "FastPad could not find the folder {}. Using Documents\\FastPad instead.",
            remembered.display()
        ));
    }
    let fallback = crate::platform::paths::default_notes_folder()
        .ok()
        .map(|folder| library::normalize_folder(&folder));
    (fallback, notice)
}
```

`open_library_step` becomes:

```rust
/// `WM_FASTPAD_OPEN_LIBRARY`: starts the worker on the startup folder. Reads only `folders.ini`:
/// whether the folder (or a command-line path) exists is checked on the worker, which may open a
/// different folder than the one assumed here. After a session that closed its notebook, with no
/// path on the command line, nothing opens and no worker starts.
pub(crate) fn open_library_step(hwnd: HWND) {
    if !notes_mode(hwnd) {
        return;
    }
    let Some(data) = data_dir(hwnd) else {
        return;
    };
    let launch =
        unsafe { app_ptr(hwnd) }.and_then(|app| match &unsafe { app.as_ref() }.launch.request {
            crate::launch::LaunchRequest::Open(path) => {
                Some(library::normalize_folder(Path::new(path)))
            }
            crate::launch::LaunchRequest::New => None,
        });
    let recent = library::local::read_folders(&library::local::folders_file(&data));
    if recent.closed && launch.is_none() {
        host(hwnd, |host| host.folder = None);
        return;
    }
    let remembered = if recent.closed {
        None
    } else {
        recent
            .folders
            .first()
            .map(|folder| library::normalize_folder(folder))
    };
    // Until the worker has checked, the name box saves into the folder that usually wins.
    let assumed = if recent.closed {
        None
    } else {
        remembered.clone().or_else(|| {
            crate::platform::paths::default_notes_folder()
                .ok()
                .map(|folder| library::normalize_folder(&folder))
        })
    };
    host(hwnd, |host| host.folder = assumed);
    spawn_load(
        hwnd,
        Some(Startup {
            launch,
            remembered,
            closed: recent.closed,
            data,
        }),
    );
}
```

In `spawn_load`, the worker closure becomes:

```rust
    std::thread::spawn(move || {
        let closed = startup.as_ref().is_some_and(|startup| startup.closed);
        let (folder, notice) = match startup {
            Some(startup) => resolve_startup(startup),
            None => (folder, None),
        };
        let opens_nothing = closed && folder.is_none();
        let (folder, result) = match folder {
            Some(folder) => {
                let local_path = library::local::local_file(&data, &folder);
                let result = library::load(&folder, &local_path, library::now_unix())
                    .map_err(|error| error.to_string());
                (folder, result)
            }
            None => (
                PathBuf::new(),
                Err("the Documents folder could not be found".to_owned()),
            ),
        };
        let payload = Box::into_raw(Box::new(Loaded {
            generation,
            folder,
            result,
            notice,
            closed: opens_nothing,
        }));
        if unsafe {
            PostMessageW(
                target as HWND,
                crate::window::WM_FASTPAD_LIBRARY_READY,
                0,
                payload as isize,
            )
        } == 0
        {
            drop(unsafe { Box::from_raw(payload) });
        }
    });
```

In `library_ready`, the destructuring and what follows it up to `// At startup the worker decides...` become:

```rust
    let Loaded {
        folder,
        result,
        notice,
        closed,
        ..
    } = *loaded;
    if let Some(notice) = notice {
        push_notice(hwnd, notice);
    }
    if closed {
        // The path on the command line was not a folder, and the last session closed its
        // notebook: none is open.
        host(hwnd, |host| host.folder = None);
        super::main_window::invalidate_title_strip(hwnd);
        return;
    }
```

`document_loaded` becomes:

```rust
/// After a file is opened into a tab: remember its disk stamp, read before the load so a change
/// landing during it still pauses the next autosave.
pub(crate) fn document_loaded(hwnd: HWND, stamp: Option<library::DiskStamp>) {
    if let Some(mut app) = unsafe { app_ptr(hwnd) } {
        let app = unsafe { app.as_mut() };
        if let Some(id) = app.tabs.active().map(|document| document.id)
            && let Some(document) = app.tabs.document_mut(id)
            && document.path.is_some()
        {
            document.disk_stamp = stamp;
        }
    }
}
```

The doc on `save_local` begins `/// Writes the per-PC local file when its expanded folders, autosave switch or missing times changed.`

Add at the end of `src/window/library_host.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(label: &str) -> Self {
            let root = std::env::temp_dir()
                .join(format!("fastpad-host-startup-{label}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(root.join("data")).unwrap();
            std::fs::create_dir_all(root.join("notes")).unwrap();
            Self(root)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_session_that_closed_its_notebook_opens_none_and_checks_no_folder() {
        // Break caught: startup reopening the last notebook, or falling back to
        // Documents\FastPad, after the user closed it.
        let scratch = Scratch::new("closed");
        let before = library::folder_checks();
        let resolved = resolve_startup(Startup {
            launch: None,
            remembered: None,
            closed: true,
            data: scratch.0.join("data"),
        });
        assert_eq!(resolved, (None, None));
        assert_eq!(library::folder_checks(), before);
    }

    #[test]
    fn a_folder_on_the_command_line_opens_after_a_close_and_clears_it() {
        // Break caught: `fastpad.exe D:\Notes` refused after a close, or leaving open=none so the
        // next plain start opens nothing again.
        let scratch = Scratch::new("closed-launch");
        let data = scratch.0.join("data");
        let mut folders = library::local::RecentFolders::default();
        folders.set_closed(true);
        library::local::write_folders(&library::local::folders_file(&data), &folders).unwrap();
        let notes = library::normalize_folder(&scratch.0.join("notes"));
        let (folder, notice) = resolve_startup(Startup {
            launch: Some(notes.clone()),
            remembered: None,
            closed: true,
            data: data.clone(),
        });
        assert_eq!((folder, notice), (Some(notes.clone()), None));
        let saved = library::local::read_folders(&library::local::folders_file(&data));
        assert!(!saved.closed);
        assert_eq!(saved.folders, [notes]);
    }
}
```

- [ ] **Step 6: The literals and window tests that used the recent-note list**

`RecentFolders { folders: ... }` literals gain `..Default::default()`:

- `src/window/main_window.rs`, `a_recent_folder_pick_opens_the_row_that_was_shown`:

```rust
                &crate::library::local::RecentFolders {
                    folders: order,
                    ..Default::default()
                },
```

- `tests/windows/library.rs` (`Scratch::new`) and `src/bin/fastpad-bench.rs` (`ScratchLocalAppData`), the same shape:

```rust
        let recent = fastpad::library::local::RecentFolders {
            folders: vec![scratch.folder()],
            ..Default::default()
        };
```

(in the bench, `folders: vec![folder],`).

In the `mod tests` block of `src/window/main_window.rs`, replace `a_metadata_flush_leaves_the_local_file_alone_and_a_recent_change_writes_it` with:

```rust
    #[test]
    fn a_metadata_flush_leaves_the_local_file_alone_and_an_expansion_change_writes_it() {
        // Break caught: every 500 ms metadata flush, every rescan or every opened note
        // re-encoding and rewriting the whole per-PC local file (with its scan cache) on the UI
        // thread.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("local-untouched");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        open_note(&window, &scratch, "a.md", "a");
        crate::window::library_host::flush_now(window.hwnd);
        let local = crate::library::local::local_file(&scratch.data(), &scratch.folder());
        assert!(local.exists(), "the load wrote it");
        std::fs::remove_file(&local).unwrap();

        let b = scratch.note("b.md", "b");
        super::open_path(window.hwnd, &b).unwrap();
        crate::window::library_host::flush_now(window.hwnd);
        assert!(!local.exists(), "opening a note changes nothing local");

        execute_command(window.hwnd, CommandId::NoteTogglePin);
        crate::window::library_host::flush_now(window.hwnd);
        assert!(crate::library::store::library_file(&scratch.folder()).exists());
        assert!(!local.exists(), "only library.ini changed");

        crate::window::library_host::with_state(window.hwnd, |state| {
            state.local.set_expanded(std::path::Path::new("sub"), true);
        });
        crate::window::library_host::flush_now(window.hwnd);
        assert!(
            std::fs::read_to_string(&local)
                .unwrap()
                .contains("expanded=sub\r\n"),
            "an expansion change is written"
        );
    }
```

Add, after `the_library_step_checks_no_folder_on_the_ui_thread_and_the_worker_falls_back`:

```rust
    #[test]
    fn a_session_that_ended_with_no_notebook_open_opens_none_at_startup() {
        // Break caught: open=none ignored, so a closed notebook came back at the next start, or
        // a worker started (and stat-ed a folder) to find out there was nothing to open.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("startup-closed");
        let mut folders = crate::library::local::RecentFolders::default();
        folders.push(scratch.folder());
        folders.set_closed(true);
        crate::library::local::write_folders(
            &crate::library::local::folders_file(&scratch.data()),
            &folders,
        )
        .unwrap();
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        app_mut(window.hwnd).library.data_dir = Some(scratch.data());

        let before = crate::library::folder_checks();
        crate::window::library_host::open_library_step(window.hwnd);
        assert_eq!(crate::library::folder_checks(), before);
        assert_eq!(crate::window::library_host::folder(window.hwnd), None);
        assert!(!app_mut(window.hwnd).library.scanning, "no worker starts");
    }
```

- [ ] **Step 7: Compile and run the targeted tests**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: no warnings or errors.

Run: `cargo fmt --all -- --check`
Expected: no diff.

Run: `cargo test --lib -- --test-threads=1 library::local library::tests library::reconcile window::library_host`
Expected: all pass, including the 12 tests in `library::local` and the 2 in `window::library_host::tests`.

Run: `cargo test --lib -- --test-threads=1 a_metadata_flush_leaves_the_local_file_alone a_session_that_ended_with_no_notebook_open the_library_step_checks_no_folder a_recent_folder_pick`
Expected: all 4 pass.

- [ ] **Step 8: Commit**

```bash
git add src/library/local.rs src/library/mod.rs src/library/reconcile.rs src/window/library_host.rs src/window/main_window.rs tests/windows/library.rs src/bin/fastpad-bench.rs
git commit -m "feat(library): expanded folders, favorite notebooks and open=none; drop recent notes"
```

---

### Task 3: The note tree

**Files:**
- Create: `src/library/tree.rs`
- Modify: `src/library/mod.rs` (`pub mod tree;`, `LibraryState.tree`, `load`, `flush`, `merge_rescan`, `apply`, `add_note`, `remove_note`, `rename_note`, tests)

**Interfaces:**
- Consumes: `LibraryState`, `Library` and `same_path` (Tasks 1 and 2).
- Produces, in `src/library/tree.rs`:
  - `UnsavedEntry { key: u64, label: String }`
  - `RowKind::{Unsaved(u64), Folder(PathBuf), Note(PathBuf)}` (paths relative to the notebook)
  - `TreeRow { kind: RowKind, depth: u16, name: String, pinned: bool, expanded: bool }`
  - `NoteTree::build(notes: &[PathBuf], pinned: &[PathBuf]) -> NoteTree`, `NoteTree::default()`
  - `NoteTree::{insert_note(&mut self, path: &Path, pinned: bool), remove_note(&mut self, path: &Path), rename_note(&mut self, old: &Path, new: &Path), set_pinned(&mut self, path: &Path, pinned: bool), note_count(&self) -> usize, rows(&self, expanded: &dyn Fn(&Path) -> bool, unsaved: &[UnsavedEntry]) -> Vec<TreeRow>}`
  - `natural_cmp(a: &str, b: &str) -> Ordering`, `row_index(rows: &[TreeRow], kind: &RowKind) -> Option<usize>`, `parent_index(rows: &[TreeRow], index: usize) -> Option<usize>`, `type_ahead(rows: &[TreeRow], from: usize, prefix: &str) -> Option<usize>`, `ancestors(path: &Path) -> Vec<PathBuf>`
  - `LibraryState.tree: NoteTree`, built by `load` on the worker and kept current by `merge_rescan`, `flush`, `add_note`, `remove_note`, `rename_note` and `apply(SetPinned)`

Design notes, binding for this task:
- The root is not a row: its contents are at depth 0. Unsaved entries come first, at depth 0.
- Folders and notes are matched ignoring case (NTFS). A folder keeps the spelling it was first seen with.
- Nothing is recursive: `build` nests an arena bottom-up, `rows` walks with an explicit stack, and `Drop` flattens, so a very deep chain cannot overflow the UI thread's stack.
- `merge_rescan` runs on the UI thread, so it does not rebuild. The rescan's tree was built on the worker; only the paths FastPad touched while it ran, and the pins the merge changed, are applied to it.

- [ ] **Step 1: Write the failing tests** at the bottom of `src/library/tree.rs`

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn all(_: &Path) -> bool {
        true
    }

    fn build(notes: &[&str], pinned: &[&str]) -> NoteTree {
        let notes: Vec<PathBuf> = notes.iter().map(PathBuf::from).collect();
        let pinned: Vec<PathBuf> = pinned.iter().map(PathBuf::from).collect();
        NoteTree::build(&notes, &pinned)
    }

    /// Each row as `<indent><name><marker>`: `/` a folder, `*` a pinned note, `?` unsaved.
    fn outline(rows: &[TreeRow]) -> Vec<String> {
        rows.iter()
            .map(|row| {
                let marker = match (&row.kind, row.pinned) {
                    (RowKind::Folder(_), _) => "/",
                    (RowKind::Unsaved(_), _) => "?",
                    (RowKind::Note(_), true) => "*",
                    (RowKind::Note(_), false) => "",
                };
                format!("{}{}{marker}", "  ".repeat(usize::from(row.depth)), row.name)
            })
            .collect()
    }

    #[test]
    fn natural_order_compares_digit_runs_as_numbers_and_ignores_case() {
        // Break caught: "Note 10" sorting before "Note 2", or "apple" after "Banana".
        assert_eq!(natural_cmp("Note 2", "Note 10"), Ordering::Less);
        assert_eq!(natural_cmp("note 2", "Note 2"), Ordering::Equal);
        assert_eq!(natural_cmp("apple", "Banana"), Ordering::Less);
        assert_eq!(natural_cmp("file10b", "file10a"), Ordering::Greater);
        assert_eq!(natural_cmp("x9", "x09"), Ordering::Equal);
        assert_eq!(natural_cmp("", "a"), Ordering::Less);
        assert_eq!(
            natural_cmp("n123456789012345678901234567890", "n2"),
            Ordering::Greater,
            "digit runs longer than any integer type"
        );
        assert_eq!(natural_cmp("Äpfel", "äpfel"), Ordering::Equal);
    }

    #[test]
    fn each_folder_lists_pinned_notes_then_subfolders_then_other_notes() {
        // Break caught: pins mixed into the name order, folders after the notes, or "Gamma 10"
        // before "Gamma 9".
        let tree = build(
            &[
                "b.md",
                "Note 10.md",
                "Note 2.md",
                r"beta\x.md",
                r"Alpha\y.md",
                r"Gamma 10\q.md",
                r"Gamma 9\q.md",
                "z.md",
                r"Alpha\pinned.md",
            ],
            &["z.md", r"Alpha\pinned.md"],
        );
        assert_eq!(
            outline(&tree.rows(&all, &[])),
            [
                "z*",
                "Alpha/",
                "  pinned*",
                "  y",
                "beta/",
                "  x",
                "Gamma 9/",
                "  q",
                "Gamma 10/",
                "  q",
                "b",
                "Note 2",
                "Note 10",
            ]
        );
        assert_eq!(tree.note_count(), 9);
    }

    #[test]
    fn ties_are_broken_by_extension_then_by_exact_name() {
        let tree = build(&["a.txt", "A2.md", "a1.md", "a.md", "a01.md"], &[]);
        let paths: Vec<PathBuf> = tree
            .rows(&all, &[])
            .into_iter()
            .map(|row| match row.kind {
                RowKind::Note(path) => path,
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(
            paths,
            ["a.md", "a.txt", "a01.md", "a1.md", "A2.md"].map(PathBuf::from)
        );
    }

    #[test]
    fn rows_do_not_depend_on_the_order_the_scan_listed_notes_in() {
        // Break caught: a rescan after an edit (which changes a file's position in the scan)
        // moving its row, so opening or editing a note reorders the tree.
        let notes = [r"s\b.md", "c.md", r"s\a.md", "a.md", "b.md"];
        let mut reversed = notes;
        reversed.reverse();
        assert_eq!(
            build(&notes, &["b.md"]).rows(&all, &[]),
            build(&reversed, &["b.md"]).rows(&all, &[])
        );
    }

    #[test]
    fn collapsed_folders_hide_their_rows_and_the_root_is_always_open() {
        let tree = build(&["top.md", r"a\one.md", r"a\b\two.md"], &[]);
        let only_a = |path: &Path| path == Path::new("a");
        let rows = tree.rows(&only_a, &[]);
        assert_eq!(outline(&rows), ["a/", "  b/", "  one", "top"]);
        assert!(rows[0].expanded && !rows[1].expanded);
        assert_eq!(rows[1].kind, RowKind::Folder(PathBuf::from(r"a\b")));
        assert_eq!(rows[2].kind, RowKind::Note(PathBuf::from(r"a\one.md")));
        assert_eq!(outline(&tree.rows(&|_| false, &[])), ["a/", "top"]);
    }

    #[test]
    fn unsaved_entries_come_first_at_the_root() {
        let tree = build(&["a.md"], &["a.md"]);
        let unsaved = [
            UnsavedEntry {
                key: 7,
                label: "Idea".into(),
            },
            UnsavedEntry {
                key: 3,
                label: "Untitled".into(),
            },
        ];
        let rows = tree.rows(&all, &unsaved);
        assert_eq!(outline(&rows), ["Idea?", "Untitled?", "a*"]);
        assert_eq!(rows[1].kind, RowKind::Unsaved(3));
        assert_eq!(rows[1].depth, 0);
    }

    #[test]
    fn folders_and_notes_match_ignoring_case() {
        // Break caught: `Sub\a.md` and `sub\b.md` showing as two folders, or a case-only rename
        // leaving the old row behind.
        let mut tree = build(&[r"Sub\a.md", r"sub\b.md"], &[]);
        assert_eq!(outline(&tree.rows(&all, &[])), ["Sub/", "  a", "  b"]);
        tree.insert_note(Path::new(r"SUB\A.MD"), true);
        assert_eq!(outline(&tree.rows(&all, &[])), ["Sub/", "  A*", "  b"]);
        assert_eq!(tree.note_count(), 2);
        tree.remove_note(Path::new(r"sub\b.MD"));
        tree.set_pinned(Path::new(r"sub\a.md"), false);
        assert_eq!(outline(&tree.rows(&all, &[])), ["Sub/", "  A"]);
        assert_eq!(tree.note_count(), 1);
    }

    #[test]
    fn incremental_updates_match_a_full_rebuild() {
        // Break caught: an insert, removal, rename or pin change leaving the tree in an order
        // (or with an empty folder) that a fresh build of the same notes would not have.
        let names: Vec<PathBuf> = [
            "a.md",
            "b.md",
            r"x\c.md",
            r"x\d.md",
            r"x\y\e.md",
            r"z\f.md",
            "Note 2.md",
            "Note 10.md",
        ]
        .map(PathBuf::from)
        .to_vec();
        let mut model: Vec<(PathBuf, bool)> = Vec::new();
        let mut tree = NoteTree::default();
        let mut seed: u64 = 0x2545_f491_4f6c_dd1d;
        let mut next = |bound: usize| {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (seed >> 33) as usize % bound
        };
        for step in 0..400 {
            let path = names[next(names.len())].clone();
            match next(4) {
                0 => {
                    let pinned = next(2) == 0;
                    tree.insert_note(&path, pinned);
                    model.retain(|(existing, _)| existing != &path);
                    model.push((path, pinned));
                }
                1 => {
                    tree.remove_note(&path);
                    model.retain(|(existing, _)| existing != &path);
                }
                2 => {
                    let pinned = next(2) == 0;
                    tree.set_pinned(&path, pinned);
                    if let Some(entry) = model.iter_mut().find(|(existing, _)| existing == &path) {
                        entry.1 = pinned;
                    }
                }
                _ => {
                    let target = names[next(names.len())].clone();
                    // A rename onto an existing note never happens: the file system refuses it.
                    if model.iter().any(|(existing, _)| existing == &target) {
                        continue;
                    }
                    tree.rename_note(&path, &target);
                    if let Some(entry) = model.iter_mut().find(|(existing, _)| existing == &path) {
                        entry.0 = target;
                    }
                }
            }
            let notes: Vec<PathBuf> = model.iter().map(|(path, _)| path.clone()).collect();
            let pinned: Vec<PathBuf> = model
                .iter()
                .filter(|(_, pinned)| *pinned)
                .map(|(path, _)| path.clone())
                .collect();
            assert_eq!(
                tree.rows(&all, &[]),
                NoteTree::build(&notes, &pinned).rows(&all, &[]),
                "step {step}"
            );
            assert_eq!(tree.note_count(), model.len(), "step {step}");
        }
    }

    #[test]
    fn removing_the_last_note_in_a_folder_chain_removes_the_chain() {
        // Break caught: an empty folder row left behind after its only note was deleted or moved.
        let mut tree = build(&[r"a\b\c\n.md", r"a\keep.md", "top.md"], &[]);
        tree.remove_note(Path::new(r"a\b\c\n.md"));
        assert_eq!(outline(&tree.rows(&all, &[])), ["a/", "  keep", "top"]);
        tree.remove_note(Path::new(r"a\keep.md"));
        assert_eq!(outline(&tree.rows(&all, &[])), ["top"]);
        tree.remove_note(Path::new("missing.md"));
        assert_eq!(tree.note_count(), 1);
    }

    #[test]
    fn pins_and_renames_move_rows_and_renames_keep_the_pin() {
        let mut tree = build(&["a.md", "b.md", "c.md"], &[]);
        tree.set_pinned(Path::new("c.md"), true);
        assert_eq!(outline(&tree.rows(&all, &[])), ["c*", "a", "b"]);
        tree.rename_note(Path::new("c.md"), Path::new(r"sub\c2.md"));
        assert_eq!(outline(&tree.rows(&all, &[])), ["sub/", "  c2*", "a", "b"]);
        tree.rename_note(Path::new("gone.md"), Path::new("new.md"));
        assert_eq!(tree.note_count(), 3, "renaming a note the tree lacks does nothing");
    }

    #[test]
    fn rows_are_found_by_path_parent_and_typed_prefix() {
        // Break caught: Left on a nested note jumping to the wrong folder, or type-ahead stuck
        // on the current row instead of moving on.
        let tree = build(&[r"a\one.md", r"a\b\two.md", "top.md"], &[]);
        let rows = tree.rows(&all, &[]);
        assert_eq!(outline(&rows), ["a/", "  b/", "    two", "  one", "top"]);
        assert_eq!(
            row_index(&rows, &RowKind::Note(PathBuf::from(r"A\B\TWO.md"))),
            Some(2)
        );
        assert_eq!(
            row_index(&rows, &RowKind::Folder(PathBuf::from(r"a\b"))),
            Some(1)
        );
        assert_eq!(row_index(&rows, &RowKind::Unsaved(3)), None);
        assert_eq!(parent_index(&rows, 2), Some(1));
        assert_eq!(parent_index(&rows, 3), Some(0));
        assert_eq!(parent_index(&rows, 1), Some(0));
        assert_eq!(parent_index(&rows, 0), None);
        assert_eq!(parent_index(&rows, 4), None);
        assert_eq!(parent_index(&rows, 9), None);
        assert_eq!(type_ahead(&rows, 0, "t"), Some(2));
        assert_eq!(type_ahead(&rows, 3, "T"), Some(4));
        assert_eq!(type_ahead(&rows, 5, "on"), Some(3), "wraps around");
        assert_eq!(type_ahead(&rows, 0, ""), None);
        assert_eq!(type_ahead(&rows, 0, "zz"), None);
        assert_eq!(
            ancestors(Path::new(r"a\b\two.md")),
            [PathBuf::from("a"), PathBuf::from(r"a\b")]
        );
        assert!(ancestors(Path::new("top.md")).is_empty());
    }

    #[test]
    fn ten_thousand_notes_in_one_folder_flatten_quickly() {
        // Break caught: a quadratic build or flatten, so opening a big notebook or expanding its
        // one huge folder stalls.
        let notes: Vec<PathBuf> = (0..10_000)
            .map(|index| PathBuf::from(format!(r"big\Note {index}.md")))
            .collect();
        let pinned = vec![PathBuf::from(r"big\Note 9999.md")];
        let started = std::time::Instant::now();
        let tree = NoteTree::build(&notes, &pinned);
        let rows = tree.rows(&all, &[]);
        let elapsed = started.elapsed();
        assert_eq!(tree.note_count(), 10_000);
        assert_eq!(rows.len(), 10_001);
        assert_eq!(
            outline(&rows[..4]),
            ["big/", "  Note 9999*", "  Note 0", "  Note 1"]
        );
        assert_eq!(rows[10_000].name, "Note 9998");
        assert_eq!(tree.rows(&|_| false, &[]).len(), 1);
        if !cfg!(debug_assertions) {
            assert!(elapsed < std::time::Duration::from_millis(100), "{elapsed:?}");
        }
    }

    #[test]
    fn a_very_deep_folder_chain_builds_flattens_and_empties_without_recursion() {
        // Break caught: a recursive walk (or drop) overflowing the UI thread's stack on a
        // notebook nested thousands of folders deep.
        let depth = 2_000;
        let folder: PathBuf = (0..depth).map(|index| format!("d{index}")).collect();
        let note = folder.join("deep.md");
        let mut tree = NoteTree::build(std::slice::from_ref(&note), &[]);
        let rows = tree.rows(&all, &[]);
        assert_eq!(rows.len(), depth + 1);
        assert_eq!(usize::from(rows[depth].depth), depth);
        assert_eq!(rows[depth].kind, RowKind::Note(note.clone()));
        tree.remove_note(&note);
        assert!(tree.rows(&all, &[]).is_empty());
        let deep = NoteTree::build(std::slice::from_ref(&note), &[]);
        drop(deep);
    }

    #[test]
    fn paths_that_are_not_plain_relative_names_are_ignored() {
        let mut tree = build(&[r"C:\abs\x.md", r"..\up.md", "", r".\dot.md", "ok.md"], &[]);
        tree.insert_note(Path::new(r"\rooted.md"), false);
        assert_eq!(outline(&tree.rows(&all, &[])), ["ok"]);
        assert_eq!(tree.note_count(), 1);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Add `pub mod tree;` to `src/library/mod.rs`, after `pub mod title;`.

Run: `cargo test --lib library::tree -- --test-threads=1`
Expected: a compile error (`NoteTree`, `TreeRow`, `natural_cmp` and the rest do not exist).

- [ ] **Step 3: Write the implementation** at the top of `src/library/tree.rs`

```rust
//! The notebook's folder tree: built from the scan's note list and the pins, kept current by
//! incremental updates, and flattened into the rows the Notebook view shows. Pure: no Win32 and
//! no disk.
//!
//! In each folder: pinned notes, then subfolders, then other notes, each group in natural,
//! case-insensitive name order ("Note 2" before "Note 10"), ties broken by extension, then by the
//! exact name. A folder exists only while it holds a note at some depth. Paths are relative to
//! the notebook and matched ignoring case, like NTFS. Nothing here recurses, so a very deep
//! folder chain cannot overflow a stack.

use std::borrow::Cow;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::path::{Component, Path, PathBuf};

/// An untitled tab, listed first at the root.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnsavedEntry {
    /// Identifies the tab to the caller (its document ID).
    pub key: u64,
    pub label: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RowKind {
    Unsaved(u64),
    Folder(PathBuf),
    Note(PathBuf),
}

/// One visible row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TreeRow {
    pub kind: RowKind,
    /// 0 for the root's contents.
    pub depth: u16,
    /// A folder's name, a note's file name without its extension, or an unsaved tab's label.
    pub name: String,
    pub pinned: bool,
    /// For a folder row: whether its contents follow it.
    pub expanded: bool,
}

#[derive(Debug, Default)]
pub struct NoteTree {
    root: Folder,
    count: usize,
}

#[derive(Debug, Default)]
struct Folder {
    /// As first seen; empty for the root.
    name: OsString,
    /// Sorted by `folder_order`.
    folders: Vec<Folder>,
    /// Pinned notes first, each group sorted by `note_order`.
    notes: Vec<Note>,
}

#[derive(Debug)]
struct Note {
    /// The file name, with its extension.
    file_name: OsString,
    pinned: bool,
}

impl Note {
    fn name(&self) -> Cow<'_, str> {
        self.file_name.to_string_lossy()
    }
}

/// Compares names the way people read them: case-insensitive, with digit runs compared as
/// numbers ("Note 2" before "Note 10"). Equal names can still differ in exact spelling ("x9" and
/// "x09", "a" and "A"); callers break those ties. Allocates nothing.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a, b);
    loop {
        let (Some(x), Some(y)) = (a.chars().next(), b.chars().next()) else {
            return b.is_empty().cmp(&a.is_empty());
        };
        if x.is_ascii_digit() && y.is_ascii_digit() {
            let (left, rest_a) = split_digits(a);
            let (right, rest_b) = split_digits(b);
            let (left, right) = (left.trim_start_matches('0'), right.trim_start_matches('0'));
            let order = left.len().cmp(&right.len()).then_with(|| left.cmp(right));
            if order != Ordering::Equal {
                return order;
            }
            (a, b) = (rest_a, rest_b);
        } else {
            let order = x.to_lowercase().cmp(y.to_lowercase());
            if order != Ordering::Equal {
                return order;
            }
            (a, b) = (&a[x.len_utf8()..], &b[y.len_utf8()..]);
        }
    }
}

fn split_digits(text: &str) -> (&str, &str) {
    let end = text
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(text.len());
    text.split_at(end)
}

/// Equal ignoring case, char by char, the way `natural_cmp` folds case.
fn eq_ignore_case(a: &str, b: &str) -> bool {
    let (mut left, mut right) = (a.chars(), b.chars());
    loop {
        match (left.next(), right.next()) {
            (None, None) => return true,
            (Some(x), Some(y)) if x == y || x.to_lowercase().eq(y.to_lowercase()) => {}
            _ => return false,
        }
    }
}

fn starts_with_ignore_case(name: &str, prefix: &str) -> bool {
    let mut name = name.chars();
    prefix.chars().all(|wanted| {
        name.next()
            .is_some_and(|c| c == wanted || c.to_lowercase().eq(wanted.to_lowercase()))
    })
}

/// A file name's stem (what a row shows) and extension, split like `Path::file_stem`.
fn split_name(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        None | Some(0) => (name, ""),
        Some(index) => (&name[..index], &name[index + 1..]),
    }
}

/// Natural stem order, then natural extension order. Equal for names that differ only in case.
fn note_loose(a: &str, b: &str) -> Ordering {
    let (stem_a, extension_a) = split_name(a);
    let (stem_b, extension_b) = split_name(b);
    natural_cmp(stem_a, stem_b).then_with(|| natural_cmp(extension_a, extension_b))
}

fn note_order(a: &Note, b: &Note) -> Ordering {
    b.pinned
        .cmp(&a.pinned)
        .then_with(|| note_loose(&a.name(), &b.name()))
        .then_with(|| a.file_name.cmp(&b.file_name))
}

fn folder_order(a: &Folder, b: &Folder) -> Ordering {
    natural_cmp(&a.name.to_string_lossy(), &b.name.to_string_lossy())
        .then_with(|| a.name.cmp(&b.name))
}

/// The folder names and the file name of a plain relative path; `None` for anything else (an
/// absolute or rooted path, `.` or `..`, or no file name).
fn split_path(path: &Path) -> Option<(Vec<&OsStr>, &OsStr)> {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => parts.push(part),
            _ => return None,
        }
    }
    let file_name = parts.pop()?;
    Some((parts, file_name))
}

/// One key per note or folder, whatever the letter case or separator.
fn path_key(path: &Path) -> String {
    let mut key = String::new();
    for component in path.components() {
        if !key.is_empty() {
            key.push('\\');
        }
        key.push_str(&component.as_os_str().to_string_lossy().to_lowercase());
    }
    key
}

impl Folder {
    fn is_empty(&self) -> bool {
        self.folders.is_empty() && self.notes.is_empty()
    }

    fn pinned_count(&self) -> usize {
        self.notes.partition_point(|note| note.pinned)
    }

    fn find_folder(&self, name: &str) -> Option<usize> {
        let start = self.folders.partition_point(|folder| {
            natural_cmp(&folder.name.to_string_lossy(), name) == Ordering::Less
        });
        self.folders[start..]
            .iter()
            .take_while(|folder| {
                natural_cmp(&folder.name.to_string_lossy(), name) == Ordering::Equal
            })
            .position(|folder| eq_ignore_case(&folder.name.to_string_lossy(), name))
            .map(|offset| start + offset)
    }

    fn child_or_insert(&mut self, name: &OsStr) -> &mut Folder {
        let index = match self.find_folder(&name.to_string_lossy()) {
            Some(index) => index,
            None => {
                let folder = Folder {
                    name: name.to_os_string(),
                    ..Folder::default()
                };
                let index = self
                    .folders
                    .partition_point(|existing| folder_order(existing, &folder) == Ordering::Less);
                self.folders.insert(index, folder);
                index
            }
        };
        &mut self.folders[index]
    }

    /// The note named `name` (ignoring case), by binary search in each pin group.
    fn find_note(&self, name: &str) -> Option<usize> {
        let split = self.pinned_count();
        for (offset, group) in [(0, &self.notes[..split]), (split, &self.notes[split..])] {
            let start =
                group.partition_point(|note| note_loose(&note.name(), name) == Ordering::Less);
            let found = group[start..]
                .iter()
                .take_while(|note| note_loose(&note.name(), name) == Ordering::Equal)
                .position(|note| eq_ignore_case(&note.name(), name));
            if let Some(found) = found {
                return Some(offset + start + found);
            }
        }
        None
    }

    fn insert_note(&mut self, note: Note) {
        let index = self
            .notes
            .partition_point(|existing| note_order(existing, &note) == Ordering::Less);
        self.notes.insert(index, note);
    }

    fn finish(&mut self) {
        self.folders.sort_by(folder_order);
        self.notes.sort_by(note_order);
    }
}

impl NoteTree {
    /// The tree of `notes`, with the ones in `pinned` pinned. Both are relative to the notebook;
    /// a path that is not a plain relative path is skipped, and a second spelling of one note
    /// (another letter case) is dropped.
    pub fn build(notes: &[PathBuf], pinned: &[PathBuf]) -> NoteTree {
        let pinned: HashSet<String> = pinned.iter().map(|path| path_key(path)).collect();
        // Folders live in an arena keyed by lower-case relative path, so each note finds its
        // folder in one lookup however many folders there are; they are nested once, at the end.
        let mut arena = vec![Folder::default()];
        let mut parents = vec![0_usize];
        let mut by_key: HashMap<String, usize> = HashMap::new();
        let mut seen = HashSet::new();
        let mut count = 0;
        for path in notes {
            let Some((folders, file_name)) = split_path(path) else {
                continue;
            };
            let key = path_key(path);
            let is_pinned = pinned.contains(&key);
            if !seen.insert(key) {
                continue;
            }
            let mut current = 0;
            let mut folder_key = String::new();
            for part in folders {
                if !folder_key.is_empty() {
                    folder_key.push('\\');
                }
                folder_key.push_str(&part.to_string_lossy().to_lowercase());
                current = match by_key.get(&folder_key) {
                    Some(&index) => index,
                    None => {
                        arena.push(Folder {
                            name: part.to_os_string(),
                            ..Folder::default()
                        });
                        parents.push(current);
                        by_key.insert(folder_key.clone(), arena.len() - 1);
                        arena.len() - 1
                    }
                };
            }
            arena[current].notes.push(Note {
                file_name: file_name.to_os_string(),
                pinned: is_pinned,
            });
            count += 1;
        }
        // Every folder comes after its parent in the arena, so taking them from the end nests
        // each one before its parent is taken.
        while arena.len() > 1 {
            let (Some(mut folder), Some(parent)) = (arena.pop(), parents.pop()) else {
                break;
            };
            folder.finish();
            arena[parent].folders.push(folder);
        }
        let mut root = arena.pop().unwrap_or_default();
        root.finish();
        NoteTree { root, count }
    }

    pub fn note_count(&self) -> usize {
        self.count
    }

    /// Adds a note, or replaces the entry for the same file (any letter case, any pin).
    pub fn insert_note(&mut self, path: &Path, pinned: bool) {
        let Some((folders, file_name)) = split_path(path) else {
            return;
        };
        self.remove_note(path);
        let mut folder = &mut self.root;
        for part in folders {
            folder = folder.child_or_insert(part);
        }
        folder.insert_note(Note {
            file_name: file_name.to_os_string(),
            pinned,
        });
        self.count += 1;
    }

    /// Removes a note, and every folder that holds nothing after it. Unknown paths do nothing.
    pub fn remove_note(&mut self, path: &Path) {
        let Some((folders, file_name)) = split_path(path) else {
            return;
        };
        let Some(mut trail) = self.trail(&folders) else {
            return;
        };
        let folder = self.folder_at_mut(&trail);
        let Some(index) = folder.find_note(&file_name.to_string_lossy()) else {
            return;
        };
        folder.notes.remove(index);
        self.count -= 1;
        // Deepest first: each folder left holding nothing goes, up to the first that keeps
        // something.
        while let Some(index) = trail.pop() {
            let parent = self.folder_at_mut(&trail);
            if !parent.folders[index].is_empty() {
                break;
            }
            parent.folders.remove(index);
        }
    }

    /// Moves `old`'s row to `new`, keeping its pin. Does nothing when `old` is not in the tree.
    pub fn rename_note(&mut self, old: &Path, new: &Path) {
        if let Some(pinned) = self.note_pinned(old) {
            self.remove_note(old);
            self.insert_note(new, pinned);
        }
    }

    /// Pins or unpins a note in place; its row moves to its new group. Unknown paths do nothing.
    pub fn set_pinned(&mut self, path: &Path, pinned: bool) {
        let Some((folders, file_name)) = split_path(path) else {
            return;
        };
        let Some(trail) = self.trail(&folders) else {
            return;
        };
        let folder = self.folder_at_mut(&trail);
        let Some(index) = folder.find_note(&file_name.to_string_lossy()) else {
            return;
        };
        if folder.notes[index].pinned == pinned {
            return;
        }
        let mut note = folder.notes.remove(index);
        note.pinned = pinned;
        folder.insert_note(note);
    }

    /// The visible rows: `unsaved` first, then the root's contents, and inside each folder that
    /// `expanded` says is open, its contents one level deeper.
    pub fn rows(&self, expanded: &dyn Fn(&Path) -> bool, unsaved: &[UnsavedEntry]) -> Vec<TreeRow> {
        struct Frame<'a> {
            folder: &'a Folder,
            path: PathBuf,
            depth: u16,
            next: usize,
        }
        let mut rows = Vec::with_capacity(
            unsaved.len() + self.root.folders.len() + self.root.notes.len(),
        );
        rows.extend(unsaved.iter().map(|entry| TreeRow {
            kind: RowKind::Unsaved(entry.key),
            depth: 0,
            name: entry.label.clone(),
            pinned: false,
            expanded: false,
        }));
        push_notes(&mut rows, &self.root, Path::new(""), 0, true);
        let mut stack = vec![Frame {
            folder: &self.root,
            path: PathBuf::new(),
            depth: 0,
            next: 0,
        }];
        while let Some(frame) = stack.last_mut() {
            let folder = frame.folder;
            let Some(child) = folder.folders.get(frame.next) else {
                let done = stack.pop().expect("the loop just saw this frame");
                push_notes(&mut rows, done.folder, &done.path, done.depth, false);
                continue;
            };
            frame.next += 1;
            let depth = frame.depth;
            let path = frame.path.join(&child.name);
            let open = expanded(&path);
            rows.push(TreeRow {
                kind: RowKind::Folder(path.clone()),
                depth,
                name: child.name.to_string_lossy().into_owned(),
                pinned: false,
                expanded: open,
            });
            if open {
                let depth = depth.saturating_add(1);
                push_notes(&mut rows, child, &path, depth, true);
                stack.push(Frame {
                    folder: child,
                    path,
                    depth,
                    next: 0,
                });
            }
        }
        rows
    }

    /// The child index of each folder on the way down to `folders`, or `None` if one is missing.
    fn trail(&self, folders: &[&OsStr]) -> Option<Vec<usize>> {
        let mut trail = Vec::with_capacity(folders.len());
        let mut folder = &self.root;
        for part in folders {
            let index = folder.find_folder(&part.to_string_lossy())?;
            trail.push(index);
            folder = &folder.folders[index];
        }
        Some(trail)
    }

    fn folder_at_mut(&mut self, trail: &[usize]) -> &mut Folder {
        let mut folder = &mut self.root;
        for &index in trail {
            folder = &mut folder.folders[index];
        }
        folder
    }

    fn note_pinned(&self, path: &Path) -> Option<bool> {
        let (folders, file_name) = split_path(path)?;
        let mut folder = &self.root;
        for part in folders {
            folder = &folder.folders[folder.find_folder(&part.to_string_lossy())?];
        }
        let index = folder.find_note(&file_name.to_string_lossy())?;
        Some(folder.notes[index].pinned)
    }
}

impl Drop for NoteTree {
    /// Nested folders would drop recursively; this empties them one level at a time instead.
    fn drop(&mut self) {
        let mut pending = std::mem::take(&mut self.root.folders);
        while let Some(mut folder) = pending.pop() {
            pending.append(&mut folder.folders);
        }
    }
}

/// Appends a folder's pinned notes (`pinned`) or its other notes.
fn push_notes(rows: &mut Vec<TreeRow>, folder: &Folder, path: &Path, depth: u16, pinned: bool) {
    let split = folder.pinned_count();
    let notes = if pinned {
        &folder.notes[..split]
    } else {
        &folder.notes[split..]
    };
    rows.extend(notes.iter().map(|note| TreeRow {
        kind: RowKind::Note(path.join(&note.file_name)),
        depth,
        name: split_name(&note.name()).0.to_owned(),
        pinned: note.pinned,
        expanded: false,
    }));
}

fn same_row_path(a: &Path, b: &Path) -> bool {
    eq_ignore_case(&a.to_string_lossy(), &b.to_string_lossy())
}

/// The row showing `kind`, with paths compared ignoring case.
pub fn row_index(rows: &[TreeRow], kind: &RowKind) -> Option<usize> {
    rows.iter().position(|row| match (&row.kind, kind) {
        (RowKind::Unsaved(a), RowKind::Unsaved(b)) => a == b,
        (RowKind::Folder(a), RowKind::Folder(b)) | (RowKind::Note(a), RowKind::Note(b)) => {
            same_row_path(a, b)
        }
        _ => false,
    })
}

/// The folder row that row `index` sits in; `None` at depth 0 or for an index past the end.
pub fn parent_index(rows: &[TreeRow], index: usize) -> Option<usize> {
    let depth = rows.get(index)?.depth;
    if depth == 0 {
        return None;
    }
    rows[..index]
        .iter()
        .rposition(|row| row.depth == depth - 1 && matches!(row.kind, RowKind::Folder(_)))
}

/// The first row at or after `from`, wrapping around, whose name starts with `prefix` ignoring
/// case. The caller passes the row after the selection for a new prefix, and the selection
/// itself while the user keeps typing the same one.
pub fn type_ahead(rows: &[TreeRow], from: usize, prefix: &str) -> Option<usize> {
    if prefix.is_empty() || rows.is_empty() {
        return None;
    }
    let start = from % rows.len();
    (start..rows.len())
        .chain(0..start)
        .find(|&index| starts_with_ignore_case(&rows[index].name, prefix))
}

/// The folders that contain `path`, outermost first: `a\b\c.md` gives `a` and `a\b`.
pub fn ancestors(path: &Path) -> Vec<PathBuf> {
    let mut folders: Vec<PathBuf> = path
        .ancestors()
        .skip(1)
        .filter(|ancestor| !ancestor.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .collect();
    folders.reverse();
    folders
}
```

- [ ] **Step 4: Run the tree tests to verify they pass**

Run: `cargo test --lib library::tree -- --test-threads=1`
Expected: all 14 tests pass.

- [ ] **Step 5: Write the failing `LibraryState` tests**

In the `mod tests` block of `src/library/mod.rs`, add:

```rust
    fn tree_rows(tree: &tree::NoteTree) -> Vec<tree::TreeRow> {
        tree.rows(&|_| true, &[])
    }

    /// What a fresh build of the state's notes and pins shows.
    fn rebuilt_rows(state: &LibraryState) -> Vec<tree::TreeRow> {
        let paths: Vec<PathBuf> = state.notes.iter().map(|note| note.path.clone()).collect();
        tree_rows(&tree::NoteTree::build(&paths, &pinned_paths(&state.library)))
    }

    #[test]
    fn the_tree_follows_the_index_and_the_pins() {
        // Break caught: the sidebar tree drifting from the notes index after a save, a pin, a
        // rename or a delete, so a row opens a file that is gone or a pin shows on the wrong note.
        let scratch = Scratch::new("tree-follows");
        std::fs::create_dir_all(scratch.folder().join("sub")).unwrap();
        std::fs::write(scratch.folder().join("a.md"), "a").unwrap();
        std::fs::write(scratch.folder().join(r"sub\b.md"), "b").unwrap();
        let mut ids = IdSource::new(1, 2);
        let mut state = load(&scratch.folder(), &scratch.local(), 100).unwrap();
        assert_eq!(state.tree.note_count(), 2);
        assert_eq!(tree_rows(&state.tree), rebuilt_rows(&state));

        let c = scratch.folder().join("c.md");
        std::fs::write(&c, "c").unwrap();
        state.add_note(&c);
        state.add_note(&c);
        assert_eq!(state.tree.note_count(), 3);
        assert_eq!(tree_rows(&state.tree), rebuilt_rows(&state));

        let target = state.note_ref(&mut ids, &c);
        state
            .apply(PendingOp::SetPinned {
                note: target,
                value: true,
            })
            .unwrap();
        assert!(tree_rows(&state.tree)[0].pinned, "a pinned note sorts first");
        assert_eq!(tree_rows(&state.tree), rebuilt_rows(&state));

        let renamed = scratch.folder().join(r"sub\z.md");
        std::fs::rename(&c, &renamed).unwrap();
        state.rename_note(&c, &renamed);
        assert_eq!(tree_rows(&state.tree), rebuilt_rows(&state));
        assert!(
            tree_rows(&state.tree).iter().any(|row| row.pinned
                && row.kind == tree::RowKind::Note(PathBuf::from(r"sub\z.md"))),
            "the pin follows the rename"
        );

        state.remove_note(&renamed);
        state.remove_note(&scratch.folder().join(r"sub\b.md"));
        assert_eq!(tree_rows(&state.tree), rebuilt_rows(&state));
        assert!(
            !tree_rows(&state.tree)
                .iter()
                .any(|row| matches!(row.kind, tree::RowKind::Folder(_))),
            "an emptied folder goes"
        );
    }

    #[test]
    fn merging_a_rescan_leaves_the_tree_equal_to_a_rebuild() {
        // Break caught: the merged state keeping the rescan's tree, which misses a note saved or
        // a pin set while the rescan ran.
        let scratch = Scratch::new("tree-merge");
        let a = scratch.folder().join("a.md");
        std::fs::write(&a, "a").unwrap();
        let mut ids = IdSource::new(1, 2);
        let mut previous = load(&scratch.folder(), &scratch.local(), 100).unwrap();
        let fresh = load(&scratch.folder(), &scratch.local(), 101).unwrap();
        let b = scratch.folder().join("b.md");
        std::fs::write(&b, "b").unwrap();
        previous.add_note(&b);
        let target = previous.note_ref(&mut ids, &a);
        previous
            .apply(PendingOp::SetPinned {
                note: target,
                value: true,
            })
            .unwrap();
        let merged = merge_rescan(previous, fresh);
        assert_eq!(merged.tree.note_count(), 2);
        assert_eq!(tree_rows(&merged.tree), rebuilt_rows(&merged));
        assert!(tree_rows(&merged.tree)[0].pinned);
    }

    #[test]
    fn a_flush_that_rereads_another_pcs_pins_updates_the_tree() {
        // Break caught: a pin synced from another PC reaching library.ini and the live library
        // but never the tree, so the row stays unpinned until the next rescan.
        let scratch = Scratch::new("tree-flush");
        let a = scratch.folder().join("a.md");
        std::fs::write(&a, "a").unwrap();
        std::fs::write(scratch.folder().join("b.md"), "b").unwrap();
        let mut ids = IdSource::new(1, 2);
        let mut state = load(&scratch.folder(), &scratch.local(), 100).unwrap();
        let target = state.note_ref(&mut ids, &a);
        state
            .apply(PendingOp::SetPinned {
                note: target,
                value: true,
            })
            .unwrap();
        let path = store::library_file(&scratch.folder());
        let mut other = Library::default();
        other
            .resolve_note(&NoteRef {
                id: NoteId(77),
                path: "b.md".into(),
            })
            .pinned = true;
        store::write(&path, &other).unwrap();
        assert_eq!(flush(&mut state).unwrap(), Flushed::Wrote);
        assert_eq!(tree_rows(&state.tree), rebuilt_rows(&state));
        assert_eq!(
            tree_rows(&state.tree).iter().filter(|row| row.pinned).count(),
            2
        );
    }
```

and `bare_state` becomes:

```rust
    fn bare_state(scratch: &Scratch, notes: Vec<NoteEntry>, truncated: bool) -> LibraryState {
        let paths: Vec<PathBuf> = notes.iter().map(|note| note.path.clone()).collect();
        LibraryState {
            folder: scratch.folder(),
            local_path: scratch.local(),
            library: Library::default(),
            metadata: Metadata::Ready,
            stamp: None,
            local: LocalState::new(scratch.folder()),
            tree: tree::NoteTree::build(&paths, &[]),
            notes,
            truncated,
            pending: Vec::new(),
            relocated: Vec::new(),
            touched: Vec::new(),
            written_local: local::Conveniences::default(),
        }
    }
```

Run: `cargo test --lib library::tests -- --test-threads=1`
Expected: a compile error (`LibraryState` has no `tree`, and `pinned_paths` does not exist).

- [ ] **Step 6: Keep `LibraryState.tree` current**

In `src/library/mod.rs`:

The `use` block gains `use std::collections::HashSet;`.

`LibraryState` gains, after `notes`:

```rust
    /// The notes as the sidebar's folder tree. Built with the scan on the worker; every later
    /// change to `notes` or to a pin updates it in place.
    pub tree: tree::NoteTree,
```

Add, after `normalize_folder`:

```rust
/// The notes a tree shows as pinned: pinned records that are not flagged deleted.
fn pinned_paths(library: &Library) -> Vec<PathBuf> {
    library
        .notes
        .iter()
        .filter(|record| record.pinned && !record.deleted && !record.path.is_absolute())
        .map(|record| record.path.clone())
        .collect()
}

fn pin_key(path: &Path) -> String {
    path.to_string_lossy().to_lowercase()
}

/// Brings the tree's pins from `before` to `after` without a rebuild.
fn sync_pins(tree: &mut tree::NoteTree, before: &[PathBuf], after: &[PathBuf]) {
    let before_keys: HashSet<String> = before.iter().map(|path| pin_key(path)).collect();
    let after_keys: HashSet<String> = after.iter().map(|path| pin_key(path)).collect();
    for path in after {
        if !before_keys.contains(&pin_key(path)) {
            tree.set_pinned(path, true);
        }
    }
    for path in before {
        if !after_keys.contains(&pin_key(path)) {
            tree.set_pinned(path, false);
        }
    }
}
```

In `load`, the `Ok(LibraryState { ... })` expression becomes:

```rust
    let notes: Vec<NoteEntry> = scan
        .entries
        .iter()
        .map(|entry| NoteEntry {
            path: entry.path.clone(),
            size: entry.size,
            mtime: entry.mtime,
        })
        .collect();
    let paths: Vec<PathBuf> = notes.iter().map(|note| note.path.clone()).collect();
    let tree = tree::NoteTree::build(&paths, &pinned_paths(&library));
    Ok(LibraryState {
        folder: folder.to_path_buf(),
        local_path: local_path.to_path_buf(),
        library,
        metadata,
        stamp,
        notes,
        tree,
        truncated: scan.truncated,
        pending: reconciled.ops,
        relocated: reconciled.relocated,
        touched: Vec::new(),
        written_local: local.conveniences(),
        local,
    })
```

In `flush`, the re-read block becomes:

```rust
    if store::stamp(&path) != state.stamp {
        let before = pinned_paths(&state.library);
        let mut fresh = match store::read(&path) {
            ReadOutcome::Loaded(library, stamp) => {
                state.stamp = Some(stamp);
                library
            }
            ReadOutcome::Absent => {
                state.stamp = None;
                Library::default()
            }
            ReadOutcome::Unreadable => {
                state.metadata = Metadata::Unreadable;
                return Err(crate::FastPadError::Invariant(
                    "library.ini was replaced by a file this FastPad cannot read",
                ));
            }
            // Keeps the pending operations and the library as they are.
            ReadOutcome::Busy => return Ok(Flushed::Busy),
        };
        ops::replay(&mut fresh, &state.pending);
        state.library = fresh;
        // Another PC's pins arrived with the re-read.
        sync_pins(&mut state.tree, &before, &pinned_paths(&state.library));
    }
```

`merge_rescan` becomes (it keeps Task 2's local conveniences, and trades its early return for an `if`/`else` so the tree update runs on both paths):

```rust
/// Installs a rescan's result without losing what changed while it ran.
pub fn merge_rescan(previous: LibraryState, fresh: LibraryState) -> LibraryState {
    let LibraryState {
        library: previous_library,
        metadata: previous_metadata,
        stamp: previous_stamp,
        pending: previous_pending,
        local: previous_local,
        touched,
        ..
    } = previous;
    let mut fresh = fresh;
    // The rescan built its tree from these pins; what the merge changes is applied to it below.
    let built_pins = pinned_paths(&fresh.library);
    fresh.notes = merge_notes(&fresh.folder, std::mem::take(&mut fresh.notes), &touched);
    if fresh.truncated {
        // A truncated scan's own list is already capped at the limit; touched entries that
        // survived the merge must not push it past that.
        fresh.notes.truncate(scan::NOTE_LIMIT);
    }
    // The UI thread owns these conveniences: what it set while the rescan ran wins.
    fresh.local.autosave = previous_local.autosave;
    fresh.local.expanded = previous_local.expanded;

    if fresh.metadata == Metadata::Busy {
        // The rescan could not read library.ini: what the live state knows still stands, and the
        // next rescan reads it again.
        fresh.library = previous_library;
        fresh.metadata = previous_metadata;
        fresh.stamp = previous_stamp;
        fresh.pending = previous_pending;
    } else {
        // A stamp mismatch alone does not say which side is current: the live library may have
        // flushed while the rescan was reading (previous is current and belongs on disk), or the
        // file may have changed outside FastPad while it was inactive, e.g. another PC's sync
        // (the rescan's own read is current). One more stamp, taken now, tells them apart.
        let previous_is_current = previous_metadata == Metadata::Ready
            && fresh.metadata == Metadata::Ready
            && previous_stamp != fresh.stamp
            && store::stamp(&store::library_file(&fresh.folder)) == previous_stamp;

        if previous_is_current {
            let mut library = previous_library;
            ops::replay(&mut library, &fresh.pending);
            fresh.library = library;
            fresh.stamp = previous_stamp;
        } else if fresh.metadata == Metadata::Ready {
            ops::replay(&mut fresh.library, &previous_pending);
        }

        let mut pending = std::mem::take(&mut fresh.pending);
        pending.extend(previous_pending);
        fresh.pending = pending;
    }
    update_merged_tree(&mut fresh, &touched, &built_pins);
    fresh
}

/// Brings the rescan's tree up to the merged notes and pins. Only the paths FastPad touched while
/// the rescan ran can differ from the notes it was built from, and only a pin the merge changed
/// can differ from its pins, so this runs on the UI thread at the cost of those few paths.
fn update_merged_tree(state: &mut LibraryState, touched: &[PathBuf], built_pins: &[PathBuf]) {
    let pins = pinned_paths(&state.library);
    let pinned: HashSet<String> = pins.iter().map(|path| pin_key(path)).collect();
    for path in touched {
        if state.notes.iter().any(|note| same_path(&note.path, path)) {
            state.tree.insert_note(path, pinned.contains(&pin_key(path)));
        } else {
            state.tree.remove_note(path);
        }
    }
    sync_pins(&mut state.tree, built_pins, &pins);
}
```

In `impl LibraryState`, `apply`, `add_note`, `remove_note` and `rename_note` become:

```rust
    /// Applies `op` to the live library and keeps it for the next write. A pin change moves the
    /// note's row in the tree.
    pub fn apply(&mut self, op: PendingOp) -> std::result::Result<(), LibraryError> {
        ops::apply(&mut self.library, &op)?;
        if let PendingOp::SetPinned { note, value } = &op {
            let path = self
                .library
                .note(note.id)
                .or_else(|| self.library.note_by_path(&note.path))
                .map(|record| record.path.clone());
            if let Some(path) = path {
                self.tree.set_pinned(&path, *value);
            }
        }
        self.pending.push(op);
        Ok(())
    }

    /// Adds a file FastPad just saved to the index and the tree, if it is a note inside the
    /// folder. Updates the entry in place when the note is already indexed.
    pub fn add_note(&mut self, path: &Path) {
        let Some(relative) = strip_folder(&self.folder, path) else {
            return;
        };
        let is_note = relative
            .extension()
            .is_some_and(|ext| title::is_note_extension(&ext.to_string_lossy()));
        if !is_note {
            return;
        }
        self.touched.push(relative.clone());
        let stamp = store::stamp(path);
        let size = stamp.map_or(0, |stamp| stamp.size);
        let mtime = stamp.map_or(0, |stamp| filetime_ticks(stamp.modified));
        if let Some(existing) = self
            .notes
            .iter_mut()
            .find(|note| same_path(&note.path, &relative))
        {
            existing.size = size;
            existing.mtime = mtime;
        } else {
            let pinned = self.is_pinned(&relative);
            self.tree.insert_note(&relative, pinned);
            self.notes.push(NoteEntry {
                path: relative,
                size,
                mtime,
            });
        }
    }

    pub fn remove_note(&mut self, path: &Path) {
        let Some(stored) = strip_folder(&self.folder, path) else {
            return;
        };
        self.notes.retain(|note| !same_path(&note.path, &stored));
        self.tree.remove_note(&stored);
        self.touched.push(stored);
    }

    /// Follows a rename FastPad made: the record, the index and the tree. The record moves first,
    /// so the entry added for the new name finds its pin.
    pub fn rename_note(&mut self, old: &Path, new: &Path) {
        let old_stored = record_path(&self.folder, old);
        let new_stored = record_path(&self.folder, new);
        if let Some(record) = self.library.note_by_path(&old_stored) {
            let note = NoteRef {
                id: record.id,
                path: old_stored,
            };
            let _ = self.apply(PendingOp::Relocate {
                note,
                path: new_stored,
            });
        }
        self.remove_note(old);
        self.add_note(new);
    }
```

- [ ] **Step 7: Run the tests to verify they pass**

Run: `cargo test --lib -- --test-threads=1 library::tree library::tests`
Expected: all pass, including `the_tree_follows_the_index_and_the_pins`, `merging_a_rescan_leaves_the_tree_equal_to_a_rebuild`, `a_flush_that_rereads_another_pcs_pins_updates_the_tree`, `a_bulk_change_made_outside_fastpad_costs_no_stat_when_a_rescan_is_merged` (the tree update stats nothing) and `ten_thousand_notes_in_one_folder_flatten_quickly`.

Run: `cargo test --release --lib library::tree::tests::ten_thousand -- --test-threads=1`
Expected: passes (this is the run that checks the 100 ms bound).

Run: `cargo clippy --all-targets -- -D warnings` and `cargo fmt --all -- --check`
Expected: clean.

- [ ] **Step 8: Commit**

```bash
git add src/library/tree.rs src/library/mod.rs
git commit -m "feat(library): the note tree, built on the scan worker and kept current in place"
```

---

### Task 4: Note-name search

**Files:**
- Create: `src/library/name_search.rs`
- Modify: `src/library/mod.rs` (add `pub mod name_search;`)

**Interfaces:**
- Consumes: `tree::natural_cmp` (Task 3).
- Produces:
  - `name_search::NameMatch { path: PathBuf, name: String, folder: String }`: `path` as given (relative to the notebook), `name` the file name without its extension, `folder` the parent path joined with `\`, or `""` at the root
  - `name_search::search(notes: &[PathBuf], query: &str, limit: usize) -> Vec<NameMatch>`: the query is trimmed and matched anywhere in the name ignoring case; names that start with it come first, then the rest, each group in natural name order, then by folder (the root first); at most `limit` results; an empty query finds nothing

- [ ] **Step 1: Write the failing tests** at the bottom of `src/library/name_search.rs`

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn paths(names: &[&str]) -> Vec<PathBuf> {
        names.iter().map(PathBuf::from).collect()
    }

    fn names(matches: &[NameMatch]) -> Vec<&str> {
        matches.iter().map(|found| found.name.as_str()).collect()
    }

    #[test]
    fn names_that_start_with_the_query_come_first_then_the_rest_in_natural_order() {
        // Break caught: "Meeting plan" listed above "Plan 2" when the user typed "plan", or
        // "Plan 10" above "Plan 2".
        let notes = paths(&[
            "Plan 10.md",
            "plan 2.md",
            "Meeting plan.md",
            "Replanning.md",
            r"work\Plans.txt",
            "Other.md",
        ]);
        assert_eq!(
            names(&search(&notes, "plan", 50)),
            ["plan 2", "Plan 10", "Plans", "Meeting plan", "Replanning"]
        );
    }

    #[test]
    fn matching_ignores_case_and_surrounding_spaces_but_not_the_extension() {
        let notes = paths(&["Über uns.md", "notes.md", "Readme"]);
        assert_eq!(names(&search(&notes, "  ÜBER ", 50)), ["Über uns"]);
        assert_eq!(names(&search(&notes, "README", 50)), ["Readme"]);
        assert!(search(&notes, "md", 50).is_empty(), "the extension is not searched");
    }

    #[test]
    fn each_match_names_its_folder_relative_to_the_notebook() {
        // Break caught: results that lose the tree's context showing no folder, a leading
        // separator, or a mix of `/` and `\`.
        let notes = paths(&[r"a\b\x.md", "x.md", "a/x.txt"]);
        let found = search(&notes, "x", 50);
        let folders: Vec<(&str, &str)> = found
            .iter()
            .map(|found| (found.folder.as_str(), found.name.as_str()))
            .collect();
        assert_eq!(folders, [("", "x"), ("a", "x"), (r"a\b", "x")]);
        assert_eq!(found[2].path, PathBuf::from(r"a\b\x.md"));
    }

    #[test]
    fn an_empty_query_or_no_match_finds_nothing() {
        let notes = paths(&["a.md"]);
        assert!(search(&notes, "", 50).is_empty());
        assert!(search(&notes, "   ", 50).is_empty());
        assert!(search(&notes, "zzz", 50).is_empty());
        assert!(search(&notes, "a", 0).is_empty());
    }

    #[test]
    fn the_limit_keeps_the_best_matches() {
        let many: Vec<PathBuf> = (0..300)
            .map(|index| PathBuf::from(format!("n{index}.md")))
            .collect();
        assert_eq!(
            names(&search(&many, "n", 5)),
            ["n0", "n1", "n2", "n3", "n4"]
        );
        let notes = paths(&["xa.md", "zz.md", "a1.md"]);
        assert_eq!(
            names(&search(&notes, "a", 1)),
            ["a1"],
            "a prefix match beats a substring under the limit"
        );
    }

    #[test]
    fn ten_thousand_names_search_quickly() {
        // Break caught: a keystroke that sorts or allocates per note so heavily that typing in
        // the search box lags on a big notebook.
        let notes: Vec<PathBuf> = (0..10_000)
            .map(|index| PathBuf::from(format!(r"folder {}\Note {index}.md", index / 100)))
            .collect();
        let started = std::time::Instant::now();
        let found = search(&notes, "note", 200);
        let elapsed = started.elapsed();
        assert_eq!(found.len(), 200);
        assert_eq!(found[0].name, "Note 0");
        assert_eq!(found[1].name, "Note 1");
        if !cfg!(debug_assertions) {
            assert!(elapsed < std::time::Duration::from_millis(20), "{elapsed:?}");
        }
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Add `pub mod name_search;` to `src/library/mod.rs`, before `pub mod ops;`.

Run: `cargo test --lib library::name_search -- --test-threads=1`
Expected: a compile error (`search` and `NameMatch` do not exist).

- [ ] **Step 3: Write the implementation** at the top of `src/library/name_search.rs`

```rust
//! Note-name search for the Search view: every note whose name contains the query, ignoring
//! case. Names that start with it come first, then the rest, each group in the tree's natural
//! name order. Pure: no Win32 and no disk.

use super::tree::natural_cmp;
use std::cmp::Ordering;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NameMatch {
    /// As given: relative to the notebook.
    pub path: PathBuf,
    /// The file name without its extension, as the tree shows it.
    pub name: String,
    /// The folder the note is in, relative to the notebook and joined with `\`; `""` at the root.
    pub folder: String,
}

struct Candidate<'a> {
    /// Whether the name only contains the query, rather than starting with it.
    inside: bool,
    name: String,
    folder: String,
    path: &'a PathBuf,
}

/// Prefix matches first, then by name, then by folder (the root first), then by exact path.
fn rank(a: &Candidate<'_>, b: &Candidate<'_>) -> Ordering {
    a.inside
        .cmp(&b.inside)
        .then_with(|| natural_cmp(&a.name, &b.name))
        .then_with(|| natural_cmp(&a.folder, &b.folder))
        .then_with(|| a.path.cmp(b.path))
}

/// The notes whose name contains `query` (trimmed, ignoring case), best first, at most `limit`.
pub fn search(notes: &[PathBuf], query: &str, limit: usize) -> Vec<NameMatch> {
    let query = query.trim().to_lowercase();
    if query.is_empty() || limit == 0 {
        return Vec::new();
    }
    let mut found: Vec<Candidate<'_>> = Vec::new();
    for path in notes {
        let Some(stem) = path.file_stem() else {
            continue;
        };
        let name = stem.to_string_lossy();
        let Some(at) = name.to_lowercase().find(&query) else {
            continue;
        };
        found.push(Candidate {
            inside: at != 0,
            name: name.into_owned(),
            folder: folder_of(path),
            path,
        });
    }
    // Only the best `limit` need a full sort: a one-letter query can match every note.
    if found.len() > limit {
        found.select_nth_unstable_by(limit, rank);
        found.truncate(limit);
    }
    found.sort_by(rank);
    found
        .into_iter()
        .map(|candidate| NameMatch {
            path: candidate.path.clone(),
            name: candidate.name,
            folder: candidate.folder,
        })
        .collect()
}

fn folder_of(path: &Path) -> String {
    path.parent()
        .map(|parent| {
            parent
                .components()
                .map(|component| component.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("\\")
        })
        .unwrap_or_default()
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib library::name_search -- --test-threads=1`
Expected: all 6 tests pass.

Run: `cargo clippy --all-targets -- -D warnings` and `cargo fmt --all -- --check`
Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add src/library/name_search.rs src/library/mod.rs
git commit -m "feat(library): note-name search ranked prefix first, in natural order"
```

---

### Task 5: `sidebar_view` and `sidebar_width` settings

**Files:**
- Modify: `src/config/persisted.rs` (`SidebarView`, `Settings`, `SettingsDelta`, `apply_delta`, `parse` doc, `apply_line`, tests)
- Modify: `src/config/defaults.rs` (defaults, range, `clamp_sidebar_width`, test)
- Modify: `src/config/mod.rs` (re-exports)

**Interfaces:**
- Consumes: nothing new.
- Produces:
  - `config::SidebarView::{Notebook, Search, Favorites, Hidden}` (`Copy`, `Eq`, `Default` = `Notebook`), with `SidebarView::token(self) -> &'static str` giving `notebook`, `search`, `favorites` or `none`
  - `Settings.sidebar_view: SidebarView`, `Settings.sidebar_width: u16`; `SettingsDelta.sidebar_view: Option<SidebarView>`, `SettingsDelta.sidebar_width: Option<u16>`
  - `config::{DEFAULT_SIDEBAR_WIDTH = 260, MIN_SIDEBAR_WIDTH = 180, MAX_SIDEBAR_WIDTH = 480}` (96-DPI pixels) and `config::clamp_sidebar_width(width: u16) -> u16`
  - `fastpad.ini`: `sidebar_view=` is case-insensitive, and anything but the four tokens is a warning; `sidebar_width=` that is not an unsigned integer is a warning, and a number outside 180–480 is pulled into the range without one

- [ ] **Step 1: Write the failing tests**

Add to the `mod tests` block of `src/config/persisted.rs`:

```rust
    #[test]
    fn sidebar_view_accepts_its_four_tokens_and_warns_on_anything_else() {
        // Break caught: a hand-edited "sidebar_view=Search" ignored, or a typo silently closing
        // the panel.
        for (value, view) in [
            ("notebook", SidebarView::Notebook),
            ("Search", SidebarView::Search),
            ("FAVORITES", SidebarView::Favorites),
            ("none", SidebarView::Hidden),
        ] {
            assert_eq!(
                parse(&format!("sidebar_view={value}")).sidebar_view,
                Some(view)
            );
        }
        let delta = parse("sidebar_view=hidden");
        assert_eq!(delta.sidebar_view, None);
        assert_eq!(delta.warnings.len(), 1);
    }

    #[test]
    fn every_sidebar_view_writes_the_token_that_parses_back_to_it() {
        for view in [
            SidebarView::Notebook,
            SidebarView::Search,
            SidebarView::Favorites,
            SidebarView::Hidden,
        ] {
            assert_eq!(
                parse(&format!("sidebar_view={}", view.token())).sidebar_view,
                Some(view)
            );
        }
        assert_eq!(SidebarView::Hidden.token(), "none");
    }

    #[test]
    fn sidebar_width_is_pulled_into_its_range_and_warns_when_not_a_number() {
        // Break caught: a hand-edited sidebar_width=5000 leaving no room for the editor, 0
        // hiding a panel that reads as open, or a typo discarding the other settings.
        assert_eq!(parse("sidebar_width=300").sidebar_width, Some(300));
        assert_eq!(parse("sidebar_width=0").sidebar_width, Some(180));
        let wide = parse("sidebar_width=5000");
        assert_eq!(wide.sidebar_width, Some(480));
        assert!(wide.warnings.is_empty());
        let delta = parse("sidebar_width=-20\nsidebar_width=wide\nfont_size=12");
        assert_eq!(delta.sidebar_width, None);
        assert_eq!(delta.warnings.len(), 2);
        assert_eq!(delta.font_size, Some(12));
    }

    #[test]
    fn sidebar_settings_default_to_the_notebook_view_at_260_pixels_and_apply_from_a_delta() {
        let mut settings = default_settings();
        assert_eq!(settings.sidebar_view, SidebarView::Notebook);
        assert_eq!(settings.sidebar_width, 260);
        settings.apply_delta(&parse("sidebar_view=none\nsidebar_width=200"));
        assert_eq!(
            (settings.sidebar_view, settings.sidebar_width),
            (SidebarView::Hidden, 200)
        );
        settings.apply_delta(&parse("font_size=12"));
        assert_eq!(settings.sidebar_view, SidebarView::Hidden, "absent keys keep theirs");
    }
```

In `src/config/defaults.rs`, `default_settings_match_the_compiled_defaults` gains, before its closing brace:

```rust
        assert_eq!(settings.sidebar_view, SidebarView::Notebook);
        assert_eq!(settings.sidebar_width, 260);
```

and the module gains:

```rust
    #[test]
    fn sidebar_widths_clamp_to_the_range() {
        assert_eq!(clamp_sidebar_width(0), MIN_SIDEBAR_WIDTH);
        assert_eq!(clamp_sidebar_width(179), 180);
        assert_eq!(clamp_sidebar_width(260), 260);
        assert_eq!(clamp_sidebar_width(481), 480);
        assert_eq!(clamp_sidebar_width(u16::MAX), MAX_SIDEBAR_WIDTH);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib config:: -- --test-threads=1`
Expected: a compile error (`SidebarView`, `sidebar_view`, `sidebar_width` and `clamp_sidebar_width` do not exist).

- [ ] **Step 3: Write the implementation**

In `src/config/persisted.rs`:

The first line becomes `use super::defaults::{clamp_sidebar_width, default_settings};`, and after `impl ThemePreference { ... follows_system ... }` add:

```rust
/// Which view the side panel shows. `Hidden` means the panel is closed; the activity bar still
/// shows while notes mode is on.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SidebarView {
    #[default]
    Notebook,
    Search,
    Favorites,
    Hidden,
}

impl SidebarView {
    const ALL: [Self; 4] = [Self::Notebook, Self::Search, Self::Favorites, Self::Hidden];

    /// The `sidebar_view=` value that parses back to this view.
    pub const fn token(self) -> &'static str {
        match self {
            Self::Notebook => "notebook",
            Self::Search => "search",
            Self::Favorites => "favorites",
            Self::Hidden => "none",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|view| value.eq_ignore_ascii_case(view.token()))
    }
}
```

`Settings` gains, after `notes_mode`:

```rust
    /// The side panel's view, or `Hidden` when it is closed. Saved when the view changes.
    pub sidebar_view: SidebarView,
    /// The side panel's width in 96-DPI pixels, within `MIN_SIDEBAR_WIDTH..=MAX_SIDEBAR_WIDTH`.
    /// Saved when a resize drag ends.
    pub sidebar_width: u16,
```

`apply_delta` gains, after the `notes_mode` block:

```rust
        if let Some(sidebar_view) = delta.sidebar_view {
            self.sidebar_view = sidebar_view;
        }
        if let Some(sidebar_width) = delta.sidebar_width {
            self.sidebar_width = sidebar_width;
        }
```

`SettingsDelta` gains, after `notes_mode`:

```rust
    pub sidebar_view: Option<SidebarView>,
    pub sidebar_width: Option<u16>,
```

The doc comment on `parse` becomes:

```rust
/// Parses a hand-written, tolerant `.ini`-style settings source: one `key=value` pair per line: ASCII
/// whitespace is trimmed from both the raw line and the split key/value, blank lines and `#` comment
/// lines are skipped, and exactly `font_face`, `font_size`, `tab_width`, `word_wrap`,
/// `line_numbers`, `theme`, `recovery_interval_seconds`, `restore_session`, `notes_mode`,
/// `sidebar_view` and `sidebar_width` are recognized. `sidebar_view` is `notebook`, `search`,
/// `favorites` or `none` (any case); `sidebar_width` is an unsigned integer in 96-DPI pixels,
/// pulled into 180–480 when it is outside. Every line is handled independently: a line with an
/// unknown key, a value that fails to parse, or no `=` at all records one `SettingWarning` and is
/// otherwise skipped — it never discards, and is never affected by, any other line's outcome.
```

`apply_line` gains, after the `notes_mode` arm:

```rust
        "sidebar_view" => match SidebarView::parse(value) {
            Some(view) => delta.sidebar_view = Some(view),
            None => warn(delta, line_number, key, value),
        },
        // A hand-edited width outside the range is pulled into it rather than rejected.
        "sidebar_width" => match value.parse::<u16>() {
            Ok(width) => delta.sidebar_width = Some(clamp_sidebar_width(width)),
            Err(_) => warn(delta, line_number, key, value),
        },
```

In `src/config/defaults.rs`, the `use` becomes `use super::persisted::{Settings, SidebarView, ThemePreference};`, and after `DEFAULT_NOTES_MODE` add:

```rust
pub const DEFAULT_SIDEBAR_VIEW: SidebarView = SidebarView::Notebook;
/// The side panel's width in 96-DPI pixels: the default, and the range a drag or `fastpad.ini`
/// can set.
pub const DEFAULT_SIDEBAR_WIDTH: u16 = 260;
pub const MIN_SIDEBAR_WIDTH: u16 = 180;
pub const MAX_SIDEBAR_WIDTH: u16 = 480;

/// `width` pulled into `MIN_SIDEBAR_WIDTH..=MAX_SIDEBAR_WIDTH`.
pub const fn clamp_sidebar_width(width: u16) -> u16 {
    if width < MIN_SIDEBAR_WIDTH {
        MIN_SIDEBAR_WIDTH
    } else if width > MAX_SIDEBAR_WIDTH {
        MAX_SIDEBAR_WIDTH
    } else {
        width
    }
}
```

`default_settings` gains, after `notes_mode: DEFAULT_NOTES_MODE,`:

```rust
        sidebar_view: DEFAULT_SIDEBAR_VIEW,
        sidebar_width: DEFAULT_SIDEBAR_WIDTH,
```

and its doc comment ends "..., session restore on, notes mode on, and the side panel showing the Notebook view at 260 pixels."

`src/config/mod.rs` becomes:

```rust
pub mod defaults;
pub mod persisted;

pub use defaults::{
    DEFAULT_SIDEBAR_WIDTH, MAX_SIDEBAR_WIDTH, MIN_SIDEBAR_WIDTH, clamp_sidebar_width,
    default_settings,
};
pub use persisted::{
    SettingWarning, Settings, SettingsDelta, SidebarView, ThemePreference, load, parse,
    save_setting, save_setting_to,
};
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib config:: -- --test-threads=1`
Expected: all pass, including the 4 new tests in `config::persisted` and `sidebar_widths_clamp_to_the_range`.

Run: `cargo clippy --all-targets -- -D warnings` and `cargo fmt --all -- --check`
Expected: clean. (The one `Settings { .. }` literal outside `config`, in `main_window.rs`'s tests, uses `..reloaded` and needs no change.)

- [ ] **Step 5: Commit**

```bash
git add src/config
git commit -m "feat(config): sidebar_view and sidebar_width settings"
```

---

### Task 6: The sidebar shell: layout slot, title-strip offset, activity bar, empty panel, resize, view commands

**Files:**
- Create: `src/window/side_panel.rs`, `src/window/activity_bar.rs`, `src/window/tooltip.rs`
- Modify:
  - `src/bootstrap.rs` (`fastpad.ini` is read before the window is created)
  - `src/window/mod.rs` (`pub(crate) mod activity_bar; pub(crate) mod side_panel; pub(crate) mod tooltip;`)
  - `src/app.rs` (`pub(crate) sidebar: Option<crate::window::side_panel::Sidebar>` and `pub(crate) preloaded_settings_warnings: Option<Vec<crate::config::SettingWarning>>`, both initialized to `None`)
  - `src/window/main_window.rs` (layout, wiring, commands, `ui_fonts`, `load_settings`, tests)
  - `src/window/titlebar.rs` (`create_ui_font`, `strip_height`, `calculate_with_offset`, the `sidebar` caption rect, painting right of the sidebar)
  - `src/window/commands.rs`, `src/window/menus.rs`, `src/window/command_palette.rs` (the four view commands)
  - `src/window/menu_band.rs`, `src/window/find_bar.rs`, `src/window/name_box.rs`, `src/window/accessibility.rs` (a left offset for everything right of the sidebar)
  - `src/window/palette.rs` (`Palette::panel_background`)

**Interfaces:**
- Consumes:
  - Task 5: `crate::config::SidebarView::{Notebook, Search, Favorites, Hidden}` (`Clone, Copy, Debug, Eq, PartialEq`), `SidebarView::token`, `Settings.sidebar_view`, `Settings.sidebar_width: u16`, and `crate::config::defaults::{DEFAULT_SIDEBAR_WIDTH, MIN_SIDEBAR_WIDTH, MAX_SIDEBAR_WIDTH}` as `u16`.
  - Task 1: the `CommandId` enum ending in `NoteDelete = 175`, and the `COMMANDS` and `ENTRIES` tables as Task 1 left them.
- Produces (the contract):
  - `App.sidebar: Option<side_panel::Sidebar>`, with the fields `bar: HWND`, `panel: HWND`, `tooltip: Option<Tooltip>` and `bar_state: BarState` public to the crate. Tasks 10, 12 and 13 add `notebook`, `favorites`, `search` and `bar_focus`.
  - `side_panel::{create, left_edge, layout, show_view, toggle, refresh, active_tab_changed, notes_mode_changed, current_view}` with the contract's signatures.
  - `TitleBarLayout::calculate_with_offset(client, dpi, tab_count, scroll, preview_buttons, left)` and `TitleBarLayout.sidebar: Rect` (caption).
  - `CommandId::{ToggleSidebar = 176, ShowNotebookView = 177, ShowSearchView = 178, ShowFavoritesView = 179}`.
  - `tooltip::Tooltip::create(owner) -> Option<Tooltip>` and `Tooltip::set_tool(&self, id, rect, text)`. An empty `text` removes the tool (`TTM_DELTOOLW` only).
  - The one font API: `titlebar::create_ui_font(pixel_height: i32, face: &str, weight: i32, italic: bool) -> HFONT` (the old private `create_font` becomes a wrapper around it), `side_panel::UiFonts { text, bold, italic, glyph, bar_glyph }` cached per DPI on the `Sidebar`, and `main_window::ui_fonts(hwnd) -> UiFonts`.
  - The view seam: `side_panel::ViewPaint { hdc, client, palette, background, fonts, dpi, focused }`, `side_panel::view_paint(main, panel, hdc, client) -> ViewPaint`, `side_panel::draw_text(hdc, text, rect, font, color, flags) -> i32` (an `unsafe fn`), `side_panel::windows(hwnd) -> Option<(HWND, HWND)>` (bar, panel), and `side_panel::PanelView::{Notebook, Search, Favorites}` with the private dispatch `paint_view(main, view, &ViewPaint)`, `view_mouse(main, view, panel, message, wparam, lparam) -> Option<LRESULT>`, `view_key(main, view, panel, message, wparam, lparam) -> Option<LRESULT>` and `header_is_caption(main, view, panel, x, y) -> bool`. Tasks 10 and 12 replace their arms.
- Produces (also used by later tasks):
  - `side_panel::sidebar_widths(client_width: i32, dpi: u32, view_open: bool, width_96: u16) -> (i32, i32)` and `side_panel::drag_width_96(panel_px: i32, dpi: u32) -> u16`.
  - `side_panel::{paint_buffered, point_of, register_child_class, with_bar_state}` and the constants `ACTIVITY_WIDTH_96 = 44`, `EDITOR_MIN_WIDTH_96 = 320`, `HEADER_HEIGHT_96 = 38`, `GRIP_WIDTH_96 = 4`.
  - `activity_bar::{ActivityButton, BarState, button_rects(client: RECT, dpi: u32) -> [RECT; 4], button_at, activate}`, and the private Settings click handler `activity_bar::open_settings(main)`, which Task 12 repoints.
  - `titlebar::strip_height(dpi) -> i32`, `Palette::panel_background()`, `CommandId::is_sidebar()` and `menus::set_sidebar_enabled`.
  - `main_window::{change_setting, open_command_palette}` become `pub(crate)`.

Notes for the implementer:
- `fastpad.ini` is read once, in `bootstrap::run`, before the window is created (Step 9). The sidebar is created in `initialize_editor_with` from those settings, so the first frame already has the saved view and width and nothing moves afterwards. `WM_FASTPAD_LOAD_SETTINGS` applies the preloaded settings and reports their warnings instead of reading the file again. In-process tests don't run `bootstrap::run`, so they keep reading `fastpad.ini` in `WM_FASTPAD_LOAD_SETTINGS`.
- The Settings button opens the normal command palette here. Task 12 repoints `activity_bar::open_settings` to `main_window::open_settings_palette`.
- Keyboard use of the activity bar (Up, Down, Enter, Space) and F6 are Task 13.

- [ ] **Step 1: Write the failing tests**

In `src/window/titlebar.rs` tests:

```rust
    #[test]
    fn a_sidebar_offset_moves_the_tabs_right_and_keeps_its_strip_as_caption() {
        // Break caught: tabs painted under the activity bar, or a sidebar top strip that no
        // longer drags the window or resizes it from the top edge.
        let client = Size::new(1200, 800);
        let plain = TitleBarLayout::calculate_with_preview(client, 96, 3, 0, false);
        assert_eq!(
            TitleBarLayout::calculate_with_offset(client, 96, 3, 0, false, 0),
            plain
        );
        let layout = TitleBarLayout::calculate_with_offset(client, 96, 3, 0, false, 304);
        assert_eq!(layout.tabs.left, 304);
        assert_eq!(layout.tab(0).left, 304);
        assert_eq!(layout.tab(1).left, layout.tab(0).right);
        assert_eq!(layout.sidebar, Rect::new(0, 0, 304, layout.height));
        assert_eq!(layout.close, plain.close);
        assert!(layout.drag_region.left >= layout.tab(2).right);
        assert_eq!(
            layout.hit_test(super::Point::new(20, layout.height / 2)),
            HitTarget::Caption
        );
        assert_eq!(layout.hit_test(layout.tab(0).center()), HitTarget::Tab(0));
        assert_eq!(
            layout.frame_hit_test(super::Point::new(20, 0), false),
            HitTarget::ResizeTop
        );
        assert_eq!(
            layout.frame_hit_test(super::Point::new(0, 0), false),
            HitTarget::ResizeTopLeft
        );
    }

    #[test]
    fn crowded_tabs_behind_a_sidebar_scroll_inside_the_narrower_viewport() {
        let client = Size::new(1200, 800);
        let layout = TitleBarLayout::calculate_with_offset(client, 96, 30, 0, false, 304);
        assert!(layout.max_scroll > 0);
        let scrolled = TitleBarLayout::calculate_with_offset(
            client,
            96,
            30,
            layout.scroll_to_reveal(29),
            false,
            304,
        );
        assert!(scrolled.tab(29).left >= scrolled.tabs.left);
        assert!(scrolled.tab(29).right <= scrolled.tabs.right);
        assert_eq!(scrolled.scroll_bar.unwrap().left, 304);
        // An offset wider than the strip leaves an empty viewport, never a negative one.
        let squeezed =
            TitleBarLayout::calculate_with_offset(Size::new(300, 800), 96, 2, 0, false, 5000);
        assert!(squeezed.tabs.left <= squeezed.tabs.right);
        assert_eq!(squeezed.close.right, 300);
    }
```

In `src/window/commands.rs` tests:

```rust
    #[test]
    fn sidebar_commands_have_their_reserved_numbers_and_need_no_document() {
        // Break caught: a renumbered view command, which would break the accelerator table and
        // any WM_COMMAND an outside test posts by number.
        assert_eq!(CommandId::try_from(176), Ok(CommandId::ToggleSidebar));
        assert_eq!(CommandId::try_from(177), Ok(CommandId::ShowNotebookView));
        assert_eq!(CommandId::try_from(178), Ok(CommandId::ShowSearchView));
        assert_eq!(CommandId::try_from(179), Ok(CommandId::ShowFavoritesView));
        for command in [
            CommandId::ToggleSidebar,
            CommandId::ShowNotebookView,
            CommandId::ShowSearchView,
            CommandId::ShowFavoritesView,
        ] {
            assert!(!command.needs_document());
            assert!(command.is_sidebar());
        }
        assert!(!CommandId::Save.is_sidebar());
    }
```

In `src/window/menus.rs` tests, raise the expected length in `shortcut_and_menu_commands_share_command_ids`'s `assert_eq!(specs.len(), ..)` by 3. At the end of `tab_zoom_and_direction_shortcuts_are_bound` add:

```rust
        assert_eq!(
            bound(FCONTROL, u16::from(b'B')),
            Some(CommandId::ToggleSidebar)
        );
        assert_eq!(
            bound(FCONTROL | FSHIFT, u16::from(b'E')),
            Some(CommandId::ShowNotebookView)
        );
        assert_eq!(
            bound(FCONTROL, u16::from(b'K')),
            Some(CommandId::ShowSearchView)
        );
```

Then add:

```rust
    #[test]
    fn the_view_menu_toggles_the_sidebar_and_grays_it_without_notes_mode() {
        // Break caught: a Sidebar entry that stays enabled with notes mode off, where it does
        // nothing, or no entry at all.
        use super::{MenuBar, set_sidebar_enabled};
        use windows_sys::Win32::UI::WindowsAndMessaging::{GetMenuState, MF_BYCOMMAND, MF_GRAYED};
        let bar = MenuBar::create().unwrap();
        let view = bar.dropdown(crate::window::menu_band::VIEW_MENU_INDEX);
        let state =
            || unsafe { GetMenuState(view, CommandId::ToggleSidebar as u32, MF_BYCOMMAND) };
        assert_ne!(state(), u32::MAX, "the View menu has a Sidebar entry");
        set_sidebar_enabled(view, false);
        assert_ne!(state() & MF_GRAYED, 0);
        set_sidebar_enabled(view, true);
        assert_eq!(state() & MF_GRAYED, 0);
    }
```

In `src/window/command_palette.rs` tests (the existing `every_command_except_tab_positions_and_the_palette_is_listed_once` also fails until the entries exist):

```rust
    #[test]
    fn the_sidebar_commands_are_listed_with_their_shortcuts() {
        assert_eq!(labels("sidebar")[0], "View: Toggle sidebar");
        assert_eq!(labels("show search")[0], "View: Show search");
        assert_eq!(
            shortcut_text(CommandId::ToggleSidebar).as_deref(),
            Some("Ctrl+B")
        );
        assert_eq!(
            shortcut_text(CommandId::ShowNotebookView).as_deref(),
            Some("Ctrl+Shift+E")
        );
        assert_eq!(shortcut_text(CommandId::ShowFavoritesView), None);
    }
```

In `src/window/menu_band.rs` tests, replace `headings_sit_side_by_side_inside_the_band` and `hit_testing_finds_the_heading_under_the_pointer` with versions that pass a left edge:

```rust
    #[test]
    fn headings_sit_side_by_side_inside_the_band() {
        let headings = heading_rects(&[20, 30, 40, 25], 0, 32, 96);
        assert_eq!(headings.len(), 4);
        assert_eq!(headings[0].left, 4);
        assert_eq!(headings[0].right, 4 + 20 + 20);
        for pair in headings.windows(2) {
            assert_eq!(pair[0].right, pair[1].left);
        }
        assert!(
            headings
                .iter()
                .all(|rect| rect.top == 32 && rect.bottom == 32 + band_height(96))
        );
        assert_eq!(heading_rects(&[20], 0, 0, 192)[0].right, 8 + 20 + 40);
        // Break caught: headings drawn under the sidebar instead of at the editor area's edge.
        assert_eq!(heading_rects(&[20], 304, 0, 96)[0].left, 304 + 4);
    }

    #[test]
    fn hit_testing_finds_the_heading_under_the_pointer() {
        let headings = heading_rects(&[20, 30, 40, 25], 0, 32, 96);
        assert_eq!(heading_at(&headings, 5, 33), Some(0));
        assert_eq!(heading_at(&headings, headings[1].left, 40), Some(1));
        assert_eq!(heading_at(&headings, 5, 31), None);
        assert_eq!(heading_at(&headings, headings[3].right, 40), None);
    }
```

In `src/window/activity_bar.rs`, which is new, write the tests module now and the code in Step 7:

```rust
#[cfg(test)]
mod tests {
    use super::{ActivityButton, button_at, button_rects};
    use crate::config::SidebarView;
    use windows_sys::Win32::Foundation::RECT;

    fn edges(rect: RECT) -> (i32, i32, i32, i32) {
        (rect.left, rect.top, rect.right, rect.bottom)
    }

    fn bar(width: i32, height: i32) -> RECT {
        RECT {
            left: 0,
            top: 0,
            right: width,
            bottom: height,
        }
    }

    #[test]
    fn buttons_stack_below_the_caption_strip_with_settings_at_the_bottom() {
        // Break caught: a first button inside the caption strip (it could not be clicked, the
        // strip drags the window), or Settings drawn over Favorites in a short window.
        // At 96 DPI the title strip is 40 px tall.
        let rects = button_rects(bar(44, 700), 96);
        assert_eq!(edges(rects[0]), (0, 40, 44, 84));
        assert_eq!(edges(rects[1]), (0, 84, 44, 128));
        assert_eq!(edges(rects[2]), (0, 128, 44, 172));
        assert_eq!(edges(rects[3]), (0, 656, 44, 700));
        let short = button_rects(bar(44, 150), 96);
        assert_eq!(short[3].top, short[2].bottom);
        assert_eq!(button_at(&rects, 10, 50), Some(ActivityButton::Notebook));
        assert_eq!(button_at(&rects, 10, 130), Some(ActivityButton::Favorites));
        assert_eq!(button_at(&rects, 10, 690), Some(ActivityButton::Settings));
        assert_eq!(button_at(&rects, 10, 20), None);
        assert_eq!(button_at(&rects, 10, 300), None);
    }

    #[test]
    fn each_view_button_names_its_view_and_settings_names_none() {
        assert_eq!(ActivityButton::Notebook.view(), Some(SidebarView::Notebook));
        assert_eq!(ActivityButton::Search.view(), Some(SidebarView::Search));
        assert_eq!(ActivityButton::Favorites.view(), Some(SidebarView::Favorites));
        assert_eq!(ActivityButton::Settings.view(), None);
        for (index, button) in ActivityButton::ALL.into_iter().enumerate() {
            assert_eq!(button.index(), index);
        }
    }
}
```

In `src/window/side_panel.rs`, which is new, the pure tests module:

```rust
#[cfg(test)]
mod tests {
    use super::{PanelView, drag_width_96, sidebar_widths};
    use crate::config::SidebarView;
    use crate::window::palette::Palette;

    #[test]
    fn the_panel_gives_way_before_the_editor_minimum_and_scales_with_dpi() {
        // Break caught: an editor squeezed below 320 px by a wide panel, a negative panel width
        // on a tiny window, or sizes that ignore the monitor's DPI.
        assert_eq!(sidebar_widths(1280, 96, true, 260), (44, 260));
        assert_eq!(sidebar_widths(1280, 96, false, 260), (44, 0));
        assert_eq!(sidebar_widths(1920, 144, true, 260), (66, 390));
        assert_eq!(sidebar_widths(600, 96, true, 260), (44, 236));
        assert_eq!(sidebar_widths(300, 96, true, 260), (44, 0));
        assert_eq!(sidebar_widths(20, 96, true, 260), (20, 0));
        assert_eq!(sidebar_widths(-5, 96, true, 260), (0, 0));
        // A hand-edited width outside the range is clamped, never trusted.
        assert_eq!(sidebar_widths(1280, 96, true, 100), (44, 180));
        assert_eq!(sidebar_widths(1280, 96, true, 900), (44, 480));
    }

    #[test]
    fn a_dragged_width_is_stored_at_96_dpi_inside_the_range() {
        assert_eq!(drag_width_96(300, 96), 300);
        assert_eq!(drag_width_96(390, 144), 260);
        assert_eq!(drag_width_96(100, 96), 180);
        assert_eq!(drag_width_96(-40, 96), 180);
        assert_eq!(drag_width_96(2000, 96), 480);
    }

    #[test]
    fn the_panel_shade_sits_between_the_strip_and_the_editor() {
        let palette = Palette {
            strip_background: 0x0020_4060,
            editor_background: 0x00a0_c0e0,
            ..Palette::neutral()
        };
        assert_eq!(palette.panel_background(), 0x0060_80a0);
    }

    #[test]
    fn every_open_view_has_a_panel_view_and_a_header_title() {
        assert_eq!(
            PanelView::of(SidebarView::Notebook),
            Some(PanelView::Notebook)
        );
        assert_eq!(PanelView::of(SidebarView::Search), Some(PanelView::Search));
        assert_eq!(
            PanelView::of(SidebarView::Favorites),
            Some(PanelView::Favorites)
        );
        assert_eq!(PanelView::of(SidebarView::Hidden), None);
        assert_eq!(PanelView::Favorites.title(), "FAVORITES");
    }
}
```

In the `src/window/main_window.rs` tests, add these helpers and tests:

```rust
    fn sidebar_windows(hwnd: HWND) -> (HWND, HWND) {
        let sidebar = app_mut(hwnd)
            .sidebar
            .as_ref()
            .expect("notes mode shows the sidebar");
        (sidebar.bar, sidebar.panel)
    }

    fn client_lparam(x: i32, y: i32) -> super::LPARAM {
        ((y as u32) << 16 | (x as u32 & 0xffff)) as super::LPARAM
    }

    /// `window`'s client point `x`, `y` as a screen-coordinate `lParam`, as WM_NCHITTEST gets it.
    fn screen_lparam(window: HWND, x: i32, y: i32) -> super::LPARAM {
        let mut point = windows_sys::Win32::Foundation::POINT { x, y };
        unsafe { windows_sys::Win32::Graphics::Gdi::ClientToScreen(window, &mut point) };
        client_lparam(point.x, point.y)
    }

    fn client_size(window: HWND) -> (i32, i32) {
        let mut rect = RECT::default();
        unsafe { GetClientRect(window, &mut rect) };
        (rect.right, rect.bottom)
    }

    /// `child`'s left edge in `parent`'s client coordinates.
    fn left_of(child: HWND, parent: HWND) -> i32 {
        use windows_sys::Win32::UI::WindowsAndMessaging::GetWindowRect;
        let mut rect = RECT::default();
        let mut origin = windows_sys::Win32::Foundation::POINT::default();
        unsafe {
            GetWindowRect(child, &mut rect);
            windows_sys::Win32::Graphics::Gdi::ClientToScreen(parent, &mut origin);
        }
        rect.left - origin.x
    }

    /// The test window is never shown, so check the child's own style bit.
    fn is_shown(window: HWND) -> bool {
        (unsafe { GetWindowLongPtrW(window, super::GWL_STYLE) }) as u32 & super::WS_VISIBLE != 0
    }

    fn click(window: HWND, x: i32, y: i32) {
        use windows_sys::Win32::UI::WindowsAndMessaging::{WM_LBUTTONDOWN, WM_LBUTTONUP};
        unsafe {
            SendMessageW(window, WM_LBUTTONDOWN, 0, client_lparam(x, y));
            SendMessageW(window, WM_LBUTTONUP, 0, client_lparam(x, y));
        }
    }

    fn button_center(hwnd: HWND, button: crate::window::activity_bar::ActivityButton) -> (i32, i32) {
        let (bar, _) = sidebar_windows(hwnd);
        let (width, height) = client_size(bar);
        let client = RECT {
            left: 0,
            top: 0,
            right: width,
            bottom: height,
        };
        let dpi = unsafe { GetDpiForWindow(bar) }.max(96);
        let rect = crate::window::activity_bar::button_rects(client, dpi)[button.index()];
        ((rect.left + rect.right) / 2, (rect.top + rect.bottom) / 2)
    }

    /// Resizes the window so its client area is `client_width` wide.
    fn set_client_width(hwnd: HWND, client_width: i32) {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GetWindowRect, SWP_NOMOVE, SWP_NOZORDER, SetWindowPos,
        };
        let mut frame = RECT::default();
        unsafe { GetWindowRect(hwnd, &mut frame) };
        let border = (frame.right - frame.left) - client_size(hwnd).0;
        unsafe {
            SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                0,
                0,
                client_width + border,
                frame.bottom - frame.top,
                SWP_NOMOVE | SWP_NOZORDER,
            );
        }
    }

    /// A scratch `fastpad.ini` holding only a comment, which settings saves go to.
    fn settings_scratch(label: &str) -> (RecoveryScratch, PathBuf) {
        let scratch = RecoveryScratch::new(label);
        let ini = scratch.path().join("fastpad.ini");
        std::fs::write(&ini, "# kept\r\n").unwrap();
        super::save_settings_to(Some(ini.clone()));
        (scratch, ini)
    }

    #[test]
    fn with_notes_mode_off_there_is_no_sidebar_and_nothing_moves() {
        // Break caught: an activity bar, or a gap where it would be, with notes mode off, where
        // the layout must stay exactly what it was before the sidebar existed.
        let _scintilla = load_native_scintilla();
        let mut app = make_app();
        app.settings.notes_mode = false;
        let window = ProductionWindow::new(app);
        let editor = install_test_editor(&window);
        assert!(app_mut(window.hwnd).sidebar.is_none());
        assert_eq!(crate::window::side_panel::left_edge(window.hwnd), 0);
        assert_eq!(super::title_layout(window.hwnd).tab(0).left, 0);
        assert_eq!(left_of(editor.hwnd(), window.hwnd), 0);
        assert_eq!(
            crate::window::side_panel::current_view(window.hwnd),
            crate::config::SidebarView::Hidden
        );
        execute_command(window.hwnd, CommandId::ToggleSidebar);
        assert!(app_mut(window.hwnd).sidebar.is_none());
        execute_command(window.hwnd, CommandId::CommandPalette);
        assert!(
            app_mut(window.hwnd)
                .command_palette
                .as_ref()
                .unwrap()
                .shown()
                .iter()
                .all(|entry| !entry.command.is_sidebar())
        );
        super::close_command_palette(window.hwnd, false);

        app_mut(window.hwnd).settings.notes_mode = true;
        crate::window::side_panel::notes_mode_changed(window.hwnd, true);
        let (bar, _) = sidebar_windows(window.hwnd);
        let left = crate::window::side_panel::left_edge(window.hwnd);
        assert!(left > 0);
        assert_eq!(left_of(editor.hwnd(), window.hwnd), left);

        app_mut(window.hwnd).settings.notes_mode = false;
        crate::window::side_panel::notes_mode_changed(window.hwnd, false);
        assert_eq!(unsafe { IsWindow(bar) }, 0);
        assert_eq!(crate::window::side_panel::left_edge(window.hwnd), 0);
        assert_eq!(left_of(editor.hwnd(), window.hwnd), 0);
    }

    #[test]
    fn the_sidebar_takes_the_left_edge_and_everything_else_starts_right_of_it() {
        // Break caught: tabs, the find bar, the menu band or the editor still starting at x = 0,
        // under the activity bar and panel.
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        let (bar, panel) = sidebar_windows(window.hwnd);
        let dpi = unsafe { GetDpiForWindow(window.hwnd) }.max(96);
        let (width, height) = client_size(window.hwnd);
        let saved = app_mut(window.hwnd).settings.sidebar_width;
        let (activity, panel_width) =
            crate::window::side_panel::sidebar_widths(width, dpi, true, saved);
        assert_eq!(client_size(bar), (activity, height));
        assert_eq!(client_size(panel), (panel_width, height));
        assert_eq!(left_of(panel, window.hwnd), activity);
        let left = activity + panel_width;
        assert_eq!(crate::window::side_panel::left_edge(window.hwnd), left);
        assert_eq!(super::title_layout(window.hwnd).tab(0).left, left);
        assert_eq!(left_of(editor.hwnd(), window.hwnd), left);
        assert_eq!(client_size(editor.hwnd()).0, width - left);
        assert_eq!(
            app_mut(window.hwnd)
                .sidebar
                .as_ref()
                .unwrap()
                .tooltip
                .unwrap()
                .tool_count(),
            4
        );
        // An empty text removes a tool instead of showing an empty tip.
        let tooltip = app_mut(window.hwnd).sidebar.as_ref().unwrap().tooltip.unwrap();
        tooltip.set_tool(9, RECT::default(), "extra");
        assert_eq!(tooltip.tool_count(), 5);
        tooltip.set_tool(9, RECT::default(), "");
        assert_eq!(tooltip.tool_count(), 4);

        execute_command(window.hwnd, CommandId::Find);
        let find = app_mut(window.hwnd).find_bar.as_ref().unwrap().panel_hwnd();
        assert_eq!(left_of(find, window.hwnd), left);
        assert_eq!(client_size(find).0, width - left);
        super::close_find_bar(window.hwnd);

        execute_command(window.hwnd, CommandId::CommandPalette);
        let palette = app_mut(window.hwnd)
            .command_palette
            .as_ref()
            .unwrap()
            .panel_hwnd();
        assert!(left_of(palette, window.hwnd) >= left);
        super::close_command_palette(window.hwnd, false);

        unsafe {
            SendMessageW(
                window.hwnd,
                super::WM_SYSCOMMAND,
                super::SC_KEYMENU as usize,
                0,
            )
        };
        assert_eq!(
            super::menu_headings(window.hwnd)[0].left,
            left + crate::window::panel::scale(4, dpi)
        );
        unsafe {
            SendMessageW(
                window.hwnd,
                super::WM_SYSCOMMAND,
                super::SC_KEYMENU as usize,
                0,
            )
        };
    }

    #[test]
    fn the_sidebar_top_strip_and_panel_header_are_caption() {
        // Break caught: child windows under the title row that swallow the caption, so the
        // window can no longer be dragged or top-resized there, or a lost left-border resize.
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GetWindowRect, HTCAPTION, HTCLIENT, HTLEFT, HTTRANSPARENT, WM_NCHITTEST,
        };
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        let (bar, panel) = sidebar_windows(window.hwnd);
        let dpi = unsafe { GetDpiForWindow(window.hwnd) }.max(96);
        let layout = super::title_layout(window.hwnd);
        let hit = |target: HWND, x: i32, y: i32| unsafe {
            SendMessageW(target, WM_NCHITTEST, 0, screen_lparam(target, x, y))
        };
        // Below the top resize band and above the first button.
        let strip_y = layout.height - 2;
        assert!(strip_y >= layout.resize_border);
        let bar_x = client_size(bar).0 / 2;
        assert_eq!(hit(bar, bar_x, strip_y), HTTRANSPARENT as LRESULT);
        assert_eq!(hit(window.hwnd, bar_x, strip_y), HTCAPTION as LRESULT);
        let (button_x, button_y) =
            button_center(window.hwnd, crate::window::activity_bar::ActivityButton::Notebook);
        assert_eq!(hit(bar, button_x, button_y), HTCLIENT as LRESULT);

        let header_y = layout.resize_border + 2;
        let header = crate::window::panel::scale(crate::window::side_panel::HEADER_HEIGHT_96, dpi);
        assert!(header_y < header);
        let panel_x = client_size(panel).0 / 2;
        assert_eq!(hit(panel, panel_x, header_y), HTTRANSPARENT as LRESULT);
        assert_eq!(
            hit(window.hwnd, left_of(panel, window.hwnd) + panel_x, header_y),
            HTCAPTION as LRESULT
        );
        // Below the header, and on the resize edge, the panel keeps its own input.
        assert_eq!(hit(panel, panel_x, header + 10), HTCLIENT as LRESULT);
        assert_eq!(
            hit(panel, client_size(panel).0 - 1, header_y),
            HTCLIENT as LRESULT
        );

        // The left border is outside the client area, so no child covers it.
        let mut frame = RECT::default();
        let mut origin = windows_sys::Win32::Foundation::POINT::default();
        unsafe {
            GetWindowRect(window.hwnd, &mut frame);
            windows_sys::Win32::Graphics::Gdi::ClientToScreen(window.hwnd, &mut origin);
        }
        if origin.x > frame.left {
            let border = client_lparam(
                frame.left + (origin.x - frame.left) / 2,
                origin.y + client_size(window.hwnd).1 / 2,
            );
            assert_eq!(
                unsafe { SendMessageW(window.hwnd, WM_NCHITTEST, 0, border) },
                HTLEFT as LRESULT
            );
        }
    }

    #[test]
    fn clicking_the_active_view_icon_closes_the_sidebar_panel_and_saves_none() {
        // Break caught: an icon that only ever opens its view, so the mouse cannot close the
        // panel, or a closed panel that reopens after a restart.
        use crate::config::SidebarView;
        use crate::window::activity_bar::ActivityButton;
        let _scintilla = load_native_scintilla();
        let (_scratch, ini) = settings_scratch("sidebar-click");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        let (bar, panel) = sidebar_windows(window.hwnd);
        let activity = client_size(bar).0;
        let view = || crate::window::side_panel::current_view(window.hwnd);
        let saved = || std::fs::read_to_string(&ini).unwrap();

        let (x, y) = button_center(window.hwnd, ActivityButton::Notebook);
        click(bar, x, y);
        assert_eq!(view(), SidebarView::Hidden);
        assert!(!is_shown(panel));
        assert_eq!(crate::window::side_panel::left_edge(window.hwnd), activity);
        assert_eq!(saved(), "# kept\r\nsidebar_view=none\r\n");

        click(bar, x, y);
        assert_eq!(view(), SidebarView::Notebook);
        assert!(is_shown(panel));
        assert_eq!(saved(), "# kept\r\nsidebar_view=notebook\r\n");

        let (x, y) = button_center(window.hwnd, ActivityButton::Search);
        click(bar, x, y);
        assert_eq!(view(), SidebarView::Search);
        assert_eq!(saved(), "# kept\r\nsidebar_view=search\r\n");

        // Settings opens the command palette; Task 12 narrows it to the settings commands.
        let (x, y) = button_center(window.hwnd, ActivityButton::Settings);
        click(bar, x, y);
        assert!(
            app_mut(window.hwnd)
                .command_palette
                .as_ref()
                .unwrap()
                .is_visible()
        );
        assert_eq!(view(), SidebarView::Search);
        super::save_settings_to(None);
    }

    #[test]
    fn ctrl_b_toggles_the_sidebar_back_to_the_last_view_and_saves_each_change() {
        // Break caught: a toggle that forgets which view was open, a shortcut that never
        // reaches its command, or a change lost on restart.
        use crate::config::SidebarView;
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
            GetKeyboardState, SetKeyboardState, VK_CONTROL, VK_SHIFT,
        };
        use windows_sys::Win32::UI::WindowsAndMessaging::{MSG, WM_KEYDOWN};
        let _scintilla = load_native_scintilla();
        let (_scratch, ini) = settings_scratch("sidebar-ctrl-b");
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        let identity = unsafe { super::window_identity(window.hwnd).unwrap() };
        let press = |key: u8, shift: bool| {
            let mut keys = [0u8; 256];
            unsafe { GetKeyboardState(keys.as_mut_ptr()) };
            let original = keys;
            keys[VK_CONTROL as usize] = 0x80;
            keys[VK_SHIFT as usize] = if shift { 0x80 } else { 0 };
            unsafe { SetKeyboardState(keys.as_ptr()) };
            let message = MSG {
                hwnd: editor.hwnd(),
                message: WM_KEYDOWN,
                wParam: usize::from(key),
                ..Default::default()
            };
            let translated =
                unsafe { super::translate_accelerator(window.hwnd, &identity, &message) };
            unsafe { SetKeyboardState(original.as_ptr()) };
            translated
        };
        let view = || crate::window::side_panel::current_view(window.hwnd);
        let saved = || std::fs::read_to_string(&ini).unwrap();

        assert_eq!(view(), SidebarView::Notebook);
        assert!(press(b'B', false));
        assert_eq!(view(), SidebarView::Hidden);
        assert_eq!(saved(), "# kept\r\nsidebar_view=none\r\n");
        assert!(press(b'K', false));
        assert_eq!(view(), SidebarView::Search);
        assert!(press(b'B', false));
        assert!(press(b'B', false));
        assert_eq!(view(), SidebarView::Search, "Ctrl+B reopens the last view");
        assert!(press(b'E', true));
        assert_eq!(view(), SidebarView::Notebook);
        execute_command(window.hwnd, CommandId::ShowFavoritesView);
        assert_eq!(view(), SidebarView::Favorites);
        assert_eq!(saved(), "# kept\r\nsidebar_view=favorites\r\n");
        super::save_settings_to(None);
    }

    #[test]
    fn a_narrow_window_squeezes_the_panel_without_saving_it() {
        // Break caught: an editor pushed below its 320 px minimum, a negative panel width, or a
        // squeeze written to fastpad.ini so the panel stays narrow once the window widens again.
        let _scintilla = load_native_scintilla();
        let (_scratch, ini) = settings_scratch("sidebar-squeeze");
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        let (_, panel) = sidebar_windows(window.hwnd);
        let dpi = unsafe { GetDpiForWindow(window.hwnd) }.max(96);
        let scale = |value| crate::window::panel::scale(value, dpi);

        set_client_width(window.hwnd, scale(44 + 320 + 200));
        let (width, _) = client_size(window.hwnd);
        let (activity, squeezed) = crate::window::side_panel::sidebar_widths(width, dpi, true, 260);
        assert_eq!(squeezed, width - activity - scale(320));
        assert!(squeezed > 0 && squeezed < scale(260), "the window squeezes the panel");
        assert_eq!(client_size(panel).0, squeezed);
        assert_eq!(
            crate::window::side_panel::left_edge(window.hwnd),
            activity + squeezed
        );
        assert_eq!(client_size(editor.hwnd()).0, scale(320).max(width - activity - squeezed));
        assert_eq!(app_mut(window.hwnd).settings.sidebar_width, 260);

        set_client_width(window.hwnd, scale(1200));
        assert_eq!(client_size(panel).0, scale(260));

        // Narrower than the activity bar and the editor minimum: the panel hides, never goes
        // below zero.
        set_client_width(window.hwnd, scale(300));
        assert!(!is_shown(panel));
        assert_eq!(
            crate::window::side_panel::left_edge(window.hwnd),
            scale(44).min(client_size(window.hwnd).0)
        );
        assert_eq!(app_mut(window.hwnd).settings.sidebar_width, 260);
        assert_eq!(std::fs::read_to_string(&ini).unwrap(), "# kept\r\n");
        super::save_settings_to(None);
    }

    #[test]
    fn dragging_the_sidebar_edge_resizes_it_and_saves_the_width_once_on_release() {
        // Break caught: a drag that writes fastpad.ini on every mouse move, never saves, ignores
        // the 180–480 range, or an edge double-click that leaves a custom width in place.
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE,
        };
        let _scintilla = load_native_scintilla();
        let (_scratch, ini) = settings_scratch("sidebar-drag");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        let (_, panel) = sidebar_windows(window.hwnd);
        let dpi = unsafe { GetDpiForWindow(window.hwnd) }.max(96);
        let scale = |value| crate::window::panel::scale(value, dpi);
        set_client_width(window.hwnd, scale(1400));
        let saved = || std::fs::read_to_string(&ini).unwrap();
        let send = |message, x: i32| unsafe {
            SendMessageW(panel, message, 0, client_lparam(x, client_size(panel).1 / 2));
        };

        send(WM_LBUTTONDOWN, client_size(panel).0 - 1);
        send(WM_MOUSEMOVE, scale(300));
        assert_eq!(client_size(panel).0, scale(300));
        assert_eq!(saved(), "# kept\r\n", "nothing is saved mid-drag");
        send(WM_LBUTTONUP, scale(300));
        assert_eq!(saved(), "# kept\r\nsidebar_width=300\r\n");
        assert_eq!(app_mut(window.hwnd).settings.sidebar_width, 300);

        send(WM_LBUTTONDOWN, client_size(panel).0 - 1);
        send(WM_MOUSEMOVE, scale(900));
        send(WM_LBUTTONUP, scale(900));
        assert_eq!(saved(), "# kept\r\nsidebar_width=480\r\n");
        assert_eq!(client_size(panel).0, scale(480));

        send(WM_LBUTTONDBLCLK, client_size(panel).0 - 1);
        assert_eq!(saved(), "# kept\r\nsidebar_width=260\r\n");
        assert_eq!(client_size(panel).0, scale(260));
        super::save_settings_to(None);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib -- window::side_panel window::activity_bar window::titlebar window::commands window::menus window::command_palette window::menu_band sidebar squeezes --test-threads=1`
Expected: a compile error. `side_panel`, `activity_bar`, `calculate_with_offset` and the new `CommandId` variants don't exist yet.

- [ ] **Step 3: Add the view commands, their shortcuts, palette entries and menu item**

`src/window/commands.rs`. After Task 1 the enum ends in `NoteDelete = 175,`. Append the new variants after it:

```rust
    NoteRename = 174,
    NoteDelete = 175,
    ToggleSidebar = 176,
    ShowNotebookView = 177,
    ShowSearchView = 178,
    ShowFavoritesView = 179,
}
```

In `needs_document`, add the four new variants to the `matches!` list of commands that need no document:

```rust
                | Self::ToggleSidebar
                | Self::ShowNotebookView
                | Self::ShowSearchView
                | Self::ShowFavoritesView
```

Add this method to `impl CommandId`, after `is_markdown_preview`:

```rust
    /// Commands that act on the sidebar, which exists only in notes mode.
    pub const fn is_sidebar(self) -> bool {
        matches!(
            self,
            Self::ToggleSidebar
                | Self::ShowNotebookView
                | Self::ShowSearchView
                | Self::ShowFavoritesView
        )
    }
```

In `TryFrom<u16>`, the `COMMANDS` table grows by 4: update its length literal. The four new entries go at the end, after `CommandId::NoteDelete,`:

```rust
            CommandId::NoteDelete,
            CommandId::ToggleSidebar,
            CommandId::ShowNotebookView,
            CommandId::ShowSearchView,
            CommandId::ShowFavoritesView,
        ];
```

`src/window/menus.rs`:
- The `accelerator_specs` array grows by 3: update its length in the return type (the length test was raised in Step 1).
- Add these three entries before `accelerator(FALT, b'Z', CommandId::ToggleWordWrap),`:

```rust
        accelerator(FCONTROL, b'B', CommandId::ToggleSidebar),
        accelerator(FCONTROL | FSHIFT, b'E', CommandId::ShowNotebookView),
        accelerator(FCONTROL, b'K', CommandId::ShowSearchView),
```

In the View popup in `MenuBar::create`, add a separator and the entry after the `Line &numbers` line:

```rust
                MenuEntry::command("Line &numbers", CommandId::ToggleLineNumbers),
                MenuEntry::Separator,
                MenuEntry::command("Side&bar\tCtrl+B", CommandId::ToggleSidebar),
```

The mnemonic is `b`, because `s` already belongs to "Markdown preview &side by side". Add this after `set_markdown_preview_enabled`:

```rust
/// Grays the View menu's Sidebar entry while notes mode is off and there is no sidebar.
pub(crate) fn set_sidebar_enabled(menu: HMENU, enabled: bool) {
    let state = MF_BYCOMMAND | if enabled { MF_ENABLED } else { MF_GRAYED };
    unsafe { EnableMenuItem(menu, CommandId::ToggleSidebar as u32, state) };
}
```

`src/window/command_palette.rs`: `ENTRIES` grows by 4: update its length literal. Insert these after `entry("View: Previous tab", CommandId::PreviousTab),`:

```rust
    entry("View: Toggle sidebar", CommandId::ToggleSidebar),
    entry("View: Show notebook", CommandId::ShowNotebookView),
    entry("View: Show search", CommandId::ShowSearchView),
    entry("View: Show favorites", CommandId::ShowFavoritesView),
```

- [ ] **Step 4: Offset the title strip**

`src/window/titlebar.rs`:
- Add `ExcludeClipRect` to the `windows_sys::Win32::Graphics::Gdi` import.
- Replace `fn create_font` with the one font constructor the title strip and the sidebar share, and keep `create_font` as a wrapper so `TitleFonts::create` is unchanged:

```rust
/// A GDI font `pixel_height` device pixels tall. The title strip and the sidebar share it.
pub(crate) fn create_ui_font(pixel_height: i32, face: &str, weight: i32, italic: bool) -> HFONT {
    let face = crate::platform::wide_null(face);
    unsafe {
        CreateFontW(
            -pixel_height,
            0,
            0,
            0,
            weight,
            u32::from(italic),
            0,
            0,
            u32::from(DEFAULT_CHARSET),
            u32::from(OUT_DEFAULT_PRECIS),
            u32::from(CLIP_DEFAULT_PRECIS),
            u32::from(CLEARTYPE_QUALITY),
            u32::from(DEFAULT_PITCH),
            face.as_ptr(),
        )
    }
}

fn create_font(pixel_height: i32, face: &str) -> HFONT {
    create_ui_font(pixel_height, face, FW_NORMAL as i32, false)
}
```

- Add the strip height as its own function, which the activity bar's button layout needs without a window:

```rust
/// The title strip's height at `dpi`: 40 px at 96 DPI, never less than a caption button.
pub(crate) fn strip_height(dpi: u32) -> i32 {
    let dpi = dpi.max(1);
    scale(40, dpi).max(unsafe { GetSystemMetricsForDpi(SM_CYSIZE, dpi) })
}
```
- In `TitleBarLayout`, add this field after `drag_region`:

```rust
    /// The sidebar's share of the strip, left of the tabs. It is caption (dragging, top-edge
    /// resizing, double-click to maximize), and empty without a sidebar.
    pub sidebar: Rect,
```

Replace `calculate_with_preview` with a wrapper, and add `calculate_with_offset` holding the old body. The changes in the body:
- tabs start at `left`;
- tab widths, `max_scroll` and the drag region use the viewport right of `left`;
- the new `sidebar` rect.

```rust
    pub fn calculate_with_preview(
        client: Size,
        dpi: u32,
        tab_count: usize,
        scroll: i32,
        preview_buttons: bool,
    ) -> Self {
        Self::calculate_with_offset(client, dpi, tab_count, scroll, preview_buttons, 0)
    }

    /// The strip with the tabs starting at `left`, right of the sidebar. The caption buttons keep
    /// the right edge, and everything left of `left` is caption.
    pub fn calculate_with_offset(
        client: Size,
        dpi: u32,
        tab_count: usize,
        scroll: i32,
        preview_buttons: bool,
        left: i32,
    ) -> Self {
        let width = client.width.max(0);
        let dpi = dpi.max(1);
        let height = strip_height(dpi);
        let resize_border = unsafe {
            GetSystemMetricsForDpi(SM_CYFRAME, dpi) + GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi)
        }
        .clamp(1, (height / 2).max(1));
        let caption_width = scale(46, dpi).max(1).min(width / 3);
        let close = Rect::new(width - caption_width, 0, width, height);
        let maximize = Rect::new(close.left - caption_width, 0, close.left, height);
        let minimize = Rect::new(maximize.left - caption_width, 0, maximize.left, height);

        let actions_right = minimize.left.max(0);
        let overflow_left = (actions_right - scale(40, dpi)).max(0);
        let overflow = Rect::new(overflow_left, 0, actions_right, height);

        let (preview_side, preview_full, buttons_left) = if preview_buttons {
            let button = scale(40, dpi);
            let full_left = (overflow_left - button).max(0);
            let side_left = (full_left - button).max(0);
            (
                Some(Rect::new(side_left, 0, full_left, height)),
                Some(Rect::new(full_left, 0, overflow_left, height)),
                side_left,
            )
        } else {
            (None, None, overflow_left)
        };

        let left = left.clamp(0, buttons_left);
        // Some empty strip always stays reachable, however many tabs are open.
        let tabs_right = (buttons_left - scale(48, dpi)).max(left);
        let tabs = Rect::new(left, 0, tabs_right, height);
        let viewport = tabs_right - left;
        let preferred_tab_width = scale(200, dpi);
        let tab_width = if tab_count == 0 {
            0
        } else {
            (viewport / tab_count as i32).clamp(scale(120, dpi), preferred_tab_width)
        };
        let content_width = tab_width.saturating_mul(tab_count as i32);
        let max_scroll = (content_width - viewport).max(0);
        let scroll = scroll.clamp(0, max_scroll);

        let close_size = scale(32, dpi).min(tab_width);
        let mut tab_rects = Vec::with_capacity(tab_count);
        let mut close_tab_rects = Vec::with_capacity(tab_count);
        for index in 0..tab_count {
            let tab_left = left + index as i32 * tab_width - scroll;
            let right = tab_left + tab_width;
            tab_rects.push(Rect::new(tab_left, 0, right, height));
            close_tab_rects.push(Rect::new(right - close_size, 0, right, height));
        }

        let drag_region = Rect::new(
            (left + content_width - scroll).clamp(left, tabs_right),
            0,
            buttons_left,
            height,
        );
        let scroll_bar = (max_scroll > 0)
            .then(|| Rect::new(tabs.left, height - scale(8, dpi), tabs.right, height));

        Self {
            tabs,
            drag_region,
            sidebar: Rect::new(0, 0, left, height),
            minimize,
            maximize,
            close,
            overflow,
            preview_side,
            preview_full,
            height,
            resize_border,
            scroll,
            max_scroll,
            scroll_bar,
            min_thumb: scale(24, dpi),
            tab_width,
            tab_rects,
            close_tab_rects,
        }
    }
```

With `left = 0`, `(buttons_left - 48s).max(0)` equals the old `buttons_left - 48s.min(buttons_left)`, so the layout without a sidebar is unchanged. The first titlebar test pins that.

In `hit_test`, replace the final `drag_region` check:

```rust
        if self.sidebar.contains(point) || self.drag_region.contains(point) {
            return HitTarget::Caption;
        }
        HitTarget::Client
```

Replace `layout_for_window` so every caller (painting, `WM_NCHITTEST`, `title_layout`, `invalidate_strip`) gets the offset:

```rust
pub(crate) fn layout_for_window(
    hwnd: HWND,
    tab_count: usize,
    scroll: i32,
    preview_buttons: bool,
) -> TitleBarLayout {
    let mut client = RECT::default();
    unsafe {
        GetClientRect(hwnd, &mut client);
    }
    TitleBarLayout::calculate_with_offset(
        Size::new(client.right - client.left, client.bottom - client.top),
        unsafe { GetDpiForWindow(hwnd) }.max(96),
        tab_count,
        scroll,
        preview_buttons,
        crate::window::side_panel::left_edge(hwnd),
    )
}
```

In `paint`, the frame never paints under the sidebar's windows. The main window has no `WS_CLIPCHILDREN`, so it clips them out itself, and the empty-tabs hint, the status bar and the menu band start at the sidebar's right edge. Replace `paint` with:

```rust
pub(crate) unsafe fn paint(hwnd: HWND, input: &TitlePaint<'_>) {
    let mut paint = PAINTSTRUCT::default();
    let dc = unsafe { BeginPaint(hwnd, &mut paint) };
    if dc.is_null() {
        return;
    }

    let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
    let layout = layout_for_window(
        hwnd,
        input.titles.len(),
        input.scroll,
        input.preview.is_some(),
    );
    let mut client = RECT::default();
    unsafe {
        GetClientRect(hwnd, &mut client);
    }
    // The activity bar and the panel paint themselves; the frame never paints under them.
    let left = layout.sidebar.right;
    if left > 0 {
        unsafe {
            ExcludeClipRect(dc, 0, 0, left, client.bottom);
        }
    }
    let maximized = unsafe { IsZoomed(hwnd) } != 0;
    if paint.rcPaint.top < layout.height {
        unsafe { paint_strip_buffered(dc, &layout, dpi, maximized, input) };
    }

    let status_height = if input.status.is_some() {
        crate::window::status::status_height(dpi)
    } else {
        0
    };
    if let Some(hint) = input.empty_hint {
        let content = Rect::new(
            left,
            layout.height,
            client.right,
            client.bottom - status_height,
        );
        let margin = scale(24, dpi);
        let middle = (content.top + content.bottom) / 2;
        unsafe {
            fill(dc, content, input.palette.editor_background);
            SetBkMode(dc, TRANSPARENT as i32);
            let previous = select_font(dc, input.fonts.text);
            SetTextColor(dc, input.palette.muted_foreground);
            draw_text(
                dc,
                hint,
                Rect::new(
                    content.left + margin,
                    middle - scale(20, dpi),
                    (content.right - margin).max(content.left + margin),
                    middle + scale(20, dpi),
                ),
                DT_CENTER | DT_WORDBREAK | DT_NOPREFIX,
            );
            restore_font(dc, previous);
        }
    }

    if let Some(status) = input.status {
        let bar = Rect::new(
            left,
            client.bottom - status_height,
            client.right,
            client.bottom,
        );
        let margin = scale(10, dpi);
        let gap = scale(24, dpi);
        let text = Rect::new(
            bar.left + margin,
            bar.top,
            (bar.right - margin).max(bar.left + margin),
            bar.bottom,
        );
        let format = DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX;
        unsafe {
            fill(dc, bar, input.palette.strip_background);
            SetBkMode(dc, TRANSPARENT as i32);
            let previous = select_font(dc, input.fonts.text);
            // The document details keep their full width; a long notice is what gets ellipsized.
            let right_width = if status.right.is_empty() {
                0
            } else {
                SetTextColor(dc, input.palette.muted_foreground);
                draw_text(dc, &status.right, text, format | DT_RIGHT);
                measure_text(dc, &status.right, format) + gap
            };
            SetTextColor(dc, input.palette.strip_foreground);
            draw_text(
                dc,
                &status.left,
                Rect::new(
                    text.left,
                    text.top,
                    (text.right - right_width).max(text.left),
                    text.bottom,
                ),
                format | DT_END_ELLIPSIS,
            );
            restore_font(dc, previous);
        }
    }

    if let Some((mode, headings)) = input.menu {
        unsafe {
            crate::window::menu_band::paint(
                dc,
                left,
                client.right,
                headings,
                mode,
                input.palette,
                input.fonts.text,
            );
        }
    }

    if let Some(divider) = input.divider {
        unsafe { fill(dc, from_native(divider), input.palette.hover_background) };
    }

    unsafe {
        EndPaint(hwnd, &paint);
    }
}
```

In `src/window/accessibility.rs`, `native_layout` computes the tabs' accessible locations. Make it use the same offset:

```rust
fn native_layout(item: &AccessibleProvider, tabs: usize) -> TitleBarLayout {
    let mut client = windows_sys::Win32::Foundation::RECT::default();
    unsafe {
        GetClientRect(item.hwnd, &mut client);
    }
    TitleBarLayout::calculate_with_offset(
        Size::new(client.right - client.left, client.bottom - client.top),
        unsafe { GetDpiForWindow(item.hwnd) }.max(96),
        tabs,
        item.selection.scroll_offset(),
        item.view.snapshot().preview_buttons,
        crate::window::side_panel::left_edge(item.hwnd),
    )
}
```

- [ ] **Step 5: Offset the bands above the editor and add the panel shade**

`src/window/menu_band.rs`: replace `heading_rects` and `paint`:

```rust
/// Heading rectangles for label widths `widths`, laid out left to right from `left` in a band at
/// `top`.
pub(crate) fn heading_rects(widths: &[i32], left: i32, top: i32, dpi: u32) -> Vec<RECT> {
    let padding = scale(HEADING_PADDING_AT_96_DPI, dpi);
    let mut left = left + scale(BAND_MARGIN_AT_96_DPI, dpi);
    widths
        .iter()
        .map(|width| {
            let right = left + width + 2 * padding;
            let rect = RECT {
                left,
                top,
                right,
                bottom: top + band_height(dpi),
            };
            left = right;
            rect
        })
        .collect()
}
```

```rust
/// Paints the band from `left` to `right` with `mode.hot` highlighted (pressed while its dropdown
/// is open).
pub(crate) unsafe fn paint(
    dc: HDC,
    left: i32,
    right: i32,
    headings: &[RECT],
    mode: MenuMode,
    palette: Palette,
    font: HFONT,
) {
    let Some(first) = headings.first() else {
        return;
    };
    let band = RECT {
        left,
        top: first.top,
        right,
        bottom: first.bottom,
    };
    unsafe {
        fill(dc, band, palette.strip_background);
        SetBkMode(dc, TRANSPARENT as i32);
        let previous = (!font.is_null()).then(|| SelectObject(dc, font as _));
        for (index, (title, rect)) in MENU_TITLES.iter().zip(headings).enumerate() {
            let foreground = if index == mode.hot {
                let background = if mode.open {
                    palette.pressed_background
                } else {
                    palette.hover_background
                };
                fill(dc, *rect, background);
                palette.hover_foreground
            } else {
                palette.strip_foreground
            };
            SetTextColor(dc, foreground);
            let mut text = wide_null(title);
            let mut rect = *rect;
            // No DT_NOPREFIX: the mnemonic letters are underlined, as keyboard menu mode shows them.
            DrawTextW(
                dc,
                text.as_mut_ptr(),
                -1,
                &mut rect,
                DT_CENTER | DT_VCENTER | DT_SINGLELINE,
            );
        }
        if let Some(previous) = previous {
            SelectObject(dc, previous);
        }
    }
}
```

`src/window/find_bar.rs`: replace `FindBar::layout` (only its signature, its doc comment and the panel's x change):

```rust
    /// Places the bar across `width` from `left`, at `top`, and its fields inside it.
    pub(crate) fn layout(&self, left: i32, width: i32, top: i32, dpi: u32, font: HFONT) {
        if !self.visible {
            return;
        }
        unsafe {
            if !font.is_null() {
                SendMessageW(self.query_edit, WM_SETFONT, font as WPARAM, 0);
                SendMessageW(self.replace_edit, WM_SETFONT, font as WPARAM, 0);
            }
        }
        let BarLayout { query, replace, .. } =
            bar_layout(width, dpi, text_height(self.query_edit, font), self.mode);
        let move_to = |hwnd, rect: RECT| unsafe {
            MoveWindow(
                hwnd,
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
                1,
            );
        };
        move_to(self.query_edit, query.edit);
        if let Some(replace) = replace {
            move_to(self.replace_edit, replace.edit);
        }
        unsafe {
            SetWindowPos(
                self.panel,
                HWND_TOP,
                left,
                top,
                width.max(0),
                find_bar_height(dpi),
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
            InvalidateRect(self.panel, std::ptr::null(), 0);
        }
    }
```

`src/window/name_box.rs`: replace `NameBox::layout` the same way:

```rust
    /// Places the box across `width` from `left`, at `top`, and its controls inside it.
    pub(crate) fn layout(&self, left: i32, width: i32, top: i32, dpi: u32, font: HFONT) {
        if !self.visible {
            return;
        }
        unsafe {
            if !font.is_null() {
                for control in [self.edit, self.save, self.browse] {
                    SendMessageW(control, WM_SETFONT, font as WPARAM, 0);
                }
            }
        }
        let layout = box_layout(
            width,
            dpi,
            text_height(self.edit, font),
            self.note_width(font),
            self.show_browse,
        );
        let move_to = |hwnd, rect: RECT| unsafe {
            MoveWindow(
                hwnd,
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
                1,
            );
        };
        move_to(self.edit, layout.edit);
        move_to(self.save, layout.save);
        if let Some(browse) = layout.browse {
            move_to(self.browse, browse);
        }
        unsafe {
            SetWindowPos(
                self.panel,
                HWND_TOP,
                left,
                top,
                width.max(0),
                name_box_height(dpi),
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
            InvalidateRect(self.panel, std::ptr::null(), 0);
        }
    }
```

`src/window/command_palette.rs`: replace `apply_layout`'s signature, doc comment and first three lines. The rest of the body is unchanged.

```rust
    /// Centers the measured panel horizontally in the `parent_width` wide editor area starting at
    /// `left`, from `top`, and places the field and list inside it.
    pub(crate) fn apply_layout(
        &self,
        left: i32,
        parent_width: i32,
        top: i32,
        dpi: u32,
        font: HFONT,
    ) {
        let Some(layout) = self.layout.filter(|_| self.visible) else {
            return;
        };
        let left = left + ((parent_width - layout.width) / 2).max(0);
```

`src/window/palette.rs`: add to `impl Palette`, after `active_tab_background`:

```rust
    /// The side panel's background: halfway between the strip and the editor, channel by channel.
    pub const fn panel_background(&self) -> u32 {
        ((self.strip_background >> 1) & 0x007f_7f7f)
            + ((self.editor_background >> 1) & 0x007f_7f7f)
    }
```

- [ ] **Step 6: The tooltip wrapper**

`src/window/tooltip.rs`:

```rust
//! A tooltip control (`TOOLTIPS_CLASS`) for rectangles of one painted window. The control
//! subclasses its owner to see the pointer, and is destroyed with it.

use crate::platform::wide_null;
use windows_sys::Win32::Foundation::{HWND, LPARAM, RECT};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::{
    ICC_BAR_CLASSES, INITCOMMONCONTROLSEX, InitCommonControlsEx, TOOLTIPS_CLASS, TTF_SUBCLASS,
    TTM_ADDTOOLW, TTM_DELTOOLW, TTS_ALWAYSTIP, TTS_NOPREFIX, TTTOOLINFOW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CW_USEDEFAULT, CreateWindowExW, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    SendMessageW, SetWindowPos, WS_EX_TOPMOST, WS_POPUP,
};

/// FastPad has no comctl32 v6 manifest. The v5 control rejects the full `TTTOOLINFOW` size, and
/// then every `TTM_ADDTOOLW` fails silently, so the structure is sized up to `lpReserved`.
const TOOL_INFO_SIZE: u32 = std::mem::offset_of!(TTTOOLINFOW, lpReserved) as u32;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Tooltip {
    hwnd: HWND,
    owner: HWND,
}

impl Tooltip {
    /// A tooltip for rectangles of `owner`, or `None` if the control can't be created.
    pub(crate) fn create(owner: HWND) -> Option<Tooltip> {
        static INITIALIZED: std::sync::Once = std::sync::Once::new();
        INITIALIZED.call_once(|| {
            let controls = INITCOMMONCONTROLSEX {
                dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
                dwICC: ICC_BAR_CLASSES,
            };
            unsafe { InitCommonControlsEx(&controls) };
        });
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOPMOST,
                TOOLTIPS_CLASS,
                std::ptr::null(),
                WS_POPUP | TTS_ALWAYSTIP | TTS_NOPREFIX,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                owner,
                std::ptr::null_mut(),
                GetModuleHandleW(std::ptr::null()),
                std::ptr::null(),
            )
        };
        if hwnd.is_null() {
            return None;
        }
        unsafe {
            SetWindowPos(
                hwnd,
                HWND_TOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
        Some(Self { hwnd, owner })
    }

    /// Shows `text` while the pointer rests on `rect` (owner client coordinates). A tool with the
    /// same `id` is replaced, and an empty `text` removes it, so no empty tip ever shows.
    pub(crate) fn set_tool(&self, id: usize, rect: RECT, text: &str) {
        let mut wide = wide_null(text);
        let mut info = TTTOOLINFOW {
            cbSize: TOOL_INFO_SIZE,
            uFlags: TTF_SUBCLASS,
            hwnd: self.owner,
            uId: id,
            rect,
            lpszText: wide.as_mut_ptr(),
            ..Default::default()
        };
        unsafe {
            SendMessageW(
                self.hwnd,
                TTM_DELTOOLW,
                0,
                &info as *const TTTOOLINFOW as LPARAM,
            );
        }
        if text.is_empty() {
            return;
        }
        unsafe {
            SendMessageW(
                self.hwnd,
                TTM_ADDTOOLW,
                0,
                &mut info as *mut TTTOOLINFOW as LPARAM,
            );
        }
    }

    #[cfg(test)]
    pub(crate) fn tool_count(&self) -> usize {
        (unsafe {
            SendMessageW(
                self.hwnd,
                windows_sys::Win32::UI::Controls::TTM_GETTOOLCOUNT,
                0,
                0,
            )
        }) as usize
    }
}
```

- [ ] **Step 7: The activity bar**

`src/window/activity_bar.rs` (the tests module is the one from Step 1):

```rust
//! The activity bar: the sidebar's narrow painted strip of view buttons at the main window's left
//! edge, with Settings at the bottom. The active view has a 2 px accent bar and full-strength
//! color, and the others are muted. The strip above the first button is caption, so the bar
//! answers `HTTRANSPARENT` there.

use super::side_panel::{self, draw_text, paint_buffered, point_of, with_bar_state};
use crate::config::SidebarView;
use crate::window::panel::{fill, scale};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    DT_CENTER, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, InvalidateRect, ScreenToClient,
};
use windows_sys::Win32::UI::Controls::WM_MOUSELEAVE;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    ReleaseCapture, SetCapture, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DefWindowProcW, GetClientRect, GetParent, HTTRANSPARENT, WM_CAPTURECHANGED, WM_ERASEBKGND,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_NCHITTEST, WM_PAINT,
};

/// Segoe MDL2 Assets glyphs: Library, Search, FavoriteStar, Setting.
const GLYPH_NOTEBOOK: &str = "\u{E8F1}";
const GLYPH_SEARCH: &str = "\u{E721}";
const GLYPH_FAVORITES: &str = "\u{E734}";
const GLYPH_SETTINGS: &str = "\u{E713}";
const ACCENT_WIDTH_96: i32 = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ActivityButton {
    Notebook,
    Search,
    Favorites,
    Settings,
}

impl ActivityButton {
    pub(crate) const ALL: [Self; 4] = [Self::Notebook, Self::Search, Self::Favorites, Self::Settings];

    pub(crate) const fn index(self) -> usize {
        self as usize
    }

    /// The view this button shows; Settings shows none.
    pub(crate) const fn view(self) -> Option<SidebarView> {
        match self {
            Self::Notebook => Some(SidebarView::Notebook),
            Self::Search => Some(SidebarView::Search),
            Self::Favorites => Some(SidebarView::Favorites),
            Self::Settings => None,
        }
    }

    /// The tooltip and accessible name (the Notebook tooltip adds the notebook's name).
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Notebook => "Notebook",
            Self::Search => "Search",
            Self::Favorites => "Favorites",
            Self::Settings => "Settings",
        }
    }

    const fn glyph(self) -> &'static str {
        match self {
            Self::Notebook => GLYPH_NOTEBOOK,
            Self::Search => GLYPH_SEARCH,
            Self::Favorites => GLYPH_FAVORITES,
            Self::Settings => GLYPH_SETTINGS,
        }
    }
}

/// Pointer state: the hovered button, the one a press went down on, and whether leave tracking
/// is armed.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct BarState {
    pub(crate) hover: Option<ActivityButton>,
    pub(crate) pressed: Option<ActivityButton>,
    pub(crate) tracking: bool,
}

/// Button rectangles in the bar's `client` coordinates at `dpi`, indexed by
/// `ActivityButton::index`. The view buttons are bar-wide squares stacked down from the bottom of
/// the title strip (`titlebar::strip_height`), whose share of the bar is caption. Settings sits at
/// the bottom and never covers them.
pub(crate) fn button_rects(client: RECT, dpi: u32) -> [RECT; 4] {
    let size = (client.right - client.left).max(0);
    let top = client.top + crate::window::titlebar::strip_height(dpi);
    let square = |top: i32| RECT {
        left: client.left,
        top,
        right: client.left + size,
        bottom: top + size,
    };
    let settings_top = (client.bottom - size).max(top + 3 * size);
    [
        square(top),
        square(top + size),
        square(top + 2 * size),
        square(settings_top),
    ]
}

pub(crate) fn button_at(rects: &[RECT; 4], x: i32, y: i32) -> Option<ActivityButton> {
    ActivityButton::ALL.into_iter().find(|button| {
        let rect = rects[button.index()];
        x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom
    })
}

fn rects_for(bar: HWND) -> [RECT; 4] {
    let mut client = RECT::default();
    unsafe { GetClientRect(bar, &mut client) };
    button_rects(client, unsafe { GetDpiForWindow(bar) }.max(96))
}

pub(crate) fn register_class() -> crate::Result<&'static [u16]> {
    static CLASS: std::sync::OnceLock<Option<Vec<u16>>> = std::sync::OnceLock::new();
    side_panel::register_child_class(&CLASS, "FastPadActivityBar", 0, Some(bar_proc))
}

/// The Settings button's click handler. Task 12 points it at `main_window::open_settings_palette`,
/// which lists only the settings commands.
fn open_settings(main: HWND) {
    super::main_window::open_command_palette(main);
}

/// A click on `button`: an inactive view opens, the active one closes the panel, and Settings
/// runs `open_settings`.
pub(crate) fn activate(main: HWND, button: ActivityButton) {
    match button.view() {
        Some(view) if side_panel::current_view(main) == view => {
            side_panel::show_view(main, SidebarView::Hidden, false)
        }
        Some(view) => side_panel::show_view(main, view, false),
        None => open_settings(main),
    }
}

unsafe extern "system" fn bar_proc(
    bar: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let main = unsafe { GetParent(bar) };
    match message {
        WM_PAINT => {
            paint(main, bar);
            0
        }
        WM_ERASEBKGND => 1,
        // Above the first button the main window's caption hit test applies.
        WM_NCHITTEST => {
            let (x, y) = point_of(lparam);
            let mut point = POINT { x, y };
            unsafe { ScreenToClient(bar, &mut point) };
            if point.y < super::main_window::title_layout(main).height {
                HTTRANSPARENT as LRESULT
            } else {
                unsafe { DefWindowProcW(bar, message, wparam, lparam) }
            }
        }
        WM_MOUSEMOVE => {
            let (x, y) = point_of(lparam);
            hover(main, bar, button_at(&rects_for(bar), x, y));
            0
        }
        WM_MOUSELEAVE => {
            with_bar_state(main, |state| state.tracking = false);
            hover(main, bar, None);
            0
        }
        WM_LBUTTONDOWN => {
            let (x, y) = point_of(lparam);
            let target = button_at(&rects_for(bar), x, y);
            with_bar_state(main, |state| state.pressed = target);
            if target.is_some() {
                unsafe { SetCapture(bar) };
            }
            unsafe { InvalidateRect(bar, std::ptr::null(), 0) };
            0
        }
        WM_LBUTTONUP => {
            let (x, y) = point_of(lparam);
            let target = button_at(&rects_for(bar), x, y);
            let pressed = with_bar_state(main, |state| state.pressed.take()).flatten();
            if pressed.is_some() {
                unsafe { ReleaseCapture() };
            }
            unsafe { InvalidateRect(bar, std::ptr::null(), 0) };
            if let Some(button) = pressed
                && target == Some(button)
            {
                activate(main, button);
            }
            0
        }
        WM_CAPTURECHANGED => {
            with_bar_state(main, |state| state.pressed = None);
            unsafe { InvalidateRect(bar, std::ptr::null(), 0) };
            0
        }
        _ => unsafe { DefWindowProcW(bar, message, wparam, lparam) },
    }
}

fn hover(main: HWND, bar: HWND, target: Option<ActivityButton>) {
    let (changed, track) = with_bar_state(main, |state| {
        let changed = state.hover != target;
        state.hover = target;
        let track = target.is_some() && !state.tracking;
        state.tracking |= track;
        (changed, track)
    })
    .unwrap_or((false, false));
    if track {
        let mut event = TRACKMOUSEEVENT {
            cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
            dwFlags: TME_LEAVE,
            hwndTrack: bar,
            dwHoverTime: 0,
        };
        unsafe { TrackMouseEvent(&mut event) };
    }
    if changed {
        unsafe { InvalidateRect(bar, std::ptr::null(), 0) };
    }
}

fn paint(main: HWND, bar: HWND) {
    let palette = super::main_window::current_palette(main);
    let dpi = unsafe { GetDpiForWindow(bar) }.max(96);
    let glyph = super::main_window::ui_fonts(main).bar_glyph;
    let state = with_bar_state(main, |state| *state).unwrap_or_default();
    let view = side_panel::current_view(main);
    let accent = scale(ACCENT_WIDTH_96, dpi);
    paint_buffered(bar, |dc, client| unsafe {
        fill(dc, client, palette.strip_background);
        let rects = button_rects(client, dpi);
        for button in ActivityButton::ALL {
            let rect = rects[button.index()];
            let active = button.view() == Some(view);
            let hovered = state.hover == Some(button);
            if hovered {
                let background = if state.pressed == Some(button) {
                    palette.pressed_background
                } else {
                    palette.hover_background
                };
                fill(dc, rect, background);
            }
            if active {
                fill(
                    dc,
                    RECT {
                        right: rect.left + accent,
                        ..rect
                    },
                    palette.editor_foreground,
                );
            }
            let color = if hovered {
                palette.hover_foreground
            } else if active {
                palette.editor_foreground
            } else {
                palette.muted_foreground
            };
            draw_text(
                dc,
                button.glyph(),
                rect,
                glyph,
                color,
                DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
            );
        }
    });
}
```

- [ ] **Step 8: The side panel and the sidebar state**

`src/window/side_panel.rs` (the tests module is the one from Step 1):

```rust
//! The sidebar: the activity bar and one side panel, two painted child windows along the main
//! window's left edge, present only in notes mode. Everything else in the window is laid out to
//! their right (`left_edge`). The panel paints the current view. The views plug in through the
//! `PanelView` dispatch (`paint_view`, `view_mouse`, `view_key`, `header_is_caption`), which
//! Tasks 10 and 12 extend.
//!
//! The strip above the first activity button and the empty part of the panel header belong to the
//! window caption. Both windows answer `WM_NCHITTEST` there with `HTTRANSPARENT`, so the main
//! window's own hit test applies: dragging, top-edge resizing and double-click to maximize.

use super::activity_bar::{self, ActivityButton, BarState};
use super::main_window::{
    app_ptr, change_setting, current_palette, focus_content, invalidate_title_strip,
    layout_editor_and_find_bar, push_notice, tab_count, title_layout, ui_fonts,
};
use super::tooltip::Tooltip;
use crate::config::SidebarView;
use crate::config::defaults::{DEFAULT_SIDEBAR_WIDTH, MAX_SIDEBAR_WIDTH, MIN_SIDEBAR_WIDTH};
use crate::platform::wide_null;
use crate::window::palette::Palette;
use crate::window::panel::{create_child, fill, scale};
use crate::window::titlebar::create_ui_font;
use windows_sys::Win32::Foundation::{
    ERROR_CLASS_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DRAW_TEXT_FORMAT, DT_CALCRECT,
    DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, DeleteDC, DeleteObject,
    DrawTextW, EndPaint, FW_NORMAL, FW_SEMIBOLD, HDC, HFONT, InvalidateRect, PAINTSTRUCT, SRCCOPY,
    ScreenToClient, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::WM_MOUSELEAVE;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetFocus, ReleaseCapture, SetCapture, SetFocus,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CS_DBLCLKS, DefWindowProcW, DestroyWindow, GWL_STYLE, GetClientRect, GetCursorPos, GetParent,
    GetWindowLongPtrW, HTTRANSPARENT, IDC_ARROW, IDC_SIZEWE, IsChild, LoadCursorW, RegisterClassW,
    SW_HIDE, SWP_NOACTIVATE, SWP_NOZORDER, SWP_SHOWWINDOW, SetCursor, SetWindowPos, ShowWindow,
    WM_CAPTURECHANGED, WM_CHAR, WM_CONTEXTMENU, WM_ERASEBKGND, WM_KEYDOWN, WM_KILLFOCUS,
    WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCHITTEST,
    WM_PAINT, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SETCURSOR, WM_SETFOCUS, WNDCLASSW, WNDPROC,
    WS_CHILD, WS_CLIPCHILDREN, WS_CLIPSIBLINGS, WS_VISIBLE,
};

/// Sizes at 96 DPI, scaled with `panel::scale`.
pub(crate) const ACTIVITY_WIDTH_96: i32 = 44;
pub(crate) const EDITOR_MIN_WIDTH_96: i32 = 320;
pub(crate) const HEADER_HEIGHT_96: i32 = 38;
/// The strip along the panel's right edge that resizes it.
pub(crate) const GRIP_WIDTH_96: i32 = 4;
const HEADER_INSET_96: i32 = 16;

/// The sidebar's fonts at one DPI. Painting copies them out; `Sidebar` owns and deletes them.
#[derive(Clone, Copy, Debug)]
#[allow(dead_code, reason = "the text, italic and glyph fonts are read from Task 10's Notebook view on")]
pub(crate) struct UiFonts {
    /// Row and body text: Segoe UI, 12 px at 96 DPI.
    pub(crate) text: HFONT,
    /// Header titles in small capitals: Segoe UI semibold, 11 px.
    pub(crate) bold: HFONT,
    /// Unsaved rows and notices inside the list: Segoe UI italic, 12 px.
    pub(crate) italic: HFONT,
    /// Row and header-button icons: Segoe MDL2 Assets, 12 px.
    pub(crate) glyph: HFONT,
    /// The activity bar's icons: Segoe MDL2 Assets, 16 px.
    pub(crate) bar_glyph: HFONT,
}

impl Default for UiFonts {
    fn default() -> Self {
        Self {
            text: std::ptr::null_mut(),
            bold: std::ptr::null_mut(),
            italic: std::ptr::null_mut(),
            glyph: std::ptr::null_mut(),
            bar_glyph: std::ptr::null_mut(),
        }
    }
}

impl UiFonts {
    fn create(dpi: u32) -> Self {
        let normal = FW_NORMAL as i32;
        Self {
            text: create_ui_font(scale(12, dpi), "Segoe UI", normal, false),
            bold: create_ui_font(scale(11, dpi), "Segoe UI", FW_SEMIBOLD as i32, false),
            italic: create_ui_font(scale(12, dpi), "Segoe UI", normal, true),
            glyph: create_ui_font(scale(12, dpi), "Segoe MDL2 Assets", normal, false),
            bar_glyph: create_ui_font(scale(16, dpi), "Segoe MDL2 Assets", normal, false),
        }
    }

    fn delete(self) {
        for font in [self.text, self.bold, self.italic, self.glyph, self.bar_glyph] {
            if !font.is_null() {
                unsafe { DeleteObject(font) };
            }
        }
    }
}

/// The sidebar's windows and state, owned by `App.sidebar`. Whether a view is open and the saved
/// width live in `Settings`. This holds what `fastpad.ini` doesn't.
#[derive(Debug)]
pub(crate) struct Sidebar {
    pub(crate) bar: HWND,
    pub(crate) panel: HWND,
    pub(crate) tooltip: Option<Tooltip>,
    pub(crate) bar_state: BarState,
    /// The view Ctrl+B reopens while the panel is closed.
    last_view: SidebarView,
    /// The live width (96-DPI pixels) while the panel edge is dragged. The setting changes once,
    /// when the drag ends.
    drag_width: Option<u16>,
    /// The fonts and the DPI they were made for (`main_window::ui_fonts`).
    fonts: Option<(u32, UiFonts)>,
}

impl Sidebar {
    /// The fonts for `dpi`, created on first use and again after a DPI change.
    pub(crate) fn fonts(&mut self, dpi: u32) -> UiFonts {
        if let Some((font_dpi, fonts)) = self.fonts
            && font_dpi == dpi
        {
            return fonts;
        }
        if let Some((_, old)) = self.fonts.take() {
            old.delete();
        }
        let fonts = UiFonts::create(dpi);
        self.fonts = Some((dpi, fonts));
        fonts
    }
}

impl Drop for Sidebar {
    fn drop(&mut self) {
        if let Some((_, fonts)) = self.fonts.take() {
            fonts.delete();
        }
    }
}

/// The view the panel is showing. Tasks 10 and 12 give each view its painting and input
/// through the `match`es in `paint_view`, `view_mouse`, `view_key` and `header_is_caption`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PanelView {
    Notebook,
    Search,
    Favorites,
}

impl PanelView {
    pub(crate) const fn of(view: SidebarView) -> Option<Self> {
        match view {
            SidebarView::Notebook => Some(Self::Notebook),
            SidebarView::Search => Some(Self::Search),
            SidebarView::Favorites => Some(Self::Favorites),
            SidebarView::Hidden => None,
        }
    }

    /// The header's title, in small capitals.
    pub(crate) const fn title(self) -> &'static str {
        match self {
            Self::Notebook => "NOTEBOOK",
            Self::Search => "SEARCH",
            Self::Favorites => "FAVORITES",
        }
    }
}

/// What a view paints with: the panel's buffered DC and everything a paint needs, built once per
/// `WM_PAINT` by `view_paint`.
#[derive(Clone, Copy)]
#[allow(dead_code, reason = "`focused` is read from Task 10's Notebook view on")]
pub(crate) struct ViewPaint {
    pub(crate) hdc: HDC,
    /// The panel's whole client rectangle. Each view lays out its header and body inside it.
    pub(crate) client: RECT,
    pub(crate) palette: Palette,
    /// The panel's fill, `Palette::panel_background`, already painted.
    pub(crate) background: u32,
    pub(crate) fonts: UiFonts,
    pub(crate) dpi: u32,
    /// The panel window itself (not a child control) has the keyboard focus.
    pub(crate) focused: bool,
}

/// The `ViewPaint` for `panel`'s `hdc` and `client` rectangle. Call it with nothing of the App
/// borrowed. The panel's paint and the views' paint tests use it.
pub(crate) fn view_paint(main: HWND, panel: HWND, hdc: HDC, client: RECT) -> ViewPaint {
    let palette = current_palette(main);
    ViewPaint {
        hdc,
        client,
        palette,
        background: palette.panel_background(),
        fonts: ui_fonts(main),
        dpi: unsafe { GetDpiForWindow(panel) }.max(96),
        focused: unsafe { GetFocus() } == panel,
    }
}

/// Draws `text` in `rect` with `font` and `color` over a transparent background, and returns the
/// width it took, at most `rect`'s. `flags` are `DrawTextW`'s. Empty text draws nothing.
pub(crate) unsafe fn draw_text(
    hdc: HDC,
    text: &str,
    rect: RECT,
    font: HFONT,
    color: u32,
    flags: DRAW_TEXT_FORMAT,
) -> i32 {
    // An empty buffer's pointer dangles, and DT_END_ELLIPSIS lets DrawTextW touch it.
    if text.is_empty() {
        return 0;
    }
    let wide: Vec<u16> = text.encode_utf16().collect();
    let mut target = rect;
    let mut measured = rect;
    unsafe {
        let previous = (!font.is_null()).then(|| SelectObject(hdc, font));
        SetBkMode(hdc, TRANSPARENT as i32);
        SetTextColor(hdc, color);
        DrawTextW(
            hdc,
            wide.as_ptr(),
            wide.len() as i32,
            &mut measured,
            flags | DT_CALCRECT,
        );
        DrawTextW(hdc, wide.as_ptr(), wide.len() as i32, &mut target, flags);
        if let Some(previous) = previous {
            SelectObject(hdc, previous);
        }
    }
    (measured.right - measured.left)
        .min(rect.right - rect.left)
        .max(0)
}

/// The activity bar's and the panel's widths in device pixels, for a `client_width` wide window.
/// The panel (saved at `width_96` 96-DPI pixels, clamped to its range) gives way first, so the
/// editor keeps its minimum. It never goes below 0.
pub(crate) fn sidebar_widths(
    client_width: i32,
    dpi: u32,
    view_open: bool,
    width_96: u16,
) -> (i32, i32) {
    let client_width = client_width.max(0);
    let activity = scale(ACTIVITY_WIDTH_96, dpi).min(client_width);
    if !view_open {
        return (activity, 0);
    }
    let wanted = scale(
        i32::from(width_96.clamp(MIN_SIDEBAR_WIDTH, MAX_SIDEBAR_WIDTH)),
        dpi,
    );
    let room = client_width - activity - scale(EDITOR_MIN_WIDTH_96, dpi);
    (activity, wanted.min(room).max(0))
}

/// The 96-DPI width a drag to `panel_px` device pixels asks for, inside the allowed range.
pub(crate) fn drag_width_96(panel_px: i32, dpi: u32) -> u16 {
    let dpi = i64::from(dpi.max(1));
    let unscaled = (i64::from(panel_px.max(0)) * 96 + dpi / 2) / dpi;
    unscaled.clamp(i64::from(MIN_SIDEBAR_WIDTH), i64::from(MAX_SIDEBAR_WIDTH)) as u16
}

fn with_sidebar<R>(hwnd: HWND, action: impl FnOnce(&mut Sidebar) -> R) -> Option<R> {
    let mut app = unsafe { app_ptr(hwnd) }?;
    unsafe { app.as_mut() }.sidebar.as_mut().map(action)
}

pub(crate) fn with_bar_state<R>(hwnd: HWND, action: impl FnOnce(&mut BarState) -> R) -> Option<R> {
    with_sidebar(hwnd, |sidebar| action(&mut sidebar.bar_state))
}

/// The activity bar and panel windows, while the sidebar exists.
pub(crate) fn windows(hwnd: HWND) -> Option<(HWND, HWND)> {
    with_sidebar(hwnd, |sidebar| (sidebar.bar, sidebar.panel))
}

/// Whether a view is open and the width to lay it out at, or `None` without a sidebar.
fn open_state(hwnd: HWND) -> Option<(bool, u16)> {
    let app = unsafe { app_ptr(hwnd) }?;
    let app = unsafe { app.as_ref() };
    let sidebar = app.sidebar.as_ref()?;
    Some((
        app.settings.sidebar_view != SidebarView::Hidden,
        sidebar.drag_width.unwrap_or(app.settings.sidebar_width),
    ))
}

/// The view the panel shows, `Hidden` while it is closed or there is no sidebar.
pub(crate) fn current_view(hwnd: HWND) -> SidebarView {
    unsafe { app_ptr(hwnd) }
        .map(|app| unsafe { app.as_ref() })
        .filter(|app| app.sidebar.is_some())
        .map_or(SidebarView::Hidden, |app| app.settings.sidebar_view)
}

/// Where the rest of the window starts: the activity bar plus the open panel, in device pixels.
/// 0 with notes mode off.
pub(crate) fn left_edge(hwnd: HWND) -> i32 {
    let Some((open, width)) = open_state(hwnd) else {
        return 0;
    };
    let mut client = RECT::default();
    unsafe { GetClientRect(hwnd, &mut client) };
    let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
    let (activity, panel) = sidebar_widths(client.right - client.left, dpi, open, width);
    activity + panel
}

pub(crate) fn create(hwnd: HWND) -> crate::Result<Sidebar> {
    let bar = create_child(
        hwnd,
        activity_bar::register_class()?,
        WS_CHILD | WS_CLIPSIBLINGS,
    )?;
    let panel = register_panel_class().and_then(|class| {
        create_child(hwnd, class, WS_CHILD | WS_CLIPSIBLINGS | WS_CLIPCHILDREN)
    });
    let panel = match panel {
        Ok(panel) => panel,
        Err(error) => {
            unsafe { DestroyWindow(bar) };
            return Err(error);
        }
    };
    Ok(Sidebar {
        bar,
        panel,
        tooltip: Tooltip::create(bar),
        bar_state: BarState::default(),
        last_view: SidebarView::Notebook,
        drag_width: None,
        fonts: None,
    })
}

/// Creates or destroys the sidebar to match notes mode, then lays the window out again. It also
/// runs once `fastpad.ini` has been applied, to pick up the saved view.
pub(crate) fn notes_mode_changed(hwnd: HWND, enabled: bool) {
    let present =
        unsafe { app_ptr(hwnd) }.is_some_and(|app| unsafe { app.as_ref() }.sidebar.is_some());
    if enabled && !present {
        match create(hwnd) {
            Ok(sidebar) => match unsafe { app_ptr(hwnd) } {
                Some(mut app) => unsafe { app.as_mut() }.sidebar = Some(sidebar),
                None => unsafe {
                    DestroyWindow(sidebar.panel);
                    DestroyWindow(sidebar.bar);
                },
            },
            Err(error) => push_notice(hwnd, format!("FastPad could not show the sidebar: {error}")),
        }
    } else if !enabled && present {
        let sidebar =
            unsafe { app_ptr(hwnd) }.and_then(|mut app| unsafe { app.as_mut() }.sidebar.take());
        if let Some(sidebar) = sidebar {
            if focus_is_in(sidebar.panel) || focus_is_in(sidebar.bar) {
                return_focus(hwnd);
            }
            // The bar owns the tooltip, which goes with it.
            unsafe {
                DestroyWindow(sidebar.panel);
                DestroyWindow(sidebar.bar);
            }
        }
    }
    if let Some(mut app) = unsafe { app_ptr(hwnd) } {
        let app = unsafe { app.as_mut() };
        let view = app.settings.sidebar_view;
        if let Some(sidebar) = app.sidebar.as_mut()
            && view != SidebarView::Hidden
        {
            sidebar.last_view = view;
        }
    }
    layout_editor_and_find_bar(hwnd);
    invalidate_title_strip(hwnd);
}

/// Places the activity bar and the panel for the main window's `client` rectangle.
pub(crate) fn layout(hwnd: HWND, client: RECT, dpi: u32) {
    let Some((open, width_96)) = open_state(hwnd) else {
        return;
    };
    let Some((bar, panel)) = windows(hwnd) else {
        return;
    };
    let height = (client.bottom - client.top).max(0);
    let (activity, panel_width) = sidebar_widths(client.right - client.left, dpi, open, width_96);
    let flags = SWP_NOZORDER | SWP_NOACTIVATE | SWP_SHOWWINDOW;
    unsafe {
        SetWindowPos(bar, std::ptr::null_mut(), 0, 0, activity, height, flags);
        InvalidateRect(bar, std::ptr::null(), 0);
    }
    if panel_width > 0 {
        unsafe {
            SetWindowPos(
                panel,
                std::ptr::null_mut(),
                activity,
                0,
                panel_width,
                height,
                flags,
            );
            InvalidateRect(panel, std::ptr::null(), 0);
        }
    } else {
        if focus_is_in(panel) {
            return_focus(hwnd);
        }
        unsafe { ShowWindow(panel, SW_HIDE) };
    }
    update_tools(hwnd);
}

/// Shows `view`, or closes the panel for `Hidden`, and saves it as `sidebar_view`. `focus` moves
/// the keyboard focus into the panel. Task 12 moves it into the search box for Search.
pub(crate) fn show_view(hwnd: HWND, view: SidebarView, focus: bool) {
    let Some(panel) = with_sidebar(hwnd, |sidebar| {
        if view != SidebarView::Hidden {
            sidebar.last_view = view;
        }
        sidebar.panel
    }) else {
        return;
    };
    if view == SidebarView::Hidden && focus_is_in(panel) {
        return_focus(hwnd);
    }
    change_setting(hwnd, |settings| {
        (settings.sidebar_view != view).then(|| {
            settings.sidebar_view = view;
            ("sidebar_view", view.token().to_owned())
        })
    });
    layout_editor_and_find_bar(hwnd);
    invalidate_title_strip(hwnd);
    if focus && view != SidebarView::Hidden && is_shown(panel) {
        unsafe { SetFocus(panel) };
    }
}

/// Ctrl+B: closes the panel, or reopens the last view.
pub(crate) fn toggle(hwnd: HWND) {
    let Some(last) = with_sidebar(hwnd, |sidebar| sidebar.last_view) else {
        return;
    };
    let next = if current_view(hwnd) == SidebarView::Hidden {
        last
    } else {
        SidebarView::Hidden
    };
    show_view(hwnd, next, false);
}

/// The library changed: re-reads what the sidebar shows of it (the notebook name in the
/// Notebook tooltip) and repaints.
pub(crate) fn refresh(hwnd: HWND) {
    let Some((bar, panel)) = windows(hwnd) else {
        return;
    };
    update_tools(hwnd);
    unsafe {
        InvalidateRect(bar, std::ptr::null(), 0);
        InvalidateRect(panel, std::ptr::null(), 0);
    }
}

/// The active tab changed (`main_window::refresh_tabs` calls it). Task 10 selects the active
/// note's row here.
pub(crate) fn active_tab_changed(hwnd: HWND) {
    if let Some((_, panel)) = windows(hwnd) {
        unsafe { InvalidateRect(panel, std::ptr::null(), 0) };
    }
}

fn update_tools(hwnd: HWND) {
    let Some((bar, _)) = windows(hwnd) else {
        return;
    };
    let Some(tooltip) = with_sidebar(hwnd, |sidebar| sidebar.tooltip).flatten() else {
        return;
    };
    let mut client = RECT::default();
    unsafe { GetClientRect(bar, &mut client) };
    let rects = activity_bar::button_rects(client, unsafe { GetDpiForWindow(bar) }.max(96));
    let notebook = crate::window::library_host::folder(hwnd)
        .and_then(|folder| {
            folder
                .file_name()
                .map(|name| format!("Notebook: {}", name.to_string_lossy()))
        })
        .unwrap_or_else(|| ActivityButton::Notebook.label().to_owned());
    for button in ActivityButton::ALL {
        let text = if button == ActivityButton::Notebook {
            notebook.as_str()
        } else {
            button.label()
        };
        tooltip.set_tool(button.index(), rects[button.index()], text);
    }
}

fn focus_is_in(window: HWND) -> bool {
    let focus = unsafe { GetFocus() };
    !focus.is_null() && (focus == window || unsafe { IsChild(window, focus) } != 0)
}

/// Focus leaving the sidebar goes to the content, or to the frame while no tab is open.
fn return_focus(hwnd: HWND) {
    if tab_count(hwnd) > 0 {
        focus_content(hwnd);
    } else {
        unsafe { SetFocus(hwnd) };
    }
}

/// The window's own visible bit; the main window may be hidden (tests) or minimized.
fn is_shown(window: HWND) -> bool {
    (unsafe { GetWindowLongPtrW(window, GWL_STYLE) }) as u32 & WS_VISIBLE != 0
}

/// The signed client or screen point in a mouse message's `lParam`.
pub(crate) fn point_of(lparam: LPARAM) -> (i32, i32) {
    (
        (lparam as u32 & 0xffff) as u16 as i16 as i32,
        ((lparam as u32 >> 16) & 0xffff) as u16 as i16 as i32,
    )
}

/// Registers a painted child window class once per process and returns its name. `cell` keeps
/// the outcome, so a failed registration is reported every time without retrying.
pub(crate) fn register_child_class(
    cell: &'static std::sync::OnceLock<Option<Vec<u16>>>,
    name: &str,
    style: u32,
    proc: WNDPROC,
) -> crate::Result<&'static [u16]> {
    cell.get_or_init(|| {
        let wide = wide_null(name);
        let class = WNDCLASSW {
            style,
            lpfnWndProc: proc,
            hInstance: unsafe { GetModuleHandleW(std::ptr::null()) },
            hCursor: unsafe { LoadCursorW(std::ptr::null_mut(), IDC_ARROW) },
            lpszClassName: wide.as_ptr(),
            ..Default::default()
        };
        let registered =
            unsafe { RegisterClassW(&class) != 0 || GetLastError() == ERROR_CLASS_ALREADY_EXISTS };
        registered.then_some(wide)
    })
    .as_deref()
    .ok_or(crate::FastPadError::Invariant(
        "a sidebar window class could not be registered",
    ))
}

fn register_panel_class() -> crate::Result<&'static [u16]> {
    static CLASS: std::sync::OnceLock<Option<Vec<u16>>> = std::sync::OnceLock::new();
    // Double-clicks reset the width from the edge and, from Task 10, open rows permanently.
    register_child_class(&CLASS, "FastPadSidePanel", CS_DBLCLKS, Some(panel_proc))
}

/// Paints `window` through an off-screen bitmap of its client size, so a repaint never flickers.
pub(crate) fn paint_buffered(window: HWND, draw: impl FnOnce(HDC, RECT)) {
    let mut paint = PAINTSTRUCT::default();
    let dc = unsafe { BeginPaint(window, &mut paint) };
    if dc.is_null() {
        return;
    }
    let mut client = RECT::default();
    unsafe { GetClientRect(window, &mut client) };
    let (width, height) = (client.right, client.bottom);
    if width > 0 && height > 0 {
        let memory = unsafe { CreateCompatibleDC(dc) };
        let bitmap = if memory.is_null() {
            std::ptr::null_mut()
        } else {
            unsafe { CreateCompatibleBitmap(dc, width, height) }
        };
        if bitmap.is_null() {
            draw(dc, client);
        } else {
            unsafe {
                let previous = SelectObject(memory, bitmap);
                draw(memory, client);
                BitBlt(dc, 0, 0, width, height, memory, 0, 0, SRCCOPY);
                SelectObject(memory, previous);
                DeleteObject(bitmap);
            }
        }
        if !memory.is_null() {
            unsafe { DeleteDC(memory) };
        }
    }
    unsafe { EndPaint(window, &paint) };
}

unsafe extern "system" fn panel_proc(
    panel: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let main = unsafe { GetParent(panel) };
    match message {
        WM_PAINT => {
            paint_panel(main, panel);
            0
        }
        WM_ERASEBKGND => 1,
        WM_NCHITTEST => panel_hit_test(main, panel, wparam, lparam),
        WM_SETCURSOR if resizing(main) || pointer_over_grip(panel) => {
            unsafe { SetCursor(LoadCursorW(std::ptr::null_mut(), IDC_SIZEWE)) };
            1
        }
        WM_LBUTTONDOWN if over_grip(panel, point_of(lparam).0) => {
            begin_resize(main, panel);
            0
        }
        WM_MOUSEMOVE if drag_resize(main, panel, point_of(lparam).0) => 0,
        WM_LBUTTONUP if finish_resize(main, true) => 0,
        WM_LBUTTONDBLCLK if over_grip(panel, point_of(lparam).0) => {
            save_width(main, DEFAULT_SIDEBAR_WIDTH);
            0
        }
        // Capture taken away mid-drag (a task switch, a dialog): keep and save what was reached.
        WM_CAPTURECHANGED if finish_resize(main, false) => 0,
        // Everything else a view may want goes to it first. Wheel and context-menu positions are
        // screen coordinates; the view converts them.
        WM_LBUTTONDOWN | WM_MOUSEMOVE | WM_LBUTTONUP | WM_LBUTTONDBLCLK | WM_CAPTURECHANGED
        | WM_MOUSELEAVE | WM_RBUTTONDOWN | WM_RBUTTONUP | WM_CONTEXTMENU | WM_MOUSEWHEEL
        | WM_KEYDOWN | WM_CHAR => route(main, panel, message, wparam, lparam),
        WM_SETFOCUS | WM_KILLFOCUS => {
            unsafe { InvalidateRect(panel, std::ptr::null(), 0) };
            0
        }
        _ => unsafe { DefWindowProcW(panel, message, wparam, lparam) },
    }
}

/// Hands `message` to the shown view: keys to `view_key`, the rest to `view_mouse`.
/// `DefWindowProcW` handles whatever the view leaves (`None`).
fn route(main: HWND, panel: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let answer = PanelView::of(current_view(main)).and_then(|view| match message {
        WM_KEYDOWN | WM_CHAR => view_key(main, view, panel, message, wparam, lparam),
        _ => view_mouse(main, view, panel, message, wparam, lparam),
    });
    answer.unwrap_or_else(|| unsafe { DefWindowProcW(panel, message, wparam, lparam) })
}

/// The empty part of the header is caption: the main window's hit test decides there.
fn panel_hit_test(main: HWND, panel: HWND, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let (x, y) = point_of(lparam);
    let mut point = POINT { x, y };
    unsafe { ScreenToClient(panel, &mut point) };
    let dpi = unsafe { GetDpiForWindow(panel) }.max(96);
    let caption = point.y >= 0
        && point.y < scale(HEADER_HEIGHT_96, dpi)
        && !over_grip(panel, point.x)
        && PanelView::of(current_view(main))
            .is_some_and(|view| header_is_caption(main, view, panel, point.x, point.y));
    if caption {
        HTTRANSPARENT as LRESULT
    } else {
        unsafe { DefWindowProcW(panel, WM_NCHITTEST, wparam, lparam) }
    }
}

fn paint_panel(main: HWND, panel: HWND) {
    let view = PanelView::of(current_view(main));
    paint_buffered(panel, |dc, client| {
        let paint = view_paint(main, panel, dc, client);
        unsafe { fill(dc, client, paint.background) };
        if let Some(view) = view {
            paint_view(main, view, &paint);
        }
        // A hairline where the editor area begins, over whatever the view painted.
        unsafe {
            fill(
                dc,
                RECT {
                    left: (client.right - 1).max(client.left),
                    ..client
                },
                paint.palette.strip_background,
            );
        }
    });
}

/// Paints `view` over the panel's background. Task 10 gives the Notebook view its own paint,
/// and Task 12 the other two.
fn paint_view(_main: HWND, view: PanelView, paint: &ViewPaint) {
    match view {
        PanelView::Notebook | PanelView::Search | PanelView::Favorites => {
            paint_header_title(view, paint);
        }
    }
}

/// A view's header title alone, until the view paints itself.
fn paint_header_title(view: PanelView, paint: &ViewPaint) {
    let inset = scale(HEADER_INSET_96, paint.dpi);
    let title = RECT {
        left: paint.client.left + inset,
        right: (paint.client.right - inset).max(paint.client.left + inset),
        bottom: (paint.client.top + scale(HEADER_HEIGHT_96, paint.dpi)).min(paint.client.bottom),
        ..paint.client
    };
    unsafe {
        draw_text(
            paint.hdc,
            view.title(),
            title,
            paint.fonts.bold,
            paint.palette.muted_foreground,
            DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX,
        );
    }
}

/// Mouse input (and `WM_CONTEXTMENU`, `WM_MOUSELEAVE`, `WM_CAPTURECHANGED`) for `view`, with the
/// message's own `wparam` and `lparam`. `None` leaves it to `DefWindowProcW`. Tasks 10 and 12
/// replace these arms with their views' handlers.
fn view_mouse(
    _main: HWND,
    view: PanelView,
    panel: HWND,
    message: u32,
    _wparam: WPARAM,
    _lparam: LPARAM,
) -> Option<LRESULT> {
    match view {
        PanelView::Notebook | PanelView::Search | PanelView::Favorites => {
            (message == WM_LBUTTONDOWN).then(|| {
                unsafe { SetFocus(panel) };
                0
            })
        }
    }
}

/// `WM_KEYDOWN` and `WM_CHAR` while the panel has the focus. `None` leaves the key to
/// `DefWindowProcW`. Task 10 handles the tree's keys and Task 12 the lists' keys.
fn view_key(
    _main: HWND,
    view: PanelView,
    _panel: HWND,
    _message: u32,
    _wparam: WPARAM,
    _lparam: LPARAM,
) -> Option<LRESULT> {
    match view {
        PanelView::Notebook | PanelView::Search | PanelView::Favorites => None,
    }
}

/// Whether header point `x`, `y` (panel client coordinates) is empty, so the window drags from
/// it. Task 10 excludes the Notebook header's buttons and title, and Task 12 the Favorites
/// header's Open notebook… button.
fn header_is_caption(_main: HWND, view: PanelView, _panel: HWND, _x: i32, _y: i32) -> bool {
    match view {
        PanelView::Notebook | PanelView::Search | PanelView::Favorites => true,
    }
}

fn over_grip(panel: HWND, x: i32) -> bool {
    let mut client = RECT::default();
    unsafe { GetClientRect(panel, &mut client) };
    let dpi = unsafe { GetDpiForWindow(panel) }.max(96);
    x >= client.right - scale(GRIP_WIDTH_96, dpi) && x < client.right
}

fn pointer_over_grip(panel: HWND) -> bool {
    let mut point = POINT::default();
    unsafe { GetCursorPos(&mut point) } != 0
        && unsafe { ScreenToClient(panel, &mut point) } != 0
        && over_grip(panel, point.x)
}

fn resizing(main: HWND) -> bool {
    with_sidebar(main, |sidebar| sidebar.drag_width.is_some()).unwrap_or(false)
}

fn begin_resize(main: HWND, panel: HWND) {
    let Some((_, width)) = open_state(main) else {
        return;
    };
    with_sidebar(main, |sidebar| sidebar.drag_width = Some(width));
    unsafe { SetCapture(panel) };
}

/// While the edge is dragged: `x` (panel coordinates) is the new width, since the panel's left
/// edge stays put. Reports whether a drag is in progress.
fn drag_resize(main: HWND, panel: HWND, x: i32) -> bool {
    if !resizing(main) {
        return false;
    }
    let dpi = unsafe { GetDpiForWindow(panel) }.max(96);
    let width = drag_width_96(x, dpi);
    let changed = with_sidebar(main, |sidebar| sidebar.drag_width.replace(width) != Some(width))
        .unwrap_or(false);
    if changed {
        layout_editor_and_find_bar(main);
        invalidate_title_strip(main);
    }
    true
}

/// Ends a drag and saves the width once. `release` is set when the button went up; otherwise
/// capture was taken away. Reports whether a drag was in progress.
fn finish_resize(main: HWND, release: bool) -> bool {
    let Some(width) = with_sidebar(main, |sidebar| sidebar.drag_width.take()).flatten() else {
        return false;
    };
    if release {
        unsafe { ReleaseCapture() };
    }
    save_width(main, width);
    true
}

fn save_width(main: HWND, width: u16) {
    change_setting(main, |settings| {
        (settings.sidebar_width != width).then(|| {
            settings.sidebar_width = width;
            ("sidebar_width", width.to_string())
        })
    });
    layout_editor_and_find_bar(main);
    invalidate_title_strip(main);
}
```

`layout` never writes a setting, so the squeeze on a narrow window is never saved. Only `finish_resize` and the edge double-click call `save_width`.

- [ ] **Step 9: Wire the sidebar into the app and the main window**

`src/window/mod.rs`: add `pub(crate) mod activity_bar;`, `pub(crate) mod side_panel;` and `pub(crate) mod tooltip;`, in alphabetical order with the others.

`src/app.rs`: add the field after `library`:

```rust
    pub(crate) library: crate::window::library_host::LibraryHost,
    /// The activity bar and side panel; present only in notes mode.
    pub(crate) sidebar: Option<crate::window::side_panel::Sidebar>,
```

In `App::new`, initialize it after `library: …`:

```rust
            sidebar: None,
```

`src/window/main_window.rs`:

1. Make `fn change_setting` and `fn open_command_palette` `pub(crate)`. Add this after `current_palette`:

```rust
/// The sidebar's fonts at the window's DPI, created on first use and again after a DPI change.
/// Null handles without a sidebar. Call it with nothing of the App borrowed.
pub(crate) fn ui_fonts(hwnd: HWND) -> crate::window::side_panel::UiFonts {
    let dpi = unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForWindow(hwnd) }.max(96);
    unsafe { app_ptr(hwnd) }
        .and_then(|mut app| {
            unsafe { app.as_mut() }
                .sidebar
                .as_mut()
                .map(|sidebar| sidebar.fonts(dpi))
        })
        .unwrap_or_default()
}
```

2. Replace `layout_editor_and_find_bar`. The sidebar is placed first, even with no editor yet. Then everything else goes right of `left`:

```rust
/// Lays out the sidebar, then the find bar, the name box, the preview and the editor right of it,
/// below the title strip. The sole layout choke point for all of them.
pub(crate) fn layout_editor_and_find_bar(hwnd: HWND) {
    let mut rect = RECT::default();
    unsafe {
        GetClientRect(hwnd, &mut rect);
    }
    let dpi = unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForWindow(hwnd) }.max(96);
    crate::window::side_panel::layout(hwnd, rect, dpi);
    layout_command_palette(hwnd);
    let Some(editor_hwnd) = (unsafe { editor_hwnd(hwnd) }) else {
        return;
    };
    let title_height = title_layout(hwnd).height + menu_band_height(hwnd);
    let left = crate::window::side_panel::left_edge(hwnd);
    let width = (rect.right - rect.left - left).max(0);
    let font = title_chrome(hwnd).1.text();
    let find_bar_height = unsafe { app_ptr(hwnd) }
        .and_then(|app| {
            let bar = unsafe { app.as_ref() }.find_bar.as_ref()?;
            bar.layout(left, width, title_height, dpi, font);
            bar.is_visible().then(|| find_bar::find_bar_height(dpi))
        })
        .unwrap_or(0);
    // Opening either bar closes the other, so at most one of the two bands is ever reserved.
    let name_box_height = unsafe { app_ptr(hwnd) }
        .and_then(|app| {
            let name_box = unsafe { app.as_ref() }.name_box.as_ref()?;
            name_box.layout(left, width, title_height + find_bar_height, dpi, font);
            name_box
                .is_visible()
                .then(|| crate::window::name_box::name_box_height(dpi))
        })
        .unwrap_or(0);
    let content_top = title_height + find_bar_height + name_box_height;
    let status_height = status_bar_height(hwnd);
    let area = RECT {
        left,
        top: content_top,
        right: left + width,
        bottom: (rect.bottom - rect.top - status_height).max(content_top),
    };
    let rects = crate::window::preview_host::layout(hwnd, area, dpi);
    if let Some(editor_rect) = rects.editor {
        unsafe {
            MoveWindow(
                editor_hwnd,
                editor_rect.left,
                editor_rect.top,
                editor_rect.right - editor_rect.left,
                editor_rect.bottom - editor_rect.top,
                1,
            );
        }
    }
}
```

3. Replace `layout_command_palette` so the palette centers in the editor area:

```rust
/// Overlays the palette at the top of the editor area, even with no tab open (New and Open stay
/// available then), below a visible find bar or name box so both stay usable.
fn layout_command_palette(hwnd: HWND) {
    if !with_command_palette(hwnd, CommandPalette::is_visible).unwrap_or(false) {
        return;
    }
    let top = title_layout(hwnd).height + menu_band_height(hwnd) + bar_band_height(hwnd);
    let mut rect = RECT::default();
    unsafe {
        GetClientRect(hwnd, &mut rect);
    }
    let dpi = unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForWindow(hwnd) }.max(96);
    let font = title_chrome(hwnd).1.text();
    let left = crate::window::side_panel::left_edge(hwnd);
    let width = (rect.right - rect.left - left).max(0);
    if let Some(mut app) = unsafe { app_ptr(hwnd) }
        && let Some(palette) = unsafe { app.as_mut() }.command_palette.as_mut()
    {
        palette.measure(width, dpi, font);
    }
    with_command_palette(hwnd, |palette| {
        palette.apply_layout(left, width, top, dpi, font)
    });
}
```

4. In `menu_headings`, pass the left edge:

```rust
    menu_band::heading_rects(
        &widths,
        crate::window::side_panel::left_edge(hwnd),
        title_layout(hwnd).height,
        dpi,
    )
```

5. In `scroll_tabs`, replace `point.x < 0` with `point.x < layout.tabs.left`, so a wheel over the sidebar strip never scrolls tabs.

6. In `refilter_command_palette`, replace the `else` branch so sidebar commands are listed only in notes mode:

```rust
    } else {
        let has_tabs = tab_count(hwnd) > 0;
        let markdown = crate::window::preview_host::buttons_visible(hwnd);
        let sidebar = notes_mode_enabled(hwnd);
        let entries = command_palette::filter_entries(&query, |command| {
            (has_tabs || !command.needs_document())
                && (markdown || !command.is_markdown_preview())
                && (sidebar || !command.is_sidebar())
        });
        if let Some(mut app) = unsafe { app_ptr(hwnd) }
            && let Some(palette) = unsafe { app.as_mut() }.command_palette.as_mut()
        {
            palette.set_entries(entries);
        }
    }
```

7. In `open_menu`, inside `if index == crate::window::menu_band::VIEW_MENU_INDEX { … }`, add after the `set_markdown_preview_enabled` call:

```rust
            menus::set_sidebar_enabled(menu, notes_mode_enabled(hwnd));
```

8. In `execute_command`, add these arms before the final `_ =>` arm:

```rust
        CommandId::ToggleSidebar => crate::window::side_panel::toggle(hwnd),
        CommandId::ShowNotebookView => crate::window::side_panel::show_view(
            hwnd,
            crate::config::SidebarView::Notebook,
            true,
        ),
        CommandId::ShowSearchView => crate::window::side_panel::show_view(
            hwnd,
            crate::config::SidebarView::Search,
            true,
        ),
        CommandId::ShowFavoritesView => crate::window::side_panel::show_view(
            hwnd,
            crate::config::SidebarView::Favorites,
            true,
        ),
```

In the `CommandId::ToggleNotesMode` arm, add after `crate::window::library_host::notes_mode_changed(hwnd, enabled);`:

```rust
            crate::window::side_panel::notes_mode_changed(hwnd, enabled);
```

9. In `initialize_editor_with`, after the `unsafe { install_editor(…)?; record_milestone(…)?; }` block and before `Ok(editor_hwnd)`, add:

```rust
    // The sidebar comes with the window, before first paint, from the settings bootstrap read.
    crate::window::side_panel::notes_mode_changed(hwnd, notes_mode_enabled(hwnd));
```

10. In `apply_loaded_settings`, after `apply_editor_settings(hwnd);`, reconcile the sidebar with the applied `notes_mode` and `sidebar_view`. After a bootstrap preload nothing changes here; in-process tests read `fastpad.ini` only now:

```rust
    let notes_mode = notes_mode_enabled(hwnd);
    crate::window::side_panel::notes_mode_changed(hwnd, notes_mode);
```

11. In `refresh_tabs`, add at the end:

```rust
    crate::window::side_panel::active_tab_changed(hwnd);
```

12. At the end of `apply_theme`, after `crate::window::preview_host::refresh_appearance(hwnd);`, add:

```rust
    crate::window::side_panel::refresh(hwnd);
```

13. In the `WM_DPICHANGED` arm, after the `set_text_padding` block and before `0`, add:

```rust
            // The suggested rectangle may keep the size, and then no WM_SIZE re-lays out the
            // sidebar and bands for the new DPI.
            layout_editor_and_find_bar(hwnd);
```

14. Over the sidebar's top strip, the caption behaves like a normal caption. Double-click maximizes and right-click shows the system menu. Elsewhere the empty tab strip still opens a tab. Replace the three `HTCAPTION` arms with:

```rust
        // The empty tab-strip space is the only caption: double-clicking it opens a tab, VSCode
        // style, instead of maximizing. Over the sidebar's top strip it maximizes as usual.
        WM_NCLBUTTONDBLCLK if wparam == HTCAPTION as usize && !over_sidebar(hwnd, lparam) => {
            execute_command(hwnd, CommandId::New);
            0
        }
        // Its context menu replaces the system menu; Alt+Space still opens that.
        WM_NCRBUTTONDOWN if wparam == HTCAPTION as usize && !over_sidebar(hwnd, lparam) => 0,
        WM_NCRBUTTONUP if wparam == HTCAPTION as usize && !over_sidebar(hwnd, lparam) => {
            let mut point = windows_sys::Win32::Foundation::POINT {
                x: (lparam as u32 & 0xffff) as u16 as i16 as i32,
                y: ((lparam as u32 >> 16) & 0xffff) as u16 as i16 as i32,
            };
            unsafe {
                windows_sys::Win32::Graphics::Gdi::ScreenToClient(hwnd, &mut point);
            }
            let has_tabs = tab_count(hwnd) > 0;
            if let Some(command) = menus::show_tab_strip_menu(hwnd, point.x, point.y, has_tabs) {
                execute_command(hwnd, command);
            }
            0
        }
```

Add the helper next to `client_title_target`:

```rust
/// Whether the screen point in a non-client mouse message's `lparam` is over the sidebar.
fn over_sidebar(hwnd: HWND, lparam: LPARAM) -> bool {
    let mut point = windows_sys::Win32::Foundation::POINT {
        x: (lparam as u32 & 0xffff) as u16 as i16 as i32,
        y: ((lparam as u32 >> 16) & 0xffff) as u16 as i16 as i32,
    };
    unsafe {
        windows_sys::Win32::Graphics::Gdi::ScreenToClient(hwnd, &mut point);
    }
    point.x < crate::window::side_panel::left_edge(hwnd)
}
```

15. `load_settings` applies what `bootstrap::run` already read, and reads `fastpad.ini` itself only when nothing was preloaded (in-process tests). Replace it with:

```rust
/// Runs only inside `WM_FASTPAD_LOAD_SETTINGS`: applies the settings `bootstrap::run` read before
/// the window existed (or resolves and parses `fastpad.ini` now when nothing was preloaded),
/// applies the editor view settings in place, and queues every rejected line as a non-modal
/// notification.
fn load_settings(hwnd: HWND) {
    let preloaded = unsafe { app_ptr(hwnd) }.and_then(|mut app| {
        let app = unsafe { app.as_mut() };
        let warnings = app.preloaded_settings_warnings.take()?;
        Some((app.settings.clone(), warnings))
    });
    let (settings, warnings) = preloaded.unwrap_or_else(crate::config::load);
    apply_loaded_settings(hwnd, settings, warnings);
    start_recovery_timer(hwnd);
}
```

`src/app.rs`: add after the `sidebar` field, and initialize it with `preloaded_settings_warnings: None,` in `App::new`:

```rust
    /// The warnings of the `fastpad.ini` that `bootstrap::run` read into `settings` before the
    /// window existed. `Some` until `WM_FASTPAD_LOAD_SETTINGS` reports them.
    pub(crate) preloaded_settings_warnings: Option<Vec<crate::config::SettingWarning>>,
```

`src/bootstrap.rs`: in `run`, right after `app.instance_mutex = instance_mutex;`, read the settings so the first frame lays the sidebar out from them (Global Constraints: one small file read before the window is created; Task 14's startup bench gate checks it):

```rust
    // The sidebar's first frame uses the saved view and width, so fastpad.ini is read before the
    // window exists. Its warnings are reported by WM_FASTPAD_LOAD_SETTINGS, once chrome is up.
    let (settings, warnings) = crate::config::load();
    app.settings = settings;
    app.preloaded_settings_warnings = Some(warnings);
```

- [ ] **Step 10: Run the tests to verify they pass**

Run: `cargo test --lib -- window::side_panel window::activity_bar window::titlebar window::commands window::menus window::command_palette window::menu_band sidebar squeezes --test-threads=1`
Expected: all pass.

Then run the existing tests the layout offset touches:

Run: `cargo test --lib -- find_bar menu_band command_palette keyboard_shortcuts tab_scroll_thumb accessibility name_box first_save notes_mode corrupt_settings --test-threads=1`
Expected: all pass. With notes mode on (the default), every in-process window now has a sidebar. These tests measure positions from `title_layout` and child rectangles, not from x = 0.

- [ ] **Step 11: Lint and format**

Run: `cargo clippy --all-targets -- -D warnings` and `cargo fmt --all -- --check`
Expected: no warnings and no diff.

- [ ] **Step 12: Commit**

```bash
git add src/app.rs src/bootstrap.rs src/window
git commit -m "feat(window): activity bar and side panel shell with view commands"
```

---

### Task 7: The virtual row list

**Files:**
- Create: `src/window/row_list.rs`
- Modify: `src/window/mod.rs` (`pub(crate) mod row_list;`)

**Interfaces:**
- Consumes: `crate::window::palette::Palette` and `crate::window::panel::fill`.
- Produces (the contract):
  - `RowListState { count, selected, hover, top, row_height }` (the fields are `pub(crate)`; one private field keeps wheel fractions).
  - `ListKey::{Up, Down, Home, End, PageUp, PageDown}`.
  - `RowListState::{new, set_count, visible_rows, row_at, row_top, move_selection, select, ensure_visible, scroll_lines, thumb, drag_thumb}`.
  - `row_list::paint(hdc, area, state, palette, focused, draw_row)` and `RowLook { selected, hover, focused }`.
- Produces (also used by Tasks 10 and 12):
  - `ListKey::from_virtual_key(key: u32) -> Option<ListKey>`.
  - `RowListState::wheel(&mut self, delta: i32, lines_per_notch: u32, height: i32) -> bool`, `RowListState::set_hover(&mut self, hover: Option<usize>) -> bool` and `RowListState::thumb_hit(&self, x: i32, y: i32, width: i32, height: i32) -> Option<i32>` (the grab offset).
  - `row_list::thumb_width(row_height: i32) -> i32`.

Every `height` argument is the list area's pixel height, and `y` is relative to the list's top. The state is plain data, so every rule is unit-tested without a window. `paint` draws into whatever DC the caller hands it. The panel's paint is already double-buffered by `side_panel::paint_buffered`.

- [ ] **Step 1: Write the failing tests**

`src/window/row_list.rs`, the tests module:

```rust
#[cfg(test)]
mod tests {
    use super::{ListKey, RowListState, RowLook, paint, thumb_width};
    use crate::window::palette::Palette;

    fn list(count: usize) -> RowListState {
        let mut list = RowListState::new(26);
        list.set_count(count);
        list
    }

    #[test]
    fn a_new_list_is_empty_and_ignores_keys() {
        let mut list = RowListState::new(0);
        assert_eq!(list.row_height, 1, "a zero row height would divide by zero");
        assert_eq!(list.selected, None);
        assert!(!list.move_selection(ListKey::Down, 100));
        list.select(3, 100);
        assert_eq!(list.selected, None);
        assert_eq!(list.visible_rows(100), 0);
        assert_eq!(list.thumb(100), None);
    }

    #[test]
    fn selection_and_hover_are_clamped_when_the_count_shrinks() {
        // Break caught: a rescan that removes rows under the selection leaving an index past the
        // end, which the next paint or Enter would read out of bounds.
        let mut list = list(10);
        list.select(9, 26 * 3);
        list.hover = Some(8);
        list.set_count(5);
        assert_eq!(list.selected, Some(4));
        assert_eq!(list.hover, None);
        assert!(list.top <= 4);
        list.set_count(0);
        assert_eq!(list.selected, None);
        assert_eq!(list.top, 0);
    }

    #[test]
    fn keyboard_moves_the_selection_and_keeps_it_in_view() {
        let height = 26 * 3;
        let mut list = list(5);
        assert!(list.move_selection(ListKey::Down, height));
        assert_eq!(list.selected, Some(0), "the first key selects the first row in view");
        assert!(list.move_selection(ListKey::Down, height));
        assert_eq!(list.selected, Some(1));
        assert!(list.move_selection(ListKey::Up, height));
        assert!(!list.move_selection(ListKey::Up, height));
        assert!(list.move_selection(ListKey::End, height));
        assert_eq!((list.selected, list.top), (Some(4), 2));
        assert!(list.move_selection(ListKey::Home, height));
        assert_eq!((list.selected, list.top), (Some(0), 0));
        for _ in 0..3 {
            list.move_selection(ListKey::Down, height);
        }
        assert_eq!((list.selected, list.top), (Some(3), 1));
    }

    #[test]
    fn paging_through_ten_thousand_rows_never_leaves_the_list() {
        // Break caught: PageDown past the end (a selection or top beyond the rows, a panic on the
        // next paint), a page that skips rows without showing them, or a selection scrolled out
        // of view.
        let height = 26 * 20 + 13; // 20 whole rows and part of one more
        let mut list = list(10_000);
        let in_view = |list: &RowListState| {
            let selected = list.selected.unwrap();
            assert!(selected < 10_000);
            assert!(list.top <= selected && selected < list.top + 20, "selection out of view");
            assert!(list.top <= 10_000 - 20, "top past the last full page");
        };
        assert!(list.move_selection(ListKey::PageDown, height));
        assert_eq!(list.selected, Some(0));
        let mut steps = 0;
        while list.move_selection(ListKey::PageDown, height) {
            steps += 1;
            in_view(&list);
            assert!(steps <= 10_000);
        }
        assert_eq!(list.selected, Some(9_999));
        assert_eq!(list.top, 10_000 - 20);
        assert_eq!(steps, 9_999usize.div_ceil(19), "a page moves by the rows in view less one");
        while list.move_selection(ListKey::PageUp, height) {
            in_view(&list);
        }
        assert_eq!((list.selected, list.top), (Some(0), 0));
        assert!(list.move_selection(ListKey::End, height));
        assert_eq!(list.selected, Some(9_999));
        assert!(!list.move_selection(ListKey::Down, height));
        list.set_count(3);
        list.ensure_visible(2, height);
        assert_eq!((list.selected, list.top), (Some(2), 0));
    }

    #[test]
    fn ensure_visible_scrolls_the_least_needed() {
        let height = 26 * 4;
        let mut list = list(100);
        list.ensure_visible(10, height);
        assert_eq!(list.top, 7, "the row lands on the last full line");
        list.ensure_visible(8, height);
        assert_eq!(list.top, 7, "a row already in view scrolls nothing");
        list.ensure_visible(2, height);
        assert_eq!(list.top, 2);
        list.ensure_visible(1_000, height);
        assert_eq!(list.top, 96);
    }

    #[test]
    fn hit_testing_maps_y_to_rows_and_back() {
        let mut list = list(10);
        assert_eq!(list.visible_rows(100), 4);
        assert_eq!(list.row_at(0), Some(0));
        assert_eq!(list.row_at(25), Some(0));
        assert_eq!(list.row_at(26), Some(1));
        assert_eq!(list.row_at(-1), None);
        assert_eq!(list.row_at(26 * 10), None);
        assert_eq!(list.row_top(1), Some(26));
        list.top = 2;
        assert_eq!(list.row_at(0), Some(2));
        assert_eq!(list.row_top(1), None);
        assert_eq!(list.row_top(2), Some(0));
        assert_eq!(list.row_top(10), None);
        assert_eq!(list.visible_rows(26 * 20), 8, "never more rows than remain");
        assert!(list.set_hover(Some(3)));
        assert!(!list.set_hover(Some(3)));
    }

    #[test]
    fn wheel_scrolls_whole_lines_and_keeps_touchpad_fractions() {
        // Break caught: a precision touchpad's small wheel deltas rounding to nothing, so the
        // list never scrolls, or a wheel that scrolls past the end.
        let height = 26 * 10;
        let mut list = list(100);
        assert!(list.wheel(-120, 3, height));
        assert_eq!(list.top, 3);
        assert!(list.wheel(120, 3, height));
        assert_eq!(list.top, 0);
        assert!(!list.wheel(120, 3, height), "already at the top");
        assert!(list.wheel(-40, 3, height));
        assert_eq!(list.top, 1);
        assert!(!list.wheel(-20, 3, height));
        assert!(list.wheel(-20, 3, height));
        assert_eq!(list.top, 2);
        assert!(list.wheel(-120, u32::MAX, height), "page scrolling");
        assert_eq!(list.top, 12);
        assert!(list.scroll_lines(1_000, height));
        assert_eq!(list.top, 90);
        assert!(list.scroll_lines(-1_000, height));
        assert_eq!(list.top, 0);
    }

    #[test]
    fn the_thumb_is_sized_by_the_visible_share_and_tracks_the_scroll() {
        let height = 26 * 10;
        let mut list = list(100);
        assert_eq!(list.thumb(height), Some((0, 26)));
        list.top = 45;
        assert_eq!(list.thumb(height), Some((117, 26)));
        list.top = 90;
        assert_eq!(list.thumb(height), Some((234, 26)));
        // Ten thousand rows keep a thumb at least one row tall, so it can be grabbed.
        assert_eq!(super::RowListState { top: 0, ..self::list(10_000) }.thumb(height), Some((0, 26)));
        assert_eq!(self::list(10).thumb(height), None, "no thumb when every row fits");
        let width = 200;
        let bar = width - thumb_width(26);
        assert_eq!(list.thumb_hit(bar, 240, width, height), Some(6));
        assert_eq!(list.thumb_hit(bar - 1, 240, width, height), None);
        assert_eq!(list.thumb_hit(bar, 10, width, height), None);
    }

    #[test]
    fn dragging_the_thumb_scrolls_from_the_first_to_the_last_row() {
        let height = 26 * 10;
        let mut list = list(10_000);
        assert!(list.drag_thumb(10, 10 + 234, height));
        assert_eq!(list.top, 10_000 - 10);
        assert!(list.drag_thumb(0, 117, height));
        assert_eq!(list.top, 4_995);
        assert_eq!(list.thumb(height).unwrap().0, 117);
        assert!(!list.drag_thumb(0, 117, height));
        assert!(list.drag_thumb(0, -500, height));
        assert_eq!(list.top, 0);
        assert!(list.drag_thumb(0, 10_000, height));
        assert_eq!(list.top, 10_000 - 10);
        assert!(!self::list(3).drag_thumb(0, 50, height), "no thumb, no drag");
    }

    #[test]
    fn virtual_keys_map_to_list_keys() {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
            VK_DOWN, VK_END, VK_HOME, VK_LEFT, VK_NEXT, VK_PRIOR, VK_UP,
        };
        let key = |vk: u16| ListKey::from_virtual_key(u32::from(vk));
        assert_eq!(key(VK_UP), Some(ListKey::Up));
        assert_eq!(key(VK_DOWN), Some(ListKey::Down));
        assert_eq!(key(VK_HOME), Some(ListKey::Home));
        assert_eq!(key(VK_END), Some(ListKey::End));
        assert_eq!(key(VK_PRIOR), Some(ListKey::PageUp));
        assert_eq!(key(VK_NEXT), Some(ListKey::PageDown));
        assert_eq!(key(VK_LEFT), None);
    }

    #[test]
    fn painting_draws_only_the_rows_in_view_with_the_selection_and_hover_backgrounds() {
        // Break caught: a paint that walks every one of ten thousand rows (a stall per frame),
        // draws past the list's bottom, or shows a focused selection in the unfocused color.
        use windows_sys::Win32::Foundation::RECT;
        use windows_sys::Win32::Graphics::Gdi::{
            CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetPixel,
            ReleaseDC, SelectObject,
        };
        let palette = Palette {
            selection_background: 0x0000_00ff,
            inactive_selection_background: 0x0000_ff00,
            hover_background: 0x00ff_0000,
            line_number_foreground: 0x0000_ffff,
            editor_background: 0x0080_8080,
            ..Palette::neutral()
        };
        let mut state = RowListState::new(20);
        state.set_count(10_000);
        state.top = 5_000;
        state.selected = Some(5_001);
        state.hover = Some(5_002);
        let (width, height) = (100, 205);
        let area = RECT {
            left: 0,
            top: 0,
            right: width,
            bottom: height,
        };
        unsafe {
            let screen = GetDC(std::ptr::null_mut());
            let dc = CreateCompatibleDC(screen);
            let bitmap = CreateCompatibleBitmap(screen, width, height);
            let previous = SelectObject(dc, bitmap);
            for focused in [true, false] {
                crate::window::panel::fill(dc, area, palette.editor_background);
                let mut drawn: Vec<(usize, i32, RowLook)> = Vec::new();
                paint(dc, area, &state, &palette, focused, &mut |_, index, rect, look| {
                    drawn.push((index, rect.top, look));
                });
                assert_eq!(
                    drawn.iter().map(|(index, ..)| *index).collect::<Vec<_>>(),
                    (5_000..5_011).collect::<Vec<_>>(),
                    "only the 11 rows at least partly in view"
                );
                assert_eq!(drawn[3].1, 60);
                assert!(drawn[1].2.selected && drawn[1].2.focused == focused);
                assert!(drawn[2].2.hover && !drawn[2].2.selected);
                let selection = if focused {
                    palette.selection_background
                } else {
                    palette.inactive_selection_background
                };
                assert_eq!(GetPixel(dc, 10, 20 + 5), selection);
                assert_eq!(GetPixel(dc, 10, 40 + 5), palette.hover_background);
                assert_eq!(GetPixel(dc, 10, 5), palette.editor_background);
                let (thumb_top, _) = state.thumb(height).unwrap();
                assert_eq!(
                    GetPixel(dc, width - 1, thumb_top + 1),
                    palette.line_number_foreground
                );
            }
            SelectObject(dc, previous);
            DeleteObject(bitmap);
            DeleteDC(dc);
            ReleaseDC(std::ptr::null_mut(), screen);
        }
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib window::row_list -- --test-threads=1`
Expected: a compile error, because `RowListState`, `ListKey`, `RowLook`, `paint` and `thumb_width` don't exist yet.

- [ ] **Step 3: Implement it**

Add `pub(crate) mod row_list;` to `src/window/mod.rs`. `src/window/row_list.rs`, above the tests module:

```rust
//! A virtual row list shared by the sidebar's views: selection, hover, scrolling, keyboard
//! paging, hit-testing and a thin scroll thumb over any number of fixed-height rows. The state is
//! plain data. `paint` draws only the rows in view, into the caller's double-buffered DC.
//!
//! Every `height` is the list area's height in pixels, and every `y` is relative to its top.
// Task 10 is the first non-test user; it removes this.
#![cfg_attr(
    not(test),
    allow(dead_code, reason = "the sidebar views use it from Task 10")
)]

use crate::window::palette::Palette;
use crate::window::panel::fill;
use windows_sys::Win32::Foundation::RECT;
use windows_sys::Win32::Graphics::Gdi::{HDC, IntersectClipRect, RestoreDC, SaveDC};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    VK_DOWN, VK_END, VK_HOME, VK_NEXT, VK_PRIOR, VK_UP,
};
use windows_sys::Win32::UI::WindowsAndMessaging::WHEEL_DELTA;

/// `SPI_GETWHEELSCROLLLINES` reports this for "one screen at a time".
const WHEEL_PAGESCROLL: u32 = u32::MAX;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ListKey {
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
}

impl ListKey {
    /// The list key a `WM_KEYDOWN` virtual key means, if any.
    pub(crate) fn from_virtual_key(key: u32) -> Option<Self> {
        match u16::try_from(key).ok()? {
            VK_UP => Some(Self::Up),
            VK_DOWN => Some(Self::Down),
            VK_HOME => Some(Self::Home),
            VK_END => Some(Self::End),
            VK_PRIOR => Some(Self::PageUp),
            VK_NEXT => Some(Self::PageDown),
            _ => None,
        }
    }
}

/// How `paint` shows one row; `draw_row` gets it to pick text colors.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RowLook {
    pub(crate) selected: bool,
    pub(crate) hover: bool,
    pub(crate) focused: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RowListState {
    pub(crate) count: usize,
    pub(crate) selected: Option<usize>,
    pub(crate) hover: Option<usize>,
    /// The first row in view.
    pub(crate) top: usize,
    pub(crate) row_height: i32,
    /// Wheel movement short of one line, kept for the next wheel message.
    wheel_remainder: i32,
}

/// The scroll thumb's width for rows `row_height` tall: 6 px at the 26 px, 96-DPI row, so it
/// scales with DPI through the row height.
pub(crate) const fn thumb_width(row_height: i32) -> i32 {
    let width = row_height * 6 / 26;
    if width < 2 { 2 } else { width }
}

impl RowListState {
    pub(crate) fn new(row_height: i32) -> Self {
        Self {
            count: 0,
            selected: None,
            hover: None,
            top: 0,
            row_height: row_height.max(1),
            wheel_remainder: 0,
        }
    }

    /// A new row count. A selection past the end moves to the last row, a hover past it clears,
    /// and `top` stays on a row. The next `ensure_visible` or scroll fills the view again.
    pub(crate) fn set_count(&mut self, count: usize) {
        self.count = count;
        self.selected = match (self.selected, count.checked_sub(1)) {
            (Some(selected), Some(last)) => Some(selected.min(last)),
            _ => None,
        };
        if self.hover.is_some_and(|hover| hover >= count) {
            self.hover = None;
        }
        self.top = self.top.min(count.saturating_sub(1));
    }

    /// Rows wholly in view, at least one: what a page is measured in.
    fn full_rows(&self, height: i32) -> usize {
        (height.max(0) / self.row_height).max(1) as usize
    }

    /// The last `top` that still fills the view.
    fn max_top(&self, height: i32) -> usize {
        self.count.saturating_sub(self.full_rows(height))
    }

    /// Rows at least partly in view from `top`: the rows `paint` draws.
    pub(crate) fn visible_rows(&self, height: i32) -> usize {
        let fits = (height.max(0) as usize).div_ceil(self.row_height as usize);
        fits.min(self.count.saturating_sub(self.top))
    }

    pub(crate) fn row_at(&self, y: i32) -> Option<usize> {
        if y < 0 {
            return None;
        }
        let index = self.top.checked_add((y / self.row_height) as usize)?;
        (index < self.count).then_some(index)
    }

    /// `index`'s top edge relative to the list's top, or `None` above the view or past the end.
    /// A row below the view has a top beyond `height`.
    pub(crate) fn row_top(&self, index: usize) -> Option<i32> {
        if index < self.top || index >= self.count {
            return None;
        }
        i32::try_from(index - self.top)
            .ok()?
            .checked_mul(self.row_height)
    }

    /// Sets the hovered row; reports whether it changed.
    pub(crate) fn set_hover(&mut self, hover: Option<usize>) -> bool {
        let hover = hover.filter(|&index| index < self.count);
        std::mem::replace(&mut self.hover, hover) != hover
    }

    /// Moves the selection for `key`, keeping it in view. With nothing selected, a move selects
    /// the first row in view. Reports whether the selection or the scroll changed.
    pub(crate) fn move_selection(&mut self, key: ListKey, height: i32) -> bool {
        let Some(last) = self.count.checked_sub(1) else {
            return false;
        };
        let page = self.full_rows(height).saturating_sub(1).max(1);
        let target = match (self.selected, key) {
            (_, ListKey::Home) => 0,
            (_, ListKey::End) => last,
            (None, _) => self.top.min(last),
            (Some(current), ListKey::Up) => current.saturating_sub(1),
            (Some(current), ListKey::Down) => current.saturating_add(1).min(last),
            (Some(current), ListKey::PageUp) => current.saturating_sub(page),
            (Some(current), ListKey::PageDown) => current.saturating_add(page).min(last),
        };
        let before = (self.selected, self.top);
        self.select(target, height);
        (self.selected, self.top) != before
    }

    /// Selects `index` (clamped to the last row) and scrolls it into view.
    pub(crate) fn select(&mut self, index: usize, height: i32) {
        let Some(last) = self.count.checked_sub(1) else {
            self.selected = None;
            return;
        };
        let index = index.min(last);
        self.selected = Some(index);
        self.ensure_visible(index, height);
    }

    /// Scrolls the least needed to show `index` wholly, and keeps the view filled.
    pub(crate) fn ensure_visible(&mut self, index: usize, height: i32) {
        let Some(last) = self.count.checked_sub(1) else {
            self.top = 0;
            return;
        };
        let index = index.min(last);
        let full = self.full_rows(height);
        if index < self.top {
            self.top = index;
        } else if index >= self.top + full {
            self.top = index + 1 - full;
        }
        self.top = self.top.min(self.max_top(height));
    }

    /// Scrolls by `lines` rows (negative is up), within the list. Reports whether it moved.
    pub(crate) fn scroll_lines(&mut self, lines: i32, height: i32) -> bool {
        let before = self.top;
        let top = if lines < 0 {
            self.top.saturating_sub(lines.unsigned_abs() as usize)
        } else {
            self.top.saturating_add(lines as usize)
        };
        self.top = top.min(self.max_top(height));
        self.top != before
    }

    /// A mouse wheel turn of `delta` (`WHEEL_DELTA` per notch, positive away from the user) at
    /// `lines_per_notch` rows per notch (`SPI_GETWHEELSCROLLLINES`; `u32::MAX` pages). Fractions
    /// of a line from precision touchpads carry over to the next turn.
    pub(crate) fn wheel(&mut self, delta: i32, lines_per_notch: u32, height: i32) -> bool {
        let per_notch = if lines_per_notch == WHEEL_PAGESCROLL {
            i32::try_from(self.full_rows(height)).unwrap_or(i32::MAX)
        } else {
            i32::try_from(lines_per_notch).unwrap_or(i32::MAX)
        }
        .clamp(1, 1_000);
        let total = self
            .wheel_remainder
            .saturating_add(delta.saturating_mul(per_notch));
        let notch = WHEEL_DELTA as i32;
        self.wheel_remainder = total % notch;
        self.scroll_lines(-(total / notch), height)
    }

    /// The scroll thumb as (top, length) in a track `height` tall, or `None` when every row
    /// fits. The thumb is at least one row tall, so it can always be grabbed.
    pub(crate) fn thumb(&self, height: i32) -> Option<(i32, i32)> {
        let full = self.full_rows(height);
        if height <= 0 || self.count <= full {
            return None;
        }
        let track = i64::from(height);
        let length = (track * full as i64 / self.count as i64)
            .max(i64::from(self.row_height).min(track))
            .min(track);
        let max_top = self.max_top(height);
        let top = (track - length) * self.top.min(max_top) as i64 / (max_top as i64).max(1);
        Some((top as i32, length as i32))
    }

    /// Where on the thumb a press at `x`, `y` landed (the grab offset for `drag_thumb`), for a
    /// list `width` wide, or `None` off the thumb.
    pub(crate) fn thumb_hit(&self, x: i32, y: i32, width: i32, height: i32) -> Option<i32> {
        let (top, length) = self.thumb(height)?;
        (x >= width - thumb_width(self.row_height) && x < width && y >= top && y < top + length)
            .then_some(y - top)
    }

    /// Drags the thumb so the point grabbed `grab_offset` into it sits at `y`. Reports whether
    /// the list scrolled.
    pub(crate) fn drag_thumb(&mut self, grab_offset: i32, y: i32, height: i32) -> bool {
        let Some((_, length)) = self.thumb(height) else {
            return false;
        };
        let travel = i64::from(height - length).max(1);
        let thumb_top = i64::from(y.saturating_sub(grab_offset)).clamp(0, travel);
        let max_top = self.max_top(height);
        let top = ((thumb_top * max_top as i64 + travel / 2) / travel) as usize;
        let before = self.top;
        self.top = top.min(max_top);
        self.top != before
    }
}

/// Paints the rows in view into `area` of `hdc`. For each row it paints the selection (the
/// unfocused color when `focused` is false) or the hover background, then calls `draw_row` with
/// the row's index, rectangle and look. Last it paints the thin scroll thumb at the right edge.
/// Rows out of view are never touched, and nothing is drawn outside `area`.
pub(crate) fn paint(
    hdc: HDC,
    area: RECT,
    state: &RowListState,
    palette: &Palette,
    focused: bool,
    draw_row: &mut dyn FnMut(HDC, usize, RECT, RowLook),
) {
    let height = area.bottom - area.top;
    if height <= 0 || area.right <= area.left {
        return;
    }
    unsafe {
        let saved = SaveDC(hdc);
        IntersectClipRect(hdc, area.left, area.top, area.right, area.bottom);
        for offset in 0..state.visible_rows(height) {
            let index = state.top + offset;
            let top = area.top + offset as i32 * state.row_height;
            let rect = RECT {
                left: area.left,
                top,
                right: area.right,
                bottom: top + state.row_height,
            };
            let look = RowLook {
                selected: state.selected == Some(index),
                hover: state.hover == Some(index),
                focused,
            };
            if look.selected {
                let background = if focused {
                    palette.selection_background
                } else {
                    palette.inactive_selection_background
                };
                fill(hdc, rect, background);
            } else if look.hover {
                fill(hdc, rect, palette.hover_background);
            }
            draw_row(hdc, index, rect, look);
        }
        if let Some((thumb_top, length)) = state.thumb(height) {
            let thumb = RECT {
                left: area.right - thumb_width(state.row_height),
                top: area.top + thumb_top,
                right: area.right,
                bottom: area.top + thumb_top + length,
            };
            fill(hdc, thumb, palette.line_number_foreground);
        }
        RestoreDC(hdc, saved);
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib window::row_list -- --test-threads=1`
Expected: all 11 tests pass.

- [ ] **Step 5: Lint and format**

Run: `cargo clippy --all-targets -- -D warnings` and `cargo fmt --all -- --check`
Expected: no warnings and no diff. The module-level `cfg_attr` covers the items only tests reach until Task 10.

- [ ] **Step 6: Commit**

```bash
git add src/window/mod.rs src/window/row_list.rs
git commit -m "feat(window): virtual row list for the sidebar views"
```

---

### Task 8: Notebook lifecycle: close, favorites, explicit open, wording

**Files:**
- Modify: `src/window/library_host.rs`, `src/window/commands.rs`, `src/window/command_palette.rs`, `src/window/menus.rs`, `src/window/main_window.rs`, `src/window/messages.rs`, `src/window/mod.rs`

**Interfaces:**
- Consumes:
  - Task 1: `library_host::toggle_pin(hwnd, path)`, the pins-only `picked` with `PickerKind::RecentFolder` only, and `ready_library`.
  - Task 2: `RecentFolders { folders, favorites, closed }` with `toggle_favorite`, `is_favorite` and `set_closed`, and `push` clearing `closed`.
  - Task 6: `side_panel::refresh(hwnd)`.
- Produces:
  - In `library_host`:
    - `close_notebook(hwnd)` and `toggle_notebook_favorite(hwnd)`;
    - `remove_favorite(hwnd, folder: &Path)`;
    - `favorites(hwnd) -> Vec<PathBuf>`, `recent_notebooks(hwnd) -> Vec<PathBuf>` and `is_favorite(hwnd) -> bool`. They answer from a cache of `folders.ini` and never read the disk;
    - `open_listed_notebook(hwnd, folder: &Path)` and `notebook_checked(hwnd, lparam)`, the handler for the worker's answer. The worker's answer carries `show_notebook: Option<bool>`, which Task 12's `open_listed_notebook_in_view` sets.
    - At the 50-favorite cap `toggle_favorite` returns false and the notice is "You can keep up to 50 favorite notebooks.".
  - `CommandId::{CloseNotebook = 180, ToggleNotebookFavorite = 181}`.
  - `WM_FASTPAD_NOTEBOOK_CHECKED = WM_APP + 12`: the existence worker's boxed answer, which the receiver frees.
  - `side_panel::refresh(hwnd)` is called after install, open, close, pin toggle, favorite change, save (`add_note`), rename and delete.

Why a cache: the sidebar lists favorites and recent notebooks when it paints, and the UI thread must not read files for the sidebar (Global Constraints). The startup step (`open_library_step`) already reads `folders.ini` and caches it, and the startup worker, which may add a command-line folder to it, posts a fresh copy with `LIBRARY_READY` unless the user changed it meanwhile. Every change the user makes re-reads the file, changes it, writes it and replaces the cache, all on the UI thread. That's allowed because the user started it, and re-reading keeps another FastPad window's change.

- [ ] **Step 1: Write the failing tests**

In `src/window/commands.rs` `mod tests`:

```rust
#[test]
fn notebook_commands_have_stable_values_and_need_no_document() {
    // Break caught: Close notebook greyed out, or silently ignored, while no tab is open.
    assert_eq!(CommandId::try_from(180), Ok(CommandId::CloseNotebook));
    assert_eq!(CommandId::try_from(181), Ok(CommandId::ToggleNotebookFavorite));
    assert!(!CommandId::CloseNotebook.needs_document());
    assert!(!CommandId::ToggleNotebookFavorite.needs_document());
}
```

In the `src/window/main_window.rs` tests, next to the other `LibraryScratch` tests (`open_note` here is the existing test helper at the autosave tests, not `super::open_note`):

```rust
#[test]
fn closing_the_notebook_saves_its_notes_keeps_the_tabs_and_is_remembered_as_closed() {
    // Break caught: Close notebook dropping a dirty note's edits, closing its tab, or leaving
    // folders.ini without open=none so the next start reopens the notebook anyway.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("close-notebook");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    app_mut(window.hwnd).library.data_dir = Some(scratch.data());
    let path = open_note(&window, &scratch, "a.md", "one");
    editor.set_text("two").unwrap();

    execute_command(window.hwnd, CommandId::CloseNotebook);

    assert_eq!(std::fs::read_to_string(&path).unwrap(), "two");
    assert_eq!(super::tab_count(window.hwnd), 1);
    assert_eq!(
        app_mut(window.hwnd).tabs.active().unwrap().path.as_deref(),
        Some(path.as_path())
    );
    assert_eq!(crate::window::library_host::folder(window.hwnd), None);
    assert!(app_mut(window.hwnd).library.state.is_none());
    let folders = crate::library::local::read_folders(&crate::library::local::folders_file(
        &scratch.data(),
    ));
    assert!(folders.closed);
    assert!(folders.folders.contains(&scratch.folder()), "it stays in the recent list");
    // The tab is a plain file now: an edit is not autosaved.
    editor.set_text("three").unwrap();
    assert_eq!(
        crate::window::library_host::autosave_active(window.hwnd),
        crate::window::library_host::Autosave::NotEligible
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "two");
}

#[test]
fn with_no_notebook_open_ctrl_s_uses_the_save_dialog_like_notes_mode_off() {
    // Break caught: after Close notebook, Ctrl+S on a new tab opening the name box for a notebook
    // that is gone, or the Save As dialog starting in the closed notebook under the tab's label.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("no-notebook-save");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    app_mut(window.hwnd).library.data_dir = Some(scratch.data());
    scratch.install(window.hwnd);
    execute_command(window.hwnd, CommandId::CloseNotebook);
    execute_command(window.hwnd, CommandId::New);
    editor.set_text("Plan\nbody").unwrap();
    let target = scratch.root.join("plan.txt");
    crate::window::answer_next_save_dialog({
        let target = target.clone();
        move |_| Some(target)
    });

    execute_command(window.hwnd, CommandId::Save);

    assert_eq!(std::fs::read_to_string(&target).unwrap(), "Plan\nbody");
    assert!(
        app_mut(window.hwnd)
            .name_box
            .as_ref()
            .is_none_or(|name_box| !name_box.is_visible())
    );
    assert_eq!(
        crate::window::modal::take_last_save_request(),
        Some(("Untitled.txt".to_owned(), None))
    );
}

#[test]
fn the_notebook_favorite_toggles_in_folders_ini_and_the_cached_lists() {
    // Break caught: a star that changes the sidebar but not folders.ini (lost at restart), or
    // favorite lists that read folders.ini on every sidebar paint.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("favorite-notebook");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    app_mut(window.hwnd).library.data_dir = Some(scratch.data());
    scratch.install(window.hwnd);
    let file = crate::library::local::folders_file(&scratch.data());

    execute_command(window.hwnd, CommandId::ToggleNotebookFavorite);
    assert!(crate::window::library_host::is_favorite(window.hwnd));
    assert_eq!(
        crate::window::library_host::favorites(window.hwnd),
        vec![scratch.folder()]
    );
    assert!(crate::library::local::read_folders(&file).is_favorite(&scratch.folder()));
    assert!(
        notices(window.hwnd)
            .iter()
            .any(|n| n.contains("to favorite notebooks"))
    );

    execute_command(window.hwnd, CommandId::ToggleNotebookFavorite);
    assert!(!crate::window::library_host::is_favorite(window.hwnd));
    assert!(!crate::library::local::read_folders(&file).is_favorite(&scratch.folder()));

    execute_command(window.hwnd, CommandId::ToggleNotebookFavorite);
    crate::window::library_host::remove_favorite(window.hwnd, &scratch.folder());
    assert!(crate::window::library_host::favorites(window.hwnd).is_empty());
    assert!(!crate::library::local::read_folders(&file).is_favorite(&scratch.folder()));

    // Listing answers from the cache: a file changed behind FastPad's back is not re-read.
    execute_command(window.hwnd, CommandId::ToggleNotebookFavorite);
    crate::library::local::write_folders(&file, &crate::library::local::RecentFolders::default())
        .unwrap();
    assert_eq!(
        crate::window::library_host::favorites(window.hwnd),
        vec![scratch.folder()]
    );
}

#[test]
fn opening_a_listed_notebook_that_is_missing_says_so_and_changes_nothing() {
    // Break caught: a click on an offline favorite unloading the open notebook first, checking
    // the drive on the UI thread, or falling back to Documents\FastPad the way startup does.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("listed-missing");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    app_mut(window.hwnd).library.data_dir = Some(scratch.data());
    scratch.install(window.hwnd);
    let missing = scratch.root.join("gone");
    let checks = crate::library::folder_checks();

    crate::window::library_host::open_listed_notebook(window.hwnd, &missing);
    assert_eq!(crate::library::folder_checks(), checks, "checked on the worker");
    pump_until(window.hwnd, || {
        notices(window.hwnd)
            .iter()
            .any(|n| n.contains("is not available"))
    });

    assert_eq!(
        crate::window::library_host::folder(window.hwnd),
        Some(scratch.folder())
    );
    assert!(app_mut(window.hwnd).library.state.is_some());
    let folders = crate::library::local::read_folders(&crate::library::local::folders_file(
        &scratch.data(),
    ));
    assert!(!folders.folders.contains(&missing));
}

#[test]
fn opening_a_listed_notebook_switches_once_the_worker_finds_it() {
    // Break caught: an explicit open that switches before the check lands (so a missing folder
    // would already have unloaded the notebook), or never switches at all.
    let _scintilla = load_native_scintilla();
    let first = LibraryScratch::new("listed-a");
    let second = LibraryScratch::new("listed-b");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    app_mut(window.hwnd).library.data_dir = Some(first.data());
    first.install(window.hwnd);

    crate::window::library_host::open_listed_notebook(window.hwnd, &second.folder());
    assert_eq!(
        crate::window::library_host::folder(window.hwnd),
        Some(first.folder()),
        "nothing changes before the check lands"
    );
    pump_until(window.hwnd, || {
        crate::window::library_host::folder(window.hwnd) == Some(second.folder())
    });
    pump_until(window.hwnd, || app_mut(window.hwnd).library.state.is_some());
    assert_eq!(
        crate::window::library_host::recent_notebooks(window.hwnd).first(),
        Some(&second.folder())
    );
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib -- notebook_commands_have_stable closing_the_notebook with_no_notebook_open the_notebook_favorite opening_a_listed_notebook --test-threads=1`
Expected: a compile error (`CloseNotebook`, `ToggleNotebookFavorite`, `favorites`, `open_listed_notebook` and the rest do not exist).

- [ ] **Step 3: Add the commands**

In `src/window/commands.rs`, after the variants Task 6 added (`ShowFavoritesView = 179`), add:

```rust
    CloseNotebook = 180,
    ToggleNotebookFavorite = 181,
```

In `needs_document`'s `matches!` list, add `| Self::CloseNotebook | Self::ToggleNotebookFavorite`. At the end of the `COMMANDS` array in `try_from`, add `CommandId::CloseNotebook, CommandId::ToggleNotebookFavorite,`: the array grows by 2, so raise its length literal by 2.

- [ ] **Step 4: Add the worker message**

In `src/window/messages.rs`, after `WM_FASTPAD_FILES_DROPPED`:

```rust
// Not part of the deferred chain: whether a notebook picked from a list exists, checked on a
// worker because an offline drive can stall, as a `Box` the receiver frees.
pub const WM_FASTPAD_NOTEBOOK_CHECKED: u32 = WM_APP + 12;
```

Add `WM_FASTPAD_NOTEBOOK_CHECKED` to the `pub use messages::{...}` list in `src/window/mod.rs`. In `main_window_proc`'s catch-all arm, right after the `WM_FASTPAD_FILES_DROPPED` branch:

```rust
            if message == crate::window::WM_FASTPAD_NOTEBOOK_CHECKED {
                crate::window::library_host::notebook_checked(hwnd, lparam);
                return 0;
            }
```

- [ ] **Step 5: Cache `folders.ini` in the host**

In `src/window/library_host.rs`, add a field to `LibraryHost`:

```rust
    /// `folders.ini` as last read or written, so the sidebar lists recent and favorite notebooks
    /// without reading the disk. `None` until the startup step or a first change fills it.
    folders: Option<library::local::RecentFolders>,
    /// The user changed `folders.ini` since startup, so the startup worker's copy is older.
    folders_edited: bool,
```

Initialize them with `folders: None, folders_edited: false,` in `LibraryHost::new`.

`open_library_step` (as Task 2 left it) already reads `folders.ini` on the UI thread. Cache what it read, right after `let recent = library::local::read_folders(&library::local::folders_file(&data));` and before the `if recent.closed && launch.is_none()` early return, so the no-notebook state lists recent notebooks even when no worker starts:

```rust
    host(hwnd, |host| host.folders = Some(recent.clone()));
```

Give `Loaded` one more field, after `closed`:

```rust
    /// `folders.ini` as the startup worker read it, after any change it made itself.
    folders: Option<library::local::RecentFolders>,
```

At startup the worker may change `folders.ini` itself (`resolve_startup` remembers a folder named on the command line), so it reads the file again once it has decided. In `spawn_load`, the worker closure as Task 2 left it starts:

```rust
    std::thread::spawn(move || {
        let closed = startup.as_ref().is_some_and(|startup| startup.closed);
        let (folder, notice) = match startup {
            Some(startup) => resolve_startup(startup),
            None => (folder, None),
        };
        let opens_nothing = closed && folder.is_none();
```

Replace those lines with:

```rust
    std::thread::spawn(move || {
        let read_folders = startup.is_some();
        let closed = startup.as_ref().is_some_and(|startup| startup.closed);
        let (folder, notice) = match startup {
            Some(startup) => resolve_startup(startup),
            None => (folder, None),
        };
        let opens_nothing = closed && folder.is_none();
        let folders = read_folders
            .then(|| library::local::read_folders(&library::local::folders_file(&data)));
```

Then build the payload with `folders` too, after `closed: opens_nothing,`: `folders,`. `test_ready_payload` gets `folders: None`.

In `library_ready`, the destructuring Task 2 wrote gains `folders`, and the cache is filled before the `if closed` early return:

```rust
    let Loaded {
        folder,
        result,
        notice,
        closed,
        folders,
        ..
    } = *loaded;
    if let Some(notice) = notice {
        push_notice(hwnd, notice);
    }
    // A change the user made while the worker ran is newer than what the worker read.
    if let Some(folders) = folders {
        host(hwnd, |host| {
            if !host.folders_edited {
                host.folders = Some(folders);
            }
        });
    }
    if closed {
        // The path on the command line was not a folder, and the last session closed its
        // notebook: none is open.
        host(hwnd, |host| host.folder = None);
        super::main_window::invalidate_title_strip(hwnd);
        super::side_panel::refresh(hwnd);
        return;
    }
```

In the same function, replace the `Err(error) => push_notice(...)` arm of `match result` with:

```rust
        Err(error) => {
            push_notice(
                hwnd,
                format!(
                    "FastPad could not load the notebook {}: {error}",
                    folder.display()
                ),
            );
            super::side_panel::refresh(hwnd);
        }
```

Replace `fn recent_folders(hwnd: HWND) -> Vec<PathBuf>` with these helpers and the public accessors:

```rust
/// The cached `folders.ini`. `read_if_unknown` reads the file when nothing is cached yet: only
/// for something the user just asked for (a picker), never for painting.
fn known_folders(hwnd: HWND, read_if_unknown: bool) -> library::local::RecentFolders {
    if let Some(folders) = host(hwnd, |host| host.folders.clone()).flatten() {
        return folders;
    }
    if !read_if_unknown {
        return library::local::RecentFolders::default();
    }
    let Some(data) = data_dir(hwnd) else {
        return library::local::RecentFolders::default();
    };
    let folders = library::local::read_folders(&library::local::folders_file(&data));
    host(hwnd, |host| host.folders = Some(folders.clone()));
    folders
}

/// Re-reads `folders.ini` (another window may have changed it), applies `change`, writes it and
/// caches the result. A write failure is reported; the cache still shows the change.
fn update_folders<R>(
    hwnd: HWND,
    change: impl FnOnce(&mut library::local::RecentFolders) -> R,
) -> Option<R> {
    let data = data_dir(hwnd)?;
    let path = library::local::folders_file(&data);
    let mut folders = library::local::read_folders(&path);
    let result = change(&mut folders);
    if let Err(error) = library::local::write_folders(&path, &folders) {
        push_notice(
            hwnd,
            format!("FastPad could not save {}: {error}", path.display()),
        );
    }
    host(hwnd, |host| {
        host.folders = Some(folders);
        host.folders_edited = true;
    });
    Some(result)
}

/// A notebook's display name: its folder's name.
fn notebook_name(folder: &Path) -> String {
    folder
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| folder.display().to_string())
}

/// Favorite notebooks in `folders.ini` order (the Favorites view sorts them by name).
pub(crate) fn favorites(hwnd: HWND) -> Vec<PathBuf> {
    known_folders(hwnd, false).favorites
}

/// Recent notebooks, most recent first.
pub(crate) fn recent_notebooks(hwnd: HWND) -> Vec<PathBuf> {
    known_folders(hwnd, false).folders
}

/// Whether the open notebook is a favorite.
pub(crate) fn is_favorite(hwnd: HWND) -> bool {
    folder(hwnd).is_some_and(|open| known_folders(hwnd, false).is_favorite(&open))
}

/// Notebook: Toggle favorite, and the Notebook view's star.
pub(crate) fn toggle_notebook_favorite(hwnd: HWND) {
    let Some(open) = folder(hwnd) else {
        push_notice(hwnd, "Open a notebook first.".to_owned());
        return;
    };
    let Some((was, now)) = update_folders(hwnd, |folders| {
        let was = folders.is_favorite(&open);
        (was, folders.toggle_favorite(&open))
    }) else {
        return;
    };
    let name = notebook_name(&open);
    let notice = match (was, now) {
        (false, true) => format!("Added {name} to favorite notebooks."),
        (true, false) => format!("Removed {name} from favorite notebooks."),
        // At the cap, `toggle_favorite` leaves the list alone and returns false.
        _ => "You can keep up to 50 favorite notebooks.".to_owned(),
    };
    push_notice(hwnd, notice);
    super::side_panel::refresh(hwnd);
}

/// Removes `folder` from the favorites; nothing happens if it is not one.
pub(crate) fn remove_favorite(hwnd: HWND, folder: &Path) {
    let removed = update_folders(hwnd, |folders| {
        folders.is_favorite(folder) && !folders.toggle_favorite(folder)
    })
    .unwrap_or(false);
    if removed {
        super::side_panel::refresh(hwnd);
    }
}
```

In `open_recent_folder_picker`, replace `let folders = recent_folders(hwnd);` with `let folders = known_folders(hwnd, true).folders;`.

- [ ] **Step 6: Split `open_folder`, and add the explicit open**

Replace `open_folder` with the version below. The part after the existence check moves into `open_checked_folder`, the path the worker's positive answer takes. Remembering the folder now goes through `update_folders`, so the cache follows. The rest of the old body is unchanged, apart from the wording (Step 9) and the refresh at the end.

```rust
/// Opens `path` as the notebook, flushing the current one first. Open tabs stay open. The folder
/// was just chosen in a dialog, dropped or named by a launch, so it is checked here.
pub(crate) fn open_folder(hwnd: HWND, path: &Path) {
    if !notes_mode(hwnd) {
        push_notice(hwnd, NOTES_MODE_OFF.to_owned());
        return;
    }
    let path = library::normalize_folder(path);
    if !path.is_dir() {
        push_notice(hwnd, format!("{} is not a folder.", path.display()));
        return;
    }
    open_checked_folder(hwnd, path);
}

const NOTES_MODE_OFF: &str =
    "Notes mode is off. Turn it on with Notes: Toggle notes mode to open notebooks.";

/// The switch itself, once `path` is known to exist.
fn open_checked_folder(hwnd: HWND, path: PathBuf) {
    if !notes_mode(hwnd) {
        push_notice(hwnd, NOTES_MODE_OFF.to_owned());
        return;
    }
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    // The old notebook's dirty notes are saved while autosave still applies to them.
    autosave_all(hwnd);
    if !identity.is_live_for(hwnd) {
        return;
    }
    // Switching would drop the unsaved pins, so a failed write keeps the old notebook.
    let failure = match try_flush(hwnd, false) {
        Ok(library::Flushed::Busy) => Some("its library.ini is in use by another program".into()),
        Ok(_) => None,
        Err(error) => Some(error.to_string()),
    };
    if let Some(error) = failure {
        schedule_write(hwnd);
        push_notice(
            hwnd,
            format!("FastPad kept this notebook open because it could not save its pins: {error}"),
        );
        return;
    }
    host(hwnd, |host| {
        host.state = None;
        host.folder = Some(path.clone());
    });
    update_folders(hwnd, |folders| folders.push(path.clone()));
    start_load(hwnd);
    super::main_window::invalidate_title_strip(hwnd);
    super::side_panel::refresh(hwnd);
}

/// What the existence worker found for a notebook picked from a list.
struct NotebookChecked {
    folder: PathBuf,
    exists: bool,
    /// Show the Notebook view once the notebook is open, with the focus in it for `Some(true)`.
    /// Task 12's Favorites view asks for it.
    show_notebook: Option<bool>,
}

/// Opens a notebook picked from a list (recent, favorites, the no-notebook panel). Such an entry
/// may be on an offline drive, so it is checked on a worker. If it is missing, a notice says so
/// and nothing changes. Unlike startup, nothing falls back to `Documents\FastPad`.
pub(crate) fn open_listed_notebook(hwnd: HWND, folder: &Path) {
    check_listed_notebook(hwnd, folder, None);
}

fn check_listed_notebook(hwnd: HWND, folder: &Path, show_notebook: Option<bool>) {
    if !notes_mode(hwnd) {
        push_notice(hwnd, NOTES_MODE_OFF.to_owned());
        return;
    }
    let folder = library::normalize_folder(folder);
    let target = hwnd as isize;
    std::thread::spawn(move || {
        let exists = library::folder_exists(&folder);
        let payload = Box::into_raw(Box::new(NotebookChecked {
            folder,
            exists,
            show_notebook,
        }));
        if unsafe {
            PostMessageW(
                target as HWND,
                crate::window::WM_FASTPAD_NOTEBOOK_CHECKED,
                0,
                payload as isize,
            )
        } == 0
        {
            drop(unsafe { Box::from_raw(payload) });
        }
    });
}

/// `WM_FASTPAD_NOTEBOOK_CHECKED`: frees the worker's answer and switches if the folder exists.
/// An answer that lands during a modal dialog is dropped, as a click there could not happen.
pub(crate) fn notebook_checked(hwnd: HWND, lparam: LPARAM) {
    if lparam == 0 {
        return;
    }
    let checked = *unsafe { Box::from_raw(lparam as *mut NotebookChecked) };
    if super::modal::modal_active(hwnd) {
        return;
    }
    if !checked.exists {
        push_notice(
            hwnd,
            format!("{} is not available.", checked.folder.display()),
        );
        return;
    }
    let already_open =
        folder(hwnd).is_some_and(|open| library::model::same_path(&open, &checked.folder));
    if !already_open {
        open_checked_folder(hwnd, checked.folder);
    }
    if let Some(focus) = checked.show_notebook {
        super::side_panel::show_view(hwnd, crate::config::SidebarView::Notebook, focus);
    }
}
```

In `picked`, the recent-folder arm becomes an explicit open. The whole function, as Task 1 left it with only `RecentFolder`, is now:

```rust
/// A picker row was chosen. Every kind resolves the row against what that picker showed.
pub(crate) fn picked(hwnd: HWND, kind: PickerKind, choice: PickerChoice) {
    #[cfg(test)]
    LAST_PICK.with(|last| *last.borrow_mut() = Some((kind, choice.clone())));
    if let (PickerKind::RecentFolder, PickerChoice::Item(index)) = (kind, choice) {
        let shown = host(hwnd, |host| std::mem::take(&mut host.shown_recent_folders));
        if let Some(folder) = shown.unwrap_or_default().get(index) {
            open_listed_notebook(hwnd, folder);
        }
    }
}
```

- [ ] **Step 7: Close the notebook**

Add to `src/window/library_host.rs`:

```rust
/// Notebook: Close. Saves the notebook's dirty notes while autosave still applies, writes its
/// pending pins and unloads it. Tabs stay open as plain files, and the next start opens no
/// notebook (`open=none`).
pub(crate) fn close_notebook(hwnd: HWND) {
    let Some(open) = folder(hwnd) else {
        push_notice(hwnd, "No notebook is open.".to_owned());
        return;
    };
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    autosave_all(hwnd);
    if !identity.is_live_for(hwnd) {
        return;
    }
    // Closing drops the unsaved pins with the state, so a failed write keeps the notebook open.
    let failure = match try_flush(hwnd, false) {
        Ok(library::Flushed::Busy) => Some("its library.ini is in use by another program".into()),
        Ok(_) => None,
        Err(error) => Some(error.to_string()),
    };
    if let Some(error) = failure {
        schedule_write(hwnd);
        push_notice(
            hwnd,
            format!("FastPad kept this notebook open because it could not save its pins: {error}"),
        );
        return;
    }
    // A first-save name box would save into the notebook that is going away.
    close_name_box(hwnd);
    unsafe {
        KillTimer(hwnd, LIBRARY_WRITE_TIMER_ID);
        KillTimer(hwnd, AUTOSAVE_TIMER_ID);
    }
    host(hwnd, |host| {
        host.state = None;
        host.folder = None;
        // A load or rescan still running for the closed notebook is ignored when it lands.
        host.generation = host.generation.wrapping_add(1);
        host.scanning = false;
        host.rescan_requested = false;
    });
    update_folders(hwnd, |folders| folders.set_closed(true));
    push_notice(hwnd, format!("Closed the notebook {}.", notebook_name(&open)));
    super::main_window::invalidate_title_strip(hwnd);
    super::side_panel::refresh(hwnd);
}
```

`save_command` and `save_as_command` already take the name-box path only when `folder(hwnd).is_some()`, so with no notebook they fall through to `save_active_document`/`save_active_document_as`. In `src/window/main_window.rs` `save_active_document_as`, replace the arm `None if notes_mode => (...)` with:

```rust
        // With no notebook open this is notes mode off's Save As.
        None if notes_mode && crate::window::library_host::folder(hwnd).is_some() => (
            crate::window::library_host::suggested_file_name(hwnd),
            crate::window::library_host::folder(hwnd),
        ),
```

In `execute_command`, add:

```rust
        CommandId::CloseNotebook => crate::window::library_host::close_notebook(hwnd),
        CommandId::ToggleNotebookFavorite => {
            crate::window::library_host::toggle_notebook_favorite(hwnd);
        }
```

- [ ] **Step 8: Refresh the sidebar wherever the library changes**

In `src/window/library_host.rs`, add `super::side_panel::refresh(hwnd);` as the last statement of each of these:
- `install`, after the `save_local(...)` call;
- `toggle_pin` (Task 1), after the "Pinned."/"Unpinned." notice;
- `document_saved`, after `close_stale_name_box(hwnd);`;
- `submit_rename`, after `super::main_window::invalidate_title_strip(hwnd);`;
- `delete_note`, after `super::main_window::close_document_without_prompt(hwnd, id);`.

`open_checked_folder`, `close_notebook`, `toggle_notebook_favorite`, `remove_favorite` and `library_ready`'s error arm already call it.

- [ ] **Step 9: Say "notebook" in the UI**

In `src/window/menus.rs`, the File menu entry becomes:

```rust
                MenuEntry::command("Open &Notebook...\tCtrl+Shift+O", CommandId::OpenFolder),
```

In `src/window/command_palette.rs` `ENTRIES`, rename two entries and add two after `"Notes: Toggle autosave for this notebook"`. `ENTRIES` grows by 2: raise its length literal by 2.

```rust
    entry("File: Open notebook...", CommandId::OpenFolder),
    entry("File: Open recent notebook...", CommandId::OpenRecentFolder),
```

```rust
    entry(
        "Notes: Toggle autosave for this notebook",
        CommandId::ToggleFolderAutosave,
    ),
    entry("Notebook: Close", CommandId::CloseNotebook),
    entry("Notebook: Toggle favorite", CommandId::ToggleNotebookFavorite),
```

In `src/window/library_host.rs`, change these notices. Match on the quoted start: Task 1 may have reworded the end of some of them.

| Starts with | Becomes |
|---|---|
| `"FastPad could not find the folder {}. Using Documents\\FastPad instead."` | `"FastPad could not find the notebook {}. Using Documents\\FastPad instead."` |
| `"This folder has more than 10,000 notes.` | `"This notebook has more than 10,000 notes. FastPad indexed the first 10,000."` |
| `"This folder's .fastpad\\library.ini` (twice, in `install` and `READ_ONLY`) and `BUSY`'s `"This folder's` | the same text with `This notebook's` |
| `"FastPad could not save this folder's` | `"FastPad could not save this notebook's pins: {error}"` |
| `"No recent folders yet. Use File: Open folder."` | `"No recent notebooks yet. Use File: Open notebook."` |
| `"the notes folder"` (in `folder_display_name`) | `"the notebook"` |
| `"Loading folder…"` (twice) | `"Loading notebook…"` |
| `"Autosave is on for this folder."` / `"Autosave is off for this folder. Use Ctrl+S to save."` | `"Autosave is on for this notebook."` / `"Autosave is off for this notebook. Use Ctrl+S to save."` |
| `"Notes mode is on. The open folder is your note library."` | `"Notes mode is on. The open notebook is your note library."` |
| `"Open a folder first."` | `"Open a notebook first."` |

In `src/window/main_window.rs`, `"{} is a folder. Turn on notes mode to open folders."` becomes `"{} is a folder. Turn on notes mode to open it as a notebook."`.

- [ ] **Step 10: Update the tests the change touches**

In the `main_window.rs` tests:
- In `a_failed_flush_keeps_the_current_folder_open`, the notice check becomes `.any(|n| n.contains("kept this notebook open"))`.
- In `a_recent_folder_pick_opens_the_row_that_was_shown`, the pick now waits for the worker's check. Replace the `assert_eq!(... Some(first.folder()))` after `picked(...)` with:

```rust
        pump_until(window.hwnd, || {
            crate::window::library_host::folder(window.hwnd) == Some(first.folder())
        });
```

- [ ] **Step 11: Run the tests to verify they pass**

Run: `cargo test --lib -- notebook_commands_have_stable closing_the_notebook with_no_notebook_open the_notebook_favorite opening_a_listed_notebook a_recent_folder_pick a_failed_flush_keeps every_command_except --test-threads=1`
Expected: all listed tests PASS.

Run: `cargo clippy --all-targets -- -D warnings` and `cargo fmt --all -- --check`
Expected: no warnings, no diff.

- [ ] **Step 12: Commit**

```bash
git add src/window/library_host.rs src/window/commands.rs src/window/command_palette.rs src/window/menus.rs src/window/main_window.rs src/window/messages.rs src/window/mod.rs
git commit -m "feat(sidebar): close and favorite notebooks, explicit opens checked on a worker, notebook wording"
```

---

### Task 9: The preview tab and `open_note`

**Files:**
- Modify: `src/document.rs`, `src/window/tabs.rs`, `src/window/titlebar.rs`, `src/window/main_window.rs`, `src/app.rs`, `src/window/library_host.rs`

**Interfaces:**
- Consumes: Task 6's `titlebar::create_ui_font(pixel_height, face, weight, italic)`. `open_path`, `activate_document_by_id`, `focus_content` and `Tabs::find_stored_path` exist today.
- Produces:
  - `Document.preview: bool`.
  - `Tabs::preview_id(&self) -> Option<DocumentId>`, `Tabs::replace_preview(&mut self, document: Document) -> Option<Document>` and `Tabs::promote(&mut self, id: DocumentId) -> bool`. `Tabs::note_active_text_change` now returns `bool`, which is true when it promoted the active tab.
  - `TabViewTab.preview: bool`.
  - `main_window::OpenMode::{Preview, Permanent}` and `main_window::open_note(hwnd: HWND, path: &Path, mode: OpenMode, focus_editor: bool) -> crate::Result<()>`.
  - `TitleFontHandles::italic() -> HFONT`, made with Task 6's `create_ui_font`. `TitlePaint.preview_tab: Option<usize>`.
  - `App.last_tab_click: Option<(DocumentId, u32)>`, for tab double-click detection.

The rules (spec §6.4):
- There is at most one preview tab.
- A preview tab is never dirty: the first text change promotes it, so replacing it can't drop text.
- `replace_preview` still refuses to replace a dirty one. It promotes that tab and adds the new tab instead, so even a missed promotion loses nothing.
- A save and a double-click on the tab also promote.
- The session stores the preview tab like any other file tab. Restoring goes through `open_path`, which makes normal tabs, so it comes back as a normal tab.

- [ ] **Step 1: Write the failing tests**

In `src/window/tabs.rs` `mod tests`:

```rust
/// `push` canonicalizes paths through the disk, so these pure tests use untitled documents.
fn preview(id: u64) -> Document {
    let mut document = document(id);
    document.preview = true;
    document
}

#[test]
fn replacing_the_preview_keeps_its_place_and_selects_it() {
    // Break caught: a second preview appended at the end (the strip grows with every click),
    // or replaced in place but left unselected.
    let mut tabs = Tabs::with_document(document(1));
    tabs.push(preview(2)).unwrap();
    tabs.push(document(3)).unwrap();
    assert_eq!(tabs.preview_id(), Some(DocumentId(2)));

    let old = tabs.replace_preview(preview(4)).unwrap();
    assert_eq!(old.id, DocumentId(2));
    assert_eq!(
        tabs.ids().collect::<Vec<_>>(),
        [DocumentId(1), DocumentId(4), DocumentId(3)]
    );
    assert_eq!(tabs.active_index(), 1);
    assert_eq!(tabs.preview_id(), Some(DocumentId(4)));
    assert!(tabs.view().snapshot().tabs[1].preview);
}

#[test]
fn a_dirty_preview_is_kept_as_a_normal_tab_and_the_new_one_is_added() {
    // Break caught: a preview whose promotion was missed being replaced with its edits in it.
    let mut tabs = Tabs::with_document(document(1));
    let mut edited = preview(2);
    edited.dirty = true;
    tabs.push(edited).unwrap();

    assert!(tabs.replace_preview(preview(3)).is_none());
    assert_eq!(tabs.len(), 3);
    assert!(!tabs.document(DocumentId(2)).unwrap().preview);
    assert_eq!(tabs.preview_id(), Some(DocumentId(3)));
    assert_eq!(tabs.active_index(), 2);
}

#[test]
fn with_no_preview_replace_preview_adds_a_tab() {
    // Break caught: the first preview of a session silently dropped because there was nothing
    // to replace.
    let mut tabs = Tabs::with_document(document(1));
    assert!(tabs.replace_preview(preview(2)).is_none());
    assert_eq!(tabs.len(), 2);
    assert_eq!(tabs.preview_id(), Some(DocumentId(2)));
}

#[test]
fn the_first_edit_promotes_the_active_preview_once() {
    // Break caught: typing into a preview leaving it a preview, so the next click replaces it.
    let mut tabs = Tabs::with_document(preview(1));
    assert!(tabs.note_active_text_change());
    assert!(!tabs.note_active_text_change());
    assert_eq!(tabs.preview_id(), None);
    assert!(!tabs.promote(DocumentId(1)));
    let mut tabs = Tabs::with_document(preview(1));
    assert!(tabs.promote(DocumentId(1)));
    assert!(!tabs.view().snapshot().tabs[0].preview);
}
```

`document(id)` is the existing helper in that module.

In the `src/window/main_window.rs` tests:

```rust
fn tab_paths(hwnd: HWND) -> Vec<Option<std::path::PathBuf>> {
    app_mut(hwnd)
        .tabs
        .documents()
        .map(|document| document.path.clone())
        .collect()
}

#[test]
fn the_first_edit_promotes_the_preview_so_a_later_click_opens_a_new_preview() {
    // Break caught: a click replacing a preview the user had started typing into, which drops
    // their text, or the edit not promoting so the tab keeps being replaced.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("preview-edit");
    let a = scratch.note("a.md", "a");
    let b = scratch.note("b.md", "b");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    scratch.install(window.hwnd);

    super::open_note(window.hwnd, &a, super::OpenMode::Preview, false).unwrap();
    assert_eq!(tab_paths(window.hwnd), [Some(a.clone())], "the empty start tab is reused");
    assert!(app_mut(window.hwnd).tabs.active().unwrap().preview);

    editor.set_text("a, edited").unwrap();
    assert!(!app_mut(window.hwnd).tabs.active().unwrap().preview);
    assert_eq!(app_mut(window.hwnd).tabs.preview_id(), None);

    super::open_note(window.hwnd, &b, super::OpenMode::Preview, false).unwrap();
    assert_eq!(tab_paths(window.hwnd), [Some(a.clone()), Some(b.clone())]);
    assert!(app_mut(window.hwnd).tabs.active().unwrap().preview);
    let a_tab = app_mut(window.hwnd).tabs.find_stored_path(&a).unwrap();
    assert!(app_mut(window.hwnd).tabs.document(a_tab).unwrap().dirty);
}

#[test]
fn a_second_preview_replaces_the_first_in_place_keeping_its_tab_index() {
    // Break caught: the replacement landing at the end of the strip, or a normal tab being
    // replaced instead of the preview.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("preview-replace");
    let x = scratch.note("x.md", "x");
    let a = scratch.note("a.md", "a");
    let y = scratch.note("y.md", "y");
    let b = scratch.note("b.md", "b");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    super::open_path(window.hwnd, &x).unwrap();
    super::open_note(window.hwnd, &a, super::OpenMode::Preview, false).unwrap();
    super::open_path(window.hwnd, &y).unwrap();

    super::open_note(window.hwnd, &b, super::OpenMode::Preview, false).unwrap();

    assert_eq!(tab_paths(window.hwnd), [Some(x), Some(b), Some(y)]);
    assert_eq!(app_mut(window.hwnd).tabs.active_index(), 1);
    assert!(app_mut(window.hwnd).tabs.active().unwrap().preview);
}

#[test]
fn opening_an_already_open_note_switches_to_its_tab() {
    // Break caught: a click on an open note replacing the preview with a second tab for the
    // same file, or doing nothing.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("preview-open");
    let a = scratch.note("a.md", "a");
    let b = scratch.note("b.md", "b");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    super::open_path(window.hwnd, &a).unwrap();
    super::open_path(window.hwnd, &b).unwrap();

    super::open_note(window.hwnd, &a, super::OpenMode::Preview, false).unwrap();

    assert_eq!(super::tab_count(window.hwnd), 2);
    assert_eq!(
        app_mut(window.hwnd).tabs.active().unwrap().path.as_deref(),
        Some(a.as_path())
    );
    assert_eq!(app_mut(window.hwnd).tabs.preview_id(), None);
}

#[test]
fn a_permanent_open_a_save_or_a_tab_double_click_keeps_the_preview() {
    // Break caught: Ctrl+Enter or a double-click opening a second tab for a note already in the
    // preview, or a saved preview still being replaced by the next click.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("preview-keep");
    let a = scratch.note("a.md", "a");
    let b = scratch.note("b.md", "b");
    let c = scratch.note("c.md", "c");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    scratch.install(window.hwnd);

    super::open_note(window.hwnd, &a, super::OpenMode::Preview, false).unwrap();
    super::open_note(window.hwnd, &a, super::OpenMode::Permanent, false).unwrap();
    assert_eq!(super::tab_count(window.hwnd), 1);
    assert_eq!(app_mut(window.hwnd).tabs.preview_id(), None);

    super::open_note(window.hwnd, &b, super::OpenMode::Preview, false).unwrap();
    crate::window::library_host::document_saved(window.hwnd);
    assert_eq!(app_mut(window.hwnd).tabs.preview_id(), None, "a save promotes");

    super::open_note(window.hwnd, &c, super::OpenMode::Preview, false).unwrap();
    let index = app_mut(window.hwnd).tabs.active_index();
    let center = super::title_layout(window.hwnd).tab(index).center();
    let pack = |x: i32, y: i32| (x as u16 as u32 | ((y as u16 as u32) << 16)) as isize;
    // Both clicks carry the same message time, well inside the double-click time.
    for _ in 0..2 {
        unsafe {
            SendMessageW(
                window.hwnd,
                windows_sys::Win32::UI::WindowsAndMessaging::WM_LBUTTONUP,
                0,
                pack(center.x, center.y),
            );
        }
    }
    assert_eq!(app_mut(window.hwnd).tabs.preview_id(), None, "a double-click promotes");
    assert_eq!(super::tab_count(window.hwnd), 3);
}

#[test]
fn a_preview_tab_is_kept_by_the_session_and_comes_back_as_a_normal_tab() {
    // Break caught: the session skipping the preview tab, so it vanishes at restart, or the
    // restored tab still being a preview that the next click silently replaces.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("preview-session");
    let a = scratch.note("a.md", "a");
    let recovery = RecoveryScratch::new("preview-session");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    super::open_note(window.hwnd, &a, super::OpenMode::Preview, false).unwrap();

    let session = super::build_session(window.hwnd, recovery.path()).unwrap();
    assert_eq!(session.entries.len(), 1);
    assert!(matches!(&session.entries[0].source, SessionSource::File(path) if *path == a));

    let id = app_mut(window.hwnd).tabs.active().unwrap().id;
    super::close_document_without_prompt(window.hwnd, id);
    super::restore_session_entry(window.hwnd, &session.entries[0]).unwrap();
    assert!(!app_mut(window.hwnd).tabs.active().unwrap().preview);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib -- replacing_the_preview a_dirty_preview with_no_preview_replace the_first_edit_promotes a_second_preview opening_an_already_open_note a_permanent_open a_preview_tab_is_kept --test-threads=1`
Expected: a compile error (`preview`, `preview_id`, `open_note` and `OpenMode` do not exist).

- [ ] **Step 3: The document flag**

In `src/document.rs`, add to `Document` after `autosave_paused`:

```rust
    /// The preview tab (spec §6.4): opened by a single click in the sidebar and replaced in
    /// place by the next one. Never dirty: the first edit makes it a normal tab.
    pub preview: bool,
```

Add `preview: false,` to `Document::untitled`.

- [ ] **Step 4: Tabs**

In `src/window/tabs.rs`, give `TabViewTab` a field and fill it in `view_tabs`:

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TabViewTab {
    pub(crate) id: DocumentId,
    pub(crate) title: String,
    /// Painted in italics.
    pub(crate) preview: bool,
}

fn view_tabs(documents: &[Document]) -> Vec<TabViewTab> {
    documents
        .iter()
        .map(|document| TabViewTab {
            id: document.id,
            title: document.title(),
            preview: document.preview,
        })
        .collect()
}
```

Replace `note_active_text_change` and add the three methods to `impl Tabs`:

```rust
    /// A text change in the active tab. The first one makes a preview tab normal, so replacing
    /// the preview can never drop an edit; returns whether that happened.
    pub(crate) fn note_active_text_change(&mut self) -> bool {
        let active = self.active_index();
        let Some(document) = self.documents.get_mut(active) else {
            return false;
        };
        document.generation = document.generation.saturating_add(1);
        if !document.preview {
            return false;
        }
        document.preview = false;
        self.view.update(&self.documents);
        true
    }

    /// The preview tab, if one is open. There is at most one.
    pub(crate) fn preview_id(&self) -> Option<DocumentId> {
        self.documents
            .iter()
            .find(|document| document.preview)
            .map(|document| document.id)
    }

    /// Puts `document` where the preview tab is, selects it and returns the document it replaced.
    /// With no preview tab, `document` is added like any new tab and `None` is returned. The same
    /// happens when the preview somehow has unsaved edits: it is kept as a normal tab. The caller
    /// has already checked that `document`'s file is not open in another tab.
    pub(crate) fn replace_preview(&mut self, document: Document) -> Option<Document> {
        match self.documents.iter().position(|existing| existing.preview) {
            Some(index) if !self.documents[index].dirty => {
                let old = std::mem::replace(&mut self.documents[index], document);
                self.selection.select(index, self.documents.len());
                self.view.update(&self.documents);
                Some(old)
            }
            edited => {
                if let Some(index) = edited {
                    self.documents[index].preview = false;
                }
                if self.push(document).is_err() {
                    self.view.update(&self.documents);
                }
                None
            }
        }
    }

    /// Makes `id` a normal tab; returns whether it was the preview.
    pub(crate) fn promote(&mut self, id: DocumentId) -> bool {
        let Some(document) = self.document_mut(id) else {
            return false;
        };
        if !document.preview {
            return false;
        }
        document.preview = false;
        self.view.update(&self.documents);
        true
    }
```

- [ ] **Step 5: The italic tab label**

In `src/window/titlebar.rs`, the preview label's italic font comes from Task 6's `create_ui_font`; `create_font` (its upright wrapper) keeps making the other two.

`TitleFontHandles` gains an italic text font:

```rust
#[derive(Clone, Copy, Debug)]
pub(crate) struct TitleFontHandles {
    text: HFONT,
    italic: HFONT,
    glyph: HFONT,
}

impl Default for TitleFontHandles {
    fn default() -> Self {
        Self {
            text: std::ptr::null_mut(),
            italic: std::ptr::null_mut(),
            glyph: std::ptr::null_mut(),
        }
    }
}
```

In `TitleFonts::create`:

```rust
            handles: TitleFontHandles {
                text: create_font(scale(12, dpi), "Segoe UI"),
                italic: create_ui_font(scale(12, dpi), "Segoe UI", FW_NORMAL as i32, true),
                glyph: create_font(scale(10, dpi), "Segoe MDL2 Assets"),
            },
```

Add to `impl TitleFontHandles`:

```rust
    /// The preview tab's label font; the plain text font until it exists.
    pub(crate) fn italic(&self) -> HFONT {
        if self.italic.is_null() {
            self.text
        } else {
            self.italic
        }
    }
```

`Drop for TitleFonts` loops over `[self.handles.text, self.handles.italic, self.handles.glyph]`.

`TitlePaint` gains a field after `active`:

```rust
    /// The preview tab's index; its label is drawn in italics.
    pub preview_tab: Option<usize>,
```

In `draw_strip`'s tab loop, replace `select_font(dc, input.fonts.text);`, the call just before `SetTextColor(dc, foreground);`, with:

```rust
            select_font(
                dc,
                if input.preview_tab == Some(index) {
                    input.fonts.italic()
                } else {
                    input.fonts.text
                },
            );
```

In `src/window/main_window.rs`, `tab_snapshot` also returns the preview index:

```rust
fn tab_snapshot(hwnd: HWND) -> (Vec<String>, usize, i32, bool, Option<usize>) {
    unsafe { app_ptr(hwnd) }
        .map(|app| {
            let app = unsafe { app.as_ref() };
            (
                app.tabs.titles().collect(),
                app.tabs.active_index(),
                app.tabs.scroll_offset(),
                app.editor.is_some() && app.tabs.is_empty(),
                app.tabs.documents().position(|document| document.preview),
            )
        })
        .unwrap_or_else(|| (vec!["Untitled".to_owned()], 0, 0, false, None))
}
```

In the `WM_PAINT` arm, destructure it as `let (titles, active, scroll, empty, preview_tab) = tab_snapshot(hwnd);` and pass `preview_tab,` in the `TitlePaint { ... }` literal after `active,`.

- [ ] **Step 6: `open_note` and the preview placement in `open_path`**

In `src/window/main_window.rs`, rename `open_path` to `fn open_path_placed(hwnd: HWND, path: &std::path::Path, preview: bool) -> Result<()>`, keeping its body. Then add `open_path` back as the normal-tab wrapper:

```rust
pub(crate) fn open_path(hwnd: HWND, path: &std::path::Path) -> Result<()> {
    open_path_placed(hwnd, path, false)
}
```

Inside `open_path_placed`, three regions change. The rest of the body stays as it is.

First, the block that computes `(editor, candidate_ids)` becomes:

```rust
    let (editor, candidate_ids, replace_preview) = {
        let mut app = unsafe { app_ptr(hwnd) }.ok_or(crate::FastPadError::Invariant(
            "main window app state was not available",
        ))?;
        let app = unsafe { app.as_mut() };
        let editor = app
            .editor
            .clone()
            .ok_or(crate::FastPadError::Invariant("editor was not initialized"))?;
        // A preview goes where the preview tab is. Without one it is placed like any new tab,
        // reusing an empty start tab.
        let replace_preview = preview && app.tabs.preview_id().is_some();
        let candidate_ids = app
            .tabs
            .active()
            .filter(|active| !replace_preview && !active.dirty && active.path.is_none())
            .map(|active| (active.id, active.recovery_id));
        (editor, candidate_ids, replace_preview)
    };
```

Second, right after `document.encoding = loaded.encoding;`:

```rust
    document.preview = preview;
```

Third, the commit block `let (commit, retired) = { ... }` becomes:

```rust
    let (commit, retired) = {
        let mut app = unsafe { app_ptr(hwnd) }.ok_or(crate::FastPadError::Invariant(
            "main window app state was not available",
        ))?;
        let app = unsafe { app.as_mut() };
        app.populating_file = false;
        result?;
        if replace_preview {
            // The old preview is never dirty, so dropping it loses nothing.
            (Ok(()), app.tabs.replace_preview(document))
        } else if reuse {
            let retired = app.tabs.replace_active_untitled(document);
            let commit = if retired.is_some() {
                Ok(())
            } else {
                Err(crate::FastPadError::Invariant(
                    "the reused tab closed during file open",
                ))
            };
            (commit, retired)
        } else {
            (
                app.tabs
                    .push(document)
                    .map_err(|_| crate::FastPadError::Invariant("duplicate document path")),
                None,
            )
        }
    };
```

Add after `open_path`:

```rust
/// How `open_note` places a note that is not open yet.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OpenMode {
    /// In the preview tab, replaced in place by the next preview.
    Preview,
    /// In a normal tab. An open preview of the same note becomes normal.
    Permanent,
}

/// Opens `path` from the sidebar (spec §6.4). An already-open note is switched to, and a
/// `Permanent` open keeps it. Otherwise `Preview` replaces the preview tab in place and
/// `Permanent` opens a normal tab. `focus_editor` then moves the keyboard focus to the editor.
pub(crate) fn open_note(
    hwnd: HWND,
    path: &std::path::Path,
    mode: OpenMode,
    focus_editor: bool,
) -> Result<()> {
    let open = unsafe { app_ptr(hwnd) }
        .and_then(|app| unsafe { app.as_ref() }.tabs.find_stored_path(path));
    match open {
        Some(id) => {
            if !activate_document_by_id(hwnd, id) {
                return Err(crate::FastPadError::Invariant(
                    "the note's tab could not be activated",
                ));
            }
            unsafe {
                PostMessageW(hwnd, crate::window::WM_FASTPAD_APPLY_LANGUAGE, 0, 0);
            }
            if mode == OpenMode::Permanent {
                promote_tab(hwnd, id);
            }
        }
        None => open_path_placed(hwnd, path, mode == OpenMode::Preview)?,
    }
    if focus_editor {
        focus_content(hwnd);
    }
    Ok(())
}

/// Makes `id` a normal tab and repaints its label.
fn promote_tab(hwnd: HWND, id: DocumentId) {
    let promoted = unsafe { app_ptr(hwnd) }
        .is_some_and(|mut app| unsafe { app.as_mut() }.tabs.promote(id));
    if promoted {
        invalidate_title_strip(hwnd);
    }
}
```

- [ ] **Step 7: Promote on the first edit, a save and a tab double-click**

In `handle_editor_notification`'s `SCN_MODIFIED` branch, replace the `if text_change && let Some(mut app) = ...` block with:

```rust
        let mut promoted = false;
        if text_change && let Some(mut app) = unsafe { app_ptr(hwnd) } {
            let app = unsafe { app.as_mut() };
            promoted = app.tabs.note_active_text_change();
            if modification.lines_added != 0
                && let Some(editor) = app.editor.as_ref()
            {
                let _ = editor.refresh_line_numbers();
            }
        }
        if promoted {
            invalidate_title_strip(hwnd);
        }
```

In `src/window/library_host.rs` `document_saved`, promote the saved tab. Replace the first `let path = ...` block with:

```rust
    let path = unsafe { app_ptr(hwnd) }.and_then(|mut app| {
        let app = unsafe { app.as_mut() };
        let id = app.tabs.active()?.id;
        // A saved preview is kept: the next click must not replace it.
        app.tabs.promote(id);
        let document = app.tabs.document_mut(id)?;
        let path = document.path.clone()?;
        document.disk_stamp = library::disk_stamp(&path);
        document.autosave_paused = false;
        Some(path)
    });
```

The strip already repaints after every save, because `complete_save` invalidates it.

In `src/app.rs`, add to `App`:

```rust
    /// The last tab click (its document and message time), so a second click on the same tab
    /// within the double-click time keeps a preview tab. The class has no `CS_DBLCLKS`.
    pub(crate) last_tab_click: Option<(crate::document::DocumentId, u32)>,
```

Initialize it with `last_tab_click: None,` in `App::new`. In `src/window/main_window.rs`, add:

```rust
/// Whether this click on tab `index` is the second of a double-click.
fn tab_double_click(hwnd: HWND, index: usize) -> bool {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime;
    use windows_sys::Win32::UI::WindowsAndMessaging::GetMessageTime;
    let now = unsafe { GetMessageTime() } as u32;
    let limit = unsafe { GetDoubleClickTime() };
    let Some(mut app) = (unsafe { app_ptr(hwnd) }) else {
        return false;
    };
    let app = unsafe { app.as_mut() };
    let Some(id) = app.tabs.view().snapshot().tabs.get(index).map(|tab| tab.id) else {
        return false;
    };
    let double = app
        .last_tab_click
        .is_some_and(|(last, at)| last == id && now.wrapping_sub(at) <= limit);
    app.last_tab_click = if double { None } else { Some((id, now)) };
    double
}
```

In the `WM_LBUTTONUP` arm, the `HitTarget::Tab(index)` case becomes:

```rust
                crate::window::titlebar::HitTarget::Tab(index) => {
                    activate_tab(hwnd, index);
                    if tab_double_click(hwnd, index)
                        && let Some(id) = unsafe { app_ptr(hwnd) }
                            .and_then(|app| Some(unsafe { app.as_ref() }.tabs.active()?.id))
                    {
                        promote_tab(hwnd, id);
                    }
                }
```

Session save and restore need no change. `build_session` records every clean file tab as `SessionSource::File`, and `restore_session_entry` reopens it through `open_path`, which always makes a normal tab. The session test pins this.

- [ ] **Step 8: Run the tests to verify they pass**

Run: `cargo test --lib -- replacing_the_preview a_dirty_preview with_no_preview_replace the_first_edit_promotes a_second_preview opening_an_already_open_note a_permanent_open a_preview_tab_is_kept --test-threads=1`
Expected: all eight PASS.

Run: `cargo test --lib window::tabs` and `cargo test --lib -- the_window_title_follows opening_another_folder --test-threads=1`
Expected: PASS. `open_path` behaves as before.

Run: `cargo clippy --all-targets -- -D warnings` and `cargo fmt --all -- --check`
Expected: no warnings, no diff.

- [ ] **Step 9: Commit**

```bash
git add src/document.rs src/window/tabs.rs src/window/titlebar.rs src/window/main_window.rs src/app.rs src/window/library_host.rs
git commit -m "feat(tabs): preview tab replaced in place, promoted by an edit, a save or a double-click"
```

---

### Task 10: The Notebook view

**Files:**
- Create: `src/window/notebook_view.rs`
- Modify: `src/window/mod.rs` (`pub(crate) mod notebook_view;`), `src/window/side_panel.rs`, `src/window/row_list.rs` (drops its dead-code allowance), `src/window/library_host.rs`, `src/window/menus.rs`, `src/window/main_window.rs` (tests)

**Interfaces:**
- Consumes:
  - Task 3:
    - `LibraryState.tree` and `NoteTree::rows(expanded, unsaved)`.
    - `RowKind`, `TreeRow` and `UnsavedEntry`, which derive `Clone, Debug, Eq, PartialEq` and have public fields. `row_index` compares paths ignoring case.
    - `row_index`, `parent_index`, `type_ahead` and `ancestors`. `ancestors("a/b/c.md")` is `["a", "a/b"]`: outermost first, without the root.
  - Task 2: `LocalState::{is_expanded, set_expanded}`, `LocalState.expanded` and `local::display_names`.
  - Task 6:
    - `App.sidebar: Option<Sidebar>` and `side_panel::{refresh, active_tab_changed, show_view, current_view, windows}`.
    - The view seam: `ViewPaint { hdc, client, palette, background, fonts, dpi, focused }`, `draw_text(hdc, text, rect, font, color, flags)`, `point_of`, `UiFonts` and `main_window::ui_fonts`, and the `PanelView` dispatch `paint_view`, `view_mouse`, `view_key` and `header_is_caption`, whose Notebook arms this task fills in.
    - `tooltip::Tooltip::{create, set_tool}`. An empty `text` in `set_tool` removes that tool.
  - Task 7: `RowListState`, `ListKey` and `row_list::paint`. `row_list::paint` is a safe `fn` that fills each row's background (selected, hovered) before calling `draw_row`, draws the scroll thumb, and `row_at(y)` and `row_top(index)` take and return y relative to the list's top.
  - Task 8: `favorites`, `recent_notebooks`, `is_favorite`, `toggle_notebook_favorite`, `open_listed_notebook` and `choose_and_open_folder`.
  - Task 9: `main_window::{open_note, OpenMode}`.
- Produces:
  - `notebook_view::NotebookView`, which holds the rows, a `RowListState`, the hovered pin and header button and the header's state. `NotebookView::new(panel: HWND)`, and the accessors Task 13 reads: `rows(&self) -> &[TreeRow]`, `list(&self) -> &RowListState`, `list_mut(&mut self) -> &mut RowListState`, `list_area(&self, client: RECT, dpi: u32) -> RECT`, `buttons(&self, client: RECT, dpi: u32) -> Vec<(String, RECT)>` (the header's star, New note and "…", then the state's Open notebook… or New note button) and `recent_rows(&self, client: RECT, dpi: u32) -> Vec<(String, RECT)>` (the no-notebook state's RECENT rows, empty otherwise).
  - `notebook_view::Mode::{NoNotebook, Loading, Empty, Tree}`, `HeaderButton::{Favorite, NewNote, More}` and `Activation::{Click, Enter, Permanent}`.
  - `notebook_view` functions:
    - `rebuild(hwnd)` and `active_tab_changed(hwnd)`;
    - `paint(hwnd, paint: &ViewPaint)`;
    - `handle(hwnd, message, wparam, lparam) -> Option<LRESULT>` (mouse, `WM_LBUTTONDBLCLK`, which opens a row permanently and so promotes its preview, keys, `WM_CHAR` and, from Task 11, `WM_CONTEXTMENU`) and `header_hit(hwnd, x, y) -> bool`;
    - `activate(hwnd, index, Activation)`, `key_down(hwnd, key: u16) -> bool` and `header_clicked(hwnd, HeaderButton)`.
  - `notebook_view` pure helpers: `row_parts`, `header_layout`, `body_rect`, `state_layout`, `follow`, `unsaved_entries` and `TypeAhead`.
  - `library_host::set_expanded(hwnd, path: &Path, expanded: bool)` and `library_host::expanded(hwnd) -> Vec<PathBuf>`.
  - `Sidebar.notebook: NotebookView`.
  - `menus::track_popup` and `menus::MenuEntry` (with `MenuEntry::command`) become `pub(crate)`.

Rows are built from `LibraryState.tree` on every `side_panel::refresh` and every tab switch. Building touches no disk. Painting goes through `row_list::paint`, which draws only the rows on screen and the scroll thumb. The view uses the sidebar's shared fonts (`ViewPaint.fonts`, or `main_window::ui_fonts` read before the view is borrowed) and `side_panel::draw_text`, and creates no fonts of its own.

After a rebuild, the selection and the top row are found again by `RowKind`, the path, not by index. A selected row that is gone yields to the row that took its index, clamped to the list (`follow`). That covers Review Focus 2.

- [ ] **Step 1: Write the failing tests**

At the end of the new `src/window/notebook_view.rs`, which Step 3 fills in:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// `RECT` has no `PartialEq` or `Debug` in windows-sys.
    fn edges(rect: RECT) -> (i32, i32, i32, i32) {
        (rect.left, rect.top, rect.right, rect.bottom)
    }

    fn row(kind: RowKind, depth: u16) -> TreeRow {
        TreeRow {
            kind,
            depth,
            name: String::new(),
            pinned: false,
            expanded: false,
        }
    }

    #[test]
    fn a_row_indents_by_depth_and_keeps_the_pin_at_the_right_edge() {
        // Break caught: deep rows pushing the pin off the row, names drawn over the chevron, or
        // an inverted name rectangle in a panel narrower than the indent.
        let rect = RECT { left: 0, top: 26, right: 260, bottom: 52 };
        let top = row_parts(rect, 0, 96);
        let deep = row_parts(rect, 3, 96);
        assert_eq!(top.chevron.left, 8);
        assert_eq!(deep.chevron.left, 8 + 3 * 12);
        assert_eq!(edges(top.pin), (236, 26, 260, 52));
        assert_eq!(edges(deep.pin), edges(top.pin));
        assert!(deep.name.left >= deep.icon.right);
        assert_eq!(deep.name.right, deep.pin.left);
        let cramped = row_parts(RECT { left: 0, top: 0, right: 40, bottom: 26 }, 9, 96);
        assert!(cramped.name.left <= cramped.name.right);
        assert_eq!(row_parts(rect, 1, 192).chevron.left, 16 + 24);
    }

    #[test]
    fn the_header_buttons_sit_right_to_left_and_the_title_stops_before_them() {
        // Break caught: the notebook name drawn under the star, or buttons that do not follow
        // the panel's right edge when it is resized.
        let area = RECT { left: 0, top: 0, right: 260, bottom: 600 };
        let layout = header_layout(area, 96);
        let [(first, star), (second, new), (third, more)] = layout.buttons;
        assert_eq!(
            (first, second, third),
            (HeaderButton::Favorite, HeaderButton::NewNote, HeaderButton::More)
        );
        assert_eq!(edges(more), (226, 5, 254, 33));
        assert_eq!((star.right, new.right), (new.left, more.left));
        assert!(layout.title.right <= star.left);
        assert_eq!(layout.title.bottom, 38);
        assert_eq!(body_rect(area, 96).top, 38);
    }

    #[test]
    fn the_no_notebook_state_lists_recent_notebooks_below_its_button() {
        // Break caught: the RECENT rows painted over the Open notebook… button, or a list rect
        // that turns inside out in a short panel.
        let body = RECT { left: 0, top: 38, right: 260, bottom: 600 };
        let layout = state_layout(body, 96);
        assert!(layout.message.bottom <= layout.button.top);
        assert!(layout.button.bottom <= layout.label.top);
        assert_eq!(layout.list.top, layout.label.bottom);
        assert_eq!(layout.list.bottom, 600);
        let short = state_layout(RECT { left: 0, top: 38, right: 260, bottom: 60 }, 96);
        assert!(short.list.top <= short.list.bottom);
    }

    #[test]
    fn a_vanished_selection_moves_to_the_row_that_took_its_place() {
        // Break caught: a stale index past the end after a rescan removed rows, or a selection
        // that jumps to the top instead of staying where it was.
        let rows = vec![
            row(RowKind::Folder("sub".into()), 0),
            row(RowKind::Note(r"sub\a.md".into()), 1),
            row(RowKind::Note("c.md".into()), 0),
        ];
        let a = RowKind::Note(r"sub\a.md".into());
        assert_eq!(follow(&rows, Some(&a), Some(7)), Some(1), "found by path first");
        let gone = RowKind::Note(r"sub\b.md".into());
        assert_eq!(follow(&rows, Some(&gone), Some(2)), Some(2));
        assert_eq!(follow(&rows, Some(&gone), Some(9)), Some(2));
        assert_eq!(follow(&[], Some(&gone), Some(1)), None);
        assert_eq!(follow(&rows, None, Some(1)), None, "nothing selected stays so");
    }

    #[test]
    fn unsaved_rows_come_from_untitled_tabs_labelled_by_their_first_line() {
        // Break caught: saved tabs listed twice (as a note and as unsaved), or untitled tabs
        // with a blank first line shown with no label at all.
        let mut labelled = Document::test_fixture(DocumentId(4), true);
        labelled.first_line_label = Some("Groceries".to_owned());
        let blank = Document::test_fixture(DocumentId(5), false);
        let mut saved = Document::test_fixture(DocumentId(6), false);
        saved.path = Some(PathBuf::from(r"C:\n\a.md"));
        let entries = unsaved_entries([&labelled, &blank, &saved].into_iter());
        let entries: Vec<_> = entries.into_iter().map(|e| (e.key, e.label)).collect();
        assert_eq!(
            entries,
            [(4, "Groceries".to_owned()), (5, "Untitled".to_owned())]
        );
    }

    #[test]
    fn type_ahead_extends_the_prefix_within_a_second_and_starts_over_after() {
        // Break caught: a prefix that never resets, so a second search a minute later matches
        // nothing.
        let start = Instant::now();
        let mut typed = TypeAhead::default();
        assert_eq!(typed.push('n', start), "n");
        assert_eq!(typed.push('o', start + Duration::from_millis(900)), "no");
        assert_eq!(typed.push('x', start + Duration::from_millis(2_000)), "x");
    }
}
```

In the `src/window/main_window.rs` tests, add these helpers and tests after the `LibraryScratch` tests:

```rust
use crate::library::tree::RowKind;
use crate::window::notebook_view::{Activation, Mode, NotebookView};

/// Task 6 creates the sidebar with the window when notes mode is on; this makes sure of it.
fn ensure_sidebar(hwnd: HWND) {
    if app_mut(hwnd).sidebar.is_none() {
        crate::window::side_panel::notes_mode_changed(hwnd, true);
    }
}

fn notebook_view<'a>(hwnd: HWND) -> &'a mut NotebookView {
    &mut app_mut(hwnd).sidebar.as_mut().unwrap().notebook
}

fn row_of(hwnd: HWND, kind: &RowKind) -> usize {
    crate::library::tree::row_index(&notebook_view(hwnd).rows, kind)
        .unwrap_or_else(|| panic!("{kind:?} is not in {:?}", notebook_view(hwnd).rows))
}

fn selected_kind(hwnd: HWND) -> Option<RowKind> {
    let view = notebook_view(hwnd);
    view.list
        .selected
        .and_then(|index| view.rows.get(index))
        .map(|row| row.kind.clone())
}

fn select_row(hwnd: HWND, kind: &RowKind) {
    let index = row_of(hwnd, kind);
    notebook_view(hwnd).list.selected = Some(index);
}

fn rescan_and_wait(hwnd: HWND) {
    crate::window::library_host::request_rescan(hwnd);
    pump_until(hwnd, || !app_mut(hwnd).library.scanning);
}

#[test]
fn a_rescan_keeps_selection_and_expansion_by_path() {
    // Break caught: a rescan that rebuilds the rows and keeps the selected index, so the
    // highlight jumps to another note; one that collapses the folder the user had open; or a
    // vanished selection left pointing past the end of the list.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("rescan-selection");
    std::fs::create_dir_all(scratch.folder().join("sub")).unwrap();
    scratch.note(r"sub\b.md", "b");
    scratch.note("c.md", "c");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    ensure_sidebar(window.hwnd);
    app_mut(window.hwnd).library.data_dir = Some(scratch.data());
    scratch.install(window.hwnd);
    crate::window::library_host::set_expanded(window.hwnd, std::path::Path::new("sub"), true);
    crate::window::notebook_view::rebuild(window.hwnd);
    let b = RowKind::Note(r"sub\b.md".into());
    select_row(window.hwnd, &b);

    scratch.note(r"sub\a.md", "a");
    rescan_and_wait(window.hwnd);
    assert_eq!(selected_kind(window.hwnd), Some(b.clone()), "followed by path");
    let sub = row_of(window.hwnd, &RowKind::Folder("sub".into()));
    assert!(notebook_view(window.hwnd).rows[sub].expanded);
    assert!(row_of(window.hwnd, &RowKind::Note(r"sub\a.md".into())) < row_of(window.hwnd, &b));

    let before = notebook_view(window.hwnd).list.selected.unwrap();
    std::fs::remove_file(scratch.folder().join(r"sub\b.md")).unwrap();
    rescan_and_wait(window.hwnd);
    let view = notebook_view(window.hwnd);
    let after = view.list.selected.expect("the selection moves, it does not vanish");
    assert!(after < view.rows.len());
    assert_eq!(after, before.min(view.rows.len() - 1));
    assert!(view.rows[sub].expanded);
}

#[test]
fn clicking_a_note_row_opens_the_preview_and_a_double_click_keeps_it() {
    // Break caught: a click opening a normal tab every time (tabs pile up), or a double-click
    // opening a second tab instead of keeping the preview.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("view-click");
    let a = scratch.note("a.md", "a");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    ensure_sidebar(window.hwnd);
    scratch.install(window.hwnd);
    let row = row_of(window.hwnd, &RowKind::Note("a.md".into()));

    crate::window::notebook_view::activate(window.hwnd, row, Activation::Click);
    let active = app_mut(window.hwnd).tabs.active().unwrap();
    assert_eq!(active.path.as_deref(), Some(a.as_path()));
    assert!(active.preview);

    let row = row_of(window.hwnd, &RowKind::Note("a.md".into()));
    crate::window::notebook_view::activate(window.hwnd, row, Activation::Permanent);
    assert_eq!(super::tab_count(window.hwnd), 1);
    assert!(!app_mut(window.hwnd).tabs.active().unwrap().preview);
}

#[test]
fn switching_to_a_note_in_a_subfolder_selects_its_row_and_expands_its_folders() {
    // Break caught: the tree not following the active tab, or following it into a collapsed
    // folder so the selected row is hidden, or forgetting that expansion at the next start.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("view-reveal");
    std::fs::create_dir_all(scratch.folder().join(r"sub\deep")).unwrap();
    let b = scratch.note(r"sub\deep\b.md", "b");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    ensure_sidebar(window.hwnd);
    app_mut(window.hwnd).library.data_dir = Some(scratch.data());
    scratch.install(window.hwnd);

    super::open_path(window.hwnd, &b).unwrap();
    crate::window::side_panel::active_tab_changed(window.hwnd);

    assert_eq!(
        selected_kind(window.hwnd),
        Some(RowKind::Note(r"sub\deep\b.md".into()))
    );
    let expanded = crate::window::library_host::expanded(window.hwnd);
    assert!(expanded.contains(&std::path::PathBuf::from("sub")));
    assert!(expanded.contains(&std::path::PathBuf::from(r"sub\deep")));
    let local = crate::library::local::local_file(&scratch.data(), &scratch.folder());
    let written = crate::library::local::read(&local, &scratch.folder());
    assert!(written.expanded.contains(&std::path::PathBuf::from(r"sub\deep")));
}

#[test]
fn right_expands_a_folder_then_enters_it_and_left_climbs_back_out() {
    // Break caught: arrow keys that only move up and down, so a folder cannot be opened from
    // the keyboard, or Left on a child that does nothing.
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_LEFT, VK_RIGHT};
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("view-keys");
    std::fs::create_dir_all(scratch.folder().join("sub")).unwrap();
    scratch.note(r"sub\a.md", "a");
    scratch.note("z.md", "z");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    ensure_sidebar(window.hwnd);
    app_mut(window.hwnd).library.data_dir = Some(scratch.data());
    scratch.install(window.hwnd);
    let sub = RowKind::Folder("sub".into());
    select_row(window.hwnd, &sub);
    let key = |key| crate::window::notebook_view::key_down(window.hwnd, key);

    assert!(key(VK_RIGHT));
    assert!(notebook_view(window.hwnd).rows[row_of(window.hwnd, &sub)].expanded);
    assert!(key(VK_RIGHT));
    assert_eq!(selected_kind(window.hwnd), Some(RowKind::Note(r"sub\a.md".into())));
    assert!(key(VK_LEFT));
    assert_eq!(selected_kind(window.hwnd), Some(sub.clone()));
    assert!(key(VK_LEFT));
    assert!(!notebook_view(window.hwnd).rows[row_of(window.hwnd, &sub)].expanded);
}

#[test]
fn the_view_says_loading_then_shows_the_tree_and_recent_notebooks_once_closed() {
    // Break caught: an empty panel while the worker loads, a tree left on screen after Close
    // notebook, or a no-notebook state without the RECENT list.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("view-states");
    scratch.note("a.md", "a");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    ensure_sidebar(window.hwnd);
    app_mut(window.hwnd).library.data_dir = Some(scratch.data());
    crate::library::local::write_folders(
        &crate::library::local::folders_file(&scratch.data()),
        &crate::library::local::RecentFolders {
            folders: vec![scratch.folder()],
            ..Default::default()
        },
    )
    .unwrap();

    app_mut(window.hwnd).library.folder = Some(scratch.folder());
    crate::window::notebook_view::rebuild(window.hwnd);
    assert_eq!(notebook_view(window.hwnd).mode, Mode::Loading);

    scratch.install(window.hwnd);
    assert_eq!(notebook_view(window.hwnd).mode, Mode::Tree);

    execute_command(window.hwnd, CommandId::CloseNotebook);
    assert_eq!(notebook_view(window.hwnd).mode, Mode::NoNotebook);
    assert_eq!(notebook_view(window.hwnd).recent, vec![scratch.folder()]);
}

#[test]
fn an_empty_notebook_says_so_until_an_untitled_tab_appears_as_an_unsaved_row() {
    // Break caught: a blank panel for a notebook with no notes, or a new untitled tab that the
    // tree does not show until it is saved.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("view-empty");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    ensure_sidebar(window.hwnd);
    scratch.install(window.hwnd);
    let start = app_mut(window.hwnd).tabs.active().unwrap().id;
    super::close_document_without_prompt(window.hwnd, start);
    crate::window::notebook_view::rebuild(window.hwnd);
    assert_eq!(notebook_view(window.hwnd).mode, Mode::Empty);

    execute_command(window.hwnd, CommandId::New);
    crate::window::notebook_view::rebuild(window.hwnd);
    let id = app_mut(window.hwnd).tabs.active().unwrap().id;
    let view = notebook_view(window.hwnd);
    assert_eq!(view.mode, Mode::Tree);
    assert_eq!(view.rows[0].kind, RowKind::Unsaved(id.0));
    assert_eq!(view.rows[0].name, "Untitled");
}

#[test]
fn the_header_star_favorites_the_notebook_and_every_state_paints() {
    // Break caught: a star that does nothing, or a paint path that panics on an empty tree,
    // the loading state or the no-notebook state.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("view-star");
    scratch.note("a.md", "a");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    ensure_sidebar(window.hwnd);
    app_mut(window.hwnd).library.data_dir = Some(scratch.data());
    scratch.install(window.hwnd);

    crate::window::notebook_view::header_clicked(
        window.hwnd,
        crate::window::notebook_view::HeaderButton::Favorite,
    );
    assert!(crate::window::library_host::is_favorite(window.hwnd));

    let panel = notebook_view(window.hwnd).panel;
    let area = RECT { left: 0, top: 0, right: 260, bottom: 400 };
    let dc = unsafe { windows_sys::Win32::Graphics::Gdi::GetDC(panel) };
    let paint = |hwnd: HWND| {
        let view_paint = crate::window::side_panel::view_paint(hwnd, panel, dc, area);
        crate::window::notebook_view::paint(hwnd, &view_paint);
    };
    paint(window.hwnd);
    execute_command(window.hwnd, CommandId::CloseNotebook);
    paint(window.hwnd);
    app_mut(window.hwnd).library.folder = Some(scratch.folder());
    crate::window::notebook_view::rebuild(window.hwnd);
    paint(window.hwnd);
    unsafe { windows_sys::Win32::Graphics::Gdi::ReleaseDC(panel, dc) };
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib -- window::notebook_view a_rescan_keeps_selection clicking_a_note_row switching_to_a_note_in_a_subfolder right_expands_a_folder the_view_says_loading an_empty_notebook_says_so the_header_star --test-threads=1`
Expected: a compile error (`notebook_view` does not exist).

- [ ] **Step 3: Write `src/window/notebook_view.rs`**

```rust
//! The Notebook view (spec §6): the open notebook's folder tree in the side panel, with its
//! header, pins, type-ahead and the loading, no-notebook and empty states. The tree itself is
//! built on the scan worker (`LibraryState.tree`). This module flattens the expanded part into
//! rows, paints only the rows on screen, and turns clicks and keys into `open_note` calls.

use super::main_window::{OpenMode, app_ptr};
use super::side_panel::{UiFonts, ViewPaint, draw_text, point_of};
use crate::document::{Document, DocumentId};
use crate::library::tree::{self, RowKind, TreeRow, UnsavedEntry};
use crate::window::commands::CommandId;
use crate::window::menus::MenuEntry;
use crate::window::palette::Palette;
use crate::window::panel::{fill, scale};
use crate::window::row_list::{self, ListKey, RowListState, RowLook};
use crate::window::tooltip::Tooltip;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    ClientToScreen, DT_CENTER, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER,
    DT_WORDBREAK, GetDC, GetTextExtentPoint32W, HDC, HFONT, InvalidateRect, ReleaseDC,
    ScreenToClient, SelectObject,
};
use windows_sys::Win32::UI::Controls::WM_MOUSELEAVE;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, SetFocus, TME_LEAVE, TRACKMOUSEEVENT,
    TrackMouseEvent, VK_CONTROL, VK_LEFT, VK_RETURN, VK_RIGHT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetClientRect, GetParent, SPI_GETWHEELSCROLLLINES, SendMessageW, SystemParametersInfoW,
    WM_CAPTURECHANGED, WM_CHAR, WM_COMMAND,
    WM_KEYDOWN, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL,
    WM_RBUTTONDOWN,
};

// Sizes at 96 DPI; everything is scaled with `panel::scale`.
const ROW_HEIGHT: i32 = 26;
const HEADER_HEIGHT: i32 = 38;
const INDENT: i32 = 12;
const LEFT_PAD: i32 = 8;
const GLYPH_BOX: i32 = 16;
const GAP: i32 = 6;
const PIN_BOX: i32 = 24;
const HEADER_BUTTON: i32 = 28;
const TYPE_AHEAD_RESET: Duration = Duration::from_secs(1);

pub(crate) const TRUNCATED_ROW: &str = "Showing the first 10,000 notes";

// Segoe MDL2 Assets, the font the title bar already uses.
const GLYPH_CHEVRON_RIGHT: &str = "\u{E76C}";
const GLYPH_CHEVRON_DOWN: &str = "\u{E70D}";
const GLYPH_FOLDER: &str = "\u{E8B7}";
const GLYPH_NOTE: &str = "\u{E8A5}";
const GLYPH_PIN: &str = "\u{E718}";
const GLYPH_PINNED: &str = "\u{E842}";
const GLYPH_STAR: &str = "\u{E734}";
const GLYPH_STAR_FILLED: &str = "\u{E735}";
const GLYPH_ADD: &str = "\u{E710}";
const GLYPH_MORE: &str = "\u{E712}";

// Tooltip tool IDs.
const TOOL_ROW: usize = 1;
const TOOL_TITLE: usize = 2;
const TOOL_FAVORITE: usize = 3;
const TOOL_NEW: usize = 4;
const TOOL_MORE: usize = 5;

/// What the view shows.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Mode {
    /// No notebook: "Open a notebook to see its notes.", a button and the RECENT list.
    NoNotebook,
    /// A notebook is open but its state has not arrived from the worker.
    Loading,
    /// Loaded, with no notes and no untitled tabs.
    Empty,
    Tree,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HeaderButton {
    Favorite,
    NewNote,
    More,
}

/// How a note row is being opened (spec §6.4).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Activation {
    /// A mouse click: the preview tab, focus to the editor.
    Click,
    /// Enter: the preview tab, focus stays in the tree for further browsing.
    Enter,
    /// Ctrl+Enter or a double-click: a normal tab, focus to the editor.
    Permanent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RowPart {
    Chevron,
    Pin,
    Body,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Hit {
    Header(HeaderButton),
    Title,
    /// "Open notebook…" (no notebook) or "New note" (empty notebook).
    StateButton,
    /// The scroll thumb, with where on it the press landed.
    Thumb(i32),
    Row { index: usize, part: RowPart },
    Empty,
}

/// What a list index stands for right now.
#[derive(Clone, Debug)]
enum Target {
    Recent(PathBuf),
    Row(TreeRow),
    Truncated,
    Nothing,
}

// windows-sys RECT is only Clone + Copy, so the layout structs are too.
#[derive(Clone, Copy)]
pub(crate) struct RowParts {
    pub chevron: RECT,
    pub icon: RECT,
    pub name: RECT,
    pub pin: RECT,
}

/// A tree row's pieces: the chevron (folders only) and icon, indented by `depth`, the name, and
/// the pin button at the right edge. Every rectangle stays inside `row`, even in a narrow panel.
pub(crate) fn row_parts(row: RECT, depth: u16, dpi: u32) -> RowParts {
    let glyph = scale(GLYPH_BOX, dpi);
    let chevron_left =
        (row.left + scale(LEFT_PAD, dpi) + i32::from(depth) * scale(INDENT, dpi)).min(row.right);
    let chevron = RECT {
        left: chevron_left,
        top: row.top,
        right: (chevron_left + glyph).min(row.right),
        bottom: row.bottom,
    };
    let icon = RECT {
        left: chevron.right,
        top: row.top,
        right: (chevron.right + glyph).min(row.right),
        bottom: row.bottom,
    };
    let pin_left = (row.right - scale(PIN_BOX, dpi)).max(icon.right);
    let pin = RECT {
        left: pin_left,
        top: row.top,
        right: row.right,
        bottom: row.bottom,
    };
    let name = RECT {
        left: (icon.right + scale(GAP, dpi)).min(pin_left),
        top: row.top,
        right: pin_left,
        bottom: row.bottom,
    };
    RowParts {
        chevron,
        icon,
        name,
        pin,
    }
}

#[derive(Clone, Copy)]
pub(crate) struct HeaderLayout {
    pub title: RECT,
    /// Left to right: star, New note, "…".
    pub buttons: [(HeaderButton, RECT); 3],
}

pub(crate) fn header_layout(area: RECT, dpi: u32) -> HeaderLayout {
    let height = scale(HEADER_HEIGHT, dpi);
    let size = scale(HEADER_BUTTON, dpi);
    let top = area.top + (height - size) / 2;
    let right = area.right - scale(6, dpi);
    let slot = |from_right: i32| RECT {
        left: right - (from_right + 1) * size,
        top,
        right: right - from_right * size,
        bottom: top + size,
    };
    let buttons = [
        (HeaderButton::Favorite, slot(2)),
        (HeaderButton::NewNote, slot(1)),
        (HeaderButton::More, slot(0)),
    ];
    let title_left = area.left + scale(12, dpi);
    let title = RECT {
        left: title_left,
        top: area.top,
        right: (buttons[0].1.left - scale(4, dpi)).max(title_left),
        bottom: area.top + height,
    };
    HeaderLayout { title, buttons }
}

/// Everything below the header.
pub(crate) fn body_rect(area: RECT, dpi: u32) -> RECT {
    RECT {
        top: (area.top + scale(HEADER_HEIGHT, dpi)).min(area.bottom),
        ..area
    }
}

#[derive(Clone, Copy)]
pub(crate) struct StateLayout {
    pub message: RECT,
    pub button: RECT,
    /// "RECENT", in the no-notebook state.
    pub label: RECT,
    /// The recent rows.
    pub list: RECT,
}

/// Where the no-notebook and empty states put their message, button and RECENT list.
pub(crate) fn state_layout(body: RECT, dpi: u32) -> StateLayout {
    let pad = scale(12, dpi);
    let left = body.left + pad;
    let right = (body.right - pad).max(left);
    let message = RECT {
        left,
        top: body.top + scale(8, dpi),
        right,
        bottom: body.top + scale(48, dpi),
    };
    let button = RECT {
        left,
        top: message.bottom + scale(4, dpi),
        right: (left + scale(140, dpi)).min(right),
        bottom: message.bottom + scale(32, dpi),
    };
    let label = RECT {
        left,
        top: button.bottom + scale(16, dpi),
        right,
        bottom: button.bottom + scale(36, dpi),
    };
    let list = RECT {
        left: body.left,
        top: label.bottom.min(body.bottom),
        right: body.right,
        bottom: body.bottom,
    };
    StateLayout {
        message,
        button,
        label,
        list,
    }
}

/// The row that stands for `kind` after a rebuild: the same path if it is still there, else the
/// row that took its index (clamped to the list). Nothing selected stays nothing.
pub(crate) fn follow(rows: &[TreeRow], kind: Option<&RowKind>, old: Option<usize>) -> Option<usize> {
    let kind = kind?;
    if let Some(index) = tree::row_index(rows, kind) {
        return Some(index);
    }
    let last = rows.len().checked_sub(1)?;
    Some(old.unwrap_or(0).min(last))
}

/// One entry per untitled tab, keyed by its `DocumentId`, labelled like its tab (spec §6.2).
pub(crate) fn unsaved_entries<'a>(
    documents: impl Iterator<Item = &'a Document>,
) -> Vec<UnsavedEntry> {
    documents
        .filter(|document| document.path.is_none())
        .map(|document| UnsavedEntry {
            key: document.id.0,
            label: document
                .untitled_label
                .clone()
                .or_else(|| document.first_line_label.clone())
                .unwrap_or_else(|| "Untitled".to_owned()),
        })
        .collect()
}

/// Letters typed into the tree within a second of each other form one prefix.
#[derive(Debug, Default)]
pub(crate) struct TypeAhead {
    text: String,
    at: Option<Instant>,
}

impl TypeAhead {
    pub(crate) fn push(&mut self, ch: char, now: Instant) -> &str {
        if self
            .at
            .is_none_or(|at| now.duration_since(at) >= TYPE_AHEAD_RESET)
        {
            self.text.clear();
        }
        self.text.push(ch);
        self.at = Some(now);
        &self.text
    }
}

/// The Notebook view's state, owned by `side_panel::Sidebar`.
pub(crate) struct NotebookView {
    pub(crate) panel: HWND,
    pub(crate) mode: Mode,
    pub(crate) rows: Vec<TreeRow>,
    /// The scan stopped at its limit: one more row, after the last, says so.
    pub(crate) truncated: bool,
    pub(crate) list: RowListState,
    /// The no-notebook state's RECENT notebooks and their display names.
    pub(crate) recent: Vec<PathBuf>,
    recent_names: Vec<(String, Option<String>)>,
    root: Option<PathBuf>,
    name: String,
    favorite: bool,
    /// The header button (or state button) under the pointer.
    hover: Option<Hit>,
    /// The pointer is over the hovered row's pin button.
    pub(crate) hover_pin: bool,
    tooltip: Option<Tooltip>,
    tooltip_failed: bool,
    typed: TypeAhead,
    thumb_grab: Option<i32>,
    tracking_leave: bool,
}

impl std::fmt::Debug for NotebookView {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NotebookView")
            .field("mode", &self.mode)
            .field("rows", &self.rows.len())
            .field("selected", &self.list.selected)
            .finish_non_exhaustive()
    }
}

const fn height(rect: RECT) -> i32 {
    let height = rect.bottom - rect.top;
    if height > 0 { height } else { 0 }
}

const fn contains(rect: RECT, x: i32, y: i32) -> bool {
    x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom
}

const LINE: u32 = DT_SINGLELINE | DT_VCENTER | DT_LEFT | DT_END_ELLIPSIS | DT_NOPREFIX;
const CENTERED: u32 = DT_SINGLELINE | DT_VCENTER | DT_CENTER | DT_NOPREFIX;

#[allow(
    clippy::too_many_arguments,
    reason = "one row's paint inputs, called from one closure"
)]
fn draw_tree_row(
    dc: HDC,
    row: Option<&TreeRow>,
    rect: RECT,
    look: RowLook,
    palette: &Palette,
    fonts: UiFonts,
    dpi: u32,
    pin_hot: bool,
) {
    let foreground = if look.selected {
        palette
            .selection_foreground
            .unwrap_or(palette.editor_foreground)
    } else {
        palette.editor_foreground
    };
    let muted = if look.selected {
        foreground
    } else {
        palette.muted_foreground
    };
    let Some(row) = row else {
        let text = RECT {
            left: rect.left + scale(LEFT_PAD, dpi),
            ..rect
        };
        unsafe { draw_text(dc, TRUNCATED_ROW, text, fonts.italic, muted, LINE) };
        return;
    };
    let parts = row_parts(rect, row.depth, dpi);
    match &row.kind {
        RowKind::Folder(_) => {
            let chevron = if row.expanded {
                GLYPH_CHEVRON_DOWN
            } else {
                GLYPH_CHEVRON_RIGHT
            };
            unsafe { draw_text(dc, chevron, parts.chevron, fonts.glyph, muted, CENTERED) };
            unsafe { draw_text(dc, GLYPH_FOLDER, parts.icon, fonts.glyph, muted, CENTERED) };
        }
        RowKind::Note(_) | RowKind::Unsaved(_) => {
            unsafe { draw_text(dc, GLYPH_NOTE, parts.icon, fonts.glyph, muted, CENTERED) };
        }
    }
    if matches!(row.kind, RowKind::Note(_)) {
        // Pinned is a filled glyph, never color alone (spec §10).
        if row.pinned {
            unsafe { draw_text(dc, GLYPH_PINNED, parts.pin, fonts.glyph, foreground, CENTERED) };
        } else if look.hover || look.selected {
            let color = if pin_hot { foreground } else { muted };
            unsafe { draw_text(dc, GLYPH_PIN, parts.pin, fonts.glyph, color, CENTERED) };
        }
    }
    let font = if matches!(row.kind, RowKind::Unsaved(_)) {
        fonts.italic
    } else {
        fonts.text
    };
    unsafe { draw_text(dc, &row.name, parts.name, font, foreground, LINE) };
}

fn draw_recent_row(
    dc: HDC,
    name: Option<&(String, Option<String>)>,
    rect: RECT,
    look: RowLook,
    palette: &Palette,
    fonts: UiFonts,
    dpi: u32,
) {
    let Some((name, hint)) = name else {
        return;
    };
    let foreground = if look.selected {
        palette
            .selection_foreground
            .unwrap_or(palette.editor_foreground)
    } else {
        palette.editor_foreground
    };
    let parts = row_parts(rect, 0, dpi);
    unsafe { draw_text(dc, GLYPH_FOLDER, parts.icon, fonts.glyph, palette.muted_foreground, CENTERED) };
    let text = RECT {
        right: rect.right - scale(LEFT_PAD, dpi),
        ..parts.name
    };
    // A clash between two notebook names shows the parent folder, dimmed, after the name.
    let label = match hint {
        Some(hint) => format!("{name}  {hint}"),
        None => name.clone(),
    };
    unsafe { draw_text(dc, &label, text, fonts.text, foreground, LINE) };
}

impl NotebookView {
    pub(crate) fn new(panel: HWND) -> Self {
        let dpi = unsafe { GetDpiForWindow(panel) }.max(96);
        Self {
            panel,
            mode: Mode::Loading,
            rows: Vec::new(),
            truncated: false,
            list: RowListState::new(scale(ROW_HEIGHT, dpi)),
            recent: Vec::new(),
            recent_names: Vec::new(),
            root: None,
            name: String::new(),
            favorite: false,
            hover: None,
            hover_pin: false,
            tooltip: None,
            tooltip_failed: false,
            typed: TypeAhead::default(),
            thumb_grab: None,
            tracking_leave: false,
        }
    }

    /// The list's rectangle for the current mode, in the panel's `client` coordinates at `dpi`:
    /// the tree rows, the RECENT rows, or an empty band while there are none.
    pub(crate) fn list_area(&self, client: RECT, dpi: u32) -> RECT {
        let body = body_rect(client, dpi);
        match self.mode {
            Mode::Tree => body,
            Mode::NoNotebook => state_layout(body, dpi).list,
            Mode::Loading | Mode::Empty => RECT {
                bottom: body.top,
                ..body
            },
        }
    }

    fn dpi(&self) -> u32 {
        unsafe { GetDpiForWindow(self.panel) }.max(96)
    }

    fn client(&self) -> RECT {
        let mut rect = RECT::default();
        unsafe {
            GetClientRect(self.panel, &mut rect);
        }
        rect
    }

    fn invalidate(&self) {
        unsafe {
            InvalidateRect(self.panel, std::ptr::null(), 0);
        }
    }

    /// The list's rectangle in panel coordinates for the current mode.
    fn list_rect(&self, area: RECT) -> RECT {
        self.list_area(area, self.dpi())
    }

    fn list_height(&self) -> i32 {
        height(self.list_rect(self.client())).max(self.list.row_height)
    }

    fn row_rect(&self, list: RECT, index: usize) -> Option<RECT> {
        let top = self.list.row_top(index)?;
        Some(RECT {
            left: list.left,
            top: list.top + top,
            right: list.right,
            bottom: list.top + top + self.list.row_height,
        })
    }

    fn target(&self, index: usize) -> Target {
        match self.mode {
            Mode::NoNotebook => self
                .recent
                .get(index)
                .cloned()
                .map_or(Target::Nothing, Target::Recent),
            Mode::Tree => match self.rows.get(index) {
                Some(row) => Target::Row(row.clone()),
                None if self.truncated && index == self.rows.len() => Target::Truncated,
                None => Target::Nothing,
            },
            Mode::Loading | Mode::Empty => Target::Nothing,
        }
    }

    fn select(&mut self, index: usize) {
        let height = self.list_height();
        self.list.select(index, height);
        self.invalidate();
    }

    fn apply(&mut self, snapshot: Snapshot, names: Vec<(String, Option<String>)>) {
        let height = self.list_height();
        let reset = snapshot.mode != self.mode || snapshot.root != self.root;
        if reset {
            self.list = RowListState::new(self.list.row_height);
            self.typed = TypeAhead::default();
        }
        let selected = self
            .list
            .selected
            .and_then(|index| self.rows.get(index))
            .map(|row| row.kind.clone());
        let top = (!reset)
            .then(|| self.rows.get(self.list.top).map(|row| row.kind.clone()))
            .flatten();
        let (old_selected, old_top) = (self.list.selected, self.list.top);
        self.mode = snapshot.mode;
        self.name = snapshot
            .root
            .as_deref()
            .and_then(Path::file_name)
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.root = snapshot.root;
        self.favorite = snapshot.favorite;
        self.rows = snapshot.rows;
        self.truncated = snapshot.truncated;
        self.recent = snapshot.recent;
        self.recent_names = names;
        let count = match self.mode {
            Mode::Tree => self.rows.len() + usize::from(self.truncated),
            Mode::NoNotebook => self.recent.len(),
            Mode::Loading | Mode::Empty => 0,
        };
        self.list.set_count(count);
        if self.mode == Mode::Tree {
            self.list.selected = follow(&self.rows, selected.as_ref(), old_selected);
            self.list.top = follow(&self.rows, top.as_ref(), Some(old_top)).unwrap_or(0);
            self.list.scroll_lines(0, height);
        }
        self.hover_pin = false;
    }

    fn hit_test(&self, x: i32, y: i32) -> Hit {
        let area = self.client();
        let dpi = self.dpi();
        if y < area.top + scale(HEADER_HEIGHT, dpi) {
            let header = header_layout(area, dpi);
            if self.mode != Mode::NoNotebook {
                for (button, rect) in header.buttons {
                    if contains(rect, x, y) {
                        return Hit::Header(button);
                    }
                }
            }
            return if contains(header.title, x, y) {
                Hit::Title
            } else {
                Hit::Empty
            };
        }
        let body = body_rect(area, dpi);
        match self.mode {
            Mode::NoNotebook | Mode::Empty => {
                let layout = state_layout(body, dpi);
                if contains(layout.button, x, y) {
                    return Hit::StateButton;
                }
                if self.mode == Mode::NoNotebook
                    && contains(layout.list, x, y)
                    && let Some(index) = self.list.row_at(y - layout.list.top)
                {
                    return Hit::Row {
                        index,
                        part: RowPart::Body,
                    };
                }
                Hit::Empty
            }
            Mode::Loading => Hit::Empty,
            Mode::Tree => {
                let list = self.list_rect(area);
                if let Some(grab) = self.list.thumb_hit(
                    x - list.left,
                    y - list.top,
                    list.right - list.left,
                    height(list),
                ) {
                    return Hit::Thumb(grab);
                }
                let Some(index) = self.list.row_at(y - list.top) else {
                    return Hit::Empty;
                };
                let (Some(row), Some(rect)) = (self.rows.get(index), self.row_rect(list, index))
                else {
                    return Hit::Row {
                        index,
                        part: RowPart::Body,
                    };
                };
                let parts = row_parts(rect, row.depth, dpi);
                let part = match row.kind {
                    RowKind::Folder(_) if x < parts.icon.right => RowPart::Chevron,
                    RowKind::Note(_) if x >= parts.pin.left => RowPart::Pin,
                    _ => RowPart::Body,
                };
                Hit::Row { index, part }
            }
        }
    }

    /// `point` in panel coordinates, converted to the main window's client coordinates, which
    /// `menus::track_popup` takes.
    fn to_main(&self, point: POINT) -> POINT {
        let mut point = point;
        unsafe {
            ClientToScreen(self.panel, &mut point);
            ScreenToClient(GetParent(self.panel), &mut point);
        }
        point
    }

    fn text_width(&mut self, text: &str, font: HFONT) -> i32 {
        let wide = text.encode_utf16().collect::<Vec<_>>();
        if wide.is_empty() {
            return 0;
        }
        let mut size = SIZE::default();
        unsafe {
            let dc = GetDC(self.panel);
            if dc.is_null() {
                return 0;
            }
            let previous = SelectObject(dc, font);
            GetTextExtentPoint32W(dc, wide.as_ptr(), wide.len() as i32, &mut size);
            SelectObject(dc, previous);
            ReleaseDC(self.panel, dc);
        }
        size.cx
    }

    fn track_leave(&mut self) {
        if self.tracking_leave {
            return;
        }
        let mut track = TRACKMOUSEEVENT {
            cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
            dwFlags: TME_LEAVE,
            hwndTrack: self.panel,
            dwHoverTime: 0,
        };
        self.tracking_leave = unsafe { TrackMouseEvent(&mut track) } != 0;
    }

    fn tooltip(&mut self) -> Option<&Tooltip> {
        if self.tooltip.is_none() && !self.tooltip_failed {
            self.tooltip = Tooltip::create(self.panel);
            self.tooltip_failed = self.tooltip.is_none();
        }
        self.tooltip.as_ref()
    }

    /// The hovered row's tip: a truncated note name in full, or a recent notebook's path.
    fn row_tip(&mut self, fonts: UiFonts) -> (RECT, String) {
        let none = (RECT::default(), String::new());
        let Some(index) = self.list.hover else {
            return none;
        };
        let list = self.list_rect(self.client());
        let Some(rect) = self.row_rect(list, index) else {
            return none;
        };
        match self.mode {
            Mode::NoNotebook => self
                .recent
                .get(index)
                .map_or(none, |path| (rect, path.display().to_string())),
            Mode::Tree => {
                let Some(row) = self.rows.get(index).cloned() else {
                    return none;
                };
                let parts = row_parts(rect, row.depth, self.dpi());
                let font = if matches!(row.kind, RowKind::Unsaved(_)) {
                    fonts.italic
                } else {
                    fonts.text
                };
                if self.text_width(&row.name, font) > parts.name.right - parts.name.left {
                    (parts.name, row.name)
                } else {
                    none
                }
            }
            Mode::Loading | Mode::Empty => none,
        }
    }

    /// `fonts` measures whether the hovered name is cut off. A tool with an empty text is
    /// removed, so a hidden button shows no tip.
    fn update_tooltips(&mut self, fonts: UiFonts) {
        let area = self.client();
        let header = header_layout(area, self.dpi());
        let title = self
            .root
            .as_ref()
            .map(|root| root.display().to_string())
            .unwrap_or_default();
        let favorite = if self.favorite {
            "Remove from favorites"
        } else {
            "Add to favorites"
        };
        let buttons_shown = self.mode != Mode::NoNotebook;
        let (row_rect, row_text) = self.row_tip(fonts);
        let Some(tooltip) = self.tooltip() else {
            return;
        };
        tooltip.set_tool(TOOL_TITLE, header.title, &title);
        for (button, rect) in header.buttons {
            let (id, text) = match button {
                HeaderButton::Favorite => (TOOL_FAVORITE, favorite),
                HeaderButton::NewNote => (TOOL_NEW, "New note"),
                HeaderButton::More => (TOOL_MORE, "More actions"),
            };
            tooltip.set_tool(id, rect, if buttons_shown { text } else { "" });
        }
        tooltip.set_tool(TOOL_ROW, row_rect, &row_text);
    }

    fn paint(&mut self, paint: &ViewPaint) {
        let (dc, area, dpi, fonts, focused) =
            (paint.hdc, paint.client, paint.dpi, paint.fonts, paint.focused);
        let palette = &paint.palette;
        self.list.row_height = scale(ROW_HEIGHT, dpi);
        self.paint_header(dc, area, palette, fonts, dpi);
        let body = body_rect(area, dpi);
        let layout = state_layout(body, dpi);
        match self.mode {
            Mode::Loading => {
                unsafe { draw_text(dc, "Loading…", layout.message, fonts.text, palette.muted_foreground, LINE) };
            }
            Mode::NoNotebook => {
                let message = "Open a notebook to see its notes.";
                unsafe { draw_text(dc, message, layout.message, fonts.text, palette.editor_foreground, DT_WORDBREAK | DT_NOPREFIX) };
                self.paint_button(dc, layout.button, "Open notebook…", palette, fonts);
                if !self.recent.is_empty() {
                    unsafe { draw_text(dc, "RECENT", layout.label, fonts.bold, palette.muted_foreground, LINE) };
                }
                let names = &self.recent_names;
                row_list::paint(dc, layout.list, &self.list, palette, focused, &mut |dc, index, rect, look| {
                    draw_recent_row(dc, names.get(index), rect, look, palette, fonts, dpi);
                });
            }
            Mode::Empty => {
                let message = format!("No notes in {} yet.", self.name);
                unsafe { draw_text(dc, &message, layout.message, fonts.text, palette.editor_foreground, DT_WORDBREAK | DT_NOPREFIX) };
                self.paint_button(dc, layout.button, "New note", palette, fonts);
            }
            Mode::Tree => {
                // `row_list::paint` draws the rows in view and the scroll thumb.
                let list = self.list_rect(area);
                let rows = &self.rows;
                let hover_pin = self.hover_pin;
                row_list::paint(dc, list, &self.list, palette, focused, &mut |dc, index, rect, look| {
                    draw_tree_row(dc, rows.get(index), rect, look, palette, fonts, dpi, hover_pin && look.hover);
                });
            }
        }
    }

    fn paint_header(&self, dc: HDC, area: RECT, palette: &Palette, fonts: UiFonts, dpi: u32) {
        let layout = header_layout(area, dpi);
        let title = match self.mode {
            Mode::NoNotebook => "NOTEBOOK".to_owned(),
            _ => self.name.to_uppercase(),
        };
        unsafe { draw_text(dc, &title, layout.title, fonts.bold, palette.muted_foreground, LINE) };
        if self.mode == Mode::NoNotebook {
            return;
        }
        for (button, rect) in layout.buttons {
            let hot = self.hover == Some(Hit::Header(button));
            if hot {
                unsafe { fill(dc, rect, palette.hover_background) };
            }
            let glyph = match button {
                HeaderButton::Favorite if self.favorite => GLYPH_STAR_FILLED,
                HeaderButton::Favorite => GLYPH_STAR,
                HeaderButton::NewNote => GLYPH_ADD,
                HeaderButton::More => GLYPH_MORE,
            };
            let color = if hot {
                palette.hover_foreground
            } else {
                palette.muted_foreground
            };
            unsafe { draw_text(dc, glyph, rect, fonts.glyph, color, CENTERED) };
        }
    }

    fn paint_button(&self, dc: HDC, rect: RECT, text: &str, palette: &Palette, fonts: UiFonts) {
        let background = if self.hover == Some(Hit::StateButton) {
            palette.hover_background
        } else {
            palette.pressed_background
        };
        unsafe { fill(dc, rect, background) };
        unsafe { draw_text(dc, text, rect, fonts.text, palette.editor_foreground, CENTERED) };
    }
}

/// What Task 13's MSAA provider reads of the view.
#[allow(dead_code, reason = "Task 13's MSAA provider is the first reader")]
impl NotebookView {
    pub(crate) fn rows(&self) -> &[TreeRow] {
        &self.rows
    }

    pub(crate) fn list(&self) -> &RowListState {
        &self.list
    }

    pub(crate) fn list_mut(&mut self) -> &mut RowListState {
        &mut self.list
    }

    /// Every push button painted, in paint order, with its accessible name: the header's star,
    /// New note and "…" (not in the no-notebook state), then the state's own button.
    pub(crate) fn buttons(&self, client: RECT, dpi: u32) -> Vec<(String, RECT)> {
        let mut buttons = Vec::new();
        if self.mode != Mode::NoNotebook {
            for (button, rect) in header_layout(client, dpi).buttons {
                let name = match button {
                    HeaderButton::Favorite if self.favorite => "Remove from favorites",
                    HeaderButton::Favorite => "Add to favorites",
                    HeaderButton::NewNote => "New note",
                    HeaderButton::More => "More actions",
                };
                buttons.push((name.to_owned(), rect));
            }
        }
        let state = state_layout(body_rect(client, dpi), dpi);
        match self.mode {
            Mode::NoNotebook => buttons.push(("Open notebook…".to_owned(), state.button)),
            Mode::Empty => buttons.push(("New note".to_owned(), state.button)),
            Mode::Loading | Mode::Tree => {}
        }
        buttons
    }

    /// The no-notebook state's RECENT rows from the first one in view, with their names (and the
    /// parent-folder hint on a name clash). Empty in every other state.
    pub(crate) fn recent_rows(&self, client: RECT, dpi: u32) -> Vec<(String, RECT)> {
        if self.mode != Mode::NoNotebook {
            return Vec::new();
        }
        let list = state_layout(body_rect(client, dpi), dpi).list;
        (0..self.recent.len())
            .filter_map(|index| {
                let rect = self.row_rect(list, index)?;
                let name = match self.recent_names.get(index)? {
                    (name, Some(hint)) => format!("{name}, {hint}"),
                    (name, None) => name.clone(),
                };
                Some((name, rect))
            })
            .collect()
    }
}

/// What a rebuild read from the library, the tabs and `folders.ini`'s cache.
struct Snapshot {
    mode: Mode,
    rows: Vec<TreeRow>,
    truncated: bool,
    recent: Vec<PathBuf>,
    root: Option<PathBuf>,
    favorite: bool,
}

fn with_view<R>(hwnd: HWND, f: impl FnOnce(&mut NotebookView) -> R) -> Option<R> {
    unsafe { app_ptr(hwnd) }.and_then(|mut app| {
        unsafe { app.as_mut() }
            .sidebar
            .as_mut()
            .map(|sidebar| f(&mut sidebar.notebook))
    })
}

/// Reads everything the rows need. Each call borrows the App on its own, never nested.
fn snapshot(hwnd: HWND) -> Snapshot {
    let Some(root) = super::library_host::folder(hwnd) else {
        return Snapshot {
            mode: Mode::NoNotebook,
            rows: Vec::new(),
            truncated: false,
            recent: super::library_host::recent_notebooks(hwnd),
            root: None,
            favorite: false,
        };
    };
    let favorite = super::library_host::is_favorite(hwnd);
    let unsaved = unsafe { app_ptr(hwnd) }
        .map(|app| unsaved_entries(unsafe { app.as_ref() }.tabs.documents()))
        .unwrap_or_default();
    let built = super::library_host::with_state(hwnd, |state| {
        let state = &*state;
        let rows = state
            .tree
            .rows(&|path: &Path| state.local.is_expanded(path), &unsaved);
        (rows, state.truncated)
    });
    let (mode, rows, truncated) = match built {
        None => (Mode::Loading, Vec::new(), false),
        Some((rows, _)) if rows.is_empty() => (Mode::Empty, rows, false),
        Some((rows, truncated)) => (Mode::Tree, rows, truncated),
    };
    Snapshot {
        mode,
        rows,
        truncated,
        recent: Vec::new(),
        root: Some(root),
        favorite,
    }
}

/// Rebuilds the rows from the library, the tabs and the notebook lists, keeping the selection
/// and the scroll position by path. `side_panel::refresh` calls it.
pub(crate) fn rebuild(hwnd: HWND) {
    let snapshot = snapshot(hwnd);
    let names = crate::library::local::display_names(&snapshot.recent);
    with_view(hwnd, |view| {
        view.apply(snapshot, names);
        view.invalidate();
    });
}

/// The row for the active tab: its note inside the open notebook, or its unsaved entry.
fn active_target(hwnd: HWND) -> Option<RowKind> {
    let root = super::library_host::folder(hwnd)?;
    let (id, path) = unsafe { app_ptr(hwnd) }.and_then(|app| {
        let active = unsafe { app.as_ref() }.tabs.active()?;
        Some((active.id, active.path.clone()))
    })?;
    match path {
        None => Some(RowKind::Unsaved(id.0)),
        Some(path) => crate::library::is_inside(&root, &path)
            .then(|| RowKind::Note(crate::library::record_path(&root, &path))),
    }
}

/// Every tab switch: the active note's row is selected and its folders expand (remembered per
/// PC), without moving the keyboard focus (spec §6.1).
pub(crate) fn active_tab_changed(hwnd: HWND) {
    let target = active_target(hwnd);
    if let Some(RowKind::Note(relative)) = &target {
        for folder in tree::ancestors(relative) {
            super::library_host::set_expanded(hwnd, &folder, true);
        }
    }
    rebuild(hwnd);
    if let Some(kind) = target {
        with_view(hwnd, |view| {
            if let Some(index) = tree::row_index(&view.rows, &kind) {
                view.select(index);
            }
        });
    }
}

/// The panel's `WM_PAINT` while the Notebook view shows (`side_panel::paint_view`). The panel
/// has already filled its background.
pub(crate) fn paint(hwnd: HWND, paint: &ViewPaint) {
    with_view(hwnd, |view| view.paint(paint));
}

/// Whether panel point (`x`, `y`) is over the header's name or a header button, which must stay
/// client area. The rest of the header is a window drag area (spec §6.5).
pub(crate) fn header_hit(hwnd: HWND, x: i32, y: i32) -> bool {
    with_view(hwnd, |view| {
        matches!(view.hit_test(x, y), Hit::Header(_) | Hit::Title)
    })
    .unwrap_or(false)
}

/// Runs a main-window command as the menus do.
fn run(hwnd: HWND, command: CommandId) {
    unsafe {
        SendMessageW(hwnd, WM_COMMAND, command as usize, 0);
    }
}

/// The panel's input while the Notebook view is shown (`side_panel::view_mouse` and `view_key`).
/// `None` leaves the message to `DefWindowProcW`. The panel handles its resize edge itself.
pub(crate) fn handle(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
    match message {
        WM_MOUSEMOVE => {
            let (x, y) = point_of(lparam);
            mouse_move(hwnd, x, y);
            Some(0)
        }
        // The panel class has CS_DBLCLKS: the second press of a double-click comes as this.
        WM_LBUTTONDBLCLK => {
            let (x, y) = point_of(lparam);
            double_click(hwnd, x, y);
            Some(0)
        }
        WM_MOUSELEAVE => {
            with_view(hwnd, |view| {
                view.tracking_leave = false;
                view.list.hover = None;
                view.hover = None;
                view.hover_pin = false;
                view.invalidate();
            });
            Some(0)
        }
        WM_LBUTTONDOWN => {
            let (x, y) = point_of(lparam);
            left_down(hwnd, x, y);
            Some(0)
        }
        WM_LBUTTONUP => {
            with_view(hwnd, |view| {
                if view.thumb_grab.take().is_some() {
                    unsafe {
                        ReleaseCapture();
                    }
                }
            });
            Some(0)
        }
        WM_CAPTURECHANGED => {
            with_view(hwnd, |view| view.thumb_grab = None);
            Some(0)
        }
        WM_RBUTTONDOWN => {
            // Selects the row; DefWindowProc turns the button-up into WM_CONTEXTMENU.
            let (x, y) = point_of(lparam);
            with_view(hwnd, |view| {
                unsafe {
                    SetFocus(view.panel);
                }
                if let Hit::Row { index, .. } = view.hit_test(x, y) {
                    view.select(index);
                }
            });
            Some(0)
        }
        WM_KEYDOWN => key_down(hwnd, wparam as u16).then_some(0),
        WM_CHAR => {
            let ch = char::from_u32(wparam as u32).filter(|ch| !ch.is_control())?;
            typed(hwnd, ch);
            Some(0)
        }
        WM_MOUSEWHEEL => {
            let delta = i32::from((wparam >> 16) as u16 as i16);
            let lines = wheel_lines();
            with_view(hwnd, |view| {
                let height = view.list_height();
                if view.list.wheel(delta, lines, height) {
                    view.invalidate();
                }
            });
            Some(0)
        }
        _ => None,
    }
}

/// The user's wheel setting (`SPI_GETWHEELSCROLLLINES`), 3 lines if it can't be read.
fn wheel_lines() -> u32 {
    let mut lines = 3u32;
    let read = unsafe {
        SystemParametersInfoW(
            SPI_GETWHEELSCROLLLINES,
            0,
            (&mut lines as *mut u32).cast(),
            0,
        )
    };
    if read == 0 { 3 } else { lines }
}

fn mouse_move(hwnd: HWND, x: i32, y: i32) {
    // Read before the view is borrowed: `ui_fonts` borrows the App itself.
    let fonts = super::main_window::ui_fonts(hwnd);
    with_view(hwnd, |view| {
        if let Some(grab) = view.thumb_grab {
            let list = view.list_rect(view.client());
            if view.list.drag_thumb(grab, y - list.top, height(list)) {
                view.invalidate();
            }
            return;
        }
        view.track_leave();
        let hit = view.hit_test(x, y);
        let (row, pin) = match hit {
            Hit::Row { index, part } => (Some(index), part == RowPart::Pin),
            _ => (None, false),
        };
        let hot = matches!(hit, Hit::Header(_) | Hit::StateButton).then_some(hit);
        let row_changed = view.list.set_hover(row);
        if row_changed || view.hover_pin != pin || view.hover != hot {
            view.hover_pin = pin;
            view.hover = hot;
            view.invalidate();
            view.update_tooltips(fonts);
        }
    });
}

fn left_down(hwnd: HWND, x: i32, y: i32) {
    let Some(hit) = with_view(hwnd, |view| {
        unsafe {
            SetFocus(view.panel);
        }
        view.hit_test(x, y)
    }) else {
        return;
    };
    match hit {
        Hit::Header(button) => header_clicked(hwnd, button),
        Hit::StateButton => state_button(hwnd),
        Hit::Thumb(grab) => {
            with_view(hwnd, |view| {
                view.thumb_grab = Some(grab);
                unsafe {
                    SetCapture(view.panel);
                }
            });
        }
        Hit::Row { index, part } => {
            with_view(hwnd, |view| view.select(index));
            row_clicked(hwnd, index, part, false);
        }
        Hit::Title | Hit::Empty => {}
    }
}

/// `WM_LBUTTONDBLCLK`: a row's second press opens it as a normal tab, which also promotes its
/// preview (spec §6.4). Anywhere else it is one more click.
fn double_click(hwnd: HWND, x: i32, y: i32) {
    match with_view(hwnd, |view| view.hit_test(x, y)) {
        Some(Hit::Row { index, part }) => row_clicked(hwnd, index, part, true),
        Some(_) => left_down(hwnd, x, y),
        None => {}
    }
}

/// A press on row `index`. `double` is the second press of a double-click, whose first press
/// already toggled a pin or a folder, or started opening a recent notebook.
fn row_clicked(hwnd: HWND, index: usize, part: RowPart, double: bool) {
    let Some(target) = with_view(hwnd, |view| view.target(index)) else {
        return;
    };
    match target {
        Target::Row(TreeRow {
            kind: RowKind::Note(relative),
            ..
        }) if part == RowPart::Pin => {
            if !double && let Some(root) = super::library_host::folder(hwnd) {
                super::library_host::toggle_pin(hwnd, &root.join(relative));
            }
        }
        Target::Row(TreeRow {
            kind: RowKind::Folder(_),
            ..
        })
        | Target::Recent(_)
            if double => {}
        _ => activate(
            hwnd,
            index,
            if double {
                Activation::Permanent
            } else {
                Activation::Click
            },
        ),
    }
}

fn set_folder_expanded(hwnd: HWND, relative: &Path, expanded: bool) {
    super::library_host::set_expanded(hwnd, relative, expanded);
    rebuild(hwnd);
}

/// Opens or toggles row `index` (spec §6.4). A folder toggles, a note opens, an unsaved row
/// switches to its tab, and a recent notebook opens.
pub(crate) fn activate(hwnd: HWND, index: usize, how: Activation) {
    let Some(target) = with_view(hwnd, |view| view.target(index)) else {
        return;
    };
    match target {
        Target::Recent(folder) => super::library_host::open_listed_notebook(hwnd, &folder),
        Target::Row(row) => match row.kind {
            RowKind::Folder(relative) => set_folder_expanded(hwnd, &relative, !row.expanded),
            RowKind::Note(relative) => {
                let Some(root) = super::library_host::folder(hwnd) else {
                    return;
                };
                let path = root.join(relative);
                let (mode, focus) = match how {
                    Activation::Click => (OpenMode::Preview, true),
                    Activation::Enter => (OpenMode::Preview, false),
                    Activation::Permanent => (OpenMode::Permanent, true),
                };
                if let Err(error) = super::main_window::open_note(hwnd, &path, mode, focus) {
                    super::main_window::report_open_failure(hwnd, &path, &error);
                }
            }
            RowKind::Unsaved(key) => {
                if super::main_window::activate_document_by_id(hwnd, DocumentId(key))
                    && how != Activation::Enter
                {
                    super::main_window::focus_content(hwnd);
                }
            }
        },
        Target::Truncated | Target::Nothing => {}
    }
}

/// The header's buttons (spec §6.5).
pub(crate) fn header_clicked(hwnd: HWND, button: HeaderButton) {
    match button {
        HeaderButton::Favorite => super::library_host::toggle_notebook_favorite(hwnd),
        HeaderButton::NewNote => run(hwnd, CommandId::New),
        HeaderButton::More => more_menu(hwnd),
    }
}

/// "…": the notebook's own actions.
fn more_menu(hwnd: HWND) {
    let Some(at) = with_view(hwnd, |view| {
        let rect = header_layout(view.client(), view.dpi()).buttons[2].1;
        view.to_main(POINT {
            x: rect.left,
            y: rect.bottom,
        })
    }) else {
        return;
    };
    let entries = [MenuEntry::command("Close notebook", CommandId::CloseNotebook)];
    if let Some(command) = super::menus::track_popup(hwnd, &entries, at) {
        run(hwnd, command);
    }
}

fn state_button(hwnd: HWND) {
    match with_view(hwnd, |view| view.mode) {
        Some(Mode::NoNotebook) => super::library_host::choose_and_open_folder(hwnd),
        Some(Mode::Empty) => run(hwnd, CommandId::New),
        _ => {}
    }
}

/// The tree's keys (spec §10). Returns false for keys it leaves to the panel.
pub(crate) fn key_down(hwnd: HWND, key: u16) -> bool {
    if let Some(list_key) = ListKey::from_virtual_key(u32::from(key)) {
        with_view(hwnd, |view| {
            let height = view.list_height();
            if view.list.move_selection(list_key, height) {
                view.invalidate();
            }
        });
        return true;
    }
    let Some(selected) = with_view(hwnd, |view| view.list.selected).flatten() else {
        return matches!(key, VK_RETURN | VK_LEFT | VK_RIGHT);
    };
    match key {
        VK_RETURN => {
            let ctrl = unsafe { GetKeyState(VK_CONTROL as i32) } < 0;
            let how = if ctrl {
                Activation::Permanent
            } else {
                Activation::Enter
            };
            activate(hwnd, selected, how);
            true
        }
        VK_RIGHT => {
            right(hwnd, selected);
            true
        }
        VK_LEFT => {
            left(hwnd, selected);
            true
        }
        _ => false,
    }
}

/// Right expands a folder, or moves into an expanded one.
fn right(hwnd: HWND, index: usize) {
    let Some(Target::Row(row)) = with_view(hwnd, |view| view.target(index)) else {
        return;
    };
    let RowKind::Folder(relative) = &row.kind else {
        return;
    };
    if !row.expanded {
        set_folder_expanded(hwnd, relative, true);
        return;
    }
    with_view(hwnd, |view| {
        if view
            .rows
            .get(index + 1)
            .is_some_and(|child| child.depth > row.depth)
        {
            view.select(index + 1);
        }
    });
}

/// Left collapses an expanded folder, or moves to the parent folder.
fn left(hwnd: HWND, index: usize) {
    let Some(Target::Row(row)) = with_view(hwnd, |view| view.target(index)) else {
        return;
    };
    if let RowKind::Folder(relative) = &row.kind
        && row.expanded
    {
        set_folder_expanded(hwnd, relative, false);
        return;
    }
    with_view(hwnd, |view| {
        if let Some(parent) = tree::parent_index(&view.rows, index) {
            view.select(parent);
        }
    });
}

/// Type-ahead: the next row whose name starts with what was typed in the last second. A single
/// letter searches from the row after the selection, so repeating it steps through matches.
fn typed(hwnd: HWND, ch: char) {
    with_view(hwnd, |view| {
        if view.mode != Mode::Tree {
            return;
        }
        let prefix = view.typed.push(ch, Instant::now()).to_owned();
        let from = match view.list.selected {
            Some(selected) if prefix.chars().count() == 1 => selected + 1,
            Some(selected) => selected,
            None => 0,
        };
        if let Some(index) = tree::type_ahead(&view.rows, from, &prefix) {
            view.select(index);
        }
    });
}
```

`row_list::paint`'s `draw_row` closure is `&mut dyn FnMut(HDC, usize, RECT, RowLook)`, as the contract says. The closures above capture only shared references.

- [ ] **Step 4: Expansion in `library_host`, and refreshing on label changes**

In `src/window/library_host.rs`:

```rust
/// Expands or collapses `path`, a folder relative to the open notebook, and remembers it in the
/// per-PC file. The write happens on a writer thread, and only when the set changed.
pub(crate) fn set_expanded(hwnd: HWND, path: &Path, expanded: bool) {
    let changed = with_state(hwnd, |state| {
        let was = state.local.is_expanded(path);
        state.local.set_expanded(path, expanded);
        was != expanded
    })
    .unwrap_or(false);
    if changed {
        save_local(
            hwnd,
            LocalWrite {
                wait: false,
                force: false,
            },
        );
    }
}

/// The open notebook's expanded folders, relative to it.
pub(crate) fn expanded(hwnd: HWND) -> Vec<PathBuf> {
    with_state(hwnd, |state| state.local.expanded.clone()).unwrap_or_default()
}
```

In `refresh_label`, an untitled tab's label is also its unsaved row's name. Replace the final `if changed { ... }` with:

```rust
    if changed {
        super::main_window::refresh_tab_view(hwnd);
        super::side_panel::refresh(hwnd);
    }
```

- [ ] **Step 5: Open up the popup helper**

In `src/window/menus.rs`, make `enum MenuEntry` `pub(crate) enum MenuEntry`, `const fn command` `pub(crate) const fn command`, and `fn track_popup` `pub(crate) fn track_popup`. Nothing else changes.

- [ ] **Step 6: Route the Notebook view through Task 6's seam in `side_panel`**

Add `pub(crate) mod notebook_view;` to `src/window/mod.rs`. Then, in `src/window/side_panel.rs`:

1. **`Sidebar` owns the view.** Add the field after `bar_state`:

```rust
    /// The Notebook view's rows, selection and hover.
    pub(crate) notebook: crate::window::notebook_view::NotebookView,
```

In `create`, the `Ok(Sidebar { .. })` literal gains it after `bar_state: BarState::default(),`:

```rust
        notebook: crate::window::notebook_view::NotebookView::new(panel),
```

2. **Paint.** Replace `paint_view`:

```rust
/// Paints `view` over the panel's background. Task 12 gives the Search and Favorites views their
/// own paint.
fn paint_view(main: HWND, view: PanelView, paint: &ViewPaint) {
    match view {
        PanelView::Notebook => crate::window::notebook_view::paint(main, paint),
        PanelView::Search | PanelView::Favorites => paint_header_title(view, paint),
    }
}
```

3. **Mouse input.** Replace `view_mouse`:

```rust
/// Mouse input (and `WM_CONTEXTMENU`, `WM_MOUSELEAVE`, `WM_CAPTURECHANGED`) for `view`, with the
/// message's own `wparam` and `lparam`. `None` leaves it to `DefWindowProcW`. Task 12 replaces
/// the Search and Favorites arm.
fn view_mouse(
    main: HWND,
    view: PanelView,
    panel: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> Option<LRESULT> {
    match view {
        PanelView::Notebook => {
            crate::window::notebook_view::handle(main, message, wparam, lparam)
        }
        PanelView::Search | PanelView::Favorites => (message == WM_LBUTTONDOWN).then(|| {
            unsafe { SetFocus(panel) };
            0
        }),
    }
}
```

4. **Keys.** Replace `view_key`:

```rust
/// `WM_KEYDOWN` and `WM_CHAR` while the panel has the focus. `None` leaves the key to
/// `DefWindowProcW`. Task 12 handles the lists' keys.
fn view_key(
    main: HWND,
    view: PanelView,
    _panel: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> Option<LRESULT> {
    match view {
        PanelView::Notebook => {
            crate::window::notebook_view::handle(main, message, wparam, lparam)
        }
        PanelView::Search | PanelView::Favorites => None,
    }
}
```

5. **The header's drag area.** Replace `header_is_caption`, so the notebook's name and the header buttons stay client area (`WM_NCHITTEST` then falls through to `DefWindowProcW`, which answers `HTCLIENT`):

```rust
/// Whether header point `x`, `y` (panel client coordinates) is empty, so the window drags from
/// it. Task 12 excludes the Favorites header's Open notebook… button.
fn header_is_caption(main: HWND, view: PanelView, _panel: HWND, x: i32, y: i32) -> bool {
    match view {
        PanelView::Notebook => !crate::window::notebook_view::header_hit(main, x, y),
        PanelView::Search | PanelView::Favorites => true,
    }
}
```

6. **Rows follow the library and the tabs.** Replace `refresh` and `active_tab_changed`:

```rust
/// The library changed: rebuilds the Notebook view's rows from `LibraryState.tree` (no disk),
/// re-reads the notebook name for the Notebook tooltip, and repaints.
pub(crate) fn refresh(hwnd: HWND) {
    let Some((bar, panel)) = windows(hwnd) else {
        return;
    };
    crate::window::notebook_view::rebuild(hwnd);
    update_tools(hwnd);
    unsafe {
        InvalidateRect(bar, std::ptr::null(), 0);
        InvalidateRect(panel, std::ptr::null(), 0);
    }
}

/// The active tab changed (`main_window::refresh_tabs` calls it): the Notebook view selects the
/// active note's row and expands its folders.
pub(crate) fn active_tab_changed(hwnd: HWND) {
    let Some((_, panel)) = windows(hwnd) else {
        return;
    };
    crate::window::notebook_view::active_tab_changed(hwnd);
    unsafe { InvalidateRect(panel, std::ptr::null(), 0) };
}
```

7. **A hidden view's rows may be stale.** In `show_view`, right after the `change_setting(..)` call and before `layout_editor_and_find_bar(hwnd);`, add:

```rust
    if view == SidebarView::Notebook {
        crate::window::notebook_view::rebuild(hwnd);
    }
```

8. **Task 6's caption test.** The Notebook header's title is client area now, so `the_sidebar_top_strip_and_panel_header_are_caption` in the `main_window.rs` tests probes left of it. Replace its `let panel_x = client_size(panel).0 / 2;` line with:

```rust
        // Left of the Notebook header's title (which starts 12 px in and stays client area, so
        // its tooltip works): empty header, a drag area.
        let panel_x = crate::window::panel::scale(4, dpi);
        assert_eq!(
            hit(panel, client_size(panel).0 / 2, header_y),
            HTCLIENT as LRESULT,
            "the notebook's name is not a drag area"
        );
```

Delete the two `#[allow(dead_code, reason = ..)]` attributes Task 6 put on `UiFonts` and `ViewPaint`: the Notebook view reads every field now.

`src/window/row_list.rs`: the Notebook view is the list's first non-test user, so delete the module-level `#![cfg_attr(not(test), allow(dead_code, reason = "the sidebar views use it from Task 10"))]` attribute and the `// Task 10 is the first non-test user; it removes this.` comment above it. The Notebook view uses every item of `row_list` (`from_virtual_key` in `key_down`, `set_hover` in `mouse_move`, `wheel` for `WM_MOUSEWHEEL`, `thumb_hit` and `drag_thumb` for the thumb).

- [ ] **Step 7: Run the tests to verify they pass**

Run: `cargo test --lib window::notebook_view`
Expected: the six pure tests PASS.

Run: `cargo test --lib -- a_rescan_keeps_selection clicking_a_note_row switching_to_a_note_in_a_subfolder right_expands_a_folder the_view_says_loading an_empty_notebook_says_so the_header_star the_sidebar_top_strip --test-threads=1`
Expected: all eight PASS.

Run: `cargo clippy --all-targets -- -D warnings` and `cargo fmt --all -- --check`
Expected: no warnings, no diff.

- [ ] **Step 8: Commit**

```bash
git add src/window/notebook_view.rs src/window/mod.rs src/window/side_panel.rs src/window/library_host.rs src/window/menus.rs src/window/main_window.rs
git commit -m "feat(sidebar): Notebook view with folder tree, pins, preview opening, type-ahead and states"
```

---

### Task 11: Context menus, Move to notebook, Reveal in Explorer, where new notes go

**Files:**
- Create: `src/platform/shell.rs`
- Modify: `src/platform/mod.rs` (`pub mod shell;`), `src/platform/files.rs`, `src/document.rs`, `src/window/library_host.rs`, `src/window/notebook_view.rs`, `src/window/command_palette.rs`, `src/window/commands.rs`, `src/window/menus.rs`, `src/window/main_window.rs`

**Interfaces:**
- Consumes:
  - Task 1: `toggle_pin` and the `PendingOp::Drop` operation.
  - Task 2: `local::display_names`.
  - Task 3: `tree::natural_cmp`.
  - Task 8: `known_folders` (private) and `notebook_name`.
  - Task 9: `open_note` and `OpenMode`.
  - Task 10: the `NotebookView` internals and `menus::track_popup`.
- Produces:
  - `platform::files::move_file(from: &Path, to: &Path) -> crate::Result<()>`.
  - `platform::shell::reveal_in_explorer(path: &Path) -> crate::Result<()>`, `platform::shell::select_argument(path: &Path) -> String` and, under `cfg(test)`, `platform::shell::take_revealed() -> Vec<PathBuf>`.
  - `Document.save_folder: Option<PathBuf>`.
  - In `library_host`:
    - `new_note_in(hwnd, folder: Option<PathBuf>)` and `first_save_folder(hwnd) -> Option<PathBuf>`;
    - `move_to_notebook(hwnd, path: &Path)`, with `PickerKind::MoveToNotebook` re-added;
    - `reveal(hwnd, path: &Path)`;
    - `rename_file(hwnd, path: &Path)` and `delete_file(hwnd, path: &Path)`. `delete_note` now calls `delete_file`.
  - In `notebook_view`:
    - `open_context_menu(hwnd, index: usize, at: Option<POINT>)`;
    - `focused_note(hwnd) -> Option<PathBuf>`;
    - `selected_folder(hwnd) -> Option<PathBuf>`.
  - `CommandId::NoteRevealInExplorer = 182`. `CommandId::NoteMoveToNotebook` has the accelerator Ctrl+Shift+M. The palette gains "Note: Move to notebook..." and "Note: Reveal in Explorer".

Decisions:
- **Reveal:** `SHOpenFolderAndSelectItems` and `ILCreateFromPathW` are gated behind `Win32_UI_Shell_Common` in windows-sys 0.61, which is not enabled. `ShellExecuteW("explorer.exe", "/select,\"<path>\"")` does the same job with the features already on, so no feature is added. Under `cfg(test)` the call is recorded instead of starting Explorer.
- **Context menu entries** are `CommandId`s, since `track_popup` returns one. The menu acts on its row, not on the active tab, so it interprets the result itself instead of calling `execute_command`. "Open in new tab" reuses `CommandId::Open` and "New note here" reuses `CommandId::New`. No command numbers are spent on menu-only entries.
- **Move:** favorites come first, sorted by name, then recent notebooks, never the open one and never twice, then "Browse…". Moving the file never replaces an existing one (`MoveFileExW` with `MOVEFILE_COPY_ALLOWED | MOVEFILE_WRITE_THROUGH`). A name clash or any failure changes nothing and shows a notice.
  - The pin record is dropped (`PendingOp::Drop`), since pins belong to a notebook. If the destination is a folder inside the open notebook (Browse can pick one), the note stays indexed and keeps its pin through `rename_note`.
  - The tab follows through `rebind_open_tab` and, outside the notebook, gets no autosave (`autosave_target` checks `is_inside`).
- **Keys in the tree:** F2 and Del act on the selected note. A note that is not open is opened as a normal tab first for Rename, because the name box renames a tab. Delete works on the file and closes its tab if it has one. Shift+F10 and the context-menu key arrive as `WM_CONTEXTMENU` with lParam `-1`, which the panel's `DefWindowProcW` generates.
- **Palette and accelerator commands** (Toggle pin, Rename, Delete, Move to notebook, Reveal) act on the selected tree row while the panel has focus, and on the active tab otherwise (spec §6.3).

- [ ] **Step 1: Write the failing tests**

In `src/platform/files.rs` `mod tests`:

```rust
#[test]
fn moving_a_file_never_replaces_the_target_and_lands_in_another_folder() {
    // Break caught: a move onto a same-named note in the destination destroying that note, or
    // a "move" that leaves the source behind.
    let dir = scratch("move");
    std::fs::create_dir_all(dir.join("other")).unwrap();
    std::fs::write(dir.join("a.md"), "a").unwrap();
    std::fs::write(dir.join("other").join("b.md"), "b").unwrap();
    assert!(move_file(&dir.join("a.md"), &dir.join("other").join("b.md")).is_err());
    assert_eq!(std::fs::read_to_string(dir.join("other").join("b.md")).unwrap(), "b");
    assert!(dir.join("a.md").exists());
    move_file(&dir.join("a.md"), &dir.join("other").join("a.md")).unwrap();
    assert!(!dir.join("a.md").exists());
    assert_eq!(std::fs::read_to_string(dir.join("other").join("a.md")).unwrap(), "a");
    let _ = std::fs::remove_dir_all(&dir);
}
```

In the new `src/platform/shell.rs`, which Step 3 fills in:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explorer_is_asked_to_select_the_quoted_path() {
        // Break caught: a path with spaces or commas split by Explorer's argument parser, which
        // then opens Documents instead of the note's folder.
        assert_eq!(
            select_argument(Path::new(r"C:\My Notes\a, b.md")),
            r#"/select,"C:\My Notes\a, b.md""#
        );
    }
}
```

In `src/window/commands.rs` `mod tests`:

```rust
#[test]
fn reveal_in_explorer_has_a_stable_value() {
    assert_eq!(CommandId::try_from(182), Ok(CommandId::NoteRevealInExplorer));
    assert!(CommandId::NoteRevealInExplorer.needs_document());
}
```

In the `src/window/main_window.rs` tests (`open_note` is the existing test helper; `notebook_view`, `row_of` and `select_row` are Task 10's):

```rust
fn write_notebooks(data: &std::path::Path, folders: Vec<PathBuf>, favorites: Vec<PathBuf>) {
    crate::library::local::write_folders(
        &crate::library::local::folders_file(data),
        &crate::library::local::RecentFolders {
            folders,
            favorites,
            closed: false,
        },
    )
    .unwrap();
}

#[test]
fn moving_a_note_to_another_notebook_moves_the_file_drops_its_pin_and_its_tab_follows() {
    // Break caught: a move that copies without deleting, a pin record left pointing at a file
    // that left the notebook, or a tab still on the old path, where autosave would recreate it.
    let _scintilla = load_native_scintilla();
    let first = LibraryScratch::new("move-a");
    let second = LibraryScratch::new("move-b");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    app_mut(window.hwnd).library.data_dir = Some(first.data());
    let a = open_note(&window, &first, "a.md", "a");
    crate::window::library_host::toggle_pin(window.hwnd, &a);
    write_notebooks(&first.data(), vec![first.folder(), second.folder()], vec![]);

    execute_command(window.hwnd, CommandId::NoteMoveToNotebook);
    crate::window::library_host::picked(
        window.hwnd,
        crate::window::command_palette::PickerKind::MoveToNotebook,
        crate::window::command_palette::PickerChoice::Item(0),
    );

    let moved = second.folder().join("a.md");
    assert!(!a.exists());
    assert_eq!(std::fs::read_to_string(&moved).unwrap(), "a");
    assert_eq!(
        app_mut(window.hwnd).tabs.active().unwrap().path.as_deref(),
        Some(moved.as_path())
    );
    crate::window::library_host::with_state(window.hwnd, |state| {
        assert!(!state.is_pinned(&a));
        assert!(state.record_for(&a).is_none());
        assert!(!state.notes.iter().any(|note| note.path == std::path::Path::new("a.md")));
    });
    editor.set_text("b").unwrap();
    assert_eq!(
        crate::window::library_host::autosave_active(window.hwnd),
        crate::window::library_host::Autosave::NotEligible,
        "a plain file outside the notebook now"
    );
}

#[test]
fn a_move_onto_an_existing_name_changes_nothing_and_says_why() {
    // Break caught: MoveFileExW's replace flag, or a fallback copy, overwriting the other
    // notebook's note of the same name.
    let _scintilla = load_native_scintilla();
    let first = LibraryScratch::new("move-clash-a");
    let second = LibraryScratch::new("move-clash-b");
    let theirs = second.note("a.md", "theirs");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    app_mut(window.hwnd).library.data_dir = Some(first.data());
    let a = open_note(&window, &first, "a.md", "mine");
    write_notebooks(&first.data(), vec![second.folder()], vec![]);

    crate::window::library_host::move_to_notebook(window.hwnd, &a);
    crate::window::library_host::picked(
        window.hwnd,
        crate::window::command_palette::PickerKind::MoveToNotebook,
        crate::window::command_palette::PickerChoice::Item(0),
    );

    assert_eq!(std::fs::read_to_string(&a).unwrap(), "mine");
    assert_eq!(std::fs::read_to_string(&theirs).unwrap(), "theirs");
    assert_eq!(
        app_mut(window.hwnd).tabs.active().unwrap().path.as_deref(),
        Some(a.as_path())
    );
    assert!(notices(window.hwnd).iter().any(|n| n.contains("already exists")));
}

#[test]
fn move_offers_favorites_by_name_then_recent_never_the_open_one_then_browse() {
    // Break caught: the open notebook offered as a destination, a notebook listed twice, or
    // Browse… not reachable.
    let _scintilla = load_native_scintilla();
    let first = LibraryScratch::new("move-list");
    let third = LibraryScratch::new("move-browse");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    app_mut(window.hwnd).library.data_dir = Some(first.data());
    let a = open_note(&window, &first, "a.md", "a");
    let (zeta, alpha, beta) = (
        first.root.join("Zeta"),
        first.root.join("alpha"),
        first.root.join("beta"),
    );
    write_notebooks(
        &first.data(),
        vec![first.folder(), beta.clone(), alpha.clone()],
        vec![zeta.clone(), alpha.clone(), first.folder()],
    );

    crate::window::library_host::move_to_notebook(window.hwnd, &a);
    let (note, destinations) = app_mut(window.hwnd).library.shown_move.clone().unwrap();
    assert_eq!(note, a);
    assert_eq!(destinations, vec![alpha, zeta, beta]);

    crate::window::answer_next_folder_dialog({
        let folder = third.folder();
        move |_| Some(folder)
    });
    crate::window::library_host::picked(
        window.hwnd,
        crate::window::command_palette::PickerKind::MoveToNotebook,
        crate::window::command_palette::PickerChoice::Item(3),
    );
    assert!(third.folder().join("a.md").exists());
}

#[test]
fn a_new_note_saves_into_the_folder_selected_when_it_was_created_or_the_root_if_that_is_gone() {
    // Break caught: Ctrl+N with a subfolder selected saving into the notebook root anyway, or
    // a first save failing because the remembered folder was deleted meanwhile.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("new-note-folder");
    std::fs::create_dir_all(scratch.folder().join("sub")).unwrap();
    scratch.note(r"sub\b.md", "b");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    ensure_sidebar(window.hwnd);
    app_mut(window.hwnd).library.data_dir = Some(scratch.data());
    scratch.install(window.hwnd);
    crate::window::library_host::set_expanded(window.hwnd, std::path::Path::new("sub"), true);
    crate::window::notebook_view::rebuild(window.hwnd);
    select_row(window.hwnd, &RowKind::Note(r"sub\b.md".into()));

    execute_command(window.hwnd, CommandId::New);
    assert_eq!(
        app_mut(window.hwnd).tabs.active().unwrap().save_folder,
        Some(scratch.folder().join("sub"))
    );
    editor.set_text("Idea").unwrap();
    execute_command(window.hwnd, CommandId::Save);
    crate::window::library_host::name_box_submit(window.hwnd);
    assert!(scratch.folder().join(r"sub\Idea.md").exists());

    let gone = scratch.folder().join("gone");
    crate::window::library_host::new_note_in(window.hwnd, Some(gone));
    editor.set_text("Other").unwrap();
    execute_command(window.hwnd, CommandId::Save);
    crate::window::library_host::name_box_submit(window.hwnd);
    assert!(scratch.folder().join("Other.md").exists());
}

#[test]
fn the_context_menu_acts_on_its_row_not_the_active_tab() {
    // Break caught: Pin from a row's menu pinning the active tab's note instead, "New note
    // here" ignoring the folder, or Close tab on an unsaved row closing another tab.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("context-menu");
    std::fs::create_dir_all(scratch.folder().join("sub")).unwrap();
    scratch.note("b.md", "b");
    scratch.note(r"sub\c.md", "c");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    ensure_sidebar(window.hwnd);
    app_mut(window.hwnd).library.data_dir = Some(scratch.data());
    let a = open_note(&window, &scratch, "a.md", "a");
    let menu = |kind: &RowKind, answer: CommandId| {
        crate::window::menus::answer_next_popup_menu(move |_| Some(answer));
        let index = row_of(window.hwnd, kind);
        crate::window::notebook_view::open_context_menu(window.hwnd, index, None);
    };

    menu(&RowKind::Note("b.md".into()), CommandId::NoteTogglePin);
    crate::window::library_host::with_state(window.hwnd, |state| {
        assert!(state.is_pinned(&scratch.folder().join("b.md")));
        assert!(!state.is_pinned(&a));
    });

    menu(&RowKind::Folder("sub".into()), CommandId::New);
    let untitled = app_mut(window.hwnd).tabs.active().unwrap();
    assert_eq!(untitled.save_folder, Some(scratch.folder().join("sub")));
    let untitled = untitled.id;

    let before = super::tab_count(window.hwnd);
    menu(&RowKind::Unsaved(untitled.0), CommandId::CloseTab);
    assert_eq!(super::tab_count(window.hwnd), before - 1);
    assert!(app_mut(window.hwnd).tabs.document(untitled).is_none());

    menu(&RowKind::Folder("sub".into()), CommandId::NoteRevealInExplorer);
    execute_command(window.hwnd, CommandId::NoteRevealInExplorer);
    assert_eq!(
        crate::platform::shell::take_revealed(),
        vec![scratch.folder().join("sub"), a.clone()]
    );
}

#[test]
fn f2_on_a_note_row_that_is_not_open_opens_it_and_the_rename_box() {
    // Break caught: F2 in the tree renaming the active tab's note instead of the selected one,
    // or doing nothing because the name box needs a tab.
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_F2;
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("tree-f2");
    let a = scratch.note("a.md", "a");
    let b = scratch.note("b.md", "b");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    ensure_sidebar(window.hwnd);
    scratch.install(window.hwnd);
    super::open_path(window.hwnd, &a).unwrap();
    select_row(window.hwnd, &RowKind::Note("b.md".into()));

    assert!(crate::window::notebook_view::key_down(window.hwnd, VK_F2));

    let active = app_mut(window.hwnd).tabs.active().unwrap();
    assert_eq!(active.path.as_deref(), Some(b.as_path()));
    assert!(!active.preview);
    let id = active.id;
    let name_box = app_mut(window.hwnd).name_box.as_ref().unwrap();
    assert!(name_box.is_visible());
    assert_eq!(
        name_box.purpose(),
        Some(&crate::window::name_box::NamePurpose::RenameNote(id))
    );
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib -- moving_a_file_never explorer_is_asked reveal_in_explorer_has moving_a_note_to_another a_move_onto_an_existing move_offers_favorites a_new_note_saves_into the_context_menu_acts f2_on_a_note_row --test-threads=1`
Expected: a compile error (`move_file`, `shell`, `NoteRevealInExplorer`, `save_folder` and the rest do not exist).

- [ ] **Step 3: Platform helpers**

In `src/platform/files.rs`, add `MOVEFILE_COPY_ALLOWED, MOVEFILE_WRITE_THROUGH` to the `windows_sys::Win32::Storage::FileSystem` import and add:

```rust
/// Moves `from` to `to`, never over an existing file. Across volumes it copies and then deletes
/// (`MOVEFILE_COPY_ALLOWED`). `MOVEFILE_WRITE_THROUGH` returns only once the copy is on disk, so
/// the source is never deleted before the copy exists.
pub fn move_file(from: &Path, to: &Path) -> Result<()> {
    let (from_wide, to_wide) = (wide(from), wide(to));
    let flags = MOVEFILE_COPY_ALLOWED | MOVEFILE_WRITE_THROUGH;
    if unsafe { MoveFileExW(from_wide.as_ptr(), to_wide.as_ptr(), flags) } == 0 {
        return Err(last_error());
    }
    Ok(())
}
```

Create `src/platform/shell.rs`, keeping the test module from Step 1 at its end:

```rust
//! Showing a file in Explorer.

use crate::platform::wide_null;
use std::path::Path;
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

/// Explorer's argument that opens `path`'s folder with `path` selected. The path is quoted:
/// Explorer splits its arguments on commas.
pub fn select_argument(path: &Path) -> String {
    format!("/select,\"{}\"", path.display())
}

#[cfg(test)]
thread_local! {
    static REVEALED: std::cell::RefCell<Vec<std::path::PathBuf>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// The paths `reveal_in_explorer` was asked to show on this thread. Tests never start Explorer.
#[cfg(test)]
pub fn take_revealed() -> Vec<std::path::PathBuf> {
    REVEALED.with(|revealed| std::mem::take(&mut *revealed.borrow_mut()))
}

#[cfg(test)]
fn recorded_for_test(path: &Path) -> bool {
    REVEALED.with(|revealed| revealed.borrow_mut().push(path.to_path_buf()));
    true
}

#[cfg(not(test))]
fn recorded_for_test(_path: &Path) -> bool {
    false
}

/// Opens an Explorer window on `path`'s folder with `path` (a file or a folder) selected.
/// `SHOpenFolderAndSelectItems` would reuse an open window, but windows-sys gates it behind
/// `Win32_UI_Shell_Common`, which FastPad does not enable. `explorer.exe /select` needs only the
/// features already on.
pub fn reveal_in_explorer(path: &Path) -> crate::Result<()> {
    if recorded_for_test(path) {
        return Ok(());
    }
    let operation = wide_null("open");
    let file = wide_null("explorer.exe");
    let arguments = wide_null(&select_argument(path));
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            operation.as_ptr(),
            file.as_ptr(),
            arguments.as_ptr(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    // ShellExecuteW reports failure as a value of 32 or less (the SE_ERR_* codes).
    if result as usize <= 32 {
        return Err(crate::FastPadError::Invariant("Explorer could not be started"));
    }
    Ok(())
}
```

Add `pub mod shell;` to `src/platform/mod.rs`.

- [ ] **Step 4: The command, the palette entries and the shortcut**

In `src/window/commands.rs`, add the variant `NoteRevealInExplorer = 182,` after `ToggleNotebookFavorite = 181`. Add `CommandId::NoteRevealInExplorer,` to the end of the `COMMANDS` array and raise its length by 1. It needs a document, so `needs_document` does not change.

Task 1 dropped the "Note: Move to notebook..." palette entry and exempted `NoteMoveToNotebook` in `every_command_except_tab_positions_and_the_palette_is_listed_once`. The entry comes back below, so remove the exemption: in that test, delete the `// Moving a note's file has no palette row until it moves the file.` comment and the `&& command != CommandId::NoteMoveToNotebook` line, which leaves:

```rust
            let expected = usize::from(
                command.tab_index().is_none()
                    && command != CommandId::CommandPalette
                    && command != CommandId::MarkdownPreviewCycle,
            );
```

In `src/window/command_palette.rs` `ENTRIES`, after `"Note: Toggle pin"`, add two entries and raise the length by 2:

```rust
    entry("Note: Move to notebook...", CommandId::NoteMoveToNotebook),
    entry("Note: Reveal in Explorer", CommandId::NoteRevealInExplorer),
```

Add `MoveToNotebook` back to `PickerKind`:

```rust
pub(crate) enum PickerKind {
    RecentFolder,
    MoveToNotebook,
}
```

In `src/window/menus.rs` `accelerator_specs`, add the line below after `accelerator(FCONTROL | FSHIFT, b'O', CommandId::OpenFolder),`. The array grows by 1: raise its length and the `specs.len()` assertion in `shortcut_and_menu_commands_share_command_ids` by 1.

```rust
        accelerator(FCONTROL | FSHIFT, b'M', CommandId::NoteMoveToNotebook),
```

- [ ] **Step 5: Where a new note is saved**

In `src/document.rs`, add to `Document` after `preview`:

```rust
    /// Where this untitled note's first save goes (spec §6.7): the folder selected in the
    /// sidebar when it was created. `None` means the notebook root.
    pub save_folder: Option<PathBuf>,
```

Add `save_folder: None,` to `Document::untitled`.

In `src/window/main_window.rs`, make `create_new_document` `pub(crate) fn create_new_document`. In `execute_command`, the `CommandId::New` arm becomes:

```rust
        CommandId::New => crate::window::library_host::new_note_in(hwnd, None),
```

In `save_active_document_as`, Task 8's notes-mode arm starts the dialog in the note's folder:

```rust
        None if notes_mode && crate::window::library_host::folder(hwnd).is_some() => (
            crate::window::library_host::suggested_file_name(hwnd),
            crate::window::library_host::first_save_folder(hwnd),
        ),
```

In `src/window/library_host.rs`:

```rust
/// Ctrl+N, the Notebook view's New note, and "New note here" (`folder`). The new untitled tab
/// remembers where its first save goes: `folder`, else the folder of the sidebar's selected row,
/// else the notebook root. With no notebook open it is a plain new tab.
pub(crate) fn new_note_in(hwnd: HWND, folder: Option<PathBuf>) {
    let destination = self::folder(hwnd).filter(|_| notes_mode(hwnd)).map(|root| {
        folder
            .or_else(|| super::notebook_view::selected_folder(hwnd))
            .filter(|candidate| {
                library::model::same_path(candidate, &root) || library::is_inside(&root, candidate)
            })
            .unwrap_or(root)
    });
    if let Err(error) = super::main_window::create_new_document(hwnd) {
        push_notice(hwnd, format!("FastPad could not create a new tab: {error}"));
        return;
    }
    if let Some(mut app) = unsafe { app_ptr(hwnd) } {
        let app = unsafe { app.as_mut() };
        if let Some(id) = app.tabs.active().map(|document| document.id)
            && let Some(document) = app.tabs.document_mut(id)
        {
            document.save_folder = destination;
        }
    }
    super::side_panel::refresh(hwnd);
}

/// Where tab `id`'s first save goes: its remembered folder while that is still a folder of the
/// open notebook, else the notebook root (spec §6.7).
fn save_folder_for(hwnd: HWND, id: crate::document::DocumentId) -> Option<PathBuf> {
    let root = folder(hwnd)?;
    let remembered = unsafe { app_ptr(hwnd) }
        .and_then(|app| unsafe { app.as_ref() }.tabs.document(id)?.save_folder.clone());
    Some(
        remembered
            .filter(|folder| library::is_inside(&root, folder) && folder.is_dir())
            .unwrap_or(root),
    )
}

/// The active tab's first-save folder, for the name box and the Save As dialog.
pub(crate) fn first_save_folder(hwnd: HWND) -> Option<PathBuf> {
    let id = active_untitled(hwnd)?;
    save_folder_for(hwnd, id)
}
```

In `submit_first_save`, replace `let Some(folder) = folder(hwnd) else { return; };` with:

```rust
    let Some(folder) = save_folder_for(hwnd, id) else {
        return;
    };
```

In `save_command`, the name box's suffix names that folder. Replace `let suffix = format!("in {}", folder_display_name(hwnd));` with:

```rust
        let suffix = format!(
            "in {}",
            first_save_folder(hwnd)
                .as_deref()
                .map_or_else(|| folder_display_name(hwnd), notebook_name)
        );
```

- [ ] **Step 6: Move to notebook, reveal, rename and delete from a path**

In `src/window/library_host.rs`, add a field to `LibraryHost`, initialized to `None` in `new`:

```rust
    /// The note an open Move to notebook picker moves, and the notebooks it lists, in row order.
    /// The row after the last is "Browse…".
    pub(crate) shown_move: Option<(PathBuf, Vec<PathBuf>)>,
```

Add the functions:

```rust
/// The Move to notebook picker's notebooks: favorites by name, then recent ones, never the open
/// notebook and never twice.
fn move_destinations(hwnd: HWND) -> Vec<PathBuf> {
    let open = folder(hwnd);
    let known = known_folders(hwnd, true);
    let elsewhere = |candidate: &PathBuf| {
        !open
            .as_ref()
            .is_some_and(|open| library::model::same_path(open, candidate))
    };
    let favorites: Vec<PathBuf> = known.favorites.iter().filter(|f| elsewhere(f)).cloned().collect();
    let mut named: Vec<(String, PathBuf)> = library::local::display_names(&favorites)
        .into_iter()
        .map(|(name, _)| name)
        .zip(favorites)
        .collect();
    named.sort_by(|a, b| library::tree::natural_cmp(&a.0, &b.0));
    let mut destinations: Vec<PathBuf> = named.into_iter().map(|(_, path)| path).collect();
    for recent in known.folders {
        if elsewhere(&recent)
            && !destinations
                .iter()
                .any(|listed| library::model::same_path(listed, &recent))
        {
            destinations.push(recent);
        }
    }
    destinations
}

/// Note: Move to notebook… (spec §6.6): picks another notebook, whose root receives the file.
pub(crate) fn move_to_notebook(hwnd: HWND, path: &Path) {
    if !folder(hwnd).is_some_and(|root| library::is_inside(&root, path)) {
        push_notice(
            hwnd,
            "Only notes in the open notebook can be moved to another notebook.".to_owned(),
        );
        return;
    }
    let destinations = move_destinations(hwnd);
    let mut items: Vec<String> = library::local::display_names(&destinations)
        .into_iter()
        .map(|(name, hint)| match hint {
            Some(hint) => format!("{name} ({hint})"),
            None => name,
        })
        .collect();
    items.push("Browse…".to_owned());
    host(hwnd, |host| {
        host.shown_move = Some((path.to_path_buf(), destinations));
    });
    super::main_window::open_picker(
        hwnd,
        Picker {
            kind: PickerKind::MoveToNotebook,
            items,
            create: None,
        },
    );
}

/// Moves `note` into `destination`'s root. A clash or a failure changes nothing and says so. The
/// pin goes, because pins belong to a notebook, and an open tab follows the file.
fn move_note_to(hwnd: HWND, note: &Path, destination: &Path) {
    let Some(file_name) = note.file_name() else {
        return;
    };
    let target = destination.join(file_name);
    let notebook = notebook_name(destination);
    if library::model::same_path(&target, note) {
        push_notice(
            hwnd,
            format!("{} is already in {notebook}.", title::note_title(note)),
        );
        return;
    }
    if target.exists() {
        push_notice(
            hwnd,
            format!(
                "{} already exists in {notebook}. Nothing was moved.",
                file_name.to_string_lossy()
            ),
        );
        return;
    }
    if let Err(error) = crate::platform::files::move_file(note, &target) {
        push_notice(
            hwnd,
            format!("FastPad could not move {} to {notebook}: {error}", note.display()),
        );
        return;
    }
    let stays_inside = folder(hwnd).is_some_and(|root| library::is_inside(&root, &target));
    if stays_inside {
        with_state(hwnd, |state| state.rename_note(note, &target));
        schedule_write(hwnd);
    } else {
        let record = with_state(hwnd, |state| state.record_for(note).map(|r| r.id)).flatten();
        if let Some(id) = record {
            report(hwnd, apply_op(hwnd, |_, _| Some(PendingOp::Drop { id })));
        }
        with_state(hwnd, |state| state.remove_note(note));
    }
    rebind_open_tab(hwnd, note, target.clone());
    push_notice(
        hwnd,
        format!("Moved {} to {notebook}.", title::note_title(&target)),
    );
    super::side_panel::refresh(hwnd);
}

/// Note: Reveal in Explorer, and the sidebar's Reveal entries.
pub(crate) fn reveal(hwnd: HWND, path: &Path) {
    if let Err(error) = crate::platform::shell::reveal_in_explorer(path) {
        push_notice(
            hwnd,
            format!("FastPad could not show {} in Explorer: {error}", path.display()),
        );
    }
}

/// Rename… from the sidebar. The note opens as a normal tab first, because the name box renames
/// a tab, then the name box opens as for Note: Rename.
pub(crate) fn rename_file(hwnd: HWND, path: &Path) {
    if let Err(error) =
        super::main_window::open_note(hwnd, path, super::main_window::OpenMode::Permanent, false)
    {
        super::main_window::report_open_failure(hwnd, path, &error);
        return;
    }
    rename_note(hwnd);
}
```

Replace `delete_note` with the path-based pair. The body is today's, working from `path` and the tab found for it rather than the active tab:

```rust
/// Note: Delete, on the active tab's file.
pub(crate) fn delete_note(hwnd: HWND) {
    let Some(path) = active_file(hwnd) else {
        return;
    };
    delete_file(hwnd, &path);
}

/// After a confirm, sends `path` to the Recycle Bin and closes its tab if it has one. Any record
/// stays, flagged deleted and marked missing, so the 30-day purge removes it.
pub(crate) fn delete_file(hwnd: HWND, path: &Path) {
    let tab = unsafe { app_ptr(hwnd) }.and_then(|app| {
        let tabs = &unsafe { app.as_ref() }.tabs;
        let id = tabs.find_stored_path(path)?;
        Some((id, tabs.document(id)?.dirty))
    });
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    // The tab's unsaved edits go with the file: they are not autosaved first.
    let question = if tab.is_some_and(|(_, dirty)| dirty) {
        format!("Move \u{201c}{name}\u{201d} to the Recycle Bin and discard unsaved changes?")
    } else {
        format!("Move \u{201c}{name}\u{201d} to the Recycle Bin?")
    };
    if !confirmed(hwnd, &question) {
        return;
    }
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    // The shell may show its own modal warning (a permanent delete), owned by this window.
    let recycled = crate::platform::files::recycle(hwnd, path);
    if !identity.is_live_for(hwnd) {
        return;
    }
    if let Err(error) = recycled {
        push_notice(
            hwnd,
            format!("FastPad could not delete {}: {error}", path.display()),
        );
        return;
    }
    let now = library::now_unix();
    let record = with_state(hwnd, |state| state.record_for(path).map(|r| r.id)).flatten();
    if let Some(note_id) = record {
        report(
            hwnd,
            apply_op(hwnd, |state, ids| {
                Some(PendingOp::SetDeleted {
                    note: state.note_ref(ids, path),
                    value: true,
                })
            }),
        );
        with_state(hwnd, |state| state.local.set_missing(note_id, now));
    }
    with_state(hwnd, |state| state.remove_note(path));
    if let Some((id, _)) = tab {
        super::main_window::close_document_without_prompt(hwnd, id);
    }
    super::side_panel::refresh(hwnd);
}
```

`picked` gains the move arm. The whole function becomes:

```rust
/// A picker row was chosen. Every kind resolves the row against what that picker showed.
pub(crate) fn picked(hwnd: HWND, kind: PickerKind, choice: PickerChoice) {
    #[cfg(test)]
    LAST_PICK.with(|last| *last.borrow_mut() = Some((kind, choice.clone())));
    match (kind, choice) {
        (PickerKind::RecentFolder, PickerChoice::Item(index)) => {
            let shown = host(hwnd, |host| std::mem::take(&mut host.shown_recent_folders));
            if let Some(folder) = shown.unwrap_or_default().get(index) {
                open_listed_notebook(hwnd, folder);
            }
        }
        (PickerKind::MoveToNotebook, PickerChoice::Item(index)) => {
            let Some((note, destinations)) = host(hwnd, |host| host.shown_move.take()).flatten()
            else {
                return;
            };
            let destination = match destinations.get(index) {
                Some(folder) => folder.clone(),
                // The row after the notebooks is "Browse…".
                None if index == destinations.len() => {
                    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
                        return;
                    };
                    let choice = crate::window::modal::choose_folder(hwnd);
                    if !identity.is_live_for(hwnd) {
                        return;
                    }
                    match choice {
                        Ok(Some(folder)) => library::normalize_folder(&folder),
                        Ok(None) => return,
                        Err(error) => {
                            push_notice(
                                hwnd,
                                format!("FastPad could not open the folder picker: {error}"),
                            );
                            return;
                        }
                    }
                }
                None => return,
            };
            move_note_to(hwnd, &note, &destination);
        }
        _ => {}
    }
}
```

- [ ] **Step 7: Commands act on the selected row while the tree has focus**

In `src/window/main_window.rs` `execute_command`, replace the `NoteTogglePin`, `NoteMoveToNotebook`, `NoteRename` and `NoteDelete` arms (as Task 1 left them) with the arms below, and add `NoteRevealInExplorer`:

```rust
        CommandId::NoteTogglePin => {
            if let Some(path) = crate::window::notebook_view::focused_note(hwnd)
                .or_else(|| crate::window::library_host::active_file(hwnd))
            {
                crate::window::library_host::toggle_pin(hwnd, &path);
            }
        }
        CommandId::NoteMoveToNotebook => {
            if let Some(path) = crate::window::notebook_view::focused_note(hwnd)
                .or_else(|| crate::window::library_host::active_file(hwnd))
            {
                crate::window::library_host::move_to_notebook(hwnd, &path);
            }
        }
        CommandId::NoteRevealInExplorer => {
            if let Some(path) = crate::window::notebook_view::focused_note(hwnd)
                .or_else(|| crate::window::library_host::active_file(hwnd))
            {
                crate::window::library_host::reveal(hwnd, &path);
            }
        }
        CommandId::NoteRename => {
            if crate::window::library_host::ready_library(hwnd) {
                match crate::window::notebook_view::focused_note(hwnd) {
                    Some(path) => crate::window::library_host::rename_file(hwnd, &path),
                    None => crate::window::library_host::rename_note(hwnd),
                }
            }
        }
        CommandId::NoteDelete => {
            if crate::window::library_host::ready_library(hwnd) {
                match crate::window::notebook_view::focused_note(hwnd) {
                    Some(path) => crate::window::library_host::delete_file(hwnd, &path),
                    None => crate::window::library_host::delete_note(hwnd),
                }
            }
        }
```

- [ ] **Step 8: Context menus, F2 and Del in the tree, and Reveal in the header menu**

In `src/window/notebook_view.rs`, add `VK_DELETE, VK_F2` to the `KeyboardAndMouse` import and `WM_CONTEXTMENU` to the `WindowsAndMessaging` import. Then add:

```rust
/// The selected note's absolute path while the panel has the keyboard focus, so palette and
/// accelerator commands act on it rather than on the active tab (spec §6.3).
pub(crate) fn focused_note(hwnd: HWND) -> Option<PathBuf> {
    let root = super::library_host::folder(hwnd)?;
    with_view(hwnd, |view| {
        let focused =
            unsafe { windows_sys::Win32::UI::Input::KeyboardAndMouse::GetFocus() } == view.panel;
        match view.list.selected.map(|index| view.target(index)) {
            Some(Target::Row(TreeRow {
                kind: RowKind::Note(relative),
                ..
            })) if focused => Some(root.join(relative)),
            _ => None,
        }
    })
    .flatten()
}

/// The folder a new note goes to (spec §6.7): a selected folder row's own folder, or a
/// selected note's parent. `None` (the root) for an unsaved row or no selection.
pub(crate) fn selected_folder(hwnd: HWND) -> Option<PathBuf> {
    let root = super::library_host::folder(hwnd)?;
    let target = with_view(hwnd, |view| view.list.selected.map(|index| view.target(index)))
        .flatten()?;
    match target {
        Target::Row(TreeRow {
            kind: RowKind::Folder(relative),
            ..
        }) => Some(root.join(relative)),
        Target::Row(TreeRow {
            kind: RowKind::Note(relative),
            ..
        }) => Some(root.join(relative).parent()?.to_path_buf()),
        _ => None,
    }
}

impl NotebookView {
    /// Under row `index`, in main-window client coordinates, for a menu opened from the
    /// keyboard.
    fn row_menu_point(&self, index: usize) -> POINT {
        let list = self.list_rect(self.client());
        let rect = self.row_rect(list, index).unwrap_or(list);
        self.to_main(POINT {
            x: rect.left + scale(24, self.dpi()),
            y: rect.bottom,
        })
    }
}

/// Row `index`'s context menu (spec §6.6), at `at` (main-window client coordinates) or under
/// the row when opened from the keyboard. The chosen entry acts on that row, not the active
/// tab. "Open in new tab" is `CommandId::Open` and "New note here" is `CommandId::New` here.
pub(crate) fn open_context_menu(hwnd: HWND, index: usize, at: Option<POINT>) {
    let Some((target, point)) = with_view(hwnd, |view| {
        if view.mode != Mode::Tree {
            return None;
        }
        view.select(index);
        Some((view.target(index), at.unwrap_or_else(|| view.row_menu_point(index))))
    })
    .flatten() else {
        return;
    };
    let Target::Row(row) = target else {
        return;
    };
    let Some(root) = super::library_host::folder(hwnd) else {
        return;
    };
    match &row.kind {
        RowKind::Note(relative) => {
            let path = root.join(relative);
            let entries = [
                MenuEntry::command("Open in new tab", CommandId::Open),
                MenuEntry::command(
                    if row.pinned { "Unpin" } else { "Pin" },
                    CommandId::NoteTogglePin,
                ),
                MenuEntry::Separator,
                MenuEntry::command(
                    "Move to notebook...\tCtrl+Shift+M",
                    CommandId::NoteMoveToNotebook,
                ),
                MenuEntry::command("Rename...\tF2", CommandId::NoteRename),
                MenuEntry::command("Reveal in Explorer", CommandId::NoteRevealInExplorer),
                MenuEntry::Separator,
                MenuEntry::command("Delete...\tDel", CommandId::NoteDelete),
            ];
            match super::menus::track_popup(hwnd, &entries, point) {
                Some(CommandId::Open) => {
                    if let Err(error) =
                        super::main_window::open_note(hwnd, &path, OpenMode::Permanent, true)
                    {
                        super::main_window::report_open_failure(hwnd, &path, &error);
                    }
                }
                Some(CommandId::NoteTogglePin) => super::library_host::toggle_pin(hwnd, &path),
                Some(CommandId::NoteMoveToNotebook) => {
                    super::library_host::move_to_notebook(hwnd, &path);
                }
                Some(CommandId::NoteRename) => {
                    if super::library_host::ready_library(hwnd) {
                        super::library_host::rename_file(hwnd, &path);
                    }
                }
                Some(CommandId::NoteRevealInExplorer) => super::library_host::reveal(hwnd, &path),
                Some(CommandId::NoteDelete) => {
                    if super::library_host::ready_library(hwnd) {
                        super::library_host::delete_file(hwnd, &path);
                    }
                }
                _ => {}
            }
        }
        RowKind::Folder(relative) => {
            let path = root.join(relative);
            let entries = [
                MenuEntry::command("New note here", CommandId::New),
                MenuEntry::command("Reveal in Explorer", CommandId::NoteRevealInExplorer),
            ];
            match super::menus::track_popup(hwnd, &entries, point) {
                Some(CommandId::New) => super::library_host::new_note_in(hwnd, Some(path)),
                Some(CommandId::NoteRevealInExplorer) => super::library_host::reveal(hwnd, &path),
                _ => {}
            }
        }
        RowKind::Unsaved(key) => {
            let entries = [MenuEntry::command("Close tab", CommandId::CloseTab)];
            if super::menus::track_popup(hwnd, &entries, point) == Some(CommandId::CloseTab)
                && super::main_window::activate_document_by_id(hwnd, DocumentId(*key))
            {
                run(hwnd, CommandId::CloseTab);
            }
        }
    }
}

/// `WM_CONTEXTMENU`: from a right-click (screen coordinates) or from Shift+F10 or the
/// context-menu key (`lparam` of -1, for the selected row).
fn context_menu(hwnd: HWND, lparam: LPARAM) {
    let keyboard = lparam as u32 == u32::MAX;
    let target = with_view(hwnd, |view| {
        if keyboard {
            return view.list.selected.map(|index| (index, None));
        }
        let (x, y) = point_of(lparam);
        let mut client = POINT { x, y };
        unsafe {
            ScreenToClient(view.panel, &mut client);
        }
        match view.hit_test(client.x, client.y) {
            Hit::Row { index, .. } => Some((index, Some(view.to_main(client)))),
            _ => None,
        }
    })
    .flatten();
    if let Some((index, at)) = target {
        open_context_menu(hwnd, index, at);
    }
}
```

In `handle`, add the arm:

```rust
        WM_CONTEXTMENU => {
            context_menu(hwnd, lparam);
            Some(0)
        }
```

In `key_down`, the early return for no selection becomes `return matches!(key, VK_RETURN | VK_LEFT | VK_RIGHT | VK_F2 | VK_DELETE);`. Add this arm to the final `match key` before `_ => false`:

```rust
        VK_F2 | VK_DELETE => {
            let note = with_view(hwnd, |view| match view.target(selected) {
                Target::Row(TreeRow {
                    kind: RowKind::Note(relative),
                    ..
                }) => Some(relative),
                _ => None,
            })
            .flatten();
            if let Some(relative) = note
                && let Some(root) = super::library_host::folder(hwnd)
                && super::library_host::ready_library(hwnd)
            {
                let path = root.join(relative);
                if key == VK_F2 {
                    super::library_host::rename_file(hwnd, &path);
                } else {
                    super::library_host::delete_file(hwnd, &path);
                }
            }
            true
        }
```

Replace Task 10's `more_menu` with a version that also reveals the notebook:

```rust
/// "…": the notebook's own actions.
fn more_menu(hwnd: HWND) {
    let Some(at) = with_view(hwnd, |view| {
        let rect = header_layout(view.client(), view.dpi()).buttons[2].1;
        view.to_main(POINT {
            x: rect.left,
            y: rect.bottom,
        })
    }) else {
        return;
    };
    let entries = [
        MenuEntry::command("Reveal in Explorer", CommandId::NoteRevealInExplorer),
        MenuEntry::command("Close notebook", CommandId::CloseNotebook),
    ];
    match super::menus::track_popup(hwnd, &entries, at) {
        Some(CommandId::NoteRevealInExplorer) => {
            if let Some(root) = super::library_host::folder(hwnd) {
                super::library_host::reveal(hwnd, &root);
            }
        }
        Some(command) => run(hwnd, command),
        None => {}
    }
}
```

- [ ] **Step 9: Run the tests to verify they pass**

Run: `cargo test --lib -- moving_a_file_never explorer_is_asked reveal_in_explorer_has moving_a_note_to_another a_move_onto_an_existing move_offers_favorites a_new_note_saves_into the_context_menu_acts f2_on_a_note_row --test-threads=1`
Expected: all nine PASS.

Run: `cargo test --lib -- every_command_except shortcuts_are_spelled accelerator the_first_save_of_an_untitled_note a_name_that_already_exists browse_and_the_close_prompt --test-threads=1`
Expected: PASS. The palette lists every command once, and the first save still defaults to the root.

Run: `cargo test --lib window::notebook_view` and `cargo test --lib platform::`
Expected: PASS.

Run: `cargo clippy --all-targets -- -D warnings` and `cargo fmt --all -- --check`
Expected: no warnings, no diff.

- [ ] **Step 10: Commit**

```bash
git add src/platform/shell.rs src/platform/mod.rs src/platform/files.rs src/document.rs src/window/library_host.rs src/window/notebook_view.rs src/window/command_palette.rs src/window/commands.rs src/window/menus.rs src/window/main_window.rs
git commit -m "feat(sidebar): row context menus, move to notebook, reveal in Explorer, new notes in the selected folder"
```

---

### Task 12: Favorites view, Search view and the Settings button

**Files:**
- Create: `src/window/favorites_view.rs`, `src/window/search_view.rs`
- Modify:
  - `src/window/mod.rs`: `pub(crate) mod favorites_view;` and `pub(crate) mod search_view;`
  - `src/window/side_panel.rs`: `Sidebar.{favorites, search}` and the Search and Favorites arms of Task 6's dispatch
  - `src/window/library_host.rs`: `open_listed_notebook_in_view`
  - `src/window/command_palette.rs`: `SETTINGS_COMMANDS` and the command subset
  - `src/window/main_window.rs`: `open_settings_palette`, and tests
  - `src/window/activity_bar.rs`: the Settings click handler

**Interfaces:**
- Consumes:
  - Task 2: `local::display_names`.
  - Task 3: `tree::natural_cmp`.
  - Task 4: `name_search::{NameMatch, search}`.
  - Task 5: `SidebarView`.
  - Task 6: `side_panel::{show_view, current_view, refresh, layout}`, `App.sidebar` with `Sidebar.{bar, panel}`, and the view seam: `ViewPaint { hdc, client, palette, background, fonts, dpi, focused }`, `draw_text(hdc, text, rect, font, color, flags) -> i32`, `main_window::ui_fonts(hwnd) -> UiFonts`, the `PanelView` dispatch (`paint_view`, `view_mouse`, `view_key`, `header_is_caption`) and `activity_bar::open_settings`.
  - Task 7: `RowListState`, `ListKey`, `RowLook`, `row_list::paint`. `row_at(y)` and `row_top(index)` are relative to the top of the list area.
  - Task 8: `library_host::{favorites, remove_favorite, folder, choose_and_open_folder}` and the private `check_listed_notebook(hwnd, folder, show_notebook)`.
  - Task 9: `main_window::{open_note, OpenMode}`.
  - Task 10: `Sidebar.notebook`, `notebook_view::{paint, handle, header_hit}` and `menus::{track_popup, MenuEntry}`, `pub(crate)` since Task 10.
  - Task 11: `library_host::reveal`.
- Produces:
  - `favorites_view::FavoritesView`, with `FavoriteRow`, `favorite_rows`, `FavoriteAction` and `KeyResult`, and the module functions `refresh`, `paint`, `handle`, `run`, `header_controls`. Its row menu reads `menus::track_popup`'s command locally, as the Notebook view's menus do: `OpenFolder` opens that notebook, `ToggleNotebookFavorite` removes it, `NoteRevealInExplorer` reveals it.
  - `search_view::SearchView`, which owns a native `Edit` child. It comes with `placeholder`, `status_text`, `RESULT_LIMIT`, and the module functions `query_changed`, `library_changed`, `layout`, `shown`, `hidden`, `paint`, `handle`, `control_color`, `open_selected`.
  - `library_host::open_listed_notebook_in_view(hwnd, folder: &Path, focus: bool)`.
  - `command_palette::SETTINGS_COMMANDS: &[CommandId]` and `main_window::open_settings_palette(hwnd)`.
  - `Sidebar.favorites: FavoritesView` and `Sidebar.search: SearchView`.

- [ ] **Step 1: Write the failing pure tests**

In `src/window/command_palette.rs`, in `mod tests`, add `SETTINGS_COMMANDS` to the `use super::{...}` list and add:

```rust
    #[test]
    fn every_settings_command_has_exactly_one_palette_entry() {
        // Break caught: a Settings button entry with no palette row, which the filtered palette
        // could never show, or a settings list that lets non-settings commands through.
        for command in SETTINGS_COMMANDS {
            let listed = ENTRIES
                .iter()
                .filter(|entry| entry.command == *command)
                .count();
            assert_eq!(listed, 1, "{command:?}");
        }
        let listed = filter_entries("", |command| SETTINGS_COMMANDS.contains(&command));
        assert_eq!(listed.len(), SETTINGS_COMMANDS.len());
        assert!(listed.iter().all(|entry| entry.command != CommandId::Save));
        assert!(
            listed
                .iter()
                .any(|entry| entry.command == CommandId::ThemeCatppuccinMocha)
        );
    }
```

At the end of the new `src/window/favorites_view.rs` (Step 5 writes the rest of the file):

```rust
#[cfg(test)]
mod tests {
    use super::{FavoriteAction, FavoritesView, KeyResult, favorite_rows};
    use std::path::{Path, PathBuf};
    use windows_sys::Win32::Foundation::{POINT, RECT};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_DELETE, VK_DOWN, VK_RETURN};

    const CLIENT: RECT = RECT {
        left: 0,
        top: 0,
        right: 260,
        bottom: 400,
    };

    fn view(favorites: &[&str]) -> FavoritesView {
        let favorites = favorites.iter().map(PathBuf::from).collect::<Vec<_>>();
        let mut view = FavoritesView::new(96);
        view.set_rows(favorite_rows(&favorites, None), 96, 400);
        view
    }

    #[test]
    fn favorites_are_sorted_by_name_with_the_open_one_marked_and_clashes_hinted() {
        // Break caught: favorites listed in folders.ini order, the open notebook not marked
        // (paths differ only in case), or two "Notes" folders shown identically.
        let favorites = [
            r"C:\b\Notes",
            r"C:\Work 10",
            r"C:\a\Notes",
            r"C:\Work 2",
        ]
        .map(PathBuf::from);
        let rows = favorite_rows(&favorites, Some(Path::new(r"c:\WORK 2")));
        let names = rows.iter().map(|row| row.name.as_str()).collect::<Vec<_>>();
        assert_eq!(names, ["Notes", "Notes", "Work 2", "Work 10"]);
        assert_eq!(rows[0].folder, PathBuf::from(r"C:\a\Notes"));
        assert!(rows[0].hint.is_some() && rows[1].hint.is_some());
        assert_ne!(rows[0].hint, rows[1].hint);
        assert_eq!(rows[2].hint, None);
        assert!(rows[2].open);
        assert!(!rows[3].open && !rows[0].open);
    }

    #[test]
    fn a_click_on_the_star_removes_elsewhere_opens_and_the_footer_browses() {
        // Break caught: the star opening the notebook instead of removing it, or the footer row
        // and header button doing nothing.
        let mut view = view(&[r"C:\a", r"C:\b"]);
        let first = PathBuf::from(r"C:\a");
        // The header is 38 px and each row 26 px at 96 DPI.
        assert_eq!(
            view.click(POINT { x: 100, y: 50 }, CLIENT, 96),
            Some(FavoriteAction::Open(first.clone()))
        );
        assert_eq!(
            view.click(POINT { x: 250, y: 50 }, CLIENT, 96),
            Some(FavoriteAction::Remove(first))
        );
        assert_eq!(
            view.click(POINT { x: 100, y: 38 + 2 * 26 + 5 }, CLIENT, 96),
            Some(FavoriteAction::Browse)
        );
        assert_eq!(
            view.click(POINT { x: 240, y: 19 }, CLIENT, 96),
            Some(FavoriteAction::Browse)
        );
        assert_eq!(view.click(POINT { x: 100, y: 390 }, CLIENT, 96), None);
    }

    #[test]
    fn keys_move_the_selection_and_enter_opens_the_selected_row_or_the_footer() {
        // Break caught: Enter doing nothing in the Favorites view, Delete removing an unselected
        // favorite, or the footer row being unreachable from the keyboard.
        let mut view = view(&[r"C:\a", r"C:\b"]);
        view.list.select(0, 400);
        assert_eq!(view.key(VK_DOWN, CLIENT, 96), KeyResult::Handled);
        assert_eq!(
            view.key(VK_RETURN, CLIENT, 96),
            KeyResult::Run(FavoriteAction::Open(PathBuf::from(r"C:\b")))
        );
        assert_eq!(
            view.key(VK_DELETE, CLIENT, 96),
            KeyResult::Run(FavoriteAction::Remove(PathBuf::from(r"C:\b")))
        );
        assert_eq!(view.key(VK_DOWN, CLIENT, 96), KeyResult::Handled);
        assert_eq!(
            view.key(VK_RETURN, CLIENT, 96),
            KeyResult::Run(FavoriteAction::Browse)
        );
        assert_eq!(view.key(VK_DELETE, CLIENT, 96), KeyResult::Handled);
    }

    #[test]
    fn a_removed_favorite_moves_the_selection_to_its_neighbor() {
        // Break caught: a stale selection index past the end after the last favorite was removed.
        let mut view = view(&[r"C:\a", r"C:\b"]);
        view.list.select(1, 400);
        view.set_rows(favorite_rows(&[PathBuf::from(r"C:\a")], None), 96, 400);
        assert_eq!(view.list.selected, Some(1), "the footer row takes its place");
        view.set_rows(Vec::new(), 96, 400);
        assert_eq!(view.list.count, 1);
        assert_eq!(view.list.selected, Some(0));
    }
}
```

At the end of the new `src/window/search_view.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::{LOADING, NO_MATCH, NO_NOTEBOOK, placeholder, status_text};
    use std::path::Path;

    #[test]
    fn the_status_line_explains_an_empty_list() {
        // Break caught: a blank Search view with no notebook open, "No notes match." shown
        // before anything is typed, or a match count of zero reported while still loading.
        assert_eq!(status_text(false, false, "x", 0), Some(NO_NOTEBOOK));
        assert_eq!(status_text(true, true, "", 0), None);
        assert_eq!(status_text(true, true, "   ", 0), None);
        assert_eq!(status_text(true, false, "x", 0), Some(LOADING));
        assert_eq!(status_text(true, true, "x", 0), Some(NO_MATCH));
        assert_eq!(status_text(true, true, "x", 3), None);
    }

    #[test]
    fn the_placeholder_names_the_open_notebook() {
        // Break caught: the box saying "Search" with no hint of which notebook it searches.
        assert_eq!(placeholder(Some(Path::new(r"C:\Users\me\Work"))), "Search Work");
        assert_eq!(placeholder(None), "Search");
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib -- every_settings_command favorites_are_sorted a_click_on_the_star keys_move_the_selection a_removed_favorite the_status_line the_placeholder --test-threads=1`
Expected: a compile error, because `SETTINGS_COMMANDS`, `favorites_view` and `search_view` don't exist yet.

- [ ] **Step 3: Add the Settings command list and the palette subset**

In `src/window/command_palette.rs`, after `ENTRIES`, add:

```rust
/// What the activity bar's Settings button lists: every command that changes a `fastpad.ini`
/// setting or the open notebook's autosave switch. The palette shows them in catalog order.
pub(crate) const SETTINGS_COMMANDS: &[CommandId] = &[
    CommandId::ToggleRestoreSession,
    CommandId::ToggleNotesMode,
    CommandId::ToggleFolderAutosave,
    CommandId::ToggleWordWrap,
    CommandId::ToggleLineNumbers,
    CommandId::FontSizeIncrease,
    CommandId::FontSizeDecrease,
    CommandId::FontSizeReset,
    CommandId::ThemeSystem,
    CommandId::ThemeLight,
    CommandId::ThemeDark,
    CommandId::ThemeCatppuccin,
    CommandId::ThemeCatppuccinLatte,
    CommandId::ThemeCatppuccinFrappe,
    CommandId::ThemeCatppuccinMacchiato,
    CommandId::ThemeCatppuccinMocha,
    CommandId::TabWidth2,
    CommandId::TabWidth4,
    CommandId::TabWidth8,
];
```

In `struct CommandPalette`, add a field after `picker_rows`:

```rust
    /// `Some` while command mode lists only these commands (the Settings button).
    subset: Option<&'static [CommandId]>,
```

Initialize it in `CommandPalette::create`: add `subset: None,` after `picker_rows: Vec::new(),`.

Replace `mark_hidden` with:

```rust
    /// Returns whether it was visible; `hide_controls` then removes it from the screen.
    pub(crate) fn mark_hidden(&mut self) -> bool {
        self.picker = None;
        self.picker_rows = Vec::new();
        self.subset = None;
        std::mem::take(&mut self.visible)
    }
```

After `picker()`, add:

```rust
    /// Limits command mode to `subset`, or lifts the limit with `None`.
    pub(crate) fn set_subset(&mut self, subset: Option<&'static [CommandId]>) {
        self.subset = subset;
    }

    pub(crate) fn subset(&self) -> Option<&'static [CommandId]> {
        self.subset
    }
```

In `src/window/main_window.rs`, replace `fn open_command_palette(hwnd: HWND)` (the whole function) with:

```rust
pub(crate) fn open_command_palette(hwnd: HWND) {
    show_command_palette(hwnd, None);
}

/// The activity bar's Settings button: the palette listing only `SETTINGS_COMMANDS`.
pub(crate) fn open_settings_palette(hwnd: HWND) {
    show_command_palette(hwnd, Some(command_palette::SETTINGS_COMMANDS));
}

fn show_command_palette(hwnd: HWND, subset: Option<&'static [CommandId]>) {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    let colors = title_chrome(hwnd).0;
    let newly_shown = unsafe { app_ptr(hwnd) }.and_then(|mut app| {
        let app = unsafe { app.as_mut() };
        if app.command_palette.is_none() {
            app.command_palette = CommandPalette::create(hwnd).ok();
        }
        let palette = app.command_palette.as_mut()?;
        let newly_shown = palette.mark_shown(colors);
        // Reopening the palette normally always shows commands, even right after a picker.
        palette.set_picker(None);
        palette.set_subset(subset);
        Some(newly_shown)
    });
    let Some(newly_shown) = newly_shown else {
        return;
    };
    // A query typed for the full list would hide most settings, so Settings always starts empty.
    if newly_shown || subset.is_some() {
        // Clearing the field sends EN_CHANGE, which lists every available command.
        with_command_palette(hwnd, CommandPalette::clear_query);
    }
    if !identity.is_live_for(hwnd) {
        return;
    }
    refilter_command_palette(hwnd);
    with_command_palette(hwnd, CommandPalette::focus_query);
}
```

In `refilter_command_palette`, replace the `else` branch's `let entries = ...;` statement (as Task 6 left it) with:

```rust
        let subset = with_command_palette(hwnd, CommandPalette::subset).flatten();
        let entries = command_palette::filter_entries(&query, |command| {
            subset.is_none_or(|subset| subset.contains(&command))
                && (has_tabs || !command.needs_document())
                && (markdown || !command.is_markdown_preview())
                && (sidebar || !command.is_sidebar())
        });
```

- [ ] **Step 4: Open a favorite and show the Notebook view once it opens**

A favorite may be on an offline drive, so Task 8 checks it on a worker. The Notebook view must show only once the notebook has opened, not when a missing one left everything as it was. In `src/window/library_host.rs`, next to `open_listed_notebook`, add:

```rust
/// Opens a listed notebook like `open_listed_notebook`, then shows the Notebook view once it is
/// open, with the focus in it for `focus`. A missing notebook changes nothing, the view included
/// (spec §7).
pub(crate) fn open_listed_notebook_in_view(hwnd: HWND, folder: &Path, focus: bool) {
    check_listed_notebook(hwnd, folder, Some(focus));
}
```

The views paint through Task 6's `ViewPaint` and `draw_text`, with the sidebar's fonts in `ViewPaint.fonts` (`text`, `bold` for header titles, `glyph`).

- [ ] **Step 5: Write `src/window/favorites_view.rs`**

```rust
//! The Favorites view (sidebar spec §7). It lists favorite notebooks by name, highlights the open
//! one, shows a filled star that removes a favorite, and ends with an "Open notebook…" footer
//! row. Clicking a notebook opens it and shows the Notebook view.

use crate::library::model::same_path;
use crate::library::tree::natural_cmp;
use crate::library::{local, normalize_folder};
use crate::window::commands::CommandId;
use crate::window::library_host;
use crate::window::menus::{self, MenuEntry};
use crate::window::panel::{fill, scale};
use crate::window::row_list::{self, ListKey, RowListState, RowLook};
use crate::window::side_panel::{ViewPaint, draw_text};
use std::path::{Path, PathBuf};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    ClientToScreen, DT_CENTER, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER,
    HDC, InvalidateRect, ScreenToClient,
};
use windows_sys::Win32::UI::Controls::WM_MOUSELEAVE;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetFocus, SetFocus, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent, VK_DELETE, VK_DOWN, VK_END,
    VK_HOME, VK_NEXT, VK_PRIOR, VK_RETURN, VK_SPACE, VK_UP,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetClientRect, WM_CONTEXTMENU, WM_KEYDOWN, WM_LBUTTONDOWN, WM_MOUSEMOVE, WM_MOUSEWHEEL,
};

const HEADER_AT_96_DPI: i32 = 38;
const ROW_AT_96_DPI: i32 = 26;
const PADDING_AT_96_DPI: i32 = 12;
const GLYPH_AT_96_DPI: i32 = 20;
const GAP_AT_96_DPI: i32 = 6;
const HEADER_BUTTON_AT_96_DPI: i32 = 28;
const WHEEL_DELTA: i32 = 120;
const WHEEL_LINES: i32 = 3;

const FOLDER_GLYPH: &str = "\u{E8B7}";
const FILLED_STAR_GLYPH: &str = "\u{E735}";
const OPEN_GLYPH: &str = "\u{E838}";

pub(crate) const HEADER_TEXT: &str = "FAVORITES";
pub(crate) const EMPTY_TEXT: &str = "Star a notebook to keep it here.";
pub(crate) const OPEN_NOTEBOOK: &str = "Open notebook\u{2026}";

/// One favorite notebook as the view lists it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FavoriteRow {
    pub folder: PathBuf,
    pub name: String,
    /// The dim parent-folder hint, shown only when two favorites share a name.
    pub hint: Option<String>,
    /// This is the open notebook.
    pub open: bool,
}

/// The favorites sorted by name, then by hint, with the open notebook marked.
pub(crate) fn favorite_rows(favorites: &[PathBuf], open: Option<&Path>) -> Vec<FavoriteRow> {
    let open = open.map(normalize_folder);
    let mut rows = favorites
        .iter()
        .zip(local::display_names(favorites))
        .map(|(folder, (name, hint))| FavoriteRow {
            open: open
                .as_ref()
                .is_some_and(|open| same_path(open, &normalize_folder(folder))),
            folder: folder.clone(),
            name,
            hint,
        })
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| {
        natural_cmp(&left.name, &right.name).then_with(|| {
            natural_cmp(
                left.hint.as_deref().unwrap_or(""),
                right.hint.as_deref().unwrap_or(""),
            )
        })
    });
    rows
}

/// What a click, key or menu choice asks for. The caller runs it with no view borrowed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum FavoriteAction {
    Open(PathBuf),
    Remove(PathBuf),
    Reveal(PathBuf),
    /// The header button and the footer row: the folder dialog.
    Browse,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum KeyResult {
    /// Not a key this view handles; the panel's default handling runs.
    Unhandled,
    /// Handled here (the selection moved); the caller repaints.
    Handled,
    Run(FavoriteAction),
}

fn inside(rect: RECT, point: POINT) -> bool {
    point.x >= rect.left && point.x < rect.right && point.y >= rect.top && point.y < rect.bottom
}

#[derive(Debug)]
pub(crate) struct FavoritesView {
    pub(crate) rows: Vec<FavoriteRow>,
    /// One row per favorite, then the "Open notebook…" footer.
    pub(crate) list: RowListState,
    header_hover: bool,
}

impl FavoritesView {
    pub(crate) fn new(dpi: u32) -> Self {
        let mut list = RowListState::new(scale(ROW_AT_96_DPI, dpi));
        list.set_count(1);
        Self {
            rows: Vec::new(),
            list,
            header_hover: false,
        }
    }

    /// Replaces the rows. The selection stays on the same notebook, or moves to the row that took
    /// the place of a removed one. `height` is the list area's height.
    pub(crate) fn set_rows(&mut self, rows: Vec<FavoriteRow>, dpi: u32, height: i32) {
        let previous = self.list.selected;
        let kept = previous
            .and_then(|index| self.rows.get(index))
            .map(|row| row.folder.clone());
        self.rows = rows;
        self.list.row_height = scale(ROW_AT_96_DPI, dpi);
        self.list.set_count(self.rows.len() + 1);
        let index = kept
            .and_then(|folder| self.rows.iter().position(|row| same_path(&row.folder, &folder)))
            .or_else(|| previous.map(|index| index.min(self.rows.len())));
        if let Some(index) = index {
            self.list.select(index, height);
        }
    }

    /// The header's "Open notebook…" button.
    pub(crate) fn header_button(client: RECT, dpi: u32) -> RECT {
        let size = scale(HEADER_BUTTON_AT_96_DPI, dpi);
        let header = scale(HEADER_AT_96_DPI, dpi);
        let right = client.right - scale(GAP_AT_96_DPI, dpi);
        let top = client.top + (header - size) / 2;
        RECT {
            left: right - size,
            top,
            right,
            bottom: top + size,
        }
    }

    /// Where the rows are. With no favorites, the empty-state line sits above the footer row.
    pub(crate) fn list_area(&self, client: RECT, dpi: u32) -> RECT {
        let mut top = client.top + scale(HEADER_AT_96_DPI, dpi);
        if self.rows.is_empty() {
            top += scale(ROW_AT_96_DPI, dpi);
        }
        RECT {
            top: top.min(client.bottom),
            ..client
        }
    }

    fn star_left(area: RECT, dpi: u32) -> i32 {
        area.right - scale(ROW_AT_96_DPI, dpi)
    }

    pub(crate) fn paint(&self, paint: &ViewPaint) {
        let dpi = paint.dpi;
        let palette = paint.palette;
        let client = paint.client;
        let pad = scale(PADDING_AT_96_DPI, dpi);
        let line = DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX;
        let header = RECT {
            bottom: client.top + scale(HEADER_AT_96_DPI, dpi),
            ..client
        };
        unsafe {
            fill(paint.hdc, client, paint.background);
            draw_text(
                paint.hdc,
                HEADER_TEXT,
                RECT {
                    left: header.left + pad,
                    ..header
                },
                paint.fonts.bold,
                palette.muted_foreground,
                line | DT_LEFT,
            );
            let button = Self::header_button(client, dpi);
            if self.header_hover {
                fill(paint.hdc, button, palette.hover_background);
            }
            draw_text(
                paint.hdc,
                OPEN_GLYPH,
                button,
                paint.fonts.glyph,
                palette.editor_foreground,
                line | DT_CENTER,
            );
            if self.rows.is_empty() {
                let empty = RECT {
                    left: client.left + pad,
                    top: header.bottom,
                    right: client.right - pad,
                    bottom: header.bottom + scale(ROW_AT_96_DPI, dpi),
                };
                draw_text(
                    paint.hdc,
                    EMPTY_TEXT,
                    empty,
                    paint.fonts.text,
                    palette.muted_foreground,
                    line | DT_LEFT | DT_END_ELLIPSIS,
                );
            }
        }
        let area = self.list_area(client, dpi);
        row_list::paint(
            paint.hdc,
            area,
            &self.list,
            &palette,
            paint.focused,
            &mut |hdc, index, rect, look| self.draw_row(hdc, index, rect, look, paint),
        );
    }

    /// One row's glyph and text. `row_list::paint` has already filled its selection or hover
    /// background.
    fn draw_row(&self, hdc: HDC, index: usize, rect: RECT, look: RowLook, paint: &ViewPaint) {
        let dpi = paint.dpi;
        let palette = paint.palette;
        let pad = scale(PADDING_AT_96_DPI, dpi);
        let glyph = RECT {
            left: rect.left + pad,
            right: rect.left + pad + scale(GLYPH_AT_96_DPI, dpi),
            ..rect
        };
        let text_left = glyph.right + scale(GAP_AT_96_DPI, dpi);
        let line = DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX;
        let foreground = if look.selected {
            palette
                .selection_foreground
                .unwrap_or(palette.editor_foreground)
        } else {
            palette.editor_foreground
        };
        unsafe {
            let Some(row) = self.rows.get(index) else {
                draw_text(
                    hdc,
                    OPEN_GLYPH,
                    glyph,
                    paint.fonts.glyph,
                    palette.muted_foreground,
                    line | DT_CENTER,
                );
                draw_text(
                    hdc,
                    OPEN_NOTEBOOK,
                    RECT {
                        left: text_left,
                        right: rect.right - pad,
                        ..rect
                    },
                    paint.fonts.text,
                    foreground,
                    line | DT_LEFT | DT_END_ELLIPSIS,
                );
                return;
            };
            // The open notebook gets an accent bar, so it isn't marked by color alone.
            if row.open {
                fill(
                    hdc,
                    RECT {
                        right: rect.left + scale(3, dpi),
                        ..rect
                    },
                    palette.selection_background,
                );
            }
            draw_text(
                hdc,
                FOLDER_GLYPH,
                glyph,
                paint.fonts.glyph,
                palette.muted_foreground,
                line | DT_CENTER,
            );
            let show_star = look.hover || look.selected;
            let star = RECT {
                left: Self::star_left(rect, dpi),
                ..rect
            };
            if show_star {
                draw_text(
                    hdc,
                    FILLED_STAR_GLYPH,
                    star,
                    paint.fonts.glyph,
                    foreground,
                    line | DT_CENTER,
                );
            }
            let text_right = if show_star {
                star.left
            } else {
                rect.right - pad
            };
            let name = RECT {
                left: text_left,
                right: text_right,
                ..rect
            };
            let width = draw_text(
                hdc,
                &row.name,
                name,
                paint.fonts.text,
                foreground,
                line | DT_LEFT | DT_END_ELLIPSIS,
            );
            if let Some(hint) = &row.hint {
                draw_text(
                    hdc,
                    hint,
                    RECT {
                        left: text_left + width + scale(GAP_AT_96_DPI, dpi),
                        ..name
                    },
                    paint.fonts.text,
                    palette.muted_foreground,
                    line | DT_LEFT | DT_END_ELLIPSIS,
                );
            }
        }
    }

    /// `WM_MOUSEMOVE`. Returns whether the hover changed.
    pub(crate) fn hover(&mut self, point: POINT, client: RECT, dpi: u32) -> bool {
        let area = self.list_area(client, dpi);
        let row = if inside(area, point) {
            self.list.row_at(point.y - area.top)
        } else {
            None
        };
        let header = inside(Self::header_button(client, dpi), point);
        let changed = row != self.list.hover || header != self.header_hover;
        self.list.hover = row;
        self.header_hover = header;
        changed
    }

    /// `WM_MOUSELEAVE`. Returns whether anything was hovered.
    pub(crate) fn leave(&mut self) -> bool {
        let changed = self.list.hover.is_some() || self.header_hover;
        self.list.hover = None;
        self.header_hover = false;
        changed
    }

    /// `WM_LBUTTONDOWN`. Selects the row under `point` and says what the click does.
    pub(crate) fn click(&mut self, point: POINT, client: RECT, dpi: u32) -> Option<FavoriteAction> {
        if inside(Self::header_button(client, dpi), point) {
            return Some(FavoriteAction::Browse);
        }
        let area = self.list_area(client, dpi);
        if !inside(area, point) {
            return None;
        }
        let index = self.list.row_at(point.y - area.top)?;
        self.list.select(index, area.bottom - area.top);
        let Some(row) = self.rows.get(index) else {
            return Some(FavoriteAction::Browse);
        };
        Some(if point.x >= Self::star_left(area, dpi) {
            FavoriteAction::Remove(row.folder.clone())
        } else {
            FavoriteAction::Open(row.folder.clone())
        })
    }

    /// `WM_KEYDOWN` while the panel has the focus.
    pub(crate) fn key(&mut self, key: u16, client: RECT, dpi: u32) -> KeyResult {
        let area = self.list_area(client, dpi);
        let height = area.bottom - area.top;
        let movement = match key {
            VK_UP => Some(ListKey::Up),
            VK_DOWN => Some(ListKey::Down),
            VK_HOME => Some(ListKey::Home),
            VK_END => Some(ListKey::End),
            VK_PRIOR => Some(ListKey::PageUp),
            VK_NEXT => Some(ListKey::PageDown),
            _ => None,
        };
        if let Some(movement) = movement {
            self.list.move_selection(movement, height);
            return KeyResult::Handled;
        }
        let selected = self.list.selected.map(|index| self.rows.get(index));
        match (key, selected) {
            (VK_RETURN | VK_SPACE, Some(Some(row))) => {
                KeyResult::Run(FavoriteAction::Open(row.folder.clone()))
            }
            (VK_RETURN | VK_SPACE, Some(None)) => KeyResult::Run(FavoriteAction::Browse),
            (VK_DELETE, Some(Some(row))) => KeyResult::Run(FavoriteAction::Remove(row.folder.clone())),
            (VK_RETURN | VK_SPACE | VK_DELETE, _) => KeyResult::Handled,
            _ => KeyResult::Unhandled,
        }
    }

    /// The favorite a context menu acts on, and where the menu opens in client coordinates. A
    /// right-click selects the row under `point`. The keyboard (`None`) uses the selected row.
    pub(crate) fn menu_target(
        &mut self,
        point: Option<POINT>,
        client: RECT,
        dpi: u32,
    ) -> Option<(PathBuf, POINT)> {
        let area = self.list_area(client, dpi);
        let (index, at) = match point {
            Some(point) => {
                if !inside(area, point) {
                    return None;
                }
                let index = self.list.row_at(point.y - area.top)?;
                self.list.select(index, area.bottom - area.top);
                (index, point)
            }
            None => {
                let index = self.list.selected?;
                let top = self.list.row_top(index)?;
                let at = POINT {
                    x: area.left + scale(PADDING_AT_96_DPI, dpi),
                    y: area.top + top + self.list.row_height,
                };
                (index, at)
            }
        };
        Some((self.rows.get(index)?.folder.clone(), at))
    }

    /// `WM_MOUSEWHEEL`'s delta. Returns whether the list scrolled.
    pub(crate) fn wheel(&mut self, delta: i32, client: RECT, dpi: u32) -> bool {
        let area = self.list_area(client, dpi);
        self.list
            .scroll_lines(-delta * WHEEL_LINES / WHEEL_DELTA, area.bottom - area.top)
    }
}

/// Runs `f` on the Favorites view with nothing else of the App borrowed.
fn with_view<R>(hwnd: HWND, f: impl FnOnce(&mut FavoritesView) -> R) -> Option<R> {
    let mut app = unsafe { super::main_window::app_ptr(hwnd) }?;
    unsafe { app.as_mut() }
        .sidebar
        .as_mut()
        .map(|sidebar| f(&mut sidebar.favorites))
}

fn geometry(panel: HWND) -> (RECT, u32) {
    let mut client = RECT::default();
    unsafe {
        GetClientRect(panel, &mut client);
    }
    (client, unsafe { GetDpiForWindow(panel) }.max(96))
}

fn invalidate(panel: HWND) {
    unsafe {
        InvalidateRect(panel, std::ptr::null(), 0);
    }
}

fn point_from(lparam: LPARAM) -> POINT {
    POINT {
        x: (lparam as u32 & 0xffff) as u16 as i16 as i32,
        y: ((lparam as u32 >> 16) & 0xffff) as u16 as i16 as i32,
    }
}

/// Re-reads the favorites and the open notebook. Part of `side_panel::refresh`.
pub(crate) fn refresh(hwnd: HWND, panel: HWND) {
    let favorites = library_host::favorites(hwnd);
    let open = library_host::folder(hwnd);
    let rows = favorite_rows(&favorites, open.as_deref());
    let (client, dpi) = geometry(panel);
    with_view(hwnd, |view| {
        let area = view.list_area(client, dpi);
        view.set_rows(rows, dpi, area.bottom - area.top);
    });
    invalidate(panel);
}

/// The panel's `WM_PAINT` while the Favorites view shows.
pub(crate) fn paint(hwnd: HWND, paint: &ViewPaint) {
    if let Some(app) = unsafe { super::main_window::app_ptr(hwnd) }
        && let Some(sidebar) = unsafe { app.as_ref() }.sidebar.as_ref()
    {
        sidebar.favorites.paint(paint);
    }
}

/// Header rectangles that are controls, not a window drag area.
pub(crate) fn header_controls(client: RECT, dpi: u32) -> Vec<RECT> {
    vec![FavoritesView::header_button(client, dpi)]
}

/// Input for the Favorites view. `None` leaves the message to the panel's default handling.
pub(crate) fn handle(
    hwnd: HWND,
    panel: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> Option<LRESULT> {
    let (client, dpi) = geometry(panel);
    match message {
        WM_MOUSEMOVE => {
            let mut track = TRACKMOUSEEVENT {
                cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: panel,
                dwHoverTime: 0,
            };
            unsafe {
                TrackMouseEvent(&mut track);
            }
            let point = point_from(lparam);
            if with_view(hwnd, |view| view.hover(point, client, dpi)).unwrap_or(false) {
                invalidate(panel);
            }
            Some(0)
        }
        WM_MOUSELEAVE => {
            if with_view(hwnd, FavoritesView::leave).unwrap_or(false) {
                invalidate(panel);
            }
            Some(0)
        }
        WM_LBUTTONDOWN => {
            if unsafe { GetFocus() } != panel {
                unsafe {
                    SetFocus(panel);
                }
            }
            let point = point_from(lparam);
            let action = with_view(hwnd, |view| view.click(point, client, dpi)).flatten();
            invalidate(panel);
            if let Some(action) = action {
                run(hwnd, action, false);
            }
            Some(0)
        }
        WM_MOUSEWHEEL => {
            let delta = ((wparam >> 16) & 0xffff) as u16 as i16 as i32;
            if with_view(hwnd, |view| view.wheel(delta, client, dpi)).unwrap_or(false) {
                invalidate(panel);
            }
            Some(0)
        }
        WM_KEYDOWN => {
            let result = with_view(hwnd, |view| view.key(wparam as u16, client, dpi))
                .unwrap_or(KeyResult::Unhandled);
            match result {
                KeyResult::Unhandled => None,
                KeyResult::Handled => {
                    invalidate(panel);
                    Some(0)
                }
                KeyResult::Run(action) => {
                    invalidate(panel);
                    run(hwnd, action, true);
                    Some(0)
                }
            }
        }
        WM_CONTEXTMENU => {
            // Shift+F10 and the context-menu key send (-1, -1) instead of a screen point.
            let keyboard = lparam as u32 == u32::MAX;
            let at = (!keyboard).then(|| {
                let mut point = point_from(lparam);
                unsafe {
                    ScreenToClient(panel, &mut point);
                }
                point
            });
            let target = with_view(hwnd, |view| view.menu_target(at, client, dpi)).flatten();
            invalidate(panel);
            if let Some((folder, point)) = target
                && let Some(action) = show_menu(hwnd, panel, point, folder)
            {
                run(hwnd, action, keyboard);
            }
            Some(0)
        }
        _ => None,
    }
}

/// Runs a Favorites action. `keyboard` keeps the focus in the panel when the Notebook view
/// replaces this one.
pub(crate) fn run(hwnd: HWND, action: FavoriteAction, keyboard: bool) {
    match action {
        // Checked on a worker: a missing folder shows a notice and changes nothing (spec §4.3).
        // Once the notebook is open, the Notebook view shows it.
        FavoriteAction::Open(folder) => {
            library_host::open_listed_notebook_in_view(hwnd, &folder, keyboard);
        }
        FavoriteAction::Remove(folder) => library_host::remove_favorite(hwnd, &folder),
        FavoriteAction::Reveal(folder) => library_host::reveal(hwnd, &folder),
        FavoriteAction::Browse => library_host::choose_and_open_folder(hwnd),
    }
}

/// A row's context menu (spec §7) at `point`, in panel client coordinates, for `folder`. The
/// entries are commands because `menus::track_popup` returns one, and the view reads them itself,
/// as the Notebook view's menus do: `OpenFolder` opens this notebook, `ToggleNotebookFavorite`
/// removes it and `NoteRevealInExplorer` reveals it. No command numbers are spent on menu-only
/// entries. Tests answer it with `menus::answer_next_popup_menu`.
fn show_menu(hwnd: HWND, panel: HWND, point: POINT, folder: PathBuf) -> Option<FavoriteAction> {
    // `track_popup` takes main-window client coordinates.
    let mut at = point;
    unsafe {
        ClientToScreen(panel, &mut at);
        ScreenToClient(hwnd, &mut at);
    }
    let entries = [
        MenuEntry::command("&Open", CommandId::OpenFolder),
        MenuEntry::command("&Remove from favorites", CommandId::ToggleNotebookFavorite),
        MenuEntry::command("Reveal in &Explorer", CommandId::NoteRevealInExplorer),
    ];
    match menus::track_popup(hwnd, &entries, at)? {
        CommandId::OpenFolder => Some(FavoriteAction::Open(folder)),
        CommandId::ToggleNotebookFavorite => Some(FavoriteAction::Remove(folder)),
        CommandId::NoteRevealInExplorer => Some(FavoriteAction::Reveal(folder)),
        _ => None,
    }
}

/// The rows the view lists, for in-process tests.
#[cfg(test)]
pub(crate) fn shown_rows(hwnd: HWND) -> Vec<FavoriteRow> {
    with_view(hwnd, |view| view.rows.clone()).unwrap_or_default()
}
```

Append the Step 1 test module to this file.

- [ ] **Step 6: Write `src/window/search_view.rs`**

```rust
//! The Search view (sidebar spec §8): a search box over the open notebook's note names, and the
//! matches with their folders. Enter or a click opens a match following the preview-tab rules
//! (§6.4). The box's placeholder is painted the way the find bar paints its placeholder. FastPad
//! has no ComCtl32 v6 manifest, so `EM_SETCUEBANNER` would show nothing.

use crate::config::SidebarView;
use crate::library::model::same_path;
use crate::library::name_search::{self, NameMatch};
use crate::platform::{last_error, wide_null};
use crate::window::library_host;
use crate::window::main_window::OpenMode;
use crate::window::palette::Palette;
use crate::window::panel::{create_child, fill, inset, scale, text_height};
use crate::window::row_list::{self, ListKey, RowListState, RowLook};
use crate::window::side_panel::{self, ViewPaint, draw_text};
use std::path::{Path, PathBuf};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, DT_CENTER, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX,
    DT_SINGLELINE, DT_VCENTER, DeleteObject, EndPaint, HBRUSH, HDC, InvalidateRect, PAINTSTRUCT,
    SetBkColor, SetTextColor,
};
use windows_sys::Win32::UI::Controls::{
    EM_GETMARGINS, EM_REPLACESEL, EM_SETSEL, EM_UNDO, WM_MOUSELEAVE,
};
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, SetFocus, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent, VK_CONTROL, VK_DOWN,
    VK_END, VK_ESCAPE, VK_HOME, VK_NEXT, VK_PRIOR, VK_RETURN, VK_UP,
};
use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DestroyWindow, ES_AUTOHSCROLL, GetClientRect, GetParent, GetWindowTextLengthW,
    GetWindowTextW, MoveWindow, SW_HIDE, SW_SHOWNA, SendMessageW, SetWindowTextW, ShowWindow,
    WM_CHAR, WM_CLEAR, WM_CUT, WM_GETFONT, WM_KEYDOWN, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN,
    WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCDESTROY, WM_PAINT, WM_PASTE, WM_SETFONT, WM_SETTEXT,
    WM_UNDO, WS_CHILD,
};

pub(crate) const RESULT_LIMIT: usize = 500;
pub(crate) const NO_NOTEBOOK: &str = "Open a notebook to search it.";
pub(crate) const NO_MATCH: &str = "No notes match.";
pub(crate) const LOADING: &str = "Loading\u{2026}";

const HEADER_AT_96_DPI: i32 = 38;
const ROW_AT_96_DPI: i32 = 26;
const PADDING_AT_96_DPI: i32 = 12;
const FIELD_MARGIN_AT_96_DPI: i32 = 8;
const FIELD_HEIGHT_AT_96_DPI: i32 = 28;
const FIELD_TEXT_INSET_AT_96_DPI: i32 = 8;
const GLYPH_AT_96_DPI: i32 = 20;
const GAP_AT_96_DPI: i32 = 6;
const WHEEL_DELTA: i32 = 120;
const WHEEL_LINES: i32 = 3;
const DOCUMENT_GLYPH: &str = "\u{E8A5}";
const SEARCH_HOOK_ID: usize = 0x4650_5356;

/// The box's placeholder: "Search <notebook>".
pub(crate) fn placeholder(notebook: Option<&Path>) -> String {
    notebook
        .and_then(Path::file_name)
        .map(|name| format!("Search {}", name.to_string_lossy()))
        .unwrap_or_else(|| "Search".to_owned())
}

/// The line shown instead of results, if any.
pub(crate) fn status_text(
    notebook_open: bool,
    loaded: bool,
    query: &str,
    results: usize,
) -> Option<&'static str> {
    if !notebook_open {
        Some(NO_NOTEBOOK)
    } else if query.trim().is_empty() {
        None
    } else if !loaded {
        Some(LOADING)
    } else if results == 0 {
        Some(NO_MATCH)
    } else {
        None
    }
}

fn inside(rect: RECT, point: POINT) -> bool {
    point.x >= rect.left && point.x < rect.right && point.y >= rect.top && point.y < rect.bottom
}

fn window_text(hwnd: HWND) -> String {
    let length = unsafe { GetWindowTextLengthW(hwnd) };
    if length <= 0 {
        return String::new();
    }
    let mut buffer = vec![0u16; length as usize + 1];
    let copied = unsafe { GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32) };
    buffer.truncate(copied.max(0) as usize);
    String::from_utf16_lossy(&buffer)
}

#[derive(Debug)]
pub(crate) struct SearchView {
    edit: HWND,
    brush: HBRUSH,
    colors: Palette,
    notebook: Option<PathBuf>,
    /// The library's state has loaded, so an empty result list means no match.
    loaded: bool,
    /// The notebook's note paths, relative to it. They are copied from the library on the first
    /// search after a change and dropped when the library changes, so an idle Search view holds
    /// nothing.
    notes: Option<Vec<PathBuf>>,
    pub(crate) query: String,
    pub(crate) results: Vec<NameMatch>,
    pub(crate) list: RowListState,
    placeholder: String,
}

impl SearchView {
    /// Creates the hidden search box inside `panel`.
    pub(crate) fn create(panel: HWND, dpi: u32) -> crate::Result<Self> {
        let edit = create_child(panel, &wide_null("Edit"), WS_CHILD | ES_AUTOHSCROLL as u32)?;
        if unsafe { SetWindowSubclass(edit, Some(search_edit_proc), SEARCH_HOOK_ID, 0) } == 0 {
            let error = last_error();
            unsafe {
                DestroyWindow(edit);
            }
            return Err(error);
        }
        let colors = Palette::neutral();
        Ok(Self {
            edit,
            brush: unsafe { CreateSolidBrush(colors.editor_background) },
            colors,
            notebook: None,
            loaded: false,
            notes: None,
            query: String::new(),
            results: Vec::new(),
            list: RowListState::new(scale(ROW_AT_96_DPI, dpi)),
            placeholder: placeholder(None),
        })
    }

    /// The painted search field, border included.
    pub(crate) fn field_rect(client: RECT, dpi: u32) -> RECT {
        let margin = scale(FIELD_MARGIN_AT_96_DPI, dpi);
        let height = scale(FIELD_HEIGHT_AT_96_DPI, dpi);
        let top = client.top + (scale(HEADER_AT_96_DPI, dpi) - height) / 2;
        RECT {
            left: client.left + margin,
            top,
            right: (client.right - margin).max(client.left + margin),
            bottom: top + height,
        }
    }

    /// Where the results are: everything under the header.
    pub(crate) fn list_area(&self, client: RECT, dpi: u32) -> RECT {
        RECT {
            top: (client.top + scale(HEADER_AT_96_DPI, dpi)).min(client.bottom),
            ..client
        }
    }

    fn set_colors(&mut self, colors: Palette) {
        if colors == self.colors {
            return;
        }
        unsafe {
            DeleteObject(self.brush);
            self.brush = CreateSolidBrush(colors.editor_background);
        }
        self.colors = colors;
    }

    /// Recomputes the results for `query`, with the first one selected.
    fn filter(&mut self, query: &str, client: RECT, dpi: u32) {
        self.query = query.to_owned();
        self.results = match &self.notes {
            Some(notes) if !query.trim().is_empty() => {
                name_search::search(notes, query, RESULT_LIMIT)
            }
            _ => Vec::new(),
        };
        let area = self.list_area(client, dpi);
        self.list.row_height = scale(ROW_AT_96_DPI, dpi);
        self.list.set_count(self.results.len());
        self.list.top = 0;
        self.list.selected = None;
        if !self.results.is_empty() {
            self.list.select(0, area.bottom - area.top);
        }
    }

    pub(crate) fn status(&self) -> Option<&'static str> {
        status_text(
            self.notebook.is_some(),
            self.loaded,
            &self.query,
            self.results.len(),
        )
    }

    pub(crate) fn paint(&self, paint: &ViewPaint) {
        let dpi = paint.dpi;
        let palette = paint.palette;
        let client = paint.client;
        let pad = scale(PADDING_AT_96_DPI, dpi);
        let line = DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX;
        unsafe {
            fill(paint.hdc, client, paint.background);
            if self.notebook.is_some() {
                let field = Self::field_rect(client, dpi);
                fill(paint.hdc, field, palette.selection_background);
                fill(paint.hdc, inset(field, 1), palette.editor_background);
            }
            if let Some(status) = self.status() {
                let area = self.list_area(client, dpi);
                let status_line = RECT {
                    left: client.left + pad,
                    top: area.top,
                    right: client.right - pad,
                    bottom: area.top + scale(ROW_AT_96_DPI, dpi),
                };
                draw_text(
                    paint.hdc,
                    status,
                    status_line,
                    paint.fonts.text,
                    palette.muted_foreground,
                    line | DT_LEFT | DT_END_ELLIPSIS,
                );
                return;
            }
        }
        let area = self.list_area(client, dpi);
        row_list::paint(
            paint.hdc,
            area,
            &self.list,
            &palette,
            paint.focused,
            &mut |hdc, index, rect, look| self.draw_row(hdc, index, rect, look, paint),
        );
    }

    /// One result: the file icon, the name, and its folder in dim text.
    fn draw_row(&self, hdc: HDC, index: usize, rect: RECT, look: RowLook, paint: &ViewPaint) {
        let Some(result) = self.results.get(index) else {
            return;
        };
        let dpi = paint.dpi;
        let palette = paint.palette;
        let pad = scale(PADDING_AT_96_DPI, dpi);
        let line = DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX;
        let glyph = RECT {
            left: rect.left + pad,
            right: rect.left + pad + scale(GLYPH_AT_96_DPI, dpi),
            ..rect
        };
        let foreground = if look.selected {
            palette
                .selection_foreground
                .unwrap_or(palette.editor_foreground)
        } else {
            palette.editor_foreground
        };
        let text = RECT {
            left: glyph.right + scale(GAP_AT_96_DPI, dpi),
            right: rect.right - pad,
            ..rect
        };
        unsafe {
            draw_text(
                hdc,
                DOCUMENT_GLYPH,
                glyph,
                paint.fonts.glyph,
                palette.muted_foreground,
                line | DT_CENTER,
            );
            let width = draw_text(
                hdc,
                &result.name,
                text,
                paint.fonts.text,
                foreground,
                line | DT_LEFT | DT_END_ELLIPSIS,
            );
            if !result.folder.is_empty() {
                draw_text(
                    hdc,
                    &result.folder,
                    RECT {
                        left: text.left + width + scale(GAP_AT_96_DPI, dpi),
                        ..text
                    },
                    paint.fonts.text,
                    palette.muted_foreground,
                    line | DT_LEFT | DT_END_ELLIPSIS,
                );
            }
        }
    }

    fn row_under(&self, point: POINT, client: RECT, dpi: u32) -> Option<usize> {
        let area = self.list_area(client, dpi);
        if !inside(area, point) {
            return None;
        }
        self.list
            .row_at(point.y - area.top)
            .filter(|&index| index < self.results.len())
    }
}

impl Drop for SearchView {
    fn drop(&mut self) {
        unsafe {
            DeleteObject(self.brush);
        }
    }
}

fn with_view<R>(hwnd: HWND, f: impl FnOnce(&mut SearchView) -> R) -> Option<R> {
    let mut app = unsafe { super::main_window::app_ptr(hwnd) }?;
    unsafe { app.as_mut() }
        .sidebar
        .as_mut()
        .map(|sidebar| f(&mut sidebar.search))
}

fn geometry(panel: HWND) -> (RECT, u32) {
    let mut client = RECT::default();
    unsafe {
        GetClientRect(panel, &mut client);
    }
    (client, unsafe { GetDpiForWindow(panel) }.max(96))
}

fn invalidate(hwnd: HWND) {
    unsafe {
        InvalidateRect(hwnd, std::ptr::null(), 0);
    }
}

fn point_from(lparam: LPARAM) -> POINT {
    POINT {
        x: (lparam as u32 & 0xffff) as u16 as i16 as i32,
        y: ((lparam as u32 >> 16) & 0xffff) as u16 as i16 as i32,
    }
}

/// `EN_CHANGE` from the box: re-runs the search.
pub(crate) fn query_changed(hwnd: HWND) {
    let Some(edit) = with_view(hwnd, |view| view.edit) else {
        return;
    };
    let query = window_text(edit);
    let needs_notes = !query.trim().is_empty()
        && with_view(hwnd, |view| view.notes.is_none()).unwrap_or(false);
    if needs_notes {
        let notes = library_host::with_state(hwnd, |state| {
            state
                .notes
                .iter()
                .map(|note| note.path.clone())
                .collect::<Vec<_>>()
        });
        with_view(hwnd, |view| {
            view.loaded = notes.is_some();
            if notes.is_some() {
                view.notes = notes;
            }
        });
    }
    let panel = unsafe { GetParent(edit) };
    let (client, dpi) = geometry(panel);
    with_view(hwnd, |view| view.filter(&query, client, dpi));
    invalidate(panel);
}

/// Part of `side_panel::refresh`. A new notebook clears the query. The same notebook re-runs it
/// against the changed note list.
pub(crate) fn library_changed(hwnd: HWND) {
    let notebook = library_host::folder(hwnd);
    let loaded = library_host::with_state(hwnd, |_| ()).is_some();
    let Some((edit, changed)) = with_view(hwnd, |view| {
        let changed = match (&view.notebook, &notebook) {
            (Some(old), Some(new)) => !same_path(old, new),
            (None, None) => false,
            _ => true,
        };
        view.notes = None;
        view.loaded = loaded;
        if changed {
            view.notebook = notebook.clone();
            view.placeholder = placeholder(notebook.as_deref());
        }
        (view.edit, changed)
    }) else {
        return;
    };
    if changed {
        // Clearing the box sends EN_CHANGE, which empties the results.
        let empty = wide_null("");
        unsafe {
            SetWindowTextW(edit, empty.as_ptr());
            InvalidateRect(edit, std::ptr::null(), 1);
        }
        layout(hwnd);
    } else {
        query_changed(hwnd);
    }
}

/// Places the box in the header and shows it while the Search view shows a notebook. Part of
/// `side_panel::layout`, and run whenever the view or the notebook changes.
pub(crate) fn layout(hwnd: HWND) {
    let Some((edit, has_notebook)) = with_view(hwnd, |view| (view.edit, view.notebook.is_some()))
    else {
        return;
    };
    let panel = unsafe { GetParent(edit) };
    let (client, dpi) = geometry(panel);
    let text_font = super::main_window::ui_fonts(hwnd).text;
    unsafe {
        if !text_font.is_null() {
            SendMessageW(edit, WM_SETFONT, text_font as WPARAM, 0);
        }
    }
    let field = SearchView::field_rect(client, dpi);
    let text = text_height(edit, text_font).clamp(1, (field.bottom - field.top - 2).max(1));
    let inset_x = scale(FIELD_TEXT_INSET_AT_96_DPI, dpi);
    let top = field.top + (field.bottom - field.top - text) / 2;
    unsafe {
        MoveWindow(
            edit,
            field.left + inset_x,
            top,
            (field.right - field.left - 2 * inset_x).max(0),
            text,
            1,
        );
    }
    with_view(hwnd, |view| view.list.row_height = scale(ROW_AT_96_DPI, dpi));
    let show = has_notebook && side_panel::current_view(hwnd) == SidebarView::Search;
    unsafe {
        ShowWindow(edit, if show { SW_SHOWNA } else { SW_HIDE });
    }
}

/// `side_panel::show_view` switched to Search. `focus` puts the caret in the box (Ctrl+K).
pub(crate) fn shown(hwnd: HWND, focus: bool) {
    layout(hwnd);
    let Some((edit, has_notebook)) = with_view(hwnd, |view| (view.edit, view.notebook.is_some()))
    else {
        return;
    };
    if !focus {
        return;
    }
    unsafe {
        if has_notebook {
            SetFocus(edit);
            SendMessageW(edit, EM_SETSEL, 0, -1);
        } else {
            SetFocus(GetParent(edit));
        }
    }
}

/// `side_panel::show_view` switched away from Search. The query stays.
pub(crate) fn hidden(hwnd: HWND) {
    if let Some(edit) = with_view(hwnd, |view| view.edit) {
        unsafe {
            ShowWindow(edit, SW_HIDE);
        }
    }
}

/// The panel's `WM_PAINT` while the Search view shows.
pub(crate) fn paint(hwnd: HWND, paint: &ViewPaint) {
    with_view(hwnd, |view| view.set_colors(paint.palette));
    if let Some(app) = unsafe { super::main_window::app_ptr(hwnd) }
        && let Some(sidebar) = unsafe { app.as_ref() }.sidebar.as_ref()
    {
        sidebar.search.paint(paint);
    }
}

/// `WM_CTLCOLOREDIT` for the box.
pub(crate) fn control_color(hwnd: HWND, dc: HDC) -> HBRUSH {
    with_view(hwnd, |view| {
        unsafe {
            SetTextColor(dc, view.colors.editor_foreground);
            SetBkColor(dc, view.colors.editor_background);
        }
        view.brush
    })
    .unwrap_or(std::ptr::null_mut())
}

/// Opens the selected result, or the first one, as the preview tab or a normal tab.
pub(crate) fn open_selected(hwnd: HWND, mode: OpenMode, focus_editor: bool) {
    let Some(path) = with_view(hwnd, |view| {
        let index = view.list.selected.unwrap_or(0);
        view.results.get(index).map(|result| result.path.clone())
    })
    .flatten() else {
        return;
    };
    open_result(hwnd, &path, mode, focus_editor);
}

fn open_result(hwnd: HWND, relative: &Path, mode: OpenMode, focus_editor: bool) {
    let Some(folder) = library_host::folder(hwnd) else {
        return;
    };
    let path = folder.join(relative);
    if let Err(error) = super::main_window::open_note(hwnd, &path, mode, focus_editor) {
        super::main_window::push_notice(
            hwnd,
            format!("FastPad could not open {}: {error}", path.display()),
        );
    }
}

/// Down from the box: the focus moves into the results.
fn enter_results(hwnd: HWND, panel: HWND) {
    let (client, dpi) = geometry(panel);
    let has_results = with_view(hwnd, |view| {
        if view.results.is_empty() {
            return false;
        }
        let area = view.list_area(client, dpi);
        let index = view.list.selected.unwrap_or(0);
        view.list.select(index, area.bottom - area.top);
        true
    })
    .unwrap_or(false);
    if has_results {
        unsafe {
            SetFocus(panel);
        }
        invalidate(panel);
    }
}

/// Input for the Search view's result list. `None` leaves the message to the panel.
pub(crate) fn handle(
    hwnd: HWND,
    panel: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> Option<LRESULT> {
    let (client, dpi) = geometry(panel);
    match message {
        WM_MOUSEMOVE => {
            let mut track = TRACKMOUSEEVENT {
                cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: panel,
                dwHoverTime: 0,
            };
            unsafe {
                TrackMouseEvent(&mut track);
            }
            let point = point_from(lparam);
            let changed = with_view(hwnd, |view| {
                let hover = view.row_under(point, client, dpi);
                std::mem::replace(&mut view.list.hover, hover) != hover
            })
            .unwrap_or(false);
            if changed {
                invalidate(panel);
            }
            Some(0)
        }
        WM_MOUSELEAVE => {
            if with_view(hwnd, |view| view.list.hover.take().is_some()).unwrap_or(false) {
                invalidate(panel);
            }
            Some(0)
        }
        WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
            let point = point_from(lparam);
            let path = with_view(hwnd, |view| {
                let index = view.row_under(point, client, dpi)?;
                let area = view.list_area(client, dpi);
                view.list.select(index, area.bottom - area.top);
                Some(view.results[index].path.clone())
            })
            .flatten();
            invalidate(panel);
            if let Some(path) = path {
                let mode = if message == WM_LBUTTONDBLCLK {
                    OpenMode::Permanent
                } else {
                    OpenMode::Preview
                };
                // A mouse click moves the focus to the editor (spec §6.4).
                open_result(hwnd, &path, mode, true);
            }
            Some(0)
        }
        WM_MOUSEWHEEL => {
            let delta = ((wparam >> 16) & 0xffff) as u16 as i16 as i32;
            let scrolled = with_view(hwnd, |view| {
                let area = view.list_area(client, dpi);
                view.list
                    .scroll_lines(-delta * WHEEL_LINES / WHEEL_DELTA, area.bottom - area.top)
            })
            .unwrap_or(false);
            if scrolled {
                invalidate(panel);
            }
            Some(0)
        }
        WM_KEYDOWN => {
            let key = wparam as u16;
            let ctrl = unsafe { GetKeyState(i32::from(VK_CONTROL)) } < 0;
            if key == VK_RETURN {
                if ctrl {
                    open_selected(hwnd, OpenMode::Permanent, true);
                } else {
                    // Enter keeps the focus in the list, so arrows and Enter browse (spec §6.4).
                    open_selected(hwnd, OpenMode::Preview, false);
                }
                return Some(0);
            }
            let at_top = with_view(hwnd, |view| view.list.selected.is_none_or(|index| index == 0))
                .unwrap_or(true);
            if key == VK_UP && at_top {
                if let Some(edit) = with_view(hwnd, |view| view.edit) {
                    unsafe {
                        SetFocus(edit);
                    }
                }
                return Some(0);
            }
            let movement = match key {
                VK_UP => ListKey::Up,
                VK_DOWN => ListKey::Down,
                VK_HOME => ListKey::Home,
                VK_END => ListKey::End,
                VK_PRIOR => ListKey::PageUp,
                VK_NEXT => ListKey::PageDown,
                _ => return None,
            };
            with_view(hwnd, |view| {
                let area = view.list_area(client, dpi);
                view.list.move_selection(movement, area.bottom - area.top)
            });
            invalidate(panel);
            Some(0)
        }
        // Typing in the list goes on in the box.
        WM_CHAR if (wparam as u32) >= 0x20 && wparam as u32 != 0x7f => {
            let edit = with_view(hwnd, |view| view.edit)?;
            unsafe {
                SetFocus(edit);
                SendMessageW(edit, WM_CHAR, wparam, lparam);
            }
            Some(0)
        }
        _ => None,
    }
}

/// The box's placeholder, painted where typed text starts.
fn paint_placeholder(hwnd: HWND, edit: HWND) -> bool {
    let Some((text, colors)) = with_view(hwnd, |view| (view.placeholder.clone(), view.colors))
    else {
        return false;
    };
    let mut paint = PAINTSTRUCT::default();
    let dc = unsafe { BeginPaint(edit, &mut paint) };
    if dc.is_null() {
        return true;
    }
    unsafe {
        let mut client = RECT::default();
        GetClientRect(edit, &mut client);
        fill(dc, client, colors.editor_background);
        let font = SendMessageW(edit, WM_GETFONT, 0, 0);
        client.left += (SendMessageW(edit, EM_GETMARGINS, 0, 0) & 0xffff) as i32;
        draw_text(
            dc,
            &text,
            client,
            font as _,
            colors.muted_foreground,
            DT_SINGLELINE | DT_NOPREFIX | DT_END_ELLIPSIS,
        );
        EndPaint(edit, &paint);
    }
    true
}

unsafe extern "system" fn search_edit_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _subclass_id: usize,
    _ref_data: usize,
) -> LRESULT {
    let panel = unsafe { GetParent(hwnd) };
    let main = unsafe { GetParent(panel) };
    if message == WM_NCDESTROY {
        unsafe {
            RemoveWindowSubclass(hwnd, Some(search_edit_proc), SEARCH_HOOK_ID);
        }
    }
    // A single-line Edit beeps at Enter and Escape characters; both are handled on key down.
    if message == WM_CHAR && matches!(wparam as u16, 0x0d | 0x1b) {
        return 0;
    }
    if message == WM_PAINT
        && unsafe { GetWindowTextLengthW(hwnd) } == 0
        && paint_placeholder(main, hwnd)
    {
        return 0;
    }
    if message == WM_KEYDOWN {
        let ctrl = unsafe { GetKeyState(i32::from(VK_CONTROL)) } < 0;
        match wparam as u16 {
            VK_DOWN | VK_NEXT => {
                enter_results(main, panel);
                return 0;
            }
            VK_RETURN if ctrl => {
                open_selected(main, OpenMode::Permanent, true);
                return 0;
            }
            VK_RETURN => {
                open_selected(main, OpenMode::Preview, false);
                return 0;
            }
            VK_ESCAPE => {
                super::main_window::focus_content(main);
                return 0;
            }
            _ => {}
        }
    }
    // The Edit repaints only the text it changes; the placeholder must go, or come back, whole.
    let edits_text = matches!(
        message,
        WM_CHAR
            | WM_KEYDOWN
            | WM_PASTE
            | WM_CUT
            | WM_CLEAR
            | WM_UNDO
            | WM_SETTEXT
            | EM_UNDO
            | EM_REPLACESEL
    );
    let was_empty = edits_text && unsafe { GetWindowTextLengthW(hwnd) } == 0;
    let result = unsafe { DefSubclassProc(hwnd, message, wparam, lparam) };
    if edits_text && was_empty != (unsafe { GetWindowTextLengthW(hwnd) } == 0) {
        unsafe {
            InvalidateRect(hwnd, std::ptr::null(), 1);
        }
    }
    result
}

/// The results listed, as (name, folder), for in-process tests.
#[cfg(test)]
pub(crate) fn shown_results(hwnd: HWND) -> Vec<(String, String)> {
    with_view(hwnd, |view| {
        view.results
            .iter()
            .map(|result| (result.name.clone(), result.folder.clone()))
            .collect()
    })
    .unwrap_or_default()
}

#[cfg(test)]
pub(crate) fn status(hwnd: HWND) -> Option<&'static str> {
    with_view(hwnd, |view| view.status()).flatten()
}

#[cfg(test)]
pub(crate) fn edit_hwnd(hwnd: HWND) -> Option<HWND> {
    with_view(hwnd, |view| view.edit)
}
```

Append the Step 1 test module to this file.

- [ ] **Step 7: Wire both views into Task 6's seam in `side_panel.rs`**

Each change below replaces code Task 6 or Task 10 wrote in `src/window/side_panel.rs`.

1. **`Sidebar` fields.** Add after `notebook`:

```rust
    pub(crate) favorites: crate::window::favorites_view::FavoritesView,
    pub(crate) search: crate::window::search_view::SearchView,
```

In `create`, after the `let panel = match panel { .. };` block, create the search box inside the panel. A failure destroys both windows, as a failed panel does:

```rust
    let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
    let search = match crate::window::search_view::SearchView::create(panel, dpi) {
        Ok(search) => search,
        Err(error) => {
            unsafe {
                DestroyWindow(panel);
                DestroyWindow(bar);
            }
            return Err(error);
        }
    };
```

and add to the `Ok(Sidebar { .. })` literal, after `notebook: ..`:

```rust
        favorites: crate::window::favorites_view::FavoritesView::new(dpi),
        search,
```

2. **Paint.** Replace `paint_view`, and delete `paint_header_title`, the `HEADER_INSET_96` constant and `PanelView::title`, which nothing uses any more (each view paints its own header). Remove `DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER` from the `Gdi` import (only `paint_header_title` used them), and in the side panel's tests delete the `assert_eq!(PanelView::Favorites.title(), "FAVORITES");` line of `every_open_view_has_a_panel_view_and_a_header_title`.

```rust
/// Paints `view` over the panel's background.
fn paint_view(main: HWND, view: PanelView, paint: &ViewPaint) {
    match view {
        PanelView::Notebook => crate::window::notebook_view::paint(main, paint),
        PanelView::Search => crate::window::search_view::paint(main, paint),
        PanelView::Favorites => crate::window::favorites_view::paint(main, paint),
    }
}
```

3. **Input.** Replace `view_mouse` and `view_key`:

```rust
/// Mouse input (and `WM_CONTEXTMENU`, `WM_MOUSELEAVE`, `WM_CAPTURECHANGED`) for `view`, with the
/// message's own `wparam` and `lparam`. `None` leaves it to `DefWindowProcW`.
fn view_mouse(
    main: HWND,
    view: PanelView,
    panel: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> Option<LRESULT> {
    match view {
        PanelView::Notebook => {
            crate::window::notebook_view::handle(main, message, wparam, lparam)
        }
        PanelView::Search => {
            crate::window::search_view::handle(main, panel, message, wparam, lparam)
        }
        PanelView::Favorites => {
            crate::window::favorites_view::handle(main, panel, message, wparam, lparam)
        }
    }
}

/// `WM_KEYDOWN` and `WM_CHAR` while the panel has the focus. `None` leaves the key to
/// `DefWindowProcW`.
fn view_key(
    main: HWND,
    view: PanelView,
    panel: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> Option<LRESULT> {
    view_mouse(main, view, panel, message, wparam, lparam)
}
```

Each view's `handle` takes keys and mouse messages alike, so `view_key` forwards to the same handlers.

4. **The header's drag area.** Replace `header_is_caption`. The search box is a child window, so it never reaches the panel's hit test:

```rust
/// Whether header point `x`, `y` (panel client coordinates) is empty, so the window drags from
/// it: not the Notebook header's title and buttons, nor the Favorites header's Open notebook…
/// button.
fn header_is_caption(main: HWND, view: PanelView, panel: HWND, x: i32, y: i32) -> bool {
    match view {
        PanelView::Notebook => !crate::window::notebook_view::header_hit(main, x, y),
        PanelView::Search => true,
        PanelView::Favorites => {
            let mut client = RECT::default();
            unsafe { GetClientRect(panel, &mut client) };
            let dpi = unsafe { GetDpiForWindow(panel) }.max(96);
            !crate::window::favorites_view::header_controls(client, dpi)
                .iter()
                .any(|rect| x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom)
        }
    }
}
```

5. **The search box's notifications.** In `panel_proc`, add these arms before the final `_ =>` arm, and add `EN_CHANGE, WM_COMMAND, WM_CTLCOLOREDIT` to the `WindowsAndMessaging` import:

```rust
        // The search box's text changed: re-run the search.
        WM_COMMAND if lparam != 0 && ((wparam >> 16) & 0xffff) as u32 == EN_CHANGE => {
            crate::window::search_view::query_changed(main);
            0
        }
        WM_CTLCOLOREDIT => {
            crate::window::search_view::control_color(main, wparam as HDC) as LRESULT
        }
```

6. **`refresh(hwnd)`.** After the `crate::window::notebook_view::rebuild(hwnd);` line Task 10 added:

```rust
    crate::window::favorites_view::refresh(hwnd, panel);
    crate::window::search_view::library_changed(hwnd);
```

7. **`layout(hwnd, client, dpi)`.** After its last line, `update_tools(hwnd);`, add:

```rust
    crate::window::search_view::layout(hwnd);
```

8. **`show_view(hwnd, view, focus)`.** At its end, after the `if focus && view != SidebarView::Hidden && is_shown(panel) { .. }` block, so `current_view` already returns the new view and the search box can take the focus from the panel:

```rust
    if view == SidebarView::Search {
        crate::window::search_view::shown(hwnd, focus);
    } else {
        crate::window::search_view::hidden(hwnd);
    }
```

`toggle` closes and reopens the panel through `show_view`, so it hides and shows the box too.

- [ ] **Step 8: The activity bar's Settings button opens the settings palette**

In `src/window/activity_bar.rs`, Task 6's Settings click handler runs the full command palette. Replace it with:

```rust
/// The Settings button's click handler: the palette listing only the settings commands.
fn open_settings(main: HWND) {
    super::main_window::open_settings_palette(main);
}
```

- [ ] **Step 9: Write the in-process tests** in the `main_window.rs` tests module

```rust
    fn sidebar_panel(hwnd: HWND) -> HWND {
        crate::window::side_panel::windows(hwnd).unwrap().1
    }

    fn type_into_search(hwnd: HWND, text: &str) {
        let edit = crate::window::search_view::edit_hwnd(hwnd).unwrap();
        let wide = crate::platform::wide_null(text);
        // The Edit sends EN_CHANGE to the panel, which re-runs the search synchronously.
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::SetWindowTextW(edit, wide.as_ptr());
        }
    }

    #[test]
    fn the_search_view_matches_names_shows_folders_and_opens_the_preview_tab() {
        // Break caught: search over full paths instead of names, results without their folder,
        // or Enter opening a normal tab instead of the preview tab.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("search-view");
        scratch.note("Alpha.md", "a");
        scratch.note("beta.md", "b");
        std::fs::create_dir_all(scratch.folder().join("sub")).unwrap();
        scratch.note(r"sub\alphabet.md", "c");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        scratch.install(window.hwnd);
        crate::window::side_panel::show_view(
            window.hwnd,
            crate::config::SidebarView::Search,
            true,
        );
        assert_eq!(crate::window::search_view::status(window.hwnd), None);

        type_into_search(window.hwnd, "alp");
        assert_eq!(
            crate::window::search_view::shown_results(window.hwnd),
            vec![
                ("Alpha".to_owned(), String::new()),
                ("alphabet".to_owned(), "sub".to_owned())
            ]
        );
        crate::window::search_view::open_selected(window.hwnd, super::OpenMode::Preview, false);
        let active = app_mut(window.hwnd).tabs.active().unwrap();
        assert_eq!(
            active.path.as_deref(),
            Some(scratch.folder().join("Alpha.md").as_path())
        );
        let active_id = active.id;
        assert_eq!(app_mut(window.hwnd).tabs.preview_id(), Some(active_id));

        type_into_search(window.hwnd, "zzz");
        assert_eq!(
            crate::window::search_view::status(window.hwnd),
            Some(crate::window::search_view::NO_MATCH)
        );
    }

    #[test]
    fn the_search_query_survives_a_view_switch_but_not_a_notebook_switch() {
        // Break caught: the query lost whenever another view is shown, or kept (with results
        // from the old notebook) after a different notebook opens.
        let _scintilla = load_native_scintilla();
        let first = LibraryScratch::new("search-keep-a");
        first.note("plan.md", "p");
        let second = LibraryScratch::new("search-keep-b");
        second.note("other.md", "o");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        first.install(window.hwnd);
        use crate::config::SidebarView;
        crate::window::side_panel::show_view(window.hwnd, SidebarView::Search, true);
        type_into_search(window.hwnd, "pl");
        crate::window::side_panel::show_view(window.hwnd, SidebarView::Notebook, false);
        crate::window::side_panel::show_view(window.hwnd, SidebarView::Search, false);
        assert_eq!(crate::window::search_view::shown_results(window.hwnd).len(), 1);

        second.install(window.hwnd);
        assert!(crate::window::search_view::shown_results(window.hwnd).is_empty());
        let edit = crate::window::search_view::edit_hwnd(window.hwnd).unwrap();
        assert_eq!(
            unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetWindowTextLengthW(edit) },
            0
        );
    }

    #[test]
    fn with_no_notebook_the_search_view_says_to_open_one() {
        // Break caught: an empty Search view with a live box that searches nothing.
        let window = ProductionWindow::new(make_app());
        crate::window::side_panel::show_view(
            window.hwnd,
            crate::config::SidebarView::Search,
            false,
        );
        assert_eq!(
            crate::window::search_view::status(window.hwnd),
            Some(crate::window::search_view::NO_NOTEBOOK)
        );
    }

    #[test]
    fn a_favorite_opens_from_the_favorites_view_and_its_menu_removes_it() {
        // Break caught: a click on a favorite not switching the notebook or leaving the Favorites
        // view up, or "Remove from favorites" in the row menu doing nothing.
        let _scintilla = load_native_scintilla();
        let first = LibraryScratch::new("favorites-a");
        let second = LibraryScratch::new("favorites-b");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        app_mut(window.hwnd).library.data_dir = Some(first.data());
        second.install(window.hwnd);
        crate::window::library_host::toggle_notebook_favorite(window.hwnd);
        first.install(window.hwnd);
        use crate::config::SidebarView;
        crate::window::side_panel::show_view(window.hwnd, SidebarView::Favorites, true);
        let rows = crate::window::favorites_view::shown_rows(window.hwnd);
        assert_eq!(rows.len(), 1);
        assert!(!rows[0].open);

        crate::window::favorites_view::run(
            window.hwnd,
            crate::window::favorites_view::FavoriteAction::Open(second.folder()),
            true,
        );
        // The folder is checked on a worker; the view switches once the notebook is open.
        pump_until(window.hwnd, || {
            crate::window::side_panel::current_view(window.hwnd) == SidebarView::Notebook
        });
        assert!(crate::library::model::same_path(
            &crate::window::library_host::folder(window.hwnd).unwrap(),
            &second.folder()
        ));

        crate::window::side_panel::show_view(window.hwnd, SidebarView::Favorites, true);
        let panel = sidebar_panel(window.hwnd);
        unsafe {
            SendMessageW(panel, windows_sys::Win32::UI::WindowsAndMessaging::WM_KEYDOWN, 0x24, 0);
        }
        crate::window::menus::answer_next_popup_menu(|_| Some(CommandId::ToggleNotebookFavorite));
        // Shift+F10 arrives as WM_CONTEXTMENU with (-1, -1).
        unsafe {
            SendMessageW(
                panel,
                windows_sys::Win32::UI::WindowsAndMessaging::WM_CONTEXTMENU,
                panel as usize,
                0xffff_ffff,
            );
        }
        assert!(crate::window::favorites_view::shown_rows(window.hwnd).is_empty());
    }

    #[test]
    fn the_settings_button_lists_only_settings_and_the_next_palette_lists_everything() {
        // Break caught: Settings showing the full command list, or its filter sticking to the
        // next Ctrl+Shift+P.
        let window = ProductionWindow::new(make_app());
        super::open_settings_palette(window.hwnd);
        let shown = with_command_palette(window.hwnd, |palette| {
            palette
                .shown()
                .iter()
                .map(|entry| entry.command)
                .collect::<Vec<_>>()
        })
        .unwrap();
        assert!(shown.contains(&CommandId::ThemeDark));
        assert!(
            shown
                .iter()
                .all(|command| crate::window::command_palette::SETTINGS_COMMANDS.contains(command))
        );
        super::close_command_palette(window.hwnd, false);

        execute_command(window.hwnd, CommandId::CommandPalette);
        let shown = with_command_palette(window.hwnd, |palette| palette.shown().len()).unwrap();
        assert!(shown > crate::window::command_palette::SETTINGS_COMMANDS.len());
    }
```

`0x24` is `VK_HOME`, which selects the first row before the menu opens.

- [ ] **Step 10: Run the targeted tests**

Run: `cargo test --lib -- favorites_view search_view every_settings_command the_search_view the_search_query with_no_notebook_the_search a_favorite_opens the_settings_button --test-threads=1`
Expected: all pass.

Run: `cargo clippy --all-targets -- -D warnings` and `cargo fmt --all -- --check`
Expected: no warnings and no diff.

- [ ] **Step 11: Commit**

```bash
git add src/window/favorites_view.rs src/window/search_view.rs src/window/mod.rs src/window/side_panel.rs src/window/library_host.rs src/window/command_palette.rs src/window/main_window.rs src/window/activity_bar.rs
git commit -m "feat(sidebar): Favorites and Search views, Settings button opens the settings palette"
```

---

### Task 13: Accessibility and keyboard focus

**Files:**
- Create: `src/window/sidebar_accessibility.rs`
- Modify:
  - `src/window/mod.rs`: `pub(crate) mod sidebar_accessibility;`
  - `src/window/side_panel.rs`: panel provider, events, Esc, `bar_focus`
  - `src/window/activity_bar.rs`: bar provider, arrow keys, focus rectangle
  - `src/window/notebook_view.rs`, `src/window/favorites_view.rs`, `src/window/search_view.rs`: `AccessibleView` implementations
  - `src/window/main_window.rs`: `FocusPart`, `next_focus_part`, `cycle_focus`, `return_focus_to_editor`, commands, and tests
  - `src/window/commands.rs`: `FocusNextPane = 183`, `FocusPreviousPane = 184`
  - `src/window/menus.rs`: F6 and Shift+F6
  - `src/window/command_palette.rs`: keeps the focus commands out of the palette

**Interfaces:**
- Consumes:
  - `accessibility::{AccessibleVtable, RawVariant, VariantValue, allocate_bstr, guid_eq}`, the IIDs, and the shared stub functions.
  - Task 3: `TreeRow`, `RowKind`.
  - Task 6: `ActivityButton` order and `activity_bar::button_rects(client: RECT, dpi: u32) -> [RECT; 4]` (Notebook, Search, Favorites, Settings), plus both window procedures.
  - Task 7: `RowListState`.
  - Task 10: the `NotebookView` accessors:
    - `rows(&self) -> &[TreeRow]`;
    - `list(&self) -> &RowListState` and `list_mut(&mut self) -> &mut RowListState`;
    - `list_area(&self, client: RECT, dpi: u32) -> RECT`;
    - `buttons(&self, client: RECT, dpi: u32) -> Vec<(String, RECT)>`, which lists every push button painted in order: the header's star, New note and "…", plus the no-notebook "Open notebook…" and the empty-notebook "New note" buttons;
    - `recent_rows(&self, client: RECT, dpi: u32) -> Vec<(String, RECT)>`, which lists the RECENT notebooks of the no-notebook state and is empty otherwise.
  - Task 6: `side_panel::windows(hwnd) -> Option<(HWND, HWND)>`. Tasks 10 and 12: `Sidebar.{notebook, favorites, search}`.
- Produces:
  - `sidebar_accessibility::AccessibleItem { name: String, role: u32, state: u32, rect: RECT, value: String }`. `value` is new: the outline level, per MSAA's outline-item convention.
  - `sidebar_accessibility::{AccessibleSource, AccessibleView, AccessibleMark, object_result, answer, run_action, events_between, raise, tree_item, list_item, button_item, row_rect, click_item}`.
  - `WM_FASTPAD_SIDEBAR_ACCESSIBLE = WM_APP + 0x60` and `WM_FASTPAD_SIDEBAR_ACTION = WM_APP + 0x61`.
  - **Contract change:** `side_panel::accessible_item_count(panel: HWND) -> usize` and `side_panel::accessible_item(panel: HWND, index: usize) -> Option<AccessibleItem>` replace `side_panel::accessible_items`. A 10,000-row tree would otherwise build 10,000 strings for every MSAA call.
  - `activity_bar::accessible_items(bar: HWND) -> Vec<AccessibleItem>` (always 4) and `activity_bar::bar_items` (pure).
  - `side_panel::{PANEL_ACCESSIBLE, with_accessible_events, bar_focus, set_bar_focus}` and `activity_bar::BAR_ACCESSIBLE`.
  - `main_window::{cycle_focus(hwnd, backwards: bool), FocusPart, next_focus_part, return_focus_to_editor}`.
  - `CommandId::{FocusNextPane = 183, FocusPreviousPane = 184}`.

**Threading:** MSAA clients call providers on RPC threads (see `src/preview/accessible.rs`). The provider holds only the window handle and a `&'static AccessibleSource`. On another thread, each query is `SendMessageW`'d to the window as `WM_FASTPAD_SIDEBAR_ACCESSIBLE`, so all App access stays on the UI thread. It uses `SendMessageW`, not `SendMessageTimeoutW`: a timed-out send can still be delivered later and would write through a dangling stack pointer. Default actions and selection are posted as `WM_FASTPAD_SIDEBAR_ACTION`.

- [ ] **Step 1: Write the failing pure tests** at the end of the new `src/window/sidebar_accessibility.rs` (Step 3 writes the rest of the file)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::tree::{RowKind, TreeRow};
    use crate::window::row_list::RowListState;
    use std::cell::Cell;
    use std::path::PathBuf;
    use windows_sys::Win32::Foundation::{SysFreeString, SysStringLen};

    thread_local! {
        static ITEMS_BUILT: Cell<usize> = const { Cell::new(0) };
    }

    const ROW: RECT = RECT {
        left: 0,
        top: 0,
        right: 100,
        bottom: 26,
    };

    fn fake_container(_: HWND) -> (String, u32) {
        ("Fake notes".to_owned(), ROLE_SYSTEM_LIST)
    }
    fn fake_count(_: HWND) -> usize {
        10_000
    }
    fn fake_item(_: HWND, index: usize) -> Option<AccessibleItem> {
        ITEMS_BUILT.set(ITEMS_BUILT.get() + 1);
        (index < 10_000).then(|| list_item(&format!("Note {index}"), index == 3, false, ROW, true))
    }
    fn fake_hit(_: HWND, _: POINT) -> Option<usize> {
        Some(7)
    }
    fn fake_current(_: HWND) -> Option<usize> {
        Some(3)
    }
    fn fake_select(_: HWND, _: usize) {}
    fn fake_activate(_: HWND, _: usize) {}

    static FAKE: AccessibleSource = AccessibleSource {
        container: fake_container,
        count: fake_count,
        item: fake_item,
        hit: fake_hit,
        current: fake_current,
        select: fake_select,
        activate: fake_activate,
    };

    fn read_bstr(value: BSTR) -> String {
        let text = unsafe { std::slice::from_raw_parts(value, SysStringLen(value) as usize) };
        let result = String::from_utf16_lossy(text);
        unsafe { SysFreeString(value) };
        result
    }

    fn row(kind: RowKind, name: &str, depth: u16, pinned: bool, expanded: bool) -> TreeRow {
        TreeRow {
            kind,
            depth,
            name: name.to_owned(),
            pinned,
            expanded,
        }
    }

    #[test]
    fn a_ten_thousand_row_list_is_counted_without_building_its_items() {
        // Break caught: accChildCount building every row's name, making each screen-reader call
        // cost O(rows) on a 10,000-note notebook.
        let provider = create_provider(std::ptr::null_mut(), &FAKE);
        ITEMS_BUILT.set(0);
        let mut count = 0;
        unsafe {
            assert_eq!((SIDEBAR_VTABLE.get_acc_child_count)(provider, &mut count), S_OK);
        }
        assert_eq!(count, 10_000);
        assert_eq!(ITEMS_BUILT.get(), 0);
        let mut name: BSTR = std::ptr::null();
        unsafe {
            assert_eq!(
                (SIDEBAR_VTABLE.get_acc_name)(provider, RawVariant::integer(10_000), &mut name),
                S_OK
            );
        }
        assert_eq!(read_bstr(name), "Note 9999");
        assert_eq!(ITEMS_BUILT.get(), 1);
        unsafe {
            assert_eq!(
                (SIDEBAR_VTABLE.get_acc_name)(provider, RawVariant::integer(10_001), &mut name),
                E_INVALIDARG
            );
            (SIDEBAR_VTABLE.release)(provider);
        }
    }

    #[test]
    fn the_container_and_children_report_their_roles_states_and_selection() {
        // Break caught: the list announced as a generic client area, or the selected row not
        // reported through accSelection.
        let provider = create_provider(std::ptr::null_mut(), &FAKE);
        let table = &SIDEBAR_VTABLE;
        unsafe {
            let mut value = RawVariant::empty();
            assert_eq!((table.get_acc_role)(provider, RawVariant::integer(0), &mut value), S_OK);
            assert_eq!(value.child_id(), Some(ROLE_SYSTEM_LIST as i32));
            assert_eq!((table.get_acc_role)(provider, RawVariant::integer(4), &mut value), S_OK);
            assert_eq!(value.child_id(), Some(ROLE_SYSTEM_LISTITEM as i32));
            assert_eq!((table.get_acc_state)(provider, RawVariant::integer(4), &mut value), S_OK);
            let state = value.child_id().unwrap() as u32;
            assert_ne!(state & STATE_SELECTED, 0);
            assert_ne!(state & STATE_SELECTABLE, 0);
            assert_eq!((table.get_acc_selection)(provider, &mut value), S_OK);
            assert_eq!(value.child_id(), Some(4));
            // Nothing has the focus in a window-less fixture.
            assert_eq!((table.get_acc_focus)(provider, &mut value), S_FALSE);
            let mut name: BSTR = std::ptr::null();
            assert_eq!((table.get_acc_name)(provider, RawVariant::integer(0), &mut name), S_OK);
            assert_eq!(read_bstr(name), "Fake notes");
            let mut next = RawVariant::empty();
            assert_eq!(
                (table.acc_navigate)(provider, NAVDIR_NEXT as i32, RawVariant::integer(1), &mut next),
                S_OK
            );
            assert_eq!(next.child_id(), Some(2));
            assert_eq!(
                (table.acc_navigate)(
                    provider,
                    NAVDIR_NEXT as i32,
                    RawVariant::integer(10_000),
                    &mut next
                ),
                S_FALSE
            );
            (table.release)(provider);
        }
    }

    #[test]
    fn tree_rows_are_outline_items_with_expansion_pin_and_unsaved_in_their_names() {
        // Break caught: a folder's expanded state missing, a pin conveyed only by the filled
        // icon, or an unsaved tab's row indistinguishable from a saved note.
        let folder = tree_item(
            &row(RowKind::Folder(PathBuf::from("Work")), "Work", 0, false, true),
            false,
            false,
            ROW,
            true,
        );
        assert_eq!(folder.role, ROLE_SYSTEM_OUTLINEITEM);
        assert_ne!(folder.state & STATE_EXPANDED, 0);
        assert_eq!(folder.state & STATE_COLLAPSED, 0);
        assert_eq!(folder.value, "0");
        let closed = tree_item(
            &row(RowKind::Folder(PathBuf::from("Old")), "Old", 1, false, false),
            false,
            false,
            ROW,
            true,
        );
        assert_ne!(closed.state & STATE_COLLAPSED, 0);
        assert_eq!(closed.value, "1");

        let pinned = tree_item(
            &row(RowKind::Note(PathBuf::from("a.md")), "a", 1, true, false),
            true,
            true,
            ROW,
            false,
        );
        assert_eq!(pinned.name, "a, pinned");
        assert_eq!(pinned.state & (STATE_EXPANDED | STATE_COLLAPSED), 0);
        assert_ne!(pinned.state & STATE_SELECTED, 0);
        assert_ne!(pinned.state & STATE_FOCUSED, 0);
        assert_ne!(pinned.state & STATE_OFFSCREEN, 0);

        let unsaved = tree_item(
            &row(RowKind::Unsaved(7), "Groceries", 0, false, false),
            false,
            true,
            ROW,
            true,
        );
        assert_eq!(unsaved.name, "Groceries, unsaved");
        assert_eq!(unsaved.state & STATE_FOCUSED, 0, "focus follows selection only");
    }

    #[test]
    fn rows_scrolled_out_of_the_list_are_offscreen() {
        // Break caught: a screen reader told that row 5,000 sits at the top of the list.
        let mut list = RowListState::new(26);
        list.set_count(100);
        list.top = 10;
        let area = RECT {
            left: 0,
            top: 38,
            right: 200,
            bottom: 38 + 26 * 5,
        };
        let (rect, visible) = row_rect(area, &list, 10);
        assert_eq!((rect.top, rect.bottom, visible), (38, 64, true));
        assert!(!row_rect(area, &list, 9).1);
        assert!(!row_rect(area, &list, 15).1);
        assert_eq!(row_rect(area, &list, 12).0.top, 38 + 52);
    }

    #[test]
    fn events_announce_selection_focus_state_and_reorders() {
        // Break caught: no event when the selection moves, so a screen reader keeps reading the
        // old row, or no state change when a folder expands or a note is pinned in place.
        let mark = |current, state, name: &str, count| AccessibleMark {
            current,
            state,
            name: name.to_owned(),
            count,
        };
        assert_eq!(
            events_between(&mark(Some(1), 0, "a", 5), &mark(Some(2), 0, "b", 5), true),
            vec![(EVENT_OBJECT_SELECTION, 3), (EVENT_OBJECT_FOCUS, 3)]
        );
        assert_eq!(
            events_between(&mark(Some(1), 0, "a", 5), &mark(Some(2), 0, "b", 5), false),
            vec![(EVENT_OBJECT_SELECTION, 3)]
        );
        assert_eq!(
            events_between(
                &mark(Some(0), STATE_COLLAPSED, "Work", 5),
                &mark(Some(0), STATE_EXPANDED, "Work", 9),
                true
            ),
            vec![(EVENT_OBJECT_REORDER, 0), (EVENT_OBJECT_STATECHANGE, 1)]
        );
        assert_eq!(
            events_between(&mark(Some(0), 0, "a", 5), &mark(Some(0), 0, "a, pinned", 5), true),
            vec![(EVENT_OBJECT_NAMECHANGE, 1)]
        );
        assert!(events_between(&mark(None, 0, "", 0), &mark(None, 0, "", 0), true).is_empty());
    }

    #[test]
    fn default_actions_follow_the_item_kind() {
        // Break caught: a folder offering "Open", or a button offering nothing to a screen
        // reader's default-action command.
        let button = button_item("Search", true, false, ROW);
        assert_eq!(default_action(&button), "Press");
        assert_ne!(button.state & STATE_PRESSED, 0);
        let folder = tree_item(
            &row(RowKind::Folder(PathBuf::from("w")), "w", 0, false, false),
            false,
            false,
            ROW,
            true,
        );
        assert_eq!(default_action(&folder), "Expand");
        let note = list_item("n", false, false, ROW, true);
        assert_eq!(default_action(&note), "Open");
    }
}
```

In the `main_window.rs` tests module, add the pure focus-order test:

```rust
    #[test]
    fn f6_order_skips_a_closed_panel_and_a_missing_sidebar() {
        // Break caught: F6 landing in a hidden panel, or getting stuck when notes mode is off.
        use super::{FocusPart, next_focus_part};
        assert_eq!(
            next_focus_part(FocusPart::Editor, false, true, true),
            FocusPart::ActivityBar
        );
        assert_eq!(
            next_focus_part(FocusPart::ActivityBar, false, true, true),
            FocusPart::Panel
        );
        assert_eq!(
            next_focus_part(FocusPart::Panel, false, true, true),
            FocusPart::Editor
        );
        assert_eq!(
            next_focus_part(FocusPart::ActivityBar, true, true, true),
            FocusPart::Editor
        );
        assert_eq!(
            next_focus_part(FocusPart::ActivityBar, false, true, false),
            FocusPart::Editor
        );
        assert_eq!(
            next_focus_part(FocusPart::Editor, false, false, false),
            FocusPart::Editor
        );
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib -- sidebar_accessibility f6_order --test-threads=1`
Expected: a compile error, because the module and `next_focus_part` don't exist yet.

- [ ] **Step 3: Write `src/window/sidebar_accessibility.rs`**

```rust
//! MSAA for the sidebar's two painted windows (spec §10). The activity bar is a toolbar of push
//! buttons. The panel is an outline of outline items (Notebook view) or a list of list items
//! (Search and Favorites), with its header buttons as push buttons.
//!
//! Children are flat child IDs answered by the provider itself, as in `accessibility.rs`. The
//! provider reads nothing but its window handle and a static `AccessibleSource`. Clients call in
//! on RPC threads, so every query goes to the window's own thread first, and App state is only
//! read there. A 10,000-row tree is counted and read one item at a time, never as a list.

use crate::library::tree::{RowKind, TreeRow};
use crate::window::accessibility::{
    AccessibleVtable, IID_IACCESSIBLE, IID_IDISPATCH, IID_IUNKNOWN, RawVariant, VariantValue,
    accessible_get_help_topic, accessible_get_ids_of_names, accessible_get_parent,
    accessible_get_type_info, accessible_get_type_info_count, accessible_invoke, allocate_bstr,
    guid_eq,
};
use crate::window::row_list::RowListState;
use std::ffi::c_void;
use std::sync::atomic::{AtomicU32, Ordering};
use windows_sys::Win32::Foundation::{
    E_INVALIDARG, E_NOINTERFACE, E_NOTIMPL, HWND, LPARAM, LRESULT, POINT, RECT, S_FALSE, S_OK,
    WPARAM,
};
use windows_sys::Win32::Graphics::Gdi::{ClientToScreen, ScreenToClient};
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::Accessibility::{
    LresultFromObject, NAVDIR_DOWN, NAVDIR_FIRSTCHILD, NAVDIR_LASTCHILD, NAVDIR_NEXT,
    NAVDIR_PREVIOUS, NAVDIR_UP, NotifyWinEvent, ROLE_SYSTEM_LISTITEM, ROLE_SYSTEM_OUTLINEITEM,
    ROLE_SYSTEM_PANE, ROLE_SYSTEM_PUSHBUTTON, SELFLAG_TAKEFOCUS, SELFLAG_TAKESELECTION,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EVENT_OBJECT_FOCUS, EVENT_OBJECT_NAMECHANGE, EVENT_OBJECT_REORDER, EVENT_OBJECT_SELECTION,
    EVENT_OBJECT_STATECHANGE, GUITHREADINFO, GetClientRect, GetGUIThreadInfo, GetWindowRect,
    GetWindowThreadProcessId, OBJID_CLIENT, PostMessageW, SendMessageW, WM_APP, WM_LBUTTONDOWN,
    WM_LBUTTONUP,
};
use windows_sys::core::{BSTR, GUID, HRESULT};

#[cfg(test)]
use windows_sys::Win32::UI::Accessibility::ROLE_SYSTEM_LIST;

/// Sent to a sidebar window with a `*mut Call` to run one query on the window's thread.
pub(crate) const WM_FASTPAD_SIDEBAR_ACCESSIBLE: u32 = WM_APP + 0x60;
/// Posted to a sidebar window: `wparam` is the child index, and `lparam` is one of `ACTION_*`.
pub(crate) const WM_FASTPAD_SIDEBAR_ACTION: u32 = WM_APP + 0x61;
pub(crate) const ACTION_ACTIVATE: LPARAM = 0;
pub(crate) const ACTION_SELECT: LPARAM = 1;
pub(crate) const ACTION_FOCUS: LPARAM = 2;

pub(crate) const STATE_SELECTED: u32 = 0x0000_0002;
pub(crate) const STATE_FOCUSED: u32 = 0x0000_0004;
pub(crate) const STATE_PRESSED: u32 = 0x0000_0008;
pub(crate) const STATE_EXPANDED: u32 = 0x0000_0200;
pub(crate) const STATE_COLLAPSED: u32 = 0x0000_0400;
pub(crate) const STATE_OFFSCREEN: u32 = 0x0001_0000;
pub(crate) const STATE_FOCUSABLE: u32 = 0x0010_0000;
pub(crate) const STATE_SELECTABLE: u32 = 0x0020_0000;
const MK_LBUTTON: WPARAM = 0x0001;

/// One MSAA child of a sidebar window, in the window's client coordinates.
#[derive(Clone)]
pub(crate) struct AccessibleItem {
    pub name: String,
    pub role: u32,
    pub state: u32,
    pub rect: RECT,
    /// An outline item's level (0 for the root's children), as tree views report it. Empty
    /// otherwise.
    pub value: String,
}

impl std::fmt::Debug for AccessibleItem {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AccessibleItem")
            .field("name", &self.name)
            .field("role", &self.role)
            .field("state", &format_args!("{:#x}", self.state))
            .field("value", &self.value)
            .finish_non_exhaustive()
    }
}

/// What a window's provider asks, always on the window's own thread.
pub(crate) struct AccessibleSource {
    /// The window itself: its name and role.
    pub container: fn(HWND) -> (String, u32),
    pub count: fn(HWND) -> usize,
    pub item: fn(HWND, usize) -> Option<AccessibleItem>,
    /// The child under a client point.
    pub hit: fn(HWND, POINT) -> Option<usize>,
    /// The selected child: the focused one while the window has the keyboard focus.
    pub current: fn(HWND) -> Option<usize>,
    /// Makes a child current without activating it.
    pub select: fn(HWND, usize),
    /// A child's default action.
    pub activate: fn(HWND, usize),
}

/// A view's children as MSAA sees them. Every method gets the panel's client rectangle and DPI.
pub(crate) trait AccessibleView {
    fn accessible_count(&self, client: RECT, dpi: u32) -> usize;
    fn accessible_item(
        &self,
        index: usize,
        client: RECT,
        dpi: u32,
        focused: bool,
    ) -> Option<AccessibleItem>;
    fn accessible_hit(&self, point: POINT, client: RECT, dpi: u32) -> Option<usize>;
    fn accessible_current(&self, client: RECT, dpi: u32) -> Option<usize>;
    fn accessible_select(&mut self, index: usize, client: RECT, dpi: u32);
}

pub(crate) fn button_item(name: &str, pressed: bool, focused: bool, rect: RECT) -> AccessibleItem {
    let mut state = STATE_FOCUSABLE;
    if pressed {
        state |= STATE_PRESSED;
    }
    if focused {
        state |= STATE_FOCUSED;
    }
    AccessibleItem {
        name: name.to_owned(),
        role: ROLE_SYSTEM_PUSHBUTTON,
        state,
        rect,
        value: String::new(),
    }
}

fn row_state(selected: bool, focused: bool, visible: bool) -> u32 {
    let mut state = STATE_SELECTABLE | STATE_FOCUSABLE;
    if selected {
        state |= STATE_SELECTED;
        if focused {
            state |= STATE_FOCUSED;
        }
    }
    if !visible {
        state |= STATE_OFFSCREEN;
    }
    state
}

/// A search result, favorite or recent notebook. `focused` means the list has the focus: only
/// the selected item is then focused.
pub(crate) fn list_item(
    name: &str,
    selected: bool,
    focused: bool,
    rect: RECT,
    visible: bool,
) -> AccessibleItem {
    AccessibleItem {
        name: name.to_owned(),
        role: ROLE_SYSTEM_LISTITEM,
        state: row_state(selected, focused, visible),
        rect,
        value: String::new(),
    }
}

/// A Notebook-view row. The name carries ", pinned" or ", unsaved", so neither is conveyed by the
/// icon alone.
pub(crate) fn tree_item(
    row: &TreeRow,
    selected: bool,
    focused: bool,
    rect: RECT,
    visible: bool,
) -> AccessibleItem {
    let mut name = row.name.clone();
    if row.pinned {
        name.push_str(", pinned");
    }
    if matches!(row.kind, RowKind::Unsaved(_)) {
        name.push_str(", unsaved");
    }
    let mut state = row_state(selected, focused, visible);
    if matches!(row.kind, RowKind::Folder(_)) {
        state |= if row.expanded {
            STATE_EXPANDED
        } else {
            STATE_COLLAPSED
        };
    }
    AccessibleItem {
        name,
        role: ROLE_SYSTEM_OUTLINEITEM,
        state,
        rect,
        value: row.depth.to_string(),
    }
}

/// Where row `index` of `list` is, scrolled or not, and whether any of it shows in `area`.
pub(crate) fn row_rect(area: RECT, list: &RowListState, index: usize) -> (RECT, bool) {
    let offset = (index as i64 - list.top as i64) * i64::from(list.row_height);
    let top = (i64::from(area.top) + offset).clamp(i64::from(i32::MIN / 2), i64::from(i32::MAX / 2))
        as i32;
    let rect = RECT {
        left: area.left,
        top,
        right: area.right,
        bottom: top.saturating_add(list.row_height),
    };
    let visible = rect.bottom > area.top && rect.top < area.bottom;
    (rect, visible)
}

pub(crate) fn default_action(item: &AccessibleItem) -> &'static str {
    if item.role == ROLE_SYSTEM_PUSHBUTTON {
        "Press"
    } else if item.state & STATE_EXPANDED != 0 {
        "Collapse"
    } else if item.state & STATE_COLLAPSED != 0 {
        "Expand"
    } else {
        "Open"
    }
}

/// Presses an item the way a click on its center does.
pub(crate) fn click_item(hwnd: HWND, rect: RECT) {
    let x = (rect.left + rect.right) / 2;
    let y = (rect.top + rect.bottom) / 2;
    let point = (x as u16 as u32 | ((y as u16 as u32) << 16)) as LPARAM;
    unsafe {
        SendMessageW(hwnd, WM_LBUTTONDOWN, MK_LBUTTON, point);
        SendMessageW(hwnd, WM_LBUTTONUP, 0, point);
    }
}

/// What screen readers last knew about the current child, compared across a change.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct AccessibleMark {
    pub current: Option<usize>,
    /// Focus and scrolling aside: those raise their own events.
    pub state: u32,
    pub name: String,
    pub count: usize,
}

impl AccessibleMark {
    /// Reads the mark on the window's own thread.
    pub(crate) fn read(hwnd: HWND, source: &AccessibleSource) -> Self {
        let current = (source.current)(hwnd);
        let item = current.and_then(|index| (source.item)(hwnd, index));
        Self {
            current,
            state: item
                .as_ref()
                .map_or(0, |item| item.state & !(STATE_FOCUSED | STATE_OFFSCREEN)),
            name: item.map(|item| item.name).unwrap_or_default(),
            count: (source.count)(hwnd),
        }
    }
}

/// The win events a change from `before` to `after` raises, as (event, child id).
pub(crate) fn events_between(
    before: &AccessibleMark,
    after: &AccessibleMark,
    focused: bool,
) -> Vec<(u32, i32)> {
    let mut events = Vec::new();
    if before.count != after.count {
        events.push((EVENT_OBJECT_REORDER, 0));
    }
    let Some(current) = after.current else {
        return events;
    };
    let id = current as i32 + 1;
    if before.current != after.current {
        events.push((EVENT_OBJECT_SELECTION, id));
        if focused {
            events.push((EVENT_OBJECT_FOCUS, id));
        }
    } else {
        if before.state != after.state {
            events.push((EVENT_OBJECT_STATECHANGE, id));
        }
        if before.name != after.name {
            events.push((EVENT_OBJECT_NAMECHANGE, id));
        }
    }
    events
}

pub(crate) fn raise(hwnd: HWND, events: &[(u32, i32)]) {
    for &(event, child) in events {
        unsafe {
            NotifyWinEvent(event, hwnd, OBJID_CLIENT, child);
        }
    }
}

/// One event for child `index` (0-based), or for the window itself with `None`.
pub(crate) fn notify(event: u32, hwnd: HWND, index: Option<usize>) {
    raise(hwnd, &[(event, index.map_or(0, |index| index as i32 + 1))]);
}

#[repr(C)]
struct SidebarAccessible {
    vtable: &'static AccessibleVtable,
    references: AtomicU32,
    hwnd: HWND,
    source: &'static AccessibleSource,
}

pub(crate) static SIDEBAR_VTABLE: AccessibleVtable = AccessibleVtable {
    query_interface,
    add_ref,
    release,
    get_type_info_count: accessible_get_type_info_count,
    get_type_info: accessible_get_type_info,
    get_ids_of_names: accessible_get_ids_of_names,
    invoke: accessible_invoke,
    get_acc_parent: accessible_get_parent,
    get_acc_child_count: child_count,
    get_acc_child: child,
    get_acc_name: name,
    get_acc_value: value,
    get_acc_description: empty_text,
    get_acc_role: role,
    get_acc_state: state,
    get_acc_help: empty_text,
    get_acc_help_topic: accessible_get_help_topic,
    get_acc_keyboard_shortcut: empty_text,
    get_acc_focus: focus,
    get_acc_selection: selection,
    get_acc_default_action: default_action_text,
    acc_select: select,
    acc_location: location,
    acc_navigate: navigate,
    acc_hit_test: hit_test,
    acc_do_default_action: do_default_action,
    put_acc_name: put_text,
    put_acc_value: put_text,
};

fn create_provider(hwnd: HWND, source: &'static AccessibleSource) -> *mut c_void {
    Box::into_raw(Box::new(SidebarAccessible {
        vtable: &SIDEBAR_VTABLE,
        references: AtomicU32::new(1),
        hwnd,
        source,
    }))
    .cast()
}

/// Answers `WM_GETOBJECT(OBJID_CLIENT)` for a sidebar window. Call it with nothing of the App
/// borrowed: a client may call back in while `LresultFromObject` runs.
pub(crate) unsafe fn object_result(
    hwnd: HWND,
    source: &'static AccessibleSource,
    wparam: WPARAM,
) -> LRESULT {
    let provider = create_provider(hwnd, source);
    let result = unsafe { LresultFromObject(&IID_IACCESSIBLE, wparam, provider) };
    unsafe { release(provider) };
    result
}

#[cfg(test)]
pub(crate) fn create_for_test(hwnd: HWND, source: &'static AccessibleSource) -> *mut c_void {
    create_provider(hwnd, source)
}

#[derive(Clone, Copy)]
enum Query {
    Container,
    Count,
    Item(usize),
    Hit(POINT),
    Current,
}

enum Answer {
    Container(String, u32),
    Count(usize),
    Item(Option<AccessibleItem>),
    Index(Option<usize>),
}

/// A query sent to the window's thread; the window procedure fills in `answer`.
struct Call {
    source: &'static AccessibleSource,
    query: Query,
    answer: Option<Answer>,
}

fn evaluate(source: &AccessibleSource, hwnd: HWND, query: Query) -> Answer {
    match query {
        Query::Container => {
            let (name, role) = (source.container)(hwnd);
            Answer::Container(name, role)
        }
        Query::Count => Answer::Count((source.count)(hwnd)),
        Query::Item(index) => Answer::Item((source.item)(hwnd, index)),
        Query::Hit(point) => Answer::Index((source.hit)(hwnd, point)),
        Query::Current => Answer::Index((source.current)(hwnd)),
    }
}

/// The window procedure's `WM_FASTPAD_SIDEBAR_ACCESSIBLE` handler.
pub(crate) unsafe fn answer(hwnd: HWND, lparam: LPARAM) -> LRESULT {
    let call = unsafe { &mut *(lparam as *mut Call) };
    call.answer = Some(evaluate(call.source, hwnd, call.query));
    1
}

/// The window procedure's `WM_FASTPAD_SIDEBAR_ACTION` handler.
pub(crate) fn run_action(hwnd: HWND, source: &AccessibleSource, wparam: WPARAM, lparam: LPARAM) {
    let index = wparam;
    // The children may have changed since the client asked; an index past the end is dropped.
    if index >= (source.count)(hwnd) {
        return;
    }
    if lparam == ACTION_ACTIVATE {
        (source.activate)(hwnd, index);
        return;
    }
    (source.select)(hwnd, index);
    if lparam == ACTION_FOCUS {
        unsafe {
            SetFocus(hwnd);
        }
    }
}

fn on_window_thread(hwnd: HWND) -> bool {
    unsafe { GetWindowThreadProcessId(hwnd, std::ptr::null_mut()) == GetCurrentThreadId() }
}

unsafe fn provider<'a>(this: *mut c_void) -> &'a SidebarAccessible {
    unsafe { &*this.cast::<SidebarAccessible>() }
}

fn ask(item: &SidebarAccessible, query: Query) -> Answer {
    if item.hwnd.is_null() || on_window_thread(item.hwnd) {
        return evaluate(item.source, item.hwnd, query);
    }
    let mut call = Call {
        source: item.source,
        query,
        answer: None,
    };
    unsafe {
        SendMessageW(
            item.hwnd,
            WM_FASTPAD_SIDEBAR_ACCESSIBLE,
            0,
            &mut call as *mut Call as LPARAM,
        );
    }
    // A destroyed window answers nothing: report no children.
    call.answer.unwrap_or(match query {
        Query::Container => Answer::Container(String::new(), ROLE_SYSTEM_PANE),
        Query::Count => Answer::Count(0),
        Query::Item(_) => Answer::Item(None),
        Query::Hit(_) | Query::Current => Answer::Index(None),
    })
}

fn count_of(item: &SidebarAccessible) -> usize {
    match ask(item, Query::Count) {
        Answer::Count(count) => count,
        _ => 0,
    }
}

fn index_answer(item: &SidebarAccessible, query: Query) -> Option<usize> {
    match ask(item, query) {
        Answer::Index(index) => index,
        _ => None,
    }
}

/// `Some(None)` for the window itself, `Some(Some(item))` for a child, `None` for a bad ID.
fn target(item: &SidebarAccessible, child: &RawVariant) -> Option<Option<AccessibleItem>> {
    match child.child_id()? {
        0 => Some(None),
        id if id > 0 => match ask(item, Query::Item(id as usize - 1)) {
            Answer::Item(Some(found)) => Some(Some(found)),
            _ => None,
        },
        _ => None,
    }
}

fn container(item: &SidebarAccessible) -> (String, u32) {
    match ask(item, Query::Container) {
        Answer::Container(name, role) => (name, role),
        _ => (String::new(), ROLE_SYSTEM_PANE),
    }
}

/// Asks the window's own thread: `GetFocus` on an RPC thread reports that thread's empty focus.
fn has_focus(item: &SidebarAccessible) -> bool {
    if item.hwnd.is_null() {
        return false;
    }
    let thread = unsafe { GetWindowThreadProcessId(item.hwnd, std::ptr::null_mut()) };
    let mut info = GUITHREADINFO {
        cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
        ..Default::default()
    };
    thread != 0
        && unsafe { GetGUIThreadInfo(thread, &mut info) } != 0
        && info.hwndFocus == item.hwnd
}

unsafe extern "system" fn query_interface(
    this: *mut c_void,
    iid: *const GUID,
    output: *mut *mut c_void,
) -> HRESULT {
    if iid.is_null() || output.is_null() {
        return E_INVALIDARG;
    }
    let requested = unsafe { *iid };
    if guid_eq(&requested, &IID_IUNKNOWN)
        || guid_eq(&requested, &IID_IDISPATCH)
        || guid_eq(&requested, &IID_IACCESSIBLE)
    {
        unsafe {
            *output = this;
            add_ref(this);
        }
        S_OK
    } else {
        unsafe { *output = std::ptr::null_mut() };
        E_NOINTERFACE
    }
}

unsafe extern "system" fn add_ref(this: *mut c_void) -> u32 {
    unsafe { provider(this) }
        .references
        .fetch_add(1, Ordering::Relaxed)
        + 1
}

unsafe extern "system" fn release(this: *mut c_void) -> u32 {
    let remaining = unsafe { provider(this) }
        .references
        .fetch_sub(1, Ordering::Release)
        - 1;
    if remaining == 0 {
        std::sync::atomic::fence(Ordering::Acquire);
        drop(unsafe { Box::from_raw(this.cast::<SidebarAccessible>()) });
    }
    remaining
}

unsafe extern "system" fn child_count(this: *mut c_void, count: *mut i32) -> HRESULT {
    if count.is_null() {
        return E_INVALIDARG;
    }
    let children = count_of(unsafe { provider(this) });
    unsafe { *count = i32::try_from(children).unwrap_or(i32::MAX) };
    S_OK
}

unsafe extern "system" fn child(
    this: *mut c_void,
    child: RawVariant,
    output: *mut *mut c_void,
) -> HRESULT {
    if output.is_null() {
        return E_INVALIDARG;
    }
    unsafe { *output = std::ptr::null_mut() };
    match child.child_id() {
        Some(id) if id > 0 && (id as usize) <= count_of(unsafe { provider(this) }) => S_FALSE,
        _ => E_INVALIDARG,
    }
}

unsafe extern "system" fn name(this: *mut c_void, child: RawVariant, output: *mut BSTR) -> HRESULT {
    let item = unsafe { provider(this) };
    match target(item, &child) {
        Some(None) => unsafe { allocate_bstr(&container(item).0, output) },
        Some(Some(found)) => unsafe { allocate_bstr(&found.name, output) },
        None => E_INVALIDARG,
    }
}

unsafe extern "system" fn value(
    this: *mut c_void,
    child: RawVariant,
    output: *mut BSTR,
) -> HRESULT {
    match target(unsafe { provider(this) }, &child) {
        Some(None) => unsafe { allocate_bstr("", output) },
        Some(Some(found)) => unsafe { allocate_bstr(&found.value, output) },
        None => E_INVALIDARG,
    }
}

unsafe extern "system" fn empty_text(
    this: *mut c_void,
    child: RawVariant,
    output: *mut BSTR,
) -> HRESULT {
    match target(unsafe { provider(this) }, &child) {
        Some(_) => unsafe { allocate_bstr("", output) },
        None => E_INVALIDARG,
    }
}

unsafe extern "system" fn role(
    this: *mut c_void,
    child: RawVariant,
    output: *mut RawVariant,
) -> HRESULT {
    if output.is_null() {
        return E_INVALIDARG;
    }
    let item = unsafe { provider(this) };
    let role = match target(item, &child) {
        Some(None) => container(item).1,
        Some(Some(found)) => found.role,
        None => return E_INVALIDARG,
    };
    unsafe { *output = RawVariant::integer(role as i32) };
    S_OK
}

unsafe extern "system" fn state(
    this: *mut c_void,
    child: RawVariant,
    output: *mut RawVariant,
) -> HRESULT {
    if output.is_null() {
        return E_INVALIDARG;
    }
    let item = unsafe { provider(this) };
    let state = match target(item, &child) {
        Some(None) => {
            STATE_FOCUSABLE
                | if has_focus(item) {
                    STATE_FOCUSED
                } else {
                    0
                }
        }
        Some(Some(found)) => found.state,
        None => return E_INVALIDARG,
    };
    unsafe { *output = RawVariant::integer(state as i32) };
    S_OK
}

unsafe extern "system" fn focus(this: *mut c_void, output: *mut RawVariant) -> HRESULT {
    if output.is_null() {
        return E_INVALIDARG;
    }
    let item = unsafe { provider(this) };
    if !has_focus(item) {
        unsafe { *output = RawVariant::empty() };
        return S_FALSE;
    }
    let id = index_answer(item, Query::Current).map_or(0, |index| index as i32 + 1);
    unsafe { *output = RawVariant::integer(id) };
    S_OK
}

unsafe extern "system" fn selection(this: *mut c_void, output: *mut RawVariant) -> HRESULT {
    if output.is_null() {
        return E_INVALIDARG;
    }
    match index_answer(unsafe { provider(this) }, Query::Current) {
        Some(index) => {
            unsafe { *output = RawVariant::integer(index as i32 + 1) };
            S_OK
        }
        None => {
            unsafe { *output = RawVariant::empty() };
            S_FALSE
        }
    }
}

unsafe extern "system" fn default_action_text(
    this: *mut c_void,
    child: RawVariant,
    output: *mut BSTR,
) -> HRESULT {
    match target(unsafe { provider(this) }, &child) {
        Some(Some(found)) => unsafe { allocate_bstr(default_action(&found), output) },
        Some(None) => unsafe { allocate_bstr("", output) },
        None => E_INVALIDARG,
    }
}

/// Posts an action for child `id`, or runs it at once in a window-less test fixture.
fn post_action(item: &SidebarAccessible, id: i32, action: LPARAM) -> HRESULT {
    if id <= 0 || id as usize > count_of(item) {
        return E_INVALIDARG;
    }
    let index = id as usize - 1;
    if item.hwnd.is_null() {
        run_action(item.hwnd, item.source, index, action);
        return S_OK;
    }
    let posted = unsafe { PostMessageW(item.hwnd, WM_FASTPAD_SIDEBAR_ACTION, index, action) };
    if posted != 0 { S_OK } else { E_INVALIDARG }
}

unsafe extern "system" fn select(this: *mut c_void, flags: i32, child: RawVariant) -> HRESULT {
    let flags = flags as u32;
    if flags & (SELFLAG_TAKEFOCUS | SELFLAG_TAKESELECTION) == 0 {
        return E_INVALIDARG;
    }
    let Some(id) = child.child_id() else {
        return E_INVALIDARG;
    };
    let action = if flags & SELFLAG_TAKEFOCUS != 0 {
        ACTION_FOCUS
    } else {
        ACTION_SELECT
    };
    post_action(unsafe { provider(this) }, id, action)
}

unsafe extern "system" fn location(
    this: *mut c_void,
    left: *mut i32,
    top: *mut i32,
    width: *mut i32,
    height: *mut i32,
    child: RawVariant,
) -> HRESULT {
    if left.is_null() || top.is_null() || width.is_null() || height.is_null() {
        return E_INVALIDARG;
    }
    let item = unsafe { provider(this) };
    let rect = match target(item, &child) {
        Some(None) => {
            let mut window = RECT::default();
            if item.hwnd.is_null() || unsafe { GetWindowRect(item.hwnd, &mut window) } == 0 {
                return S_FALSE;
            }
            window
        }
        Some(Some(found)) => {
            let mut origin = POINT { x: 0, y: 0 };
            if !item.hwnd.is_null() && unsafe { ClientToScreen(item.hwnd, &mut origin) } == 0 {
                return S_FALSE;
            }
            RECT {
                left: found.rect.left + origin.x,
                top: found.rect.top + origin.y,
                right: found.rect.right + origin.x,
                bottom: found.rect.bottom + origin.y,
            }
        }
        None => return E_INVALIDARG,
    };
    unsafe {
        *left = rect.left;
        *top = rect.top;
        *width = rect.right - rect.left;
        *height = rect.bottom - rect.top;
    }
    S_OK
}

unsafe extern "system" fn navigate(
    this: *mut c_void,
    direction: i32,
    start: RawVariant,
    output: *mut RawVariant,
) -> HRESULT {
    if output.is_null() {
        return E_INVALIDARG;
    }
    let Some(id) = start.child_id() else {
        return E_INVALIDARG;
    };
    let count = i32::try_from(count_of(unsafe { provider(this) })).unwrap_or(i32::MAX);
    if id < 0 || id > count {
        return E_INVALIDARG;
    }
    let target = match (direction as u32, id) {
        (NAVDIR_FIRSTCHILD, 0) if count > 0 => Some(1),
        (NAVDIR_LASTCHILD, 0) if count > 0 => Some(count),
        (NAVDIR_NEXT | NAVDIR_DOWN, value) if value > 0 && value < count => Some(value + 1),
        (NAVDIR_PREVIOUS | NAVDIR_UP, value) if value > 1 => Some(value - 1),
        (
            NAVDIR_FIRSTCHILD | NAVDIR_LASTCHILD | NAVDIR_NEXT | NAVDIR_DOWN | NAVDIR_PREVIOUS
            | NAVDIR_UP,
            _,
        ) => None,
        _ => return E_INVALIDARG,
    };
    unsafe { *output = target.map_or_else(RawVariant::empty, RawVariant::integer) };
    if target.is_some() { S_OK } else { S_FALSE }
}

unsafe extern "system" fn hit_test(
    this: *mut c_void,
    x: i32,
    y: i32,
    output: *mut RawVariant,
) -> HRESULT {
    if output.is_null() {
        return E_INVALIDARG;
    }
    let item = unsafe { provider(this) };
    let mut point = POINT { x, y };
    let mut client = RECT::default();
    if item.hwnd.is_null()
        || unsafe { ScreenToClient(item.hwnd, &mut point) } == 0
        || unsafe { GetClientRect(item.hwnd, &mut client) } == 0
        || point.x < client.left
        || point.x >= client.right
        || point.y < client.top
        || point.y >= client.bottom
    {
        unsafe { *output = RawVariant::empty() };
        return S_FALSE;
    }
    let id = index_answer(item, Query::Hit(point)).map_or(0, |index| index as i32 + 1);
    unsafe { *output = RawVariant::integer(id) };
    S_OK
}

unsafe extern "system" fn do_default_action(this: *mut c_void, child: RawVariant) -> HRESULT {
    let Some(id) = child.child_id() else {
        return E_INVALIDARG;
    };
    post_action(unsafe { provider(this) }, id, ACTION_ACTIVATE)
}

unsafe extern "system" fn put_text(
    _this: *mut c_void,
    _child: RawVariant,
    _value: BSTR,
) -> HRESULT {
    E_NOTIMPL
}
```

Append the Step 1 test module.

- [ ] **Step 4: The views' `AccessibleView` implementations**

In `src/window/favorites_view.rs`, add:

```rust
impl crate::window::sidebar_accessibility::AccessibleView for FavoritesView {
    /// The header button, one item per favorite, then the footer row.
    fn accessible_count(&self, _client: RECT, _dpi: u32) -> usize {
        1 + self.rows.len() + 1
    }

    fn accessible_item(
        &self,
        index: usize,
        client: RECT,
        dpi: u32,
        focused: bool,
    ) -> Option<crate::window::sidebar_accessibility::AccessibleItem> {
        use crate::window::sidebar_accessibility::{button_item, list_item, row_rect};
        if index == 0 {
            return Some(button_item(
                OPEN_NOTEBOOK,
                false,
                false,
                Self::header_button(client, dpi),
            ));
        }
        let row = index - 1;
        if row > self.rows.len() {
            return None;
        }
        let (rect, visible) = row_rect(self.list_area(client, dpi), &self.list, row);
        let selected = self.list.selected == Some(row);
        let name = match self.rows.get(row) {
            Some(favorite) => {
                let mut name = favorite.name.clone();
                if let Some(hint) = &favorite.hint {
                    name.push_str(", ");
                    name.push_str(hint);
                }
                if favorite.open {
                    name.push_str(", open");
                }
                name
            }
            None => OPEN_NOTEBOOK.to_owned(),
        };
        Some(list_item(&name, selected, focused, rect, visible))
    }

    fn accessible_hit(&self, point: POINT, client: RECT, dpi: u32) -> Option<usize> {
        if inside(Self::header_button(client, dpi), point) {
            return Some(0);
        }
        let area = self.list_area(client, dpi);
        if !inside(area, point) {
            return None;
        }
        self.list.row_at(point.y - area.top).map(|row| row + 1)
    }

    fn accessible_current(&self, _client: RECT, _dpi: u32) -> Option<usize> {
        self.list.selected.map(|row| row + 1)
    }

    fn accessible_select(&mut self, index: usize, client: RECT, dpi: u32) {
        if let Some(row) = index.checked_sub(1).filter(|&row| row <= self.rows.len()) {
            let area = self.list_area(client, dpi);
            self.list.select(row, area.bottom - area.top);
        }
    }
}
```

In `src/window/search_view.rs`, add:

```rust
impl crate::window::sidebar_accessibility::AccessibleView for SearchView {
    /// One list item per result. The search box is a real `Edit` with its own MSAA object.
    fn accessible_count(&self, _client: RECT, _dpi: u32) -> usize {
        self.results.len()
    }

    fn accessible_item(
        &self,
        index: usize,
        client: RECT,
        dpi: u32,
        focused: bool,
    ) -> Option<crate::window::sidebar_accessibility::AccessibleItem> {
        let result = self.results.get(index)?;
        let (rect, visible) = crate::window::sidebar_accessibility::row_rect(
            self.list_area(client, dpi),
            &self.list,
            index,
        );
        let name = if result.folder.is_empty() {
            result.name.clone()
        } else {
            format!("{}, {}", result.name, result.folder)
        };
        Some(crate::window::sidebar_accessibility::list_item(
            &name,
            self.list.selected == Some(index),
            focused,
            rect,
            visible,
        ))
    }

    fn accessible_hit(&self, point: POINT, client: RECT, dpi: u32) -> Option<usize> {
        self.row_under(point, client, dpi)
    }

    fn accessible_current(&self, _client: RECT, _dpi: u32) -> Option<usize> {
        self.list.selected
    }

    fn accessible_select(&mut self, index: usize, client: RECT, dpi: u32) {
        if index < self.results.len() {
            let area = self.list_area(client, dpi);
            self.list.select(index, area.bottom - area.top);
        }
    }
}
```

In `src/window/notebook_view.rs`, delete the `#[allow(dead_code, reason = "Task 13's MSAA provider is the first reader")]` attribute on Task 10's accessor `impl NotebookView` block, and add the following. It uses those accessors.

```rust
impl crate::window::sidebar_accessibility::AccessibleView for NotebookView {
    /// Push buttons first, then the RECENT notebooks (no-notebook state), then the tree rows.
    fn accessible_count(&self, client: RECT, dpi: u32) -> usize {
        self.buttons(client, dpi).len() + self.recent_rows(client, dpi).len() + self.rows().len()
    }

    fn accessible_item(
        &self,
        index: usize,
        client: RECT,
        dpi: u32,
        focused: bool,
    ) -> Option<crate::window::sidebar_accessibility::AccessibleItem> {
        use crate::window::sidebar_accessibility::{button_item, list_item, row_rect, tree_item};
        let buttons = self.buttons(client, dpi);
        if let Some((name, rect)) = buttons.get(index) {
            return Some(button_item(name, false, false, *rect));
        }
        let index = index - buttons.len();
        let recent = self.recent_rows(client, dpi);
        if let Some((name, rect)) = recent.get(index) {
            return Some(list_item(name, false, false, *rect, true));
        }
        let index = index - recent.len();
        let row = self.rows().get(index)?;
        let (rect, visible) = row_rect(self.list_area(client, dpi), self.list(), index);
        Some(tree_item(
            row,
            self.list().selected == Some(index),
            focused,
            rect,
            visible,
        ))
    }

    fn accessible_hit(&self, point: POINT, client: RECT, dpi: u32) -> Option<usize> {
        let inside = |rect: &RECT| {
            point.x >= rect.left && point.x < rect.right && point.y >= rect.top && point.y < rect.bottom
        };
        let buttons = self.buttons(client, dpi);
        if let Some(index) = buttons.iter().position(|(_, rect)| inside(rect)) {
            return Some(index);
        }
        let recent = self.recent_rows(client, dpi);
        if let Some(index) = recent.iter().position(|(_, rect)| inside(rect)) {
            return Some(buttons.len() + index);
        }
        let area = self.list_area(client, dpi);
        if !inside(&area) {
            return None;
        }
        self.list()
            .row_at(point.y - area.top)
            .filter(|&row| row < self.rows().len())
            .map(|row| buttons.len() + recent.len() + row)
    }

    fn accessible_current(&self, client: RECT, dpi: u32) -> Option<usize> {
        let offset = self.buttons(client, dpi).len() + self.recent_rows(client, dpi).len();
        // In the no-notebook state the list's selection is a RECENT row, not a tree row.
        self.list()
            .selected
            .filter(|&row| row < self.rows().len())
            .map(|row| offset + row)
    }

    fn accessible_select(&mut self, index: usize, client: RECT, dpi: u32) {
        let offset = self.buttons(client, dpi).len() + self.recent_rows(client, dpi).len();
        let Some(row) = index.checked_sub(offset).filter(|&row| row < self.rows().len()) else {
            return;
        };
        let area = self.list_area(client, dpi);
        self.list_mut().select(row, area.bottom - area.top);
    }
}
```

Add `POINT` and `RECT` to the file's `windows_sys::Win32::Foundation` import if they're missing.

- [ ] **Step 5: The panel provider, events and Esc in `side_panel.rs`**

Add `pub(crate) bar_focus: usize,` to `Sidebar`, initialized to `0` in `create`.

Add:

```rust
use crate::window::sidebar_accessibility::{
    self, AccessibleItem, AccessibleMark, AccessibleSource, AccessibleView,
};

/// Runs `f` on the view the panel shows, with its client rectangle, DPI and focus.
fn with_accessible_view<R>(
    panel: HWND,
    f: impl FnOnce(&mut dyn AccessibleView, RECT, u32, bool) -> R,
) -> Option<R> {
    let main = unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetParent(panel) };
    let view = current_view(main);
    let mut client = RECT::default();
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect(panel, &mut client);
    }
    let dpi = unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForWindow(panel) }.max(96);
    let focused = unsafe { windows_sys::Win32::UI::Input::KeyboardAndMouse::GetFocus() } == panel;
    let mut app = unsafe { super::main_window::app_ptr(main) }?;
    let sidebar = unsafe { app.as_mut() }.sidebar.as_mut()?;
    match view {
        SidebarView::Notebook => Some(f(&mut sidebar.notebook, client, dpi, focused)),
        SidebarView::Search => Some(f(&mut sidebar.search, client, dpi, focused)),
        SidebarView::Favorites => Some(f(&mut sidebar.favorites, client, dpi, focused)),
        SidebarView::Hidden => None,
    }
}

/// How many MSAA children the panel has: one per header button and visible (flattened) row.
pub(crate) fn accessible_item_count(panel: HWND) -> usize {
    with_accessible_view(panel, |view, client, dpi, _| view.accessible_count(client, dpi))
        .unwrap_or(0)
}

/// The panel's MSAA child `index` (0-based), built on its own.
pub(crate) fn accessible_item(panel: HWND, index: usize) -> Option<AccessibleItem> {
    with_accessible_view(panel, |view, client, dpi, focused| {
        view.accessible_item(index, client, dpi, focused)
    })
    .flatten()
}

fn accessible_container(panel: HWND) -> (String, u32) {
    use windows_sys::Win32::UI::Accessibility::{
        ROLE_SYSTEM_LIST, ROLE_SYSTEM_OUTLINE, ROLE_SYSTEM_PANE,
    };
    let main = unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetParent(panel) };
    match current_view(main) {
        SidebarView::Notebook => {
            let name = super::library_host::folder(main)
                .and_then(|folder| {
                    folder
                        .file_name()
                        .map(|name| format!("Notebook {}", name.to_string_lossy()))
                })
                .unwrap_or_else(|| "Notebook".to_owned());
            (name, ROLE_SYSTEM_OUTLINE)
        }
        SidebarView::Search => ("Search results".to_owned(), ROLE_SYSTEM_LIST),
        SidebarView::Favorites => ("Favorite notebooks".to_owned(), ROLE_SYSTEM_LIST),
        SidebarView::Hidden => ("Side panel".to_owned(), ROLE_SYSTEM_PANE),
    }
}

fn accessible_hit(panel: HWND, point: POINT) -> Option<usize> {
    with_accessible_view(panel, |view, client, dpi, _| {
        view.accessible_hit(point, client, dpi)
    })
    .flatten()
}

fn accessible_current(panel: HWND) -> Option<usize> {
    with_accessible_view(panel, |view, client, dpi, _| view.accessible_current(client, dpi))
        .flatten()
}

fn accessible_select(panel: HWND, index: usize) {
    with_accessible_view(panel, |view, client, dpi, _| {
        view.accessible_select(index, client, dpi)
    });
    unsafe {
        windows_sys::Win32::Graphics::Gdi::InvalidateRect(panel, std::ptr::null(), 0);
    }
}

/// A child's default action: a click on its center, after scrolling it into view.
fn accessible_activate(panel: HWND, index: usize) {
    let Some(mut item) = accessible_item(panel, index) else {
        return;
    };
    if item.state & sidebar_accessibility::STATE_OFFSCREEN != 0 {
        accessible_select(panel, index);
        let Some(shown) = accessible_item(panel, index) else {
            return;
        };
        item = shown;
    }
    sidebar_accessibility::click_item(panel, item.rect);
}

pub(crate) static PANEL_ACCESSIBLE: AccessibleSource = AccessibleSource {
    container: accessible_container,
    count: accessible_item_count,
    item: accessible_item,
    hit: accessible_hit,
    current: accessible_current,
    select: accessible_select,
    activate: accessible_activate,
};

thread_local! {
    static ANNOUNCING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Clears `ANNOUNCING` however the change returns.
struct Announcing;

impl Drop for Announcing {
    fn drop(&mut self) {
        ANNOUNCING.set(false);
    }
}

/// Runs `change` and raises the win events for what it did to the panel's current child (spec
/// §10). Nested calls run `change` alone, so one input raises one set of events.
pub(crate) fn with_accessible_events<R>(hwnd: HWND, change: impl FnOnce() -> R) -> R {
    let Some((_, panel)) = windows(hwnd) else {
        return change();
    };
    if ANNOUNCING.get() {
        return change();
    }
    ANNOUNCING.set(true);
    let guard = Announcing;
    let before = AccessibleMark::read(panel, &PANEL_ACCESSIBLE);
    let result = change();
    let after = AccessibleMark::read(panel, &PANEL_ACCESSIBLE);
    drop(guard);
    let focused = unsafe { windows_sys::Win32::UI::Input::KeyboardAndMouse::GetFocus() } == panel;
    sidebar_accessibility::raise(
        panel,
        &sidebar_accessibility::events_between(&before, &after, focused),
    );
    result
}

pub(crate) fn announcing() -> bool {
    ANNOUNCING.get()
}

/// The activity-bar button the keyboard is on (0 Notebook, 1 Search, 2 Favorites, 3 Settings).
pub(crate) fn bar_focus(hwnd: HWND) -> usize {
    unsafe { super::main_window::app_ptr(hwnd) }
        .and_then(|app| unsafe { app.as_ref() }.sidebar.as_ref().map(|s| s.bar_focus))
        .unwrap_or(0)
}

pub(crate) fn set_bar_focus(hwnd: HWND, index: usize) {
    if let Some(mut app) = unsafe { super::main_window::app_ptr(hwnd) }
        && let Some(sidebar) = unsafe { app.as_mut() }.sidebar.as_mut()
    {
        sidebar.bar_focus = index.min(3);
    }
}

/// The activity-bar button of a view.
pub(crate) fn view_button(view: SidebarView) -> Option<usize> {
    match view {
        SidebarView::Notebook => Some(0),
        SidebarView::Search => Some(1),
        SidebarView::Favorites => Some(2),
        SidebarView::Hidden => None,
    }
}
```

Then change the functions Task 6 wrote:

1. **Panel window procedure: MSAA and events.** In `panel_proc`, right after `let main = unsafe { GetParent(panel) };` and before its `match message`, add:

   ```rust
       if matches!(
           message,
           WM_KEYDOWN
               | WM_CHAR
               | WM_LBUTTONDOWN
               | WM_LBUTTONUP
               | WM_LBUTTONDBLCLK
               | WM_MOUSEWHEEL
               | WM_COMMAND
               | sidebar_accessibility::WM_FASTPAD_SIDEBAR_ACTION
       ) && !announcing()
       {
           return with_accessible_events(main, || unsafe {
               panel_proc(panel, message, wparam, lparam)
           });
       }
   ```

   Here `panel_proc` is the procedure itself: the inner call sees `announcing()` and runs normally. Then add these arms to its `match message`, right after the `WM_NCHITTEST` arm, so the Esc arm comes before the `WM_LBUTTONDOWN | .. | WM_KEYDOWN | WM_CHAR => route(..)` arm:

   ```rust
       WM_GETOBJECT if lparam as i32 == OBJID_CLIENT => unsafe {
           sidebar_accessibility::object_result(panel, &PANEL_ACCESSIBLE, wparam)
       },
       sidebar_accessibility::WM_FASTPAD_SIDEBAR_ACCESSIBLE => unsafe {
           sidebar_accessibility::answer(panel, lparam)
       },
       sidebar_accessibility::WM_FASTPAD_SIDEBAR_ACTION => {
           sidebar_accessibility::run_action(panel, &PANEL_ACCESSIBLE, wparam, lparam);
           0
       }
       // Esc anywhere in the panel returns to the editor (spec §10). The search box's own Esc
       // is handled in its subclass.
       WM_KEYDOWN if wparam as u16 == VK_ESCAPE => {
           super::main_window::return_focus_to_editor(main);
           0
       }
   ```

2. **`refresh` and `active_tab_changed`.** Rename each function's existing body to a private `fn refresh_now(hwnd: HWND)` and `fn active_tab_changed_now(hwnd: HWND)`, and make the public functions:

   ```rust
   pub(crate) fn refresh(hwnd: HWND) {
       with_accessible_events(hwnd, || refresh_now(hwnd));
   }

   pub(crate) fn active_tab_changed(hwnd: HWND) {
       with_accessible_events(hwnd, || active_tab_changed_now(hwnd));
   }
   ```

3. **`show_view` and `toggle`.** Rename each existing body to `show_view_now` and `toggle_now` in the same way, and wrap them. This also tells the activity bar which button's pressed state changed:

   ```rust
   pub(crate) fn show_view(hwnd: HWND, view: SidebarView, focus: bool) {
       let before = current_view(hwnd);
       with_accessible_events(hwnd, || show_view_now(hwnd, view, focus));
       bar_views_changed(hwnd, before);
   }

   pub(crate) fn toggle(hwnd: HWND) {
       let before = current_view(hwnd);
       with_accessible_events(hwnd, || toggle_now(hwnd));
       bar_views_changed(hwnd, before);
   }

   fn bar_views_changed(hwnd: HWND, before: SidebarView) {
       let after = current_view(hwnd);
       let Some((bar, _)) = windows(hwnd) else {
           return;
       };
       if before == after {
           return;
       }
       for index in [view_button(before), view_button(after)].into_iter().flatten() {
           sidebar_accessibility::notify(
               windows_sys::Win32::UI::WindowsAndMessaging::EVENT_OBJECT_STATECHANGE,
               bar,
               Some(index),
           );
       }
   }
   ```

Add `WM_GETOBJECT` and `OBJID_CLIENT` to the `WindowsAndMessaging` import and `VK_ESCAPE` to the `KeyboardAndMouse` import (`WM_CHAR`, `WM_LBUTTONDBLCLK`, `WM_MOUSEWHEEL`, `WM_COMMAND` and `POINT` are imported already).

- [ ] **Step 6: The activity bar provider and keyboard in `activity_bar.rs`**

```rust
use crate::window::sidebar_accessibility::{self, AccessibleItem, AccessibleSource, button_item};

/// The four buttons as MSAA children, in `button_rects` order.
pub(crate) fn bar_items(
    rects: [RECT; 4],
    view: crate::config::SidebarView,
    notebook: Option<&str>,
    focused: Option<usize>,
) -> Vec<AccessibleItem> {
    use crate::config::SidebarView;
    let names = [
        notebook.map_or_else(|| "Notebook".to_owned(), |name| format!("Notebook: {name}")),
        "Search".to_owned(),
        "Favorites".to_owned(),
        "Settings".to_owned(),
    ];
    let pressed = [
        view == SidebarView::Notebook,
        view == SidebarView::Search,
        view == SidebarView::Favorites,
        false,
    ];
    names
        .iter()
        .enumerate()
        .map(|(index, name)| button_item(name, pressed[index], focused == Some(index), rects[index]))
        .collect()
}

fn bar_geometry(bar: HWND) -> (RECT, u32) {
    let mut client = RECT::default();
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect(bar, &mut client);
    }
    (
        client,
        unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForWindow(bar) }.max(96),
    )
}

pub(crate) fn accessible_items(bar: HWND) -> Vec<AccessibleItem> {
    let main = unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetParent(bar) };
    let (client, dpi) = bar_geometry(bar);
    let notebook = super::library_host::folder(main)
        .and_then(|folder| folder.file_name().map(|name| name.to_string_lossy().into_owned()));
    let focused = unsafe { windows_sys::Win32::UI::Input::KeyboardAndMouse::GetFocus() } == bar;
    bar_items(
        button_rects(client, dpi),
        super::side_panel::current_view(main),
        notebook.as_deref(),
        focused.then(|| super::side_panel::bar_focus(main)),
    )
}

fn bar_container(_: HWND) -> (String, u32) {
    (
        "Activity bar".to_owned(),
        windows_sys::Win32::UI::Accessibility::ROLE_SYSTEM_TOOLBAR,
    )
}

fn bar_count(_: HWND) -> usize {
    4
}

fn bar_item(bar: HWND, index: usize) -> Option<AccessibleItem> {
    accessible_items(bar).into_iter().nth(index)
}

fn bar_hit(bar: HWND, point: POINT) -> Option<usize> {
    let (client, dpi) = bar_geometry(bar);
    button_rects(client, dpi).iter().position(|rect| {
        point.x >= rect.left && point.x < rect.right && point.y >= rect.top && point.y < rect.bottom
    })
}

fn bar_current(bar: HWND) -> Option<usize> {
    let main = unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetParent(bar) };
    Some(super::side_panel::bar_focus(main))
}

fn bar_select(bar: HWND, index: usize) {
    let main = unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetParent(bar) };
    super::side_panel::set_bar_focus(main, index);
    unsafe {
        windows_sys::Win32::Graphics::Gdi::InvalidateRect(bar, std::ptr::null(), 0);
    }
}

/// Presses button `index` exactly as a click does.
fn bar_activate(bar: HWND, index: usize) {
    let (client, dpi) = bar_geometry(bar);
    if let Some(rect) = button_rects(client, dpi).get(index) {
        sidebar_accessibility::click_item(bar, *rect);
    }
}

pub(crate) static BAR_ACCESSIBLE: AccessibleSource = AccessibleSource {
    container: bar_container,
    count: bar_count,
    item: bar_item,
    hit: bar_hit,
    current: bar_current,
    select: bar_select,
    activate: bar_activate,
};

/// `WM_SETFOCUS` and `WM_KILLFOCUS`. Gaining the focus starts on the active view's button.
pub(crate) fn focus_changed(bar: HWND, gained: bool) {
    let main = unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetParent(bar) };
    if gained {
        let index =
            super::side_panel::view_button(super::side_panel::current_view(main)).unwrap_or(0);
        super::side_panel::set_bar_focus(main, index);
        sidebar_accessibility::notify(
            windows_sys::Win32::UI::WindowsAndMessaging::EVENT_OBJECT_FOCUS,
            bar,
            Some(index),
        );
    }
    unsafe {
        windows_sys::Win32::Graphics::Gdi::InvalidateRect(bar, std::ptr::null(), 0);
    }
}

/// `WM_KEYDOWN` on the bar: Up and Down move, Home and End jump, Enter and Space press.
pub(crate) fn key_down(bar: HWND, key: u16) -> bool {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        VK_DOWN, VK_END, VK_HOME, VK_RETURN, VK_SPACE, VK_UP,
    };
    let main = unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetParent(bar) };
    let focus = super::side_panel::bar_focus(main);
    let next = match key {
        VK_UP => focus.saturating_sub(1),
        VK_DOWN => (focus + 1).min(3),
        VK_HOME => 0,
        VK_END => 3,
        VK_RETURN | VK_SPACE => {
            bar_activate(bar, focus);
            return true;
        }
        _ => return false,
    };
    if next != focus {
        bar_select(bar, next);
        sidebar_accessibility::notify(
            windows_sys::Win32::UI::WindowsAndMessaging::EVENT_OBJECT_FOCUS,
            bar,
            Some(next),
        );
    }
    true
}

/// Draws the keyboard focus rectangle. Call it last in the bar's `WM_PAINT`, before `EndPaint`.
pub(crate) fn paint_keyboard_focus(bar: HWND, hdc: windows_sys::Win32::Graphics::Gdi::HDC) {
    if unsafe { windows_sys::Win32::UI::Input::KeyboardAndMouse::GetFocus() } != bar {
        return;
    }
    let main = unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetParent(bar) };
    let (client, dpi) = bar_geometry(bar);
    let rect = crate::window::panel::inset(
        button_rects(client, dpi)[super::side_panel::bar_focus(main)],
        crate::window::panel::scale(3, dpi),
    );
    unsafe {
        windows_sys::Win32::Graphics::Gdi::DrawFocusRect(hdc, &rect);
    }
}
```

In `bar_proc` (Task 6), add these arms to its `match message`, right after the `WM_ERASEBKGND => 1,` arm. Task 6's bar handles none of these messages yet. Add `WM_GETOBJECT, OBJID_CLIENT, WM_SETFOCUS, WM_KILLFOCUS, WM_KEYDOWN` to its `WindowsAndMessaging` import:

```rust
        WM_GETOBJECT if lparam as i32 == OBJID_CLIENT => unsafe {
            sidebar_accessibility::object_result(bar, &BAR_ACCESSIBLE, wparam)
        },
        sidebar_accessibility::WM_FASTPAD_SIDEBAR_ACCESSIBLE => unsafe {
            sidebar_accessibility::answer(bar, lparam)
        },
        sidebar_accessibility::WM_FASTPAD_SIDEBAR_ACTION => {
            sidebar_accessibility::run_action(bar, &BAR_ACCESSIBLE, wparam, lparam);
            0
        }
        WM_SETFOCUS | WM_KILLFOCUS => {
            focus_changed(bar, message == WM_SETFOCUS);
            0
        }
        WM_KEYDOWN if key_down(bar, wparam as u16) => 0,
```

In the bar's `paint` (Task 6), the `paint_buffered(bar, |dc, client| unsafe { .. })` closure ends with the `for button in ActivityButton::ALL { .. }` loop. After that loop, still inside the closure, draw the focus rectangle into the same buffered DC:

```rust
        paint_keyboard_focus(bar, dc);
```

- [ ] **Step 7: F6 and Shift+F6**

1. **`src/window/commands.rs`.** Add after `NoteRevealInExplorer = 182`:

   ```rust
       FocusNextPane = 183,
       FocusPreviousPane = 184,
   ```

   - Append `CommandId::FocusNextPane, CommandId::FocusPreviousPane` to the `COMMANDS` array in `try_from`, and raise its length by 2.
   - Add `| Self::FocusNextPane | Self::FocusPreviousPane` to the `needs_document` exclusion list.
   - In `native_command_values_are_stable_and_round_trip`, add:

   ```rust
           assert_eq!(CommandId::try_from(183), Ok(CommandId::FocusNextPane));
           assert_eq!(CommandId::try_from(184), Ok(CommandId::FocusPreviousPane));
           assert!(!CommandId::FocusNextPane.needs_document());
   ```

2. **`src/window/menus.rs`.** Append two entries to the array in `accelerator_specs`, and raise the array length and the `assert_eq!(specs.len(), ..)` in `shortcut_and_menu_commands_share_command_ids` by 2:

   ```rust
           virtual_key(0, VK_F6, CommandId::FocusNextPane),
           virtual_key(FSHIFT, VK_F6, CommandId::FocusPreviousPane),
   ```

   Add `VK_F6` to the `KeyboardAndMouse` import.

3. **`src/window/command_palette.rs`.** F6 moves between window parts, so the palette doesn't list it. In `every_command_except_tab_positions_and_the_palette_is_listed_once`, extend `expected`:

   ```rust
               let expected = usize::from(
                   command.tab_index().is_none()
                       && command != CommandId::CommandPalette
                       && command != CommandId::MarkdownPreviewCycle
                       && command != CommandId::FocusNextPane
                       && command != CommandId::FocusPreviousPane,
               );
   ```

4. **`src/window/main_window.rs`.** Add:

   ```rust
   /// The three parts F6 moves between, in tab order (spec §10).
   #[derive(Clone, Copy, Debug, Eq, PartialEq)]
   pub(crate) enum FocusPart {
       ActivityBar,
       Panel,
       Editor,
   }

   /// The part after `current`, skipping a closed panel and, with notes mode off, the sidebar.
   pub(crate) fn next_focus_part(
       current: FocusPart,
       backwards: bool,
       sidebar: bool,
       panel_open: bool,
   ) -> FocusPart {
       let parts: &[FocusPart] = match (sidebar, panel_open) {
           (false, _) => &[FocusPart::Editor],
           (true, false) => &[FocusPart::ActivityBar, FocusPart::Editor],
           (true, true) => &[FocusPart::ActivityBar, FocusPart::Panel, FocusPart::Editor],
       };
       let index = parts
           .iter()
           .position(|part| *part == current)
           .unwrap_or(parts.len() - 1);
       let next = if backwards {
           (index + parts.len() - 1) % parts.len()
       } else {
           (index + 1) % parts.len()
       };
       parts[next]
   }

   /// The editor, or the frame while no tab is open.
   pub(crate) fn return_focus_to_editor(hwnd: HWND) {
       if tab_count(hwnd) > 0 {
           focus_content(hwnd);
       } else {
           unsafe {
               SetFocus(hwnd);
           }
       }
   }

   /// F6 and Shift+F6: activity bar, panel, editor.
   pub(crate) fn cycle_focus(hwnd: HWND, backwards: bool) {
       use crate::config::SidebarView;
       use crate::window::side_panel;
       let windows = side_panel::windows(hwnd);
       let panel_open = windows.is_some() && side_panel::current_view(hwnd) != SidebarView::Hidden;
       let focus = unsafe { windows_sys::Win32::UI::Input::KeyboardAndMouse::GetFocus() };
       let current = match windows {
           Some((bar, _)) if focus == bar => FocusPart::ActivityBar,
           Some((_, panel))
               if focus == panel
                   || unsafe { windows_sys::Win32::UI::WindowsAndMessaging::IsChild(panel, focus) }
                       != 0 =>
           {
               FocusPart::Panel
           }
           _ => FocusPart::Editor,
       };
       match next_focus_part(current, backwards, windows.is_some(), panel_open) {
           FocusPart::ActivityBar => {
               if let Some((bar, _)) = windows {
                   unsafe {
                       SetFocus(bar);
                   }
               }
           }
           FocusPart::Panel => side_panel::show_view(hwnd, side_panel::current_view(hwnd), true),
           FocusPart::Editor => return_focus_to_editor(hwnd),
       }
   }
   ```

   In `execute_command`, add:

   ```rust
           CommandId::FocusNextPane => cycle_focus(hwnd, false),
           CommandId::FocusPreviousPane => cycle_focus(hwnd, true),
   ```

- [ ] **Step 8: Write the in-process window tests** in the `main_window.rs` tests module

```rust
    fn focused() -> HWND {
        unsafe { windows_sys::Win32::UI::Input::KeyboardAndMouse::GetFocus() }
    }

    fn shown_window() -> ProductionWindow {
        let window = ProductionWindow::new(make_app());
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::ShowWindow(
                window.hwnd,
                windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOW,
            );
        }
        window
    }

    #[test]
    fn f6_cycles_activity_bar_panel_and_editor_and_shift_f6_goes_back() {
        // Break caught: F6 doing nothing, skipping the panel, or leaving the focus in a closed
        // panel.
        let _scintilla = load_native_scintilla();
        let window = shown_window();
        let _editor = install_test_editor(&window);
        let (bar, panel) = crate::window::side_panel::windows(window.hwnd).unwrap();
        use crate::config::SidebarView;
        crate::window::side_panel::show_view(window.hwnd, SidebarView::Notebook, false);
        super::return_focus_to_editor(window.hwnd);
        let editor = focused();

        execute_command(window.hwnd, CommandId::FocusNextPane);
        assert_eq!(focused(), bar);
        execute_command(window.hwnd, CommandId::FocusNextPane);
        assert_eq!(focused(), panel);
        execute_command(window.hwnd, CommandId::FocusNextPane);
        assert_eq!(focused(), editor);
        execute_command(window.hwnd, CommandId::FocusPreviousPane);
        assert_eq!(focused(), panel);

        crate::window::side_panel::toggle(window.hwnd);
        assert_eq!(
            crate::window::side_panel::current_view(window.hwnd),
            SidebarView::Hidden
        );
        super::return_focus_to_editor(window.hwnd);
        execute_command(window.hwnd, CommandId::FocusNextPane);
        assert_eq!(focused(), bar);
        execute_command(window.hwnd, CommandId::FocusNextPane);
        assert_eq!(focused(), editor, "a closed panel is skipped");
    }

    #[test]
    fn escape_in_the_panel_returns_the_focus_to_the_editor() {
        // Break caught: Esc in the tree leaving the keyboard stuck in the sidebar.
        let _scintilla = load_native_scintilla();
        let window = shown_window();
        let _editor = install_test_editor(&window);
        super::return_focus_to_editor(window.hwnd);
        let editor = focused();
        crate::window::side_panel::show_view(
            window.hwnd,
            crate::config::SidebarView::Notebook,
            true,
        );
        let panel = crate::window::side_panel::windows(window.hwnd).unwrap().1;
        assert_eq!(focused(), panel);
        unsafe {
            SendMessageW(
                panel,
                windows_sys::Win32::UI::WindowsAndMessaging::WM_KEYDOWN,
                windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE as usize,
                0,
            );
        }
        assert_eq!(focused(), editor);
    }

    #[test]
    fn the_activity_bar_moves_with_arrows_and_presses_with_enter() {
        // Break caught: activity-bar buttons reachable only with the mouse, or their pressed
        // state not following the shown view.
        let _scintilla = load_native_scintilla();
        let window = shown_window();
        let _editor = install_test_editor(&window);
        use crate::config::SidebarView;
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{SetFocus, VK_DOWN, VK_RETURN};
        use windows_sys::Win32::UI::WindowsAndMessaging::WM_KEYDOWN;
        crate::window::side_panel::show_view(window.hwnd, SidebarView::Notebook, false);
        let bar = crate::window::side_panel::windows(window.hwnd).unwrap().0;
        unsafe {
            SetFocus(bar);
        }
        assert_eq!(crate::window::side_panel::bar_focus(window.hwnd), 0);
        let items = crate::window::activity_bar::accessible_items(bar);
        assert_eq!(items.len(), 4);
        assert_ne!(
            items[0].state & crate::window::sidebar_accessibility::STATE_PRESSED,
            0
        );
        assert_ne!(
            items[0].state & crate::window::sidebar_accessibility::STATE_FOCUSED,
            0
        );
        assert_eq!(items[3].name, "Settings");

        unsafe {
            SendMessageW(bar, WM_KEYDOWN, VK_DOWN as usize, 0);
        }
        assert_eq!(crate::window::side_panel::bar_focus(window.hwnd), 1);
        unsafe {
            SendMessageW(bar, WM_KEYDOWN, VK_RETURN as usize, 0);
        }
        assert_eq!(
            crate::window::side_panel::current_view(window.hwnd),
            SidebarView::Search
        );
        let items = crate::window::activity_bar::accessible_items(bar);
        assert_ne!(
            items[1].state & crate::window::sidebar_accessibility::STATE_PRESSED,
            0
        );
        assert_eq!(
            items[0].state & crate::window::sidebar_accessibility::STATE_PRESSED,
            0
        );
    }

    #[test]
    fn the_panel_exposes_the_tree_as_an_outline_with_pinned_and_folder_states() {
        // Break caught: the tree invisible to screen readers, the child count not matching the
        // visible rows, or a pin and a collapsed folder not reported.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("tree-msaa");
        let a = scratch.note("a.md", "a");
        std::fs::create_dir_all(scratch.folder().join("sub")).unwrap();
        scratch.note(r"sub\b.md", "b");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        scratch.install(window.hwnd);
        crate::window::library_host::toggle_pin(window.hwnd, &a);
        crate::window::side_panel::show_view(
            window.hwnd,
            crate::config::SidebarView::Notebook,
            false,
        );
        let panel = crate::window::side_panel::windows(window.hwnd).unwrap().1;
        let count = crate::window::side_panel::accessible_item_count(panel);
        let items = (0..count)
            .filter_map(|index| crate::window::side_panel::accessible_item(panel, index))
            .collect::<Vec<_>>();
        assert_eq!(items.len(), count);
        let rows = items
            .iter()
            .filter(|item| {
                item.role == windows_sys::Win32::UI::Accessibility::ROLE_SYSTEM_OUTLINEITEM
                    && !item.name.ends_with(", unsaved")
            })
            .collect::<Vec<_>>();
        // The window's untitled tab is an unsaved row, left out above. "sub" is collapsed, so b
        // is not a row: pinned a first, then the folder.
        assert_eq!(rows.len(), 2, "{items:?}");
        assert_eq!(rows[0].name, "a, pinned");
        assert_eq!(rows[1].name, "sub");
        assert_ne!(
            rows[1].state & crate::window::sidebar_accessibility::STATE_COLLAPSED,
            0
        );

        let provider = crate::window::sidebar_accessibility::create_for_test(
            panel,
            &crate::window::side_panel::PANEL_ACCESSIBLE,
        );
        let table = &crate::window::sidebar_accessibility::SIDEBAR_VTABLE;
        use crate::window::accessibility::{RawVariant, VariantValue};
        unsafe {
            let mut children = 0;
            (table.get_acc_child_count)(provider, &mut children);
            assert_eq!(children as usize, count);
            let mut role = RawVariant::empty();
            (table.get_acc_role)(provider, RawVariant::integer(0), &mut role);
            assert_eq!(
                role.child_id(),
                Some(windows_sys::Win32::UI::Accessibility::ROLE_SYSTEM_OUTLINE as i32)
            );
            (table.release)(provider);
        }
    }
```

- [ ] **Step 9: Run the targeted tests**

Run: `cargo test --lib -- sidebar_accessibility f6_ escape_in_the_panel the_activity_bar_moves the_panel_exposes native_command_values every_command_except shortcut_and_menu --test-threads=1`
Expected: all pass.

Run: `cargo clippy --all-targets -- -D warnings` and `cargo fmt --all -- --check`
Expected: no warnings and no diff.

- [ ] **Step 10: Commit**

```bash
git add src/window/sidebar_accessibility.rs src/window/mod.rs src/window/side_panel.rs src/window/activity_bar.rs src/window/notebook_view.rs src/window/favorites_view.rs src/window/search_view.rs src/window/main_window.rs src/window/commands.rs src/window/menus.rs src/window/command_palette.rs
git commit -m "feat(sidebar): MSAA for the activity bar and panel, win events, F6 focus cycle"
```

---

### Task 14: End-to-end tests, bench and docs

**Files:**
- Modify:
  - `tests/windows/library.rs`
  - `src/bin/fastpad-bench.rs`
  - `benchmarks/README.md`
  - `README.md`
  - `docs/superpowers/specs/2026-09-23-note-sidebar-design.md`

**Interfaces:**
- Consumes the real `fastpad.exe` through `tests/windows/support`, all of Tasks 1–13, and the window class names Task 6 registers. **These are now part of the contract:** `"FastPadActivityBar"` for the activity bar and `"FastPadSidePanel"` for the panel.
- Produces:
  - The end-to-end tests below.
  - `fastpad-bench --sidebar-view VIEW`.
  - `library-scan` output lines `tree_build_ms`, `tree_rows_expanded_ms` and `name_search_ms`.
  - The README and spec updates.

**Machine hygiene:** every real-exe test runs against a scratch `LOCALAPPDATA` and a scratch notes folder, and `Scratch::new` refuses to run while any FastPad runs in the session. The bench gives each launch its own scratch profile. Nothing here touches `%LOCALAPPDATA%\FastPad` or `Documents`. Before any manual run of the real app against your own profile, copy `%LOCALAPPDATA%\FastPad\fastpad.ini` and `folders.ini` aside, and restore them afterwards.

- [ ] **Step 1: Add the sidebar helpers to `tests/windows/library.rs`**

Extend the imports:

```rust
use std::ffi::c_void;
use support::win32::{Deadline, find_child_by_class, focused_window, scintilla_text, send_text};
use windows_sys::Win32::Foundation::{POINT, RECT, SysFreeString, SysStringLen};
use windows_sys::Win32::Graphics::Gdi::ScreenToClient;
use windows_sys::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};
use windows_sys::Win32::System::Variant::{VARIANT, VT_I4};
use windows_sys::Win32::UI::Accessibility::{
    AccessibleObjectFromWindow, ROLE_SYSTEM_OUTLINEITEM, ROLE_SYSTEM_PAGETAB,
};
use windows_sys::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    SetThreadDpiAwarenessContext,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    OBJID_CLIENT, PostMessageW, WM_ACTIVATEAPP, WM_CHAR, WM_CLOSE, WM_COMMAND, WM_KEYDOWN,
    WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP,
};
use windows_sys::core::{BSTR, GUID, HRESULT};
```

Then add, after `close`:

```rust
const ACTIVITY_BAR_CLASS: &str = "FastPadActivityBar";
const SIDE_PANEL_CLASS: &str = "FastPadSidePanel";
const IID_IACCESSIBLE: GUID = GUID::from_u128(0x618736e0_3c3d_11cf_810c_00aa00389b71);
const MK_LBUTTON: usize = 0x0001;

/// MSAA locations are physical pixels; this thread must read them the same way.
struct DpiContext(DPI_AWARENESS_CONTEXT);

impl DpiContext {
    fn per_monitor_v2() -> Self {
        Self(unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) })
    }
}

impl Drop for DpiContext {
    fn drop(&mut self) {
        unsafe {
            SetThreadDpiAwarenessContext(self.0);
        }
    }
}

struct ComApartment;

impl ComApartment {
    fn initialize() -> Self {
        let result = unsafe { CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32) };
        assert!(result >= 0, "CoInitializeEx failed: {result:#x}");
        Self
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

#[repr(C)]
struct AccessibleVtable {
    query_interface: usize,
    add_ref: usize,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    get_type_info_count: usize,
    get_type_info: usize,
    get_ids_of_names: usize,
    invoke: usize,
    get_acc_parent: usize,
    get_acc_child_count: unsafe extern "system" fn(*mut c_void, *mut i32) -> HRESULT,
    get_acc_child: usize,
    get_acc_name: unsafe extern "system" fn(*mut c_void, VARIANT, *mut BSTR) -> HRESULT,
    get_acc_value: usize,
    get_acc_description: usize,
    get_acc_role: unsafe extern "system" fn(*mut c_void, VARIANT, *mut VARIANT) -> HRESULT,
    get_acc_state: usize,
    get_acc_help: usize,
    get_acc_help_topic: usize,
    get_acc_keyboard_shortcut: usize,
    get_acc_focus: usize,
    get_acc_selection: usize,
    get_acc_default_action: usize,
    acc_select: usize,
    acc_location: unsafe extern "system" fn(
        *mut c_void,
        *mut i32,
        *mut i32,
        *mut i32,
        *mut i32,
        VARIANT,
    ) -> HRESULT,
}

fn child_variant(id: i32) -> VARIANT {
    let mut variant = VARIANT::default();
    variant.Anonymous.Anonymous.vt = VT_I4;
    variant.Anonymous.Anonymous.Anonymous.lVal = id;
    variant
}

/// A window's MSAA object, read out of process as a screen reader reads it.
struct Accessible(*mut c_void);

impl Accessible {
    fn from_window(hwnd: HWND) -> Option<Self> {
        let mut object = std::ptr::null_mut();
        let result = unsafe {
            AccessibleObjectFromWindow(hwnd, OBJID_CLIENT as u32, &IID_IACCESSIBLE, &mut object)
        };
        (result >= 0 && !object.is_null()).then_some(Self(object))
    }

    fn vtable(&self) -> &AccessibleVtable {
        unsafe { &**(self.0 as *const *const AccessibleVtable) }
    }

    fn child_count(&self) -> i32 {
        let mut count = 0;
        unsafe { (self.vtable().get_acc_child_count)(self.0, &mut count) };
        count
    }

    fn name(&self, child: i32) -> Option<String> {
        let mut value: BSTR = std::ptr::null();
        let result = unsafe { (self.vtable().get_acc_name)(self.0, child_variant(child), &mut value) };
        if result < 0 || value.is_null() {
            return None;
        }
        let length = unsafe { SysStringLen(value) } as usize;
        let name = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(value, length) });
        unsafe { SysFreeString(value) };
        Some(name)
    }

    fn role(&self, child: i32) -> Option<u32> {
        let mut value = VARIANT::default();
        let result = unsafe { (self.vtable().get_acc_role)(self.0, child_variant(child), &mut value) };
        (result >= 0).then(|| unsafe { value.Anonymous.Anonymous.Anonymous.lVal } as u32)
    }

    fn location(&self, child: i32) -> Option<RECT> {
        let (mut left, mut top, mut width, mut height) = (0, 0, 0, 0);
        let result = unsafe {
            (self.vtable().acc_location)(
                self.0,
                &mut left,
                &mut top,
                &mut width,
                &mut height,
                child_variant(child),
            )
        };
        (result >= 0).then_some(RECT {
            left,
            top,
            right: left + width,
            bottom: top + height,
        })
    }

    /// The child IDs and names, in order.
    fn children(&self) -> Vec<(i32, String)> {
        (1..=self.child_count())
            .filter_map(|id| Some((id, self.name(id)?)))
            .collect()
    }
}

impl Drop for Accessible {
    fn drop(&mut self) {
        unsafe {
            (self.vtable().release)(self.0);
        }
    }
}

/// The main window's tab titles, from its title-strip MSAA object.
fn tab_titles(hwnd: HWND) -> Vec<String> {
    let Some(strip) = Accessible::from_window(hwnd) else {
        return Vec::new();
    };
    (1..=strip.child_count())
        .filter(|&id| strip.role(id) == Some(ROLE_SYSTEM_PAGETAB))
        .filter_map(|id| strip.name(id))
        .collect()
}

/// Whether the panel lists a child named `name` now.
fn panel_lists(panel: HWND, name: &str) -> bool {
    Accessible::from_window(panel)
        .is_some_and(|accessible| accessible.children().iter().any(|(_, n)| n == name))
}

/// The note and folder rows the panel lists, in order. Rows for untitled tabs (", unsaved") are
/// left out: every launch starts with one.
fn tree_rows(panel: HWND) -> Vec<String> {
    let Some(accessible) = Accessible::from_window(panel) else {
        return Vec::new();
    };
    (1..=accessible.child_count())
        .filter(|&id| accessible.role(id) == Some(ROLE_SYSTEM_OUTLINEITEM))
        .filter_map(|id| accessible.name(id))
        .filter(|name| !name.ends_with(", unsaved"))
        .collect()
}

/// The panel-client center of the child named `name`, once the panel lists it.
fn child_center(panel: HWND, name: &str) -> isize {
    let deadline = Deadline::after(WAIT);
    loop {
        if let Some(accessible) = Accessible::from_window(panel)
            && let Some((id, _)) = accessible.children().into_iter().find(|(_, n)| n == name)
            && let Some(rect) = accessible.location(id)
        {
            let mut center = POINT {
                x: (rect.left + rect.right) / 2,
                y: (rect.top + rect.bottom) / 2,
            };
            unsafe {
                ScreenToClient(panel, &mut center);
            }
            return (center.x as u16 as u32 | ((center.y as u16 as u32) << 16)) as isize;
        }
        assert!(!deadline.expired(), "timed out waiting for the sidebar to list {name}");
        deadline.sleep_step();
    }
}

fn click_child(panel: HWND, name: &str) {
    let point = child_center(panel, name);
    unsafe {
        PostMessageW(panel, WM_LBUTTONDOWN, MK_LBUTTON, point);
        PostMessageW(panel, WM_LBUTTONUP, 0, point);
    }
}

fn double_click_child(panel: HWND, name: &str) {
    let point = child_center(panel, name);
    unsafe {
        PostMessageW(panel, WM_LBUTTONDOWN, MK_LBUTTON, point);
        PostMessageW(panel, WM_LBUTTONUP, 0, point);
        PostMessageW(panel, WM_LBUTTONDBLCLK, MK_LBUTTON, point);
        PostMessageW(panel, WM_LBUTTONUP, 0, point);
    }
}

/// The first `folder=` line of `folders.ini`: the open notebook.
fn first_folder(data: &Scratch) -> Option<String> {
    read(&data.data().join("folders.ini"))
        .lines()
        .find(|line| line.starts_with("folder="))
        .map(str::to_owned)
}

fn write_folders(data: &Scratch, folders: fastpad::library::local::RecentFolders) {
    std::fs::write(
        fastpad::library::local::folders_file(&data.data()),
        folders.encode(),
    )
    .unwrap();
}
```

- [ ] **Step 2: Add the end-to-end tests** to `tests/windows/library.rs`

```rust
#[test]
fn a_click_opens_a_preview_tab_a_second_click_replaces_it_and_a_double_click_keeps_it() {
    // Break caught: every click opening a new tab, the replacement landing at the end of the
    // strip instead of in place, or a double-click leaving the tab to be replaced.
    let _lock = LIBRARY_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _dpi = DpiContext::per_monitor_v2();
    let _com = ComApartment::initialize();
    let data = Scratch::new("preview-tab");
    data.note("a.md", "alpha");
    data.note("b.md", "beta");
    data.note("c.md", "gamma");
    let mut process =
        FastPadProcess::spawn_with_local_app_data([data.folder()], &data.root).unwrap();
    let hwnd = process.wait_for_main_window(WAIT).unwrap();
    let editor = find_child_by_class(hwnd, "Scintilla").unwrap();
    wait_for_library(&data);
    let panel = find_child_by_class(hwnd, SIDE_PANEL_CLASS).unwrap();

    click_child(panel, "a");
    wait_until("a to open", || {
        scintilla_text(editor).is_ok_and(|t| t == "alpha")
    });
    let first = tab_titles(hwnd);
    let slot = first.iter().position(|t| t == "a.md").expect("a preview tab");

    click_child(panel, "b");
    wait_until("b to replace a", || {
        scintilla_text(editor).is_ok_and(|t| t == "beta")
    });
    let replaced = tab_titles(hwnd);
    assert_eq!(replaced.len(), first.len(), "{replaced:?}");
    assert!(!replaced.iter().any(|t| t == "a.md"));
    assert_eq!(replaced.iter().position(|t| t == "b.md"), Some(slot));

    double_click_child(panel, "b");
    click_child(panel, "c");
    wait_until("c to open", || {
        scintilla_text(editor).is_ok_and(|t| t == "gamma")
    });
    let kept = tab_titles(hwnd);
    assert_eq!(kept.len(), first.len() + 1, "{kept:?}");
    assert!(kept.iter().any(|t| t == "b.md") && kept.iter().any(|t| t == "c.md"));
    close(process, hwnd);
}

#[test]
fn pinning_from_the_tree_writes_a_version_2_record_that_survives_a_restart() {
    // Break caught: a pin written in the version 1 format, pinned from the active tab instead of
    // the tree's selected row, or lost (and unsorted) after a restart.
    let _lock = LIBRARY_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _dpi = DpiContext::per_monitor_v2();
    let _com = ComApartment::initialize();
    let data = Scratch::new("pin-tree");
    data.note("a.md", "a");
    data.note("b.md", "b");
    let mut process =
        FastPadProcess::spawn_with_local_app_data([data.folder()], &data.root).unwrap();
    let hwnd = process.wait_for_main_window(WAIT).unwrap();
    wait_for_library(&data);
    let panel = find_child_by_class(hwnd, SIDE_PANEL_CLASS).unwrap();
    click_child(panel, "b");
    // Ctrl+Shift+E moves the focus into the tree, on the active tab's row; the pin then acts on it.
    command(hwnd, CommandId::ShowNotebookView);
    command(hwnd, CommandId::NoteTogglePin);
    wait_until("a version 2 pin record", || {
        let library = read(&data.library_ini());
        library.starts_with("version=2")
            && library
                .lines()
                .any(|l| l.starts_with("note=") && l.contains("|p|") && l.ends_with("|b.md"))
    });
    wait_until("the pinned row to sort first", || {
        tree_rows(panel).first().map(String::as_str) == Some("b, pinned")
    });
    close(process, hwnd);

    let mut process =
        FastPadProcess::spawn_with_local_app_data(std::iter::empty::<&str>(), &data.root).unwrap();
    let hwnd = process.wait_for_main_window(WAIT).unwrap();
    let panel = find_child_by_class(hwnd, SIDE_PANEL_CLASS).unwrap();
    wait_until("the pin to come back after a restart", || {
        tree_rows(panel) == ["b, pinned", "a"]
    });
    close(process, hwnd);
}

#[test]
fn a_favorite_notebook_opens_from_the_favorites_view() {
    // Break caught: "Toggle favorite notebook" not writing favorite=, or a click in the
    // Favorites view not switching notebooks and showing the Notebook view.
    let _lock = LIBRARY_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _dpi = DpiContext::per_monitor_v2();
    let _com = ComApartment::initialize();
    let data = Scratch::new("favorites");
    data.note("a.md", "a");
    let other = data.root.join("other");
    std::fs::create_dir_all(&other).unwrap();
    std::fs::write(other.join("c.md"), "c").unwrap();
    let mut process =
        FastPadProcess::spawn_with_local_app_data([data.folder()], &data.root).unwrap();
    let hwnd = process.wait_for_main_window(WAIT).unwrap();
    wait_for_library(&data);
    let panel = find_child_by_class(hwnd, SIDE_PANEL_CLASS).unwrap();
    command(hwnd, CommandId::ToggleNotebookFavorite);
    let favorite = format!("favorite={}", data.folder().display());
    wait_until("folders.ini to list the favorite", || {
        read(&data.data().join("folders.ini"))
            .lines()
            .any(|l| l == favorite)
    });

    forward(&data, &other);
    let other_line = format!("folder={}", other.display());
    wait_until("the other notebook to open", || {
        first_folder(&data).as_deref() == Some(other_line.as_str())
    });
    wait_until("the tree to list c", || tree_rows(panel) == ["c"]);

    command(hwnd, CommandId::ShowFavoritesView);
    click_child(panel, "notes");
    let notes_line = format!("folder={}", data.folder().display());
    wait_until("the favorite to open", || {
        first_folder(&data).as_deref() == Some(notes_line.as_str())
    });
    wait_until("the Notebook view to list a", || tree_rows(panel) == ["a"]);
    close(process, hwnd);
}

#[test]
fn closing_the_notebook_writes_open_none_and_the_next_start_opens_nothing() {
    // Break caught: Close notebook not remembered, so the next start reopens it, or falls back
    // to Documents\FastPad.
    let _lock = LIBRARY_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _dpi = DpiContext::per_monitor_v2();
    let _com = ComApartment::initialize();
    let data = Scratch::new("close-notebook");
    data.note("a.md", "a");
    let folders = data.data().join("folders.ini");
    let mut process =
        FastPadProcess::spawn_with_local_app_data([data.folder()], &data.root).unwrap();
    let hwnd = process.wait_for_main_window(WAIT).unwrap();
    wait_for_library(&data);
    command(hwnd, CommandId::CloseNotebook);
    wait_until("open=none", || read(&folders).lines().any(|l| l == "open=none"));
    close(process, hwnd);

    let mut process =
        FastPadProcess::spawn_with_local_app_data(std::iter::empty::<&str>(), &data.root).unwrap();
    let hwnd = process.wait_for_main_window(WAIT).unwrap();
    let panel = find_child_by_class(hwnd, SIDE_PANEL_CLASS).unwrap();
    wait_until("the no-notebook state", || {
        panel_lists(panel, "Open notebook\u{2026}")
    });
    // The worker decides the startup notebook; give a wrong decision time to show up.
    std::thread::sleep(Duration::from_millis(1_500));
    assert!(tree_rows(panel).is_empty());
    close(process, hwnd);
    assert!(read(&folders).lines().any(|l| l == "open=none"));
}

#[test]
fn moving_a_note_to_another_notebook_moves_the_file_and_its_tab_follows() {
    // Break caught: Move to notebook copying instead of moving, or the open tab still saving to
    // (and re-creating) the old path.
    let _lock = LIBRARY_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _dpi = DpiContext::per_monitor_v2();
    let _com = ComApartment::initialize();
    let data = Scratch::new("move-note");
    let note = data.note("a.md", "alpha");
    let other = data.root.join("other");
    std::fs::create_dir_all(&other).unwrap();
    write_folders(
        &data,
        fastpad::library::local::RecentFolders {
            folders: vec![data.folder(), other.clone()],
            ..Default::default()
        },
    );
    let mut process =
        FastPadProcess::spawn_with_local_app_data([data.folder()], &data.root).unwrap();
    let hwnd = process.wait_for_main_window(WAIT).unwrap();
    let editor = find_child_by_class(hwnd, "Scintilla").unwrap();
    wait_for_library(&data);
    let panel = find_child_by_class(hwnd, SIDE_PANEL_CLASS).unwrap();
    click_child(panel, "a");
    wait_until("a to open", || {
        scintilla_text(editor).is_ok_and(|t| t == "alpha")
    });

    command(hwnd, CommandId::NoteMoveToNotebook);
    wait_until("the picker to take focus", || {
        focused_window(hwnd).is_ok_and(|f| f != editor)
    });
    // The first row is the recent notebook that is not the open one.
    let field = focused_window(hwnd).unwrap();
    unsafe {
        PostMessageW(field, WM_KEYDOWN, VK_RETURN as usize, 0);
    }
    let moved = other.join("a.md");
    wait_until("the file to move", || moved.exists() && !note.exists());

    type_more(editor, "!");
    command(hwnd, CommandId::Save);
    wait_until("the tab to save into the moved file", || {
        read(&moved) == "alpha!"
    });
    assert!(!note.exists(), "the old path must not be re-created");
    close(process, hwnd);
}

#[test]
fn with_notes_mode_off_there_is_no_activity_bar_or_side_panel() {
    // Break caught: the sidebar created, or its space reserved, with notes mode off.
    let _lock = LIBRARY_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let data = Scratch::new("mode-off-sidebar");
    std::fs::write(data.data().join("fastpad.ini"), "notes_mode=false\n").unwrap();
    let mut process =
        FastPadProcess::spawn_with_local_app_data(std::iter::empty::<&str>(), &data.root).unwrap();
    let hwnd = process.wait_for_main_window(WAIT).unwrap();
    find_child_by_class(hwnd, "Scintilla").unwrap();
    assert!(find_child_by_class(hwnd, ACTIVITY_BAR_CLASS).is_err());
    assert!(find_child_by_class(hwnd, SIDE_PANEL_CLASS).is_err());
    close(process, hwnd);
}
```

- [ ] **Step 3: Build and run the target**

Run: `cargo build`, then `cargo test --test library -- --test-threads=1`
Expected: all pass: the six existing tests as Tasks 1–11 left them, plus the six new ones. A panic "close every FastPad window in this session…" means a FastPad is running. Close it and run again.

- [ ] **Step 4: Extend the bench.** In `src/bin/fastpad-bench.rs`:

1. **`--sidebar-view` on `Action::Run`.** Add `sidebar_view: Option<String>,` after `notes_folder`, and update `USAGE`:

   ```rust
   const USAGE: &str = "usage: fastpad-bench [--runs N] [--warmup N] [--output FILE] [--launch-file FILE] \
                        [--notes-folder DIR] [--sidebar-view notebook|search|favorites|none] \
                        [--enforce-reference]\n       \
                        fastpad-bench compare BASELINE.jsonl CANDIDATE.jsonl\n       \
                        fastpad-bench library-scan DIR [--count N] [--enforce-reference]";
   ```

   In `parse_args`:
   - add `let mut sidebar_view = None;` beside `notes_folder`;
   - add `"--sidebar-view"` to the value-taking flag list;
   - add the match arm:

   ```rust
                       "--sidebar-view" => {
                           let view = value
                               .to_str()
                               .filter(|view| {
                                   matches!(*view, "notebook" | "search" | "favorites" | "none")
                               })
                               .ok_or_else(|| {
                                   "--sidebar-view must be notebook, search, favorites or none"
                                       .to_owned()
                               })?;
                           sidebar_view = Some(view.to_owned());
                       }
   ```

   Then add `sidebar_view,` to the returned `Action::Run`.

   Thread it through:
   - `run_main` passes `sidebar_view.as_deref()`;
   - `run_distribution` gains a `sidebar_view: Option<&str>` parameter;
   - both `run_once` definitions gain `sidebar_view: Option<&str>` (the non-Windows one names it `_sidebar_view`).

   In the Windows `run_once`, after `local_app_data.seed_notes_folder(notes_folder)?;`, add `local_app_data.seed_sidebar_view(sidebar_view)?;`. Add to `impl ScratchLocalAppData`:

   ```rust
       /// Writes `FastPad\fastpad.ini` with `sidebar_view=VIEW`, so a run can measure startup
       /// with the panel open or closed. Without a view the scratch profile keeps the default.
       fn seed_sidebar_view(&self, view: Option<&str>) -> Result<(), String> {
           let Some(view) = view else {
               return Ok(());
           };
           let path = self.0.join("FastPad").join("fastpad.ini");
           std::fs::write(&path, format!("sidebar_view={view}\r\n"))
               .map_err(|error| format!("could not write {}: {error}", path.display()))
       }
   ```

2. **Tree and search timings in `library-scan`.** Add these constants beside the existing `LIBRARY_SCAN_*` ones:

   ```rust
   /// Spec §12 targets on the reference machine.
   const TREE_BUILD_REFERENCE_MS: f64 = 20.0;
   const TREE_ROWS_REFERENCE_MS: f64 = 16.0;
   const NAME_SEARCH_REFERENCE_MS: f64 = 5.0;
   ```

   and this function after `run_library_scan`:

   ```rust
   /// The median of five timings of `work`, in milliseconds.
   fn median_ms(mut work: impl FnMut()) -> f64 {
       let mut times = (0..LIBRARY_SCAN_WARM_LOADS)
           .map(|_| {
               let started = std::time::Instant::now();
               work();
               started.elapsed().as_secs_f64() * 1_000.0
           })
           .collect::<Vec<_>>();
       times.sort_by(f64::total_cmp);
       times[times.len() / 2]
   }
   ```

   In `run_library_scan`, replace the block from `println!("notes={}", state.notes.len());` to the end of the function with:

   ```rust
       let paths = state
           .notes
           .iter()
           .map(|note| note.path.clone())
           .collect::<Vec<_>>();
       let pinned = state
           .library
           .notes
           .iter()
           .filter(|record| record.pinned)
           .map(|record| record.path.clone())
           .collect::<Vec<_>>();
       let tree_build_ms = median_ms(|| {
           std::hint::black_box(fastpad::library::tree::NoteTree::build(&paths, &pinned));
       });
       let tree = fastpad::library::tree::NoteTree::build(&paths, &pinned);
       // Every folder expanded: the fixture's folders hold 500 notes each.
       let tree_rows_expanded_ms = median_ms(|| {
           std::hint::black_box(tree.rows(&|_| true, &[]));
       });
       let name_search_ms = median_ms(|| {
           std::hint::black_box(fastpad::library::name_search::search(&paths, "note 12", 500));
       });

       println!("notes={}", state.notes.len());
       println!("cold_ms={cold_ms:.1}");
       println!("warm_median_ms={warm_median_ms:.1}");
       println!("index_bytes~{index_bytes}");
       println!("tree_build_ms={tree_build_ms:.2}");
       println!("tree_rows_expanded_ms={tree_rows_expanded_ms:.2}");
       println!("name_search_ms={name_search_ms:.2}");
       if enforce_reference {
           let failures = [
               ("library-scan warm median", warm_median_ms, LIBRARY_SCAN_REFERENCE_MS),
               ("tree build", tree_build_ms, TREE_BUILD_REFERENCE_MS),
               ("tree rows, all expanded", tree_rows_expanded_ms, TREE_ROWS_REFERENCE_MS),
               ("name search", name_search_ms, NAME_SEARCH_REFERENCE_MS),
           ]
           .into_iter()
           .filter(|(_, measured, limit)| measured >= limit)
           .collect::<Vec<_>>();
           for (what, measured, limit) in &failures {
               eprintln!("reference threshold failed: {what}={measured:.2}ms (limit {limit}ms)");
           }
           if !failures.is_empty() {
               return Ok(2);
           }
       }
       Ok(0)
   ```

3. **The fixture's pins.** Rename `LIBRARY_SCAN_FAVORITES` to `LIBRARY_SCAN_PINS` everywhere, and change its doc comment to "Pinned records written into a generated library, spread evenly over its notes.". In `create_library_fixture`, the loop's first operation is `PendingOp::SetPinned { note: note.clone(), value: true }`. Task 1 already switched it from `SetFavorite` to compile. Change the function's doc comment to say "pinned records".

4. **Tests.** In `command_line_supports_run_and_compare_modes`, add `sidebar_view: None,` to the `Action::Run` literal. Add:

   ```rust
       #[test]
       fn command_line_supports_the_sidebar_view() {
           // Break caught: a mistyped view silently measuring the default layout, or the flag
           // swallowing the next option.
           assert!(matches!(
               parse_args(["--notes-folder", r"C:\n", "--sidebar-view", "none"]).unwrap(),
               Action::Run { sidebar_view: Some(view), .. } if view == "none"
           ));
           assert!(parse_args(["--sidebar-view", "tree"]).is_err());
           assert!(parse_args(["--sidebar-view"]).is_err());
       }
   ```

Run: `cargo test --bin fastpad-bench`
Expected: all pass.

- [ ] **Step 5: Run the bench once by hand and record the numbers**

The TTI runs use per-run scratch profiles. Nothing here reads or writes the real `fastpad.ini`.

```
cargo build --release --bin fastpad --bin fastpad-bench
target\release\fastpad-bench.exe library-scan %TEMP%\fastpad-10k --count 10000
target\release\fastpad-bench.exe --runs 30 --warmup 5 --output benchmarks\sidebar-none.jsonl
target\release\fastpad-bench.exe --runs 30 --warmup 5 --notes-folder %TEMP%\fastpad-10k --sidebar-view none --output benchmarks\sidebar-10k-closed.jsonl
target\release\fastpad-bench.exe --runs 30 --warmup 5 --notes-folder %TEMP%\fastpad-10k --sidebar-view notebook --output benchmarks\sidebar-10k-open.jsonl
target\release\fastpad-bench.exe compare benchmarks\sidebar-10k-closed.jsonl benchmarks\sidebar-10k-open.jsonl
```

Expected:
- `library-scan` reports `tree_build_ms` under 20, `tree_rows_expanded_ms` under 16 and `name_search_ms` under 5 on the reference i5-4590.
- `compare` reports no regressed milestone between the closed and open panel.
- `idle_private_working_set_bytes` p50 of the open-panel run is at most 1 MB (1,048,576 bytes) above the closed-panel run.

For the spec §12 gate against the library spec's own measurement, build `feat/note-library` in a separate worktree:
- run `git worktree add ..\FastPad-note-library feat/note-library`;
- copy `native\out` into it, because the worktree has no native build output;
- in the worktree, run `cargo build --release --bin fastpad --bin fastpad-bench`, then `target\release\fastpad-bench.exe --runs 30 --warmup 5 --notes-folder %TEMP%\fastpad-10k --output ..\FastPad\benchmarks\library-10k.jsonl`;
- run `fastpad-bench compare benchmarks\library-10k.jsonl benchmarks\sidebar-10k-open.jsonl`;
- remove the worktree with `git worktree remove ..\FastPad-note-library`.

That `compare` is also the gate for Task 6's one new startup cost, reading `fastpad.ini` in `bootstrap::run` before the window exists (Global Constraints). Expected: no regressed milestone, so warm startup (first paint and first input) is within noise of `feat/note-library`.

If `compare` reports a regressed first-paint or first-input milestone, move the settings read back after first paint:
1. In `src/bootstrap.rs`, delete the comment and the three lines Task 6 added after `app.instance_mutex = instance_mutex;` (`let (settings, warnings) = crate::config::load();` and the two assignments).
2. In `src/app.rs`, delete the `preloaded_settings_warnings` field and its `preloaded_settings_warnings: None,` initializer.
3. In `src/window/main_window.rs`, `load_settings` becomes:

```rust
/// Runs only inside `WM_FASTPAD_LOAD_SETTINGS`: resolves and parses `fastpad.ini`, applies the
/// editor view settings in place, and queues every rejected line as a non-modal notification.
fn load_settings(hwnd: HWND) {
    let (settings, warnings) = crate::config::load();
    apply_loaded_settings(hwnd, settings, warnings);
    start_recovery_timer(hwnd);
}
```

   The sidebar is then created from the compiled defaults in `initialize_editor_with`, and `apply_loaded_settings` (Task 6, item 10) reconciles it with the saved view, width and notes mode.
4. Rebuild, rerun the Step 5 commands, and add the Step 8 bullet for the fallback.

Paste all outputs into the PR description. Delete `%TEMP%\fastpad-10k` and the `benchmarks\*.jsonl` files from this step afterwards; they are not committed.

- [ ] **Step 6: Document the bench** in `benchmarks/README.md`. Append:

```markdown
## Note sidebar

`library-scan` also times the sidebar's pure work over the generated notebook (the median of
five runs each):
- `tree_build_ms`: `NoteTree::build` over every note. The target is under 20 ms for 10,000 notes.
- `tree_rows_expanded_ms`: flattening with every folder expanded (500 notes per folder). The
  target is under 16 ms.
- `name_search_ms`: one Search-view keystroke over every name. The target is under 5 ms.

`--enforce-reference` fails the run when any of them reaches its target.

Startup with the sidebar is measured with `--notes-folder DIR --sidebar-view notebook` against
the same folder with `--sidebar-view none`, and compared with `fastpad-bench compare`. No
milestone may regress, and the idle private working set with a 10,000-note notebook may grow by
at most 1 MB over the `feat/note-library` build with the same folder.
```

- [ ] **Step 7: Update `README.md`**

1. **Replace the section.** Replace the whole "### Notes and folders" section (heading and its three paragraphs) with:

   ```markdown
   ### Notes and notebooks

   Open any folder with **Ctrl+Shift+O** and it becomes your notebook. The sidebar on the left
   shows it as a tree: folders as they are on disk, pinned notes first. Click a note to open it
   in a preview tab that the next click replaces. Double-click it, or start typing, to keep it.
   Hover a note to pin it.

   - **Ctrl+N** gives you a new note, labelled by its first line as you type. The first Ctrl+S asks
     for its name inline and saves it into the notebook. Notes in the notebook save themselves
     from then on.
   - **Ctrl+K** searches note names. Star a notebook to keep it in **Favorites**, and switch
     between notebooks from there.
   - **Ctrl+B** hides or shows the sidebar, and **F6** moves between the sidebar and the editor.
     Everything is reachable from the keyboard and exposed to screen readers.

   Pins are stored in `.fastpad\library.ini` inside the notebook, so they travel with it. Nothing
   is written into a folder until you pin something. Turn it all off with `notes_mode=false` in
   `fastpad.ini`.
   ```

2. **Shortcuts table.** Replace the last row (`| Open folder | `Ctrl+Shift+O` | | | |`) with:

   ```markdown
   | Open notebook | `Ctrl+Shift+O` | | Toggle sidebar | `Ctrl+B` |
   | Show notebook | `Ctrl+Shift+E` | | Search notes | `Ctrl+K` |
   | Move note to notebook | `Ctrl+Shift+M` | | Sidebar / editor focus | `F6` / `Shift+F6` |
   ```

3. **Settings table.** After the `restore_session` row of the "Make it yours" table, add:

   ```markdown
   | `sidebar_view` | `notebook`, `search`, `favorites`, `none` | `notebook` |
   | `sidebar_width` | 180–480 (pixels at 100% scaling) | `260` |
   ```

4. **Settings button.** In "### A command palette for everything", add a last sentence: "The **Settings** button at the bottom of the sidebar opens the palette with just the settings."

- [ ] **Step 8: Update the sidebar spec.** In `docs/superpowers/specs/2026-09-23-note-sidebar-design.md`:

1. Set `**Status:** Approved design`.
2. Append:

   ```markdown
   ## 16. Implementation notes

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
     `WM_FASTPAD_LOAD_SETTINGS` only applies it and reports its warnings. The startup bench
     showed no regressed milestone. (If Step 5's fallback was taken, write instead: "The sidebar
     is created from the compiled defaults and reconciled once `WM_FASTPAD_LOAD_SETTINGS` applies
     `fastpad.ini`, because reading it before the window regressed warm startup by <measured> ms.")
   - **A favorite opens asynchronously.** The Favorites view shows the Notebook view once the
     worker has found the folder, so a missing favorite changes nothing, the view included.
   ```

3. Append one bullet per ruling recorded in the implementation ledger for this branch: each deviation from this spec accepted during Tasks 1–14, in the same one-sentence-plus-reason form.

- [ ] **Step 9: Commit**

```bash
git add tests/windows/library.rs src/bin/fastpad-bench.rs benchmarks/README.md README.md docs/superpowers/specs/2026-09-23-note-sidebar-design.md
git commit -m "test: note sidebar end to end; bench: tree, search and sidebar TTI; docs: the sidebar"
```

- [ ] **Step 10: Final verification**

1. Run `cargo fmt --all -- --check`. Expected: no diff.
2. Run `cargo clippy --all-targets -- -D warnings`. Expected: no warnings.
3. Close every FastPad window in the session. Then run the full suite once: `cargo test -- --test-threads=1`. Expected: every test passes, including all `tests/windows/*` targets.
4. Run the Step 5 bench commands again on the final commit. Expected: the thresholds hold.
5. **Manual check.** Before launching the release build against your own profile, copy `%LOCALAPPDATA%\FastPad\fastpad.ini` and `folders.ini` to a safe place, and restore both afterwards.
   1. Walk through the activity bar, the tree, search and favorites with Narrator: arrows, Enter, F6 and Esc.
   2. Repeat in high contrast.
   3. Check 150% and 200% display scaling.
