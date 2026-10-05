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

const RUST: crate::editor::comment::CommentSyntax =
    crate::document::Language::Rust.comment_syntax();

#[test]
fn toggle_line_comment_comments_every_selections_lines_in_one_step() {
    let f = fixture("fn a() {}\n    x\ny\nz");
    f.editor.set_selection(0..0).unwrap();
    f.editor.add_selection_for_test(18); // "z"
    f.editor.toggle_line_comment(RUST).unwrap();
    assert_eq!(f.editor.text().unwrap(), "// fn a() {}\n    x\ny\n// z");
    assert_one_undo_restores(&f.editor, "fn a() {}\n    x\ny\nz");
}

#[test]
fn toggle_line_comment_round_trips_and_the_caret_follows_its_text() {
    let f = fixture("    foo\nbar");
    f.editor.set_selection(7..7).unwrap(); // end of "    foo"
    f.editor.toggle_line_comment(RUST).unwrap();
    assert_eq!(f.editor.text().unwrap(), "    // foo\nbar");
    assert_eq!(f.editor.selection().unwrap(), 10..10);
    f.editor.toggle_line_comment(RUST).unwrap();
    assert_eq!(f.editor.text().unwrap(), "    foo\nbar");
}

#[test]
fn toggle_block_comment_wraps_and_unwraps_the_main_selection() {
    let f = fixture("let a = b + c;");
    f.editor.set_selection(8..13).unwrap();
    f.editor.toggle_block_comment(RUST).unwrap();
    assert_eq!(f.editor.text().unwrap(), "let a = /* b + c */;");
    assert_eq!(f.editor.selection().unwrap(), 8..19);
    f.editor.toggle_block_comment(RUST).unwrap();
    assert_eq!(f.editor.text().unwrap(), "let a = b + c;");
    drop(f);
    let empty = fixture("x");
    empty.editor.set_selection(1..1).unwrap();
    empty.editor.toggle_block_comment(RUST).unwrap();
    assert_eq!(empty.editor.text().unwrap(), "x/*  */");
    assert_eq!(empty.editor.selection().unwrap(), 4..4);
}

#[test]
fn comment_toggles_do_nothing_without_markers() {
    let none = crate::document::Language::PlainText.comment_syntax();
    let f = fixture("a");
    f.editor.toggle_line_comment(none).unwrap();
    f.editor.toggle_block_comment(none).unwrap();
    assert_eq!(f.editor.text().unwrap(), "a");
    assert!(!f.editor.can_undo().unwrap());
}

#[test]
fn add_next_occurrence_selects_the_word_then_adds_matches() {
    let f = fixture("foo bar foo");
    f.editor.set_selection(1..1).unwrap();
    f.editor.add_next_occurrence().unwrap();
    assert_eq!(f.editor.selections().unwrap(), vec![0..3]);
    f.editor.add_next_occurrence().unwrap();
    assert_eq!(f.editor.selections().unwrap(), vec![0..3, 8..11]);
}

#[test]
fn add_next_occurrence_from_a_caret_matches_whole_words_only() {
    let f = fixture("foo food foo");
    f.editor.set_selection(0..0).unwrap();
    f.editor.add_next_occurrence().unwrap(); // the word: 0..3
    f.editor.add_next_occurrence().unwrap(); // skips "food"
    assert_eq!(f.editor.selections().unwrap(), vec![0..3, 9..12]);
}

#[test]
fn add_next_occurrence_from_a_selection_matches_substrings_but_not_case() {
    let f = fixture("foo food Foo");
    f.editor.set_selection(0..3).unwrap();
    f.editor.add_next_occurrence().unwrap();
    f.editor.add_next_occurrence().unwrap();
    // Scintilla makes the newest selection main and may reorder; compare as a set.
    let mut found = f.editor.selections().unwrap();
    found.sort_by_key(|range| range.start);
    assert_eq!(found, vec![0..3, 4..7]);
}

#[test]
fn select_all_occurrences_selects_every_match() {
    let f = fixture("ab x ab y ab");
    f.editor.set_selection(0..2).unwrap();
    f.editor.select_all_occurrences().unwrap();
    let mut found = f.editor.selections().unwrap();
    found.sort_by_key(|range| range.start);
    assert_eq!(found, vec![0..2, 5..7, 10..12]);
}

#[test]
fn add_cursor_above_and_below_keep_the_column_and_clamp() {
    let f = fixture("abcd\nab\nabcd");
    f.editor.set_selection(7..7).unwrap(); // line 1, column 2 (end of "ab")
    f.editor.add_cursor(true).unwrap();
    f.editor.add_cursor(false).unwrap();
    let mut carets = f.editor.carets().unwrap();
    carets.sort_unstable();
    assert_eq!(carets, vec![2, 7, 10]);
    f.editor.add_cursor(true).unwrap(); // topmost is line 0: nothing above
    assert_eq!(f.editor.carets().unwrap().len(), 3);
}

#[test]
fn select_all_occurrences_from_a_caret_selects_every_whole_word_match() {
    // Break caught: Scintilla only selecting the word at an empty caret, so Ctrl+Shift+L needed
    // a second press (VS Code selects every match at once).
    let f = fixture("ab abc ab");
    f.editor.set_selection(0..0).unwrap();
    f.editor.select_all_occurrences().unwrap();
    let mut found = f.editor.selections().unwrap();
    found.sort_by_key(|range| range.start);
    assert_eq!(found, vec![0..2, 7..9]);
}

#[test]
fn editing_commands_have_their_spec_keys() {
    use crate::window::keymap::KeyStroke;
    let keymap = crate::window::keymap::Keymap::defaults();
    for (text, command) in [
        ("Alt+Up", CommandId::MoveLinesUp),
        ("Alt+Down", CommandId::MoveLinesDown),
        ("Shift+Alt+Up", CommandId::CopyLinesUp),
        ("Shift+Alt+Down", CommandId::CopyLinesDown),
        ("Ctrl+Shift+K", CommandId::DeleteLines),
        ("Ctrl+Enter", CommandId::InsertLineBelow),
        ("Ctrl+Shift+Enter", CommandId::InsertLineAbove),
        ("Ctrl+]", CommandId::IndentLines),
        ("Ctrl+[", CommandId::OutdentLines),
        ("Ctrl+L", CommandId::ExpandLineSelection),
        ("Ctrl+/", CommandId::ToggleLineComment),
        ("Shift+Alt+A", CommandId::ToggleBlockComment),
        ("Ctrl+D", CommandId::AddNextOccurrence),
        ("Ctrl+Shift+L", CommandId::SelectAllOccurrences),
        ("Ctrl+Alt+Up", CommandId::AddCursorAbove),
        ("Ctrl+Alt+Down", CommandId::AddCursorBelow),
    ] {
        assert_eq!(
            keymap.command_for(KeyStroke::parse(text).unwrap()),
            Some(command),
            "{text}"
        );
        assert!(command.is_editing() && command.needs_text(), "{command:?}");
    }
}

#[test]
fn toggle_line_comment_uses_the_active_tabs_language() {
    let f = fixture("a");
    app_mut(f.window.hwnd).tabs.active_mut().unwrap().language = Language::Python;
    execute_command(f.window.hwnd, CommandId::ToggleLineComment);
    assert_eq!(f.editor.text().unwrap(), "# a");
}

#[test]
fn line_commands_run_on_the_active_editor() {
    let f = fixture("a\r\nb");
    f.editor.set_selection(0..0).unwrap();
    execute_command(f.window.hwnd, CommandId::MoveLinesDown);
    assert_eq!(f.editor.text().unwrap(), "b\r\na");
    execute_command(f.window.hwnd, CommandId::DeleteLines);
    assert_eq!(f.editor.text().unwrap(), "b");
}
