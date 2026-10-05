//! Editing shortcuts (editing shortcuts spec): line operations, comments, multiple carets and
//! their keys, on a real Scintilla.

use super::*;
#[allow(unused_imports)]
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyboardState, SetKeyboardState, VK_CONTROL, VK_ESCAPE, VK_MENU, VK_SHIFT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::WM_KEYDOWN;

/// A main window with a real editor holding `text`. Fields drop in order: the editor and window
/// before the Scintilla DLL.
struct Fixture {
    editor: crate::editor::Editor,
    window: ProductionWindow,
    _scintilla: crate::platform::OwnedModule,
}

fn fixture(text: &str) -> Fixture {
    let scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    // Four-space indentation, whatever Scintilla's defaults (tabs, width 8).
    unsafe {
        SendMessageW(
            editor.hwnd(),
            crate::editor::scintilla_constants::SCI_SETTABWIDTH,
            4,
            0,
        );
        SendMessageW(
            editor.hwnd(),
            crate::editor::scintilla_constants::SCI_SETUSETABS,
            0,
            0,
        );
    }
    editor.populate_clean(text).unwrap();
    Fixture {
        editor,
        window,
        _scintilla: scintilla,
    }
}

/// Holds `keys` down in this thread's keyboard state while `run` runs.
fn with_keys_down<R>(keys: &[u16], run: impl FnOnce() -> R) -> R {
    let mut state = [0u8; 256];
    unsafe { GetKeyboardState(state.as_mut_ptr()) };
    let original = state;
    for key in keys {
        state[usize::from(*key)] = 0x80;
    }
    unsafe { SetKeyboardState(state.as_ptr()) };
    let result = run();
    unsafe { SetKeyboardState(original.as_ptr()) };
    result
}

/// One Undo restores `original` and leaves nothing more to undo.
fn assert_one_undo_restores(editor: &crate::editor::Editor, original: &str) {
    editor.undo().unwrap();
    assert_eq!(editor.text().unwrap(), original);
    assert!(!editor.can_undo().unwrap(), "more than one undo step");
}

#[test]
fn copy_with_an_empty_selection_copies_the_line_and_paste_puts_it_above() {
    // Break caught: Copy with nothing selected copying nothing, so Paste does nothing (VS Code
    // copies the whole line and pastes it as a line).
    let f = fixture("one\r\ntwo\r\nthree");
    f.editor.set_selection(6..6).unwrap();
    f.editor.copy().unwrap();
    f.editor.set_selection(1..1).unwrap();
    f.editor.paste().unwrap();
    assert_eq!(f.editor.text().unwrap(), "two\r\none\r\ntwo\r\nthree");
}

#[test]
fn cut_with_an_empty_selection_cuts_the_line() {
    let f = fixture("one\r\ntwo\r\nthree");
    f.editor.set_selection(6..6).unwrap();
    f.editor.cut().unwrap();
    assert_eq!(f.editor.text().unwrap(), "one\r\nthree");
}

#[test]
fn every_caret_types() {
    // Break caught: multiple selection off, or additional selections not typing, so a second
    // caret is ignored.
    let f = fixture("ab\r\ncd");
    f.editor.set_selection(0..0).unwrap();
    f.editor.add_selection_for_test(4);
    unsafe {
        SendMessageW(
            f.editor.hwnd(),
            windows_sys::Win32::UI::WindowsAndMessaging::WM_CHAR,
            usize::from(b'x'),
            0,
        )
    };
    assert_eq!(f.editor.text().unwrap(), "xab\r\nxcd");
}

#[test]
fn escape_in_the_editor_drops_the_extra_carets() {
    let f = fixture("ab\r\ncd");
    f.editor.set_selection(0..0).unwrap();
    f.editor.add_selection_for_test(4);
    assert_eq!(f.editor.carets().unwrap().len(), 2);
    unsafe { SendMessageW(f.editor.hwnd(), WM_KEYDOWN, usize::from(VK_ESCAPE), 0) };
    assert_eq!(f.editor.carets().unwrap().len(), 1);
}

#[test]
fn scintillas_own_line_keys_are_cleared() {
    // Break caught: Ctrl+D / Ctrl+L / Ctrl+Shift+L reaching Scintilla (a rebound key, or focus
    // in the editor with the accelerator unbound) duplicating, cutting or deleting a line.
    let f = fixture("one\r\ntwo");
    f.editor.set_selection(1..1).unwrap();
    for (keys, letter) in [
        (&[VK_CONTROL][..], b'D'),
        (&[VK_CONTROL][..], b'L'),
        (&[VK_CONTROL, VK_SHIFT][..], b'L'),
        (&[VK_CONTROL][..], b'T'),
    ] {
        with_keys_down(keys, || unsafe {
            SendMessageW(f.editor.hwnd(), WM_KEYDOWN, usize::from(letter), 0)
        });
    }
    assert_eq!(f.editor.text().unwrap(), "one\r\ntwo");
}
