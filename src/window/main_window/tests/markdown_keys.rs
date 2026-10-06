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
