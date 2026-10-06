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
