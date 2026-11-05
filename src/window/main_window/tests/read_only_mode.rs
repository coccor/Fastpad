//! Read-only mode (issue #80): a tab open for viewing only.

use super::*;
use windows_sys::Win32::UI::WindowsAndMessaging::WM_CHAR;

fn active(hwnd: HWND) -> &'static crate::document::Document {
    app_mut(hwnd).tabs.active().unwrap()
}

#[test]
fn a_read_only_tab_takes_no_typing_never_saves_and_still_follows_its_file() {
    // Break caught: typing into a viewed file, Save writing over it, a read-only tab that stops
    // following changes made outside FastPad, or one that can't be made editable again.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("read-only");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    let path = open_note(&window, &scratch, "a.md", "one\n");

    execute_command(window.hwnd, CommandId::ToggleReadOnly);
    assert!(active(window.hwnd).read_only);
    assert!(editor.is_read_only().unwrap());
    unsafe { SendMessageW(editor.hwnd(), WM_CHAR, 'x' as usize, 0) };
    assert_eq!(editor.text().unwrap(), "one\n");
    assert!(!active(window.hwnd).dirty);

    execute_command(window.hwnd, CommandId::Save);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "one\n");
    assert!(
        notices(window.hwnd)
            .iter()
            .any(|notice| notice.contains("is read-only")),
        "{:?}",
        notices(window.hwnd)
    );

    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(&path, "one\ntwo, written elsewhere\n").unwrap();
    crate::window::library_host::check_all_tabs(window.hwnd);
    pump_posted_messages(window.hwnd);
    assert_eq!(editor.text().unwrap(), "one\ntwo, written elsewhere\n");
    assert!(
        editor.is_read_only().unwrap(),
        "a reload keeps it read-only"
    );

    execute_command(window.hwnd, CommandId::ToggleReadOnly);
    assert!(!active(window.hwnd).read_only);
    assert!(!editor.is_read_only().unwrap());
    unsafe { SendMessageW(editor.hwnd(), WM_CHAR, 'x' as usize, 0) };
    assert_ne!(editor.text().unwrap(), "one\ntwo, written elsewhere\n");
}

#[test]
fn a_tab_with_unsaved_edits_is_not_made_read_only() {
    // Break caught: edits locked into a tab that can then neither save them nor change.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("read-only-dirty");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    open_note(&window, &scratch, "a.md", "one\n");
    unsafe { SendMessageW(editor.hwnd(), WM_CHAR, 'x' as usize, 0) };
    assert!(active(window.hwnd).dirty);

    execute_command(window.hwnd, CommandId::ToggleReadOnly);
    assert!(!active(window.hwnd).read_only);
    assert!(!editor.is_read_only().unwrap());
    assert!(
        notices(window.hwnd)
            .iter()
            .any(|notice| notice.contains("unsaved changes"))
    );
}

#[test]
fn opening_read_only_marks_the_tab_and_the_status_bar_says_so() {
    // Break caught: `--read-only` (or its forwarded open) opening the file editable.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("read-only-open");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    let path = scratch.note("b.md", "viewed\n");
    super::super::open_path_read_only(window.hwnd, &path).unwrap();
    pump_posted_messages(window.hwnd);
    assert!(active(window.hwnd).read_only);
    assert!(editor.is_read_only().unwrap());
    // A notice takes the left of the bar; the label shows once none is up.
    app_mut(window.hwnd).notifications.dismiss_all();
    let bar = super::super::current_status_bar(window.hwnd).unwrap();
    assert!(bar.left.ends_with("Read-only"), "{bar:?}");
}
