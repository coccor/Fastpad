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
