//! Window basics: the title, close review, IPC and startup work held by modal loops, tab
//! shortcuts and switching.

use super::*;

#[test]
fn the_main_window_has_a_title_for_the_taskbar() {
    // Break caught: WM_NCCREATE handled without the default processing leaves the window text
    // empty, so the taskbar button and Alt+Tab show only the icon.
    let window = ProductionWindow::new(make_app());
    let mut text = [0u16; 32];
    let len = unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::GetWindowTextW(
            window.hwnd,
            text.as_mut_ptr(),
            text.len() as i32,
        )
    };
    assert_eq!(String::from_utf16_lossy(&text[..len as usize]), "FastPad");
}

#[test]
fn the_main_window_class_resets_the_cursor_over_its_client_area() {
    // Break caught: without a class cursor, WM_SETCURSOR over the tab strip (HTCLIENT) leaves
    // whatever cursor was last shown, such as the editor's I-beam or a resize arrow.
    let window = ProductionWindow::new(make_app());
    let cursor = unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::GetClassLongPtrW(
            window.hwnd,
            windows_sys::Win32::UI::WindowsAndMessaging::GCLP_HCURSOR,
        )
    };
    assert_ne!(cursor, 0);
}

#[test]
fn the_window_title_follows_the_active_tab_and_its_dirty_state() {
    // Break caught: the taskbar button keeps showing a stale or bare title while the tab strip
    // shows which file is open and whether it has unsaved changes.
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    let window_text = || {
        unsafe {
            SendMessageW(window.hwnd, WM_PAINT, 0, 0);
        }
        let mut text = [0u16; 64];
        let len = unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::GetWindowTextW(
                window.hwnd,
                text.as_mut_ptr(),
                text.len() as i32,
            )
        };
        String::from_utf16_lossy(&text[..len as usize])
    };

    assert_eq!(window_text(), "Untitled - FastPad");
    editor.set_text("dirty").unwrap();
    // Notes mode is on by default, so the untitled tab picks up "dirty" as its label.
    assert_eq!(window_text(), "dirty * - FastPad");
    assert_eq!(super::super::window_title(None), "FastPad");
}

#[test]
fn a_forwarded_request_is_handled_before_the_close_review_starts() {
    // Break caught: a launch forwarded just before WM_CLOSE is dropped when the window closes,
    // so the file the user double-clicked silently never opens.
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    editor.set_text("dirty").unwrap();
    app_mut(window.hwnd)
        .ipc_requests
        .push(crate::ipc::IpcRequest::New);
    let tabs_at_prompt = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&tabs_at_prompt);
    answer_next_close_prompt(move |hwnd| {
        observed.store(app_mut(hwnd).tabs.len(), Ordering::SeqCst);
        CloseDecision::Cancel
    });

    unsafe {
        SendMessageW(window.hwnd, WM_CLOSE, 0, 0);
    }

    assert_ne!(
        unsafe { IsWindow(window.hwnd) },
        0,
        "Cancel must abort the close"
    );
    assert_eq!(
        tabs_at_prompt.load(Ordering::SeqCst),
        2,
        "the queued request must be handled before the review starts"
    );
    assert!(app_mut(window.hwnd).ipc_requests.is_empty());
}

#[test]
fn a_confirmed_close_releases_the_pipe_server_and_the_instance_mutex() {
    // Break caught: dropping the instance mutex before the pipe server lets the next launch
    // claim the session and fail to bind a name this process still owns.
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    let names = crate::ipc::server::tests::unique_names();
    app_mut(window.hwnd).instance_mutex = Some(unnamed_mutex());
    super::super::start_ipc_server_with(window.hwnd, || {
        crate::ipc::IpcServer::bind(&names, &crate::ipc::CurrentUserAcl::current()?)
    });
    assert!(app_mut(window.hwnd).ipc.is_some());

    unsafe {
        SendMessageW(window.hwnd, WM_CLOSE, 0, 0);
    }

    assert_eq!(unsafe { IsWindow(window.hwnd) }, 0);
    crate::ipc::IpcServer::bind(&names, &crate::ipc::CurrentUserAcl::current().unwrap())
        .expect("the pipe name must be free once the window has closed");
}

#[test]
fn deferred_startup_work_is_held_until_the_modal_prompt_closes() {
    // Break caught: a nested modal loop dispatches deferred chain units, so recovery can push
    // and activate tabs while a close prompt or file dialog is deciding about another one.
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    let root = RecoveryScratch::new("modal-deferred");
    write_snapshot(
        root.path(),
        &Snapshot::new(
            RecoveryId::from_u128(0x5151),
            None,
            Encoding::Utf8,
            "recovered elsewhere",
        ),
    )
    .unwrap();
    app_mut(window.hwnd).recovery_root = Some(root.path().to_path_buf());
    editor.set_text("dirty").unwrap();
    let before = app_mut(window.hwnd).tabs.len();

    answer_next_close_prompt(|hwnd| {
        unsafe {
            SendMessageW(hwnd, crate::window::WM_FASTPAD_RECOVERY, 0, 0);
        }
        CloseDecision::Cancel
    });
    execute_command(window.hwnd, CommandId::CloseTab);

    assert_eq!(
        app_mut(window.hwnd).tabs.len(),
        before,
        "the recovery unit ran inside the modal loop"
    );

    pump_posted_messages(window.hwnd);

    assert_eq!(
        app_mut(window.hwnd).tabs.len(),
        before + 1,
        "the held recovery unit must run once the modal loop ends"
    );
}

#[test]
fn recovery_never_reopens_a_snapshot_an_open_tab_already_holds() {
    // Break caught: a later Open (every open re-runs the recovery unit) or a session restore
    // opening a second copy of text that is already in a tab.
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    let root = RecoveryScratch::new("claimed");
    write_snapshot(
        root.path(),
        &Snapshot::new(
            RecoveryId::from_u128(0x7171),
            None,
            Encoding::Utf8,
            "held once",
        ),
    )
    .unwrap();
    app_mut(window.hwnd).recovery_root = Some(root.path().to_path_buf());

    unsafe { SendMessageW(window.hwnd, crate::window::WM_FASTPAD_RECOVERY, 0, 0) };
    assert_eq!(app_mut(window.hwnd).tabs.len(), 2);
    unsafe { SendMessageW(window.hwnd, crate::window::WM_FASTPAD_RECOVERY, 0, 0) };
    assert_eq!(app_mut(window.hwnd).tabs.len(), 2);
}

#[test]
fn queued_ipc_requests_wait_for_the_overflow_menu_to_close() {
    // Break caught: a popup menu's own modal loop dispatches a forwarded request, so a
    // new tab becomes active underneath it and the command the user picks acts on that tab
    // instead of the one they opened the menu on.
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    editor.set_text("dirty").unwrap();
    app_mut(window.hwnd)
        .ipc_requests
        .push(crate::ipc::IpcRequest::New);
    let before = app_mut(window.hwnd).tabs.len();

    answer_next_popup_menu(|hwnd| {
        unsafe {
            SendMessageW(hwnd, crate::window::WM_FASTPAD_IPC_REQUEST, 0, 0);
        }
        None
    });
    assert!(crate::window::menus::show_tab_strip_menu(window.hwnd, 0, 0, true).is_none());

    assert_eq!(
        app_mut(window.hwnd).tabs.len(),
        before,
        "a forwarded request was handled inside the popup menu's modal loop"
    );

    pump_posted_messages(window.hwnd);

    assert_eq!(
        app_mut(window.hwnd).tabs.len(),
        before + 1,
        "the held request must run once the overflow menu closes"
    );
}

#[test]
fn closing_every_tab_hides_the_editor_until_the_empty_strip_opens_a_new_one() {
    // Break caught: the last tab being silently replaced (its close button looks inert), the
    // hidden editor still taking edits, or the empty strip's double-click and context menu
    // not reaching New and Close all tabs.
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GWL_STYLE, GetWindowLongPtrW, WM_LBUTTONDBLCLK, WM_RBUTTONUP, WS_VISIBLE,
    };
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    let editor_visible =
        || unsafe { GetWindowLongPtrW(editor.hwnd(), GWL_STYLE) } as u32 & WS_VISIBLE != 0;
    assert!(editor_visible());

    execute_command(window.hwnd, CommandId::CloseTab);

    assert!(app_mut(window.hwnd).tabs.is_empty());
    assert!(!editor_visible());
    execute_command(window.hwnd, CommandId::Paste);
    execute_command(window.hwnd, CommandId::CloseTab);
    assert_eq!(editor.text().unwrap(), "");

    // A double-click on the group's empty strip opens a tab; the caption maximizes instead.
    // The far end of the tab viewport stays empty while two tabs fit.
    let group = super::super::group_hwnd(window.hwnd).unwrap();
    let layout = super::super::strip_layout(window.hwnd).unwrap();
    let strip = client_lparam(layout.tabs.right - 10, layout.height / 2);
    unsafe {
        SendMessageW(group, WM_LBUTTONDBLCLK, 1, strip);
        SendMessageW(group, WM_LBUTTONDBLCLK, 1, strip);
    }
    assert_eq!(app_mut(window.hwnd).tabs.len(), 2);
    assert!(editor_visible());

    answer_next_popup_menu(|_| Some(CommandId::CloseAllTabs));
    unsafe {
        SendMessageW(group, WM_RBUTTONUP, 0, strip);
    }
    assert!(app_mut(window.hwnd).tabs.is_empty());
    assert!(!editor_visible());
}

#[test]
fn keyboard_shortcuts_reach_their_commands_through_the_accelerator_table() {
    // Break caught: every shortcut dead in the running app while command-level tests pass,
    // because nothing exercised the accelerator translation the message loop depends on.
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetKeyboardState, SetKeyboardState, VK_CONTROL,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{MSG, WM_KEYDOWN};
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    let identity = unsafe { super::super::window_identity(window.hwnd).unwrap() };
    let mut keys = [0u8; 256];
    unsafe { GetKeyboardState(keys.as_mut_ptr()) };
    let original = keys;
    keys[VK_CONTROL as usize] = 0x80;
    unsafe { SetKeyboardState(keys.as_ptr()) };
    let message = MSG {
        hwnd: editor.hwnd(),
        message: WM_KEYDOWN,
        wParam: usize::from(b'T'),
        ..Default::default()
    };
    let translated =
        unsafe { super::super::translate_accelerator(window.hwnd, &identity, &message) };
    unsafe { SetKeyboardState(original.as_ptr()) };

    assert!(translated, "Ctrl+T was not translated");
    assert_eq!(app_mut(window.hwnd).tabs.len(), 2);
}

#[test]
fn tab_shortcuts_cycle_with_wrap_around_and_select_by_position() {
    // Break caught: Ctrl+Tab stopping at the last tab, or Ctrl+9 with fewer than nine tabs
    // activating some other tab instead of doing nothing.
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    execute_command(window.hwnd, CommandId::New);
    execute_command(window.hwnd, CommandId::New);
    let active = || app_mut(window.hwnd).tabs.active_index();
    assert_eq!(active(), 2);

    execute_command(window.hwnd, CommandId::NextTab);
    assert_eq!(active(), 0);
    execute_command(window.hwnd, CommandId::PreviousTab);
    assert_eq!(active(), 2);
    execute_command(window.hwnd, CommandId::PreviousTab);
    assert_eq!(active(), 1);
    execute_command(window.hwnd, CommandId::SelectTab1);
    assert_eq!(active(), 0);
    execute_command(window.hwnd, CommandId::SelectTab3);
    assert_eq!(active(), 2);
    execute_command(window.hwnd, CommandId::SelectTab9);
    assert_eq!(active(), 2);
}

#[test]
fn switching_tabs_restores_each_tabs_caret_and_scroll() {
    // Break caught: switching tabs resetting the caret and scroll to the start of the document.
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    editor.set_text(&"line\n".repeat(2000)).unwrap();
    let saved = crate::editor::ViewState {
        caret: 1210 * 5 + 2,
        anchor: 1210 * 5,
        first_line: 1200,
        x_offset: 0,
    };
    editor.apply_view_state(saved).unwrap();
    execute_command(window.hwnd, CommandId::New);
    editor.set_text("second").unwrap();
    assert_eq!(editor.view_state().unwrap().first_line, 0);

    execute_command(window.hwnd, CommandId::SelectTab1);
    assert_eq!(editor.view_state().unwrap(), saved);
}

#[test]
fn replacing_in_a_background_tab_leaves_the_active_view_alone() {
    // Break caught: a background replace swapping the document through the visible editor,
    // so the active tab's caret, selection direction or scroll jumps.
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    editor.set_text(&"line\n".repeat(2000)).unwrap();
    let first = app_mut(window.hwnd).tabs.active().unwrap().id;
    execute_command(window.hwnd, CommandId::New);
    editor.set_text("foo foo").unwrap();
    let second = app_mut(window.hwnd).tabs.active().unwrap().id;
    execute_command(window.hwnd, CommandId::SelectTab1);
    let shown = crate::editor::ViewState {
        caret: 1210 * 5,
        anchor: 1210 * 5 + 3,
        first_line: 1200,
        x_offset: 0,
    };
    editor.apply_view_state(shown).unwrap();

    let matcher = crate::search::Matcher::new("foo", Default::default()).unwrap();
    assert_eq!(
        super::super::replace_in_document(window.hwnd, second, &matcher, "bar"),
        Some(2)
    );

    assert_eq!(app_mut(window.hwnd).tabs.active().unwrap().id, first);
    assert_eq!(editor.view_state().unwrap(), shown);
    assert_eq!(
        super::super::document_text(window.hwnd, second).unwrap(),
        "bar bar"
    );
    assert!(app_mut(window.hwnd).tabs.document(second).unwrap().dirty);
}

#[test]
fn the_primary_window_saves_where_it_closed_and_reopens_there() {
    // Break caught: FastPad opening at the default spot after being closed elsewhere, reopening
    // a maximized window restored, or a `--new-window` window overwriting the primary's spot.
    use crate::config::WindowPlacement;
    use windows_sys::Win32::UI::WindowsAndMessaging::{SW_SHOW, SW_SHOWMAXIMIZED};
    let _scintilla = load_native_scintilla();
    let scratch = RecoveryScratch::new("window-placement");
    let ini = scratch.path().join("fastpad.ini");
    std::fs::write(&ini, "# kept\r\n").unwrap();
    super::super::save_settings_to(Some(ini.clone()));
    let placed = WindowPlacement {
        x: 120,
        y: 90,
        width: 900,
        height: 600,
        maximized: false,
    };
    let close_at = |placement: WindowPlacement, primary: bool| {
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        if primary {
            app_mut(window.hwnd).instance_mutex = Some(unnamed_mutex());
        }
        let show = super::super::restore_placement(window.hwnd, Some(placement));
        let expected = if placement.maximized {
            SW_SHOWMAXIMIZED
        } else {
            SW_SHOW
        };
        assert_eq!(show, expected);
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::ShowWindow(window.hwnd, show);
            SendMessageW(window.hwnd, WM_CLOSE, 0, 0);
        }
        assert_eq!(unsafe { IsWindow(window.hwnd) }, 0);
        std::fs::read_to_string(&ini).unwrap()
    };

    assert_eq!(
        close_at(placed, true),
        "# kept\r\nwindow_placement=120,90,900x600\r\n"
    );
    let maximized = WindowPlacement {
        maximized: true,
        ..placed
    };
    assert_eq!(
        close_at(maximized, true),
        "# kept\r\nwindow_placement=120,90,900x600,maximized\r\n",
        "maximized keeps the restored frame under it"
    );
    let elsewhere = WindowPlacement { x: 200, ..placed };
    assert_eq!(
        close_at(elsewhere, false),
        "# kept\r\nwindow_placement=120,90,900x600,maximized\r\n",
        "a window that is not the primary saves nothing"
    );
    super::super::save_settings_to(None);
}

#[test]
fn a_placement_on_no_monitor_leaves_the_window_where_it_was_created() {
    // Break caught: FastPad reopening off screen after the monitor it closed on was unplugged.
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetWindowRect, SW_SHOW};
    let window = ProductionWindow::new(make_app());
    let frame = || {
        let mut rect = RECT::default();
        unsafe { GetWindowRect(window.hwnd, &mut rect) };
        (rect.left, rect.top, rect.right, rect.bottom)
    };
    let before = frame();
    let gone = crate::config::WindowPlacement {
        x: -60_000,
        y: -60_000,
        width: 900,
        height: 600,
        maximized: true,
    };

    assert_eq!(
        super::super::restore_placement(window.hwnd, Some(gone)),
        SW_SHOW
    );
    assert_eq!(super::super::restore_placement(window.hwnd, None), SW_SHOW);
    assert_eq!(frame(), before);
}
