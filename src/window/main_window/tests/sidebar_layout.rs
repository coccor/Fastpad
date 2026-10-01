//! The sidebar's layout, caption areas, logo and resizing.

use super::*;

/// Moves the pointer onto the activity bar, which makes its tooltip.
fn hover_bar(bar: HWND) {
    unsafe {
        SendMessageW(
            bar,
            windows_sys::Win32::UI::WindowsAndMessaging::WM_MOUSEMOVE,
            0,
            client_lparam(10, 60),
        );
    }
}

fn button_center(hwnd: HWND, button: crate::window::activity_bar::ActivityButton) -> (i32, i32) {
    let (bar, _) = sidebar_windows(hwnd);
    let (width, height) = client_size(bar);
    let client = RECT {
        left: 0,
        top: 0,
        right: width,
        bottom: height,
    };
    let dpi = unsafe { GetDpiForWindow(bar) }.max(96);
    let rect = crate::window::activity_bar::button_rects(client, dpi)[button.index()];
    ((rect.left + rect.right) / 2, (rect.top + rect.bottom) / 2)
}

/// Resizes the window so its client area is `client_width` wide.
fn set_client_width(hwnd: HWND, client_width: i32) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowRect, SWP_NOMOVE, SWP_NOZORDER, SetWindowPos,
    };
    let mut frame = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut frame) };
    let border = (frame.right - frame.left) - client_size(hwnd).0;
    unsafe {
        SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            0,
            0,
            client_width + border,
            frame.bottom - frame.top,
            SWP_NOMOVE | SWP_NOZORDER,
        );
    }
}

/// A scratch `fastpad.ini` holding only a comment, which settings saves go to.
fn settings_scratch(label: &str) -> (RecoveryScratch, PathBuf) {
    let scratch = RecoveryScratch::new(label);
    let ini = scratch.path().join("fastpad.ini");
    std::fs::write(&ini, "# kept\r\n").unwrap();
    super::super::save_settings_to(Some(ini.clone()));
    (scratch, ini)
}

#[test]
fn with_notes_mode_off_there_is_no_sidebar_and_nothing_moves() {
    // Break caught: an activity bar, or a gap where it would be, with notes mode off, where
    // the layout must stay exactly what it was before the sidebar existed.
    let _scintilla = load_native_scintilla();
    let mut app = make_app();
    app.settings.notes_mode = false;
    let window = ProductionWindow::new(app);
    let editor = install_test_editor(&window);
    assert!(app_mut(window.hwnd).sidebar.is_none());
    assert_eq!(crate::window::side_panel::left_edge(window.hwnd), 0);
    assert_eq!(
        left_of(super::super::group_hwnd(window.hwnd).unwrap(), window.hwnd),
        0
    );
    assert_eq!(left_of(editor.hwnd(), window.hwnd), 0);
    assert_eq!(
        crate::window::side_panel::current_view(window.hwnd),
        crate::config::SidebarView::Hidden
    );
    execute_command(window.hwnd, CommandId::ToggleSidebar);
    assert!(app_mut(window.hwnd).sidebar.is_none());
    execute_command(window.hwnd, CommandId::CommandPalette);
    assert!(
        app_mut(window.hwnd)
            .command_palette
            .as_ref()
            .unwrap()
            .shown()
            .iter()
            .all(|entry| !entry.command.is_sidebar())
    );
    super::super::close_command_palette(window.hwnd, false);

    app_mut(window.hwnd).settings.notes_mode = true;
    crate::window::side_panel::notes_mode_changed(window.hwnd, true);
    let (bar, _) = sidebar_windows(window.hwnd);
    hover_bar(bar);
    let tip = app_mut(window.hwnd)
        .sidebar
        .as_ref()
        .and_then(|sidebar| sidebar.tooltip)
        .expect("the activity bar has a tooltip")
        .hwnd();
    let left = crate::window::side_panel::left_edge(window.hwnd);
    assert!(left > 0);
    assert_eq!(left_of(editor.hwnd(), window.hwnd), left);

    app_mut(window.hwnd).settings.notes_mode = false;
    crate::window::side_panel::notes_mode_changed(window.hwnd, false);
    assert_eq!(unsafe { IsWindow(bar) }, 0);
    // Break caught: a tooltip left alive (owned by the main window, not the bar) each time
    // notes mode goes off.
    assert_eq!(unsafe { IsWindow(tip) }, 0);
    assert_eq!(crate::window::side_panel::left_edge(window.hwnd), 0);
    assert_eq!(left_of(editor.hwnd(), window.hwnd), 0);
}

#[test]
fn the_sidebar_takes_the_left_edge_and_everything_else_starts_right_of_it() {
    // Break caught: tabs, the find bar, the menu band or the editor still starting at x = 0,
    // under the activity bar and panel.
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    let (bar, panel) = sidebar_windows(window.hwnd);
    let dpi = unsafe { GetDpiForWindow(window.hwnd) }.max(96);
    let (width, height) = client_size(window.hwnd);
    let saved = app_mut(window.hwnd).settings.sidebar_width;
    let (activity, panel_width) =
        crate::window::side_panel::sidebar_widths(width, dpi, true, saved);
    assert_eq!(client_size(bar), (activity, height));
    assert_eq!(client_size(panel), (panel_width, height));
    assert_eq!(left_of(panel, window.hwnd), activity);
    let left = activity + panel_width;
    assert_eq!(crate::window::side_panel::left_edge(window.hwnd), left);
    assert_eq!(
        left_of(super::super::group_hwnd(window.hwnd).unwrap(), window.hwnd),
        left
    );
    assert_eq!(left_of(editor.hwnd(), window.hwnd), left);
    assert_eq!(client_size(editor.hwnd()).0, width - left);
    // Nothing before the pointer needs the tooltip, so the first frame goes without it.
    assert!(
        app_mut(window.hwnd)
            .sidebar
            .as_ref()
            .unwrap()
            .tooltip
            .is_none()
    );
    hover_bar(bar);
    assert_eq!(
        app_mut(window.hwnd)
            .sidebar
            .as_ref()
            .unwrap()
            .tooltip
            .unwrap()
            .tool_count(),
        4
    );
    // An empty text removes a tool instead of showing an empty tip.
    let tooltip = app_mut(window.hwnd)
        .sidebar
        .as_ref()
        .unwrap()
        .tooltip
        .unwrap();
    tooltip.set_tool(9, RECT::default(), "extra");
    assert_eq!(tooltip.tool_count(), 5);
    tooltip.set_tool(9, RECT::default(), "");
    assert_eq!(tooltip.tool_count(), 4);

    execute_command(window.hwnd, CommandId::Find);
    let find = app_mut(window.hwnd).find_bar().unwrap().panel_hwnd();
    assert_eq!(left_of(find, window.hwnd), left);
    assert_eq!(client_size(find).0, width - left);
    super::super::close_find_bar(window.hwnd);

    execute_command(window.hwnd, CommandId::CommandPalette);
    let palette = app_mut(window.hwnd)
        .command_palette
        .as_ref()
        .unwrap()
        .panel_hwnd();
    assert!(left_of(palette, window.hwnd) >= left);
    super::super::close_command_palette(window.hwnd, false);

    unsafe {
        SendMessageW(
            window.hwnd,
            super::super::WM_SYSCOMMAND,
            super::super::SC_KEYMENU as usize,
            0,
        )
    };
    assert_eq!(
        super::super::menu_headings(window.hwnd)[0].left,
        left + crate::window::design::metrics::scale(4, dpi)
    );
    unsafe {
        SendMessageW(
            window.hwnd,
            super::super::WM_SYSCOMMAND,
            super::super::SC_KEYMENU as usize,
            0,
        )
    };
}

#[test]
fn the_sidebar_top_strip_and_panel_header_are_caption() {
    // Break caught: child windows under the title row that swallow the caption, so the
    // window can no longer be dragged or top-resized there, or a lost left-border resize.
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowRect, HTCAPTION, HTCLIENT, HTLEFT, HTTRANSPARENT, WM_NCHITTEST,
    };
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    let (bar, panel) = sidebar_windows(window.hwnd);
    let dpi = unsafe { GetDpiForWindow(window.hwnd) }.max(96);
    let layout = super::super::title_layout(window.hwnd);
    let hit = |target: HWND, x: i32, y: i32| unsafe {
        SendMessageW(target, WM_NCHITTEST, 0, screen_lparam(target, x, y))
    };
    // Below the top resize band and above the first button.
    let strip_y = layout.height - 2;
    assert!(strip_y >= layout.resize_border);
    let bar_x = client_size(bar).0 / 2;
    assert_eq!(hit(bar, bar_x, strip_y), HTTRANSPARENT as LRESULT);
    assert_eq!(hit(window.hwnd, bar_x, strip_y), HTCAPTION as LRESULT);
    let (button_x, button_y) = button_center(
        window.hwnd,
        crate::window::activity_bar::ActivityButton::Notebook,
    );
    assert_eq!(hit(bar, button_x, button_y), HTCLIENT as LRESULT);

    let header_y = layout.resize_border + 2;
    let header =
        crate::window::design::metrics::scale(crate::window::design::metrics::PANEL_HEADER, dpi);
    assert!(header_y < header);
    // The Notebook view's title band holds only its caption, "Notebook": all of it is a
    // drag area, the old title point included.
    let panel_x = crate::window::design::metrics::scale(4, dpi);
    assert_eq!(
        hit(panel, client_size(panel).0 / 2, header_y),
        HTTRANSPARENT as LRESULT,
        "the whole title band is caption"
    );
    assert_eq!(hit(panel, panel_x, header_y), HTTRANSPARENT as LRESULT);
    assert_eq!(
        hit(window.hwnd, left_of(panel, window.hwnd) + panel_x, header_y),
        HTCAPTION as LRESULT
    );
    // Below the header, and on the resize edge, the panel keeps its own input.
    assert_eq!(hit(panel, panel_x, header + 10), HTCLIENT as LRESULT);
    assert_eq!(
        hit(panel, client_size(panel).0 - 1, header_y),
        HTCLIENT as LRESULT
    );

    // The left border is outside the client area, so no child covers it.
    let mut frame = RECT::default();
    let mut origin = windows_sys::Win32::Foundation::POINT::default();
    unsafe {
        GetWindowRect(window.hwnd, &mut frame);
        windows_sys::Win32::Graphics::Gdi::ClientToScreen(window.hwnd, &mut origin);
    }
    if origin.x > frame.left {
        let border = client_lparam(
            frame.left + (origin.x - frame.left) / 2,
            origin.y + client_size(window.hwnd).1 / 2,
        );
        assert_eq!(
            unsafe { SendMessageW(window.hwnd, WM_NCHITTEST, 0, border) },
            HTLEFT as LRESULT
        );
    }
}

// The icon resource (`APP_ICON_RESOURCE_ID`) is embedded by `build.rs` only into the FastPad
// binaries (`rustc-link-arg-bins`), not into this lib's own unit-test binary, so
// `load_logo_icon` returns `None` here regardless of DPI. The two tests below cover what is
// true either way: the load never runs before the deferred chrome step, and the square it
// would draw into stays caption; `ensure_logo_icon`'s replace-on-a-different-DPI wiring is
// exercised with a synthetic icon standing in for a loaded one. The real load, the drawn
// pixels and the destroy-on-drop are covered end to end by
// `tests/windows/titlebar.rs`'s `the_activity_bar_draws_the_app_logo_above_the_first_button_and_the_square_stays_caption`
// (a real `fastpad.exe`, which does have the resource) and by
// `titlebar::tests::a_logo_icon_destroys_its_handle_on_drop`.
#[test]
fn the_deferred_chrome_step_is_the_first_to_touch_the_logo_and_its_square_stays_caption() {
    // Break caught: the logo loaded before first paint (new startup latency), or its square
    // stealing the caption hit test once something is drawn there.
    use windows_sys::Win32::UI::WindowsAndMessaging::{HTCAPTION, HTTRANSPARENT, WM_NCHITTEST};
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    assert!(
        app_mut(window.hwnd).logo_icon.is_none(),
        "nothing loads the logo before the deferred chrome step"
    );

    super::super::build_chrome(window.hwnd);

    let dpi = unsafe { GetDpiForWindow(window.hwnd) }.max(96);
    let (bar, _panel) = sidebar_windows(window.hwnd);
    let (width, height) = client_size(bar);
    let client = RECT {
        left: 0,
        top: 0,
        right: width,
        bottom: height,
    };
    let rect = crate::window::activity_bar::logo_rect(client, dpi);
    let (x, y) = ((rect.left + rect.right) / 2, (rect.top + rect.bottom) / 2);
    let hit = |target: HWND| unsafe {
        SendMessageW(target, WM_NCHITTEST, 0, screen_lparam(target, x, y))
    };
    assert_eq!(hit(bar), HTTRANSPARENT as LRESULT);
    assert_eq!(hit(window.hwnd), HTCAPTION as LRESULT);
}

#[test]
fn ensure_logo_icon_leaves_a_matching_dpi_alone_and_replaces_a_different_one() {
    // Break caught: a DPI change that keeps the old icon around (never reloaded) or leaves it
    // set at the wrong DPI.
    use windows_sys::Win32::UI::WindowsAndMessaging::CreateIcon;
    let window = ProductionWindow::new(make_app());
    // Stands in for a load already having succeeded at 96 DPI; the real loader can't run in
    // this test binary (see the comment above).
    let and_mask = [0xffu8];
    let xor_mask = [0x00u8];
    let icon = unsafe {
        CreateIcon(
            std::ptr::null_mut(),
            1,
            1,
            1,
            1,
            and_mask.as_ptr(),
            xor_mask.as_ptr(),
        )
    };
    assert!(!icon.is_null());
    app_mut(window.hwnd).logo_icon = Some(crate::window::titlebar::LogoIcon::new(96, icon));

    super::super::ensure_logo_icon(window.hwnd, 96);
    assert_eq!(
        app_mut(window.hwnd).logo_icon.as_ref().unwrap().icon(),
        icon,
        "the same DPI is a no-op, not a reload"
    );

    super::super::ensure_logo_icon(window.hwnd, 144);
    assert!(
        app_mut(window.hwnd)
            .logo_icon
            .as_ref()
            .is_none_or(|logo| logo.dpi() != 96),
        "a different DPI replaces the stale one"
    );
}

#[test]
fn clicking_the_active_view_icon_closes_the_sidebar_panel_and_saves_none() {
    // Break caught: an icon that only ever opens its view, so the mouse cannot close the
    // panel, or a closed panel that reopens after a restart.
    use crate::config::SidebarView;
    use crate::window::activity_bar::ActivityButton;
    let _scintilla = load_native_scintilla();
    let (_scratch, ini) = settings_scratch("sidebar-click");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    let (bar, panel) = sidebar_windows(window.hwnd);
    let activity = client_size(bar).0;
    let view = || crate::window::side_panel::current_view(window.hwnd);
    let saved = || std::fs::read_to_string(&ini).unwrap();

    let (x, y) = button_center(window.hwnd, ActivityButton::Notebook);
    click(bar, x, y);
    assert_eq!(view(), SidebarView::Hidden);
    assert!(!is_shown(panel));
    assert_eq!(crate::window::side_panel::left_edge(window.hwnd), activity);
    assert_eq!(saved(), "# kept\r\nsidebar_view=none\r\n");

    click(bar, x, y);
    assert_eq!(view(), SidebarView::Notebook);
    assert!(is_shown(panel));
    assert_eq!(saved(), "# kept\r\nsidebar_view=notebook\r\n");

    let (x, y) = button_center(window.hwnd, ActivityButton::Search);
    click(bar, x, y);
    assert_eq!(view(), SidebarView::Search);
    assert_eq!(saved(), "# kept\r\nsidebar_view=search\r\n");

    // Settings opens the Settings dialog and leaves the panel alone.
    let shown = std::rc::Rc::new(std::cell::Cell::new(false));
    let seen = shown.clone();
    crate::window::settings_dialog::answer_next(move |dialog| {
        seen.set(true);
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(
                dialog,
                windows_sys::Win32::UI::WindowsAndMessaging::WM_KEYDOWN,
                usize::from(windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE),
                0,
            )
        };
    });
    let (x, y) = button_center(window.hwnd, ActivityButton::Settings);
    click(bar, x, y);
    assert!(shown.get(), "the gear opened Settings");
    assert_eq!(view(), SidebarView::Search);
    super::super::save_settings_to(None);
}

#[test]
fn ctrl_b_toggles_the_sidebar_back_to_the_last_view_and_saves_each_change() {
    // Break caught: a toggle that forgets which view was open, a shortcut that never
    // reaches its command, or a change lost on restart.
    use crate::config::SidebarView;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetKeyboardState, SetKeyboardState, VK_CONTROL, VK_SHIFT,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{MSG, WM_KEYDOWN};
    let _scintilla = load_native_scintilla();
    let (_scratch, ini) = settings_scratch("sidebar-ctrl-b");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    let identity = unsafe { super::super::window_identity(window.hwnd).unwrap() };
    let press = |key: u8, shift: bool| {
        let mut keys = [0u8; 256];
        unsafe { GetKeyboardState(keys.as_mut_ptr()) };
        let original = keys;
        keys[VK_CONTROL as usize] = 0x80;
        keys[VK_SHIFT as usize] = if shift { 0x80 } else { 0 };
        unsafe { SetKeyboardState(keys.as_ptr()) };
        let message = MSG {
            hwnd: editor.hwnd(),
            message: WM_KEYDOWN,
            wParam: usize::from(key),
            ..Default::default()
        };
        let translated =
            unsafe { super::super::translate_accelerator(window.hwnd, &identity, &message) };
        unsafe { SetKeyboardState(original.as_ptr()) };
        translated
    };
    let view = || crate::window::side_panel::current_view(window.hwnd);
    let saved = || std::fs::read_to_string(&ini).unwrap();

    assert_eq!(view(), SidebarView::Notebook);
    assert!(press(b'B', false));
    assert_eq!(view(), SidebarView::Hidden);
    assert_eq!(saved(), "# kept\r\nsidebar_view=none\r\n");
    // Break caught: Ctrl+K still bound after Search moved to Ctrl+Shift+F.
    assert!(!press(b'K', false));
    assert_eq!(view(), SidebarView::Hidden);
    assert!(press(b'F', true));
    assert_eq!(view(), SidebarView::Search);
    assert!(press(b'B', false));
    assert!(press(b'B', false));
    assert_eq!(view(), SidebarView::Search, "Ctrl+B reopens the last view");
    assert!(press(b'E', true));
    assert_eq!(view(), SidebarView::Notebook);
    execute_command(window.hwnd, CommandId::ShowFavoritesView);
    assert_eq!(view(), SidebarView::Favorites);
    assert_eq!(saved(), "# kept\r\nsidebar_view=favorites\r\n");
    super::super::save_settings_to(None);
}

#[test]
fn a_narrow_window_squeezes_the_panel_without_saving_it() {
    // Break caught: an editor pushed below its 320 px minimum, a negative panel width, or a
    // squeeze written to fastpad.ini so the panel stays narrow once the window widens again.
    let _scintilla = load_native_scintilla();
    let (_scratch, ini) = settings_scratch("sidebar-squeeze");
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    let (_, panel) = sidebar_windows(window.hwnd);
    let dpi = unsafe { GetDpiForWindow(window.hwnd) }.max(96);
    let scale = |value| crate::window::design::metrics::scale(value, dpi);

    set_client_width(window.hwnd, scale(44 + 320 + 200));
    let (width, _) = client_size(window.hwnd);
    let (activity, squeezed) = crate::window::side_panel::sidebar_widths(width, dpi, true, 260);
    assert_eq!(squeezed, width - activity - scale(320));
    assert!(
        squeezed > 0 && squeezed < scale(260),
        "the window squeezes the panel"
    );
    assert_eq!(client_size(panel).0, squeezed);
    assert_eq!(
        crate::window::side_panel::left_edge(window.hwnd),
        activity + squeezed
    );
    assert_eq!(
        client_size(editor.hwnd()).0,
        scale(320).max(width - activity - squeezed)
    );
    assert_eq!(app_mut(window.hwnd).settings.sidebar_width, 260);

    set_client_width(window.hwnd, scale(1200));
    assert_eq!(client_size(panel).0, scale(260));

    // Narrower than the activity bar and the editor minimum: the panel hides, never goes
    // below zero.
    set_client_width(window.hwnd, scale(300));
    assert!(!is_shown(panel));
    assert_eq!(
        crate::window::side_panel::left_edge(window.hwnd),
        scale(44).min(client_size(window.hwnd).0)
    );
    assert_eq!(app_mut(window.hwnd).settings.sidebar_width, 260);
    assert_eq!(std::fs::read_to_string(&ini).unwrap(), "# kept\r\n");
    super::super::save_settings_to(None);
}

#[test]
fn dragging_the_sidebar_edge_resizes_it_and_saves_the_width_once_on_release() {
    // Break caught: a drag that writes fastpad.ini on every mouse move, never saves, ignores
    // the 180–480 range, or an edge double-click that leaves a custom width in place.
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE,
    };
    let _scintilla = load_native_scintilla();
    let (_scratch, ini) = settings_scratch("sidebar-drag");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    let (_, panel) = sidebar_windows(window.hwnd);
    let dpi = unsafe { GetDpiForWindow(window.hwnd) }.max(96);
    let scale = |value| crate::window::design::metrics::scale(value, dpi);
    set_client_width(window.hwnd, scale(1400));
    let saved = || std::fs::read_to_string(&ini).unwrap();
    let send = |message, x: i32| unsafe {
        SendMessageW(
            panel,
            message,
            0,
            client_lparam(x, client_size(panel).1 / 2),
        );
    };

    send(WM_LBUTTONDOWN, client_size(panel).0 - 1);
    send(WM_MOUSEMOVE, scale(300));
    assert_eq!(client_size(panel).0, scale(300));
    assert_eq!(saved(), "# kept\r\n", "nothing is saved mid-drag");
    send(WM_LBUTTONUP, scale(300));
    assert_eq!(saved(), "# kept\r\nsidebar_width=300\r\n");
    assert_eq!(app_mut(window.hwnd).settings.sidebar_width, 300);

    send(WM_LBUTTONDOWN, client_size(panel).0 - 1);
    send(WM_MOUSEMOVE, scale(900));
    send(WM_LBUTTONUP, scale(900));
    assert_eq!(saved(), "# kept\r\nsidebar_width=480\r\n");
    assert_eq!(client_size(panel).0, scale(480));

    send(WM_LBUTTONDBLCLK, client_size(panel).0 - 1);
    assert_eq!(saved(), "# kept\r\nsidebar_width=260\r\n");
    assert_eq!(client_size(panel).0, scale(260));
    super::super::save_settings_to(None);
}

#[test]
fn the_sidebar_fonts_are_rebuilt_when_only_the_text_size_changes() {
    // Break caught: a text-size change leaving the old fonts in the cache, so the sidebar keeps
    // its old text size until the DPI changes.
    use crate::window::design::text_scale::set_factor_for_test;
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    set_factor_for_test(100);
    let sidebar = app_mut(window.hwnd)
        .sidebar
        .as_mut()
        .expect("notes mode has a sidebar");
    let first = sidebar.fonts(96).text;
    assert_eq!(sidebar.fonts(96).text, first, "same key reuses the fonts");
    set_factor_for_test(150);
    let second = sidebar.fonts(96).text;
    set_factor_for_test(100);
    assert_ne!(second, first);
}
