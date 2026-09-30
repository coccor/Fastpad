use crate::config::Settings;
use crate::document::{DocumentId, RecoveryId};
use crate::editor::Editor;
use crate::languages::LanguageManager;
use crate::launch::LaunchOptions;
use crate::perf::{Milestone, StartupMetrics};
use crate::platform::theme::SystemTheme;
use crate::window::accessibility::AccessibilityState;
use crate::window::command_palette::CommandPalette;
use crate::window::commands::CommandId;
use crate::window::editor_group::GroupWindow;
use crate::window::find_bar::FindBar;
use crate::window::menu_band::MenuMode;
use crate::window::menus::{AcceleratorTable, MenuBar};
use crate::window::notification::NotificationCenter;
use crate::window::split_tree::GroupId;
use crate::window::status::StatusModel;
use crate::window::tabs::Tabs;
use crate::window::titlebar::{LogoIcon, PointerState, TitleFonts};
use std::cell::Cell;
use std::ffi::c_void;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::IsChild;

#[derive(Clone, Debug)]
pub(crate) struct WindowIdentity {
    state: Rc<Cell<WindowIdentityState>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WindowIdentityState {
    Unbound,
    Live(HWND),
    Invalidated,
}

#[derive(Debug)]
pub struct App {
    pub hwnd: HWND,
    pub launch: LaunchOptions,
    pub startup: StartupMetrics,
    pub(crate) tabs: Tabs,
    /// The hidden Scintilla that creates every document and reads and edits background tabs
    /// (split editors spec §3.1).
    pub(crate) document_host: Option<Editor>,
    /// The editor group windows, each with its own editor, find bar, preview and image view, in
    /// creation order; the first exists once the editor does (split editors spec §4.2).
    pub(crate) groups: Vec<GroupWindow>,
    /// How the groups are arranged: rows and columns of groups (split editors spec §4.3).
    pub(crate) layout: crate::window::split_tree::SplitTree,
    /// The sash being dragged, from the press to the release.
    pub(crate) sash_drag: Option<crate::window::split_tree::Sash>,
    /// The last press on a sash and its message time, so a second press there within the
    /// double-click time equalizes its branch.
    pub(crate) last_sash_click: Option<(crate::window::split_tree::SashId, u32)>,
    /// A tab pressed on a strip, and dragged once past the drag distance (split editors spec §6).
    pub(crate) tab_drag: Option<crate::window::tab_drag::TabDrag>,
    /// The drop overlay of the drag under way: a tab drag or an Open Editors row drag.
    pub(crate) drop_overlay: Option<crate::window::drop_overlay::DropOverlay>,
    /// `BUILD_CHROME` wrapped the group editors' drop targets: a group made from now on gets its
    /// wrapper when it is created.
    pub(crate) file_drops_accepted: bool,
    /// The Direct2D factories every group's preview and image view share, created on first use.
    pub(crate) graphics: Option<Rc<crate::preview::dwrite::Graphics>>,
    pub(crate) accessibility: AccessibilityState,
    pub(crate) accelerators: Option<AcceleratorTable>,
    /// The shortcuts in force: the defaults until settings load, then with the user's overrides.
    pub(crate) keymap: crate::window::keymap::Keymap,
    pub(crate) menu_bar: Option<MenuBar>,
    /// Present while the Alt/F10 menu band is showing.
    pub(crate) menu_mode: Option<MenuMode>,
    /// Where focus returns when menu mode ends; the frame holds it meanwhile for the key handling.
    pub(crate) menu_return_focus: HWND,
    pub(crate) name_box: Option<crate::window::name_box::NameBox>,
    pub(crate) command_palette: Option<CommandPalette>,
    pub(crate) language_manager: Option<LanguageManager>,
    pub(crate) settings: Settings,
    pub(crate) theme: Option<SystemTheme>,
    pub(crate) status: Option<StatusModel>,
    pub(crate) title_fonts: Option<TitleFonts>,
    pub(crate) title_pointer: PointerState,
    /// The activity bar's logo icon, loaded for the window's DPI by the post-first-paint deferred
    /// chrome step (`main_window::build_chrome`) and reloaded on a DPI change.
    pub(crate) logo_icon: Option<LogoIcon>,
    pub(crate) dark_frame_applied: bool,
    pub(crate) notifications: NotificationCenter,
    pub(crate) launch_open_completed: bool,
    pub(crate) populating_file: bool,
    pub(crate) modal_depth: u32,
    pub(crate) held_messages: Vec<u32>,
    identity: WindowIdentity,
    first_paint_completed: bool,
    deferred_start_pending: bool,
    prioritize_input: bool,
    menu_alt_pending: bool,
    pub(crate) recovery_root: Option<std::path::PathBuf>,
    pub(crate) recovery_owner: Option<crate::platform::OwnedHandle>,
    /// `session.ini` for a primary window, resolved on first use; tests pre-seed it.
    pub(crate) session_path: Option<std::path::PathBuf>,
    /// Present while the last session's entries are still being reopened.
    pub(crate) session_restore: Option<crate::session::SessionRestore>,
    // Declared before `instance_mutex` so an emergency drop closes the pipe before releasing the
    // mutex: a new primary must never claim the session while this server still exists.
    pub(crate) ipc: Option<crate::ipc::IpcServer>,
    pub(crate) instance_mutex: Option<crate::platform::OwnedHandle>,
    pub(crate) ipc_requests: Vec<crate::ipc::IpcRequest>,
    pub(crate) last_snapshot_duration: Option<std::time::Duration>,
    pub(crate) last_snapshot_attempt: Option<DocumentId>,
    pub(crate) library: crate::window::library_host::LibraryHost,
    /// The Search view's text search: its debounce, generation, cancel flag and narrowing record.
    pub(crate) text_search: crate::window::text_search_host::TextSearchHost,
    /// The activity bar and side panel; present only in notes mode.
    pub(crate) sidebar: Option<crate::window::side_panel::Sidebar>,
    /// The warnings of the `fastpad.ini` that `bootstrap::run` read into `settings` before the
    /// window existed. `Some` until `WM_FASTPAD_LOAD_SETTINGS` reports them.
    pub(crate) preloaded_settings_warnings: Option<Vec<crate::config::SettingWarning>>,
    /// The sidebar's focused note when the command palette most recently opened while the panel
    /// had the keyboard focus (spec §6.3). Taken once by `run_command_palette_selection`, or
    /// discarded when the palette closes without running a command.
    pub(crate) palette_note_target: Option<std::path::PathBuf>,
    /// Where focus returns when the command palette closes, since opening it took focus away
    /// from the sidebar panel. `std::ptr::null_mut()` restores focus to the editor as before.
    pub(crate) palette_focus_return: HWND,
    next_document_id: u64,
    process_start: u64,
}

static NEXT_RECOVERY_COUNTER: AtomicU64 = AtomicU64::new(1);

impl App {
    pub fn new(launch: LaunchOptions, startup: StartupMetrics) -> Self {
        let process_start = startup.start_tick() as u64;
        let tabs = Tabs::new();
        let layout = crate::window::split_tree::SplitTree::new(tabs.active_group());
        Self {
            hwnd: std::ptr::null_mut(),
            launch,
            tabs,
            document_host: None,
            groups: Vec::new(),
            layout,
            sash_drag: None,
            tab_drag: None,
            drop_overlay: None,
            file_drops_accepted: false,
            last_sash_click: None,
            graphics: None,
            accessibility: AccessibilityState::default(),
            keymap: crate::window::keymap::Keymap::defaults(),
            accelerators: AcceleratorTable::create(&crate::window::keymap::Keymap::defaults()).ok(),
            menu_bar: None,
            menu_mode: None,
            menu_return_focus: std::ptr::null_mut(),
            name_box: None,
            command_palette: None,
            language_manager: None,
            settings: crate::config::default_settings(),
            theme: None,
            status: None,
            title_fonts: None,
            title_pointer: PointerState::default(),
            logo_icon: None,
            dark_frame_applied: false,
            notifications: NotificationCenter::new(),
            launch_open_completed: false,
            populating_file: false,
            modal_depth: 0,
            held_messages: Vec::new(),
            identity: WindowIdentity {
                state: Rc::new(Cell::new(WindowIdentityState::Unbound)),
            },
            first_paint_completed: false,
            deferred_start_pending: false,
            prioritize_input: false,
            menu_alt_pending: false,
            recovery_root: None,
            recovery_owner: None,
            session_path: None,
            session_restore: None,
            ipc: None,
            instance_mutex: None,
            ipc_requests: Vec::new(),
            last_snapshot_duration: None,
            last_snapshot_attempt: None,
            next_document_id: 2,
            library: crate::window::library_host::LibraryHost::new(process_start),
            text_search: Default::default(),
            sidebar: None,
            preloaded_settings_warnings: None,
            palette_note_target: None,
            palette_focus_return: std::ptr::null_mut(),
            process_start,
            startup,
        }
    }

    pub fn mark_first_paint_complete(&mut self) {
        if !self.first_paint_completed {
            let _ = self.startup.record_now(Milestone::FirstPaint);
            self.first_paint_completed = true;
            self.deferred_start_pending = true;
        }
    }

    pub fn take_deferred_start_pending(&mut self) -> bool {
        let pending = self.deferred_start_pending;
        self.deferred_start_pending = false;
        pending
    }

    pub fn request_input_priority(&mut self) {
        self.prioritize_input = true;
    }

    pub fn clear_input_priority(&mut self) {
        self.prioritize_input = false;
    }

    pub fn prioritizes_input(&self) -> bool {
        self.prioritize_input
    }

    pub(crate) fn set_menu_alt_pending(&mut self, pending: bool) {
        self.menu_alt_pending = pending;
    }

    pub(crate) fn take_menu_alt_pending(&mut self) -> bool {
        std::mem::take(&mut self.menu_alt_pending)
    }

    pub fn execute(&mut self, command: CommandId) {
        if command == CommandId::Exit && !self.hwnd.is_null() {
            unsafe {
                windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(
                    self.hwnd,
                    windows_sys::Win32::UI::WindowsAndMessaging::WM_CLOSE,
                    0,
                    0,
                );
            }
        }
    }

    // HWND-based orchestration deliberately does not borrow App across native callbacks.
    pub(crate) fn open_path(hwnd: HWND, path: &std::path::Path) -> crate::Result<()> {
        crate::window::open_path(hwnd, path)
    }

    pub(crate) fn allocate_document_identity(&mut self) -> (DocumentId, RecoveryId) {
        let id = DocumentId(self.next_document_id);
        self.next_document_id = self.next_document_id.saturating_add(1);
        (id, self.allocate_recovery_id())
    }

    pub(crate) fn allocate_recovery_id(&self) -> RecoveryId {
        let counter = NEXT_RECOVERY_COUNTER.fetch_add(1, Ordering::Relaxed);
        RecoveryId::compose(self.process_start, std::process::id(), counter)
    }

    pub(crate) fn recovery_owner_id(&self) -> RecoveryId {
        RecoveryId::compose(self.process_start, std::process::id(), 0)
    }

    pub(crate) fn owns_recovery_id(&self, id: RecoveryId) -> bool {
        id.is_from_process(self.process_start, std::process::id())
    }

    pub(crate) fn ensure_accessibility(&mut self) -> *mut c_void {
        self.accessibility.ensure(
            self.hwnd,
            crate::window::accessibility::ProviderKind::TitleBar,
            self.tabs.view(),
            self.tabs.selection(),
        )
    }

    pub(crate) fn active_group(&self) -> Option<&GroupWindow> {
        self.group(self.tabs.active_group())
    }

    pub(crate) fn active_group_mut(&mut self) -> Option<&mut GroupWindow> {
        let id = self.tabs.active_group();
        self.group_mut(id)
    }

    pub(crate) fn group(&self, id: GroupId) -> Option<&GroupWindow> {
        self.groups.iter().find(|group| group.id == id)
    }

    pub(crate) fn group_mut(&mut self, id: GroupId) -> Option<&mut GroupWindow> {
        self.groups.iter_mut().find(|group| group.id == id)
    }

    /// The active group's editor.
    pub(crate) fn editor(&self) -> Option<&Editor> {
        self.active_group().map(|group| &group.editor)
    }

    pub(crate) fn find_bar(&self) -> Option<&FindBar> {
        self.active_group()?.find_bar.as_ref()
    }

    pub(crate) fn find_bar_mut(&mut self) -> Option<&mut FindBar> {
        self.active_group_mut()?.find_bar.as_mut()
    }

    /// The group whose window is `hwnd` or holds it.
    pub(crate) fn group_containing(&self, hwnd: HWND) -> Option<GroupId> {
        self.groups
            .iter()
            .find(|group| group.hwnd == hwnd || unsafe { IsChild(group.hwnd, hwnd) } != 0)
            .map(|group| group.id)
    }

    pub(crate) fn window_identity(&self) -> WindowIdentity {
        self.identity.clone()
    }

    pub(crate) fn bind_window(&mut self, hwnd: HWND) -> bool {
        if self.identity.state.get() != WindowIdentityState::Unbound {
            return false;
        }
        self.hwnd = hwnd;
        self.identity.state.set(WindowIdentityState::Live(hwnd));
        true
    }

    pub(crate) fn invalidate_window(&self, hwnd: HWND) {
        if self.identity.state.get() == WindowIdentityState::Live(hwnd) {
            self.identity.state.set(WindowIdentityState::Invalidated);
        }
    }
}

impl WindowIdentity {
    pub(crate) fn is_live_for(&self, hwnd: HWND) -> bool {
        self.state.get() == WindowIdentityState::Live(hwnd)
    }

    #[cfg(test)]
    pub(crate) fn is_invalidated(&self) -> bool {
        self.state.get() == WindowIdentityState::Invalidated
    }
}

#[cfg(test)]
mod tests {
    use super::App;
    use crate::launch::LaunchOptions;
    use crate::perf::StartupMetrics;

    #[test]
    fn multiple_paints_schedule_deferred_start_only_once() {
        // Break caught: clearing the only first-paint latch lets later WM_PAINT messages restart
        // the deferred startup chain.
        let mut app = App::new(
            LaunchOptions::default(),
            StartupMetrics::with_frequency(1, 0),
        );

        app.mark_first_paint_complete();
        assert!(app.take_deferred_start_pending());

        app.mark_first_paint_complete();
        assert!(!app.take_deferred_start_pending());
    }
}
