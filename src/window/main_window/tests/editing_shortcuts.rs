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

#[test]
fn move_lines_moves_the_touched_lines_in_one_undo_step() {
    // Scintilla ends a line moved to the end with the document's EOL mode (CRLF here).
    let f = fixture("a\r\nb\r\nc");
    f.editor.set_selection(3..4).unwrap(); // "b"
    f.editor.move_lines(true).unwrap();
    assert_eq!(f.editor.text().unwrap(), "b\r\na\r\nc");
    f.editor.move_lines(false).unwrap();
    f.editor.move_lines(false).unwrap();
    assert_eq!(f.editor.text().unwrap(), "a\r\nc\r\nb");
    f.editor.undo().unwrap();
    f.editor.undo().unwrap();
    assert_one_undo_restores(&f.editor, "a\r\nb\r\nc");
}

#[test]
fn copy_lines_up_keeps_the_selection_on_the_upper_copy() {
    let f = fixture("a\nbc\nd");
    f.editor.set_selection(3..4).unwrap(); // "c"
    f.editor.copy_lines(false).unwrap();
    assert_eq!(f.editor.text().unwrap(), "a\nbc\nbc\nd");
    assert_eq!(f.editor.selection().unwrap(), 3..4);
    assert_one_undo_restores(&f.editor, "a\nbc\nd");
}

#[test]
fn copy_lines_down_moves_the_selection_to_the_lower_copy() {
    let f = fixture("a\nbc\nd");
    f.editor.set_selection(3..4).unwrap();
    f.editor.copy_lines(true).unwrap();
    assert_eq!(f.editor.text().unwrap(), "a\nbc\nbc\nd");
    assert_eq!(f.editor.selection().unwrap(), 6..7);
}

#[test]
fn copy_lines_down_on_the_last_line_adds_a_line_end() {
    let f = fixture("a\r\nb");
    f.editor.set_selection(4..4).unwrap();
    f.editor.copy_lines(true).unwrap();
    assert_eq!(f.editor.text().unwrap(), "a\r\nb\r\nb");
}

#[test]
fn delete_lines_removes_every_touched_line_and_keeps_the_column() {
    let f = fixture("aa\nbb\ncc\ndd\nee");
    f.editor.set_selection(4..4).unwrap(); // line 1, column 1
    f.editor.add_selection_for_test(10); // line 3
    f.editor.delete_lines().unwrap();
    assert_eq!(f.editor.text().unwrap(), "aa\ncc\nee");
    // The added caret is the main one: its line 3 ("dd") becomes line 2 ("ee"), column 1.
    assert_eq!(f.editor.carets().unwrap(), vec![7]);
    assert_one_undo_restores(&f.editor, "aa\nbb\ncc\ndd\nee");
}

#[test]
fn delete_lines_on_the_last_line_removes_the_preceding_line_end() {
    let f = fixture("a\nb");
    f.editor.set_selection(2..2).unwrap();
    f.editor.delete_lines().unwrap();
    assert_eq!(f.editor.text().unwrap(), "a");
    drop(f);
    let only = fixture("solo");
    only.editor.delete_lines().unwrap();
    assert_eq!(only.editor.text().unwrap(), "");
}

#[test]
fn insert_line_below_and_above_keep_the_indentation() {
    let f = fixture("  a\nb");
    f.editor.set_selection(1..1).unwrap();
    f.editor.insert_line(true).unwrap();
    assert_eq!(f.editor.text().unwrap(), "  a\n  \nb");
    assert_eq!(f.editor.selection().unwrap(), 6..6);
    f.editor.set_selection(2..2).unwrap();
    f.editor.insert_line(false).unwrap();
    assert_eq!(f.editor.text().unwrap(), "  \n  a\n  \nb");
    assert_eq!(f.editor.selection().unwrap(), 2..2);
}

#[test]
fn indent_and_outdent_move_lines_by_one_level_whatever_the_selection() {
    let f = fixture("a\n\n   b");
    f.editor.set_selection(0..6).unwrap(); // touches all three lines
    f.editor.indent_lines(false).unwrap();
    assert_eq!(f.editor.text().unwrap(), "    a\n\n    b"); // blank line left alone; 3 → 4
    assert_one_undo_restores(&f.editor, "a\n\n   b");
    f.editor.set_selection(0..6).unwrap(); // Undo moved the selection
    f.editor.indent_lines(true).unwrap();
    assert_eq!(f.editor.text().unwrap(), "a\n\nb"); // 3 → 0, 0 stays 0
}

#[test]
fn select_line_selects_the_line_then_extends_it() {
    let f = fixture("ab\ncd\nef");
    f.editor.set_selection(4..4).unwrap();
    f.editor.expand_line_selection().unwrap();
    assert_eq!(f.editor.selection().unwrap(), 3..6);
    f.editor.expand_line_selection().unwrap();
    assert_eq!(f.editor.selection().unwrap(), 3..8);
    f.editor.expand_line_selection().unwrap();
    assert_eq!(f.editor.selection().unwrap(), 3..8);
}
