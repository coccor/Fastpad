//! The Live Markdown view takes the editor's place for keys, commands and focus (phase 1 plan,
//! Task 7): every check that asked "is the focus on the editor?" accepts the shown Live view.

use super::*;
use crate::document::Language;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{SetFocus, VK_UP};

/// A shown main window with a real editor holding `text` as Markdown, its tab in Live. Fields
/// drop in order: the editor and window before the Scintilla DLL.
struct Fixture {
    editor: crate::editor::Editor,
    live: HWND,
    window: ProductionWindow,
    _scintilla: crate::platform::OwnedModule,
}

fn fixture(text: &str) -> Fixture {
    let scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::ShowWindow(
            window.hwnd,
            windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOW,
        );
    }
    let editor = install_test_editor(&window);
    editor.populate_clean(text).unwrap();
    app_mut(window.hwnd)
        .tabs
        .set_active_language(Language::Markdown);
    unsafe { SetFocus(editor.hwnd()) };
    let group = app_mut(window.hwnd).tabs.active_group();
    crate::window::live_host::set_live(window.hwnd, group, true);
    let live = crate::window::live_host::view_hwnd(window.hwnd, group).expect("a shown Live view");
    Fixture {
        editor,
        live,
        window,
        _scintilla: scintilla,
    }
}

#[test]
fn the_live_view_takes_the_focus_and_is_the_content_target() {
    let f = fixture("hello");
    assert_eq!(focused(), f.live);
    assert_eq!(
        super::super::content_focus_target(f.window.hwnd),
        Some(f.live)
    );
    super::super::focus_content(f.window.hwnd);
    assert_eq!(focused(), f.live);
}

#[test]
fn ctrl_b_in_live_bolds_not_toggles_sidebar() {
    let f = fixture("hello world");
    f.editor.set_selection(2..2).unwrap();
    let key = translate_key_with(f.window.hwnd, f.live, b'B', true, false, false);
    assert_eq!(key, Some(CommandId::MarkdownBold));
    assert_eq!(f.editor.text().unwrap(), "**hello** world");
}

#[test]
fn alt_up_moves_lines_in_live() {
    let f = fixture("one\r\ntwo");
    f.editor.set_selection(6..6).unwrap();
    let key = translate_key_with(f.window.hwnd, f.live, VK_UP as u8, false, false, true);
    assert_eq!(key, Some(CommandId::MoveLinesUp));
    assert_eq!(f.editor.text().unwrap(), "two\r\none");
}

#[test]
fn escape_from_full_preview_returns_focus_to_live() {
    let f = fixture("# Title\n");
    execute_command(f.window.hwnd, CommandId::MarkdownPreviewFull);
    assert_ne!(focused(), f.live);
    let group = app_mut(f.window.hwnd).tabs.active_group();
    crate::window::preview_host::escape(f.window.hwnd, group);
    assert_eq!(focused(), f.live);
    execute_command(f.window.hwnd, CommandId::MarkdownPreviewClose);
    assert_eq!(focused(), f.live);
}
