//! Markdown-scoped keys (live mode spec §9), through the real accelerator path.

use super::*;
use crate::document::Language;

/// A main window with a real editor holding `text` in `language`. Fields drop in order: the
/// editor and window before the Scintilla DLL.
struct Fixture {
    editor: crate::editor::Editor,
    window: ProductionWindow,
    _scintilla: crate::platform::OwnedModule,
}

fn fixture(text: &str, language: Language) -> Fixture {
    let scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    editor.populate_clean(text).unwrap();
    app_mut(window.hwnd).tabs.set_active_language(language);
    Fixture {
        editor,
        window,
        _scintilla: scintilla,
    }
}

#[test]
fn ctrl_b_is_bold_in_markdown_and_toggle_sidebar_elsewhere() {
    // Break caught: Ctrl+B bolding in a .txt file, or toggling the sidebar in a Markdown file.
    let f = fixture("hello", Language::Markdown);
    let key = translate_key_with(f.window.hwnd, f.editor.hwnd(), b'B', true, false, false);
    assert_eq!(key, Some(CommandId::MarkdownBold));
    // One main window at a time: its class is registered while it lives.
    drop(f);
    let plain = fixture("hello", Language::PlainText);
    let key = translate_key_with(
        plain.window.hwnd,
        plain.editor.hwnd(),
        b'B',
        true,
        false,
        false,
    );
    assert_eq!(key, Some(CommandId::ToggleSidebar));
}

#[test]
fn markdown_keys_off_the_editor_fall_through_to_global() {
    let f = fixture("hello", Language::Markdown);
    let key = translate_key_with(f.window.hwnd, f.window.hwnd, b'B', true, false, false);
    assert_ne!(key, Some(CommandId::MarkdownBold));
}

#[test]
fn ctrl_alt_v_toggles_live_markdown() {
    let f = fixture("hello", Language::Markdown);
    let key = translate_key_with(f.window.hwnd, f.editor.hwnd(), b'V', true, false, true);
    assert_eq!(key, Some(CommandId::MarkdownToggleLive));
}

/// One Undo restores `original`.
fn assert_one_undo_restores(editor: &crate::editor::Editor, original: &str) {
    editor.undo().unwrap();
    assert_eq!(editor.text().unwrap(), original);
}

#[test]
fn ctrl_b_bolds_the_word_under_the_caret_in_one_undo_step() {
    let f = fixture("hello world", Language::Markdown);
    f.editor.set_selection(2..2).unwrap();
    translate_key_with(f.window.hwnd, f.editor.hwnd(), b'B', true, false, false);
    assert_eq!(f.editor.text().unwrap(), "**hello** world");
    assert_eq!(f.editor.selection().unwrap(), 4..4);
    assert_one_undo_restores(&f.editor, "hello world");
}

#[test]
fn ctrl_i_ctrl_backtick_and_ctrl_k_wrap_the_selection() {
    let f = fixture("a b", Language::Markdown);
    f.editor.set_selection(2..3).unwrap();
    translate_key_with(f.window.hwnd, f.editor.hwnd(), b'I', true, false, false);
    assert_eq!(f.editor.text().unwrap(), "a *b*");
    f.editor.set_selection(0..1).unwrap();
    translate_key_with(f.window.hwnd, f.editor.hwnd(), 0xC0, true, false, false); // VK_OEM_3
    assert_eq!(f.editor.text().unwrap(), "`a` *b*");
    f.editor.set_selection(0..3).unwrap();
    translate_key_with(f.window.hwnd, f.editor.hwnd(), b'K', true, false, false);
    assert_eq!(f.editor.text().unwrap(), "[`a`]() *b*");
    assert_eq!(f.editor.selection().unwrap(), 6..6);
}

#[test]
fn ctrl_b_in_a_text_file_leaves_the_text_alone() {
    let f = fixture("hello", Language::PlainText);
    f.editor.set_selection(2..2).unwrap();
    translate_key_with(f.window.hwnd, f.editor.hwnd(), b'B', true, false, false);
    assert_eq!(f.editor.text().unwrap(), "hello");
}

#[test]
fn format_is_one_undo_step_with_crlf_and_multibyte_text() {
    let f = fixture("é one\r\nü two", Language::Markdown);
    f.editor.set_selections(&[0..2, 8..10]).unwrap();
    translate_key_with(f.window.hwnd, f.editor.hwnd(), b'B', true, false, false);
    assert_eq!(f.editor.text().unwrap(), "**é** one\r\n**ü** two");
    assert_one_undo_restores(&f.editor, "é one\r\nü two");
}

use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_RETURN, VK_TAB};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, MSG, PM_REMOVE, PeekMessageW, TranslateMessage, WM_KEYDOWN,
};

/// Presses `key` as the message loop delivers it: accelerators first, then TranslateMessage
/// and dispatch, then the WM_CHAR that produced, then anything posted meanwhile.
fn press(f: &Fixture, key: u16, shift: bool) {
    let identity = unsafe { super::super::window_identity(f.window.hwnd).unwrap() };
    let message = MSG {
        hwnd: f.editor.hwnd(),
        message: WM_KEYDOWN,
        wParam: usize::from(key),
        ..Default::default()
    };
    with_shift(shift, || unsafe {
        if !super::super::translate_accelerator(f.window.hwnd, &identity, &message) {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        drain_messages();
    });
}

fn with_shift(shift: bool, run: impl FnOnce()) {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetKeyboardState, SetKeyboardState, VK_SHIFT,
    };
    let mut keys = [0u8; 256];
    unsafe { GetKeyboardState(keys.as_mut_ptr()) };
    let original = keys;
    keys[VK_SHIFT as usize] = if shift { 0x80 } else { 0 };
    unsafe { SetKeyboardState(keys.as_ptr()) };
    run();
    unsafe { SetKeyboardState(original.as_ptr()) };
}

fn drain_messages() {
    let mut message = MSG::default();
    while unsafe { PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_REMOVE) } != 0 {
        unsafe {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

#[test]
fn enter_continues_a_list_through_the_real_key_path() {
    let f = fixture("- a\r\nx", Language::Markdown);
    f.editor.set_selection(3..3).unwrap();
    press(&f, VK_RETURN, false);
    assert_eq!(f.editor.text().unwrap(), "- a\r\n- \r\nx");
    assert_eq!(f.editor.selection().unwrap(), 7..7);
}

#[test]
fn enter_continues_a_list_with_the_documents_line_ending() {
    // Break caught: an LF document getting CRLF continuations, or a document with no line
    // ending yet not using the editor's end-of-line mode (CRLF by default).
    let f = fixture("- a\nx", Language::Markdown);
    f.editor.set_selection(3..3).unwrap();
    press(&f, VK_RETURN, false);
    assert_eq!(f.editor.text().unwrap(), "- a\n- \nx");
    assert_eq!(f.editor.selection().unwrap(), 6..6);
    drop(f);
    let f = fixture("1. a", Language::Markdown);
    f.editor.set_selection(4..4).unwrap();
    press(&f, VK_RETURN, false);
    assert_eq!(f.editor.text().unwrap(), "1. a\r\n2. ");
    assert_eq!(f.editor.selection().unwrap(), 9..9);
}

#[test]
fn enter_on_an_empty_item_ends_the_list() {
    let f = fixture("- a\r\n- ", Language::Markdown);
    f.editor.set_selection(7..7).unwrap();
    press(&f, VK_RETURN, false);
    assert_eq!(f.editor.text().unwrap(), "- a\r\n");
}

#[test]
fn enter_outside_lists_and_in_text_files_is_a_plain_newline() {
    let f = fixture("ab", Language::Markdown);
    f.editor.set_selection(1..1).unwrap();
    press(&f, VK_RETURN, false);
    assert_eq!(f.editor.text().unwrap(), "a\r\nb");
    drop(f);
    let plain = fixture("- a", Language::PlainText);
    plain.editor.set_selection(3..3).unwrap();
    press(&plain, VK_RETURN, false);
    assert_eq!(plain.editor.text().unwrap(), "- a\r\n");
}

#[test]
fn enter_and_tab_inside_fenced_code_and_front_matter_keep_their_default() {
    // Break caught: a `- x` line inside a code block continuing as a list item.
    let f = fixture("```\r\n- x\r\n```", Language::Markdown);
    f.editor.set_selection(8..8).unwrap();
    press(&f, VK_RETURN, false);
    assert_eq!(f.editor.text().unwrap(), "```\r\n- x\r\n\r\n```");
    drop(f);
    let f = fixture("---\r\n- x\r\n---\r\n", Language::Markdown);
    f.editor.set_selection(8..8).unwrap();
    press(&f, VK_TAB, false);
    assert_eq!(f.editor.text().unwrap(), "---\r\n- x\t\r\n---\r\n");
}

#[test]
fn tab_nests_a_list_item_and_shift_tab_un_nests_it() {
    let f = fixture("- a\r\n- b", Language::Markdown);
    f.editor.set_selection(8..8).unwrap();
    press(&f, VK_TAB, false);
    assert_eq!(f.editor.text().unwrap(), "- a\r\n  - b");
    press(&f, VK_TAB, true);
    assert_eq!(f.editor.text().unwrap(), "- a\r\n- b");
}

#[test]
fn tab_on_a_first_item_is_consumed_without_an_edit() {
    // Break caught: a tab character inserted into an item CommonMark cannot nest.
    let f = fixture("- a", Language::Markdown);
    f.editor.set_selection(3..3).unwrap();
    press(&f, VK_TAB, false);
    assert_eq!(f.editor.text().unwrap(), "- a");
}

#[test]
fn tab_with_a_selection_on_one_list_line_nests_the_item() {
    // Spec §8.2: a selection confined to one list item's line nests it, like a caret.
    let f = fixture("- a\r\n- bcd", Language::Markdown);
    f.editor.set_selection(7..9).unwrap();
    press(&f, VK_TAB, false);
    assert_eq!(f.editor.text().unwrap(), "- a\r\n  - bcd");
}

#[test]
fn tab_in_a_table_selects_the_next_cell() {
    let text = "| a | b |\r\n| - | - |\r\n| c | d |";
    let f = fixture(text, Language::Markdown);
    let b = text.find('b').unwrap();
    f.editor.set_selection(2..2).unwrap();
    press(&f, VK_TAB, false);
    assert_eq!(f.editor.selection().unwrap(), b..b + 1);
    assert_eq!(f.editor.text().unwrap(), text);
}

#[test]
fn a_table_is_formatted_when_the_caret_leaves_it_in_one_undo_step() {
    let f = fixture("|a|b|\r\n|-|-|\r\n|c|d|\r\n\r\nx", Language::Markdown);
    f.editor.set_selection(1..1).unwrap();
    f.editor.replace_target(1..1, "z").unwrap(); // an edit inside the table
    drain_messages();
    let edited = f.editor.text().unwrap();
    let x = edited.len() - 1;
    f.editor.set_selection(x..x).unwrap();
    drain_messages();
    assert_eq!(
        f.editor.text().unwrap(),
        "| za  | b   |\r\n| --- | --- |\r\n| c   | d   |\r\n\r\nx"
    );
    f.editor.undo().unwrap();
    assert_eq!(f.editor.text().unwrap(), edited);
}

#[test]
fn an_untouched_table_is_not_rewritten_when_the_caret_passes_through() {
    let text = "|a|b|\r\n|-|-|\r\n\r\nx";
    let f = fixture(text, Language::Markdown);
    f.editor.set_selection(1..1).unwrap();
    drain_messages();
    f.editor.set_selection(text.len()..text.len()).unwrap();
    drain_messages();
    assert_eq!(f.editor.text().unwrap(), text);
    assert!(!f.editor.can_undo().unwrap());
}
