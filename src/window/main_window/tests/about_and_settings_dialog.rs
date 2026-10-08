//! The About box, the Settings dialog and key rebinding.

use super::*;

#[test]
fn about_shows_a_modal_window_over_the_disabled_main_window_until_escape() {
    // Break caught: About doing nothing, leaving the main window usable behind the box (or
    // disabled after it closes), or a modal scope that never ends, which holds back every
    // deferred message for the rest of the session; or one with no native frame (no DWM
    // shadow) or a visible one, as for Settings.
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{IsWindowEnabled, VK_ESCAPE};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GW_OWNER, GWL_STYLE, GetClientRect, GetWindow, GetWindowLongW, GetWindowRect, HTCAPTION,
        HTCLIENT, PostMessageW, SendMessageW, WM_KEYDOWN, WM_NCHITTEST, WS_CAPTION,
    };
    let window = ProductionWindow::new(make_app());
    let owner = window.hwnd;
    let shown = std::rc::Rc::new(std::cell::Cell::new(None));
    let seen = shown.clone();
    crate::window::about::answer_next(move |dialog| {
        let owned = unsafe { GetWindow(dialog, GW_OWNER) } == owner;
        let owner_disabled = unsafe { IsWindowEnabled(owner) } == 0;
        let modal = crate::window::modal::modal_active(owner);
        // A hidden native frame: WS_CAPTION earns the DWM shadow, WM_NCCALCSIZE leaves no
        // visible frame, so the client is the whole window.
        let style = unsafe { GetWindowLongW(dialog, GWL_STYLE) } as u32;
        let (mut client, mut frame) = (RECT::default(), RECT::default());
        unsafe {
            GetClientRect(dialog, &mut client);
            GetWindowRect(dialog, &mut frame);
        }
        let framed = style & WS_CAPTION == WS_CAPTION
            && client.right - client.left == frame.right - frame.left
            && client.bottom - client.top == frame.bottom - frame.top
            && client.right > 0;
        // The header still drags the box; its × and the body below do not.
        let hit = |x: i32, y: i32| unsafe {
            SendMessageW(
                dialog,
                WM_NCHITTEST,
                0,
                ((u32::from(y as u16) << 16) | u32::from(x as u16)) as LPARAM,
            )
        };
        let framed = framed
            && hit(frame.left + 30, frame.top + 10) == HTCAPTION as LRESULT
            && hit(frame.right - 5, frame.top + 10) == HTCLIENT as LRESULT
            && hit(frame.left + 5, frame.bottom - 5) == HTCLIENT as LRESULT;
        seen.set(Some((dialog, owned, owner_disabled, modal, framed)));
        unsafe { PostMessageW(dialog, WM_KEYDOWN, usize::from(VK_ESCAPE), 0) };
    });

    execute_command(owner, CommandId::About);

    let (dialog, owned, owner_disabled, modal, framed) =
        shown.get().expect("the About box was shown");
    assert!(owned, "owned by the main window");
    assert!(
        framed,
        "WS_CAPTION, a client rect the size of the window, and the header still a caption"
    );
    assert!(
        owner_disabled,
        "the main window is disabled while About is up"
    );
    assert!(modal, "About runs inside a modal scope");
    assert_eq!(unsafe { IsWindow(dialog) }, 0, "Escape closed it");
    assert_ne!(
        unsafe { IsWindowEnabled(owner) },
        0,
        "the main window is usable again"
    );
    assert!(!crate::window::modal::modal_active(owner));
}

#[test]
fn about_opens_the_repository_link_from_the_keyboard_and_stays_open() {
    // Break caught: a link that Tab can't reach or Enter doesn't follow, or following a link
    // that also closes the box.
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_ESCAPE, VK_RETURN, VK_TAB};
    use windows_sys::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_KEYDOWN};
    let window = ProductionWindow::new(make_app());
    crate::window::about::take_opened_urls();
    crate::window::about::answer_next(move |dialog| unsafe {
        PostMessageW(dialog, WM_KEYDOWN, usize::from(VK_TAB), 0);
        PostMessageW(dialog, WM_KEYDOWN, usize::from(VK_RETURN), 0);
        PostMessageW(dialog, WM_KEYDOWN, usize::from(VK_ESCAPE), 0);
    });

    execute_command(window.hwnd, CommandId::About);

    assert_eq!(
        crate::window::about::take_opened_urls(),
        [crate::window::about::Link::Repository.url()]
    );
}

#[test]
fn settings_opens_an_owned_modal_dialog_that_escape_closes() {
    // Break caught: a dialog that can hide behind the main window, leaves it disabled after
    // closing, or never ends its modal scope; or one with no native frame (no DWM shadow)
    // or a visible one.
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{IsWindowEnabled, VK_ESCAPE};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GW_OWNER, GWL_STYLE, GetClientRect, GetWindow, GetWindowLongW, GetWindowRect, HTCAPTION,
        HTCLIENT, HTRIGHT, PostMessageW, SendMessageW, WM_KEYDOWN, WM_NCHITTEST, WS_CAPTION,
    };
    let window = ProductionWindow::new(make_app());
    let owner = window.hwnd;
    let shown = std::rc::Rc::new(std::cell::Cell::new(None));
    let seen = shown.clone();
    crate::window::settings_dialog::answer_next(move |dialog| {
        let owned = unsafe { GetWindow(dialog, GW_OWNER) } == owner;
        let owner_disabled = unsafe { IsWindowEnabled(owner) } == 0;
        let modal = crate::window::modal::modal_active(owner)
            && crate::window::settings_dialog::open_dialog(owner) == Some(dialog);
        // A hidden native frame: WS_CAPTION earns the DWM shadow, WM_NCCALCSIZE leaves no
        // visible frame, so the client is the whole window.
        let style = unsafe { GetWindowLongW(dialog, GWL_STYLE) } as u32;
        let (mut client, mut frame) = (RECT::default(), RECT::default());
        unsafe {
            GetClientRect(dialog, &mut client);
            GetWindowRect(dialog, &mut frame);
        }
        let framed = style & WS_CAPTION == WS_CAPTION
            && client.right - client.left == frame.right - frame.left
            && client.bottom - client.top == frame.bottom - frame.top
            && client.right > 0;
        // The title row still drags the dialog; its × does not; the hidden frame's edges
        // still size it.
        let hit = |x: i32, y: i32| unsafe {
            SendMessageW(
                dialog,
                WM_NCHITTEST,
                0,
                ((u32::from(y as u16) << 16) | u32::from(x as u16)) as LPARAM,
            )
        };
        let framed = framed
            && hit(frame.left + 30, frame.top + 10) == HTCAPTION as LRESULT
            && hit(frame.right - 20, frame.top + 20) == HTCLIENT as LRESULT
            && hit(frame.right - 1, (frame.top + frame.bottom) / 2) == HTRIGHT as LRESULT;
        seen.set(Some((dialog, owned, owner_disabled, modal, framed)));
        unsafe { PostMessageW(dialog, WM_KEYDOWN, usize::from(VK_ESCAPE), 0) };
    });

    super::super::show_settings(owner);

    let (dialog, owned, owner_disabled, modal, framed) = shown.get().expect("Settings was shown");
    assert!(owned && owner_disabled && modal);
    assert!(
        framed,
        "WS_CAPTION, a client rect the size of the window, and the title row still a caption"
    );
    assert_eq!(unsafe { IsWindow(dialog) }, 0, "Escape closed it");
    assert_eq!(crate::window::settings_dialog::open_dialog(owner), None);
    assert_ne!(unsafe { IsWindowEnabled(owner) }, 0);
    assert!(!crate::window::modal::modal_active(owner));
}

#[test]
fn rebinding_save_rebuilds_the_accelerator_table_and_saves_one_line() {
    // Break caught: a new key saved but the old table still dispatching, or Reset leaving a
    // `key.file.save=` line that unbinds Save on the next start.
    use crate::window::keymap::KeyStroke;
    use windows_sys::Win32::UI::WindowsAndMessaging::{FALT, FCONTROL, FVIRTKEY};
    let scratch = RecoveryScratch::new("keymap-rebind");
    let ini = scratch.path().join("fastpad.ini");
    std::fs::write(&ini, "# kept\r\n").unwrap();
    super::super::save_settings_to(Some(ini.clone()));
    let window = ProductionWindow::new(make_app());
    let save_entries = |hwnd| {
        app_mut(hwnd)
            .accelerators
            .as_ref()
            .unwrap()
            .entries()
            .into_iter()
            .filter(|entry| entry.cmd == CommandId::Save as u16)
            .map(|entry| (entry.fVirt, entry.key))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        save_entries(window.hwnd),
        [(FVIRTKEY | FCONTROL, u16::from(b'S'))]
    );

    super::super::set_command_keys(
        window.hwnd,
        CommandId::Save,
        vec![KeyStroke::parse("Ctrl+Alt+S").unwrap()],
    );
    assert_eq!(
        save_entries(window.hwnd),
        [(FVIRTKEY | FCONTROL | FALT, u16::from(b'S'))]
    );
    assert_eq!(
        std::fs::read_to_string(&ini).unwrap(),
        "# kept\r\nkey.file.save=Ctrl+Alt+S\r\n"
    );
    assert_eq!(
        app_mut(window.hwnd)
            .settings
            .key_overrides
            .get("file.save")
            .map(String::as_str),
        Some("Ctrl+Alt+S")
    );

    super::super::reset_command_keys(window.hwnd, CommandId::Save);
    assert_eq!(
        save_entries(window.hwnd),
        [(FVIRTKEY | FCONTROL, u16::from(b'S'))]
    );
    assert_eq!(std::fs::read_to_string(&ini).unwrap(), "# kept\r\n");
    assert!(app_mut(window.hwnd).settings.key_overrides.is_empty());

    // Keys equal to the defaults are a reset too.
    super::super::set_command_keys(window.hwnd, CommandId::Save, vec![]);
    assert_eq!(
        std::fs::read_to_string(&ini).unwrap(),
        "# kept\r\nkey.file.save=\r\n"
    );
    super::super::set_command_keys(
        window.hwnd,
        CommandId::Save,
        vec![KeyStroke::parse("Ctrl+S").unwrap()],
    );
    assert_eq!(std::fs::read_to_string(&ini).unwrap(), "# kept\r\n");
    super::super::save_settings_to(None);
}

#[test]
fn resetting_a_command_removes_an_ignored_key_line() {
    // Break caught: `key.file.save=Bogus` is ignored at load, so Reset saw an unchanged
    // keymap, returned early and left the line (and its warning) forever.
    let scratch = RecoveryScratch::new("keymap-stale");
    let ini = scratch.path().join("fastpad.ini");
    std::fs::write(
        &ini,
        "# kept
key.file.save=Bogus
",
    )
    .unwrap();
    super::super::save_settings_to(Some(ini.clone()));
    let window = ProductionWindow::new(make_app());
    let mut settings = crate::config::default_settings();
    settings
        .key_overrides
        .insert("file.save".into(), "Bogus".into());
    super::super::apply_loaded_settings(window.hwnd, settings, Vec::new());
    assert!(!app_mut(window.hwnd).keymap.is_user(CommandId::Save));

    super::super::reset_command_keys(window.hwnd, CommandId::Save);
    assert_eq!(
        std::fs::read_to_string(&ini).unwrap(),
        "# kept
"
    );
    assert!(app_mut(window.hwnd).settings.key_overrides.is_empty());
    super::super::save_settings_to(None);
}

#[test]
fn loaded_key_overrides_rebuild_the_table_and_warn_about_bad_lines() {
    // Break caught: `key.` lines read but never applied, or an unknown command dropped
    // without telling the user.
    let window = ProductionWindow::new(make_app());
    let mut settings = crate::config::default_settings();
    settings
        .key_overrides
        .insert("search.find".into(), "F9".into());
    settings
        .key_overrides
        .insert("nope.command".into(), "F8".into());
    super::super::apply_loaded_settings(window.hwnd, settings, Vec::new());
    let app = app_mut(window.hwnd);
    assert!(app.keymap.is_user(CommandId::Find));
    assert!(
        app.accelerators
            .as_ref()
            .unwrap()
            .entries()
            .iter()
            .any(|entry| {
                entry.cmd == CommandId::Find as u16
                    && entry.key == windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_F9
            })
    );
    assert!(
        app.notifications
            .pending()
            .iter()
            .any(|notification| notification.message.contains("key.nope.command"))
    );
}

#[test]
fn the_settings_dialog_changes_settings_from_the_keyboard() {
    // Break caught: arrows or Space that change nothing, a typed font size lost when Tab
    // leaves the field, or changes that aren't saved (settings dialog spec §3.3).
    use crate::config::FileIconSet;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        VK_ESCAPE, VK_RIGHT, VK_SPACE, VK_TAB, VK_UP,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_CHAR, WM_KEYDOWN};
    let _scintilla = load_native_scintilla();
    let scratch = RecoveryScratch::new("settings-dialog-keys");
    let ini = scratch.path().join("fastpad.ini");
    std::fs::write(&ini, "# kept\r\n").unwrap();
    super::super::save_settings_to(Some(ini.clone()));
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    crate::window::settings_dialog::answer_next(|dialog| unsafe {
        let key = |vk: u16| PostMessageW(dialog, WM_KEYDOWN, usize::from(vk), 0);
        let char = |c: char| PostMessageW(dialog, WM_CHAR, c as usize, 0);
        key(VK_TAB); // File icons
        key(VK_RIGHT); // Minimal
        key(VK_TAB); // Font
        key(VK_TAB); // Font size
        char('1');
        char('6');
        key(VK_TAB); // commits 16; Preview font
        key(VK_TAB); // Preview font size
        key(VK_UP); // 14 -> 15
        key(VK_TAB); // Preview line height
        key(VK_UP); // 1.6 -> 1.7
        key(VK_TAB); // Tab width
        key(VK_RIGHT); // 4 → 8
        key(VK_TAB); // Indent with spaces
        key(VK_SPACE);
        key(VK_ESCAPE);
    });

    super::super::show_settings(window.hwnd);

    let settings = app_mut(window.hwnd).settings.clone();
    assert_eq!(settings.file_icons, FileIconSet::Minimal);
    assert_eq!(settings.font_size, 16);
    assert_eq!(settings.preview_font_size, 15);
    assert_eq!(settings.preview_line_height, 17);
    assert_eq!(settings.tab_width, 8);
    assert!(settings.insert_spaces);
    super::super::save_settings_to(None);
    assert_eq!(
        std::fs::read_to_string(&ini).unwrap(),
        "# kept\r\nfile_icons=minimal\r\nfont_size=16\r\npreview_font_size=15\r\n\
         preview_line_height=1.7\r\ntab_width=8\r\ninsert_spaces=true\r\n"
    );
}

#[test]
fn the_theme_dropdown_opens_with_enter_and_picks_with_the_keyboard() {
    // Break caught: a dropdown that opens but ignores the arrows, or picks without applying.
    use crate::config::ThemePreference;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_DOWN, VK_ESCAPE, VK_RETURN};
    use windows_sys::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_KEYDOWN};
    let scratch = RecoveryScratch::new("settings-dialog-theme");
    super::super::save_settings_to(Some(scratch.path().join("fastpad.ini")));
    let window = ProductionWindow::new(make_app());
    crate::window::settings_dialog::answer_next(|dialog| unsafe {
        let key = |vk: u16| PostMessageW(dialog, WM_KEYDOWN, usize::from(vk), 0);
        key(VK_RETURN); // opens the Theme list on System
        key(VK_DOWN); // Light
        key(VK_RETURN); // picks it and closes the list
        key(VK_ESCAPE); // closes the dialog
    });

    super::super::show_settings(window.hwnd);

    assert_eq!(app_mut(window.hwnd).settings.theme, ThemePreference::Light);
    super::super::save_settings_to(None);
}

#[test]
fn up_and_down_step_the_focused_theme_dropdown_without_opening_it() {
    // Break caught: arrows that only work once the list is open, or a step that wraps past
    // the first item (dropdown arrows brief).
    use crate::config::ThemePreference;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_DOWN, VK_ESCAPE, VK_UP};
    use windows_sys::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_KEYDOWN};
    let scratch = RecoveryScratch::new("settings-dialog-theme-arrows");
    let ini = scratch.path().join("fastpad.ini");
    super::super::save_settings_to(Some(ini.clone()));
    let window = ProductionWindow::new(make_app());
    crate::window::settings_dialog::answer_next(|dialog| unsafe {
        let key = |vk: u16| PostMessageW(dialog, WM_KEYDOWN, usize::from(vk), 0);
        // The Theme row has the focus, on System, the first item.
        key(VK_UP); // clamped: still System
        key(VK_DOWN); // Light
        key(VK_DOWN); // Dark
        key(VK_UP); // Light
        key(VK_ESCAPE);
        // Break caught: were a step to open the list, the first Escape would close only the
        // list and show_settings would never return; this one closes the dialog, so the
        // assertions below fail instead.
        key(VK_ESCAPE);
    });

    super::super::show_settings(window.hwnd);

    assert_eq!(app_mut(window.hwnd).settings.theme, ThemePreference::Light);
    super::super::save_settings_to(None);
    assert_eq!(
        std::fs::read_to_string(&ini).unwrap(),
        "theme=light\n",
        "each step rewrote the one theme line"
    );
}

#[test]
fn the_dialog_keeps_the_focus_after_a_change_that_moves_it() {
    // Break caught: switching notes mode off from the dialog tears down the sidebar, the
    // focus lands in the main window, and the dialog stops answering the keyboard (review
    // focus 1). Posted test keys reach the dialog whatever the focus, so the dialog records
    // the focus after each change and the test checks that record.
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_ESCAPE, VK_SPACE, VK_TAB};
    use windows_sys::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_KEYDOWN};
    let _scintilla = load_native_scintilla();
    let scratch = RecoveryScratch::new("settings-dialog-focus");
    super::super::save_settings_to(Some(scratch.path().join("fastpad.ini")));
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    crate::window::settings_dialog::take_focus_checks();
    crate::window::settings_dialog::answer_next(|dialog| unsafe {
        let key = |vk: u16| PostMessageW(dialog, WM_KEYDOWN, usize::from(vk), 0);
        // Theme → … → Notes mode is the 11th row: ten Tabs.
        for _ in 0..10 {
            key(VK_TAB);
        }
        key(VK_SPACE); // notes mode off
        key(VK_SPACE); // and on again
        key(VK_ESCAPE);
    });

    super::super::show_settings(window.hwnd);

    assert!(app_mut(window.hwnd).settings.notes_mode, "both toggles ran");
    assert_eq!(
        crate::window::settings_dialog::take_focus_checks(),
        [true, true],
        "the dialog had the keyboard after each change"
    );
    super::super::save_settings_to(None);
}

#[test]
fn edit_fastpad_ini_closes_the_dialog_and_opens_the_file_in_a_tab() {
    // Break caught: the link doing nothing when fastpad.ini doesn't exist yet, or opening it
    // under the still-modal dialog (settings dialog spec §3.6).
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_RETURN, VK_TAB};
    use windows_sys::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_KEYDOWN};
    let _scintilla = load_native_scintilla();
    let scratch = RecoveryScratch::new("settings-dialog-edit-ini");
    let ini = scratch.path().join("FastPad").join("fastpad.ini");
    super::super::save_settings_to(Some(ini.clone()));
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    crate::window::settings_dialog::answer_next(|dialog| unsafe {
        // 16 enabled rows (no notebook, so autosave is skipped): 16 Tabs reach the link.
        for _ in 0..16 {
            PostMessageW(dialog, WM_KEYDOWN, usize::from(VK_TAB), 0);
        }
        PostMessageW(dialog, WM_KEYDOWN, usize::from(VK_RETURN), 0);
    });

    super::super::show_settings(window.hwnd);

    assert!(ini.exists(), "created when missing");
    assert!(
        app_mut(window.hwnd).tabs.find_path(&ini).is_some(),
        "opened in a tab"
    );
    // Break caught: a hand edit saved there looking ignored because FastPad reads the file
    // only at startup, with nothing saying so (final review 5).
    assert!(
        app_mut(window.hwnd)
            .notifications
            .pending()
            .iter()
            .any(|notice| notice.message == super::super::EDIT_INI_NOTICE),
        "the notice says when hand edits apply"
    );
    super::super::save_settings_to(None);
}

/// Point `(x, y)` packed as a mouse message's `lparam`.
fn settings_click_at(x: i32, y: i32) -> LPARAM {
    (x as u16 as usize | ((y as u16 as usize) << 16)) as LPARAM
}

/// The center of `row` in the open Settings dialog's client area, before any scrolling.
fn settings_row_center(dialog: HWND, row: crate::window::settings_model::Row) -> (i32, i32) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GW_OWNER, GetWindow};
    let owner = unsafe { GetWindow(dialog, GW_OWNER) };
    let dpi = unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForWindow(owner) }.max(96);
    let layout = crate::window::settings_dialog::Layout::calculate(dpi, i32::MAX, i32::MAX, 100);
    let rect = layout.row_rect(row, 0);
    ((rect.left + rect.right) / 2, (rect.top + rect.bottom) / 2)
}

#[test]
fn the_second_click_of_a_double_click_on_a_dropdown_item_changes_nothing_else() {
    // Break caught: double-clicking a theme in the list picks it on the first click, the
    // list goes, and the second click lands on the Word wrap row underneath and toggles it
    // (final review 3). A later click still toggles it: only one press is swallowed.
    use crate::config::ThemePreference;
    use crate::window::settings_model::Row;
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::Graphics::Gdi::ClientToScreen;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_ESCAPE, VK_RETURN};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        PostMessageW, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP,
    };
    let _scintilla = load_native_scintilla();
    let scratch = RecoveryScratch::new("settings-dialog-double-click");
    super::super::save_settings_to(Some(scratch.path().join("fastpad.ini")));
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    assert!(!app_mut(window.hwnd).settings.word_wrap);
    crate::window::settings_dialog::answer_next(|dialog| unsafe {
        let (x, y) = settings_row_center(dialog, Row::WordWrap);
        let mut screen = POINT { x, y };
        ClientToScreen(dialog, &mut screen);
        PostMessageW(dialog, WM_KEYDOWN, usize::from(VK_RETURN), 0); // the Theme list
        // The list's first click picks Light at a point over the Word wrap row.
        PostMessageW(
            dialog,
            crate::window::dropdown_list::WM_LIST_PICKED,
            1,
            settings_click_at(screen.x, screen.y),
        );
        for _ in 0..2 {
            PostMessageW(dialog, WM_LBUTTONDOWN, 1, settings_click_at(x, y));
            PostMessageW(dialog, WM_LBUTTONUP, 0, settings_click_at(x, y));
        }
        PostMessageW(dialog, WM_KEYDOWN, usize::from(VK_ESCAPE), 0);
    });

    super::super::show_settings(window.hwnd);

    let settings = app_mut(window.hwnd).settings.clone();
    assert_eq!(settings.theme, ThemePreference::Light, "the pick applied");
    assert!(settings.word_wrap, "toggled once: by the later click only");
    super::super::save_settings_to(None);
}

#[test]
fn clicking_the_greyed_autosave_row_leaves_the_focus_where_it_was() {
    // Break caught: a click on the Notebook autosave row, greyed with no notebook open,
    // moving the focus onto a row the keyboard can't use (final review 6). Tab and Right
    // after the click then change File icons, the row after Theme.
    use crate::config::FileIconSet;
    use crate::window::settings_model::Row;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_ESCAPE, VK_RIGHT, VK_TAB};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        PostMessageW, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP,
    };
    let scratch = RecoveryScratch::new("settings-dialog-greyed-row");
    super::super::save_settings_to(Some(scratch.path().join("fastpad.ini")));
    let window = ProductionWindow::new(make_app());
    crate::window::settings_dialog::answer_next(|dialog| unsafe {
        let (x, y) = settings_row_center(dialog, Row::NotebookAutosave);
        PostMessageW(dialog, WM_LBUTTONDOWN, 1, settings_click_at(x, y));
        PostMessageW(dialog, WM_LBUTTONUP, 0, settings_click_at(x, y));
        PostMessageW(dialog, WM_KEYDOWN, usize::from(VK_TAB), 0);
        PostMessageW(dialog, WM_KEYDOWN, usize::from(VK_RIGHT), 0);
        PostMessageW(dialog, WM_KEYDOWN, usize::from(VK_ESCAPE), 0);
    });

    super::super::show_settings(window.hwnd);

    assert_eq!(
        app_mut(window.hwnd).settings.file_icons,
        FileIconSet::Minimal
    );
    super::super::save_settings_to(None);
}
