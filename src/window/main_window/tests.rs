//! Tests for the main window. This file holds the imports and helpers the themed test
//! modules under `tests/` share; each of them starts with `use super::*;`.

use super::{
    MainWindowClass, WindowCreateContext, execute_command, handle_paint_with,
    mark_first_paint_complete, pump_posted_messages, sidebar_command_runs,
    take_deferred_start_pending, with_command_palette,
};
use crate::app::App;
use crate::document::{CloseDecision, Language, RecoveryId};
use crate::editor::scintilla_constants::SCI_GETMODIFY;
use crate::file::encoding::Encoding;
use crate::languages::LanguageManager;
use crate::launch::LaunchOptions;
use crate::library::tree::RowKind;
use crate::perf::StartupMetrics;
use crate::recovery::snapshot::snapshot_path;
use crate::recovery::{Snapshot, write_snapshot};
use crate::session::{Session, SessionEntry, SessionSource};
use crate::window::commands::CommandId;
use crate::window::menus::answer_next_popup_menu;
use crate::window::modal::{answer_next_close_prompt, answer_next_save_dialog};
use crate::window::notebook_view::{Activation, Mode, NotebookView};
use crate::window::split_tree::GroupId;
use crate::window::tree_drag::DragSource;
use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DestroyWindow, DispatchMessageW, GWLP_USERDATA, GetClientRect, GetWindowLongPtrW, IsWindow,
    MSG, PM_REMOVE, PeekMessageW, SendMessageW, WM_CLOSE, WM_PAINT,
};

mod about_and_settings_dialog;
mod command_palette;
mod copy_host_and_panel_drop;
mod editing_shortcuts;
mod find_bar;
mod first_save_and_autosave;
mod focus_and_accessibility;
mod folder_rename_and_tree_move;
mod group_strip;
mod inline_new_items;
mod json_and_recovery;
mod markdown_keys;
mod menus_and_settings_commands;
mod note_operations;
mod notebook_tree;
mod open_editors_and_folder_commands;
mod preview_tabs_and_folders;
mod search_replace;
mod search_replace_writes;
mod search_view;
mod session_restore;
mod settings_shortcuts_page;
mod sidebar_layout;
mod split_groups;
mod tab_drag;
mod tree_and_open_editors_drag;
mod window_basics;
mod window_lifecycle;

pub(super) fn active_path(hwnd: HWND) -> Option<std::path::PathBuf> {
    app_mut(hwnd)
        .tabs
        .active()
        .and_then(|document| document.path.clone())
}

pub(super) fn line_number_margin_width(editor: &crate::editor::Editor) -> isize {
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::SendMessageW(
            editor.hwnd(),
            crate::editor::scintilla_constants::SCI_GETMARGINWIDTHN,
            0,
            0,
        )
    }
}

pub(super) fn app_mut<'a>(hwnd: HWND) -> &'a mut App {
    unsafe { super::app_ptr(hwnd).unwrap().as_mut() }
}

/// The non-modal notification messages currently queued on the window (spec 239).
pub(super) fn notices(hwnd: HWND) -> Vec<String> {
    app_mut(hwnd)
        .notifications
        .pending()
        .iter()
        .map(|notice| notice.message.clone())
        .collect()
}

pub(super) fn read_snapshot_text(path: &std::path::Path) -> String {
    Snapshot::decode(&std::fs::read(path).unwrap())
        .unwrap()
        .text
}

pub(super) struct RecoveryScratch(PathBuf);

impl RecoveryScratch {
    pub(super) fn new(label: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "fastpad-window-recovery-{label}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    pub(super) fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for RecoveryScratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Installs a real Scintilla editor onto `window` (mirroring
/// `failed_language_activation_leaves_document_language_unchanged_and_records_a_warning`'s own
/// setup) and returns it for direct `text`/`set_text`/`selection` calls in JSON command tests.
pub(super) fn install_test_editor(window: &ProductionWindow) -> crate::editor::Editor {
    let identity = unsafe { super::window_identity(window.hwnd).unwrap() };
    unsafe { super::initialize_editor_with(window.hwnd, &identity, crate::editor::Editor::create) }
        .unwrap();
    // What the first WM_SIZE does once `bootstrap::run` shows the window.
    super::layout_editor_and_find_bar(window.hwnd);
    unsafe { super::app_ptr(window.hwnd).unwrap().as_ref() }
        .editor()
        .cloned()
        .unwrap()
}

pub(super) fn sidebar_windows(hwnd: HWND) -> (HWND, HWND) {
    let sidebar = app_mut(hwnd)
        .sidebar
        .as_ref()
        .expect("notes mode shows the sidebar");
    (sidebar.bar, sidebar.panel)
}

pub(super) fn client_lparam(x: i32, y: i32) -> super::LPARAM {
    ((y as u32) << 16 | (x as u32 & 0xffff)) as super::LPARAM
}

/// `window`'s client point `x`, `y` as a screen-coordinate `lParam`, as WM_NCHITTEST gets it.
pub(super) fn screen_lparam(window: HWND, x: i32, y: i32) -> super::LPARAM {
    let mut point = windows_sys::Win32::Foundation::POINT { x, y };
    unsafe { windows_sys::Win32::Graphics::Gdi::ClientToScreen(window, &mut point) };
    client_lparam(point.x, point.y)
}

pub(super) fn client_size(window: HWND) -> (i32, i32) {
    let mut rect = RECT::default();
    unsafe { GetClientRect(window, &mut rect) };
    (rect.right, rect.bottom)
}

/// `child`'s left edge in `parent`'s client coordinates.
pub(super) fn left_of(child: HWND, parent: HWND) -> i32 {
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
pub(super) fn is_shown(window: HWND) -> bool {
    (unsafe { GetWindowLongPtrW(window, super::GWL_STYLE) }) as u32 & super::WS_VISIBLE != 0
}

pub(super) fn click(window: HWND, x: i32, y: i32) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{WM_LBUTTONDOWN, WM_LBUTTONUP};
    unsafe {
        SendMessageW(window, WM_LBUTTONDOWN, 0, client_lparam(x, y));
        SendMessageW(window, WM_LBUTTONUP, 0, client_lparam(x, y));
    }
}

pub(super) fn unnamed_mutex() -> crate::platform::OwnedHandle {
    let raw = unsafe {
        windows_sys::Win32::System::Threading::CreateMutexW(std::ptr::null(), 0, std::ptr::null())
    };
    unsafe { crate::platform::OwnedHandle::from_raw_owned(raw) }.unwrap()
}

pub(super) fn make_app() -> Box<App> {
    Box::new(App::new(
        LaunchOptions::default(),
        StartupMetrics::with_frequency(1, 0),
    ))
}

pub(super) fn load_native_scintilla() -> crate::platform::OwnedModule {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("native/out/x64/Scintilla.dll");
    let path = crate::platform::wide_null(path.to_str().unwrap());
    let module = unsafe {
        windows_sys::Win32::System::LibraryLoader::LoadLibraryExW(
            path.as_ptr(),
            std::ptr::null_mut(),
            windows_sys::Win32::System::LibraryLoader::LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR
                | windows_sys::Win32::System::LibraryLoader::LOAD_LIBRARY_SEARCH_SYSTEM32,
        )
    };
    unsafe { crate::platform::OwnedModule::from_raw_owned(module) }.unwrap()
}

pub(super) struct ProductionWindow {
    pub(super) hwnd: HWND,
    pub(super) _class: MainWindowClass,
}

impl ProductionWindow {
    pub(super) fn new(app: Box<App>) -> Self {
        let instance = unsafe { GetModuleHandleW(std::ptr::null()) };
        let class = MainWindowClass::register(instance).unwrap();
        let mut context = WindowCreateContext::new(app);
        let hwnd = class.create(&mut context).unwrap();

        Self {
            hwnd,
            _class: class,
        }
    }
}

impl Drop for ProductionWindow {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

/// Makes the window a primary instance saving its session under `scratch`.
pub(super) fn enable_session(hwnd: HWND, scratch: &RecoveryScratch) {
    let recovery = scratch.path().join("Recovery");
    std::fs::create_dir_all(&recovery).unwrap();
    let app = app_mut(hwnd);
    app.instance_mutex = Some(unnamed_mutex());
    app.recovery_root = Some(recovery);
    app.session_path = Some(scratch.path().join("session.ini"));
}

pub(super) fn write_session(scratch: &RecoveryScratch, entries: Vec<SessionEntry>, active: usize) {
    crate::session::write(
        &scratch.path().join("session.ini"),
        &Session::single(active, entries),
    )
    .unwrap();
}

/// Runs only the session unit until it hands over to `WM_FASTPAD_OPEN_LIBRARY`, without
/// pumping the rest of the chain (which would bind the real single-instance pipe).
pub(super) fn run_session_restore(hwnd: HWND) {
    use windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW;
    let restore = crate::window::WM_FASTPAD_RESTORE_SESSION;
    unsafe { PostMessageW(hwnd, restore, 0, 0) };
    let mut message = MSG::default();
    while unsafe { PeekMessageW(&mut message, hwnd, restore, restore, PM_REMOVE) } != 0 {
        unsafe { DispatchMessageW(&message) };
    }
}

/// Removes every queued `message` for `hwnd` without dispatching it.
pub(super) fn discard_posted(hwnd: HWND, message: u32) {
    let mut queued = MSG::default();
    while unsafe { PeekMessageW(&mut queued, hwnd, message, message, PM_REMOVE) } != 0 {}
}

pub(super) struct LibraryScratch {
    pub(super) root: std::path::PathBuf,
}

impl LibraryScratch {
    pub(super) fn new(label: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("fastpad-libhost-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("notes")).unwrap();
        std::fs::create_dir_all(root.join("data")).unwrap();
        Self { root }
    }
    pub(super) fn folder(&self) -> std::path::PathBuf {
        self.root.join("notes")
    }
    pub(super) fn data(&self) -> std::path::PathBuf {
        self.root.join("data")
    }
    pub(super) fn note(&self, name: &str, text: &str) -> std::path::PathBuf {
        let path = self.folder().join(name);
        std::fs::write(&path, text).unwrap();
        path
    }
    /// Loads the folder synchronously and installs it, as LIBRARY_READY would.
    pub(super) fn install(&self, hwnd: HWND) {
        let local = crate::library::local::local_file(&self.data(), &self.folder());
        let state =
            crate::library::load(&self.folder(), &local, crate::library::now_unix()).unwrap();
        crate::window::library_host::install_for_test(hwnd, state);
    }
}

impl Drop for LibraryScratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// Pumps posted messages until `done` or 5 s.
pub(super) fn pump_until(hwnd: HWND, done: impl Fn() -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !done() {
        assert!(std::time::Instant::now() < deadline, "timed out");
        pump_posted_messages(hwnd);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

pub(super) fn tab_paths(hwnd: HWND) -> Vec<Option<std::path::PathBuf>> {
    app_mut(hwnd)
        .tabs
        .documents()
        .map(|document| document.path.clone())
        .collect()
}

pub(super) fn type_into_name_box(hwnd: HWND, text: &str) {
    let edit = app_mut(hwnd).name_box.as_ref().unwrap().edit_hwnd();
    let wide = crate::platform::wide_null(text);
    unsafe { windows_sys::Win32::UI::WindowsAndMessaging::SetWindowTextW(edit, wide.as_ptr()) };
}

pub(super) fn open_first_save_box(
    label: &str,
) -> (LibraryScratch, ProductionWindow, crate::editor::Editor) {
    let scratch = LibraryScratch::new(label);
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    super::create_new_document(window.hwnd).unwrap();
    editor.set_text("Draft").unwrap();
    execute_command(window.hwnd, CommandId::Save);
    assert!(app_mut(window.hwnd).name_box.as_ref().unwrap().is_visible());
    (scratch, window, editor)
}

pub(super) fn name_box_visible(hwnd: HWND) -> bool {
    app_mut(hwnd)
        .name_box
        .as_ref()
        .is_some_and(|name_box| name_box.is_visible())
}

pub(super) fn open_note(
    window: &ProductionWindow,
    scratch: &LibraryScratch,
    name: &str,
    text: &str,
) -> std::path::PathBuf {
    let path = scratch.note(name, text);
    scratch.install(window.hwnd);
    super::open_path(window.hwnd, &path).unwrap();
    pump_posted_messages(window.hwnd);
    path
}

pub(super) fn library(hwnd: HWND) -> &'static crate::library::model::Library {
    &app_mut(hwnd).library.state.as_ref().unwrap().library
}

/// Task 6 creates the sidebar with the window when notes mode is on; this makes sure of it.
pub(super) fn ensure_sidebar(hwnd: HWND) {
    if app_mut(hwnd).sidebar.is_none() {
        crate::window::side_panel::notes_mode_changed(hwnd, true);
    }
}

pub(super) fn notebook_view<'a>(hwnd: HWND) -> &'a mut NotebookView {
    &mut app_mut(hwnd).sidebar.as_mut().unwrap().notebook
}

pub(super) fn row_of(hwnd: HWND, kind: &RowKind) -> usize {
    crate::library::tree::row_index(&notebook_view(hwnd).rows, kind)
        .unwrap_or_else(|| panic!("{kind:?} is not in {:?}", notebook_view(hwnd).rows))
}

pub(super) fn selected_kind(hwnd: HWND) -> Option<RowKind> {
    let view = notebook_view(hwnd);
    view.list
        .selected
        .and_then(|index| view.rows.get(index))
        .map(|row| row.kind.clone())
}

pub(super) fn select_row(hwnd: HWND, kind: &RowKind) {
    let index = row_of(hwnd, kind);
    notebook_view(hwnd).list.selected = Some(index);
}

/// A window with a sidebar showing `scratch`'s notebook in the Notebook view.
pub(super) fn notebook_window(
    scratch: &LibraryScratch,
) -> (ProductionWindow, crate::editor::Editor) {
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    ensure_sidebar(window.hwnd);
    scratch.install(window.hwnd);
    crate::window::side_panel::show_view(window.hwnd, crate::config::SidebarView::Notebook, false);
    crate::window::notebook_view::rebuild(window.hwnd);
    (window, editor)
}

pub(super) fn inline_field(hwnd: HWND) -> HWND {
    crate::window::inline_name::field_hwnd(hwnd).expect("the name field was made")
}

pub(super) fn inline_open(hwnd: HWND) -> bool {
    crate::window::inline_name::is_open(hwnd)
}

/// Types `text` into the name field as a paste would: the Edit sends EN_CHANGE to the panel.
pub(super) fn type_into_field(hwnd: HWND, text: &str) {
    let wide = crate::platform::wide_null(text);
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::SetWindowTextW(
            inline_field(hwnd),
            wide.as_ptr(),
        )
    };
}

pub(super) fn field_key(hwnd: HWND, key: u16) {
    unsafe {
        SendMessageW(
            inline_field(hwnd),
            windows_sys::Win32::UI::WindowsAndMessaging::WM_KEYDOWN,
            usize::from(key),
            0,
        )
    };
}

/// A new untitled tab (Ctrl+N) whose first save goes to `folder`, as if that folder's row
/// had been selected when it was made.
pub(super) fn untitled_tab_saving_in(hwnd: HWND, folder: std::path::PathBuf) {
    execute_command(hwnd, CommandId::New);
    let tabs = &mut app_mut(hwnd).tabs;
    let id = tabs.active().unwrap().id;
    tabs.document_mut(id).unwrap().save_folder = Some(folder);
}

pub(super) fn field_text(hwnd: HWND) -> String {
    let mut buffer = [0u16; 260];
    let copied = unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::GetWindowTextW(
            inline_field(hwnd),
            buffer.as_mut_ptr(),
            buffer.len() as i32,
        )
    };
    String::from_utf16_lossy(&buffer[..copied.max(0) as usize])
}

pub(super) fn field_selection(hwnd: HWND) -> (u32, u32) {
    let (mut start, mut end) = (0_u32, 0_u32);
    unsafe {
        SendMessageW(
            inline_field(hwnd),
            windows_sys::Win32::UI::Controls::EM_GETSEL,
            &mut start as *mut u32 as usize,
            &mut end as *mut u32 as isize,
        )
    };
    (start, end)
}

/// The draft row's index and depth, while one shows.
pub(super) fn draft_row(hwnd: HWND) -> Option<(usize, u16)> {
    let rows = &notebook_view(hwnd).rows;
    rows.iter()
        .position(|row| row.kind == RowKind::Draft)
        .map(|index| (index, rows[index].depth))
}

/// The middle of the row showing `kind`, as a panel mouse message's `lParam`.
pub(super) fn row_lparam(hwnd: HWND, kind: &RowKind) -> super::LPARAM {
    let index = row_of(hwnd, kind);
    let rect = notebook_view(hwnd).row_rect_at(index).unwrap();
    client_lparam((rect.left + rect.right) / 2, (rect.top + rect.bottom) / 2)
}

pub(super) fn rescan_and_wait(hwnd: HWND) {
    crate::window::library_host::request_rescan(hwnd);
    pump_until(hwnd, || !app_mut(hwnd).library.scanning);
}

pub(super) fn sidebar_panel(hwnd: HWND) -> HWND {
    crate::window::side_panel::windows(hwnd).unwrap().1
}

pub(super) fn type_into_search(hwnd: HWND, text: &str) {
    let edit = crate::window::search_view::edit_hwnd(hwnd).unwrap();
    let wide = crate::platform::wide_null(text);
    // The Edit sends EN_CHANGE to the panel, which restarts the 150 ms debounce.
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::SetWindowTextW(edit, wide.as_ptr());
    }
}

pub(super) fn type_into_replace(hwnd: HWND, text: &str) {
    let edit = crate::window::search_view::replace_edit_hwnd(hwnd).unwrap();
    let wide = crate::platform::wide_null(text);
    // The Edit sends EN_CHANGE to the panel, which keeps the text; no search runs.
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::SetWindowTextW(edit, wide.as_ptr());
    }
}

pub(super) fn search_state(hwnd: HWND) -> crate::window::search_view::SearchState {
    crate::window::search_view::search_state(hwnd)
}

pub(super) fn search_generation(hwnd: HWND) -> u64 {
    crate::window::text_search_host::generation(hwnd)
}

/// Waits until a search that began after generation `after` has finished.
pub(super) fn wait_for_search(hwnd: HWND, after: u64) {
    pump_until(hwnd, || {
        search_generation(hwnd) != after
            && matches!(
                search_state(hwnd),
                crate::window::search_view::SearchState::Done { .. }
            )
    });
}

/// Types `text` into the Search box and waits past the debounce for its search to finish.
pub(super) fn search_for(hwnd: HWND, text: &str) {
    type_into_search(hwnd, text);
    wait_for_search(hwnd, search_generation(hwnd));
}

/// How many notes the finished search visited.
pub(super) fn searched_total(hwnd: HWND) -> usize {
    match search_state(hwnd) {
        crate::window::search_view::SearchState::Done { progress, .. } => progress.total,
        other => panic!("the search has not finished: {other:?}"),
    }
}

pub(super) fn search_rows(hwnd: HWND) -> Vec<(String, String)> {
    crate::window::search_view::shown_results(hwnd)
}

pub(super) fn search_row(name: &str, snippet: &str) -> (String, String) {
    (name.to_owned(), snippet.to_owned())
}

/// Pumps posted messages, timers included, for twice the debounce.
pub(super) fn pump_past_debounce(hwnd: HWND) {
    let wait = 2 * u64::from(crate::window::text_search_host::DEBOUNCE_MS);
    let until = std::time::Instant::now() + std::time::Duration::from_millis(wait);
    while std::time::Instant::now() < until {
        pump_posted_messages(hwnd);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

pub(super) fn search_selected(hwnd: HWND) -> Option<usize> {
    app_mut(hwnd).sidebar.as_ref().unwrap().search.list.selected
}

pub(super) fn selected_name(hwnd: HWND) -> Option<String> {
    let index = search_selected(hwnd)?;
    search_rows(hwnd).get(index).map(|(name, _)| name.clone())
}

pub(super) fn stray_hit(name: &str) -> crate::library::text_search::TextHit {
    crate::library::text_search::TextHit {
        path: PathBuf::from(format!("{name}.md")),
        name: name.to_owned(),
        folder: String::new(),
        snippet: crate::search::Snippet {
            text: format!("{name} needle"),
            highlight: name.len() + 1..name.len() + 7,
        },
        stamp: None,
    }
}

/// Opens the replace field, runs the search for `query` to its end, and types `replacement`.
pub(super) fn search_to_replace(hwnd: HWND, query: &str, replacement: &str) {
    crate::window::search_view::show_replace(hwnd);
    search_for(hwnd, query);
    type_into_replace(hwnd, replacement);
}

/// Pumps until a replace report is pushed, and returns it.
pub(super) fn wait_for_report(hwnd: HWND) -> String {
    let report = || {
        notices(hwnd)
            .into_iter()
            .find(|notice| notice.starts_with("Replaced "))
    };
    pump_until(hwnd, || report().is_some());
    report().unwrap()
}

/// Queues a No for the next question and returns whether it was asked.
pub(super) fn decline_next_confirm() -> std::rc::Rc<std::cell::Cell<bool>> {
    let asked = std::rc::Rc::new(std::cell::Cell::new(false));
    let answered = std::rc::Rc::clone(&asked);
    crate::window::answer_next_confirm(move |_| {
        answered.set(true);
        false
    });
    asked
}

pub(super) const SAVED_LINE: &str = "\nNotes that aren't open are saved and can't be undone.";

/// Runs `key` with the given modifiers through the accelerator table, as the message loop
/// does, and reports the command it ran.
pub(super) fn translate_key_with(
    hwnd: HWND,
    target: HWND,
    key: u8,
    ctrl: bool,
    shift: bool,
    alt: bool,
) -> Option<CommandId> {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetKeyboardState, SetKeyboardState, VK_CONTROL, VK_MENU, VK_SHIFT,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{MSG, WM_KEYDOWN, WM_SYSKEYDOWN};
    let identity = unsafe { super::window_identity(hwnd).unwrap() };
    let mut keys = [0u8; 256];
    unsafe { GetKeyboardState(keys.as_mut_ptr()) };
    let original = keys;
    let down = |on: bool| if on { 0x80 } else { 0 };
    keys[VK_CONTROL as usize] = down(ctrl);
    keys[VK_SHIFT as usize] = down(shift);
    keys[VK_MENU as usize] = down(alt);
    unsafe { SetKeyboardState(keys.as_ptr()) };
    let message = MSG {
        hwnd: target,
        message: if alt { WM_SYSKEYDOWN } else { WM_KEYDOWN },
        wParam: usize::from(key),
        // Bit 29: the Alt key was down, as the system reports it.
        lParam: if alt { 1 << 29 } else { 0 },
        ..Default::default()
    };
    super::LAST_COMMAND.with(|last| last.set(None));
    let translated = unsafe { super::translate_accelerator(hwnd, &identity, &message) };
    unsafe { SetKeyboardState(original.as_ptr()) };
    let command = super::LAST_COMMAND.with(std::cell::Cell::get);
    translated.then_some(command).flatten()
}

/// Sends `key` to `window` as a key press with Ctrl and Shift held as given.
pub(super) fn press_with(window: HWND, key: u16, ctrl: bool, shift: bool, alt: bool) {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetKeyboardState, SetKeyboardState, VK_CONTROL, VK_MENU, VK_SHIFT,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{WM_KEYDOWN, WM_SYSKEYDOWN};
    let mut keys = [0u8; 256];
    unsafe { GetKeyboardState(keys.as_mut_ptr()) };
    let original = keys;
    keys[VK_CONTROL as usize] = if ctrl { 0x80 } else { 0 };
    keys[VK_SHIFT as usize] = if shift { 0x80 } else { 0 };
    keys[VK_MENU as usize] = if alt { 0x80 } else { 0 };
    unsafe { SetKeyboardState(keys.as_ptr()) };
    let message = if alt { WM_SYSKEYDOWN } else { WM_KEYDOWN };
    unsafe { SendMessageW(window, message, usize::from(key), 0) };
    unsafe { SetKeyboardState(original.as_ptr()) };
}

pub(super) fn focused() -> HWND {
    unsafe { windows_sys::Win32::UI::Input::KeyboardAndMouse::GetFocus() }
}

pub(super) fn shown_window() -> ProductionWindow {
    let window = ProductionWindow::new(make_app());
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::ShowWindow(
            window.hwnd,
            windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOW,
        );
    }
    window
}

pub(super) fn mouse(panel: HWND, message: u32, buttons: usize, lparam: super::LPARAM) {
    unsafe { SendMessageW(panel, message, buttons, lparam) };
}

pub(super) fn drag_over(panel: HWND, lparam: super::LPARAM) {
    mouse(
        panel,
        windows_sys::Win32::UI::WindowsAndMessaging::WM_MOUSEMOVE,
        1,
        lparam,
    );
}

pub(super) fn drop_at(panel: HWND, lparam: super::LPARAM) {
    mouse(
        panel,
        windows_sys::Win32::UI::WindowsAndMessaging::WM_LBUTTONUP,
        0,
        lparam,
    );
}

/// Presses on Open Editors row `index` and moves past the drag distance.
pub(super) fn start_tab_drag(hwnd: HWND, panel: HWND, index: usize) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{WM_LBUTTONDOWN, WM_MOUSEMOVE};
    let rect = notebook_view(hwnd).editor_rect_at(index).unwrap();
    let (x, y) = ((rect.left + rect.right) / 3, (rect.top + rect.bottom) / 2);
    mouse(panel, WM_LBUTTONDOWN, 1, client_lparam(x, y));
    mouse(panel, WM_MOUSEMOVE, 1, client_lparam(x, y + 40));
}

/// The centre of `rect` as a panel `lParam`.
pub(super) fn centre(rect: RECT) -> super::LPARAM {
    client_lparam((rect.left + rect.right) / 2, (rect.top + rect.bottom) / 2)
}

/// Opens `path` with no sharing, so a copy of it fails, until the handle is dropped.
pub(super) fn locked(path: &std::path::Path) -> std::fs::File {
    use std::os::windows::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(path)
        .unwrap()
}

/// Window `to`'s client point (`x`, `y`) in window `from`'s client coordinates, as an
/// `lParam`: where the source group, which has the capture, sees the pointer.
pub(super) fn lparam_in(from: HWND, to: HWND, x: i32, y: i32) -> super::LPARAM {
    let mut point = windows_sys::Win32::Foundation::POINT { x, y };
    unsafe { windows_sys::Win32::Graphics::Gdi::MapWindowPoints(to, from, &mut point, 1) };
    client_lparam(point.x, point.y)
}

pub(super) fn content(hwnd: HWND, id: GroupId) -> RECT {
    let area = super::with_group_id(hwnd, id, |state| state.content).unwrap();
    RECT {
        left: area.left,
        top: area.top,
        right: area.right,
        bottom: area.bottom,
    }
}
