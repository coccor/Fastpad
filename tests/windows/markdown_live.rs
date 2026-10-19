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

/// One step of typing: characters as `WM_CHAR`s, or a key press with Ctrl and Shift as given.
#[derive(Clone, Copy)]
enum Input {
    Text(&'static str),
    Key(u16, bool, bool),
}

/// Sends `inputs` to `target` as the message loop would deliver them, with the keyboard state
/// holding the modifiers each key press names.
fn send_inputs(target: HWND, inputs: &[Input]) {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetKeyboardState, SetKeyboardState, VK_CONTROL, VK_SHIFT,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{WM_CHAR, WM_KEYDOWN, WM_KEYUP};
    for input in inputs {
        match *input {
            Input::Text(text) => {
                for unit in text.encode_utf16() {
                    unsafe { SendMessageW(target, WM_CHAR, usize::from(unit), 0) };
                }
            }
            Input::Key(vk, ctrl, shift) => {
                let mut keys = [0u8; 256];
                unsafe { GetKeyboardState(keys.as_mut_ptr()) };
                let original = keys;
                keys[usize::from(VK_CONTROL)] = if ctrl { 0x80 } else { 0 };
                keys[usize::from(VK_SHIFT)] = if shift { 0x80 } else { 0 };
                unsafe { SetKeyboardState(keys.as_ptr()) };
                unsafe { SendMessageW(target, WM_KEYDOWN, usize::from(vk), 0) };
                // The character TranslateMessage would post for this key.
                let character = match vk {
                    VK_RETURN => Some(0x0D),
                    VK_TAB => Some(0x09),
                    VK_BACK if ctrl => Some(0x7F),
                    VK_BACK => Some(0x08),
                    _ => None,
                };
                if let Some(character) = character {
                    unsafe { SendMessageW(target, WM_CHAR, character, 0) };
                }
                unsafe { SendMessageW(target, WM_KEYUP, usize::from(vk), 0) };
                unsafe { SetKeyboardState(original.as_ptr()) };
            }
        }
        pump_pending();
    }
}

fn caret(main: &TestMain) -> (usize, usize) {
    main.editor().anchor_and_caret().unwrap()
}

/// The document's text after `inputs` and how many undo steps they made: typed into the source
/// view, then into the Live view, each time in a fresh Markdown tab holding `start`.
fn typed_both_ways(start: &str, caret_at: usize, inputs: &[Input]) -> [(String, usize); 2] {
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    let mut results = Vec::new();
    for live in [false, true] {
        main.command(CommandId::New);
        main.make_markdown("");
        let editor = main.editor();
        editor.populate_clean(start).unwrap();
        editor.set_selection(caret_at..caret_at).unwrap();
        main.set_live(live);
        let target = if live {
            main.live_view().unwrap().hwnd()
        } else {
            editor.hwnd()
        };
        unsafe { SetFocus(target) };
        pump_pending();
        send_inputs(target, inputs);
        let text = editor.text().unwrap();
        let mut steps = 0;
        while editor.can_undo().unwrap() {
            editor.undo().unwrap();
            steps += 1;
        }
        assert_eq!(editor.text().unwrap(), start, "undo restores the start");
        results.push((text, steps));
    }
    [results[0].clone(), results[1].clone()]
}

use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    VK_BACK, VK_DELETE, VK_DOWN, VK_END, VK_HOME, VK_LEFT, VK_RETURN, VK_RIGHT, VK_TAB, VK_UP,
};

#[test]
fn typing_in_live_matches_typing_in_source() {
    let script = [
        Input::Text("Hello wörld 😀"),
        Input::Key(VK_RETURN, false, false),
        Input::Text("- item"),
        Input::Key(VK_RETURN, false, false),
        Input::Text("next one"),
        Input::Key(VK_BACK, false, false),
        Input::Key(VK_BACK, true, false),
        Input::Key(VK_LEFT, false, false),
        Input::Text("X"),
        Input::Key(VK_LEFT, true, false),
        Input::Key(VK_DELETE, true, false),
        Input::Key(VK_RIGHT, false, true),
        Input::Text("Y"),
    ];
    let [source, live] = typed_both_ways("start\n", 6, &script);
    assert_eq!(live, source);
}

#[test]
fn enter_continues_a_list_in_live() {
    let script = [Input::Key(VK_RETURN, false, false), Input::Text("two")];
    let [source, live] = typed_both_ways("- one", 5, &script);
    assert_eq!(live, source);
    assert!(live.0.ends_with("- two"), "{:?}", live.0);
}

#[test]
fn tab_nests_a_list_item_in_live() {
    let script = [Input::Key(VK_TAB, false, false)];
    let [source, live] = typed_both_ways("- one\n- two", 11, &script);
    assert_eq!(live, source);
    assert_ne!(live.0, "- one\n- two");
}

#[test]
fn typing_in_a_table_then_leaving_formats_it_in_live() {
    let start = "| a | b |\n|---|---|\n| c | d |\n\nafter\n";
    let script = [
        Input::Text("ccc"),
        Input::Key(VK_DOWN, false, false),
        Input::Key(VK_DOWN, false, false),
    ];
    let caret_at = start.find("c |").unwrap() + 1;
    let [source, live] = typed_both_ways(start, caret_at, &script);
    assert_eq!(live.0, source.0);
    let widths = live.0.lines().take(3).map(str::len).collect::<Vec<_>>();
    assert!(
        widths.iter().all(|&width| width == widths[0]),
        "{:?}",
        live.0
    );
}

/// A shown Markdown tab in Live holding `text`, in a window narrow enough to wrap it.
fn narrow_live(text: &str) -> (support::win32::WindowHarness, TestMain, HWND) {
    let scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::SetWindowPos(
            main.hwnd,
            std::ptr::null_mut(),
            0,
            0,
            420,
            600,
            windows_sys::Win32::UI::WindowsAndMessaging::SWP_NOZORDER,
        );
    }
    main.make_markdown(text);
    main.editor().set_selection(0..0).unwrap();
    main.set_live(true);
    let live = main.live_view().unwrap().hwnd();
    unsafe {
        SetFocus(live);
        UpdateWindow(live);
    }
    pump_pending();
    (scintilla, main, live)
}

const LONG: &str = "The quick brown fox jumps over the lazy dog and keeps running past the \
                    river, the mill and the old stone bridge until night falls.";

#[test]
fn shift_down_extends_over_a_wrapped_row() {
    let (_scintilla, main, live) = narrow_live(&format!("{LONG}\nnext\n"));
    send_inputs(live, &[Input::Key(VK_DOWN, false, true)]);
    let (anchor, caret) = caret(&main);
    assert_eq!(anchor, 0);
    assert!(
        caret > 0 && caret < LONG.len(),
        "caret {caret} left the first line"
    );
}

#[test]
fn home_end_follow_rows() {
    let (_scintilla, main, live) = narrow_live(&format!("{LONG}\n"));
    send_inputs(live, &[Input::Key(VK_END, false, false)]);
    let (_, row_end) = caret(&main);
    assert!(row_end > 0 && row_end < LONG.len(), "End went to {row_end}");
    send_inputs(live, &[Input::Key(VK_DOWN, false, false)]);
    let (_, below) = caret(&main);
    send_inputs(live, &[Input::Key(VK_HOME, false, false)]);
    let (_, row_start) = caret(&main);
    assert!(
        row_start > row_end && row_start <= below,
        "Home went to {row_start}"
    );
    send_inputs(live, &[Input::Key(VK_HOME, false, false)]);
    assert_eq!(caret(&main).1, 0, "a second Home goes to the line's start");
}

#[test]
fn up_down_keep_the_goal_column() {
    let (_scintilla, main, live) = narrow_live("abcdef\nab\nabcdef");
    main.editor().set_selection(5..5).unwrap();
    send_inputs(live, &[Input::Key(VK_DOWN, false, false)]);
    assert_eq!(caret(&main).1, 9, "clamped to the short line's end");
    send_inputs(live, &[Input::Key(VK_DOWN, false, false)]);
    assert_eq!(caret(&main).1, 15, "back at the same column");
    send_inputs(
        live,
        &[
            Input::Key(VK_UP, false, false),
            Input::Key(VK_UP, false, false),
        ],
    );
    assert_eq!(caret(&main).1, 5);
}

/// Paints `view`, then checks every line it has laid out shows that line of `editor`'s document
/// as it is now.
fn assert_layout_matches(view: LiveView, editor: &editor::Editor) {
    unsafe { UpdateWindow(view.hwnd()) };
    pump_pending();
    let text = editor.text().unwrap();
    let lines = text
        .split('\n')
        .map(|line| line.trim_end_matches('\r'))
        .collect::<Vec<_>>();
    let laid = view.debug_lines();
    assert_eq!(laid.len(), lines.len(), "line count of {text:?}");
    assert!(laid.iter().any(Option::is_some), "nothing laid out");
    for (n, (laid, line)) in laid.iter().zip(&lines).enumerate() {
        if let Some(laid) = laid {
            assert_eq!(laid, line, "line {n} of {text:?}");
        }
    }
}

/// A shown Markdown tab in Live holding `text`, focused.
fn live_tab(text: &str) -> (support::win32::WindowHarness, TestMain, LiveView) {
    let scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    main.make_markdown(text);
    main.editor().set_selection(0..0).unwrap();
    main.set_live(true);
    let view = main.live_view().unwrap();
    unsafe {
        SetFocus(view.hwnd());
        UpdateWindow(view.hwnd());
    }
    pump_pending();
    (scintilla, main, view)
}

#[test]
fn undo_and_redo_redraw_the_changed_lines() {
    let (_scintilla, main, view) = live_tab("one\ntwo\n");
    send_inputs(
        view.hwnd(),
        &[
            Input::Key(VK_END, false, false),
            Input::Text(" more"),
            Input::Key(VK_RETURN, false, false),
            Input::Text("added"),
        ],
    );
    let editor = main.editor();
    assert_eq!(editor.text().unwrap(), "one more\r\nadded\ntwo\n");
    assert_layout_matches(view, &editor);
    main.command(CommandId::Undo);
    assert_layout_matches(view, &editor);
    main.command(CommandId::Undo);
    assert_layout_matches(view, &editor);
    main.command(CommandId::Redo);
    assert_layout_matches(view, &editor);
}

#[test]
fn undo_of_a_multi_line_paste_redraws_every_line() {
    let (_scintilla, main, view) = live_tab("first\nlast\n");
    let editor = main.editor();
    editor.replace_target(6..6, "a\nb\nc\nd\n").unwrap();
    assert_layout_matches(view, &editor);
    main.command(CommandId::Undo);
    assert_eq!(editor.text().unwrap(), "first\nlast\n");
    assert_layout_matches(view, &editor);
}

#[test]
fn an_external_change_while_live_shows_the_new_text() {
    let (_scintilla, main, view) = live_tab("old\ntext\n");
    main.set_text("entirely\nnew\ntext\nhere\n");
    assert_layout_matches(view, &main.editor());
}

fn long_document() -> String {
    (0..400).map(|n| format!("line {n}\n")).collect::<String>()
}

#[test]
fn scintilla_keys_and_commands_scroll_the_caret_into_view() {
    let (_scintilla, main, view) = live_tab(&long_document());
    assert_eq!(view.scroll_y(), 0.0);
    // Scintilla's own key, forwarded.
    send_inputs(view.hwnd(), &[Input::Key(VK_END, true, false)]);
    assert_eq!(caret(&main).1, main.editor().length().unwrap());
    unsafe { UpdateWindow(view.hwnd()) };
    assert!(view.scroll_y() > 0.0, "Ctrl+End scrolled to the end");
    // A command: Undo puts the caret back where the undone typing was.
    send_inputs(
        view.hwnd(),
        &[Input::Text("x"), Input::Key(VK_HOME, true, false)],
    );
    unsafe { UpdateWindow(view.hwnd()) };
    assert_eq!(view.scroll_y(), 0.0);
    main.command(CommandId::Undo);
    unsafe { UpdateWindow(view.hwnd()) };
    assert!(view.scroll_y() > 0.0, "Undo left the caret at the end");
}

#[test]
fn ctrl_d_switches_to_source_with_a_notice() {
    let (_scintilla, main, view) = live_tab("word word word\n");
    main.editor().set_selection(1..1).unwrap();
    main.command(CommandId::AddNextOccurrence);
    main.command(CommandId::AddNextOccurrence);
    assert!(main.editor().selection_count().unwrap() > 1);
    assert!(!visible(view.hwnd()));
    assert!(visible(main.editor().hwnd()));
    assert!(
        main.notices()
            .iter()
            .any(|notice| notice.starts_with("Live Markdown works with one selection"))
    );
}

#[test]
fn a_second_group_showing_the_note_follows_edits_in_live() {
    let (_scintilla, main, view) = live_tab("shared\n");
    main.command(CommandId::SplitRight);
    let second = main.editor();
    second.set_selection(6..6).unwrap();
    unsafe { SetFocus(second.hwnd()) };
    send_inputs(
        second.hwnd(),
        &[Input::Text(" text"), Input::Key(VK_RETURN, false, false)],
    );
    assert_layout_matches(view, &second);
}

#[test]
fn ctrl_wheel_zooms_live_and_source_together() {
    let (_scintilla, main, view) = live_tab("zoom me\n");
    let before = view.font_size();
    let wheel_up = (120_usize << 16) | 0x0008; // MK_CONTROL
    unsafe {
        SendMessageW(
            view.hwnd(),
            windows_sys::Win32::UI::WindowsAndMessaging::WM_MOUSEWHEEL,
            wheel_up,
            0,
        )
    };
    pump_until("the zoom", Duration::from_secs(2), || {
        view.font_size() > before
    });
    assert!(main.editor().zoom().unwrap() > 0);
}

#[test]
fn scrolling_live_scrolls_the_hidden_editor_and_the_split_preview() {
    let (_scintilla, main, view) = live_tab(&long_document());
    main.command(CommandId::MarkdownPreviewSide);
    unsafe {
        SendMessageW(
            view.hwnd(),
            windows_sys::Win32::UI::WindowsAndMessaging::WM_VSCROLL,
            windows_sys::Win32::UI::WindowsAndMessaging::SB_PAGEDOWN as usize,
            0,
        );
        SendMessageW(
            view.hwnd(),
            windows_sys::Win32::UI::WindowsAndMessaging::WM_VSCROLL,
            windows_sys::Win32::UI::WindowsAndMessaging::SB_PAGEDOWN as usize,
            0,
        );
    }
    assert!(view.scroll_y() > 0.0);
    pump_until("the editor to follow", Duration::from_secs(2), || {
        main.editor().first_visible_line().unwrap() > 0
    });
    let top = view.top_line();
    let editor = main.editor();
    let editor_top = editor
        .doc_line_from_visible(editor.first_visible_line().unwrap())
        .unwrap();
    assert_eq!(editor_top, top);
}

/// Sends a mouse message at client point (`x`, `y`) to `view`, with Ctrl and Shift as given.
fn mouse(view: HWND, message: u32, (x, y): (i32, i32), ctrl: bool, shift: bool) {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetKeyboardState, SetKeyboardState, VK_CONTROL, VK_SHIFT,
    };
    let mut keys = [0u8; 256];
    unsafe { GetKeyboardState(keys.as_mut_ptr()) };
    let original = keys;
    keys[usize::from(VK_CONTROL)] = if ctrl { 0x80 } else { 0 };
    keys[usize::from(VK_SHIFT)] = if shift { 0x80 } else { 0 };
    unsafe { SetKeyboardState(keys.as_ptr()) };
    let lparam = ((y as u32 & 0xFFFF) << 16 | (x as u32 & 0xFFFF)) as isize;
    unsafe { SendMessageW(view, message, 0, lparam) };
    unsafe { SetKeyboardState(original.as_ptr()) };
    pump_pending();
}

fn click(view: HWND, point: (i32, i32), ctrl: bool, shift: bool) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{WM_LBUTTONDOWN, WM_LBUTTONUP};
    mouse(view, WM_LBUTTONDOWN, point, ctrl, shift);
    mouse(view, WM_LBUTTONUP, point, ctrl, shift);
}

/// Client point `x_chars` average characters into row `row` (from the top of the view).
fn row_point(view: LiveView, row: i32, x: i32) -> (i32, i32) {
    let (height, left) = view.row_metrics();
    (left + x, height * row + height / 2)
}

#[test]
fn click_places_the_caret_on_a_wrapped_row() {
    let (_scintilla, main, live) = narrow_live(&format!("{LONG}\nnext\n"));
    let view = main.live_view().unwrap();
    click(live, row_point(view, 1, 2), false, false);
    let (anchor, second_row) = caret(&main);
    assert_eq!(anchor, second_row);
    assert!(
        second_row > 0 && second_row < LONG.len(),
        "caret at {second_row}"
    );
    // Past the end of the first row stays on that row.
    click(live, row_point(view, 0, 2000), false, false);
    let row_end = caret(&main).1;
    assert!(row_end < second_row, "{row_end} is on the first row");
}

#[test]
fn shift_click_extends() {
    let (_scintilla, main, live) = narrow_live(&format!("{LONG}\nnext\n"));
    let view = main.live_view().unwrap();
    click(live, row_point(view, 0, 0), false, false);
    click(live, row_point(view, 1, 30), false, true);
    let (anchor, caret) = caret(&main);
    assert_eq!(anchor, 0);
    assert!(caret > 0);
}

#[test]
fn double_click_selects_a_word_and_triple_click_the_line() {
    let (_scintilla, main, view) = live_tab("alpha beta gamma\nnext\n");
    let point = row_point(view, 0, 2);
    click(view.hwnd(), point, false, false);
    click(view.hwnd(), point, false, false);
    assert_eq!(caret(&main), (0, 5), "the word");
    click(view.hwnd(), point, false, false);
    let (anchor, caret) = caret(&main);
    assert_eq!(anchor, 0);
    assert_eq!(
        caret,
        main.editor().line_start(1).unwrap(),
        "the line and its ending"
    );
}

#[test]
fn drag_below_the_view_autoscrolls() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetClientRect, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE,
    };
    let (_scintilla, main, view) = live_tab(&long_document());
    let mut client = windows_sys::Win32::Foundation::RECT::default();
    unsafe { GetClientRect(view.hwnd(), &mut client) };
    mouse(
        view.hwnd(),
        WM_LBUTTONDOWN,
        row_point(view, 0, 0),
        false,
        false,
    );
    let below = (40, client.bottom + 60);
    mouse(view.hwnd(), WM_MOUSEMOVE, below, false, false);
    pump_until("the drag to scroll", Duration::from_secs(3), || {
        view.scroll_y() > 100.0
    });
    mouse(view.hwnd(), WM_LBUTTONUP, below, false, false);
    let (anchor, caret) = caret(&main);
    assert_eq!(anchor, 0);
    assert!(caret > 0, "the selection grew with the scroll");
    let after = view.scroll_y();
    pump_for(Duration::from_millis(150));
    assert_eq!(view.scroll_y(), after, "releasing stops the scrolling");
}

fn pump_for(duration: Duration) {
    let deadline = Instant::now() + duration;
    pump_until("pump_for", duration + Duration::from_secs(1), || {
        Instant::now() >= deadline
    });
}

#[test]
fn ctrl_click_follows_a_wikilink_and_takes_the_click() {
    use windows_sys::Win32::UI::WindowsAndMessaging::PeekMessageW;
    let (_scintilla, main, view) = live_tab("[[Other note]] and text\n");
    main.editor().set_selection(20..20).unwrap();
    // Pressed without pumping: the link is followed through the preview's link message, taken
    // here before anything opens.
    {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
            GetKeyboardState, SetKeyboardState, VK_CONTROL,
        };
        use windows_sys::Win32::UI::WindowsAndMessaging::WM_LBUTTONDOWN;
        let (x, y) = row_point(view, 0, 20);
        let mut keys = [0u8; 256];
        unsafe { GetKeyboardState(keys.as_mut_ptr()) };
        let original = keys;
        keys[usize::from(VK_CONTROL)] = 0x80;
        unsafe {
            SetKeyboardState(keys.as_ptr());
            SendMessageW(
                view.hwnd(),
                WM_LBUTTONDOWN,
                0,
                ((y as u32) << 16 | x as u32) as isize,
            );
            SetKeyboardState(original.as_ptr());
        }
    }
    let mut msg = MSG::default();
    let posted = unsafe {
        PeekMessageW(
            &mut msg,
            main.hwnd,
            window::WM_FASTPAD_PREVIEW_LINK,
            window::WM_FASTPAD_PREVIEW_LINK,
            PM_REMOVE,
        )
    } != 0;
    assert!(posted, "a link was followed");
    let destination = unsafe { Box::from_raw(msg.lParam as *mut String) };
    assert!(destination.contains("Other note"), "{destination}");
    assert_eq!(caret(&main), (20, 20), "the click did not move the caret");
    // Off a link, Ctrl+click places the caret.
    click(view.hwnd(), row_point(view, 0, 2000), true, false);
    assert!(caret(&main).1 > 20);
}

#[test]
fn system_caret_tracks_the_caret() {
    use windows_sys::Win32::UI::WindowsAndMessaging::GetCaretPos;
    let (_scintilla, _main, view) = live_tab("\n");
    let at = |view: LiveView| {
        unsafe { UpdateWindow(view.hwnd()) };
        let mut point = windows_sys::Win32::Foundation::POINT::default();
        unsafe { GetCaretPos(&mut point) };
        point
    };
    let before = at(view);
    send_inputs(view.hwnd(), &[Input::Text("abc")]);
    let after = at(view);
    assert!(after.x > before.x, "{} -> {}", before.x, after.x);
    send_inputs(view.hwnd(), &[Input::Key(VK_RETURN, false, false)]);
    assert!(at(view).y > after.y);
}

/// Sets the editor's `line_numbers` setting without writing `fastpad.ini`, and lets Live follow.
fn set_line_numbers(main: &TestMain, on: bool) {
    main.with_app(|app| app.settings.line_numbers = on);
    window::live_host::refresh_appearance(main.hwnd);
    pump_pending();
}

#[test]
fn line_numbers_follow_the_setting_and_move_the_text() {
    let (_scintilla, main, view) = live_tab("one\ntwo\nthree\n");
    set_line_numbers(&main, true);
    unsafe { UpdateWindow(view.hwnd()) };
    let with_numbers = view.gutter();
    let (_, text_left_numbered) = view.row_metrics();
    assert!(with_numbers > 0.0);
    set_line_numbers(&main, false);
    unsafe { UpdateWindow(view.hwnd()) };
    assert_eq!(view.gutter(), 0.0);
    let (_, text_left_plain) = view.row_metrics();
    assert!(text_left_numbered > text_left_plain);
    // A click at the text's left edge lands at the line start either way.
    set_line_numbers(&main, true);
    unsafe { UpdateWindow(view.hwnd()) };
    click(view.hwnd(), row_point(view, 1, 1), false, false);
    assert_eq!(caret(&main), (4, 4));
}

#[test]
fn the_gutter_widens_for_three_digit_line_numbers() {
    let (_scintilla, main, view) = live_tab("a\nb\n");
    set_line_numbers(&main, true);
    unsafe { UpdateWindow(view.hwnd()) };
    let two_digits = view.gutter();
    main.set_text(&"line\n".repeat(150));
    unsafe { UpdateWindow(view.hwnd()) };
    assert!(view.gutter() > two_digits);
}

#[test]
fn ime_result_inserts_one_undo_step() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_IME_CHAR};
    let (_scintilla, main, view) = live_tab("");
    for unit in "日本".encode_utf16() {
        unsafe { PostMessageW(view.hwnd(), WM_IME_CHAR, usize::from(unit), 1) };
    }
    pump_pending();
    let editor = main.editor();
    assert_eq!(editor.text().unwrap(), "日本");
    main.command(CommandId::Undo);
    assert_eq!(editor.text().unwrap(), "");
}

#[test]
fn the_default_setting_opens_markdown_in_live() {
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    main.with_app(|app| app.settings.markdown_live_default = true);
    main.make_markdown("# Opens Live\n");
    let view = main.live_view().expect("a Live window");
    assert!(visible(view.hwnd()));
    assert!(!visible(main.editor().hwnd()));
    // Ctrl+Alt+V still turns it off for this tab, and the choice sticks.
    main.command(CommandId::MarkdownToggleLive);
    assert!(!visible(view.hwnd()));
    main.command(CommandId::New);
    main.command(CommandId::PreviousTab);
    assert!(!visible(view.hwnd()));
    assert!(visible(main.editor().hwnd()));
}

#[test]
fn the_default_setting_skips_files_over_the_limit_silently() {
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    main.with_app(|app| app.settings.markdown_live_default = true);
    main.make_markdown(&"x".repeat(live::LIVE_MAX_BYTES + 1));
    assert!(main.live_view().is_none());
    assert!(visible(main.editor().hwnd()));
    assert!(main.notices().is_empty(), "{:?}", main.notices());
}

/// A Markdown document of about `bytes` with headings, prose, lists, a table and fenced code in
/// every section, as `markdown_preview.rs` measures the preview with.
fn sample_markdown(bytes: usize) -> String {
    let section = "## Heading\n\nParagraph with **bold**, *emphasis*, `code`, and a [link](https://x.dev).\n\n- item one\n- item two\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n```rust\nfn f() {}\n```\n\n";
    section.repeat(bytes / section.len() + 1)[..bytes]
        .rsplit_once("\n\n")
        .map_or_else(String::new, |(text, _)| format!("{text}\n"))
}

fn p95(mut samples: Vec<u64>) -> u64 {
    samples.sort_unstable();
    samples[(samples.len() * 95 / 100).min(samples.len() - 1)]
}

/// Keystroke-to-painted p95 in microseconds, typing into `target` with the caret at `caret`:
/// each `x` is sent, the messages it queued are handled, and `target` paints. Runs start on a
/// fresh line and break it every 40 characters (unmeasured), as the preview's test does.
fn typing_p95(main: &TestMain, target: HWND, caret: usize) -> u64 {
    main.editor().set_selection(caret..caret).unwrap();
    pump_pending();
    let mut samples = Vec::new();
    send_inputs(target, &[Input::Key(VK_RETURN, false, false)]);
    for index in 0..200 {
        if index % 40 == 0 {
            send_inputs(target, &[Input::Key(VK_RETURN, false, false)]);
        }
        let started = Instant::now();
        unsafe {
            SendMessageW(
                target,
                windows_sys::Win32::UI::WindowsAndMessaging::WM_CHAR,
                usize::from(b'x'),
                0,
            )
        };
        pump_pending();
        unsafe { UpdateWindow(target) };
        samples.push(started.elapsed().as_micros() as u64);
    }
    p95(samples)
}

/// A Live keystroke's p95 budget: a quarter of a 60 Hz frame. The source view is measured
/// alongside for comparison; matching it within 10% needs a present limited to the changed
/// rectangle, planned for phase 4 (phase 1 plan, deviation 11).
const LIVE_KEYSTROKE_BUDGET_MICROS: u64 = 4_000;

/// Typing in Live against typing in the source view on the same document, in one window: three
/// interleaved rounds of each, keeping each one's best p95, so a busy moment on the machine
/// during one round does not decide the comparison.
fn live_against_source(text: &str, caret: usize) -> (u64, u64) {
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    main.make_markdown(text);
    let (mut source, mut live) = (u64::MAX, u64::MAX);
    for _ in 0..3 {
        main.set_live(false);
        let editor = main.editor().hwnd();
        unsafe { SetFocus(editor) };
        source = source.min(typing_p95(&main, editor, caret));
        main.set_live(true);
        let view = main.live_view().unwrap().hwnd();
        unsafe {
            SetFocus(view);
            UpdateWindow(view);
        }
        pump_pending();
        let _ = live::view::take_timings();
        live = live.min(typing_p95(&main, view, caret));
    }
    // Where a Live keystroke's time went, in the last round.
    let mut phases = std::collections::BTreeMap::<&str, Vec<u64>>::new();
    for (name, micros) in live::view::take_timings() {
        phases.entry(name).or_default().push(micros);
    }
    for (name, mut samples) in phases {
        samples.sort_unstable();
        println!(
            "  {name}: p50 {} us, p95 {} us over {}",
            samples[samples.len() / 2],
            samples[(samples.len() * 95 / 100).min(samples.len() - 1)],
            samples.len()
        );
    }
    (source, live)
}

#[test]
#[ignore = "performance measurement: cargo test --release --test markdown_live typing_in_live -- --ignored --test-threads=1"]
fn typing_in_live_costs_the_same_as_typing_in_source() {
    let (source, live) = live_against_source(&sample_markdown(1_000_000), 500_000);
    println!("keystroke p95, 1 MB: source {source} us, live {live} us");
    assert!(
        live <= LIVE_KEYSTROKE_BUDGET_MICROS,
        "source {source} us, live {live} us"
    );
}

#[test]
#[ignore = "performance measurement: cargo test --release --test markdown_live typing_in_live -- --ignored --test-threads=1"]
fn typing_in_live_on_a_10k_line_note_costs_the_same_as_typing_in_source() {
    let text = (0..10_000)
        .map(|n| format!("Line {n} of a long note, with some words to wrap.\n"))
        .collect::<String>();
    let caret = text.len() / 2;
    let (source, live) = live_against_source(&text, caret);
    println!("keystroke p95, 10k lines: source {source} us, live {live} us");
    assert!(
        live <= LIVE_KEYSTROKE_BUDGET_MICROS,
        "source {source} us, live {live} us"
    );
}
