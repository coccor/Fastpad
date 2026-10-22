#![cfg(windows)]
mod support;

// Build the crate in-process with cfg(test), as json_commands.rs does, so tests can reach App.
include!("../../src/lib.rs");

use crate::document::Language;
use crate::preview::ViewMode;
use crate::window::commands::CommandId;
use std::time::{Duration, Instant};
use support::acceptance::AcceptanceHarness;
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetFocus, VK_ESCAPE, VK_RETURN, VK_TAB};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, IsWindowVisible, MSG, PM_QS_INPUT, PM_REMOVE, PeekMessageW, SendMessageW,
    TranslateMessage, WM_COMMAND, WM_KEYDOWN,
};

/// Direct2D, DirectWrite, and WIC must load only when a preview opens, never through the import
/// table: a static import would load them into every launch and slow cold startup.
#[test]
fn binary_does_not_statically_import_preview_graphics_libraries() {
    AcceptanceHarness::new().assert_no_preview_imports();
}

struct TestMain {
    hwnd: HWND,
    editor: HWND,
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
        let editor = unsafe {
            window::initialize_editor_with(hwnd, &identity, editor::Editor::create).unwrap()
        };
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::ShowWindow(
                hwnd,
                windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOW,
            );
        }
        Self {
            hwnd,
            editor,
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
    }

    fn set_text(&self, text: &str) {
        let bytes = std::ffi::CString::new(text).unwrap();
        unsafe {
            SendMessageW(
                self.editor,
                crate::editor::scintilla_constants::SCI_SETTEXT,
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

    fn mode(&self) -> ViewMode {
        window::preview_host::mode(self.hwnd)
    }

    fn view(&self) -> Option<preview::view::PreviewView> {
        self.with_app(|app| app.active_group().unwrap().preview.view)
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

/// Dispatches everything queued, draining input before each other message as FastPad's own loop
/// prioritizes it. Posted messages are retrieved ahead of input, so a plain `PeekMessageW` loop
/// livelocks when real mouse or keyboard input reaches the (foreground) test window while a
/// deferred startup unit is queued: the unit sees input pending, reposts itself, and is retrieved
/// again before the input ever is.
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

fn pump_for(duration: Duration) {
    let deadline = Instant::now() + duration;
    pump_until("pump_for", duration + Duration::from_secs(1), || {
        Instant::now() >= deadline
    });
}

fn visible(hwnd: HWND) -> bool {
    unsafe { IsWindowVisible(hwnd) != 0 }
}

#[test]
fn the_split_preview_and_source_segments_show_their_modes() {
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    main.make_markdown("# Title\n\nBody\n");
    assert!(window::preview_host::buttons_visible(main.hwnd));

    window::preview_host::set_mode(main.hwnd, ViewMode::Split);
    assert_eq!(main.mode(), ViewMode::Split);
    let view = main.view().expect("preview window");
    assert!(visible(view.hwnd()) && visible(main.editor));
    pump_until("first preview frame", Duration::from_secs(3), || {
        view.stats().block_count == 2
    });

    window::preview_host::set_mode(main.hwnd, ViewMode::Preview);
    assert_eq!(main.mode(), ViewMode::Preview);
    assert!(!visible(main.editor));
    assert_eq!(unsafe { GetFocus() }, view.hwnd());

    // Preview again changes nothing: Source is the way out.
    window::preview_host::set_mode(main.hwnd, ViewMode::Preview);
    assert_eq!(main.mode(), ViewMode::Preview);
    window::preview_host::set_mode(main.hwnd, ViewMode::Source);
    assert_eq!(main.mode(), ViewMode::Source);
    assert!(main.view().is_none());
    assert!(visible(main.editor));
    assert_eq!(unsafe { GetFocus() }, main.editor);
    assert_eq!(
        support::win32::scintilla_text(main.editor).unwrap(),
        "# Title\n\nBody\n"
    );
}

#[test]
fn ctrl_shift_v_cycles_source_live_split_preview() {
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    main.make_markdown("text\n");
    for expected in [
        ViewMode::Live,
        ViewMode::Split,
        ViewMode::Preview,
        ViewMode::Source,
    ] {
        main.command(CommandId::MarkdownViewCycle);
        assert_eq!(main.mode(), expected);
    }
}

#[test]
fn a_plain_text_tab_gets_a_notice_instead_of_a_preview() {
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    main.command(CommandId::MarkdownViewSplit);
    assert_eq!(main.mode(), ViewMode::Source);
    assert!(
        main.notices()
            .iter()
            .any(|notice| notice.contains("Markdown preview is available"))
    );
}

#[test]
fn a_non_markdown_tab_hides_the_preview_and_the_markdown_tab_keeps_its_mode() {
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    main.make_markdown("# One\n");
    main.command(CommandId::MarkdownViewSplit);
    let view = main.view().unwrap();
    pump_until("first render", Duration::from_secs(3), || {
        view.stats().block_count == 1
    });
    main.command(CommandId::New);
    assert_eq!(main.mode(), ViewMode::Source);
    assert!(!visible(view.hwnd()));
    assert!(main.with_app(|app| app.tabs.view().snapshot().view_buttons.segments.is_empty()));
    assert_eq!(
        view.stats().block_count,
        0,
        "a hidden preview releases its document"
    );
    main.command(CommandId::SelectTab1);
    assert_eq!(main.mode(), ViewMode::Split);
    assert!(visible(view.hwnd()));
    assert!(!main.with_app(|app| app.tabs.view().snapshot().view_buttons.segments.is_empty()));
    pump_until("reloaded render", Duration::from_secs(3), || {
        view.stats().block_count == 1
    });
}

#[test]
fn a_large_document_never_shows_the_previous_tabs_preview_while_it_parses() {
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    let paragraph = "Paragraph text for the worker parse.\n\n";
    main.make_markdown(&paragraph.repeat(1_100_000 / paragraph.len()));
    main.command(CommandId::MarkdownViewSplit);
    let view = main.view().unwrap();
    pump_until("worker parse", Duration::from_secs(10), || {
        view.stats().block_count > 1000
    });
    main.command(CommandId::New);
    main.make_markdown("# Small\n\n[link](https://x.dev)\n");
    // Each tab has its own mode: the new one opens in Source.
    main.command(CommandId::MarkdownViewSplit);
    pump_until("small render", Duration::from_secs(3), || {
        view.stats().block_count == 2 && !view.accessible_links().read().unwrap().is_empty()
    });
    main.command(CommandId::SelectTab1);
    assert_eq!(
        view.stats().block_count,
        0,
        "the previous tab's blocks stood in for the parsing document"
    );
    assert!(view.visible_links().is_empty());
    assert!(view.accessible_links().read().unwrap().is_empty());
    pump_until("worker parse again", Duration::from_secs(10), || {
        view.stats().block_count > 1000
    });
}

#[test]
fn full_mode_reloads_keep_the_preview_position() {
    // Break caught: reloading in Full mode jumped the preview to the hidden editor's top line.
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    main.make_markdown(&long_markdown());
    main.command(CommandId::MarkdownViewPreview);
    let view = main.view().unwrap();
    pump_until("render", Duration::from_secs(3), || {
        view.stats().block_count == 400
    });
    unsafe {
        SendMessageW(
            view.hwnd(),
            WM_KEYDOWN,
            windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_NEXT as usize,
            0,
        );
        SendMessageW(
            view.hwnd(),
            WM_KEYDOWN,
            windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_NEXT as usize,
            0,
        );
    }
    pump_for(Duration::from_millis(100));
    let before = view.top_line();
    assert!(before > 0);
    window::preview_host::refresh(main.hwnd, main.with_app(|app| app.tabs.active_group()));
    pump_for(Duration::from_millis(100));
    assert_eq!(view.top_line(), before, "inline reparse");

    let paragraph = "Paragraph text for the worker parse.\n\n";
    main.set_text(&paragraph.repeat(1_100_000 / paragraph.len()));
    window::preview_host::document_reloaded(main.hwnd);
    pump_until("worker parse", Duration::from_secs(10), || {
        view.stats().block_count > 1000
    });
    for _ in 0..3 {
        unsafe {
            SendMessageW(
                view.hwnd(),
                WM_KEYDOWN,
                windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_NEXT as usize,
                0,
            )
        };
    }
    pump_for(Duration::from_millis(100));
    let before = view.top_line();
    assert!(before > 0);
    window::preview_host::refresh(main.hwnd, main.with_app(|app| app.tabs.active_group()));
    assert_eq!(
        view.stats().block_count,
        0,
        "the worker parse is outstanding"
    );
    pump_until("worker reparse", Duration::from_secs(10), || {
        view.stats().block_count > 1000
    });
    assert_eq!(view.top_line(), before, "worker reparse");
}

#[test]
fn escape_in_preview_returns_to_the_tabs_last_editing_mode() {
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    main.make_markdown("text\n");
    let escape = |expected| {
        main.command(CommandId::MarkdownViewPreview);
        let view = main.view().unwrap();
        unsafe { SendMessageW(view.hwnd(), WM_KEYDOWN, VK_ESCAPE as usize, 0) };
        pump_until("Esc handling", Duration::from_secs(2), || {
            main.mode() == expected
        });
    };
    // Never in another mode: Source.
    escape(ViewMode::Source);
    main.command(CommandId::MarkdownViewSplit);
    escape(ViewMode::Split);
}

fn type_text(hwnd: HWND, text: &str) {
    for unit in text.encode_utf16() {
        unsafe {
            SendMessageW(
                hwnd,
                windows_sys::Win32::UI::WindowsAndMessaging::WM_CHAR,
                unit as usize,
                0,
            )
        };
    }
}

#[test]
fn typing_updates_the_preview_only_after_the_pause() {
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    main.make_markdown("# A\n");
    main.command(CommandId::MarkdownViewSplit);
    let view = main.view().unwrap();
    pump_until("initial render", Duration::from_secs(3), || {
        view.stats().block_count == 1
    });
    unsafe {
        SendMessageW(
            main.editor,
            crate::editor::scintilla_constants::SCI_DOCUMENTEND,
            0,
            0,
        );
    }
    type_text(main.editor, "\n\npara");
    assert_eq!(view.stats().block_count, 1, "SCN_MODIFIED must not parse");
    pump_until("debounced update", Duration::from_secs(3), || {
        view.stats().block_count == 2
    });
}

#[test]
fn theme_changes_recolor_the_preview() {
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    main.make_markdown("# A\n");
    main.command(CommandId::MarkdownViewSplit);
    main.command(CommandId::ThemeCatppuccinMocha);
    let view = main.view().unwrap();
    assert_eq!(view.colors().link, crate::catppuccin::MOCHA.blue);
}

#[test]
fn relative_markdown_links_open_in_a_tab() {
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let dir = std::env::temp_dir().join(format!("fastpad-preview-links-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.md"), "[next](b.md)\n").unwrap();
    std::fs::write(dir.join("b.md"), "# B\n").unwrap();
    let main = TestMain::new();
    window::open_path(main.hwnd, &dir.join("a.md")).unwrap();
    pump_until("a.md loaded", Duration::from_secs(3), || {
        !main.with_app(|app| app.populating_file)
    });
    main.with_app(|app| app.tabs.set_active_language(Language::Markdown));
    main.command(CommandId::MarkdownViewSplit);
    let payload = Box::into_raw(Box::new(String::from("b.md")));
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(
            main.hwnd,
            window::WM_FASTPAD_PREVIEW_LINK,
            main.view().unwrap().hwnd() as usize,
            payload as isize,
        );
    }
    pump_until("b.md tab", Duration::from_secs(3), || {
        main.with_app(|app| app.tabs.active().and_then(|document| document.path.clone()))
            == Some(dir.join("b.md"))
    });
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn unsupported_links_explain_themselves() {
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    main.make_markdown("[x](ftp://x.dev)\n");
    main.command(CommandId::MarkdownViewSplit);
    let payload = Box::into_raw(Box::new(String::from("ftp://x.dev")));
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(
            main.hwnd,
            window::WM_FASTPAD_PREVIEW_LINK,
            main.view().unwrap().hwnd() as usize,
            payload as isize,
        );
    }
    pump_until("link notice", Duration::from_secs(2), || {
        main.notices()
            .iter()
            .any(|notice| notice.contains("does not open this kind of link"))
    });
}

#[test]
fn large_documents_parse_on_a_worker_and_huge_ones_pause() {
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    let paragraph = "Paragraph text for the worker parse.\n\n";
    main.make_markdown(&paragraph.repeat(2_000_000 / paragraph.len()));
    main.command(CommandId::MarkdownViewSplit);
    let view = main.view().unwrap();
    pump_until("worker parse", Duration::from_secs(10), || {
        view.stats().block_count > 1000
    });
    assert!(!view.is_paused());

    main.command(CommandId::MarkdownViewSource);
    main.make_markdown(&paragraph.repeat(11_000_000 / paragraph.len()));
    main.command(CommandId::MarkdownViewSplit);
    let view = main.view().unwrap();
    assert!(view.is_paused());
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(
            main.hwnd,
            window::WM_FASTPAD_PREVIEW_REFRESH,
            view.hwnd() as usize,
            0,
        );
    }
    pump_until("refresh parse", Duration::from_secs(20), || {
        view.stats().block_count > 1000
    });
}

fn long_markdown() -> String {
    (0..400)
        .map(|index| {
            format!(
                "Paragraph {index}

"
            )
        })
        .collect()
}

#[test]
fn scrolling_the_editor_scrolls_the_preview() {
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    main.make_markdown(&long_markdown());
    main.command(CommandId::MarkdownViewSplit);
    let view = main.view().unwrap();
    pump_until("render", Duration::from_secs(3), || {
        view.stats().block_count == 400
    });
    unsafe {
        SendMessageW(
            main.editor,
            crate::editor::scintilla_constants::SCI_SETFIRSTVISIBLELINE,
            300,
            0,
        );
        windows_sys::Win32::Graphics::Gdi::UpdateWindow(main.editor);
    }
    pump_until("preview follows", Duration::from_secs(3), || {
        view.top_line() >= 280
    });
}

#[test]
fn scrolling_the_preview_scrolls_the_editor_without_echo() {
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    main.make_markdown(&long_markdown());
    main.command(CommandId::MarkdownViewSplit);
    let view = main.view().unwrap();
    pump_until("render", Duration::from_secs(3), || {
        view.stats().block_count == 400
    });
    let before = main.with_app(|app| app.active_group().unwrap().preview.sync_count);
    unsafe {
        SendMessageW(
            view.hwnd(),
            WM_KEYDOWN,
            windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_NEXT as usize,
            0,
        );
    }
    let preview_line = view.top_line();
    assert!(preview_line > 0);
    pump_until("editor follows", Duration::from_secs(3), || {
        (unsafe {
            SendMessageW(
                main.editor,
                crate::editor::scintilla_constants::SCI_GETFIRSTVISIBLELINE,
                0,
                0,
            )
        }) > 0
    });
    pump_for(Duration::from_millis(250));
    let syncs = main.with_app(|app| app.active_group().unwrap().preview.sync_count) - before;
    assert!(
        syncs <= 1,
        "scroll sync echoed: {syncs} syncs for one preview scroll"
    );
    assert_eq!(
        view.top_line(),
        preview_line,
        "the editor's echo moved the preview"
    );
}

#[test]
fn closing_the_find_bar_in_full_mode_focuses_the_preview() {
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    main.make_markdown("# A\n");
    main.command(CommandId::MarkdownViewPreview);
    let view = main.view().unwrap();
    main.command(CommandId::Find);
    let query = main.with_app(|app| app.find_bar().unwrap().query_hwnd());
    assert_eq!(unsafe { GetFocus() }, query);
    unsafe { SendMessageW(query, WM_KEYDOWN, VK_ESCAPE as usize, 0) };
    assert_eq!(unsafe { GetFocus() }, view.hwnd());
}

#[test]
fn losing_capture_ends_a_divider_drag() {
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    main.make_markdown("# A\n");
    main.command(CommandId::MarkdownViewSplit);
    let divider = window::preview_host::divider_rect(main.hwnd).expect("divider");
    // The preview sits in the editor group, whose client area the divider is laid out in.
    let group = unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::GetParent(main.view().unwrap().hwnd())
    };
    assert_ne!(group, main.hwnd);
    assert!(window::preview_host::begin_divider_drag(
        main.hwnd,
        group,
        divider.left,
        divider.top
    ));
    assert!(window::preview_host::drag_divider(main.hwnd, divider.left));
    unsafe { windows_sys::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture() };
    assert!(!window::preview_host::drag_divider(main.hwnd, divider.left));
}

#[test]
fn menu_mode_keeps_the_frame_focus_in_full_mode() {
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    main.make_markdown("# A\n");
    main.command(CommandId::MarkdownViewPreview);
    // A tapped F10 enters menu mode, which parks the focus on the frame for the menu keys.
    unsafe {
        SendMessageW(
            main.hwnd,
            windows_sys::Win32::UI::WindowsAndMessaging::WM_SYSCOMMAND,
            windows_sys::Win32::UI::WindowsAndMessaging::SC_KEYMENU as usize,
            0,
        )
    };
    assert!(main.with_app(|app| app.menu_mode.is_some()));
    assert_eq!(unsafe { GetFocus() }, main.hwnd);
}

#[test]
fn full_mode_keeps_the_editor_position() {
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    main.make_markdown(&long_markdown());
    unsafe {
        SendMessageW(
            main.editor,
            crate::editor::scintilla_constants::SCI_SETFIRSTVISIBLELINE,
            100,
            0,
        )
    };
    main.command(CommandId::MarkdownViewPreview);
    let view = main.view().unwrap();
    unsafe {
        SendMessageW(
            view.hwnd(),
            WM_KEYDOWN,
            windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_END as usize,
            0,
        )
    };
    pump_for(Duration::from_millis(300));
    main.command(CommandId::MarkdownViewSplit);
    assert_eq!(
        unsafe {
            SendMessageW(
                main.editor,
                crate::editor::scintilla_constants::SCI_GETFIRSTVISIBLELINE,
                0,
                0,
            )
        },
        100
    );
}

fn sample_markdown(bytes: usize) -> String {
    let section = "## Heading\n\nParagraph with **bold**, *emphasis*, `code`, and a [link](https://x.dev).\n\n- item one\n- item two\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n```rust\nfn f() {}\n```\n\n";
    section.repeat(bytes / section.len() + 1)[..bytes]
        .rsplit_once("\n\n")
        .map_or_else(String::new, |(text, _)| format!("{text}\n"))
}

fn html_markdown(bytes: usize) -> String {
    let section = "<div align=\"center\">\n\n<img src=\"https://x.dev/badge.svg\" alt=\"badge\" width=\"96\">\n\n## Heading\n\n</div>\n\nPress <kbd>Ctrl</kbd>+<kbd>S</kbd> in <b>bold</b> text with a [link](https://x.dev).\n\n<details>\n<summary>More</summary>\n\n| a | b |\n|---|---|\n| 1<br>2 | <code>x</code> |\n\n</details>\n\n";
    section.repeat(bytes / section.len() + 1)[..bytes]
        .rsplit_once("\n\n")
        .map_or_else(String::new, |(text, _)| format!("{text}\n"))
}

fn preview_open_p95(main: &TestMain, text: &str) -> u64 {
    main.make_markdown(text);
    let mut samples = Vec::new();
    for _ in 0..30 {
        main.command(CommandId::MarkdownViewSplit);
        let view = main.view().unwrap();
        pump_until("first frame", Duration::from_secs(5), || {
            view.stats().first_frame_micros > 0
        });
        samples.push(view.stats().first_frame_micros);
        main.command(CommandId::MarkdownViewSource);
    }
    p95(samples)
}

fn one_paragraph_update_p95(main: &TestMain, text: &str) -> u64 {
    main.make_markdown(text);
    main.command(CommandId::MarkdownViewSplit);
    let view = main.view().unwrap();
    pump_until("initial render", Duration::from_secs(10), || {
        view.stats().block_count > 0
    });
    unsafe {
        SendMessageW(
            main.editor,
            crate::editor::scintilla_constants::SCI_GOTOPOS,
            text.len() / 2,
            0,
        )
    };
    pump_for(Duration::from_millis(300));
    // Edit a line the synced preview actually shows: an off-screen edit correctly skips the
    // repaint, which would leave `last_update_micros` holding the initial full render.
    unsafe {
        let line_start = SendMessageW(
            main.editor,
            crate::editor::scintilla_constants::SCI_POSITIONFROMLINE,
            view.top_line() + 1,
            0,
        );
        SendMessageW(
            main.editor,
            crate::editor::scintilla_constants::SCI_GOTOPOS,
            line_start as usize,
            0,
        );
    }
    let mut samples = Vec::new();
    for _ in 0..50 {
        let revision = view.stats().revision;
        let painted = view.stats().painted_updates;
        type_text(main.editor, "x");
        pump_until("update", Duration::from_secs(5), || {
            view.stats().revision > revision && view.stats().painted_updates > painted
        });
        pump_for(Duration::from_millis(20));
        samples.push(view.stats().last_update_micros);
    }
    p95(samples)
}

fn p95(mut samples: Vec<u64>) -> u64 {
    samples.sort_unstable();
    samples[(samples.len() * 95 / 100).min(samples.len() - 1)]
}

#[test]
#[ignore = "performance measurement: cargo test --release --test markdown_preview -- --ignored --test-threads=1"]
fn opening_a_100_kb_preview_renders_within_50_ms_p95() {
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    let p95 = preview_open_p95(&main, &sample_markdown(100_000));
    println!("preview open p95: {p95} us");
    assert!(p95 < 50_000);
}

#[test]
#[ignore = "performance measurement: cargo test --release --test markdown_preview -- --ignored --test-threads=1"]
fn one_paragraph_updates_in_a_1_mb_document_within_2_ms_p95() {
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    let p95 = one_paragraph_update_p95(&main, &sample_markdown(1_000_000));
    println!("incremental update p95: {p95} us");
    assert!(p95 < 2_000);
}

#[test]
#[ignore = "performance measurement: cargo test --release --test markdown_preview -- --ignored --test-threads=1"]
fn typing_with_split_open_costs_the_same_as_without_a_preview() {
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    main.make_markdown(&sample_markdown(1_000_000));
    unsafe {
        SendMessageW(
            main.editor,
            crate::editor::scintilla_constants::SCI_GOTOPOS,
            500_000,
            0,
        )
    };
    // Each run starts on a fresh line and breaks the line every 40 characters (unmeasured): one
    // ever-growing line would make later samples pay for re-laying out a longer line and for
    // horizontal caret scrolling in the narrower split editor, which is not the preview's cost.
    let measure = |main: &TestMain| {
        let mut samples = Vec::new();
        type_text(main.editor, "\n\n");
        for index in 0..200 {
            if index % 40 == 0 {
                type_text(main.editor, "\n");
            }
            let started = Instant::now();
            type_text(main.editor, "x");
            unsafe { windows_sys::Win32::Graphics::Gdi::UpdateWindow(main.editor) };
            samples.push(started.elapsed().as_micros() as u64);
        }
        p95(samples)
    };
    let baseline = measure(&main);
    main.command(CommandId::MarkdownViewSplit);
    pump_for(Duration::from_millis(500));
    let with_preview = measure(&main);
    println!("keystroke p95: off {baseline} us, split {with_preview} us");
    assert!(with_preview <= baseline + baseline / 10 + 100);
}

/// A Markdown document of about `bytes` with headings, prose, a list with task items, a table and
/// fenced code in every section, and where each section's typing spots start (byte offsets):
/// the paragraph's end, the task item's end, the table cell's end and the code line's end.
fn writing_markdown(bytes: usize) -> (String, Vec<[usize; 4]>) {
    let mut text = String::new();
    let mut spots = Vec::new();
    for n in 0.. {
        if text.len() >= bytes {
            break;
        }
        text.push_str(&format!("## Heading {n}\n\n"));
        text.push_str("Paragraph with **bold**, *emphasis*, `code` and a [link](https://x.dev).");
        let paragraph = text.len();
        text.push_str("\n\n- item one\n- [ ] task two");
        let task = text.len();
        text.push_str("\n- [x] task three\n\n| a | b |\n|---|---|\n| 1");
        let cell = text.len();
        text.push_str(" | 2 |\n\n```rust\nfn f() {}");
        let code = text.len();
        text.push_str("\n- not a list\n```\n\n");
        spots.push([paragraph, task, cell, code]);
    }
    (text, spots)
}

/// One keystroke as the message loop delivers it (key down, then the character it produces)
/// and everything it posted, timed through the repaint.
fn timed_key(main: &TestMain, vk: u16, char: Option<u16>) -> u64 {
    let started = Instant::now();
    unsafe {
        SendMessageW(main.editor, WM_KEYDOWN, usize::from(vk), 0);
        if let Some(char) = char {
            SendMessageW(
                main.editor,
                windows_sys::Win32::UI::WindowsAndMessaging::WM_CHAR,
                usize::from(char),
                0,
            );
        }
    }
    pump_pending();
    unsafe { windows_sys::Win32::Graphics::Gdi::UpdateWindow(main.editor) };
    started.elapsed().as_micros() as u64
}

fn timed_typing(main: &TestMain, text: &str, samples: &mut Vec<u64>) {
    for unit in text.encode_utf16() {
        samples.push(timed_key(main, unit, Some(unit)));
    }
}

fn editor_text(main: &TestMain) -> String {
    let length = unsafe {
        SendMessageW(
            main.editor,
            crate::editor::scintilla_constants::SCI_GETLENGTH,
            0,
            0,
        )
    } as usize;
    let mut bytes = vec![0u8; length + 1];
    unsafe {
        SendMessageW(
            main.editor,
            crate::editor::scintilla_constants::SCI_GETTEXT,
            bytes.len(),
            bytes.as_mut_ptr() as isize,
        )
    };
    bytes.truncate(length);
    String::from_utf8(bytes).unwrap()
}

/// Types into a 200 KB document in `language`: prose with Enter, a task item continued by Enter
/// and ended by Enter on the empty item, a table cell then Down out of the table, and a code
/// line with Enter. Sections are visited bottom-up, and within a section bottom-up, so each
/// spot's offset is still valid when it is reached. Returns every keystroke's time.
fn writing_samples(main: &TestMain, language: Language) -> Vec<u64> {
    let (text, spots) = writing_markdown(200_000);
    main.set_text(&text);
    main.with_app(|app| app.tabs.set_active_language(language));
    unsafe { windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus(main.editor) };
    pump_for(Duration::from_millis(200));
    let goto = |at: usize| {
        unsafe {
            SendMessageW(
                main.editor,
                crate::editor::scintilla_constants::SCI_GOTOPOS,
                at,
                0,
            )
        };
        pump_pending();
    };
    let enter = |samples: &mut Vec<u64>| samples.push(timed_key(main, VK_RETURN, Some(0x0D)));
    let mut samples = Vec::new();
    let count = spots.len();
    for section in (0..12).map(|round| count - 1 - round * (count / 12)) {
        let [paragraph, task, cell, code] = spots[section];
        goto(code);
        timed_typing(main, " // x", &mut samples);
        enter(&mut samples);
        goto(cell);
        timed_typing(main, "23456", &mut samples);
        samples.push(timed_key(
            main,
            windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_DOWN,
            None,
        ));
        goto(task);
        timed_typing(main, " and more", &mut samples);
        enter(&mut samples);
        timed_typing(main, "next task", &mut samples);
        enter(&mut samples);
        enter(&mut samples); // the empty item ends the list
        goto(paragraph);
        timed_typing(main, " More prose follows here", &mut samples);
        enter(&mut samples);
        timed_typing(main, "A new line of prose", &mut samples);
        enter(&mut samples);
    }
    samples
}

#[test]
#[ignore = "performance measurement: cargo test --release --test markdown_preview -- --ignored --test-threads=1"]
fn typing_with_the_writing_helpers_costs_the_same_as_plain_text() {
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    // A warm-up pass of each, then two measured passes alternating, so neither side gets the
    // warm caches.
    writing_samples(&main, Language::PlainText);
    writing_samples(&main, Language::Markdown);
    let (mut plain, mut markdown) = (Vec::new(), Vec::new());
    for _ in 0..2 {
        plain.extend(writing_samples(&main, Language::PlainText));
        markdown.extend(writing_samples(&main, Language::Markdown));
        // The helpers ran: the task item was continued, and the table re-aligned on leaving it.
        let text = editor_text(&main);
        assert!(text.contains("- [ ] task two and more\n- [ ] next task\n\n"));
        assert!(text.contains("| 123456 | 2   |"));
    }
    let (count, baseline, helpers) = (plain.len(), p95(plain), p95(markdown));
    println!("keystroke p95 over {count} keys: plain text {baseline} us, Markdown {helpers} us");
    assert!(helpers <= baseline + baseline / 10 + 100);
}

#[test]
#[ignore = "performance measurement: cargo test --release --test markdown_preview -- --ignored --test-threads=1"]
fn private_bytes_do_not_grow_across_open_close_cycles() {
    use windows_sys::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    fn private_bytes() -> u64 {
        let mut counters = PROCESS_MEMORY_COUNTERS_EX {
            cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
            ..Default::default()
        };
        unsafe {
            GetProcessMemoryInfo(
                GetCurrentProcess(),
                (&mut counters as *mut PROCESS_MEMORY_COUNTERS_EX).cast(),
                counters.cb,
            )
        };
        counters.PrivateUsage as u64
    }
    // The first open loads Direct2D, DirectWrite, Direct3D, and the GPU driver for the session:
    // about 45 MB of private bytes that closing does not return. Unloading them on close still
    // left about 21 MB and made every reopen cost about 120 ms. Closing must return everything
    // else, so repeated open/close cycles may not grow. Two warm-up cycles come first (the heap
    // settles on the first reopen: a one-time step of up to about 2 MB with a 1 MB document), then
    // a reference cycle. Single samples jitter by up to about 3 MB, so the median of the next five
    // closes is compared with the reference.
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    main.make_markdown(&sample_markdown(1_000_000));
    pump_for(Duration::from_millis(300));
    let never_opened = private_bytes();
    let cycle = || {
        main.command(CommandId::MarkdownViewSplit);
        let view = main.view().unwrap();
        pump_until("render", Duration::from_secs(10), || {
            view.stats().block_count > 0
        });
        pump_for(Duration::from_millis(300));
        let open = private_bytes();
        main.command(CommandId::MarkdownViewSource);
        pump_for(Duration::from_millis(500));
        (open, private_bytes())
    };
    for warm_up in 1..=2 {
        let (open, closed) = cycle();
        println!("private bytes: warm-up {warm_up} open {open}, closed {closed}");
    }
    let (_, settled) = cycle();
    println!("private bytes: never {never_opened}, settled close {settled}");
    let mut closes = Vec::new();
    for round in 1..=5 {
        let (open, closed) = cycle();
        println!("private bytes: round {round} open {open}, closed {closed}");
        closes.push(closed);
    }
    closes.sort_unstable();
    let median = closes[closes.len() / 2];
    println!(
        "private bytes: median close {median}, growth {} bytes, closed vs never {} bytes",
        median as i64 - settled as i64,
        median as i64 - never_opened as i64
    );
    assert!(median.saturating_sub(settled) < 2 * 1024 * 1024);
}
/// Guards the zero-startup-cost rule at runtime, complementing the import-table guard: launching
/// with a Markdown file must not load the preview's graphics libraries.
#[test]
fn launching_with_a_markdown_file_loads_no_preview_graphics_library() {
    use support::process::{FastPadProcess, process_has_module_loaded};
    /// Removes the temporary folder even when an assertion fails.
    struct TempRoot(std::path::PathBuf);
    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let root =
        TempRoot(std::env::temp_dir().join(format!("fastpad-startup-{}", std::process::id())));
    let root = &root.0;
    let local_app_data = root.join("LocalAppData");
    std::fs::create_dir_all(&local_app_data).unwrap();
    let path = root.join("startup.md");
    std::fs::write(&path, "# Title\n\nBody with a [link](https://x.dev).\n").unwrap();
    let mut process = FastPadProcess::spawn_with_local_app_data(
        [std::ffi::OsStr::new("--new-window"), path.as_os_str()],
        &local_app_data,
    )
    .unwrap();
    process
        .wait_for_main_window(Duration::from_secs(10))
        .unwrap();
    std::thread::sleep(Duration::from_millis(500));
    for module in ["d2d1.dll", "dwrite.dll", "windowscodecs.dll"] {
        assert!(
            !process_has_module_loaded(process.id(), module).unwrap(),
            "{module} was loaded before any preview was opened"
        );
    }
    process.close().unwrap();
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn the_html_readme_fixture_renders_its_svg_and_toggles_its_section() {
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    let icon = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("assets")
        .join("fastpad-icon.svg");
    let fixture = include_str!("../fixtures/html-readme.md")
        .replace("assets/fastpad-icon.svg", icon.to_str().unwrap());
    main.make_markdown(&fixture);
    main.command(CommandId::MarkdownViewSplit);
    let view = main.view().unwrap();
    pump_until("the SVG icon decodes", Duration::from_secs(10), || {
        view.image_states()
            .iter()
            .any(|(path, ready)| path.ends_with("fastpad-icon.svg") && *ready)
    });
    let disclosure = || {
        view.accessible_links()
            .read()
            .unwrap()
            .iter()
            .find_map(|link| {
                link.disclosure
                    .as_ref()
                    .map(|disclosure| (disclosure.expanded, link.focused))
            })
    };
    for _ in 0..20 {
        if disclosure().is_some_and(|(_, focused)| focused) {
            break;
        }
        unsafe { SendMessageW(view.hwnd(), WM_KEYDOWN, VK_TAB as usize, 0) };
        pump_pending();
    }
    assert_eq!(disclosure(), Some((false, true)), "Tab reaches the section");
    let collapsed = view.stats().content_height;
    unsafe { SendMessageW(view.hwnd(), WM_KEYDOWN, VK_RETURN as usize, 0) };
    pump_until("the section expands", Duration::from_secs(3), || {
        disclosure() == Some((true, true)) && view.stats().content_height > collapsed
    });
    unsafe { SendMessageW(view.hwnd(), WM_KEYDOWN, VK_RETURN as usize, 0) };
    pump_until("the section collapses", Duration::from_secs(3), || {
        disclosure() == Some((false, true))
    });
}

#[test]
#[ignore = "performance measurement: cargo test --release --test markdown_preview -- --ignored --test-threads=1"]
fn opening_a_100_kb_html_heavy_preview_renders_within_50_ms_p95() {
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    let p95 = preview_open_p95(&main, &html_markdown(100_000));
    println!("HTML-heavy preview open p95: {p95} us");
    assert!(p95 < 50_000);
}

#[test]
#[ignore = "performance measurement: cargo test --release --test markdown_preview -- --ignored --test-threads=1"]
fn one_paragraph_update_inside_a_div_spanning_1_mb_is_recorded() {
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    let text = format!("<div>\n\n{}\n</div>\n", sample_markdown(1_000_000));
    let p95 = one_paragraph_update_p95(&main, &text);
    // No target: an element spanning the document reparses whole on every edit (spec §4.5).
    println!("update inside a 1 MB div p95: {p95} us");
}

// Text selection (2026-10-22 preview selection spec).

use crate::preview::selection::{Selection, TextPoint};

/// The active tab as `text` in Preview, painted, so its visible blocks have layouts.
fn selectable_preview(main: &TestMain, text: &str) -> preview::view::PreviewView {
    main.make_markdown(text);
    main.command(CommandId::MarkdownViewPreview);
    let view = main.view().expect("preview window");
    pump_until("first render", Duration::from_secs(3), || {
        view.stats().block_count > 0
    });
    unsafe { windows_sys::Win32::Graphics::Gdi::UpdateWindow(view.hwnd()) };
    pump_pending();
    view
}

fn mouse(view: HWND, message: u32, (x, y): (i32, i32), keys: usize) {
    let lparam = (((y as u32) << 16) | (x as u32 & 0xFFFF)) as isize;
    unsafe { SendMessageW(view, message, keys, lparam) };
}

fn click(view: HWND, at: (i32, i32), keys: usize) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{WM_LBUTTONDOWN, WM_LBUTTONUP};
    mouse(view, WM_LBUTTONDOWN, at, 0x0001 | keys);
    mouse(view, WM_LBUTTONUP, at, keys);
}

fn drag(view: HWND, from: (i32, i32), to: (i32, i32)) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE};
    mouse(view, WM_LBUTTONDOWN, from, 0x0001);
    mouse(view, WM_MOUSEMOVE, to, 0x0001);
    mouse(view, WM_LBUTTONUP, to, 0);
}

fn ordered(view: &preview::view::PreviewView) -> Option<(TextPoint, TextPoint)> {
    view.selection()
        .map(|selection: Selection| selection.ordered())
}

#[test]
fn dragging_selects_across_blocks_and_copies_the_rendered_text() {
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    let view = selectable_preview(&main, "First **para**\n\nSecond para\n\nThird\n");
    let from = view.client_of(TextPoint::new(0, 0, 0)).unwrap();
    let to = view.client_of(TextPoint::new(1, 0, 6)).unwrap();
    drag(view.hwnd(), from, to);
    assert_eq!(
        ordered(&view),
        Some((TextPoint::new(0, 0, 0), TextPoint::new(1, 0, 6)))
    );
    let clip = view.selected_text().unwrap();
    assert_eq!(clip.plain, "First para\n\nSecond");
    assert_eq!(clip.html, "<p>First <strong>para</strong></p><p>Second</p>");
}

#[test]
fn a_plain_click_selects_nothing_and_shift_click_extends() {
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    let view = selectable_preview(&main, "One two\n\nThree four\n");
    click(
        view.hwnd(),
        view.client_of(TextPoint::new(0, 0, 4)).unwrap(),
        0,
    );
    assert_eq!(view.selection(), None);
    // Shift+click extends from where the last selection began.
    drag(
        view.hwnd(),
        view.client_of(TextPoint::new(0, 0, 0)).unwrap(),
        view.client_of(TextPoint::new(0, 0, 3)).unwrap(),
    );
    click(
        view.hwnd(),
        view.client_of(TextPoint::new(1, 0, 5)).unwrap(),
        0x0004,
    );
    assert_eq!(
        ordered(&view),
        Some((TextPoint::new(0, 0, 0), TextPoint::new(1, 0, 5)))
    );
    assert_eq!(view.selected_text().unwrap().plain, "One two\n\nThree");
}

#[test]
fn double_click_selects_a_word_and_triple_click_the_paragraph() {
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    let view = selectable_preview(&main, "Hello brave world\n");
    let at = view.client_of(TextPoint::new(0, 0, 8)).unwrap();
    click(view.hwnd(), at, 0);
    click(view.hwnd(), at, 0);
    assert_eq!(view.selected_text().unwrap().plain, "brave");
    click(view.hwnd(), at, 0);
    assert_eq!(view.selected_text().unwrap().plain, "Hello brave world");
}

#[test]
fn dragging_from_a_link_selects_text() {
    // Break caught: a press on a link swallowing the drag, so link text could never be selected.
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    let view = selectable_preview(&main, "See [the heading](#top) here\n\n# Top\n");
    drag(
        view.hwnd(),
        view.client_of(TextPoint::new(0, 0, 4)).unwrap(),
        view.client_of(TextPoint::new(0, 0, 15)).unwrap(),
    );
    assert_eq!(view.selected_text().unwrap().plain, "the heading");
}

#[test]
fn select_all_on_a_large_note_lays_out_nothing_more() {
    // Break caught: copying laying out every block of a long note to read its text.
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    let text = (0..3000)
        .map(|n| format!("Paragraph {n} with **bold**.\n\n"))
        .collect::<String>();
    let view = selectable_preview(&main, &text);
    let laid = view.laid_count();
    assert!(laid < 3000, "only the visible blocks are laid out ({laid})");
    view.select_all();
    let clip = view.selected_text().unwrap();
    assert_eq!(view.laid_count(), laid);
    assert!(
        clip.plain
            .starts_with("Paragraph 0 with bold.\n\nParagraph 1 ")
    );
    assert!(clip.plain.ends_with("Paragraph 2999 with bold."));
}

#[test]
fn ctrl_a_selects_all_and_escape_clears_it_before_leaving_preview() {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetKeyboardState, SetKeyboardState, VK_A, VK_CONTROL,
    };
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    let view = selectable_preview(&main, "Alpha\n\nBeta\n");
    let mut keys = [0u8; 256];
    unsafe { GetKeyboardState(keys.as_mut_ptr()) };
    let original = keys;
    keys[usize::from(VK_CONTROL)] = 0x80;
    unsafe { SetKeyboardState(keys.as_ptr()) };
    unsafe { SendMessageW(view.hwnd(), WM_KEYDOWN, usize::from(VK_A), 0) };
    unsafe { SetKeyboardState(original.as_ptr()) };
    assert_eq!(view.selected_text().unwrap().plain, "Alpha\n\nBeta");
    unsafe { SendMessageW(view.hwnd(), WM_KEYDOWN, VK_ESCAPE as usize, 0) };
    pump_pending();
    assert_eq!(view.selection(), None);
    assert_eq!(
        main.mode(),
        ViewMode::Preview,
        "the first Escape only clears"
    );
    unsafe { SendMessageW(view.hwnd(), WM_KEYDOWN, VK_ESCAPE as usize, 0) };
    pump_until("Escape to leave Preview", Duration::from_secs(2), || {
        main.mode() != ViewMode::Preview
    });
}

#[test]
fn copy_while_the_preview_has_focus_copies_the_preview_not_the_editor() {
    // Break caught: Edit > Copy in Preview copying the hidden editor's selection.
    use crate::platform::clipboard::test_support::{Backup, read_html, read_text};
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    let view = selectable_preview(&main, "Plain **rich** text\n");
    unsafe {
        SendMessageW(
            main.editor,
            crate::editor::scintilla_constants::SCI_SETSEL,
            0,
            5,
        );
        windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus(view.hwnd());
    }
    view.select_all();
    let _backup = Backup::take();
    main.command(CommandId::Copy);
    assert_eq!(read_text(), "Plain rich text");
    assert!(
        read_html().contains(
            "<!--StartFragment--><p>Plain <strong>rich</strong> text</p><!--EndFragment-->"
        )
    );
}

#[test]
fn a_resize_keeps_the_selection_and_an_edit_clears_it() {
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    let view = selectable_preview(
        &main,
        "Some text that wraps when the window is narrow enough.\n\nMore\n",
    );
    view.select_all();
    let before = view.selection();
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::SetWindowPos(
            main.hwnd,
            std::ptr::null_mut(),
            0,
            0,
            500,
            600,
            windows_sys::Win32::UI::WindowsAndMessaging::SWP_NOMOVE
                | windows_sys::Win32::UI::WindowsAndMessaging::SWP_NOZORDER,
        );
        windows_sys::Win32::Graphics::Gdi::UpdateWindow(view.hwnd());
    }
    pump_pending();
    assert_eq!(view.selection(), before);
    main.set_text("Edited\n");
    pump_until(
        "the edit to reach the preview",
        Duration::from_secs(3),
        || view.selection().is_none(),
    );
}
