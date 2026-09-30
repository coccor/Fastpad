//! The window title and each group's tab strip: tab snapshots, strip layout and scrolling, the
//! strip's pointer messages and accessible object, and refreshing the tabs.

use super::*;

pub(super) const EMPTY_TABS_HINT: &str =
    "No tabs are open.\nPress Ctrl+N or double-click the tab bar to start a new one.";

pub(crate) fn tab_count(hwnd: HWND) -> usize {
    unsafe { app_ptr(hwnd) }
        .map(|app| unsafe { app.as_ref() }.tabs.len())
        .unwrap_or(1)
}

pub(crate) fn title_layout(hwnd: HWND) -> TitleBarLayout {
    crate::window::titlebar::layout_for_window(hwnd)
}

/// Titles, active index, scroll offset, and whether the editor is hidden because no tab is open.
/// The frame's window text, which the taskbar button and Alt+Tab show: the active tab's title as
/// the tab strip paints it, followed by the app name.
pub(super) fn window_title(active_tab: Option<&str>) -> String {
    active_tab.map_or_else(
        || "FastPad".to_owned(),
        |title| format!("{title} - FastPad"),
    )
}

/// The window title for the active tab, as the title bar and the taskbar show it.
fn active_window_title(hwnd: HWND) -> String {
    window_title(
        unsafe { app_ptr(hwnd) }
            .and_then(|app| unsafe { app.as_ref() }.tabs.active().map(Document::title))
            .as_deref(),
    )
}

/// Brings the window text in line with the active tab. Runs on every frame paint, since every tab
/// change (switch, open, close, save, dirty state) repaints the title strip.
pub(super) fn sync_window_title(hwnd: HWND) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetWindowTextW, SetWindowTextW};
    if unsafe { app_ptr(hwnd) }.is_none() {
        return;
    }
    let title = active_window_title(hwnd);
    let wanted = title.encode_utf16().collect::<Vec<_>>();
    // One spare unit so a longer current title never reads as equal after truncation.
    let mut current = vec![0u16; wanted.len() + 2];
    let len = unsafe { GetWindowTextW(hwnd, current.as_mut_ptr(), current.len() as i32) };
    if current[..len.max(0) as usize] != wanted[..] {
        unsafe {
            SetWindowTextW(hwnd, wide_null(&title).as_ptr());
        }
    }
}

/// Group `id`'s tab titles, selected tab, strip scroll, whether it is empty, and its preview tab.
pub(super) fn tab_snapshot(
    hwnd: HWND,
    id: GroupId,
) -> (Vec<String>, usize, i32, bool, Option<usize>) {
    unsafe { app_ptr(hwnd) }
        .and_then(|app| {
            let app = unsafe { app.as_ref() };
            let group = app.tabs.group(id)?;
            let documents = app.tabs.group_documents(id);
            Some((
                documents.iter().map(|document| document.title()).collect(),
                group.active_index(),
                group.scroll_offset(),
                app.group(id).is_some() && group.is_empty(),
                documents.iter().position(|document| document.preview),
            ))
        })
        .unwrap_or_else(|| (vec!["Untitled".to_owned()], 0, 0, false, None))
}

pub(super) fn group_tab_count(hwnd: HWND, id: GroupId) -> usize {
    unsafe { app_ptr(hwnd) }
        .and_then(|app| Some(unsafe { app.as_ref() }.tabs.group(id)?.len()))
        .unwrap_or(0)
}

/// Runs `f` on the active editor group's window state.
pub(crate) fn with_group<R>(
    hwnd: HWND,
    f: impl FnOnce(&mut crate::window::editor_group::GroupWindow) -> R,
) -> Option<R> {
    let mut app = unsafe { app_ptr(hwnd) }?;
    unsafe { app.as_mut() }.active_group_mut().map(f)
}

/// The title-row strip in the main window's client coordinates, which the frame leaves to the
/// group when it paints the title row.
pub(super) fn group_strip_bounds(hwnd: HWND) -> Vec<crate::window::titlebar::Rect> {
    let groups = unsafe { app_ptr(hwnd) }
        .map(|app| {
            unsafe { app.as_ref() }
                .groups
                .iter()
                .map(|group| (group.id, group.hwnd))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    groups
        .into_iter()
        .filter_map(|(id, group)| {
            let bounds = strip_layout_of(hwnd, id)?.bounds();
            let mut origin = windows_sys::Win32::Foundation::POINT { x: 0, y: 0 };
            unsafe {
                windows_sys::Win32::Graphics::Gdi::MapWindowPoints(group, hwnd, &mut origin, 1)
            };
            (origin.y == 0).then(|| {
                crate::window::titlebar::Rect::new(
                    origin.x,
                    0,
                    origin.x + bounds.right,
                    bounds.bottom,
                )
            })
        })
        .collect()
}

/// The active group's tab strip as laid out now, the same one it paints and hit-tests with.
pub(crate) fn strip_layout(hwnd: HWND) -> Option<crate::window::group_strip::StripLayout> {
    let id = unsafe { app_ptr(hwnd) }.map(|app| unsafe { app.as_ref() }.tabs.active_group())?;
    strip_layout_of(hwnd, id)
}

/// Group `id`'s tab strip as laid out now.
pub(crate) fn strip_layout_of(
    hwnd: HWND,
    id: GroupId,
) -> Option<crate::window::group_strip::StripLayout> {
    let (group, count, scroll) = unsafe { app_ptr(hwnd) }.and_then(|app| {
        let app = unsafe { app.as_ref() };
        let tabs = app.tabs.group(id)?;
        Some((app.group(id)?.hwnd, tabs.len(), tabs.scroll_offset()))
    })?;
    let dpi = unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForWindow(group) }.max(96);
    Some(crate::window::group_strip::StripLayout::calculate(
        crate::window::group_strip::strip_width(group),
        dpi,
        count,
        scroll,
    ))
}

pub(crate) fn invalidate_group_strip(hwnd: HWND, id: GroupId) {
    let (Some(group), Some(layout)) = (
        with_group_id(hwnd, id, |state| state.hwnd),
        strip_layout_of(hwnd, id),
    ) else {
        return;
    };
    let bounds = layout.bounds();
    let rect = RECT {
        left: bounds.left,
        top: bounds.top,
        right: bounds.right,
        bottom: bounds.bottom,
    };
    unsafe {
        InvalidateRect(group, &rect, 0);
    }
}

pub(crate) fn update_strip_pointer(
    hwnd: HWND,
    id: GroupId,
    update: impl FnOnce(
        crate::window::group_strip::StripPointer,
    ) -> crate::window::group_strip::StripPointer,
) {
    let changed = with_group_id(hwnd, id, |group| {
        let next = update(group.pointer);
        let changed = next != group.pointer;
        group.pointer = next;
        changed
    })
    .unwrap_or(false);
    if changed {
        invalidate_group_strip(hwnd, id);
    }
}

/// What the group-client point is over in group `id`'s strip; `None` below it.
pub(crate) fn strip_target(
    hwnd: HWND,
    id: GroupId,
    x: i32,
    y: i32,
) -> Option<crate::window::group_strip::StripTarget> {
    let layout = strip_layout_of(hwnd, id)?;
    (y >= 0 && y < layout.height)
        .then(|| layout.hit_test(crate::window::titlebar::Point::new(x, y)))
}

/// Scrolls the tabs when the wheel turns over the strip; reports whether it was over it. The
/// point is in screen coordinates, as wheel messages carry it.
fn scroll_tabs(hwnd: HWND, id: GroupId, group: HWND, lparam: LPARAM, delta: i32) -> bool {
    let mut point = windows_sys::Win32::Foundation::POINT {
        x: (lparam as u32 & 0xffff) as u16 as i16 as i32,
        y: ((lparam as u32 >> 16) & 0xffff) as u16 as i16 as i32,
    };
    unsafe {
        windows_sys::Win32::Graphics::Gdi::ScreenToClient(group, &mut point);
    }
    let Some(layout) = strip_layout_of(hwnd, id) else {
        return false;
    };
    if point.y < 0 || point.y >= layout.height || point.x < 0 || point.x >= layout.tabs.right {
        return false;
    }
    let scroll = layout.scroll_by_wheel(delta, WHEEL_DELTA as i32);
    if set_strip_scroll(hwnd, id, scroll) {
        // The hovered tab moved out from under the pointer; the next mouse move finds the new one.
        update_strip_pointer(hwnd, id, |pointer| pointer.hover(None));
        invalidate_group_strip(hwnd, id);
    }
    true
}

/// Scrolls group `id`'s tabs to `offset`; returns whether it moved.
fn set_strip_scroll(hwnd: HWND, id: GroupId, offset: i32) -> bool {
    unsafe { app_ptr(hwnd) }.is_some_and(|app| {
        unsafe { app.as_ref() }
            .tabs
            .group(id)
            .is_some_and(|tabs| tabs.selection().set_scroll_offset(offset))
    })
}

/// Starts dragging the tab scroll thumb. Pressing the track beside the thumb first jumps the
/// thumb there, centred under the pointer, so the same press can keep dragging it.
fn begin_tab_thumb_drag(hwnd: HWND, id: GroupId, group: HWND, x: i32) {
    let Some(layout) = strip_layout_of(hwnd, id) else {
        return;
    };
    let Some(thumb) = layout.scroll_thumb() else {
        return;
    };
    let grab = if (thumb.left..thumb.right).contains(&x) {
        x - thumb.left
    } else {
        let grab = (thumb.right - thumb.left) / 2;
        set_strip_scroll(hwnd, id, layout.scroll_for_thumb(x - grab));
        grab
    };
    with_group_id(hwnd, id, |state| state.thumb_grab = Some(grab));
    unsafe {
        SetCapture(group);
    }
    invalidate_group_strip(hwnd, id);
}

/// Follows the pointer while the thumb is dragged; reports whether a drag is in progress.
fn drag_tab_thumb(hwnd: HWND, id: GroupId, x: i32) -> bool {
    let Some(grab) = with_group_id(hwnd, id, |group| group.thumb_grab).flatten() else {
        return false;
    };
    let Some(layout) = strip_layout_of(hwnd, id) else {
        return true;
    };
    if set_strip_scroll(hwnd, id, layout.scroll_for_thumb(x - grab)) {
        invalidate_group_strip(hwnd, id);
    }
    true
}

/// Ends a thumb drag; reports whether one was in progress, so the release activates nothing.
fn end_tab_thumb_drag(hwnd: HWND, id: GroupId) -> bool {
    let dragging = with_group_id(hwnd, id, |group| group.thumb_grab.take())
        .flatten()
        .is_some();
    if dragging {
        unsafe {
            ReleaseCapture();
        }
    }
    dragging
}

/// Opens the tab-strip menu at group-client `x`, `y`. The menu belongs to the main window, whose
/// modal accounting holds deferred work while it is open.
pub(super) fn show_group_strip_menu(hwnd: HWND, group: HWND, x: i32, y: i32) {
    let mut point = windows_sys::Win32::Foundation::POINT { x, y };
    unsafe {
        windows_sys::Win32::Graphics::Gdi::MapWindowPoints(group, hwnd, &mut point, 1);
    }
    let has_tabs = tab_count(hwnd) > 0;
    if let Some(command) = menus::show_tab_strip_menu(hwnd, point.x, point.y, has_tabs) {
        execute_command(hwnd, command);
    }
}

/// The tab strip's share of the group window's pointer messages (split editors spec §4.2): what
/// the title bar used to do for the tabs. `None` leaves the message to the window's default.
pub(crate) fn group_strip_message(
    hwnd: HWND,
    group: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> Option<LRESULT> {
    use crate::window::group_strip::StripTarget;
    // Everything below acts on this strip's own group; a press has already made it active.
    let id = group_id_of(hwnd, group)?;
    let (x, y) = (
        (lparam as u32 & 0xffff) as u16 as i16 as i32,
        ((lparam as u32 >> 16) & 0xffff) as u16 as i16 as i32,
    );
    // Clicking the strip leaves menu mode, as a click anywhere else off the menu band does.
    if matches!(
        message,
        WM_LBUTTONDOWN | WM_LBUTTONDBLCLK | WM_MBUTTONDOWN | WM_RBUTTONUP
    ) {
        exit_menu_mode(hwnd);
    }
    match message {
        WM_RBUTTONDOWN if crate::window::tab_drag::cancel_for_right_press(hwnd) => Some(0),
        WM_MOUSEMOVE => {
            if drag_tab_thumb(hwnd, id, x) {
                return Some(0);
            }
            if crate::window::tab_drag::mouse_move(hwnd, group, x, y, wparam) {
                return Some(0);
            }
            let mut track = windows_sys::Win32::UI::Input::KeyboardAndMouse::TRACKMOUSEEVENT {
                cbSize: std::mem::size_of::<
                    windows_sys::Win32::UI::Input::KeyboardAndMouse::TRACKMOUSEEVENT,
                >() as u32,
                dwFlags: windows_sys::Win32::UI::Input::KeyboardAndMouse::TME_LEAVE,
                hwndTrack: group,
                dwHoverTime: 0,
            };
            unsafe { windows_sys::Win32::UI::Input::KeyboardAndMouse::TrackMouseEvent(&mut track) };
            let target = strip_target(hwnd, id, x, y);
            update_strip_pointer(hwnd, id, |pointer| pointer.hover(target));
            Some(0)
        }
        WM_MOUSELEAVE => {
            update_strip_pointer(hwnd, id, |pointer| pointer.hover(None));
            // A middle press whose release never reaches the strip must not close a tab later.
            with_group_id(hwnd, id, |state| state.middle_press = None);
            Some(0)
        }
        // The class has CS_DBLCLKS, so a second click comes as a double-click: on empty strip it
        // opens a tab (VS Code style); anywhere else it is one more press.
        WM_LBUTTONDBLCLK if strip_target(hwnd, id, x, y) == Some(StripTarget::Empty) => {
            execute_command(hwnd, CommandId::New);
            Some(0)
        }
        WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
            let target = strip_target(hwnd, id, x, y)?;
            update_strip_pointer(hwnd, id, |pointer| {
                pointer.hover(Some(target)).press(Some(target))
            });
            if target == StripTarget::ScrollBar {
                begin_tab_thumb_drag(hwnd, id, group, x);
            }
            if let StripTarget::Tab(index) = target {
                crate::window::tab_drag::arm(hwnd, id, group, index, x, y);
            }
            Some(0)
        }
        // A release acts only over the target its press went down on, so the release that ends
        // a double-click on empty strip never hits the tab that double-click just opened.
        WM_LBUTTONUP => {
            if crate::window::tab_drag::release(hwnd, group, x, y, wparam) {
                update_strip_pointer(hwnd, id, |pointer| pointer.press(None));
                return Some(0);
            }
            let mut activated = None;
            update_strip_pointer(hwnd, id, |pointer| {
                let (next, released) = pointer.release(strip_target(hwnd, id, x, y));
                activated = released;
                next
            });
            if end_tab_thumb_drag(hwnd, id) {
                return Some(0);
            }
            match activated? {
                StripTarget::CloseTab(index) => {
                    activate_tab(hwnd, index);
                    execute_command(hwnd, CommandId::CloseTab);
                }
                StripTarget::Tab(index) => {
                    activate_tab(hwnd, index);
                    if tab_double_click(hwnd, index)
                        && let Some(id) = unsafe { app_ptr(hwnd) }
                            .and_then(|app| Some(unsafe { app.as_ref() }.tabs.active()?.id))
                    {
                        promote_tab(hwnd, id);
                    }
                }
                StripTarget::ScrollBar | StripTarget::Empty => {}
            }
            Some(0)
        }
        WM_RBUTTONUP if crate::window::tab_drag::right_release(hwnd) => Some(0),
        WM_RBUTTONUP if strip_target(hwnd, id, x, y) == Some(StripTarget::Empty) => {
            show_group_strip_menu(hwnd, group, x, y);
            Some(0)
        }
        // A tab's menu acts on that tab, so it is activated first.
        WM_RBUTTONUP => match strip_target(hwnd, id, x, y) {
            Some(StripTarget::Tab(index) | StripTarget::CloseTab(index)) => {
                activate_tab(hwnd, index);
                let mut point = windows_sys::Win32::Foundation::POINT { x, y };
                unsafe {
                    windows_sys::Win32::Graphics::Gdi::MapWindowPoints(group, hwnd, &mut point, 1)
                };
                if let Some(command) = menus::show_tab_menu(hwnd, point.x, point.y) {
                    execute_command(hwnd, command);
                }
                Some(0)
            }
            _ => None,
        },
        // A middle-click closes the tab under the pointer (quick-open spec §5).
        WM_MBUTTONDOWN => {
            let press = match strip_target(hwnd, id, x, y) {
                Some(StripTarget::Tab(index) | StripTarget::CloseTab(index)) => {
                    tab_id_at(hwnd, index).map(|id| (index, id))
                }
                _ => None,
            };
            with_group_id(hwnd, id, |state| state.middle_press = press);
            Some(0)
        }
        WM_MBUTTONUP => {
            let press = with_group_id(hwnd, id, |state| state.middle_press.take()).flatten();
            // Only over the pressed tab, and only while it still shows the same document.
            if let Some((index, document)) = press
                && let Some(StripTarget::Tab(released) | StripTarget::CloseTab(released)) =
                    strip_target(hwnd, id, x, y)
                && released == index
                && tab_id_at(hwnd, index) == Some(document)
            {
                close_tab_at(hwnd, index);
            }
            Some(0)
        }
        WM_MOUSEWHEEL | WM_MOUSEHWHEEL => {
            let delta = i32::from((wparam >> 16) as u16 as i16);
            // Wheel up scrolls toward the first tab; a tilt to the right toward the last.
            let delta = if message == WM_MOUSEWHEEL {
                -delta
            } else {
                delta
            };
            scroll_tabs(hwnd, id, group, lparam, delta).then_some(0)
        }
        WM_CAPTURECHANGED => {
            with_group_id(hwnd, id, |state| state.thumb_grab = None);
            if lparam as HWND != group {
                crate::window::tab_drag::cancel(hwnd);
            }
            Some(0)
        }
        _ => None,
    }
}

/// The tab strip's accessible object, for the group window's `WM_GETOBJECT`.
pub(crate) fn group_accessible_object(hwnd: HWND, group: HWND, wparam: WPARAM) -> LRESULT {
    let provider = unsafe { app_ptr(hwnd) }.and_then(|mut app| {
        let app = unsafe { app.as_mut() };
        let id = app.groups.iter().find(|state| state.hwnd == group)?.id;
        let tabs = app.tabs.group(id)?;
        let (view, selection) = (tabs.view(), tabs.selection());
        let state = app.group_mut(id)?;
        Some(state.accessibility.ensure(
            group,
            crate::window::accessibility::ProviderKind::GroupStrip,
            view,
            selection,
        ))
    });
    match provider {
        Some(provider) => unsafe { crate::window::accessibility::object_result(provider, wparam) },
        None => 0,
    }
}

/// Follows every change to the set of tabs or the active one: scrolls the active tab into view,
/// shows the editor only while a tab is open, and repaints.
pub(crate) fn refresh_tabs(hwnd: HWND) {
    let Some((count, active, editor_hwnd)) = (unsafe { app_ptr(hwnd) }).map(|app| {
        let app = unsafe { app.as_ref() };
        (
            app.tabs.len(),
            app.tabs.active_index(),
            app.editor().map(Editor::hwnd),
        )
    }) else {
        return;
    };
    let scroll = if count == 0 {
        0
    } else {
        strip_layout(hwnd).map_or(0, |layout| layout.scroll_to_reveal(active))
    };
    if let Some(app) = unsafe { app_ptr(hwnd) } {
        unsafe { app.as_ref() }.tabs.set_scroll_offset(scroll);
    }
    // An image tab shows the image view instead of the editor (image preview spec §5).
    let text_active = count > 0 && !crate::window::image_host::active_is_image(hwnd);
    if let Some(editor_hwnd) = editor_hwnd {
        let visible = unsafe { GetWindowLongPtrW(editor_hwnd, GWL_STYLE) } as u32 & WS_VISIBLE != 0;
        if !text_active && visible {
            close_find_bar(hwnd);
            unsafe {
                ShowWindow(editor_hwnd, SW_HIDE);
                if GetFocus() == editor_hwnd {
                    SetFocus(hwnd);
                }
            }
        } else if text_active && !visible {
            unsafe {
                ShowWindow(editor_hwnd, SW_SHOWNA);
                if GetFocus() == hwnd {
                    SetFocus(editor_hwnd);
                }
            }
        }
    }
    crate::window::library_host::close_stale_name_box(hwnd);
    unsafe {
        InvalidateRect(hwnd, std::ptr::null(), 0);
    }
    crate::window::image_host::sync(hwnd);
    crate::window::preview_host::sync_visibility(hwnd);
    crate::window::library_host::refresh_label(hwnd);
    crate::window::side_panel::active_tab_changed(hwnd);
}
