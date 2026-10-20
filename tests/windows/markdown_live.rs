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

/// 200 one-line paragraphs, one per even line: `**needle** here` on line 300 and
/// `a needle too` on line 100.
fn needle_document() -> String {
    (0..200)
        .map(|n| match n {
            50 => "a needle too\n\n".to_owned(),
            150 => "**needle** here\n\n".to_owned(),
            n => format!("para {n}\n\n"),
        })
        .collect()
}

/// The caret at byte `position` is drawn inside the Live view's client area, after a paint.
fn on_screen(view: LiveView, position: usize) -> bool {
    unsafe { UpdateWindow(view.hwnd()) };
    pump_pending();
    let mut client = unsafe { std::mem::zeroed() };
    unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect(view.hwnd(), &mut client) };
    view.client_point_of(position)
        .is_some_and(|(_, y)| (0..client.bottom).contains(&y))
}

/// The match's byte on line 300 or line 100 of `needle_document`.
fn needle_at(text: &str, line: usize) -> usize {
    match line {
        300 => text.find("needle**").unwrap(),
        _ => text.find("needle too").unwrap(),
    }
}

fn open_find_bar(main: &TestMain, command: CommandId) -> HWND {
    main.command(command);
    main.with_app(|app| app.find_bar().unwrap().query_hwnd())
}

fn type_into(hwnd: HWND, text: &str) {
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
    pump_pending();
}

fn press(hwnd: HWND, key: u16) {
    unsafe {
        SendMessageW(
            hwnd,
            windows_sys::Win32::UI::WindowsAndMessaging::WM_KEYDOWN,
            key as usize,
            0,
        )
    };
    pump_pending();
}

#[test]
fn find_from_the_find_bar_moves_live_to_the_match() {
    let text = needle_document();
    let (_scintilla, main, view) = live_tab(&text);
    assert!(!on_screen(view, needle_at(&text, 300)));
    let query = open_find_bar(&main, CommandId::Find);
    type_into(query, "needle");
    // Past the first match, so Enter lands on line 300's.
    caret_to(&main, text.find("para 51").unwrap());
    press(query, VK_RETURN);
    let at = text.find("needle**").unwrap();
    assert_eq!(main.editor().selection().unwrap(), at..at + 6);
    assert!(
        on_screen(view, needle_at(&text, 300)),
        "top line {}",
        view.top_line()
    );
    assert_eq!(view.revealed(), 300..301);
    assert_eq!(shown_line(view, 300).0, "**needle** here");
    assert_eq!(
        unsafe { GetFocus() },
        query,
        "focus stays in the find field"
    );
    // F3 (find again, through command dispatch) wraps to line 100's.
    main.command(CommandId::FindNext);
    assert!(
        on_screen(view, needle_at(&text, 100)),
        "top line {}",
        view.top_line()
    );
    assert_eq!(view.revealed(), 100..101);
}

#[test]
fn find_previous_and_replace_follow_in_live() {
    let text = needle_document();
    let (_scintilla, main, view) = live_tab(&text);
    let query = open_find_bar(&main, CommandId::Replace);
    type_into(query, "needle");
    let replacement = main.with_app(|app| app.find_bar().unwrap().replace_hwnd());
    type_into(replacement, "pin");
    caret_to(&main, text.len());
    window::find_previous(main.hwnd);
    assert!(on_screen(view, needle_at(&text, 300)));
    assert_eq!(view.revealed(), 300..301);
    // Replaces line 300's match, then moves to (and reveals) line 100's.
    window::replace_current(main.hwnd);
    assert!(main.editor().text().unwrap().contains("**pin** here"));
    assert!(
        on_screen(view, needle_at(&text, 100)),
        "top line {}",
        view.top_line()
    );
    assert_eq!(view.revealed(), 100..101);
    assert_eq!(shown_line(view, 300).0, "pin here");
}

#[test]
fn replace_all_keeps_live_in_step() {
    let text = needle_document();
    let (_scintilla, main, view) = live_tab(&text);
    let query = open_find_bar(&main, CommandId::Replace);
    type_into(query, "needle");
    let replacement = main.with_app(|app| app.find_bar().unwrap().replace_hwnd());
    type_into(replacement, "pin");
    window::replace_all_matches(main.hwnd);
    let editor = main.editor();
    assert!(!editor.text().unwrap().contains("needle"));
    let caret = editor.selection().unwrap().end;
    assert!(
        on_screen(view, caret),
        "caret {caret}, top {}",
        view.top_line()
    );
    let line = editor.line_from_position(caret).unwrap();
    assert_eq!(
        shown_line(view, line).0,
        editor.line_text(line).unwrap().trim_end()
    );
}

#[test]
fn go_to_line_moves_live() {
    let text = needle_document();
    let (_scintilla, main, view) = live_tab(&text);
    window::go_to_line(main.hwnd, 301);
    let editor = main.editor();
    assert_eq!(
        editor.selection().unwrap(),
        editor.line_start(300).unwrap()..editor.line_start(300).unwrap()
    );
    assert!(
        on_screen(view, needle_at(&text, 300)),
        "top line {}",
        view.top_line()
    );
    assert_eq!(view.revealed(), 300..301);
    assert_eq!(shown_line(view, 300).0, "**needle** here");
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
    assert_eq!(
        main.notices(),
        ["Add next occurrence needs the source view. Switched to the source view."]
    );
}

#[test]
fn fold_all_switches_to_source_with_a_notice() {
    let (_scintilla, main, view) = live_tab(
        "# One

text

# Two

more
",
    );
    main.command(CommandId::FoldAll);
    assert!(!visible(view.hwnd()));
    assert!(visible(main.editor().hwnd()));
    assert_eq!(
        main.notices(),
        ["Fold all needs the source view. Switched to the source view."]
    );
    // Turning Live back on shows every line again.
    main.set_live(true);
    assert!(visible(view.hwnd()));
}

#[test]
fn a_one_row_rectangular_selection_switches_to_source() {
    const SCI_SETSELECTIONMODE: u32 = 2422;
    const SC_SEL_RECTANGLE: usize = 1;
    const SCI_SETRECTANGULARSELECTIONANCHOR: u32 = 2590;
    const SCI_SETRECTANGULARSELECTIONCARET: u32 = 2588;
    let (_scintilla, main, view) = live_tab(
        "abcdef
ghijkl
",
    );
    let editor = main.editor();
    unsafe {
        SendMessageW(editor.hwnd(), SCI_SETSELECTIONMODE, SC_SEL_RECTANGLE, 0);
        SendMessageW(editor.hwnd(), SCI_SETRECTANGULARSELECTIONANCHOR, 1, 0);
        SendMessageW(editor.hwnd(), SCI_SETRECTANGULARSELECTIONCARET, 3, 0);
    }
    assert_eq!(editor.selection_count().unwrap(), 1);
    window::live_host::poll_active(main.hwnd);
    pump_pending();
    assert!(!visible(view.hwnd()));
    assert_eq!(
        main.notices(),
        ["Live Markdown cannot show a column selection. Switched to the source view."]
    );
}

#[test]
fn turning_off_current_line_highlight_drops_the_block_fill() {
    let text = "top

**one**
**two**

after
";
    let (_scintilla, main, view) = live_tab(text);
    caret_to(&main, text.find("two").unwrap());
    painted(view);
    assert_eq!(view.painted_decorations().active_block, 2);
    main.command(CommandId::ToggleHighlightCurrentLine);
    painted(view);
    assert_eq!(view.painted_decorations().active_block, 0);
    main.command(CommandId::ToggleHighlightCurrentLine);
    painted(view);
    assert_eq!(view.painted_decorations().active_block, 2);
}

#[test]
fn source_only_names_the_commands_that_need_scintillas_ui() {
    let named = [
        CommandId::AddNextOccurrence,
        CommandId::SelectAllOccurrences,
        CommandId::AddCursorAbove,
        CommandId::AddCursorBelow,
        CommandId::ToggleCodeFolding,
        CommandId::FoldAll,
        CommandId::UnfoldAll,
    ];
    for command in named {
        assert!(
            window::live_host::source_only(command).is_some(),
            "{command:?}"
        );
    }
    for command in window::commands::TEXT_COMMANDS {
        assert_eq!(
            window::live_host::source_only(command).is_some(),
            named.contains(&command),
            "{command:?}"
        );
    }
}

const AUDIT_NOTE: &str = "# Title

Some **bold** text
second line

- item one
- item two
";

/// `command` run on `AUDIT_NOTE` with "bold" selected, in Live or in the source view: the text
/// after, whether Live is still shown, and the notices.
fn audited(command: CommandId, live: bool) -> (String, bool, Vec<String>) {
    let scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    main.make_markdown(AUDIT_NOTE);
    let bold = AUDIT_NOTE.find("bold").unwrap();
    main.editor().set_selection(bold..bold + 4).unwrap();
    main.set_live(live);
    let view = main.live_view();
    if let Some(view) = view.filter(|_| live) {
        unsafe { SetFocus(view.hwnd()) };
    } else {
        unsafe { SetFocus(main.editor().hwnd()) };
    }
    pump_pending();
    main.command(command);
    pump_pending();
    let shown = view.is_some_and(|view| visible(view.hwnd()));
    if shown {
        // Live followed the command's selection: the caret's block is the one revealed.
        let editor = main.editor();
        assert_eq!(editor.selection_count().unwrap(), 1, "{command:?}");
        let caret = editor.anchor_and_caret().unwrap().1;
        let line = editor.line_from_position(caret).unwrap();
        let view = view.unwrap();
        unsafe { UpdateWindow(view.hwnd()) };
        if !editor.line_text(line).unwrap().trim().is_empty() {
            assert!(
                view.revealed().contains(&line),
                "{command:?}: caret line {line}, revealed {:?}",
                view.revealed()
            );
        }
    }
    let text = main.editor().text().unwrap();
    let notices = main.notices();
    main.mark_clean();
    drop(main);
    drop(scintilla);
    (text, shown, notices)
}

#[test]
fn every_text_command_works_in_live_or_switches_to_source() {
    let mut commands = window::commands::TEXT_COMMANDS.to_vec();
    for command in window::commands::EDITING_COMMANDS {
        if !commands.contains(&command) {
            commands.push(command);
        }
    }
    for command in commands {
        // Skipped, with why: these leave Live by design or need a dialog or a file.
        let skip = match command {
            CommandId::Save | CommandId::SaveAs => Some("opens the save dialog"),
            CommandId::MarkdownToggleLive => Some("is the toggle itself"),
            CommandId::NoteReloadFromDisk
            | CommandId::NoteKeepMine
            | CommandId::NoteCompareWithDisk => Some("needs a file changed on disk"),
            command
                if command != CommandId::LanguageMarkdown
                    && format!("{command:?}").starts_with("Language") =>
            {
                Some("makes the tab not Markdown, which leaves Live with its own notice")
            }
            _ => None,
        };
        if skip.is_some() {
            continue;
        }
        let (live_text, shown, notices) = audited(command, true);
        let (source_text, _, _) = audited(command, false);
        if let Some(name) = window::live_host::source_only(command) {
            assert!(!shown, "{command:?} left Live");
            assert!(
                notices.contains(&format!(
                    "{name} needs the source view. Switched to the source view."
                )),
                "{command:?}: {notices:?}"
            );
        } else {
            assert!(shown, "{command:?} kept Live: {notices:?}");
            assert!(
                !notices
                    .iter()
                    .any(|notice| notice.contains("Switched to the source view")),
                "{command:?}: {notices:?}"
            );
        }
        assert_eq!(
            live_text, source_text,
            "{command:?} edits as in the source view"
        );
    }
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

/// Headings, paragraphs and a table every tenth section: blocks the preview and Live size
/// differently, so a sync by document line is what keeps them together.
fn sectioned_document() -> String {
    (0..120)
        .map(|n| {
            let mut section = format!(
                "## Heading {n}

Some **text** for section {n}.

"
            );
            if n % 10 == 0 {
                section.push_str(
                    "| a | b |
|---|---|
| 1 | 2 |
| 3 | 4 |

",
                );
            }
            section
        })
        .collect()
}

/// A shown Live tab holding `text`, with the preview beside it.
fn live_split(text: &str) -> (support::win32::WindowHarness, TestMain, LiveView, HWND) {
    let (scintilla, main, view) = live_tab(text);
    main.command(CommandId::MarkdownPreviewSide);
    let preview = main.with_app(|app| app.active_group().unwrap().preview.view.unwrap());
    pump_until("the preview to lay out", Duration::from_secs(5), || {
        preview.stats().block_count > 1
    });
    (scintilla, main, view, preview.hwnd())
}

fn scroll_page_down(hwnd: HWND, pages: usize) {
    for _ in 0..pages {
        unsafe {
            SendMessageW(
                hwnd,
                windows_sys::Win32::UI::WindowsAndMessaging::WM_VSCROLL,
                windows_sys::Win32::UI::WindowsAndMessaging::SB_PAGEDOWN as usize,
                0,
            )
        };
    }
    pump_pending();
}

fn sync_count(main: &TestMain) -> u64 {
    main.with_app(|app| app.active_group().unwrap().preview.sync_count)
}

#[test]
fn scrolling_the_split_preview_scrolls_live() {
    let (_scintilla, main, view, preview) = live_split(&sectioned_document());
    let preview_view = main.with_app(|app| app.active_group().unwrap().preview.view.unwrap());
    for pages in [3, 4, 5] {
        scroll_page_down(preview, pages);
        let target = preview_view.top_line();
        pump_until("Live to follow the preview", Duration::from_secs(2), || {
            view.top_line() == target
        });
        // The hidden editor keeps the place for a switch back to source.
        let editor = main.editor();
        assert_eq!(
            editor
                .doc_line_from_visible(editor.first_visible_line().unwrap())
                .unwrap(),
            target
        );
    }
}

#[test]
fn split_sync_settles_after_scrolling_either_side() {
    let (_scintilla, main, view, preview) = live_split(&sectioned_document());
    let preview_view = main.with_app(|app| app.active_group().unwrap().preview.view.unwrap());
    let before = sync_count(&main);
    scroll_page_down(view.hwnd(), 3);
    pump_until("the preview to follow Live", Duration::from_secs(2), || {
        sync_count(&main) > before
    });
    let after_live = sync_count(&main);
    scroll_page_down(preview, 3);
    pump_until("Live to follow the preview", Duration::from_secs(2), || {
        view.top_line() == preview_view.top_line()
    });
    let after_preview = sync_count(&main);
    assert!(after_preview > after_live);
    // No echo left behind: Live's next scroll still moves the preview.
    let preview_top = preview_view.top_line();
    scroll_page_down(view.hwnd(), 2);
    pump_until(
        "the preview to follow Live again",
        Duration::from_secs(2),
        || preview_view.top_line() > preview_top,
    );
    // And everything settles.
    let (live_top, preview_top, count) =
        (view.top_line(), preview_view.top_line(), sync_count(&main));
    pump_for(Duration::from_millis(300));
    assert_eq!(
        (view.top_line(), preview_view.top_line(), sync_count(&main)),
        (live_top, preview_top, count)
    );
}

#[test]
fn toggling_to_source_after_a_preview_scroll_keeps_the_place() {
    let (_scintilla, main, view, preview) = live_split(&sectioned_document());
    let preview_view = main.with_app(|app| app.active_group().unwrap().preview.view.unwrap());
    scroll_page_down(preview, 4);
    let target = preview_view.top_line();
    pump_until("Live to follow the preview", Duration::from_secs(2), || {
        view.top_line() == target
    });
    main.set_live(false);
    let editor = main.editor();
    assert_eq!(
        editor
            .doc_line_from_visible(editor.first_visible_line().unwrap())
            .unwrap(),
        target
    );
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
/// How a perf test types.
#[derive(Clone, Copy)]
enum Typing {
    /// `x`s, with an Enter every 40 (so a line never grows past a few rows).
    Lines,
    /// `x` and Backspace in turn, so the text (a table cell) keeps its width.
    InPlace,
}

fn typing_p95(main: &TestMain, target: HWND, caret: usize, typing: Typing) -> u64 {
    use windows_sys::Win32::UI::WindowsAndMessaging::{WM_CHAR, WM_KEYDOWN, WM_KEYUP};
    main.editor().set_selection(caret..caret).unwrap();
    pump_pending();
    let mut samples = Vec::new();
    if let Typing::Lines = typing {
        send_inputs(target, &[Input::Key(VK_RETURN, false, false)]);
    }
    for index in 0..200 {
        if matches!(typing, Typing::Lines) && index % 40 == 0 {
            send_inputs(target, &[Input::Key(VK_RETURN, false, false)]);
        }
        let started = Instant::now();
        if matches!(typing, Typing::InPlace) && index % 2 == 1 {
            unsafe {
                SendMessageW(target, WM_KEYDOWN, usize::from(VK_BACK), 0);
                SendMessageW(target, WM_CHAR, 0x08, 0);
                SendMessageW(target, WM_KEYUP, usize::from(VK_BACK), 0);
            }
        } else {
            unsafe { SendMessageW(target, WM_CHAR, usize::from(b'x'), 0) };
        }
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
    live_against_source_typing(text, caret, Typing::Lines)
}

fn live_against_source_typing(text: &str, caret: usize, typing: Typing) -> (u64, u64) {
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    main.make_markdown(text);
    let (mut source, mut live) = (u64::MAX, u64::MAX);
    for _ in 0..3 {
        main.set_live(false);
        let editor = main.editor().hwnd();
        unsafe { SetFocus(editor) };
        source = source.min(typing_p95(&main, editor, caret, typing));
        main.set_live(true);
        let view = main.live_view().unwrap().hwnd();
        unsafe {
            SetFocus(view);
            UpdateWindow(view);
        }
        pump_pending();
        let _ = live::view::take_timings();
        live = live.min(typing_p95(&main, view, caret, typing));
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
fn typing_in_live_inside_a_long_paragraph_stays_in_budget() {
    // Every keystroke reparses this 40-line block and restyles its lines.
    let paragraph = (0..40)
        .map(|n| format!("Line {n} with **bold**, *italic*, `code` and a [link](u{n}).\n"))
        .collect::<String>();
    let text = format!("# Notes\n\n{paragraph}\n{}", sample_markdown(200_000));
    let caret = text.find("Line 20").unwrap() + 4;
    let (source, live) = live_against_source(&text, caret);
    println!("keystroke p95, long paragraph: source {source} us, live {live} us");
    assert!(
        live <= LIVE_KEYSTROKE_BUDGET_MICROS,
        "source {source} us, live {live} us"
    );
}

#[test]
#[ignore = "performance measurement: cargo test --release --test markdown_live typing_in_live -- --ignored --test-threads=1"]
fn typing_in_live_inside_a_long_list_stays_in_budget() {
    // Every keystroke reparses the whole 300-item list (one block) and restyles its lines.
    let list = (0..300)
        .map(|n| format!("{}- item {n} with a few words\n", "  ".repeat(n % 3)))
        .collect::<String>();
    let text = format!("# Notes\n\n{list}\n{}", sample_markdown(100_000));
    let caret = text.find("item 150 with a few words").unwrap() + "item 150 with".len();
    let (source, live) = live_against_source(&text, caret);
    println!("keystroke p95, long list: source {source} us, live {live} us");
    assert!(
        live <= LIVE_KEYSTROKE_BUDGET_MICROS,
        "source {source} us, live {live} us"
    );
}

#[test]
#[ignore = "performance measurement: cargo test --release --test markdown_live typing_in_live -- --ignored --test-threads=1"]
fn typing_in_live_inside_a_table_stays_in_budget() {
    let mut table = String::from("| one | two | three | four | five |\n|---|---|---|---|---|\n");
    for row in 0..60 {
        table.push_str(&format!(
            "| r{row}c0 | r{row}c1 | r{row}c2 | r{row}c3 | r{row}c4 |\n"
        ));
    }
    let text = format!("# Table\n\n{table}\n{}", sample_markdown(100_000));
    let caret = text.find("r30c2").unwrap() + 3;
    let (source, live) = live_against_source_typing(&text, caret, Typing::InPlace);
    println!("keystroke p95, table: source {source} us, live {live} us");
    assert!(
        live <= LIVE_KEYSTROKE_BUDGET_MICROS,
        "source {source} us, live {live} us"
    );
}

#[test]
#[ignore = "performance measurement: cargo test --release --test markdown_live typing_in_live -- --ignored --test-threads=1"]
fn typing_in_live_inside_a_code_block_stays_in_budget() {
    let code = (0..500)
        .map(|n| format!("let value_{n} = compute({n});\n"))
        .collect::<String>();
    let text = format!(
        "# Code\n\n```rust\n{code}```\n\n{}",
        sample_markdown(100_000)
    );
    let caret = text.find("let value_250 =").unwrap() + "let value_250".len();
    let (source, live) = live_against_source(&text, caret);
    println!("keystroke p95, code block: source {source} us, live {live} us");
    assert!(
        live <= LIVE_KEYSTROKE_BUDGET_MICROS,
        "source {source} us, live {live} us"
    );
}

#[test]
#[ignore = "performance measurement: cargo test --release --test markdown_live typing_in_live -- --ignored --test-threads=1"]
fn typing_in_live_entering_live_on_a_1_mb_note_is_quick() {
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    // The process's first Direct2D device costs a one-time ~150 ms whatever is drawn; warm it
    // on a small note so this measures what entering Live costs per document.
    main.make_markdown("# Warm up\n");
    main.set_live(true);
    unsafe { UpdateWindow(main.live_view().unwrap().hwnd()) };
    main.set_live(false);
    main.make_markdown(&sample_markdown(1_000_000));
    // The best of three entries, as the typing tests take the best of three rounds: run after
    // the other perf tests in one process, a single entry swings by tens of milliseconds.
    let mut elapsed = u128::MAX;
    for _ in 0..3 {
        main.set_live(false);
        unsafe { SetFocus(main.editor().hwnd()) };
        pump_pending();
        let started = Instant::now();
        main.set_live(true);
        let view = main.live_view().unwrap();
        unsafe { UpdateWindow(view.hwnd()) };
        elapsed = elapsed.min(started.elapsed().as_millis());
    }
    println!("entering Live on 1 MB: {elapsed} ms to the first frame");
    assert!(elapsed <= 200, "{elapsed} ms");
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

/// Line `line` of the Live view as laid out now: (layout text, revealed, height).
fn shown_line(view: LiveView, line: usize) -> (String, bool, f32) {
    unsafe { UpdateWindow(view.hwnd()) };
    pump_pending();
    view.debug_line(line).expect("a laid-out line")
}

/// Moves the caret to `at` the way a command does, so the view follows it.
fn caret_to(main: &TestMain, at: usize) {
    main.editor().set_selection(at..at).unwrap();
    window::live_host::poll_active(main.hwnd);
    pump_pending();
}

#[test]
fn rendered_lines_hide_markup_and_the_caret_line_shows_source() {
    let (_scintilla, main, view) = live_tab("# Title\n\n**bold** and [link](u)\n");
    let row = view.row_metrics().0 as f32;
    caret_to(&main, 12);
    let (title, revealed, height) = shown_line(view, 0);
    assert_eq!(title, "Title");
    assert!(!revealed);
    assert!(
        height > row * 1.5,
        "a rendered H1 is taller: {height} vs {row}"
    );
    assert_eq!(shown_line(view, 2).0, "**bold** and [link](u)");
    assert_eq!(view.revealed(), 2..3);

    caret_to(&main, 3);
    assert_eq!(shown_line(view, 0).0, "# Title");
    assert_eq!(shown_line(view, 2).0, "bold and link");
}

#[test]
fn a_selection_reveals_every_block_it_touches() {
    let text = "**a**\n**b**\n\n**c**\n\n**d**\n";
    let (_scintilla, main, view) = live_tab(text);
    // From inside `a` to inside `c`: the first block, the blank line and the second block.
    let end = text.find("c**").unwrap();
    main.editor().set_selection(1..end).unwrap();
    window::live_host::poll_active(main.hwnd);
    assert_eq!(view.revealed(), 0..4);
    assert_eq!(shown_line(view, 1).0, "**b**");
    assert_eq!(shown_line(view, 3).0, "**c**");
    assert_eq!(shown_line(view, 5).0, "d");
}

#[test]
fn the_caret_reveals_its_whole_paragraph() {
    let text = "top\n\n**one**\n**two**\n**three**\n\nafter\n";
    let (_scintilla, main, view) = live_tab(text);
    caret_to(&main, text.find("two").unwrap());
    assert_eq!(view.revealed(), 2..5);
    for (line, source) in [(2, "**one**"), (3, "**two**"), (4, "**three**")] {
        assert_eq!(shown_line(view, line).0, source);
    }
    assert_eq!(shown_line(view, 0).0, "top");
    assert_eq!(shown_line(view, 6).0, "after");
}

#[test]
fn a_code_block_reveals_with_its_fences() {
    let text = "top\n\n```rust\nlet a;\n\nlet b;\n```\n\nafter\n";
    let (_scintilla, main, view) = live_tab(text);
    caret_to(&main, text.find("let b").unwrap());
    // The blank line inside the fence does not split the block.
    assert_eq!(view.revealed(), 2..7);
    assert_eq!(shown_line(view, 2).0, "```rust");
    assert_eq!(shown_line(view, 6).0, "```");
    caret_to(&main, 0);
    assert_eq!(shown_line(view, 2).0, "rust");
}

#[test]
fn a_revealed_heading_keeps_its_size_and_bold_stays_bold() {
    let text = "top\n\n# Big **title**\n\nafter\n";
    let (_scintilla, main, view) = live_tab(text);
    caret_to(&main, 0);
    let (shown, revealed, rendered_height) = shown_line(view, 2);
    assert_eq!(shown, "Big title");
    assert!(!revealed);
    caret_to(&main, text.find("Big").unwrap());
    let (shown, revealed, height) = shown_line(view, 2);
    assert_eq!(shown, "# Big **title**");
    assert!(revealed);
    assert_eq!(height, rendered_height, "the heading keeps its size");
}

#[test]
fn the_edited_block_is_highlighted() {
    let text = "one\ntwo\n\nthree\n";
    let (_scintilla, main, view) = live_tab(text);
    caret_to(&main, 0);
    full_frame(view);
    assert_eq!(view.painted_decorations().active_block, 2);
    caret_to(&main, text.find("three").unwrap());
    full_frame(view);
    assert_eq!(view.painted_decorations().active_block, 1);
}

#[test]
fn typing_bold_markers_restyles_only_the_caret_line_until_it_is_left() {
    let (_scintilla, main, view) = live_tab("first\n\nsecond\n");
    caret_to(&main, 5);
    send_inputs(view.hwnd(), &[Input::Text(" **b**")]);
    assert_eq!(
        shown_line(view, 0).0,
        "first **b**",
        "the caret line stays source"
    );
    send_inputs(view.hwnd(), &[Input::Key(VK_DOWN, false, false)]);
    assert_eq!(
        shown_line(view, 0).0,
        "first b",
        "rendered once the caret leaves"
    );
    assert_eq!(shown_line(view, 2).0, "second");
}

#[test]
fn opening_a_fence_restyles_the_lines_below() {
    let (_scintilla, main, view) = live_tab("top\n\n**x**\n\n**y**\n");
    assert_eq!(shown_line(view, 2).0, "x");
    caret_to(&main, 3);
    send_inputs(
        view.hwnd(),
        &[Input::Key(VK_RETURN, false, false), Input::Text("```")],
    );
    // Everything below an unclosed fence is code: shown as written.
    assert_eq!(shown_line(view, 3).0, "**x**");
    assert_eq!(shown_line(view, 5).0, "**y**");
}

#[test]
fn undo_restores_the_rendering() {
    let (_scintilla, main, view) = live_tab("plain\n\nend\n");
    caret_to(&main, 5);
    send_inputs(view.hwnd(), &[Input::Text(" *it*")]);
    caret_to(&main, main.editor().length().unwrap());
    assert_eq!(shown_line(view, 0).0, "plain it");
    main.command(CommandId::Undo);
    caret_to(&main, main.editor().length().unwrap());
    assert_eq!(main.editor().text().unwrap(), "plain\n\nend\n");
    assert_eq!(shown_line(view, 0).0, "plain");
    main.command(CommandId::Redo);
    caret_to(&main, main.editor().length().unwrap());
    assert_eq!(shown_line(view, 0).0, "plain it");
}

#[test]
fn a_replace_all_reparses_once() {
    let text = "x *a* y\n".repeat(200);
    let (_scintilla, main, view) = live_tab(&text);
    unsafe { UpdateWindow(view.hwnd()) };
    pump_pending();
    let before = view.document_updates();
    let replaced = main.editor().replace_all("a", "bb", 0).unwrap();
    assert_eq!(replaced, 200);
    caret_to(&main, main.editor().length().unwrap());
    unsafe { UpdateWindow(view.hwnd()) };
    pump_pending();
    assert_eq!(view.document_updates() - before, 1);
    assert_eq!(shown_line(view, 5).0, "x bb y");
}

#[test]
fn an_external_change_while_live_rerenders() {
    let (_scintilla, main, view) = live_tab("old\n");
    main.set_text("# New\n\n**fresh**\n");
    caret_to(&main, 0);
    assert_eq!(shown_line(view, 2).0, "fresh");
}

#[test]
fn front_matter_is_dimmed_and_not_a_heading() {
    let (_scintilla, main, view) = live_tab("---\ntitle: x\n---\n\n# H\n");
    caret_to(&main, 18);
    let (front, revealed, height) = shown_line(view, 1);
    assert_eq!(front, "title: x");
    assert!(!revealed);
    assert!(
        height < view.row_metrics().0 as f32 * 1.5,
        "not a heading: {height}"
    );
    assert_eq!(
        shown_line(view, 2).0,
        "---",
        "the closing line is not a setext underline"
    );
}

/// `lines` lines of filler after `head`, so the view can scroll.
fn with_filler(head: &str) -> String {
    let mut text = head.to_owned();
    for n in 0..200 {
        text.push_str(&format!("filler line {n}\n"));
    }
    text
}

fn painted(view: LiveView) {
    unsafe { UpdateWindow(view.hwnd()) };
    pump_pending();
    unsafe { UpdateWindow(view.hwnd()) };
}

#[test]
fn clicking_below_a_heading_keeps_the_caret_under_the_pointer() {
    let (_scintilla, main, view) = live_tab(&with_filler("# A big heading\n\nline a\nline b\n"));
    caret_to(&main, 3);
    painted(view);
    // Where line b is drawn while the heading is being edited.
    let line_b = main.editor().line_start(3).unwrap();
    let (x, y) = view.client_point_of(line_b + 2).unwrap();
    click(view.hwnd(), (x, y), false, false);
    painted(view);
    let caret = caret(&main).1;
    assert!(
        caret >= line_b && caret <= line_b + 6,
        "caret {caret} on line b"
    );
    // The heading keeps its size as it stops being edited: the caret is still under the
    // pointer.
    let (row, _) = view.row_metrics();
    let caret_y = view.caret_client_y().unwrap();
    assert!(
        (caret_y + row / 2 - y).abs() <= row / 2 + 1,
        "caret row top {caret_y}, clicked at {y}"
    );
}

#[test]
fn moving_onto_a_heading_keeps_the_caret_row_in_place() {
    let (_scintilla, main, view) = live_tab(&with_filler("intro\n# A big heading\nafter\n"));
    caret_to(&main, 2);
    painted(view);
    let heading_top = view.client_point_of(8).unwrap().1;
    send_inputs(view.hwnd(), &[Input::Key(VK_DOWN, false, false)]);
    painted(view);
    assert_eq!(view.revealed(), 1..2);
    let (row, _) = view.row_metrics();
    let caret_y = view.caret_client_y().unwrap();
    assert!(
        (caret_y - row).abs() <= 1,
        "the caret row starts where the heading did: {caret_y} vs {row} (heading middle {heading_top})"
    );
}

#[test]
fn click_on_rendered_bold_places_the_caret_inside_it() {
    let (_scintilla, main, view) = live_tab("top\n\naaa **bold** b\n");
    caret_to(&main, 0);
    painted(view);
    // Between `o` and `l`, as drawn.
    let (x, y) = view.client_point_of(5 + 8).unwrap();
    click(view.hwnd(), (x + 1, y), false, false);
    let line = main.editor().line_start(2).unwrap();
    let inside = caret(&main).1 - line;
    assert!(
        (6..=10).contains(&inside),
        "caret {inside} inside the bold text"
    );
    // The right half of `d`: before the closing markers, inside the bold text.
    caret_to(&main, 0);
    painted(view);
    let (d, _) = view.client_point_of(line + 9).unwrap();
    let (after, _) = view.client_point_of(line + 12).unwrap();
    click(view.hwnd(), (d + (after - d) * 3 / 4, y), false, false);
    assert_eq!(caret(&main).1 - line, 10);
}

#[test]
fn up_down_cross_rendered_lines_by_their_drawn_rows() {
    let text = "**alpha** **beta** **gamma**\nplain line here\n";
    let (_scintilla, main, view) = live_tab(text);
    caret_to(&main, 38);
    painted(view);
    send_inputs(view.hwnd(), &[Input::Key(VK_UP, false, false)]);
    let caret = caret(&main).1;
    for marker in [0..2, 7..11, 15..19, 24..26] {
        assert!(
            caret <= marker.start || caret >= marker.end,
            "caret {caret} inside the marker {marker:?}"
        );
    }
    assert!(caret < 28, "Up went to the first line");
}

#[test]
fn moving_the_caret_through_rendered_lines_never_dirties() {
    let (_scintilla, main, view) = live_tab("# H\n\n**b** and `c`\n\n[l](u)\n");
    let editor = main.editor();
    main.mark_clean();
    let could_undo = editor.can_undo().unwrap();
    let dirty = || main.with_app(|app| app.tabs.active().unwrap().dirty);
    send_inputs(
        view.hwnd(),
        &[
            Input::Key(VK_DOWN, false, false),
            Input::Key(VK_DOWN, false, false),
            Input::Key(VK_DOWN, false, false),
            Input::Key(VK_END, true, false),
            Input::Key(VK_HOME, true, false),
        ],
    );
    click(view.hwnd(), row_point(view, 2, 10), false, false);
    painted(view);
    assert_eq!(editor.can_undo().unwrap(), could_undo, "no undo step added");
    assert!(!dirty());
}

#[test]
fn inline_code_paints_its_background() {
    let (_scintilla, main, view) = live_tab("top\n\nsome `code` here\n");
    caret_to(&main, 0);
    painted(view);
    // A whole frame: a partial one (a caret blink) may not reach the code line.
    view.set_top_line(0);
    unsafe { UpdateWindow(view.hwnd()) };
    assert!(view.paint_stats().0 > 0, "a code background was drawn");
}

#[test]
fn recreating_the_target_rebinds_colours() {
    let (_scintilla, main, view) = live_tab("top\n\na [link](u)\n");
    caret_to(&main, 0);
    painted(view);
    let (_, generation) = view.paint_stats();
    assert_eq!(view.line_binding(2), generation);
    view.drop_target();
    painted(view);
    let (_, recreated) = view.paint_stats();
    assert!(recreated > generation);
    assert_eq!(view.line_binding(2), recreated);
}

#[test]
fn theme_change_recolours_rendered_lines() {
    let (_scintilla, main, view) = live_tab("top\n\na [link](u)\n");
    caret_to(&main, 0);
    // Explicit themes: the system one may already be either.
    let theme = |preference| {
        main.with_app(|app| app.settings.theme = preference);
        window::live_host::refresh_appearance(main.hwnd);
        painted(view);
        view.paint_stats().1
    };
    let light = theme(config::ThemePreference::Light);
    let dark = theme(config::ThemePreference::Dark);
    assert!(dark > light, "new brushes for the dark theme");
    assert_eq!(view.line_binding(2), dark);
}

#[test]
fn ctrl_click_on_a_rendered_link_opens_it() {
    let text = with_filler("# Target heading\n\nsee [jump](#target-heading) here\n\n");
    let (_scintilla, main, view) = live_tab(&text);
    let end = main.editor().length().unwrap();
    caret_to(&main, end);
    caret_to(&main, 36);
    caret_to(&main, end);
    painted(view);
    // Scroll back to the top so the link is on screen, rendered.
    view.set_top_line(0);
    painted(view);
    let link_text = text.find("jump").unwrap() + 1;
    let point = view.client_point_of(link_text).unwrap();
    click(view.hwnd(), point, true, false);
    pump_pending();
    assert_eq!(caret(&main), (0, 0), "the caret went to the heading");
}

#[test]
fn ctrl_click_on_a_rendered_wikilink_follows_it() {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetKeyboardState, SetKeyboardState, VK_CONTROL,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::WM_LBUTTONDOWN;
    let (_scintilla, main, view) = live_tab("top\n\nsee [[Other note|the other]] end\n");
    caret_to(&main, 0);
    painted(view);
    let (x, y) = view
        .client_point_of(5 + "see [[Other note|th".len())
        .unwrap();
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
    assert!(posted, "the wikilink was followed");
    let destination = unsafe { Box::from_raw(msg.lParam as *mut String) };
    assert!(destination.contains("Other note"), "{destination}");
    assert_eq!(caret(&main), (0, 0), "the click was taken");
}

#[test]
fn plain_click_on_a_link_places_the_caret() {
    let (_scintilla, main, view) = live_tab("top\n\nsee [jump](#nowhere) end\n");
    caret_to(&main, 0);
    painted(view);
    let point = view.client_point_of(5 + "see [ju".len()).unwrap();
    click(view.hwnd(), point, false, false);
    let line = main.editor().line_start(2).unwrap();
    let at = caret(&main).1 - line;
    assert!((5..=9).contains(&at), "caret {at} in the link text");
    assert!(main.notices().is_empty(), "{:?}", main.notices());
}

#[test]
fn the_cursor_is_a_hand_over_a_link_with_ctrl() {
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::Graphics::Gdi::ClientToScreen;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetKeyboardState, SetKeyboardState, VK_CONTROL,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetCursorPos, HTCLIENT, SetCursorPos, WM_SETCURSOR,
    };
    let (_scintilla, main, view) = live_tab("top\n\nsee [jump](u) and plain text\n");
    caret_to(&main, 0);
    painted(view);
    let mut original_pointer = POINT::default();
    unsafe { GetCursorPos(&mut original_pointer) };
    let hand_at = |position: usize, ctrl: bool| {
        let (x, y) = view.client_point_of(position).unwrap();
        let mut point = POINT { x: x + 2, y };
        unsafe {
            ClientToScreen(view.hwnd(), &mut point);
            SetCursorPos(point.x, point.y);
        }
        let mut keys = [0u8; 256];
        unsafe { GetKeyboardState(keys.as_mut_ptr()) };
        let original = keys;
        keys[usize::from(VK_CONTROL)] = if ctrl { 0x80 } else { 0 };
        unsafe {
            SetKeyboardState(keys.as_ptr());
            SendMessageW(
                view.hwnd(),
                WM_SETCURSOR,
                view.hwnd() as usize,
                HTCLIENT as isize,
            );
            SetKeyboardState(original.as_ptr());
        }
        view.hand_cursor()
    };
    let link = 5 + "see [ju".len();
    let plain = 5 + "see [jump](u) and pl".len();
    assert!(hand_at(link, true), "a hand over a link with Ctrl");
    assert!(!hand_at(link, false), "the I-beam without Ctrl");
    assert!(!hand_at(plain, true), "the I-beam off a link");
    unsafe { SetCursorPos(original_pointer.x, original_pointer.y) };
}

/// A whole frame from the top: a partial one (a caret blink) may not reach every line.
fn full_frame(view: LiveView) {
    view.set_top_line(0);
    unsafe { UpdateWindow(view.hwnd()) };
    pump_pending();
    unsafe { UpdateWindow(view.hwnd()) };
}

#[test]
fn a_bullet_list_draws_bullets_in_the_margin() {
    let (_scintilla, main, view) = live_tab("top\n\n- one\n- two\n");
    caret_to(&main, 0);
    full_frame(view);
    assert_eq!(view.painted_decorations().bullets, 2);
    let (_, text_left) = view.row_metrics();
    let (x, _) = view.client_point_of("top\n\n- ".len()).unwrap();
    assert!(
        x > text_left + 10,
        "the item's text starts at its indent: {x} vs {text_left}"
    );
}

#[test]
fn quote_bars_join_across_lines() {
    let (_scintilla, main, view) = live_tab("top\n\n> a\n> b\n");
    caret_to(&main, 0);
    full_frame(view);
    assert_eq!(view.painted_decorations().fills, 2);
    let (_, first, height) = view.debug_line(2).unwrap();
    assert!(!first);
    // Each bar spans its line's whole height, so the two meet.
    assert!(height > 0.0);
}

#[test]
fn a_rule_draws_a_line() {
    let (_scintilla, main, view) = live_tab("top\n\npara\n\n---\n\nend\n");
    caret_to(&main, 0);
    full_frame(view);
    assert_eq!(view.painted_decorations().lines, 1);
    assert_eq!(view.debug_line(4).unwrap().0, "");
}

#[test]
fn a_code_block_background_fills_the_width() {
    let (_scintilla, main, view) = live_tab("top\n\n```rust\ncode\n```\n");
    caret_to(&main, 0);
    full_frame(view);
    assert_eq!(view.painted_decorations().fills, 3);
    assert_eq!(view.debug_line(2).unwrap().0, "rust");
    assert_eq!(view.debug_line(3).unwrap().0, "code");
}

#[test]
fn a_table_draws_its_grid_and_lines_up_columns() {
    let text = "top\n\n| a | bbbbbb |\n|---|---|\n| cccccc | d |\n";
    let (_scintilla, main, view) = live_tab(text);
    caret_to(&main, 0);
    full_frame(view);
    let painted = view.painted_decorations();
    assert!(painted.lines >= 9, "{painted:?}");
    assert_eq!(painted.fills, 1, "the header's fill");
    let (b, _) = view.client_point_of(text.find("bbbbbb").unwrap()).unwrap();
    let (d, _) = view.client_point_of(text.find("| d").unwrap() + 2).unwrap();
    assert!((b - d).abs() <= 1, "second column at {b} and {d}");
}

/// A 32-bit BMP `width` by `height` pixels, all one colour, in the temp folder.
fn write_bmp(name: &str, width: u32, height: u32) -> std::path::PathBuf {
    let pixels = (width * height * 4) as usize;
    let mut bytes = Vec::with_capacity(54 + pixels);
    bytes.extend_from_slice(b"BM");
    bytes.extend_from_slice(&(54 + pixels as u32).to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&54u32.to_le_bytes());
    bytes.extend_from_slice(&40u32.to_le_bytes());
    bytes.extend_from_slice(&(width as i32).to_le_bytes());
    bytes.extend_from_slice(&(height as i32).to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&32u16.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&(pixels as u32).to_le_bytes());
    bytes.extend_from_slice(&2835u32.to_le_bytes());
    bytes.extend_from_slice(&2835u32.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    for _ in 0..width * height {
        bytes.extend_from_slice(&[40, 120, 200, 255]);
    }
    let path = std::env::temp_dir().join(format!("fastpad-live-{name}-{}.bmp", std::process::id()));
    std::fs::write(&path, bytes).unwrap();
    path
}

/// Waits until a frame has drawn an image, repainting as decodes land.
fn wait_for_image(view: LiveView) {
    pump_until("the image to be drawn", Duration::from_secs(10), || {
        full_frame(view);
        view.painted_decorations().images > 0
    });
}

#[test]
fn a_local_image_renders_as_a_thumbnail() {
    let image = write_bmp("thumbnail", 40, 120);
    let text = format!("top\n\n![dot]({})\n\nend\n", image.display());
    let (_scintilla, main, view) = live_tab(&text);
    caret_to(&main, 0);
    full_frame(view);
    wait_for_image(view);
    let (shown, revealed, height) = view.debug_line(2).unwrap();
    assert_eq!(shown, "");
    assert!(!revealed);
    assert!(height > 120.0, "the line is as tall as the image: {height}");
    let _ = std::fs::remove_file(image);
}

#[test]
fn a_missing_image_shows_its_alt_text() {
    let missing = std::env::temp_dir().join("fastpad-live-no-such-image.png");
    let text = format!("top\n\n![cat]({})\n", missing.display());
    let (_scintilla, main, view) = live_tab(&text);
    caret_to(&main, 0);
    full_frame(view);
    pump_pending();
    full_frame(view);
    assert_eq!(view.debug_line(2).unwrap().0, "cat");
    assert_eq!(view.painted_decorations().images, 0);
}

#[test]
fn recreating_the_target_redraws_images() {
    let image = write_bmp("recreate", 30, 30);
    let text = format!("top\n\n![dot]({})\n", image.display());
    let (_scintilla, main, view) = live_tab(&text);
    caret_to(&main, 0);
    wait_for_image(view);
    view.drop_target();
    full_frame(view);
    assert_eq!(view.painted_decorations().images, 1);
    let _ = std::fs::remove_file(image);
}

#[test]
fn clicking_a_checkbox_toggles_it_in_one_undo_step() {
    let (_scintilla, main, view) = live_tab("top\n\n- [ ] task\n");
    caret_to(&main, 1);
    full_frame(view);
    assert_eq!(view.painted_decorations().checkboxes, 1);
    let point = view.checkbox_point(2).unwrap();
    click(view.hwnd(), point, false, false);
    let editor = main.editor();
    assert_eq!(editor.text().unwrap(), "top\n\n- [x] task\n");
    assert_eq!(caret(&main), (1, 1), "the selection stays");
    click(view.hwnd(), view.checkbox_point(2).unwrap(), false, false);
    assert_eq!(editor.text().unwrap(), "top\n\n- [ ] task\n");
    editor.undo().unwrap();
    pump_pending();
    assert_eq!(editor.text().unwrap(), "top\n\n- [x] task\n");
    editor.undo().unwrap();
    pump_pending();
    assert_eq!(editor.text().unwrap(), "top\n\n- [ ] task\n");
}

#[test]
fn the_cursor_is_a_hand_over_a_checkbox() {
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::Graphics::Gdi::ClientToScreen;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetCursorPos, HTCLIENT, SetCursorPos, WM_SETCURSOR,
    };
    let (_scintilla, main, view) = live_tab("top\n\n- [ ] task\n");
    caret_to(&main, 0);
    full_frame(view);
    let mut original_pointer = POINT::default();
    unsafe { GetCursorPos(&mut original_pointer) };
    let hand_at = |(x, y): (i32, i32)| {
        let mut point = POINT { x, y };
        unsafe {
            ClientToScreen(view.hwnd(), &mut point);
            SetCursorPos(point.x, point.y);
            SendMessageW(
                view.hwnd(),
                WM_SETCURSOR,
                view.hwnd() as usize,
                HTCLIENT as isize,
            );
        }
        view.hand_cursor()
    };
    let checkbox = view.checkbox_point(2).unwrap();
    let task = view.client_point_of("top\n\n- [ ] ta".len()).unwrap();
    assert!(hand_at(checkbox), "a hand over the checkbox");
    assert!(!hand_at(task), "the I-beam over the task's text");
    unsafe { SetCursorPos(original_pointer.x, original_pointer.y) };
}

/// The line holding the caret.
fn caret_line(main: &TestMain) -> usize {
    let editor = main.editor();
    editor.line_from_position(caret(main).1).unwrap()
}

#[test]
fn up_down_keep_their_column_across_list_levels() {
    let text = "first plain line here\n\n- item line here\n\nlast plain line here\n";
    let (_scintilla, main, view) = live_tab(text);
    caret_to(&main, 14);
    painted(view);
    let goal = view.client_point_of(14).unwrap().0;
    send_inputs(view.hwnd(), &[Input::Key(VK_DOWN, false, false)]);
    painted(view);
    // The item line as drawn now (rendered, at its indent): the position nearest the goal.
    let item = text.find("- item").unwrap();
    let expected = (item + 2..item + "- item line here".len())
        .min_by_key(|&at| (view.client_point_of(at).unwrap().0 - goal).abs())
        .unwrap();
    send_inputs(view.hwnd(), &[Input::Key(VK_DOWN, false, false)]);
    assert_eq!(
        caret(&main).1,
        expected,
        "Down lands under the goal as drawn"
    );
    send_inputs(
        view.hwnd(),
        &[
            Input::Key(VK_DOWN, false, false),
            Input::Key(VK_DOWN, false, false),
        ],
    );
    painted(view);
    let (row, _) = view.row_metrics();
    let x = view.client_point_of(caret(&main).1).unwrap().0;
    assert!(
        (x - goal).abs() <= row / 2,
        "back on prose, the caret is under the goal: {x} vs {goal}"
    );
}

#[test]
fn moving_into_a_list_reveals_the_lines_between_blank_lines() {
    let text = "top\n\n- one\n- two\n\n- three\n";
    let (_scintilla, main, view) = live_tab(text);
    caret_to(&main, text.find("two").unwrap());
    assert_eq!(view.revealed(), 2..4);
    assert_eq!(shown_line(view, 2).0, "- one");
    assert_eq!(shown_line(view, 3).0, "- two");
    assert_eq!(shown_line(view, 5).0, "three");
}

#[test]
fn enter_in_a_rendered_list_continues_it() {
    let text = "top\n\n- one";
    let (_scintilla, main, view) = live_tab(text);
    caret_to(&main, text.len());
    send_inputs(
        view.hwnd(),
        &[Input::Key(VK_RETURN, false, false), Input::Text("two")],
    );
    assert_eq!(main.editor().text().unwrap(), "top\n\n- one\n- two");
    caret_to(&main, 0);
    assert_eq!(
        shown_line(view, 3).0,
        "two",
        "the new item renders once left"
    );
}

#[test]
fn typing_in_a_code_block_never_changes_its_height() {
    let text = "top\n\n```\ncode line\n```\n";
    let (_scintilla, main, view) = live_tab(text);
    caret_to(&main, 0);
    let (_, revealed, rendered) = shown_line(view, 3);
    assert!(!revealed);
    let inside = text.find("code line").unwrap() + 4;
    caret_to(&main, inside);
    let (_, revealed, source) = shown_line(view, 3);
    assert!(revealed);
    assert_eq!(source, rendered, "revealing a code line keeps its height");
    send_inputs(view.hwnd(), &[Input::Text("x = 1;")]);
    assert_eq!(shown_line(view, 3).2, rendered, "typing keeps it too");
}

#[test]
fn clicking_into_a_table_reveals_the_whole_table() {
    let text = "top\n\n| a | b |\n|---|---|\n| c | d |\n| e | f |\n";
    let (_scintilla, main, view) = live_tab(text);
    caret_to(&main, text.find("| c").unwrap() + 2);
    assert_eq!(view.revealed(), 2..6);
    for (line, source) in [
        (2, "| a | b |"),
        (3, "|---|---|"),
        (4, "| c | d |"),
        (5, "| e | f |"),
    ] {
        assert_eq!(shown_line(view, line).0, source);
    }
    caret_to(&main, 0);
    for line in [2, 4, 5] {
        assert!(
            shown_line(view, line).0.contains('\u{FFFC}'),
            "row {line} renders as a grid row once the caret leaves"
        );
    }
}

#[test]
fn leaving_a_table_row_formats_the_table_and_keeps_the_grid() {
    let text = "top\n\n| a | b |\n|---|---|\n| c | d |\n\nafter\n";
    let (_scintilla, main, view) = live_tab(text);
    caret_to(&main, text.find("c |").unwrap() + 1);
    send_inputs(
        view.hwnd(),
        &[
            Input::Text("cccc"),
            Input::Key(VK_DOWN, false, false),
            Input::Key(VK_DOWN, false, false),
        ],
    );
    let formatted = main.editor().text().unwrap();
    let widths = formatted
        .lines()
        .skip(2)
        .take(3)
        .map(str::len)
        .collect::<Vec<_>>();
    assert!(
        widths.iter().all(|&width| width == widths[0]),
        "{formatted:?}"
    );
    caret_to(&main, 0);
    painted(view);
    for line in [2, 4] {
        assert!(
            shown_line(view, line).0.contains('\u{FFFC}'),
            "line {line} is a grid row"
        );
    }
    let (b, _) = view
        .client_point_of(formatted.find(" b").unwrap() + 1)
        .unwrap();
    let (d, _) = view
        .client_point_of(formatted.find(" d").unwrap() + 1)
        .unwrap();
    assert!((b - d).abs() <= 1, "the second column lines up: {b} vs {d}");
}

#[test]
fn moving_onto_an_image_line_keeps_the_caret_row_in_place() {
    let image = write_bmp("anchor", 40, 120);
    let text = with_filler(&format!("intro\n\n![dot]({})\n\nafter\n", image.display()));
    let (_scintilla, main, view) = live_tab(&text);
    caret_to(&main, 2);
    wait_for_image(view);
    let (_, _, image_height) = view.debug_line(2).unwrap();
    assert!(image_height > 120.0);
    send_inputs(
        view.hwnd(),
        &[
            Input::Key(VK_DOWN, false, false),
            Input::Key(VK_DOWN, false, false),
        ],
    );
    painted(view);
    assert_eq!(view.revealed(), 2..3);
    let (row, _) = view.row_metrics();
    let caret_y = view.caret_client_y().unwrap();
    assert!(
        (caret_y - row * 2).abs() <= 1,
        "the caret row starts where the image line did: {caret_y} vs {}",
        row * 2
    );
    let _ = std::fs::remove_file(image);
}

#[test]
fn page_down_over_an_image_line_moves_by_a_page() {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_NEXT;
    let image = write_bmp("page", 40, 200);
    let text = with_filler(&format!("intro\n\n![dot]({})\n\n", image.display()));
    let (_scintilla, main, view) = live_tab(&text);
    caret_to(&main, 0);
    wait_for_image(view);
    send_inputs(view.hwnd(), &[Input::Key(VK_NEXT, false, false)]);
    painted(view);
    let line = caret_line(&main);
    assert!(line > 2, "past the image line: {line}");
    let (row, _) = view.row_metrics();
    let mut client = windows_sys::Win32::Foundation::RECT::default();
    unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect(view.hwnd(), &mut client) };
    let caret_y = view.caret_client_y().unwrap();
    assert!(
        caret_y >= 0 && caret_y + row <= client.bottom,
        "the caret stays on screen: {caret_y} in {}",
        client.bottom
    );
    // About a page of rows (the image line counts as its own height), not the whole note.
    let page_rows = (client.bottom / row) as usize;
    assert!(
        line <= page_rows + 3,
        "{line} lines down for a page of {page_rows} rows"
    );
    let _ = std::fs::remove_file(image);
}

#[test]
fn clicking_a_rule_places_the_caret_on_it() {
    let text = "top\n\npara\n\n---\n\nend\n";
    let (_scintilla, main, view) = live_tab(text);
    caret_to(&main, 0);
    painted(view);
    let rule = text.find("---").unwrap();
    let (_, y) = view.client_point_of(rule).unwrap();
    let (_, left) = view.row_metrics();
    click(view.hwnd(), (left + 200, y), false, false);
    assert_eq!(caret_line(&main), 4);
}

#[test]
fn revealing_blocks_never_dirties() {
    let image = write_bmp("dirty", 20, 20);
    let text = format!(
        "# Notes\n\n- one\n  - [ ] two\n1. three\n\n> quoted\n> - item\n\n```rs\nlet x;\n```\n\n| a | b |\n|---|---|\n| c | d |\n\n---\n\n![i]({})\n",
        image.display()
    );
    let (_scintilla, main, view) = live_tab(&text);
    let editor = main.editor();
    main.mark_clean();
    let could_undo = editor.can_undo().unwrap();
    let dirty = || main.with_app(|app| app.tabs.active().unwrap().dirty);
    let mut keys = vec![Input::Key(VK_DOWN, false, false); 22];
    keys.push(Input::Key(VK_END, true, false));
    keys.push(Input::Key(VK_HOME, true, false));
    send_inputs(view.hwnd(), &keys);
    for row in [2, 5, 9, 14, 18] {
        click(view.hwnd(), row_point(view, row, 30), false, false);
    }
    painted(view);
    assert_eq!(editor.text().unwrap(), text);
    assert_eq!(editor.can_undo().unwrap(), could_undo, "no undo step added");
    assert!(!dirty());
    let _ = std::fs::remove_file(image);
}

#[test]
fn a_relative_image_next_to_a_saved_note_renders() {
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let dir = std::env::temp_dir().join(format!("fastpad-live-images-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("img")).unwrap();
    let image = write_bmp("relative", 40, 120);
    std::fs::copy(&image, dir.join("img").join("pic.bmp")).unwrap();
    std::fs::write(
        dir.join("img").join("dot.png"),
        crate::preview::images::PNG_2X2,
    )
    .unwrap();
    let _ = std::fs::remove_file(image);
    std::fs::write(
        dir.join("note.md"),
        "# Note\n\ntext\n\n![pic](img/pic.bmp)\n\n![dot](img/dot.png)\n\nend\n",
    )
    .unwrap();
    let main = TestMain::new();
    window::open_path(main.hwnd, &dir.join("note.md")).unwrap();
    pump_until("note.md loaded", Duration::from_secs(3), || {
        !main.with_app(|app| app.populating_file)
    });
    main.with_app(|app| app.tabs.set_active_language(Language::Markdown));
    main.set_live(true);
    let view = main.live_view().unwrap();
    unsafe { SetFocus(view.hwnd()) };
    caret_to(&main, 0);
    wait_for_image(view);
    assert!(view.debug_line(4).unwrap().2 > 120.0);
    full_frame(view);
    pump_until("the PNG to be drawn too", Duration::from_secs(10), || {
        full_frame(view);
        view.painted_decorations().images == 2
    });
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_html_img_renders_at_its_width_as_in_a_readme() {
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let dir = std::env::temp_dir().join(format!("fastpad-live-html-img-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("docs")).unwrap();
    let image = write_bmp("html", 40, 120);
    std::fs::copy(&image, dir.join("docs").join("hero.bmp")).unwrap();
    let _ = std::fs::remove_file(image);
    std::fs::write(
        dir.join("README.md"),
        "<div align=\"center\">\n\n<img src=\"docs/hero.bmp\" alt=\"The hero\" width=\"80\">\n\n</div>\n\nA badge ![build](https://example.com/b.svg) here.\n",
    )
    .unwrap();
    let main = TestMain::new();
    window::open_path(main.hwnd, &dir.join("README.md")).unwrap();
    pump_until("README.md loaded", Duration::from_secs(3), || {
        !main.with_app(|app| app.populating_file)
    });
    main.with_app(|app| app.tabs.set_active_language(Language::Markdown));
    main.set_live(true);
    let view = main.live_view().unwrap();
    unsafe { SetFocus(view.hwnd()) };
    caret_to(&main, 0);
    wait_for_image(view);
    // 80 pixels wide: twice its natural width, so twice its natural height.
    let (shown, _, height) = view.debug_line(2).unwrap();
    assert_eq!(shown, "");
    assert!(height > 240.0, "{height}");
    // A remote image inside text shows its alt text.
    assert_eq!(view.debug_line(6).unwrap().0, "A badge build here.");
    let _ = std::fs::remove_dir_all(&dir);
}
