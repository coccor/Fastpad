//! Favorites, the Settings dialog's size, and the keyboard shortcuts page.

use super::*;

#[test]
fn a_favorite_opens_from_the_favorites_view_and_its_menu_removes_it() {
    // Break caught: a click on a favorite not switching the notebook or leaving the Favorites
    // view up, or "Remove from favorites" in the row menu doing nothing.
    let _scintilla = load_native_scintilla();
    let first = LibraryScratch::new("favorites-a");
    let second = LibraryScratch::new("favorites-b");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    app_mut(window.hwnd).library.data_dir = Some(first.data());
    second.install(window.hwnd);
    crate::window::library_host::toggle_notebook_favorite(window.hwnd);
    first.install(window.hwnd);
    use crate::config::SidebarView;
    crate::window::side_panel::show_view(window.hwnd, SidebarView::Favorites, true);
    let rows = crate::window::favorites_view::shown_rows(window.hwnd);
    assert_eq!(rows.len(), 1);
    assert!(!rows[0].open);

    crate::window::favorites_view::run(
        window.hwnd,
        crate::window::favorites_view::FavoriteAction::Open(second.folder()),
        true,
    );
    // The folder is checked on a worker; the view switches once the notebook is open.
    pump_until(window.hwnd, || {
        crate::window::side_panel::current_view(window.hwnd) == SidebarView::Notebook
    });
    assert!(crate::library::model::same_path(
        &crate::window::library_host::folder(window.hwnd).unwrap(),
        &second.folder()
    ));

    crate::window::side_panel::show_view(window.hwnd, SidebarView::Favorites, true);
    let panel = sidebar_panel(window.hwnd);
    unsafe {
        SendMessageW(
            panel,
            windows_sys::Win32::UI::WindowsAndMessaging::WM_KEYDOWN,
            0x24,
            0,
        );
    }
    crate::window::menus::answer_next_popup_menu(|_| Some(CommandId::ToggleNotebookFavorite));
    // Shift+F10 arrives as WM_CONTEXTMENU with (-1, -1).
    unsafe {
        SendMessageW(
            panel,
            windows_sys::Win32::UI::WindowsAndMessaging::WM_CONTEXTMENU,
            panel as usize,
            0xffff_ffff,
        );
    }
    assert!(crate::window::favorites_view::shown_rows(window.hwnd).is_empty());
}

#[test]
fn ctrl_comma_and_edit_settings_file_run_from_the_command_table() {
    // Break caught: OpenSettings or EditSettingsFile falling through to `App::execute`,
    // which ignores them.
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE;
    use windows_sys::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_KEYDOWN};
    let _scintilla = load_native_scintilla();
    let scratch = RecoveryScratch::new("settings-commands");
    let ini = scratch.path().join("fastpad.ini");
    super::super::save_settings_to(Some(ini.clone()));
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    let shown = std::rc::Rc::new(std::cell::Cell::new(false));
    let seen = shown.clone();
    crate::window::settings_dialog::answer_next(move |dialog| {
        seen.set(true);
        unsafe { PostMessageW(dialog, WM_KEYDOWN, usize::from(VK_ESCAPE), 0) };
    });
    execute_command(window.hwnd, CommandId::OpenSettings);
    assert!(shown.get());

    execute_command(window.hwnd, CommandId::EditSettingsFile);
    assert!(app_mut(window.hwnd).tabs.find_path(&ini).is_some());
    super::super::save_settings_to(None);
}

#[test]
fn open_keyboard_shortcuts_opens_settings_on_the_shortcuts_page() {
    // Break caught: the palette command opening Settings on General, or not at all.
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE;
    use windows_sys::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_KEYDOWN};
    let window = ProductionWindow::new(make_app());
    crate::window::settings_dialog::answer_next(|dialog| unsafe {
        assert_eq!(
            crate::window::settings_dialog::current_page(dialog),
            Some(crate::window::settings_model::Page::Shortcuts)
        );
        PostMessageW(dialog, WM_KEYDOWN, usize::from(VK_ESCAPE), 0);
    });
    super::super::execute_command(window.hwnd, CommandId::OpenKeyboardShortcuts);
    assert_eq!(
        crate::window::settings_dialog::open_dialog(window.hwnd),
        None
    );
}

#[test]
fn the_wheel_on_the_shortcuts_page_scrolls_the_table() {
    // Break caught: the wheel scrolling General's hidden cards while the table stays put.
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        PostMessageW, SendMessageW, WM_KEYDOWN, WM_MOUSEWHEEL,
    };
    let window = ProductionWindow::new(make_app());
    let tops = std::rc::Rc::new(RefCell::new(Vec::new()));
    let seen = tops.clone();
    crate::window::settings_dialog::answer_next(move |dialog| unsafe {
        let top = || crate::window::settings_dialog::shortcuts_model(dialog).map(|m| m.top);
        seen.borrow_mut().push(top());
        // One notch down, then one back up.
        for delta in [-120i16, 120] {
            SendMessageW(dialog, WM_MOUSEWHEEL, usize::from(delta as u16) << 16, 0);
            seen.borrow_mut().push(top());
        }
        PostMessageW(dialog, WM_KEYDOWN, usize::from(VK_ESCAPE), 0);
    });
    super::super::show_keyboard_shortcuts(window.hwnd);
    assert_eq!(*tops.borrow(), [Some(0), Some(3), Some(0)]);
}

#[test]
fn settings_remembers_the_size_it_was_dragged_to_and_shows_sizing_cursors() {
    // Break caught: Settings reopening at its default size after the user sized it, a move
    // (no resize) rewriting fastpad.ini, or the arrow cursor over the edges hiding that they
    // size the dialog.
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetCursor, GetWindowRect, HTRIGHT, IDC_SIZEWE, LoadCursorW, PostMessageW, SWP_NOMOVE,
        SWP_NOZORDER, SendMessageW, SetWindowPos, WM_EXITSIZEMOVE, WM_KEYDOWN, WM_MOUSEMOVE,
        WM_SETCURSOR,
    };
    let scratch = RecoveryScratch::new("settings-size");
    let ini = scratch.path().join("fastpad.ini");
    std::fs::write(&ini, "# kept\r\n").unwrap();
    super::super::save_settings_to(Some(ini.clone()));
    let window = ProductionWindow::new(make_app());
    let sizing_cursor = std::rc::Rc::new(std::cell::Cell::new(false));
    let seen = sizing_cursor.clone();
    crate::window::settings_dialog::answer_next(move |dialog| unsafe {
        SendMessageW(
            dialog,
            WM_SETCURSOR,
            dialog as usize,
            ((WM_MOUSEMOVE as isize) << 16) | HTRIGHT as isize,
        );
        seen.set(GetCursor() == LoadCursorW(std::ptr::null_mut(), IDC_SIZEWE));
        // A drag that ends where it began only moved it: nothing is written.
        SendMessageW(dialog, WM_EXITSIZEMOVE, 0, 0);
        SetWindowPos(
            dialog,
            std::ptr::null_mut(),
            0,
            0,
            700,
            500,
            SWP_NOMOVE | SWP_NOZORDER,
        );
        SendMessageW(dialog, WM_EXITSIZEMOVE, 0, 0);
        PostMessageW(dialog, WM_KEYDOWN, usize::from(VK_ESCAPE), 0);
    });
    let saved = || std::fs::read_to_string(&ini).unwrap();
    let before = std::cell::Cell::new(String::new());
    super::super::show_settings(window.hwnd);
    before.set(saved());
    let dpi = unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForWindow(window.hwnd) }.max(96);
    let size = std::rc::Rc::new(std::cell::Cell::new((0, 0)));
    let reopened = size.clone();
    crate::window::settings_dialog::answer_next(move |dialog| unsafe {
        let mut rect = RECT::default();
        GetWindowRect(dialog, &mut rect);
        reopened.set((rect.right - rect.left, rect.bottom - rect.top));
        PostMessageW(dialog, WM_KEYDOWN, usize::from(VK_ESCAPE), 0);
    });
    super::super::show_settings(window.hwnd);
    super::super::save_settings_to(None);
    assert!(sizing_cursor.get(), "the edge shows the sizing cursor");
    let expected = |pixels: i32| (pixels * 96 + dpi as i32 / 2) / dpi as i32;
    assert_eq!(
        before.take(),
        format!(
            "# kept\r\nsettings_size={}x{}\r\n",
            expected(700),
            expected(500)
        )
    );
    assert_eq!(
        app_mut(window.hwnd).settings.settings_size,
        Some((expected(700) as u16, expected(500) as u16))
    );
    assert_eq!(size.get(), (700, 500), "reopens at the saved size");
}

#[test]
fn resizing_settings_lays_out_the_table_and_the_search_field_again() {
    // Break caught: a Settings dialog that can't be sized, or one whose table and search
    // field keep their opening size (rows cut off, or space under the last row) after it is.
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GWL_STYLE, GetWindowLongW, GetWindowRect, PostMessageW, SWP_NOMOVE, SWP_NOZORDER,
        SetWindowPos, WM_KEYDOWN, WS_THICKFRAME,
    };
    let window = ProductionWindow::new(make_app());
    let seen = std::rc::Rc::new(RefCell::new(Vec::new()));
    let record = seen.clone();
    crate::window::settings_dialog::answer_next(move |dialog| unsafe {
        let sizable = GetWindowLongW(dialog, GWL_STYLE) as u32 & WS_THICKFRAME != 0;
        let measure = || {
            let visible = crate::window::settings_dialog::shortcuts_model(dialog)
                .map_or(0, |model| model.visible);
            let mut dialog_rect = RECT::default();
            let mut field = RECT::default();
            GetWindowRect(dialog, &mut dialog_rect);
            GetWindowRect(
                crate::window::settings_dialog::search_hwnd(dialog),
                &mut field,
            );
            (
                visible,
                field.right - field.left,
                dialog_rect.right - dialog_rect.left,
            )
        };
        // Small enough to fit a 1024 px wide screen (a CI runner's) from the 860 px opening
        // width: the system holds a window to the screen, so a bigger step would come up short.
        const GROWTH: i32 = 100;
        let before = measure();
        let (_, _, width) = before;
        SetWindowPos(
            dialog,
            std::ptr::null_mut(),
            0,
            0,
            width + GROWTH,
            900,
            SWP_NOMOVE | SWP_NOZORDER,
        );
        let bigger = measure();
        // Far below the minimum: held at it.
        SetWindowPos(
            dialog,
            std::ptr::null_mut(),
            0,
            0,
            100,
            100,
            SWP_NOMOVE | SWP_NOZORDER,
        );
        let smallest = measure();
        record
            .borrow_mut()
            .push((sizable, before, bigger, smallest));
        PostMessageW(dialog, WM_KEYDOWN, usize::from(VK_ESCAPE), 0);
    });
    super::super::show_keyboard_shortcuts(window.hwnd);
    let seen = seen.borrow();
    let (sizable, before, bigger, smallest) = seen[0];
    assert!(sizable, "the dialog has a sizing frame");
    assert!(bigger.0 > before.0, "a taller dialog shows more rows");
    assert_eq!(
        bigger.1,
        before.1 + 100,
        "the search field widens with the dialog"
    );
    assert!(smallest.2 > 100, "held at a minimum width");
    assert!(smallest.0 >= 1 && smallest.1 > 0, "still a row and a field");
}

#[test]
fn small_wheel_deltas_add_up_to_whole_rows_on_the_shortcuts_page() {
    // Break caught: a touchpad's small deltas each rounding to zero rows, so slow scrolling
    // never moves the table; or a leftover from one direction eating the first reverse step.
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        PostMessageW, SendMessageW, WM_KEYDOWN, WM_MOUSEWHEEL,
    };
    let window = ProductionWindow::new(make_app());
    let tops = std::rc::Rc::new(RefCell::new(Vec::new()));
    let seen = tops.clone();
    crate::window::settings_dialog::answer_next(move |dialog| unsafe {
        let top = || crate::window::settings_dialog::shortcuts_model(dialog).map(|m| m.top);
        let wheel = |delta: i16| {
            SendMessageW(dialog, WM_MOUSEWHEEL, usize::from(delta as u16) << 16, 0);
        };
        // Four quarter notches down: one notch, three rows.
        for _ in 0..4 {
            wheel(-30);
        }
        seen.borrow_mut().push(top());
        // A leftover third of a row down, then one notch up: exactly three rows back.
        wheel(-30);
        wheel(120);
        seen.borrow_mut().push(top());
        PostMessageW(dialog, WM_KEYDOWN, usize::from(VK_ESCAPE), 0);
    });
    super::super::show_keyboard_shortcuts(window.hwnd);
    assert_eq!(*tops.borrow(), [Some(3), Some(0)]);
}

#[test]
fn a_double_click_on_a_row_opens_the_recording_box_and_f9_rebinds_it() {
    // Break caught: rows that select but never open the box, or a confirmed key that the
    // window never applies.
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_ESCAPE, VK_F9, VK_RETURN};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        PostMessageW, WM_CLOSE, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP,
    };
    let scratch = RecoveryScratch::new("shortcuts-double-click");
    let ini = scratch.path().join("fastpad.ini");
    super::super::save_settings_to(Some(ini.clone()));
    let window = ProductionWindow::new(make_app());
    crate::window::settings_dialog::answer_next(|dialog| unsafe {
        let (x, y) = crate::window::settings_dialog::page_row_point(dialog, 0);
        let at = ((y as isize) << 16 | (x as isize & 0xffff)) as LPARAM;
        for _ in 0..2 {
            PostMessageW(dialog, WM_LBUTTONDOWN, 1, at);
            PostMessageW(dialog, WM_LBUTTONUP, 0, at);
        }
        let key = |vk: u16| PostMessageW(dialog, WM_KEYDOWN, usize::from(vk), 0);
        key(VK_F9);
        key(VK_RETURN);
        key(VK_ESCAPE);
        // Should the box never open, Escape only closes it: this ends the dialog anyway, so
        // the test fails instead of hanging.
        PostMessageW(dialog, WM_CLOSE, 0, 0);
    });
    super::super::show_keyboard_shortcuts(window.hwnd);
    let first = crate::window::shortcuts_model::ShortcutsModel::new(
        crate::window::keymap::Keymap::defaults(),
        1,
    )
    .rows[0]
        .clone();
    assert_eq!(
        app_mut(window.hwnd)
            .keymap
            .keys_of(first.command)
            .last()
            .map(|stroke| stroke.text())
            .as_deref(),
        Some("F9")
    );
    super::super::save_settings_to(None);
    assert!(
        std::fs::read_to_string(&ini)
            .unwrap()
            .contains(&format!("key.{}=", first.id))
    );
}

#[test]
fn typing_in_the_search_filters_and_down_enters_the_table() {
    // Break caught: EN_CHANGE not reaching the model, or Down leaving the focus in the field.
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_DOWN, VK_ESCAPE};
    use windows_sys::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_CHAR, WM_KEYDOWN};
    let window = ProductionWindow::new(make_app());
    let seen = std::rc::Rc::new(std::cell::RefCell::new(None));
    let record = seen.clone();
    crate::window::settings_dialog::answer_next(move |dialog| unsafe {
        let search = crate::window::settings_dialog::search_hwnd(dialog);
        for c in "save as".chars() {
            PostMessageW(search, WM_CHAR, c as usize, 0);
        }
        PostMessageW(search, WM_KEYDOWN, usize::from(VK_DOWN), 0);
        // Read the state from inside the loop, before Escape closes the dialog.
        crate::window::settings_dialog::answer_in_loop(dialog, move |dialog| {
            *record.borrow_mut() =
                crate::window::settings_dialog::shortcuts_model(dialog).map(|model| {
                    (
                        model.rows.len(),
                        model.rows[0].command,
                        crate::window::settings_dialog::current_focus(dialog),
                    )
                });
            PostMessageW(dialog, WM_KEYDOWN, usize::from(VK_ESCAPE), 0);
        });
    });
    super::super::show_keyboard_shortcuts(window.hwnd);
    let (count, command, focus) = seen.borrow().unwrap();
    assert_eq!((count, command), (1, CommandId::SaveAs));
    assert_eq!(focus, Some(crate::window::settings_model::Focus::Table));
}

#[test]
fn record_keys_search_shows_only_the_stroke() {
    // Break caught: record-keys mode letting the key's character into the field ("Ss").
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowTextW, PostMessageW, WM_CHAR, WM_KEYDOWN,
    };
    let window = ProductionWindow::new(make_app());
    let seen = std::rc::Rc::new(std::cell::RefCell::new(String::new()));
    let record = seen.clone();
    crate::window::settings_dialog::answer_next(move |dialog| unsafe {
        let search = crate::window::settings_dialog::search_hwnd(dialog);
        crate::window::settings_dialog::toggle_record_keys_for_test(dialog);
        PostMessageW(search, WM_KEYDOWN, usize::from(b'S'), 0);
        PostMessageW(search, WM_CHAR, usize::from(b's'), 0);
        crate::window::settings_dialog::answer_in_loop(dialog, move |dialog| {
            let mut buffer = [0u16; 64];
            let length = GetWindowTextW(search, buffer.as_mut_ptr(), 64);
            *record.borrow_mut() = String::from_utf16_lossy(&buffer[..length as usize]);
            PostMessageW(dialog, WM_KEYDOWN, usize::from(VK_ESCAPE), 0);
        });
    });
    super::super::show_keyboard_shortcuts(window.hwnd);
    assert_eq!(seen.borrow().as_str(), "S");
}

#[test]
fn delete_unbinds_and_the_context_menus_reset_restores_the_defaults() {
    // Break caught: Reset offered for default rows, or leaving `key.file.saveAs=` behind.
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_DELETE, VK_DOWN, VK_ESCAPE};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        PostMessageW, WM_CHAR, WM_KEYDOWN, WM_RBUTTONUP,
    };
    let scratch = RecoveryScratch::new("shortcuts-reset");
    let ini = scratch.path().join("fastpad.ini");
    super::super::save_settings_to(Some(ini.clone()));
    let window = ProductionWindow::new(make_app());
    let offered = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let record = offered.clone();
    crate::window::menus::answer_next_choice(move |items| {
        *record.borrow_mut() = items.iter().map(|(label, _)| label.clone()).collect();
        items
            .iter()
            .find(|(label, _)| label.starts_with("Reset"))
            .map(|(_, id)| *id)
    });
    crate::window::settings_dialog::answer_next(|dialog| unsafe {
        let search = crate::window::settings_dialog::search_hwnd(dialog);
        for c in "save as".chars() {
            PostMessageW(search, WM_CHAR, c as usize, 0);
        }
        PostMessageW(search, WM_KEYDOWN, usize::from(VK_DOWN), 0);
        PostMessageW(dialog, WM_KEYDOWN, usize::from(VK_DELETE), 0);
        crate::window::settings_dialog::answer_in_loop(dialog, |dialog| {
            let (x, y) = crate::window::settings_dialog::page_row_point(dialog, 0);
            PostMessageW(
                dialog,
                WM_RBUTTONUP,
                0,
                ((y as isize) << 16 | (x as isize & 0xffff)) as LPARAM,
            );
            PostMessageW(dialog, WM_KEYDOWN, usize::from(VK_ESCAPE), 0);
        });
    });
    super::super::show_keyboard_shortcuts(window.hwnd);
    assert!(
        offered
            .borrow()
            .iter()
            .any(|label| label.starts_with("Reset"))
    );
    assert!(
        !offered
            .borrow()
            .iter()
            .any(|label| label.starts_with("Remove")),
        "the row has no key to remove"
    );
    assert!(!app_mut(window.hwnd).keymap.is_user(CommandId::SaveAs));
    super::super::save_settings_to(None);
    assert_eq!(std::fs::read_to_string(&ini).unwrap_or_default(), "");
}

#[test]
fn recording_sees_f10_and_refuses_it() {
    // Break caught: F10 (a WM_SYSKEYDOWN) opening the dialog's system menu or beeping
    // instead of reaching the recording box.
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_DOWN, VK_ESCAPE, VK_F10, VK_RETURN};
    use windows_sys::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_KEYDOWN, WM_SYSKEYDOWN};
    let window = ProductionWindow::new(make_app());
    let refusal = std::rc::Rc::new(std::cell::RefCell::new(None));
    let record = refusal.clone();
    crate::window::settings_dialog::answer_next(move |dialog| unsafe {
        let search = crate::window::settings_dialog::search_hwnd(dialog);
        PostMessageW(search, WM_KEYDOWN, usize::from(VK_DOWN), 0);
        PostMessageW(dialog, WM_KEYDOWN, usize::from(VK_RETURN), 0);
        PostMessageW(dialog, WM_SYSKEYDOWN, usize::from(VK_F10), 0);
        crate::window::settings_dialog::answer_in_loop(dialog, move |dialog| {
            *record.borrow_mut() = crate::window::settings_dialog::shortcuts_model(dialog)
                .and_then(|model| model.recording)
                .and_then(|recording| recording.refusal);
            PostMessageW(dialog, WM_KEYDOWN, usize::from(VK_ESCAPE), 0);
            PostMessageW(dialog, WM_KEYDOWN, usize::from(VK_ESCAPE), 0);
        });
    });
    super::super::show_keyboard_shortcuts(window.hwnd);
    assert_eq!(*refusal.borrow(), Some("F10 and Shift+F10 open the menus."));
}

#[test]
fn a_click_on_the_search_field_cancels_the_recording_box() {
    // Break caught: a click on the search field focusing it behind the open recording box,
    // so typing filters the table and Escape closes the dialog instead of the box.
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_DOWN, VK_ESCAPE, VK_RETURN};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        PostMessageW, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP,
    };
    let window = ProductionWindow::new(make_app());
    let seen = std::rc::Rc::new(std::cell::RefCell::new(None));
    let record = seen.clone();
    crate::window::settings_dialog::answer_next(move |dialog| unsafe {
        let search = crate::window::settings_dialog::search_hwnd(dialog);
        PostMessageW(search, WM_KEYDOWN, usize::from(VK_DOWN), 0);
        PostMessageW(dialog, WM_KEYDOWN, usize::from(VK_RETURN), 0);
        crate::window::settings_dialog::answer_in_loop(dialog, move |dialog| {
            let open = crate::window::settings_dialog::shortcuts_model(dialog)
                .is_some_and(|model| model.recording.is_some());
            PostMessageW(search, WM_LBUTTONDOWN, 1, 0x0005_0005);
            PostMessageW(search, WM_LBUTTONUP, 0, 0x0005_0005);
            crate::window::settings_dialog::answer_in_loop(dialog, move |dialog| {
                let still_open = crate::window::settings_dialog::shortcuts_model(dialog)
                    .is_some_and(|model| model.recording.is_some());
                *record.borrow_mut() = Some((open, still_open));
                // With the box still open (the break), this Escape closes the dialog anyway.
                PostMessageW(search, WM_KEYDOWN, usize::from(VK_ESCAPE), 0);
            });
        });
    });
    super::super::show_keyboard_shortcuts(window.hwnd);
    assert_eq!(*seen.borrow(), Some((true, false)));
}

#[test]
fn record_keys_turned_on_from_the_table_moves_the_focus_to_the_search_field() {
    // Break caught: Alt+K from the table turning record-keys on with the focus left on the
    // table, so the next stroke goes to the table (and Escape closes the dialog) instead of
    // being recorded in the field. Alt can't be posted, so the hook runs Alt+K's own path.
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetFocus, VK_DOWN, VK_ESCAPE, VK_F9};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowTextW, PostMessageW, WM_CLOSE, WM_KEYDOWN,
    };
    let window = ProductionWindow::new(make_app());
    let seen = std::rc::Rc::new(std::cell::RefCell::new(None));
    let record = seen.clone();
    crate::window::settings_dialog::answer_next(move |dialog| unsafe {
        let search = crate::window::settings_dialog::search_hwnd(dialog);
        PostMessageW(search, WM_KEYDOWN, usize::from(VK_DOWN), 0);
        crate::window::settings_dialog::answer_in_loop(dialog, move |dialog| {
            let before = crate::window::settings_dialog::current_focus(dialog);
            crate::window::settings_dialog::toggle_record_keys_for_test(dialog);
            // The stroke goes wherever the keyboard focus is, as a real key would.
            let focused = GetFocus();
            PostMessageW(focused, WM_KEYDOWN, usize::from(VK_F9), 0);
            crate::window::settings_dialog::answer_in_loop(dialog, move |dialog| {
                let mut buffer = [0u16; 64];
                let length = GetWindowTextW(search, buffer.as_mut_ptr(), 64);
                *record.borrow_mut() = Some((
                    before,
                    focused == search,
                    crate::window::settings_dialog::current_focus(dialog),
                    String::from_utf16_lossy(&buffer[..length as usize]),
                ));
                // Escape leaves record-keys; the second closes the dialog.
                PostMessageW(GetFocus(), WM_KEYDOWN, usize::from(VK_ESCAPE), 0);
                PostMessageW(dialog, WM_CLOSE, 0, 0);
            });
        });
    });
    super::super::show_keyboard_shortcuts(window.hwnd);
    let (before, focused, focus, text) = seen.borrow().clone().unwrap();
    assert_eq!(before, Some(crate::window::settings_model::Focus::Table));
    assert!(focused, "the search field does not have the keyboard focus");
    assert_eq!(focus, Some(crate::window::settings_model::Focus::Search));
    assert_eq!(text, "F9");
}

#[test]
fn the_context_menu_key_opens_the_row_menu_and_resets() {
    // Break caught: the row menu reachable only by right-click, so a keyboard user can't
    // reset a command's keys; or the key opening a menu of its own besides the one its
    // WM_CONTEXTMENU opens.
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_APPS, VK_DELETE, VK_DOWN, VK_ESCAPE};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        PostMessageW, WM_CHAR, WM_CONTEXTMENU, WM_KEYDOWN, WM_KEYUP,
    };
    let scratch = RecoveryScratch::new("shortcuts-apps-key");
    super::super::save_settings_to(Some(scratch.path().join("fastpad.ini")));
    let window = ProductionWindow::new(make_app());
    let offered = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let record = offered.clone();
    crate::window::menus::answer_next_choice(move |items| {
        *record.borrow_mut() = items.iter().map(|(label, _)| label.clone()).collect();
        items
            .iter()
            .find(|(label, _)| label.starts_with("Reset"))
            .map(|(_, id)| *id)
    });
    let again = std::rc::Rc::new(std::cell::Cell::new(false));
    let second = again.clone();
    crate::window::menus::answer_next_choice(move |_| {
        second.set(true);
        None
    });
    crate::window::settings_dialog::answer_next(|dialog| unsafe {
        let search = crate::window::settings_dialog::search_hwnd(dialog);
        for c in "save as".chars() {
            PostMessageW(search, WM_CHAR, c as usize, 0);
        }
        PostMessageW(search, WM_KEYDOWN, usize::from(VK_DOWN), 0);
        PostMessageW(dialog, WM_KEYDOWN, usize::from(VK_DELETE), 0);
        crate::window::settings_dialog::answer_in_loop(dialog, |dialog| {
            // The key itself opens nothing; Windows follows it with a context menu with no
            // point, which opens the menu once.
            PostMessageW(dialog, WM_KEYDOWN, usize::from(VK_APPS), 0);
            PostMessageW(dialog, WM_KEYUP, usize::from(VK_APPS), 0xC000_0001);
            PostMessageW(dialog, WM_CONTEXTMENU, dialog as usize, -1isize as LPARAM);
            PostMessageW(dialog, WM_KEYDOWN, usize::from(VK_ESCAPE), 0);
        });
    });
    super::super::show_keyboard_shortcuts(window.hwnd);
    super::super::save_settings_to(None);
    assert!(
        offered
            .borrow()
            .iter()
            .any(|label| label.starts_with("Reset")),
        "no menu, or no Reset in it: {:?}",
        offered.borrow()
    );
    assert!(!again.get(), "the menu opened twice");
    assert!(!app_mut(window.hwnd).keymap.is_user(CommandId::SaveAs));
}

#[test]
fn an_ignored_key_line_can_be_reset_from_the_row_menu() {
    // Break caught: `key.file.saveAs=Bogus` (ignored, so the keys are the defaults) offering
    // no Reset, leaving the stale line in fastpad.ini for good.
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_DOWN, VK_ESCAPE};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        PostMessageW, WM_CHAR, WM_CONTEXTMENU, WM_KEYDOWN,
    };
    let scratch = RecoveryScratch::new("shortcuts-stale-line");
    super::super::save_settings_to(Some(scratch.path().join("fastpad.ini")));
    let window = ProductionWindow::new(make_app());
    app_mut(window.hwnd)
        .settings
        .key_overrides
        .insert("file.saveAs".into(), "Bogus".into());
    let offered = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let record = offered.clone();
    crate::window::menus::answer_next_choice(move |items| {
        *record.borrow_mut() = items.iter().map(|(label, _)| label.clone()).collect();
        items
            .iter()
            .find(|(label, _)| label.starts_with("Reset"))
            .map(|(_, id)| *id)
    });
    crate::window::settings_dialog::answer_next(|dialog| unsafe {
        let search = crate::window::settings_dialog::search_hwnd(dialog);
        for c in "save as".chars() {
            PostMessageW(search, WM_CHAR, c as usize, 0);
        }
        PostMessageW(search, WM_KEYDOWN, usize::from(VK_DOWN), 0);
        // Shift+F10's message: a context menu with no point.
        PostMessageW(dialog, WM_CONTEXTMENU, dialog as usize, -1isize as LPARAM);
        PostMessageW(dialog, WM_KEYDOWN, usize::from(VK_ESCAPE), 0);
    });
    super::super::show_keyboard_shortcuts(window.hwnd);
    super::super::save_settings_to(None);
    assert!(
        offered
            .borrow()
            .iter()
            .any(|label| label.starts_with("Reset")),
        "{:?}",
        offered.borrow()
    );
    assert!(app_mut(window.hwnd).settings.key_overrides.is_empty());
}
