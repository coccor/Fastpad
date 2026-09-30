//! The command palette, pickers, quick open and closing tabs from the keyboard and mouse.

use super::*;

#[test]
fn the_command_palette_filters_as_typed_and_runs_the_selection_on_enter() {
    // Break caught: a palette whose field never refilters the list, or whose Enter leaves the
    // palette open or runs nothing.
    use crate::editor::scintilla_constants::SCI_GETZOOM;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_DOWN, VK_ESCAPE, VK_RETURN};
    use windows_sys::Win32::UI::WindowsAndMessaging::{SetWindowTextW, WM_KEYDOWN};
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    let palette = |hwnd| app_mut(hwnd).command_palette.as_ref().unwrap();

    // The test window itself is never shown, so check the panel's own style bit.
    let panel_visible = |hwnd| {
        (unsafe { GetWindowLongPtrW(palette(hwnd).panel_hwnd(), super::super::GWL_STYLE) }) as u32
            & super::super::WS_VISIBLE
            != 0
    };
    execute_command(window.hwnd, CommandId::CommandPalette);
    assert!(palette(window.hwnd).is_visible());
    assert!(panel_visible(window.hwnd));
    // Markdown preview commands are listed only while the active tab is Markdown, and New
    // note and New folder only while a notebook is open (this window has none).
    assert_eq!(
        palette(window.hwnd).shown().len(),
        crate::window::command_palette::ENTRIES
            .iter()
            .filter(|entry| !entry.command.is_markdown_preview())
            .filter(|entry| !matches!(entry.command, CommandId::NoteNew | CommandId::NoteNewFolder))
            .count()
    );
    let query = palette(window.hwnd).query_hwnd();
    let typed = crate::platform::wide_null("zoom");
    unsafe { SetWindowTextW(query, typed.as_ptr()) };
    let shown = palette(window.hwnd)
        .shown()
        .iter()
        .map(|entry| entry.command)
        .collect::<Vec<_>>();
    assert_eq!(
        shown,
        [CommandId::ZoomIn, CommandId::ZoomOut, CommandId::ZoomReset]
    );
    assert_eq!(palette(window.hwnd).shown_shortcut(0), Some("Ctrl+="));

    unsafe { SendMessageW(query, WM_KEYDOWN, VK_DOWN as usize, 0) };
    assert_eq!(
        palette(window.hwnd).selected_command(),
        Some(CommandId::ZoomOut)
    );
    unsafe { SendMessageW(query, WM_KEYDOWN, VK_RETURN as usize, 0) };
    assert!(!palette(window.hwnd).is_visible());
    assert!(!panel_visible(window.hwnd));
    assert_eq!(
        unsafe { SendMessageW(editor.hwnd(), SCI_GETZOOM, 0, 0) },
        -1
    );

    // Reopening starts from an empty query; Escape closes without running anything.
    execute_command(window.hwnd, CommandId::CommandPalette);
    assert_eq!(palette(window.hwnd).query_text(), "");
    unsafe { SendMessageW(query, WM_KEYDOWN, VK_ESCAPE as usize, 0) };
    assert!(!palette(window.hwnd).is_visible());
    assert_eq!(
        unsafe { SendMessageW(editor.hwnd(), SCI_GETZOOM, 0, 0) },
        -1
    );
}

#[test]
fn a_picker_lists_its_items_and_enter_reports_the_choice() {
    // Break caught: a picker whose choice never reaches library_host, or that leaves the
    // palette stuck showing runtime items the next time it opens for commands.
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    super::super::open_picker(
        window.hwnd,
        crate::window::command_palette::Picker {
            kind: crate::window::command_palette::PickerKind::RecentFolder,
            items: vec![r"D:\A".into(), r"D:\B".into()],
            create: None,
        },
    );
    super::super::move_command_palette_selection(window.hwnd, 1);
    super::super::run_command_palette_selection(window.hwnd);
    assert_eq!(
        crate::window::library_host::take_last_pick(),
        Some((
            crate::window::command_palette::PickerKind::RecentFolder,
            crate::window::command_palette::PickerChoice::Item(1)
        ))
    );
    // The palette went back to command mode.
    execute_command(window.hwnd, CommandId::CommandPalette);
    assert!(with_command_palette(window.hwnd, |p| p.picker().is_none()).unwrap());
}

#[test]
fn another_picker_opened_over_quick_open_repaints_the_field_without_its_hint() {
    // Break caught (review round 1): a picker opened from quick open's empty field (Move to
    // notebook, Open recent notebook) skips clearing the query, so nothing repaints the
    // field and it keeps showing "Go to note by name". Test windows are never shown, so
    // the repaint is counted rather than read from the field's update region.
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    let palette = || app_mut(window.hwnd).command_palette.as_ref().unwrap();
    execute_command(window.hwnd, CommandId::QuickOpen);
    assert!(palette().placeholder().is_some());
    let repaints = palette().hint_repaints();

    super::super::open_picker(
        window.hwnd,
        crate::window::command_palette::Picker {
            kind: crate::window::command_palette::PickerKind::RecentFolder,
            items: vec![r"D:\A".into()],
            create: None,
        },
    );

    assert_eq!(palette().placeholder(), None);
    assert_eq!(palette().hint_repaints(), repaints + 1);
    // Back to quick open, the hint returns; command mode, it goes again.
    execute_command(window.hwnd, CommandId::QuickOpen);
    assert_eq!(palette().hint_repaints(), repaints + 2);
    execute_command(window.hwnd, CommandId::CommandPalette);
    assert_eq!(palette().hint_repaints(), repaints + 3);
}

#[test]
fn ctrl_w_closes_the_active_tab_and_in_the_palette_field_closes_the_palette() {
    // Break caught (review focus 4): Ctrl+W dead, closing a background tab, or, typed in the
    // palette's query field, closing the tab behind the palette (the accelerator table sees
    // the key before the field's hook).
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetKeyboardState, SetKeyboardState, VK_CONTROL,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{MSG, WM_KEYDOWN};
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    let identity = unsafe { super::super::window_identity(window.hwnd).unwrap() };
    execute_command(window.hwnd, CommandId::New);
    execute_command(window.hwnd, CommandId::New);
    let ids = || {
        app_mut(window.hwnd)
            .tabs
            .documents()
            .map(|document| document.id)
            .collect::<Vec<_>>()
    };
    let &[first, second, _] = &ids()[..] else {
        panic!("three tabs")
    };
    let mut keys = [0u8; 256];
    unsafe { GetKeyboardState(keys.as_mut_ptr()) };
    let original = keys;
    keys[VK_CONTROL as usize] = 0x80;
    unsafe { SetKeyboardState(keys.as_ptr()) };
    let ctrl_w = |target: HWND| MSG {
        hwnd: target,
        message: WM_KEYDOWN,
        wParam: usize::from(b'W'),
        ..Default::default()
    };

    let closed = unsafe {
        super::super::translate_accelerator(window.hwnd, &identity, &ctrl_w(editor.hwnd()))
    };
    execute_command(window.hwnd, CommandId::CommandPalette);
    let query = with_command_palette(window.hwnd, |palette| palette.query_hwnd()).unwrap();
    let in_palette =
        unsafe { super::super::translate_accelerator(window.hwnd, &identity, &ctrl_w(query)) };
    unsafe { SendMessageW(query, WM_KEYDOWN, usize::from(b'W'), 0) };
    unsafe { SetKeyboardState(original.as_ptr()) };

    assert!(closed, "Ctrl+W was not translated");
    assert!(!in_palette, "the palette's field keeps Ctrl+W");
    assert!(!with_command_palette(window.hwnd, |palette| palette.is_visible()).unwrap());
    assert_eq!(ids(), [first, second], "only the active tab closed");
}

#[test]
fn a_middle_click_closes_a_clean_background_tab_and_keeps_the_active_one() {
    // Break caught (review focus 3): a middle-click switching to the tab it closes, closing
    // the active tab instead, a press on one tab and a release on another closing either, a
    // release with no press closing anything, or a press kept after the pointer left.
    use windows_sys::Win32::UI::WindowsAndMessaging::{WM_MBUTTONDOWN, WM_MBUTTONUP};
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    execute_command(window.hwnd, CommandId::New);
    execute_command(window.hwnd, CommandId::New);
    let ids = || {
        app_mut(window.hwnd)
            .tabs
            .documents()
            .map(|document| document.id)
            .collect::<Vec<_>>()
    };
    let &[first, second, third] = &ids()[..] else {
        panic!("three tabs")
    };
    let group = super::super::group_hwnd(window.hwnd).unwrap();
    let center = |index: usize| {
        super::super::strip_layout(window.hwnd)
            .unwrap()
            .tab(index)
            .unwrap()
            .center()
    };
    let send = |message: u32, index: usize| {
        let point = center(index);
        unsafe { SendMessageW(group, message, 0, client_lparam(point.x, point.y)) };
    };

    send(WM_MBUTTONDOWN, 0);
    send(WM_MBUTTONUP, 1);
    assert_eq!(ids(), [first, second, third], "released over another tab");
    send(WM_MBUTTONUP, 0);
    assert_eq!(ids(), [first, second, third], "a release with no press");
    send(WM_MBUTTONDOWN, 0);
    unsafe { SendMessageW(group, windows_sys::Win32::UI::Controls::WM_MOUSELEAVE, 0, 0) };
    send(WM_MBUTTONUP, 0);
    assert_eq!(ids(), [first, second, third], "the pointer left in between");

    send(WM_MBUTTONDOWN, 0);
    send(WM_MBUTTONUP, 0);
    assert_eq!(ids(), [second, third]);
    assert_eq!(app_mut(window.hwnd).tabs.active().unwrap().id, third);
    assert_eq!(app_mut(window.hwnd).tabs.active_index(), 1);
}

#[test]
fn a_middle_click_on_a_dirty_background_tab_shows_it_and_asks_first() {
    // Break caught: the save prompt asking about a tab that isn't on screen, a dirty tab
    // closed without asking, or Cancel putting the previously active tab back.
    use windows_sys::Win32::UI::WindowsAndMessaging::{WM_MBUTTONDOWN, WM_MBUTTONUP};
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    editor.set_text("unsaved").unwrap();
    let dirty = app_mut(window.hwnd).tabs.active().unwrap().id;
    assert!(app_mut(window.hwnd).tabs.active().unwrap().dirty);
    execute_command(window.hwnd, CommandId::New);
    let asked = std::rc::Rc::new(std::cell::Cell::new(false));
    let seen = asked.clone();
    answer_next_close_prompt(move |hwnd| {
        assert_eq!(
            app_mut(hwnd).tabs.active().unwrap().id,
            dirty,
            "the prompt's tab is on screen"
        );
        seen.set(true);
        CloseDecision::Cancel
    });
    let group = super::super::group_hwnd(window.hwnd).unwrap();
    let center = super::super::strip_layout(window.hwnd)
        .unwrap()
        .tab(0)
        .unwrap()
        .center();

    unsafe {
        SendMessageW(group, WM_MBUTTONDOWN, 0, client_lparam(center.x, center.y));
        SendMessageW(group, WM_MBUTTONUP, 0, client_lparam(center.x, center.y));
    }

    assert!(asked.get(), "no prompt");
    assert_eq!(super::super::tab_count(window.hwnd), 2);
    assert_eq!(
        app_mut(window.hwnd).tabs.active().unwrap().id,
        dirty,
        "after Cancel it stays active"
    );
    answer_next_close_prompt(|_| CloseDecision::Discard);
    super::super::close_tab_at(window.hwnd, 0);
    assert_eq!(super::super::tab_count(window.hwnd), 1);
}

/// The quick-open rows as their names, or the row itself for a non-note row.
fn quick_open_names(hwnd: HWND) -> Vec<String> {
    with_command_palette(hwnd, |palette| {
        palette
            .shown_picker_rows()
            .iter()
            .map(|row| match row {
                crate::window::command_palette::PickerRow::Note { found, .. }
                | crate::window::command_palette::PickerRow::View { found, .. } => {
                    found.name.clone()
                }
                other => format!("{other:?}"),
            })
            .collect()
    })
    .unwrap()
}

fn type_query(hwnd: HWND, text: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::SetWindowTextW;
    let query = with_command_palette(hwnd, |palette| palette.query_hwnd()).unwrap();
    let typed = crate::platform::wide_null(text);
    unsafe { SetWindowTextW(query, typed.as_ptr()) };
}

fn press_enter_in_palette(hwnd: HWND) {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_RETURN;
    use windows_sys::Win32::UI::WindowsAndMessaging::WM_KEYDOWN;
    let query = with_command_palette(hwnd, |palette| palette.query_hwnd()).unwrap();
    unsafe { SendMessageW(query, WM_KEYDOWN, VK_RETURN as usize, 0) };
}

fn palette_visible(hwnd: HWND) -> bool {
    with_command_palette(hwnd, |palette| palette.is_visible()).unwrap_or(false)
}

#[test]
fn ctrl_p_then_enter_switches_to_the_previous_note() {
    // Break caught: Ctrl+P dead in the running app, the open tabs listed in strip order, a
    // file outside the notebook or an unopened note listed, or the selection on the current
    // note, so Enter does nothing.
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetKeyboardState, SetKeyboardState, VK_CONTROL,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{MSG, WM_KEYDOWN};
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("quick-open-previous");
    let a = scratch.note("a.md", "a");
    let b = scratch.note("b.md", "b");
    scratch.note("c.md", "c");
    let outside = scratch.root.join("outside.txt");
    std::fs::write(&outside, "outside").unwrap();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    let identity = unsafe { super::super::window_identity(window.hwnd).unwrap() };
    super::super::open_path(window.hwnd, &a).unwrap();
    super::super::open_path(window.hwnd, &outside).unwrap();
    super::super::open_note(window.hwnd, &b, super::super::OpenMode::Permanent, false).unwrap();

    let mut keys = [0u8; 256];
    unsafe { GetKeyboardState(keys.as_mut_ptr()) };
    let original = keys;
    keys[VK_CONTROL as usize] = 0x80;
    unsafe { SetKeyboardState(keys.as_ptr()) };
    let message = MSG {
        hwnd: editor.hwnd(),
        message: WM_KEYDOWN,
        wParam: usize::from(b'P'),
        ..Default::default()
    };
    let translated =
        unsafe { super::super::translate_accelerator(window.hwnd, &identity, &message) };
    unsafe { SetKeyboardState(original.as_ptr()) };

    assert!(translated, "Ctrl+P was not translated");
    assert!(palette_visible(window.hwnd));
    assert_eq!(
        with_command_palette(window.hwnd, |p| p.picker().map(|p| p.kind)).flatten(),
        Some(crate::window::command_palette::PickerKind::QuickOpen)
    );
    assert_eq!(
        with_command_palette(window.hwnd, |p| p.query_text()).unwrap(),
        ""
    );
    assert_eq!(quick_open_names(window.hwnd), ["b.md", "a.md"]);
    assert_eq!(
        with_command_palette(window.hwnd, |p| p.selected_row()).flatten(),
        Some(1)
    );
    press_enter_in_palette(window.hwnd);
    assert!(!palette_visible(window.hwnd));
    assert_eq!(active_path(window.hwnd), Some(a));
}

#[test]
fn typing_a_name_then_enter_opens_a_closed_note_as_a_normal_tab() {
    // Break caught: typed letters never reaching the matcher, a folder-only match dropped,
    // or the pick opening in the preview tab the next sidebar click replaces.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("quick-open-type");
    let alpha = scratch.note("alpha.md", "a");
    scratch.note("beta.md", "b");
    std::fs::create_dir_all(scratch.folder().join("work")).unwrap();
    let gamma = scratch.note(r"work\gamma notes.md", "g");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    super::super::open_path(window.hwnd, &alpha).unwrap();

    execute_command(window.hwnd, CommandId::QuickOpen);
    type_query(window.hwnd, "wk gmn");
    assert_eq!(quick_open_names(window.hwnd), ["gamma notes.md"]);
    assert_eq!(
        with_command_palette(window.hwnd, |p| p.selected_row()).flatten(),
        Some(0)
    );
    press_enter_in_palette(window.hwnd);

    assert_eq!(active_path(window.hwnd), Some(gamma));
    assert_eq!(super::super::tab_count(window.hwnd), 2);
    assert_eq!(app_mut(window.hwnd).tabs.preview_id(), None);
}

#[test]
fn a_line_suffix_puts_the_caret_on_that_line_and_colon_digits_alone_moves_the_current_tab() {
    // Break caught: "lines:3" matched as text, the line applied 0-based (caret on line 4),
    // a line past the end ignored, or ":2" offering notes instead of moving the caret.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("quick-open-line");
    scratch.note("lines.md", "one\r\ntwo\r\nthree\r\nfour");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    let caret_line = || {
        editor
            .line_from_position(editor.selection().unwrap().start)
            .unwrap()
    };

    execute_command(window.hwnd, CommandId::QuickOpen);
    type_query(window.hwnd, "lines:3");
    assert_eq!(quick_open_names(window.hwnd), ["lines.md"]);
    press_enter_in_palette(window.hwnd);
    assert_eq!(caret_line(), 2);

    execute_command(window.hwnd, CommandId::QuickOpen);
    type_query(window.hwnd, ":2");
    assert_eq!(
        with_command_palette(window.hwnd, |p| p.shown_picker_rows().to_vec()).unwrap(),
        [crate::window::command_palette::PickerRow::GoToLine(2)]
    );
    press_enter_in_palette(window.hwnd);
    assert_eq!(caret_line(), 1);

    execute_command(window.hwnd, CommandId::QuickOpen);
    type_query(window.hwnd, "lines:99");
    press_enter_in_palette(window.hwnd);
    assert_eq!(caret_line(), 3, "past the end goes to the last line");

    execute_command(window.hwnd, CommandId::QuickOpen);
    type_query(window.hwnd, ":0");
    press_enter_in_palette(window.hwnd);
    assert_eq!(caret_line(), 0, "line 0 behaves as line 1");
}

#[test]
fn a_go_to_line_pick_focuses_the_editor_even_when_the_sidebar_had_focus() {
    // Break caught: a note pick focuses the editor (`open_note(.., true)`), but a ":n" pick
    // moved the caret and left the keyboard focus in the sidebar when it had it.
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetFocus, SetFocus};
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("quick-open-line-focus");
    let note = scratch.note("lines.md", "one\r\ntwo\r\nthree\r\nfour");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    ensure_sidebar(window.hwnd);
    scratch.install(window.hwnd);
    super::super::open_path(window.hwnd, &note).unwrap();
    let (_, panel) = sidebar_windows(window.hwnd);
    unsafe { SetFocus(panel) };
    assert_eq!(
        unsafe { GetFocus() },
        panel,
        "the panel must hold focus to start"
    );

    execute_command(window.hwnd, CommandId::QuickOpen);
    type_query(window.hwnd, ":3");
    press_enter_in_palette(window.hwnd);

    assert_eq!(
        unsafe { GetFocus() },
        editor.hwnd(),
        "a :n pick focuses the editor, the same as a note pick"
    );
}

#[test]
fn with_no_notebook_open_the_picker_shows_one_row_that_cannot_be_picked() {
    // Break caught: an empty list that looks broken, Enter closing the picker or opening
    // something, or ":5" refused although it needs no notebook.
    use crate::window::command_palette::{NO_NOTEBOOK, PickerRow};
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    assert!(crate::window::library_host::folder(window.hwnd).is_none());

    execute_command(window.hwnd, CommandId::QuickOpen);
    let rows = || with_command_palette(window.hwnd, |p| p.shown_picker_rows().to_vec()).unwrap();
    assert_eq!(rows(), [PickerRow::Notice(NO_NOTEBOOK)]);
    assert_eq!(
        with_command_palette(window.hwnd, |p| p.selected_row()).flatten(),
        None
    );
    press_enter_in_palette(window.hwnd);
    assert!(palette_visible(window.hwnd), "Enter does nothing");
    assert_eq!(super::super::tab_count(window.hwnd), 1);

    type_query(window.hwnd, ":5");
    assert_eq!(rows(), [PickerRow::GoToLine(5)]);
}

#[test]
fn ctrl_p_again_keeps_the_query_and_a_query_of_spaces_lists_the_open_tabs() {
    // Break caught (review focus 1 and 4): a second Ctrl+P clearing what was typed, or a
    // query of spaces listing every note, or none.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("quick-open-again");
    let a = scratch.note("alpha.md", "a");
    let b = scratch.note("beta.md", "b");
    scratch.note("gamma.md", "g");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    super::super::open_path(window.hwnd, &a).unwrap();
    super::super::open_path(window.hwnd, &b).unwrap();

    execute_command(window.hwnd, CommandId::QuickOpen);
    type_query(window.hwnd, "gam");
    execute_command(window.hwnd, CommandId::QuickOpen);
    assert_eq!(
        with_command_palette(window.hwnd, |p| p.query_text()).unwrap(),
        "gam"
    );
    assert_eq!(quick_open_names(window.hwnd), ["gamma.md"]);

    type_query(window.hwnd, "   ");
    assert_eq!(quick_open_names(window.hwnd), ["beta.md", "alpha.md"]);
    assert_eq!(
        with_command_palette(window.hwnd, |p| p.selected_row()).flatten(),
        Some(1)
    );
}

#[test]
fn a_tab_closed_while_the_picker_is_open_drops_its_row() {
    // Break caught (review focus 5): a row naming a tab that closed under the open picker
    // switching to a dead document, or the row lingering after the close so a middle-click
    // that closes a background tab leaves it pickable although the tab is gone.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("quick-open-closed-tab");
    let a = scratch.note("a.md", "a");
    let b = scratch.note("b.md", "b");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    super::super::open_path(window.hwnd, &a).unwrap();
    super::super::open_path(window.hwnd, &b).unwrap();

    execute_command(window.hwnd, CommandId::QuickOpen);
    assert_eq!(quick_open_names(window.hwnd), ["b.md", "a.md"]);
    // The clean background tab (index 0, "a") closes the way a middle-click closes it: no
    // focus moves, so the palette stays open and must refresh its rows (spec §5).
    super::super::close_tab_at(window.hwnd, 0);
    assert_eq!(tab_paths(window.hwnd), [Some(b.clone())]);
    assert!(palette_visible(window.hwnd), "the palette stayed open");
    assert_eq!(
        quick_open_names(window.hwnd),
        ["b.md"],
        "the closed tab's row is gone"
    );
    press_enter_in_palette(window.hwnd);

    assert_eq!(tab_paths(window.hwnd), [Some(b.clone())]);
    assert_eq!(active_path(window.hwnd), Some(b));
}

#[test]
fn a_note_removed_from_the_library_after_listing_reports_it_and_opens_nothing() {
    // Break caught (review focus 5): a note deleted after the list was shown opening an
    // empty tab, failing silently, or crashing the pick.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("quick-open-removed");
    scratch.note("alpha.md", "a");
    let gamma = scratch.note("gamma.md", "g");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    scratch.install(window.hwnd);
    let tabs_before = super::super::tab_count(window.hwnd);

    execute_command(window.hwnd, CommandId::QuickOpen);
    type_query(window.hwnd, "gam");
    assert_eq!(quick_open_names(window.hwnd), ["gamma.md"]);
    crate::window::library_host::with_state(window.hwnd, |state| state.remove_note(&gamma));
    std::fs::remove_file(&gamma).unwrap();
    press_enter_in_palette(window.hwnd);

    assert_eq!(super::super::tab_count(window.hwnd), tabs_before);
    assert!(
        notices(window.hwnd)
            .iter()
            .any(|n| n.contains("could not open") && n.contains("no longer in the notebook")),
        "{:?}",
        notices(window.hwnd)
    );
}

#[test]
fn quick_open_rows_carry_their_text_for_screen_readers_and_draw_their_hits_in_bold() {
    // Break caught: rows a screen reader reads as blank, the notice read as anything else,
    // hits drawn in the regular font, or the hint missing from the empty field (no ComCtl32
    // v6 manifest, so EM_SETCUEBANNER shows nothing) or left behind in command mode.
    use windows_sys::Win32::Graphics::Gdi::{CreateCompatibleDC, DeleteDC};
    use windows_sys::Win32::UI::Controls::DRAWITEMSTRUCT;
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    let palette = || app_mut(window.hwnd).command_palette.as_ref().unwrap();

    execute_command(window.hwnd, CommandId::QuickOpen);
    assert_eq!(
        palette().list_text(0),
        crate::window::command_palette::NO_NOTEBOOK
    );
    assert_eq!(palette().placeholder(), Some("Go to note by name"));
    let query = palette().query_hwnd();
    assert!(palette().paint_placeholder(query));
    execute_command(window.hwnd, CommandId::CommandPalette);
    assert_eq!(palette().placeholder(), None);
    assert!(!palette().paint_placeholder(query));

    let scratch = LibraryScratch::new("quick-open-draw");
    std::fs::create_dir_all(scratch.folder().join("work")).unwrap();
    scratch.note(r"work\gamma notes.md", "g");
    scratch.install(window.hwnd);
    execute_command(window.hwnd, CommandId::QuickOpen);
    type_query(window.hwnd, "wk gmn");
    assert_eq!(palette().list_text(0), r"gamma notes.md, in work");

    let dc = unsafe { CreateCompatibleDC(std::ptr::null_mut()) };
    let item = DRAWITEMSTRUCT {
        itemID: 0,
        hDC: dc,
        rcItem: RECT {
            left: 0,
            top: 0,
            right: 400,
            bottom: 26,
        },
        ..Default::default()
    };
    palette().draw_item(&item);
    unsafe { DeleteDC(dc) };
    assert!(palette().has_bold_font());
}
