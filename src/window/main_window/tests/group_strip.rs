//! The group window's strip as the title row, launch tabs, floating preview buttons and
//! early palette layout.

use super::*;

#[test]
fn the_editor_and_find_bar_live_in_the_group_window() {
    // Break caught: a child left parented to the main window, painting over or under the
    // group and missing its layout.
    use windows_sys::Win32::UI::WindowsAndMessaging::GetParent;
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    let group = super::super::group_hwnd(window.hwnd).expect("the editor group window");
    assert_eq!(unsafe { GetParent(group) }, window.hwnd);
    assert_eq!(unsafe { GetParent(editor.hwnd()) }, group);
    execute_command(window.hwnd, CommandId::Find);
    let panel = app_mut(window.hwnd).find_bar().unwrap().panel_hwnd();
    assert_eq!(unsafe { GetParent(panel) }, group);
    let (width, height) = client_size(window.hwnd);
    let (group_width, group_height) = client_size(group);
    assert_eq!(left_of(group, window.hwnd) + group_width, width);
    // The group reaches up into the title row, down to the status bar.
    assert!(group_height > 0 && group_height <= height);
}

#[test]
fn find_bar_keys_still_reach_the_main_window() {
    // Break caught: the find field hook sending to its parent, now the group, so Enter and
    // Escape in the query field do nothing.
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_ESCAPE, VK_RETURN};
    use windows_sys::Win32::UI::WindowsAndMessaging::{SetWindowTextW, WM_KEYDOWN};
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    editor.set_text("xyz abc abc").unwrap();
    editor.set_selection(0..0).unwrap();
    execute_command(window.hwnd, CommandId::Find);
    let query = app_mut(window.hwnd).find_bar().unwrap().query_hwnd();
    let text = crate::platform::wide_null("abc");
    unsafe { SetWindowTextW(query, text.as_ptr()) };
    editor.set_selection(0..0).unwrap();

    unsafe { SendMessageW(query, WM_KEYDOWN, usize::from(VK_RETURN), 0) };
    assert_eq!(editor.selected_text().unwrap(), "abc");

    unsafe { SendMessageW(query, WM_KEYDOWN, usize::from(VK_ESCAPE), 0) };
    assert!(!app_mut(window.hwnd).find_bar().unwrap().is_visible());
}

#[test]
fn typing_in_the_grouped_editor_still_marks_the_tab_dirty() {
    // Break caught: WM_NOTIFY now going to the group, which drops it, so typing never dirties
    // the tab and never autosaves.
    use windows_sys::Win32::UI::WindowsAndMessaging::WM_CHAR;
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    assert!(!app_mut(window.hwnd).tabs.active().unwrap().dirty);
    unsafe { SendMessageW(editor.hwnd(), WM_CHAR, usize::from(b'x'), 0) };
    assert!(app_mut(window.hwnd).tabs.active().unwrap().dirty);
}

#[test]
fn clicking_a_tab_in_the_group_strip_activates_it() {
    // Break caught: strip input still handled by the main window, so clicks on the strip that
    // moved into the group do nothing.
    use windows_sys::Win32::UI::WindowsAndMessaging::{WM_LBUTTONDOWN, WM_LBUTTONUP};
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    execute_command(window.hwnd, CommandId::New);
    assert_eq!(app_mut(window.hwnd).tabs.active_index(), 1);
    let group = super::super::group_hwnd(window.hwnd).unwrap();
    let tab = super::super::strip_layout(window.hwnd)
        .unwrap()
        .tab(0)
        .unwrap();
    let point = client_lparam(tab.left + 10, tab.bottom / 2);
    unsafe {
        SendMessageW(group, WM_LBUTTONDOWN, 1, point);
        SendMessageW(group, WM_LBUTTONUP, 0, point);
    }
    assert_eq!(app_mut(window.hwnd).tabs.active_index(), 0);
}

#[test]
fn double_clicking_the_empty_strip_where_the_new_tab_closes_keeps_it_open() {
    // Break caught: the release after the double-click acting on whatever the new tab put
    // under the pointer, so a double-click on its close box's spot opens a tab and closes it.
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP,
    };
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    execute_command(window.hwnd, CommandId::New);
    let close = super::super::strip_layout(window.hwnd)
        .unwrap()
        .close_tab(1)
        .unwrap()
        .center();
    execute_command(window.hwnd, CommandId::CloseTab);
    assert_eq!(super::super::tab_count(window.hwnd), 1);
    assert_eq!(
        super::super::strip_target(
            window.hwnd,
            app_mut(window.hwnd).tabs.active_group(),
            close.x,
            close.y
        ),
        Some(crate::window::group_strip::StripTarget::Empty)
    );

    let group = super::super::group_hwnd(window.hwnd).unwrap();
    let point = client_lparam(close.x, close.y);
    unsafe {
        SendMessageW(group, WM_LBUTTONDOWN, 1, point);
        SendMessageW(group, WM_LBUTTONUP, 0, point);
        SendMessageW(group, WM_LBUTTONDBLCLK, 1, point);
        SendMessageW(group, WM_LBUTTONUP, 0, point);
    }
    assert_eq!(super::super::tab_count(window.hwnd), 2);
}

/// `child`'s top-left corner in `parent`'s client coordinates.
fn origin_in(child: HWND, parent: HWND) -> (i32, i32) {
    let mut point = windows_sys::Win32::Foundation::POINT { x: 0, y: 0 };
    unsafe { windows_sys::Win32::Graphics::Gdi::MapWindowPoints(child, parent, &mut point, 1) };
    (point.x, point.y)
}

#[test]
fn the_group_strip_is_the_title_row() {
    // Break caught: the strip drawn in a row of its own under the title bar (the file name
    // shown twice and a row of height lost), or running under the app menu and the caption
    // buttons.
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    let group = super::super::group_hwnd(window.hwnd).unwrap();
    let title = super::super::title_layout(window.hwnd);
    let left = crate::window::side_panel::left_edge(window.hwnd);
    assert_eq!(origin_in(group, window.hwnd), (left, 0));
    assert_eq!(origin_in(editor.hwnd(), window.hwnd).1, title.height);
    let strip = super::super::strip_layout(window.hwnd).unwrap();
    assert_eq!(strip.height, title.height);
    assert_eq!(strip.bounds().right, title.minimize.left - left);
}

#[test]
fn empty_title_row_strip_space_is_caption_and_tabs_are_client() {
    // Break caught: a strip that swallows the caption, so the window can no longer be dragged
    // or top-resized by the empty space beside the tabs.
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        HTCAPTION, HTCLIENT, HTTRANSPARENT, WM_NCHITTEST,
    };
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    let group = super::super::group_hwnd(window.hwnd).unwrap();
    let title = super::super::title_layout(window.hwnd);
    let strip = super::super::strip_layout(window.hwnd).unwrap();
    let left = crate::window::side_panel::left_edge(window.hwnd);
    let hit = |target: HWND, x: i32, y: i32| unsafe {
        SendMessageW(target, WM_NCHITTEST, 0, screen_lparam(target, x, y))
    };
    let y = strip.height - 2;
    assert!(y >= title.resize_border);
    let tab = strip.tab(0).unwrap().center();
    let empty = strip.tabs.right - 10;
    assert_eq!(hit(group, tab.x, y), HTCLIENT as LRESULT);
    assert_eq!(hit(group, empty, y), HTTRANSPARENT as LRESULT);
    assert_eq!(hit(window.hwnd, empty + left, y), HTCAPTION as LRESULT);
    // A restored window's top band resizes, over the tabs too.
    assert_eq!(hit(group, tab.x, 0), HTTRANSPARENT as LRESULT);
}

#[test]
fn a_caption_double_click_over_the_strip_opens_a_tab_and_over_the_sidebar_does_not() {
    // Break caught: the caption's double-click maximizing over the strip's empty space, where
    // 0.2.0 opened a new tab, or opening tabs from the caption over the sidebar.
    use windows_sys::Win32::UI::WindowsAndMessaging::{HTCAPTION, WM_NCLBUTTONDBLCLK};
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    let title = super::super::title_layout(window.hwnd);
    let strip = super::super::strip_layout(window.hwnd).unwrap();
    let left = crate::window::side_panel::left_edge(window.hwnd);
    let double_click = |x: i32, y: i32| unsafe {
        SendMessageW(
            window.hwnd,
            WM_NCLBUTTONDBLCLK,
            HTCAPTION as usize,
            screen_lparam(window.hwnd, x, y),
        )
    };
    double_click(strip.tabs.right - 10 + left, strip.height / 2);
    assert_eq!(super::super::tab_count(window.hwnd), 2);
    assert!(
        title.sidebar.right > title.sidebar.left,
        "no sidebar to test"
    );
    let sidebar = title.sidebar.center();
    double_click(sidebar.x, sidebar.y);
    assert_eq!(super::super::tab_count(window.hwnd), 2);
}

#[test]
fn a_caption_right_click_over_the_strip_opens_the_strip_menu() {
    // Break caught: the strip's menu lost once its empty space answers as caption.
    use windows_sys::Win32::UI::WindowsAndMessaging::{HTCAPTION, WM_NCRBUTTONUP};
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    let strip = super::super::strip_layout(window.hwnd).unwrap();
    let left = crate::window::side_panel::left_edge(window.hwnd);
    answer_next_popup_menu(|_| Some(CommandId::CloseAllTabs));
    unsafe {
        SendMessageW(
            window.hwnd,
            WM_NCRBUTTONUP,
            HTCAPTION as usize,
            screen_lparam(window.hwnd, strip.tabs.right - 10 + left, strip.height / 2),
        )
    };
    assert!(app_mut(window.hwnd).tabs.is_empty());
}

#[test]
fn the_group_region_leaves_the_caption_buttons_and_the_menu_band_to_the_main_window() {
    // Break caught: the group window covering the app menu and caption buttons, or the menu
    // band shown under it, so the main window can neither paint them nor get their clicks.
    use windows_sys::Win32::Graphics::Gdi::{
        CreateRectRgn, DeleteObject, GetWindowRgn, PtInRegion,
    };
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    let group = super::super::group_hwnd(window.hwnd).unwrap();
    let left = crate::window::side_panel::left_edge(window.hwnd);
    let covers = |x: i32, y: i32| unsafe {
        let region = CreateRectRgn(0, 0, 0, 0);
        GetWindowRgn(group, region);
        let inside = PtInRegion(region, x - left, y) != 0;
        DeleteObject(region);
        inside
    };
    let title = super::super::title_layout(window.hwnd);
    let tab = super::super::strip_layout(window.hwnd)
        .unwrap()
        .tab(0)
        .unwrap()
        .center();
    assert!(covers(tab.x + left, tab.y));
    for button in [title.minimize, title.maximize, title.close] {
        let center = button.center();
        assert!(!covers(center.x, center.y));
    }

    super::super::enter_menu_mode(window.hwnd, 0);
    let dpi = unsafe { GetDpiForWindow(window.hwnd) }.max(96);
    let band = super::super::menu_band::band_height(dpi);
    let heading = super::super::menu_headings(window.hwnd)[0];
    assert!(!covers(
        (heading.left + heading.right) / 2,
        (heading.top + heading.bottom) / 2
    ));
    assert!(covers(tab.x + left, tab.y));
    assert_eq!(origin_in(editor.hwnd(), window.hwnd).1, title.height + band);
}

#[test]
fn markdown_tabs_show_floating_preview_buttons_at_the_content_top_right() {
    // Break caught: preview buttons left in the strip, shown for tabs that cannot preview,
    // or drawn under the editor.
    use windows_sys::Win32::UI::WindowsAndMessaging::{GW_HWNDPREV, GetParent, GetWindow};
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    let group = super::super::group_hwnd(window.hwnd).unwrap();
    let buttons = || crate::window::preview_buttons::hwnd(window.hwnd);
    let visible = |hwnd: HWND| {
        (unsafe { GetWindowLongPtrW(hwnd, super::super::GWL_STYLE) }) as u32
            & super::super::WS_VISIBLE
            != 0
    };
    assert!(buttons().is_none_or(|hwnd| !visible(hwnd)));

    // What a successful language switch does, without loading Lexilla.
    app_mut(window.hwnd)
        .tabs
        .set_active_language(crate::document::Language::Markdown);
    crate::window::preview_host::sync_visibility(window.hwnd);
    let hwnd = buttons().expect("floating preview buttons");
    assert!(visible(hwnd));
    assert_eq!(unsafe { GetParent(hwnd) }, group);
    let strip = super::super::strip_layout(window.hwnd).unwrap();
    assert_eq!(
        strip.tabs.right,
        strip.bounds().right,
        "the strip holds only tabs"
    );
    let dpi = unsafe { GetDpiForWindow(group) }.max(96);
    let (group_width, _) = client_size(group);
    let (x, y) = origin_in(hwnd, group);
    let (width, _) = client_size(hwnd);
    assert_eq!(y, strip.height + crate::window::titlebar::scale(8, dpi));
    assert!(x + width < group_width, "clear of the vertical scroll bar");
    assert!(x + width >= group_width - crate::window::titlebar::scale(48, dpi));
    // Above the editor in z-order: no sibling before it.
    assert!(unsafe { GetWindow(hwnd, GW_HWNDPREV) }.is_null());
}

#[test]
fn a_plain_launch_leaves_no_empty_untitled_tab() {
    // Break caught: double-clicking FastPad.exe opens on an untitled document nobody asked
    // for.
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    assert_eq!(app_mut(window.hwnd).tabs.len(), 1);

    super::super::handle_open_request(window.hwnd);

    assert!(app_mut(window.hwnd).tabs.is_empty());
}

#[test]
fn typing_into_a_window_with_no_tabs_starts_an_untitled_tab() {
    // Break caught: a plain launch with no tab swallowing the first keystrokes, so FastPad is
    // no longer instant-to-type.
    use windows_sys::Win32::UI::WindowsAndMessaging::{MSG, WM_CHAR};
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    let identity = unsafe { super::super::window_identity(window.hwnd).unwrap() };
    super::super::handle_open_request(window.hwnd);
    assert!(app_mut(window.hwnd).tabs.is_empty());
    let message = MSG {
        hwnd: editor.hwnd(),
        message: WM_CHAR,
        wParam: usize::from(b'a'),
        ..Default::default()
    };

    let translated =
        unsafe { super::super::translate_accelerator(window.hwnd, &identity, &message) };
    unsafe { SendMessageW(editor.hwnd(), WM_CHAR, message.wParam, 0) };

    assert!(!translated, "the character still reaches the editor");
    assert_eq!(app_mut(window.hwnd).tabs.len(), 1);
    assert_eq!(editor.text().unwrap(), "a");
    assert!(app_mut(window.hwnd).tabs.active().unwrap().dirty);
}

#[test]
fn typing_with_the_focus_on_the_main_window_and_no_tabs_types_into_a_new_tab() {
    // Break caught: closing the last tab hides the editor and leaves the focus on the main
    // window, whose characters go nowhere.
    use windows_sys::Win32::UI::WindowsAndMessaging::{MSG, WM_CHAR};
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    let identity = unsafe { super::super::window_identity(window.hwnd).unwrap() };
    super::super::handle_open_request(window.hwnd);
    let message = MSG {
        hwnd: window.hwnd,
        message: WM_CHAR,
        wParam: usize::from(b'a'),
        ..Default::default()
    };

    let translated =
        unsafe { super::super::translate_accelerator(window.hwnd, &identity, &message) };

    assert!(
        translated,
        "the character went to the new tab's editor instead"
    );
    assert_eq!(app_mut(window.hwnd).tabs.len(), 1);
    assert_eq!(editor.text().unwrap(), "a");
}

#[test]
fn a_plain_launch_keeps_the_untitled_tab_once_it_has_text() {
    // Break caught: text typed before the startup chain finished thrown away with its tab.
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    editor.set_text("typed early").unwrap();

    super::super::handle_open_request(window.hwnd);

    assert_eq!(app_mut(window.hwnd).tabs.len(), 1);
}

#[test]
fn only_the_active_group_shows_the_floating_preview_buttons() {
    // Break caught: a Markdown document shown in two groups floats a pair of preview buttons
    // over both, though only the selected one's act on the active tab.
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    let first = app_mut(window.hwnd).tabs.active_group();
    app_mut(window.hwnd)
        .tabs
        .set_active_language(crate::document::Language::Markdown);
    crate::window::preview_host::sync_visibility(window.hwnd);
    execute_command(window.hwnd, CommandId::SplitRight);
    let second = app_mut(window.hwnd).tabs.active_group();
    let shown = |id| {
        super::super::with_group_id(window.hwnd, id, |state| state.preview_buttons.hwnd)
            .filter(|hwnd| !hwnd.is_null())
            .is_some_and(|hwnd| {
                (unsafe { GetWindowLongPtrW(hwnd, super::super::GWL_STYLE) }) as u32
                    & super::super::WS_VISIBLE
                    != 0
            })
    };
    assert!(shown(second));
    assert!(!shown(first));

    assert!(super::super::activate_group(window.hwnd, first));

    assert!(shown(first));
    assert!(!shown(second));
}

#[test]
fn the_floating_side_button_opens_and_closes_the_side_preview() {
    // Break caught: floating buttons that paint but do not act.
    use windows_sys::Win32::UI::WindowsAndMessaging::{WM_LBUTTONDOWN, WM_LBUTTONUP};
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    // What a successful language switch does, without loading Lexilla.
    app_mut(window.hwnd)
        .tabs
        .set_active_language(crate::document::Language::Markdown);
    crate::window::preview_host::sync_visibility(window.hwnd);
    let hwnd = crate::window::preview_buttons::hwnd(window.hwnd).unwrap();
    let side = crate::window::preview_buttons::button_rect(
        hwnd,
        crate::window::preview_buttons::PreviewButton::Side,
    )
    .center();
    let click = || unsafe {
        SendMessageW(hwnd, WM_LBUTTONDOWN, 1, client_lparam(side.x, side.y));
        SendMessageW(hwnd, WM_LBUTTONUP, 0, client_lparam(side.x, side.y));
    };
    click();
    assert_eq!(
        crate::window::preview_host::mode(window.hwnd),
        crate::preview::PreviewMode::Split
    );
    click();
    assert_eq!(
        crate::window::preview_host::mode(window.hwnd),
        crate::preview::PreviewMode::Off
    );
}

#[test]
fn clicking_a_tab_in_menu_mode_leaves_menu_mode() {
    // Break caught: the group taking strip clicks without the main window's menu-mode exit, so
    // after Alt a tab click switches tabs while keystrokes still go to menu mnemonics.
    use windows_sys::Win32::UI::WindowsAndMessaging::{WM_LBUTTONDOWN, WM_LBUTTONUP};
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    execute_command(window.hwnd, CommandId::New);
    super::super::enter_menu_mode(window.hwnd, 0);
    assert!(app_mut(window.hwnd).menu_mode.is_some());

    let group = super::super::group_hwnd(window.hwnd).unwrap();
    let tab = super::super::strip_layout(window.hwnd)
        .unwrap()
        .tab(0)
        .unwrap();
    let point = client_lparam(tab.left + 10, tab.bottom / 2);
    unsafe {
        SendMessageW(group, WM_LBUTTONDOWN, 1, point);
        SendMessageW(group, WM_LBUTTONUP, 0, point);
    }
    assert_eq!(app_mut(window.hwnd).tabs.active_index(), 0);
    assert_eq!(app_mut(window.hwnd).menu_mode, None);
}

#[test]
fn the_command_palette_opens_below_the_tab_strip_and_the_find_bar() {
    // Break caught: the palette's top still summing only the title and bar heights, so once
    // the strip and find bar moved into the group it covers the strip and the find bar.
    use windows_sys::Win32::UI::WindowsAndMessaging::GetWindowRect;
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    execute_command(window.hwnd, CommandId::Find);
    execute_command(window.hwnd, CommandId::CommandPalette);
    let rect = |hwnd| {
        let mut rect = RECT::default();
        unsafe { GetWindowRect(hwnd, &mut rect) };
        rect
    };
    let app = app_mut(window.hwnd);
    let find = rect(app.find_bar().unwrap().panel_hwnd());
    let palette = rect(app.command_palette.as_ref().unwrap().panel_hwnd());
    assert!(
        palette.top >= find.bottom,
        "palette top {} overlaps the find bar ending at {}",
        palette.top,
        find.bottom
    );
}

#[test]
fn the_tab_strip_paints_before_deferred_startup_begins() {
    // Break caught: the group's first paint queued behind the deferred startup chain, whose
    // posted steps outrank WM_PAINT, so the first frame shows no tabs until restore finishes.
    use windows_sys::Win32::Graphics::Gdi::{GetUpdateRect, InvalidateRect};
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    use windows_sys::Win32::UI::WindowsAndMessaging::{SW_SHOWNA, ShowWindow};
    let group = super::super::group_hwnd(window.hwnd).unwrap();
    let identity = app_mut(window.hwnd).window_identity();
    app_mut(window.hwnd).mark_first_paint_complete();
    // Only a visible window has an update region to wait on.
    unsafe {
        ShowWindow(window.hwnd, SW_SHOWNA);
        InvalidateRect(group, std::ptr::null(), 0);
    }
    assert_ne!(unsafe { GetUpdateRect(group, std::ptr::null_mut(), 0) }, 0);

    unsafe { super::super::maybe_post_deferred_start(window.hwnd, &identity) };
    assert_eq!(
        unsafe { GetUpdateRect(group, std::ptr::null_mut(), 0) },
        0,
        "the strip still waits for a WM_PAINT"
    );
}

#[test]
fn zoom_resizes_the_line_number_gutter() {
    // Break caught: zoomed digits clipped by a gutter measured at the unzoomed size.
    use crate::editor::scintilla_constants::SCI_GETZOOM;
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    let zoom = || unsafe { SendMessageW(editor.hwnd(), SCI_GETZOOM, 0, 0) };
    let unzoomed_width = line_number_margin_width(&editor);

    execute_command(window.hwnd, CommandId::ZoomIn);
    execute_command(window.hwnd, CommandId::ZoomIn);
    assert_eq!(zoom(), 2);
    assert!(line_number_margin_width(&editor) > unzoomed_width);
    execute_command(window.hwnd, CommandId::ZoomOut);
    assert_eq!(zoom(), 1);
    execute_command(window.hwnd, CommandId::ZoomReset);
    assert_eq!(zoom(), 0);
    assert_eq!(line_number_margin_width(&editor), unzoomed_width);
}
