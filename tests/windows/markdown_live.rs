#![cfg(windows)]
mod support;

// Build the crate in-process with cfg(test), as markdown_preview.rs does, so tests can reach App.
include!("../../src/lib.rs");

use crate::document::Language;
use crate::editor::scintilla_constants::{SCI_SETSAVEPOINT, SCI_SETTEXT};
use crate::live::view::LiveView;
use crate::window::commands::CommandId;
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::Graphics::Gdi::UpdateWindow;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetFocus, SetFocus};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, IsWindowVisible, MSG, PM_QS_INPUT, PM_REMOVE, PeekMessageW, SendMessageW,
    TranslateMessage, WM_COMMAND,
};

struct TestMain {
    hwnd: HWND,
    identity: app::WindowIdentity,
    _class: window::MainWindowClass,
}

impl TestMain {
    fn new() -> Self {
        let app = app::App::new(
            launch::LaunchOptions::default(),
            perf::StartupMetrics::begin().unwrap(),
        );
        let identity = app.window_identity();
        let instance = unsafe {
            windows_sys::Win32::System::LibraryLoader::GetModuleHandleW(std::ptr::null())
        };
        let class = window::MainWindowClass::register(instance).unwrap();
        let mut context = window::WindowCreateContext::new(Box::new(app));
        let hwnd = class.create(&mut context).unwrap();
        unsafe {
            window::initialize_editor_with(hwnd, &identity, editor::Editor::create).unwrap();
            windows_sys::Win32::UI::WindowsAndMessaging::ShowWindow(
                hwnd,
                windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOW,
            );
        }
        Self {
            hwnd,
            identity,
            _class: class,
        }
    }

    fn with_app<R>(&self, run: impl FnOnce(&mut app::App) -> R) -> R {
        assert!(self.identity.is_live_for(self.hwnd));
        let raw = unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::GetWindowLongPtrW(
                self.hwnd,
                windows_sys::Win32::UI::WindowsAndMessaging::GWLP_USERDATA,
            )
        } as *mut app::App;
        assert!(!raw.is_null());
        run(unsafe { &mut *raw })
    }

    fn command(&self, command: CommandId) {
        unsafe { SendMessageW(self.hwnd, WM_COMMAND, command as usize, 0) };
        pump_pending();
    }

    /// The active group's editor.
    fn editor(&self) -> editor::Editor {
        self.with_app(|app| app.active_group().unwrap().editor.clone())
    }

    fn group(&self) -> window::split_tree::GroupId {
        self.with_app(|app| app.tabs.active_group())
    }

    fn set_text(&self, text: &str) {
        let bytes = std::ffi::CString::new(text).unwrap();
        unsafe {
            SendMessageW(
                self.editor().hwnd(),
                SCI_SETTEXT,
                0,
                bytes.as_ptr() as isize,
            )
        };
    }

    /// Makes the active tab Markdown without loading Lexilla, then lets the host react.
    fn make_markdown(&self, text: &str) {
        self.set_text(text);
        self.with_app(|app| app.tabs.set_active_language(Language::Markdown));
        window::preview_host::sync_visibility(self.hwnd);
    }

    /// Marks the active document saved, so closing it asks nothing.
    fn mark_clean(&self) {
        unsafe { SendMessageW(self.editor().hwnd(), SCI_SETSAVEPOINT, 0, 0) };
        pump_pending();
    }

    fn set_live(&self, on: bool) {
        window::live_host::set_live(self.hwnd, self.group(), on);
        pump_pending();
    }

    /// The active group's Live view, whether or not it is shown.
    fn live_view(&self) -> Option<LiveView> {
        self.with_app(|app| app.active_group().unwrap().live.view_for_test())
    }

    fn notices(&self) -> Vec<String> {
        self.with_app(|app| {
            app.notifications
                .pending()
                .iter()
                .map(|notice| notice.message.clone())
                .collect()
        })
    }
}

impl Drop for TestMain {
    fn drop(&mut self) {
        if self.identity.is_live_for(self.hwnd) {
            unsafe { windows_sys::Win32::UI::WindowsAndMessaging::DestroyWindow(self.hwnd) };
        }
    }
}

fn pump_until(what: &str, timeout: Duration, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + timeout;
    loop {
        pump_pending();
        if condition() {
            return;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Dispatches everything queued, input first, as FastPad's own loop does (see
/// `markdown_preview.rs`'s `pump_pending` for why plain peeking livelocks).
fn pump_pending() {
    let mut msg = MSG::default();
    loop {
        while unsafe {
            PeekMessageW(
                &mut msg,
                std::ptr::null_mut(),
                0,
                0,
                PM_REMOVE | PM_QS_INPUT,
            )
        } != 0
        {
            unsafe {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        if unsafe { PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) } == 0 {
            return;
        }
        unsafe {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

fn visible(hwnd: HWND) -> bool {
    unsafe { IsWindowVisible(hwnd) != 0 }
}

#[test]
fn live_view_replaces_scintilla_and_back() {
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    main.make_markdown("# Title\n\nSome text.\n");
    let editor = main.editor();
    unsafe { SetFocus(editor.hwnd()) };

    main.set_live(true);
    let view = main.live_view().expect("a Live window");
    assert!(visible(view.hwnd()));
    assert!(!visible(editor.hwnd()));
    assert_eq!(unsafe { GetFocus() }, view.hwnd());

    main.set_live(false);
    assert!(!visible(view.hwnd()));
    assert!(visible(editor.hwnd()));
    assert_eq!(unsafe { GetFocus() }, editor.hwnd());
}

#[test]
fn toggling_does_not_dirty_or_add_undo() {
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    main.make_markdown("text\n");
    let editor = main.editor();
    let _ = editor.populate_clean("one\ntwo\n");
    main.mark_clean();
    assert!(!editor.can_undo().unwrap());
    let dirty = || main.with_app(|app| app.tabs.active().unwrap().dirty);
    assert!(!dirty());

    main.set_live(true);
    main.set_live(false);
    main.set_live(true);
    assert!(!editor.can_undo().unwrap());
    assert!(!dirty());
    assert_eq!(editor.text().unwrap(), "one\ntwo\n");
}

#[test]
fn switching_tabs_and_preview_modes_keeps_scintilla_hidden_in_live() {
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    main.make_markdown("# Live\n");
    main.set_live(true);
    let editor = main.editor();
    let view = main.live_view().unwrap();

    // A new plain tab shows Scintilla; going back shows Live again.
    main.command(CommandId::New);
    assert!(visible(editor.hwnd()) && !visible(view.hwnd()));
    main.command(CommandId::PreviousTab);
    assert!(!visible(editor.hwnd()) && visible(view.hwnd()));

    // Split keeps Live on the left; Full hides both; back to Split shows Live.
    main.command(CommandId::MarkdownPreviewSide);
    assert!(!visible(editor.hwnd()) && visible(view.hwnd()));
    main.command(CommandId::MarkdownPreviewFull);
    assert!(!visible(editor.hwnd()) && !visible(view.hwnd()));
    main.command(CommandId::MarkdownPreviewSide);
    assert!(!visible(editor.hwnd()) && visible(view.hwnd()));
    main.command(CommandId::MarkdownPreviewClose);
    assert!(!visible(editor.hwnd()) && visible(view.hwnd()));
}

#[test]
fn a_tab_that_is_not_markdown_stays_in_source_with_a_notice() {
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    main.set_text("plain text");
    main.set_live(true);
    assert!(main.live_view().is_none());
    assert!(visible(main.editor().hwnd()));
    assert!(
        main.notices()
            .iter()
            .any(|notice| notice.starts_with("Live Markdown is available for Markdown"))
    );
}

#[test]
fn a_note_over_the_size_limit_stays_in_source_with_a_notice() {
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    main.make_markdown(&"x".repeat(live::LIVE_MAX_BYTES + 1));
    main.set_live(true);
    assert!(main.live_view().is_none());
    assert!(visible(main.editor().hwnd()));
    assert!(
        main.notices()
            .iter()
            .any(|notice| notice.contains("up to 4 MB"))
    );
}

#[test]
fn closing_the_tab_forgets_its_live_choice() {
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    main.command(CommandId::New);
    main.make_markdown("# Closing\n");
    main.set_live(true);
    let choices = || main.with_app(|app| app.active_group().unwrap().live.choice_count());
    assert_eq!(choices(), 1);
    main.mark_clean();
    main.command(CommandId::CloseTab);
    pump_until("the tab to close", Duration::from_secs(2), || {
        choices() == 0
    });
}

#[test]
fn live_lays_out_and_paints_the_visible_lines() {
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    main.make_markdown("first line\nsecond line\r\nthird é😀\n");
    main.set_live(true);
    let view = main.live_view().unwrap();
    unsafe { UpdateWindow(view.hwnd()) };
    pump_pending();
    let lines = view.debug_lines();
    assert_eq!(
        lines,
        vec![
            Some("first line".to_owned()),
            Some("second line".to_owned()),
            Some("third é😀".to_owned()),
            Some(String::new()),
        ]
    );
}

#[test]
fn live_answers_wm_gettext_with_the_document() {
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    main.make_markdown("héllo\n");
    main.set_live(true);
    let view = main.live_view().unwrap();
    let text = support::win32::scintilla_text(view.hwnd()).unwrap();
    assert_eq!(text, "héllo\n");
}
