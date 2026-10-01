//! Dragging tabs along a strip, between groups and onto folders.

use super::*;

/// Group `from`'s window, and a press on its tab `index` followed by a move past the drag
/// distance.
fn start_strip_drag(hwnd: HWND, from: GroupId, index: usize) -> HWND {
    use windows_sys::Win32::UI::WindowsAndMessaging::{WM_LBUTTONDOWN, WM_MOUSEMOVE};
    let window = super::super::with_group_id(hwnd, from, |state| state.hwnd).unwrap();
    let tab = super::super::strip_layout_of(hwnd, from)
        .unwrap()
        .tab(index)
        .unwrap()
        .center();
    unsafe {
        SendMessageW(window, WM_LBUTTONDOWN, 1, client_lparam(tab.x, tab.y));
        SendMessageW(window, WM_MOUSEMOVE, 1, client_lparam(tab.x + 30, tab.y));
    }
    window
}

/// Moves the drag to window `to`'s client point and releases there; `buttons` carries
/// `MK_CONTROL` for a copy.
fn drop_strip_drag(from: HWND, to: HWND, x: i32, y: i32, buttons: usize) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{WM_LBUTTONUP, WM_MOUSEMOVE};
    let point = lparam_in(from, to, x, y);
    unsafe {
        SendMessageW(from, WM_MOUSEMOVE, 1 | buttons, point);
        SendMessageW(from, WM_LBUTTONUP, buttons, point);
    }
}

fn strip_ids(hwnd: HWND, group: GroupId) -> Vec<crate::document::DocumentId> {
    app_mut(hwnd).tabs.group(group).unwrap().document_ids()
}

#[test]
fn a_tab_dragged_along_its_strip_moves_there_and_stays_active() {
    // Break caught: a strip drag that does nothing, or reorders but leaves the editor on
    // another tab.
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    execute_command(window.hwnd, CommandId::New);
    execute_command(window.hwnd, CommandId::New);
    let group = app_mut(window.hwnd).tabs.active_group();
    let ids = strip_ids(window.hwnd, group);
    let source = start_strip_drag(window.hwnd, group, 0);
    assert!(
        app_mut(window.hwnd)
            .tab_drag
            .as_ref()
            .is_some_and(|drag| drag.started)
    );
    let layout = super::super::strip_layout_of(window.hwnd, group).unwrap();
    let end = layout.tab(2).unwrap();
    drop_strip_drag(source, source, end.right - 2, end.center().y, 0);
    assert_eq!(strip_ids(window.hwnd, group), [ids[1], ids[2], ids[0]]);
    assert_eq!(app_mut(window.hwnd).tabs.active().unwrap().id, ids[0]);
    assert!(app_mut(window.hwnd).tab_drag.is_none());
    assert!(app_mut(window.hwnd).drop_overlay.is_none());
}

#[test]
fn a_wobble_under_the_drag_distance_is_still_a_click() {
    // Break caught (Review Focus 1): a slightly shaky click starting a drag, so the tab
    // never activates.
    use windows_sys::Win32::UI::WindowsAndMessaging::{WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE};
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    execute_command(window.hwnd, CommandId::New);
    let group = app_mut(window.hwnd).tabs.active_group();
    let first = strip_ids(window.hwnd, group)[0];
    let tab = super::super::strip_layout_of(window.hwnd, group)
        .unwrap()
        .tab(0)
        .unwrap()
        .center();
    let source = super::super::with_group_id(window.hwnd, group, |state| state.hwnd).unwrap();
    unsafe {
        SendMessageW(source, WM_LBUTTONDOWN, 1, client_lparam(tab.x, tab.y));
        SendMessageW(source, WM_MOUSEMOVE, 1, client_lparam(tab.x + 1, tab.y));
        SendMessageW(source, WM_LBUTTONUP, 0, client_lparam(tab.x + 1, tab.y));
    }
    assert_eq!(app_mut(window.hwnd).tabs.active().unwrap().id, first);
    assert!(app_mut(window.hwnd).tab_drag.is_none());
}

#[test]
fn a_press_on_a_tabs_close_button_never_starts_a_drag() {
    // Break caught: a jittery click on × dragging the tab instead of closing it.
    use windows_sys::Win32::UI::WindowsAndMessaging::{WM_LBUTTONDOWN, WM_MOUSEMOVE};
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    let group = app_mut(window.hwnd).tabs.active_group();
    let close = super::super::strip_layout_of(window.hwnd, group)
        .unwrap()
        .close_tab(0)
        .unwrap()
        .center();
    let source = super::super::with_group_id(window.hwnd, group, |state| state.hwnd).unwrap();
    unsafe {
        SendMessageW(source, WM_LBUTTONDOWN, 1, client_lparam(close.x, close.y));
        SendMessageW(
            source,
            WM_MOUSEMOVE,
            1,
            client_lparam(close.x - 40, close.y),
        );
    }
    assert!(
        app_mut(window.hwnd)
            .tab_drag
            .as_ref()
            .is_none_or(|drag| !drag.started)
    );
}

#[test]
fn esc_cancels_a_tab_drag_and_takes_its_label_and_overlay() {
    // Break caught: Esc typed into the editor while a tab drag hangs on the pointer.
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE;
    use windows_sys::Win32::UI::WindowsAndMessaging::{MSG, WM_KEYDOWN};
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    execute_command(window.hwnd, CommandId::New);
    let group = app_mut(window.hwnd).tabs.active_group();
    let ids = strip_ids(window.hwnd, group);
    start_strip_drag(window.hwnd, group, 0);
    let escape = MSG {
        hwnd: editor.hwnd(),
        message: WM_KEYDOWN,
        wParam: VK_ESCAPE as usize,
        ..Default::default()
    };
    assert!(crate::window::tab_drag::keeps_key(window.hwnd, &escape));
    assert!(app_mut(window.hwnd).tab_drag.is_none());
    assert!(app_mut(window.hwnd).drop_overlay.is_none());
    assert_eq!(strip_ids(window.hwnd, group), ids);
    assert_eq!(
        unsafe { windows_sys::Win32::UI::Input::KeyboardAndMouse::GetCapture() },
        std::ptr::null_mut()
    );
}

#[test]
fn a_lost_capture_cancels_a_tab_drag() {
    // Break caught (Review Focus 2): Alt+Tab mid-drag leaving the label on screen and the
    // next click dropping the tab somewhere.
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    execute_command(window.hwnd, CommandId::New);
    let group = app_mut(window.hwnd).tabs.active_group();
    start_strip_drag(window.hwnd, group, 0);
    unsafe { ReleaseCapture() };
    assert!(app_mut(window.hwnd).tab_drag.is_none());
    assert!(app_mut(window.hwnd).drop_overlay.is_none());
}

#[test]
fn a_right_press_cancels_a_tab_drag_and_its_release_opens_no_menu() {
    // Break caught: the right release after a cancel falling through to the strip's menu.
    use windows_sys::Win32::UI::WindowsAndMessaging::{WM_RBUTTONDOWN, WM_RBUTTONUP};
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    execute_command(window.hwnd, CommandId::New);
    let group = app_mut(window.hwnd).tabs.active_group();
    let menu = std::rc::Rc::new(std::cell::Cell::new(false));
    let shown = menu.clone();
    crate::window::menus::answer_next_popup_menu(move |_| {
        shown.set(true);
        None
    });
    let source = start_strip_drag(window.hwnd, group, 0);
    unsafe { SendMessageW(source, WM_RBUTTONDOWN, 2, client_lparam(20, 10)) };
    assert!(
        app_mut(window.hwnd)
            .tab_drag
            .as_ref()
            .is_some_and(|drag| drag.eat_right_up)
    );
    unsafe { SendMessageW(source, WM_RBUTTONUP, 0, client_lparam(20, 10)) };
    assert!(app_mut(window.hwnd).tab_drag.is_none());
    assert!(!menu.get(), "the release opened a menu");
}

#[test]
fn the_insertion_bar_shows_over_the_strip_under_the_pointer() {
    // Break caught: no feedback until the drop, so the user cannot see where the tab lands.
    use windows_sys::Win32::UI::WindowsAndMessaging::WM_MOUSEMOVE;
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    execute_command(window.hwnd, CommandId::New);
    execute_command(window.hwnd, CommandId::New);
    let group = app_mut(window.hwnd).tabs.active_group();
    let source = start_strip_drag(window.hwnd, group, 0);
    let layout = super::super::strip_layout_of(window.hwnd, group).unwrap();
    let x = layout.insertion_x(2);
    unsafe { SendMessageW(source, WM_MOUSEMOVE, 1, client_lparam(x + 1, 10)) };
    let overlay = app_mut(window.hwnd).drop_overlay.expect("an insertion bar");
    let mut origin = windows_sys::Win32::Foundation::POINT { x, y: 0 };
    unsafe { windows_sys::Win32::Graphics::Gdi::ClientToScreen(source, &mut origin) };
    let rect = overlay.rect();
    assert!(
        (rect.left - origin.x).abs() <= 2,
        "bar at {} vs insertion point {}",
        rect.left,
        origin.x
    );
    assert_eq!(rect.bottom - rect.top, layout.height);
    crate::window::tab_drag::cancel(window.hwnd);
}

#[test]
fn near_a_content_edge_the_half_the_new_group_takes_is_tinted() {
    // Break caught: no zone highlight near the edges, a tint over the whole group for an edge
    // (the user cannot tell a split from a move), the wrong half, or a tint where the drop
    // does nothing.
    use windows_sys::Win32::UI::WindowsAndMessaging::WM_MOUSEMOVE;
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    execute_command(window.hwnd, CommandId::New);
    let group = app_mut(window.hwnd).tabs.active_group();
    let area = super::super::with_group_id(window.hwnd, group, |state| state.content).unwrap();
    let area = RECT {
        left: area.left,
        top: area.top,
        right: area.right,
        bottom: area.bottom,
    };
    let screen = |source: HWND, rect: RECT| {
        let mut corners = [
            windows_sys::Win32::Foundation::POINT {
                x: rect.left,
                y: rect.top,
            },
            windows_sys::Win32::Foundation::POINT {
                x: rect.right,
                y: rect.bottom,
            },
        ];
        unsafe {
            windows_sys::Win32::Graphics::Gdi::MapWindowPoints(
                source,
                std::ptr::null_mut(),
                corners.as_mut_ptr(),
                2,
            )
        };
        (corners[0].x, corners[0].y, corners[1].x, corners[1].y)
    };
    let tint = || {
        app_mut(window.hwnd).drop_overlay.map(|overlay| {
            let rect = overlay.rect();
            (rect.left, rect.top, rect.right, rect.bottom)
        })
    };
    let (width, height) = (area.right - area.left, area.bottom - area.top);
    let source = start_strip_drag(window.hwnd, group, 0);

    // The right edge: the right half.
    let point = client_lparam(area.right - 5, (area.top + area.bottom) / 2);
    unsafe { SendMessageW(source, WM_MOUSEMOVE, 1, point) };
    assert_eq!(
        tint(),
        Some(screen(
            source,
            RECT {
                left: area.right - width / 2,
                ..area
            }
        ))
    );
    // The bottom edge: the bottom half; the zone follows the pointer.
    let point = client_lparam((area.left + area.right) / 2, area.bottom - 5);
    unsafe { SendMessageW(source, WM_MOUSEMOVE, 1, point) };
    assert_eq!(
        tint(),
        Some(screen(
            source,
            RECT {
                top: area.bottom - height / 2,
                ..area
            }
        ))
    );
    crate::window::tab_drag::cancel(window.hwnd);
    assert!(app_mut(window.hwnd).drop_overlay.is_none());

    // A lone tab over its own edge would do nothing: no tint.
    execute_command(window.hwnd, CommandId::CloseTab);
    let source = start_strip_drag(window.hwnd, group, 0);
    let point = client_lparam(area.right - 5, (area.top + area.bottom) / 2);
    unsafe { SendMessageW(source, WM_MOUSEMOVE, 1, point) };
    assert!(tint().is_none());
    crate::window::tab_drag::cancel(window.hwnd);
}

/// Two groups side by side, the second showing the first's document; the second active.
fn two_groups(hwnd: HWND) -> (GroupId, GroupId) {
    let first = app_mut(hwnd).tabs.active_group();
    execute_command(hwnd, CommandId::SplitRight);
    (first, app_mut(hwnd).tabs.active_group())
}

fn group_window(hwnd: HWND, id: GroupId) -> HWND {
    super::super::with_group_id(hwnd, id, |state| state.hwnd).unwrap()
}

#[test]
fn a_tab_dropped_on_another_groups_strip_moves_there_at_that_point() {
    // Break caught: a cross-group drop appending, keeping the source view, or leaving the
    // focus in the source group.
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    let (first, second) = two_groups(window.hwnd);
    execute_command(window.hwnd, CommandId::New);
    let moving = app_mut(window.hwnd).tabs.active().unwrap().id;
    assert!(super::super::activate_group(window.hwnd, first));
    execute_command(window.hwnd, CommandId::New);
    let dragged = app_mut(window.hwnd).tabs.active().unwrap().id;
    let index = strip_ids(window.hwnd, first)
        .iter()
        .position(|id| *id == dragged)
        .unwrap();
    let source = start_strip_drag(window.hwnd, first, index);
    let target = group_window(window.hwnd, second);
    let layout = super::super::strip_layout_of(window.hwnd, second).unwrap();
    drop_strip_drag(source, target, layout.insertion_x(1) + 1, 10, 0);
    assert_eq!(strip_ids(window.hwnd, second)[1], dragged);
    assert!(strip_ids(window.hwnd, second).contains(&moving));
    assert!(!strip_ids(window.hwnd, first).contains(&dragged));
    assert_eq!(app_mut(window.hwnd).tabs.active_group(), second);
    assert_eq!(
        unsafe { windows_sys::Win32::UI::Input::KeyboardAndMouse::GetFocus() },
        super::super::group_editor(window.hwnd, second)
            .unwrap()
            .hwnd()
    );
}

#[test]
fn a_ctrl_drop_on_another_group_adds_a_view_and_keeps_the_source() {
    // Break caught: Ctrl ignored, so a copy drag takes the tab away from where it was.
    const MK_CONTROL: usize = 0x0008;
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    let (first, second) = two_groups(window.hwnd);
    assert!(super::super::activate_group(window.hwnd, first));
    execute_command(window.hwnd, CommandId::New);
    let dragged = app_mut(window.hwnd).tabs.active().unwrap().id;
    let index = strip_ids(window.hwnd, first)
        .iter()
        .position(|id| *id == dragged)
        .unwrap();
    let source = start_strip_drag(window.hwnd, first, index);
    let target = group_window(window.hwnd, second);
    let middle = content(window.hwnd, second);
    drop_strip_drag(
        source,
        target,
        (middle.left + middle.right) / 2,
        (middle.top + middle.bottom) / 2,
        MK_CONTROL,
    );
    assert!(strip_ids(window.hwnd, first).contains(&dragged));
    assert_eq!(strip_ids(window.hwnd, second).last(), Some(&dragged));
    assert_eq!(app_mut(window.hwnd).tabs.views_of(dragged).len(), 2);
}

#[test]
fn a_drop_where_the_document_is_already_open_activates_that_view_and_still_moves() {
    // Break caught (spec §6.2): a second view of one document in one group, or the source
    // view left behind by a move.
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    let (first, second) = two_groups(window.hwnd);
    let shared = strip_ids(window.hwnd, second)[0];
    execute_command(window.hwnd, CommandId::New);
    assert!(super::super::activate_group(window.hwnd, first));
    execute_command(window.hwnd, CommandId::New);
    let source = start_strip_drag(window.hwnd, first, 0);
    let target = group_window(window.hwnd, second);
    let middle = content(window.hwnd, second);
    drop_strip_drag(
        source,
        target,
        (middle.left + middle.right) / 2,
        (middle.top + middle.bottom) / 2,
        0,
    );
    assert_eq!(
        strip_ids(window.hwnd, second)
            .iter()
            .filter(|id| **id == shared)
            .count(),
        1
    );
    assert_eq!(
        app_mut(window.hwnd)
            .tabs
            .group(second)
            .unwrap()
            .active_document(),
        Some(shared)
    );
    assert!(!strip_ids(window.hwnd, first).contains(&shared));
}

#[test]
fn dragging_a_groups_last_tab_to_another_group_closes_the_source_group() {
    // Break caught (Review Focus 4): an empty group left behind after its last tab moved.
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    let (first, second) = two_groups(window.hwnd);
    execute_command(window.hwnd, CommandId::New);
    let dragged = app_mut(window.hwnd).tabs.active().unwrap().id;
    let index = strip_ids(window.hwnd, second)
        .iter()
        .position(|id| *id == dragged)
        .unwrap();
    let other = strip_ids(window.hwnd, second)[1 - index];
    // Leave `dragged` alone in the second group.
    super::super::focus_view(window.hwnd, second, other);
    super::super::close_document_tab(window.hwnd, other);
    let source = start_strip_drag(window.hwnd, second, 0);
    let target = group_window(window.hwnd, first);
    let middle = content(window.hwnd, first);
    drop_strip_drag(
        source,
        target,
        (middle.left + middle.right) / 2,
        (middle.top + middle.bottom) / 2,
        0,
    );
    assert_eq!(super::super::group_order(window.hwnd), [first]);
    assert!(strip_ids(window.hwnd, first).contains(&dragged));
}

#[test]
fn a_lone_tab_dropped_on_its_own_edge_or_middle_does_nothing() {
    // Break caught (Review Focus 4): a one-tab group splitting itself and closing, which
    // shuffles the layout for nothing.
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    let group = app_mut(window.hwnd).tabs.active_group();
    let ids = strip_ids(window.hwnd, group);
    let area = content(window.hwnd, group);
    let own = group_window(window.hwnd, group);
    let source = start_strip_drag(window.hwnd, group, 0);
    drop_strip_drag(source, own, area.right - 5, (area.top + area.bottom) / 2, 0);
    assert_eq!(super::super::group_order(window.hwnd), [group]);
    let source = start_strip_drag(window.hwnd, group, 0);
    drop_strip_drag(
        source,
        own,
        (area.left + area.right) / 2,
        (area.top + area.bottom) / 2,
        0,
    );
    assert_eq!(strip_ids(window.hwnd, group), ids);
}

#[test]
fn a_tab_dropped_on_an_edge_splits_that_way_and_moves_into_the_new_group() {
    // Break caught: the split going the wrong way, or the view copied instead of moved.
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    execute_command(window.hwnd, CommandId::New);
    let group = app_mut(window.hwnd).tabs.active_group();
    let dragged = strip_ids(window.hwnd, group)[0];
    let area = content(window.hwnd, group);
    let own = group_window(window.hwnd, group);
    let source = start_strip_drag(window.hwnd, group, 0);
    drop_strip_drag(
        source,
        own,
        (area.left + area.right) / 2,
        area.bottom - 5,
        0,
    );
    let order = super::super::group_order(window.hwnd);
    assert_eq!(order.len(), 2);
    let new = order[1];
    assert_eq!(strip_ids(window.hwnd, new), [dragged]);
    assert!(!strip_ids(window.hwnd, group).contains(&dragged));
    let layout = super::super::tree_layout(window.hwnd).unwrap();
    assert!(
        layout.rect_of(new).unwrap().top > layout.rect_of(group).unwrap().top,
        "below"
    );
}

#[test]
fn an_edge_drop_without_room_says_so_and_leaves_the_tab_where_it_was() {
    // Break caught (Review Focus 5): the view removed from its group before the split was
    // refused, losing the tab.
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    execute_command(window.hwnd, CommandId::New);
    let group = app_mut(window.hwnd).tabs.active_group();
    let ids = strip_ids(window.hwnd, group);
    let own = group_window(window.hwnd, group);
    let source = start_strip_drag(window.hwnd, group, 0);
    // Shrink the window below two minimum-width groups mid-drag.
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::SetWindowPos(
            window.hwnd,
            std::ptr::null_mut(),
            0,
            0,
            300,
            400,
            windows_sys::Win32::UI::WindowsAndMessaging::SWP_NOMOVE
                | windows_sys::Win32::UI::WindowsAndMessaging::SWP_NOZORDER,
        );
    }
    super::super::layout_editor_and_find_bar(window.hwnd);
    let area = content(window.hwnd, group);
    drop_strip_drag(source, own, area.right - 3, (area.top + area.bottom) / 2, 0);
    assert_eq!(super::super::group_order(window.hwnd), [group]);
    assert_eq!(strip_ids(window.hwnd, group), ids);
    assert!(
        notices(window.hwnd)
            .iter()
            .any(|notice| notice.contains(super::super::NO_ROOM_TO_SPLIT))
    );
}

#[test]
fn a_tab_closed_mid_drag_drops_nothing() {
    // Break caught (Review Focus 2): a stale id moving some other tab, or a panic.
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    let (first, second) = two_groups(window.hwnd);
    assert!(super::super::activate_group(window.hwnd, first));
    execute_command(window.hwnd, CommandId::New);
    let dragged = app_mut(window.hwnd).tabs.active().unwrap().id;
    let index = strip_ids(window.hwnd, first)
        .iter()
        .position(|id| *id == dragged)
        .unwrap();
    let source = start_strip_drag(window.hwnd, first, index);
    super::super::close_document_without_prompt(window.hwnd, dragged);
    let before = strip_ids(window.hwnd, second);
    let target = group_window(window.hwnd, second);
    let middle = content(window.hwnd, second);
    drop_strip_drag(
        source,
        target,
        (middle.left + middle.right) / 2,
        (middle.top + middle.bottom) / 2,
        0,
    );
    assert_eq!(strip_ids(window.hwnd, second), before);
    assert!(app_mut(window.hwnd).drop_overlay.is_none());
}

#[test]
fn a_dirty_document_moves_between_groups_without_a_prompt_and_stays_dirty() {
    // Break caught: a move implemented as close plus open, asking to save.
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    let (first, second) = two_groups(window.hwnd);
    assert!(super::super::activate_group(window.hwnd, first));
    execute_command(window.hwnd, CommandId::New);
    editor.set_text("unsaved").unwrap();
    let dragged = app_mut(window.hwnd).tabs.active().unwrap().id;
    app_mut(window.hwnd).tabs.set_dirty(dragged, true);
    assert!(app_mut(window.hwnd).tabs.document(dragged).unwrap().dirty);
    let index = strip_ids(window.hwnd, first)
        .iter()
        .position(|id| *id == dragged)
        .unwrap();
    let source = start_strip_drag(window.hwnd, first, index);
    let target = group_window(window.hwnd, second);
    let middle = content(window.hwnd, second);
    drop_strip_drag(
        source,
        target,
        (middle.left + middle.right) / 2,
        (middle.top + middle.bottom) / 2,
        0,
    );
    assert!(crate::window::modal::take_last_confirm().is_none());
    assert!(app_mut(window.hwnd).tabs.document(dragged).unwrap().dirty);
    assert!(strip_ids(window.hwnd, second).contains(&dragged));
}

#[test]
fn a_strip_tab_dropped_on_a_notebook_folder_copies_its_file() {
    // Break caught: strip drags ignoring the tree the Open Editors rows already copy into
    // (split editors spec §6.1).
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("strip-drag-folder");
    std::fs::create_dir_all(scratch.folder().join("work")).unwrap();
    scratch.note(r"work\b.md", "b");
    let outside = scratch.root.join("draft.txt");
    std::fs::write(&outside, "draft").unwrap();
    let (window, _editor) = notebook_window(&scratch);
    super::super::open_path(window.hwnd, &outside).unwrap();
    let group = app_mut(window.hwnd).tabs.active_group();
    let index = strip_ids(window.hwnd, group)
        .iter()
        .position(|id| {
            app_mut(window.hwnd)
                .tabs
                .document(*id)
                .unwrap()
                .path
                .as_deref()
                == Some(outside.as_path())
        })
        .unwrap();
    let source = start_strip_drag(window.hwnd, group, index);
    let panel = sidebar_windows(window.hwnd).1;
    let work = row_lparam(window.hwnd, &RowKind::Folder("work".into()));
    let (x, y) = (
        (work & 0xffff) as i16 as i32,
        ((work >> 16) & 0xffff) as i16 as i32,
    );
    drop_strip_drag(source, panel, x, y, 0);
    crate::window::copy_host::wait_for_copies(window.hwnd);
    assert_eq!(
        std::fs::read_to_string(scratch.folder().join(r"work\draft.txt")).unwrap(),
        "draft"
    );
    assert!(
        strip_ids(window.hwnd, group).iter().any(|id| {
            app_mut(window.hwnd)
                .tabs
                .document(*id)
                .unwrap()
                .path
                .as_deref()
                == Some(outside.as_path())
        }),
        "the tab stays"
    );
    assert!(notebook_view(window.hwnd).drag.is_none());
}

#[test]
fn files_dropped_on_a_second_groups_editor_open_in_that_group() {
    // Break caught: only group 1's editor taking Explorer drops, so a drop on the right-hand
    // editor opens on the left (split editors spec §6.2).
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("editor-drop-group");
    let note = scratch.note("dropped.md", "dropped");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    let first = app_mut(window.hwnd).tabs.active_group();
    execute_command(window.hwnd, CommandId::SplitRight);
    let second = app_mut(window.hwnd).tabs.active_group();
    assert!(super::super::activate_group(window.hwnd, first));
    crate::window::library_host::accept_editor_file_drops(window.hwnd);
    let target = super::super::group_editor(window.hwnd, second).unwrap();
    crate::editor::file_drop::test_support::drag_and_drop(target.hwnd(), &[note.as_path()]);
    pump_until(window.hwnd, || {
        app_mut(window.hwnd)
            .tabs
            .group_documents(second)
            .iter()
            .any(|document| document.path.as_deref() == Some(note.as_path()))
    });
    assert_eq!(app_mut(window.hwnd).tabs.active_group(), second);
}

#[test]
fn a_group_split_off_after_the_chrome_takes_explorer_drops() {
    // Break caught: the wrapper installed once in BUILD_CHROME, so every later group's
    // editor refuses files.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("editor-drop-late");
    let note = scratch.note("late.md", "late");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    crate::window::library_host::accept_editor_file_drops(window.hwnd);
    execute_command(window.hwnd, CommandId::SplitRight);
    let second = app_mut(window.hwnd).tabs.active_group();
    let target = super::super::group_editor(window.hwnd, second).unwrap();
    let effects =
        crate::editor::file_drop::test_support::drag_and_drop(target.hwnd(), &[note.as_path()]);
    assert_eq!(
        effects,
        [windows_sys::Win32::System::Ole::DROPEFFECT_COPY; 3]
    );
    pump_until(window.hwnd, || {
        app_mut(window.hwnd)
            .tabs
            .group_documents(second)
            .iter()
            .any(|document| document.path.as_deref() == Some(note.as_path()))
    });
}

#[test]
fn files_dropped_on_a_groups_strip_open_in_that_group() {
    // Break caught: WM_DROPFILES (a drop on a strip, a preview or an image) always opening
    // in the active group.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("strip-drop");
    let note = scratch.note("strip.md", "strip");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    let first = app_mut(window.hwnd).tabs.active_group();
    execute_command(window.hwnd, CommandId::SplitRight);
    let second = app_mut(window.hwnd).tabs.active_group();
    assert!(super::super::activate_group(window.hwnd, first));
    let strip = super::super::with_group_id(window.hwnd, second, |state| state.hwnd).unwrap();
    let mut point = windows_sys::Win32::Foundation::POINT { x: 20, y: 10 };
    unsafe {
        windows_sys::Win32::Graphics::Gdi::MapWindowPoints(strip, window.hwnd, &mut point, 1)
    };
    let drop = crate::platform::win32::test_hdrop_at(&[note.as_path()], point);
    unsafe { SendMessageW(window.hwnd, super::super::WM_DROPFILES, drop as usize, 0) };
    assert!(
        app_mut(window.hwnd)
            .tabs
            .group_documents(second)
            .iter()
            .any(|document| document.path.as_deref() == Some(note.as_path()))
    );
}

#[test]
fn a_left_release_after_a_right_press_cancel_drops_nothing_and_opens_no_menu() {
    // Break caught: the right press hides the drag, but releasing the left button still
    // drops the tab, and the right release then falls through to a context menu.
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        WM_LBUTTONUP, WM_MOUSEMOVE, WM_RBUTTONDOWN, WM_RBUTTONUP,
    };
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    let (first, second) = two_groups(window.hwnd);
    assert!(super::super::activate_group(window.hwnd, first));
    execute_command(window.hwnd, CommandId::New);
    let dragged = app_mut(window.hwnd).tabs.active().unwrap().id;
    let index = strip_ids(window.hwnd, first)
        .iter()
        .position(|id| *id == dragged)
        .unwrap();
    let menu = std::rc::Rc::new(std::cell::Cell::new(false));
    let shown = menu.clone();
    crate::window::menus::answer_next_popup_menu(move |_| {
        shown.set(true);
        None
    });
    let source = start_strip_drag(window.hwnd, first, index);
    let target = group_window(window.hwnd, second);
    let middle = content(window.hwnd, second);
    let point = lparam_in(
        source,
        target,
        (middle.left + middle.right) / 2,
        (middle.top + middle.bottom) / 2,
    );
    unsafe {
        SendMessageW(source, WM_MOUSEMOVE, 1, point);
        SendMessageW(source, WM_RBUTTONDOWN, 3, point);
        SendMessageW(source, WM_LBUTTONUP, 2, point);
        SendMessageW(source, WM_RBUTTONUP, 0, point);
    }
    assert!(strip_ids(window.hwnd, first).contains(&dragged));
    assert!(!strip_ids(window.hwnd, second).contains(&dragged));
    assert!(!menu.get(), "the right release opened a menu");
    assert!(app_mut(window.hwnd).tab_drag.is_none());
}

#[test]
fn a_right_press_cancel_over_a_notebook_folder_ends_the_trees_drag() {
    // Break caught: the folder's band and the tree's drag timer left running after the tab
    // drag was cancelled.
    use windows_sys::Win32::UI::WindowsAndMessaging::{WM_MOUSEMOVE, WM_RBUTTONDOWN, WM_RBUTTONUP};
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("strip-drag-right-cancel");
    std::fs::create_dir_all(scratch.folder().join("work")).unwrap();
    scratch.note(r"work\b.md", "b");
    let outside = scratch.root.join("draft.txt");
    std::fs::write(&outside, "draft").unwrap();
    let (window, _editor) = notebook_window(&scratch);
    super::super::open_path(window.hwnd, &outside).unwrap();
    let group = app_mut(window.hwnd).tabs.active_group();
    let index = strip_ids(window.hwnd, group)
        .iter()
        .position(|id| {
            app_mut(window.hwnd)
                .tabs
                .document(*id)
                .unwrap()
                .path
                .as_deref()
                == Some(outside.as_path())
        })
        .unwrap();
    let source = start_strip_drag(window.hwnd, group, index);
    let panel = sidebar_windows(window.hwnd).1;
    let work = row_lparam(window.hwnd, &RowKind::Folder("work".into()));
    let (x, y) = (
        (work & 0xffff) as i16 as i32,
        ((work >> 16) & 0xffff) as i16 as i32,
    );
    let point = lparam_in(source, panel, x, y);
    unsafe { SendMessageW(source, WM_MOUSEMOVE, 1, point) };
    assert!(notebook_view(window.hwnd).drag.is_some(), "over the folder");
    unsafe { SendMessageW(source, WM_RBUTTONDOWN, 3, point) };
    assert!(notebook_view(window.hwnd).drag.is_none());
    unsafe { SendMessageW(source, WM_RBUTTONUP, 0, point) };
}

#[test]
fn a_strip_tab_dropped_on_a_folder_asks_to_replace_with_the_drag_already_gone() {
    // Break caught: the "Replace?" question opening under a frozen drag label with the group
    // window still holding the mouse.
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("strip-drag-replace");
    std::fs::create_dir_all(scratch.folder().join("work")).unwrap();
    scratch.note(r"work\draft.txt", "old");
    let outside = scratch.root.join("draft.txt");
    std::fs::write(&outside, "draft").unwrap();
    let (window, _editor) = notebook_window(&scratch);
    super::super::open_path(window.hwnd, &outside).unwrap();
    let group = app_mut(window.hwnd).tabs.active_group();
    let index = strip_ids(window.hwnd, group)
        .iter()
        .position(|id| {
            app_mut(window.hwnd)
                .tabs
                .document(*id)
                .unwrap()
                .path
                .as_deref()
                == Some(outside.as_path())
        })
        .unwrap();
    let asked = std::rc::Rc::new(std::cell::Cell::new(None));
    let seen = asked.clone();
    crate::window::modal::answer_next_confirm(move |_| {
        let capture = unsafe { windows_sys::Win32::UI::Input::KeyboardAndMouse::GetCapture() };
        seen.set(Some(capture.is_null()));
        false
    });
    let source = start_strip_drag(window.hwnd, group, index);
    let panel = sidebar_windows(window.hwnd).1;
    let work = row_lparam(window.hwnd, &RowKind::Folder("work".into()));
    let (x, y) = (
        (work & 0xffff) as i16 as i32,
        ((work >> 16) & 0xffff) as i16 as i32,
    );
    drop_strip_drag(source, panel, x, y, 0);
    assert_eq!(
        asked.get(),
        Some(true),
        "asked, with the capture already released"
    );
    assert_eq!(
        std::fs::read_to_string(scratch.folder().join(r"work\draft.txt")).unwrap(),
        "old"
    );
}

#[test]
fn files_dropped_on_a_groups_edge_open_in_a_group_split_off_it() {
    // Break caught: an Explorer drop on an editor's edge opening in that group, though the
    // overlay (and a tab dropped there) splits it.
    use windows_sys::Win32::Foundation::{POINT, POINTL};
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("editor-drop-edge");
    let note = scratch.note("edge.md", "edge");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    crate::window::library_host::accept_editor_file_drops(window.hwnd);
    let first = app_mut(window.hwnd).tabs.active_group();
    let groups = app_mut(window.hwnd).tabs.group_ids().len();
    let editor = super::super::group_editor(window.hwnd, first).unwrap();
    let mut client = windows_sys::Win32::Foundation::RECT::default();
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect(editor.hwnd(), &mut client)
    };
    let mut point = POINT {
        x: client.right - 2,
        y: client.bottom / 2,
    };
    unsafe { windows_sys::Win32::Graphics::Gdi::ClientToScreen(editor.hwnd(), &mut point) };
    crate::editor::file_drop::test_support::drag_and_drop_at(
        editor.hwnd(),
        &[note.as_path()],
        POINTL {
            x: point.x,
            y: point.y,
        },
    );
    pump_until(window.hwnd, || {
        app_mut(window.hwnd).tabs.group_ids().len() == groups + 1
    });
    let new = app_mut(window.hwnd).tabs.active_group();
    assert_ne!(new, first);
    assert!(
        app_mut(window.hwnd)
            .tabs
            .group_documents(new)
            .iter()
            .any(|document| document.path.as_deref() == Some(note.as_path()))
    );
}

#[test]
fn files_dropped_on_a_groups_strip_through_ole_open_there_as_normal_tabs() {
    // Break caught: only the editor taking Explorer's OLE drag, so a drag over the group's strip
    // or margins shows no overlay and opens through WM_DROPFILES; or a dropped file left as the
    // preview tab.
    use windows_sys::Win32::Foundation::{POINT, POINTL};
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("group-ole-drop");
    let note = scratch.note("strip-ole.md", "strip");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    let first = app_mut(window.hwnd).tabs.active_group();
    execute_command(window.hwnd, CommandId::SplitRight);
    let second = app_mut(window.hwnd).tabs.active_group();
    assert!(super::super::activate_group(window.hwnd, first));
    crate::window::library_host::accept_editor_file_drops(window.hwnd);
    let strip = super::super::with_group_id(window.hwnd, second, |state| state.hwnd).unwrap();
    assert!(crate::editor::file_drop::is_file_drop_target(
        crate::editor::file_drop::registered_target(strip)
    ));
    let mut point = POINT { x: 20, y: 10 };
    unsafe { windows_sys::Win32::Graphics::Gdi::ClientToScreen(strip, &mut point) };
    crate::editor::file_drop::test_support::drag_and_drop_at(
        strip,
        &[note.as_path()],
        POINTL {
            x: point.x,
            y: point.y,
        },
    );
    pump_until(window.hwnd, || {
        app_mut(window.hwnd)
            .tabs
            .group_documents(second)
            .iter()
            .any(|document| document.path.as_deref() == Some(note.as_path()))
    });
    assert!(
        app_mut(window.hwnd)
            .tabs
            .group_documents(second)
            .iter()
            .all(|document| !document.preview)
    );
    assert!(
        app_mut(window.hwnd).drop_overlay.is_none(),
        "gone after the drop"
    );
}

#[test]
fn an_explorer_drag_over_a_group_shows_the_overlay_until_it_leaves() {
    // Break caught: Explorer drags over an editor or a strip showing no overlay, though a tab
    // dragged there shows one; or the overlay left behind after the drag goes.
    use windows_sys::Win32::Foundation::{POINT, POINTL};
    let _scintilla = load_native_scintilla();
    let scratch = LibraryScratch::new("group-ole-hover");
    let note = scratch.note("hover.md", "hover");
    let window = ProductionWindow::new(make_app());
    let _editor = install_test_editor(&window);
    let group = app_mut(window.hwnd).tabs.active_group();
    crate::window::library_host::accept_editor_file_drops(window.hwnd);
    let editor = super::super::group_editor(window.hwnd, group)
        .unwrap()
        .hwnd();
    let strip = super::super::with_group_id(window.hwnd, group, |state| state.hwnd).unwrap();
    for (window_under, local) in [
        (editor, POINT { x: 60, y: 60 }),
        (strip, POINT { x: 20, y: 10 }),
    ] {
        let mut point = local;
        unsafe { windows_sys::Win32::Graphics::Gdi::ClientToScreen(window_under, &mut point) };
        crate::editor::file_drop::test_support::hover_at(
            window_under,
            &[note.as_path()],
            POINTL {
                x: point.x,
                y: point.y,
            },
            || {
                assert!(
                    app_mut(window.hwnd).drop_overlay.is_some(),
                    "shown while hovering"
                )
            },
        );
        assert!(
            app_mut(window.hwnd).drop_overlay.is_none(),
            "gone when it leaves"
        );
    }
}
