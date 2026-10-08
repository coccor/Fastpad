//! Layout of the sidebar, name box and editor groups: sashes, creating and destroying group
//! windows, configuring their editors, and each group's own layout, hit test and painting.

use super::*;

/// Lays out the sidebar, then the name box and the editor group right of it, below the title
/// strip. The sole layout choke point for all of them; the group lays out its own children
/// (`layout_group`).
pub(crate) fn layout_editor_and_find_bar(hwnd: HWND) {
    let mut rect = RECT::default();
    unsafe {
        GetClientRect(hwnd, &mut rect);
    }
    let dpi = unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForWindow(hwnd) }.max(96);
    crate::window::side_panel::layout(hwnd, rect, dpi);
    let left = crate::window::side_panel::left_edge(hwnd);
    layout_command_palette(hwnd);
    if group_hwnd(hwnd).is_none() {
        return;
    }
    let title_height = title_layout(hwnd).height + menu_band_height(hwnd);
    let width = (rect.right - rect.left - left).max(0);
    let font = title_chrome(hwnd).1.text();
    // The name box spans the editor area under the title row, over the top groups.
    if let Some(app) = unsafe { app_ptr(hwnd) }
        && let Some(name_box) = unsafe { app.as_ref() }.name_box.as_ref()
    {
        name_box.layout(left, width, title_height, dpi, font);
    }
    let (Some(area), Some(layout)) = (tree_area(hwnd), tree_layout(hwnd)) else {
        return;
    };
    for (id, rect) in &layout.groups {
        let Some(group) = with_group_id(hwnd, *id, |state| state.hwnd) else {
            continue;
        };
        // A top group reaches up into the title row and draws its strip there (spec §4.1); a
        // lower group draws its strip at its own top.
        let top = if rect.top == area.top { 0 } else { rect.top };
        unsafe {
            MoveWindow(
                group,
                rect.left,
                top,
                rect.right - rect.left,
                rect.bottom - top,
                1,
            );
        }
        // A move that keeps the group's size sends no WM_SIZE, but what is inside may have
        // changed.
        layout_group(hwnd, group);
    }
    unsafe { InvalidateRect(hwnd, std::ptr::null(), 0) };
}

/// The sash under the cursor sets the resize cursor; false elsewhere.
pub(super) fn set_sash_cursor(hwnd: HWND) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetCursorPos, IDC_SIZENS, IDC_SIZEWE, LoadCursorW, SetCursor,
    };
    let mut point = windows_sys::Win32::Foundation::POINT { x: 0, y: 0 };
    unsafe {
        GetCursorPos(&mut point);
        windows_sys::Win32::Graphics::Gdi::ScreenToClient(hwnd, &mut point);
    }
    let Some(axis) =
        tree_layout(hwnd).and_then(|layout| layout.sash_at(point.x, point.y).map(|sash| sash.axis))
    else {
        return false;
    };
    let cursor = match axis {
        crate::window::split_tree::Axis::Row => IDC_SIZEWE,
        crate::window::split_tree::Axis::Column => IDC_SIZENS,
    };
    unsafe { SetCursor(LoadCursorW(std::ptr::null_mut(), cursor)) };
    true
}

/// A press on a sash starts dragging it; a second press there within the double-click time
/// shares its branch out evenly instead (spec §4.3). The main window has no `CS_DBLCLKS`, so the
/// double-click is timed here, as the tab strip does. False when no sash is under the point.
pub(super) fn press_sash(hwnd: HWND, x: i32, y: i32) -> bool {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime;
    use windows_sys::Win32::UI::WindowsAndMessaging::GetMessageTime;
    let Some(sash) = tree_layout(hwnd).and_then(|layout| layout.sash_at(x, y).cloned()) else {
        return false;
    };
    let now = unsafe { GetMessageTime() } as u32;
    let double_click_time = unsafe { GetDoubleClickTime() };
    let equalized = unsafe { app_ptr(hwnd) }.is_some_and(|mut app| {
        let app = unsafe { app.as_mut() };
        let double = app.last_sash_click.as_ref().is_some_and(|(id, time)| {
            *id == sash.id && now.wrapping_sub(*time) <= double_click_time
        });
        if double {
            app.last_sash_click = None;
            app.layout.equalize(&sash.id);
        } else {
            app.last_sash_click = Some((sash.id.clone(), now));
            app.sash_drag = Some(sash);
        }
        double
    });
    if equalized {
        layout_editor_and_find_bar(hwnd);
    } else {
        unsafe { SetCapture(hwnd) };
    }
    true
}

/// Moves the dragged sash to the pointer; false when no sash is being dragged.
pub(super) fn drag_sash(hwnd: HWND, lparam: LPARAM) -> bool {
    let (x, y) = (
        (lparam as u32 & 0xffff) as u16 as i16 as i32,
        ((lparam as u32 >> 16) & 0xffff) as u16 as i16 as i32,
    );
    let dpi = unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForWindow(hwnd) }.max(96);
    let moved = unsafe { app_ptr(hwnd) }.and_then(|mut app| {
        let app = unsafe { app.as_mut() };
        let sash = app.sash_drag.clone()?;
        let at = match sash.axis {
            crate::window::split_tree::Axis::Row => x,
            crate::window::split_tree::Axis::Column => y,
        };
        Some(app.layout.set_sash(&sash, at, dpi))
    });
    match moved {
        Some(changed) => {
            if changed {
                layout_editor_and_find_bar(hwnd);
                unsafe { windows_sys::Win32::Graphics::Gdi::UpdateWindow(hwnd) };
            }
            true
        }
        None => false,
    }
}

/// Ends a sash drag; returns whether one was under way.
pub(super) fn end_sash_drag(hwnd: HWND) -> bool {
    unsafe { app_ptr(hwnd) }
        .is_some_and(|mut app| unsafe { app.as_mut() }.sash_drag.take().is_some())
}

/// The editor area the groups share, in the main window's client coordinates: below the title
/// row, above the status bar, right of the sidebar.
pub(crate) fn tree_area(hwnd: HWND) -> Option<crate::window::titlebar::Rect> {
    let mut client = RECT::default();
    unsafe {
        GetClientRect(hwnd, &mut client);
    }
    let left = crate::window::side_panel::left_edge(hwnd);
    let top = title_layout(hwnd).height;
    let bottom = (client.bottom - status_bar_height(hwnd)).max(top + 1);
    (client.right > left)
        .then(|| crate::window::titlebar::Rect::new(left, top, client.right, bottom))
}

/// Where each group and sash goes now.
pub(crate) fn tree_layout(hwnd: HWND) -> Option<crate::window::split_tree::TreeLayout> {
    let area = tree_area(hwnd)?;
    let dpi = unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForWindow(hwnd) }.max(96);
    let app = unsafe { app_ptr(hwnd) }?;
    Some(unsafe { app.as_ref() }.layout.layout(area, dpi))
}

/// The groups in layout order: left to right, top to bottom (spec §4.3).
pub(crate) fn group_order(hwnd: HWND) -> Vec<GroupId> {
    unsafe { app_ptr(hwnd) }
        .map(|app| unsafe { app.as_ref() }.layout.leaves())
        .unwrap_or_default()
}

/// Whether `group`'s window starts at the top of the main window's client area, so its strip is
/// in the title row.
fn is_top_group(hwnd: HWND, group: HWND) -> bool {
    let mut origin = windows_sys::Win32::Foundation::POINT { x: 0, y: 0 };
    unsafe { windows_sys::Win32::Graphics::Gdi::MapWindowPoints(group, hwnd, &mut origin, 1) };
    origin.y == 0
}

/// The active editor group's window, once the editor exists.
pub(crate) fn group_hwnd(hwnd: HWND) -> Option<HWND> {
    let app = unsafe { app_ptr(hwnd) }?;
    unsafe { app.as_ref() }
        .active_group()
        .map(|group| group.hwnd)
}

/// The group whose window is `group`.
/// The group whose window contains screen point `point`, with that window.
pub(crate) fn group_at(
    hwnd: HWND,
    point: windows_sys::Win32::Foundation::POINT,
) -> Option<(GroupId, HWND)> {
    let windows = unsafe { app_ptr(hwnd) }.map(|app| {
        unsafe { app.as_ref() }
            .groups
            .iter()
            .map(|group| (group.id, group.hwnd))
            .collect::<Vec<_>>()
    })?;
    windows.into_iter().find(|(_, window)| {
        let mut rect = RECT::default();
        let shown = unsafe {
            // Its own style: the main window of a test is never shown.
            GetWindowLongPtrW(*window, GWL_STYLE) as u32 & WS_VISIBLE != 0
                && windows_sys::Win32::UI::WindowsAndMessaging::GetWindowRect(*window, &mut rect)
                    != 0
        };
        shown
            && point.x >= rect.left
            && point.x < rect.right
            && point.y >= rect.top
            && point.y < rect.bottom
    })
}

pub(crate) fn group_id_of(hwnd: HWND, group: HWND) -> Option<GroupId> {
    let app = unsafe { app_ptr(hwnd) }?;
    unsafe { app.as_ref() }
        .groups
        .iter()
        .find(|state| state.hwnd == group)
        .map(|state| state.id)
}

/// Runs `f` on group `id`'s window state.
pub(crate) fn with_group_id<R>(
    hwnd: HWND,
    id: GroupId,
    f: impl FnOnce(&mut crate::window::editor_group::GroupWindow) -> R,
) -> Option<R> {
    let mut app = unsafe { app_ptr(hwnd) }?;
    unsafe { app.as_mut() }.group_mut(id).map(f)
}

pub(crate) fn group_editor(hwnd: HWND, id: GroupId) -> Option<Editor> {
    let app = unsafe { app_ptr(hwnd) }?;
    unsafe { app.as_ref() }
        .group(id)
        .map(|group| group.editor.clone())
}

/// Creates an empty group window with its own editor, set up like the others. The caller puts it
/// in the layout (split editors spec §4.2).
pub(crate) fn create_group(hwnd: HWND) -> Result<GroupId> {
    #[cfg(test)]
    if FAIL_GROUP_AFTER.with(|after| match after.get() {
        Some(0) => {
            after.set(None);
            true
        }
        Some(count) => {
            after.set(Some(count - 1));
            false
        }
        None => false,
    }) {
        return Err(crate::FastPadError::Invariant(
            "test: group creation failed",
        ));
    }
    let host = unsafe { app_ptr(hwnd) }
        .and_then(|app| unsafe { app.as_ref() }.document_host.clone())
        .ok_or(crate::FastPadError::Invariant(
            "main window app state was not available",
        ))?;
    let window = crate::window::editor_group::create(hwnd)?;
    let editor = match Editor::create_with_host(window, &host) {
        Ok(editor) => editor,
        Err(error) => {
            unsafe { DestroyWindow(window) };
            return Err(error);
        }
    };
    configure_editor(hwnd, &editor);
    // A new group shows nothing until a view is added.
    unsafe { ShowWindow(editor.hwnd(), SW_HIDE) };
    let Some(mut app) = (unsafe { app_ptr(hwnd) }) else {
        unsafe { DestroyWindow(window) };
        return Err(crate::FastPadError::Invariant(
            "main window app state was not available",
        ));
    };
    let editor_hwnd = editor.hwnd();
    let app = unsafe { app.as_mut() };
    let id = app.tabs.add_group();
    app.groups
        .push(crate::window::editor_group::GroupWindow::new(
            id, window, editor,
        ));
    // A group made after `BUILD_CHROME` takes Explorer drops too (split editors spec §6.2).
    let wrap = app.file_drops_accepted;
    if wrap {
        crate::window::library_host::wrap_group_drop_target(hwnd, window, editor_hwnd);
    }
    Ok(id)
}

#[cfg(test)]
thread_local! {
    /// How many more group windows are made before one fails, for the failure tests.
    pub(super) static FAIL_GROUP_AFTER: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
pub(crate) fn fail_next_group_creation() {
    fail_group_creation_after(0);
}

/// The group window made after `count` more succeed fails.
#[cfg(test)]
pub(crate) fn fail_group_creation_after(count: usize) {
    FAIL_GROUP_AFTER.with(|after| after.set(Some(count)));
}

/// Destroys group `id`'s window, its editor and what it showed. The caller has already emptied it
/// and taken it out of the layout.
pub(crate) fn destroy_group(hwnd: HWND, id: GroupId) {
    if unsafe { app_ptr(hwnd) }.is_some_and(|app| {
        unsafe { app.as_ref() }
            .tab_drag
            .as_ref()
            .is_some_and(|drag| drag.source.group == id)
    }) {
        crate::window::tab_drag::cancel(hwnd);
    }
    let removed = unsafe { app_ptr(hwnd) }.and_then(|mut app| {
        let app = unsafe { app.as_mut() };
        let index = app.groups.iter().position(|group| group.id == id)?;
        app.tabs.remove_group(id).then(|| app.groups.remove(index))
    });
    if let Some(group) = removed {
        let window = group.hwnd;
        drop(group);
        unsafe { DestroyWindow(window) };
    }
}

/// Applies the editor settings, the theme's colours and the shared zoom to `editor`, as every
/// group's editor has them.
fn configure_editor(hwnd: HWND, editor: &Editor) {
    let Some((settings, palette, zoom)) = (unsafe { app_ptr(hwnd) }).map(|app| {
        let app = unsafe { app.as_ref() };
        let palette = Palette::for_cached_theme(app.theme, app.settings.theme);
        let zoom = app.editor().and_then(|editor| editor.zoom().ok());
        (app.settings.clone(), palette, zoom)
    }) else {
        return;
    };
    apply_settings_to(editor, &settings, palette);
    apply_colors_to(editor, palette, settings.highlight_current_line);
    crate::window::titlebar::apply_scrollbar_theme(editor.hwnd(), palette.dark_frame);
    if let Some(zoom) = zoom {
        let _ = editor.set_zoom(zoom);
    }
    install_group_hooks(hwnd, editor);
}

/// The Markdown Enter and Tab helpers (Markdown design spec §8), on every group's editor.
pub(super) fn install_group_hooks(hwnd: HWND, editor: &Editor) {
    editor.set_hooks(Some(std::rc::Rc::new(
        crate::window::markdown_host::GroupHooks::new(hwnd, editor.hwnd()),
    )));
}

pub(super) fn apply_settings_to(
    editor: &Editor,
    settings: &crate::config::Settings,
    palette: Palette,
) {
    let _ = editor.set_line_numbers(settings.line_numbers);
    let dpi = unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForWindow(editor.hwnd()) };
    let _ = editor.set_code_folding(settings.code_folding, dpi);
    let _ = editor.apply_view_settings(
        &settings.font_face,
        settings.font_size,
        settings.tab_width,
        settings.word_wrap,
    );
    let _ = editor.apply_whitespace_settings(settings.insert_spaces, settings.show_whitespace);
    let _ =
        editor.set_line_number_colors(palette.line_number_foreground, palette.editor_background);
    apply_chrome_colors_to(editor, palette, settings.highlight_current_line);
}

pub(super) fn apply_colors_to(editor: &Editor, palette: Palette, highlight_current_line: bool) {
    let _ = editor.set_base_colors(palette.editor_foreground, palette.editor_background);
    let _ =
        editor.set_line_number_colors(palette.line_number_foreground, palette.editor_background);
    apply_chrome_colors_to(editor, palette, highlight_current_line);
    let _ = editor.set_selection_text_colors(palette.selection_foreground);
}

/// The selection backgrounds, the caret line's when `highlight_current_line` is on, and the fold
/// margin's markers.
fn apply_chrome_colors_to(editor: &Editor, palette: Palette, highlight_current_line: bool) {
    let _ = editor.set_fold_colors(palette.line_number_foreground, palette.editor_background);
    let _ = editor.set_chrome_colors(
        palette.selection_background,
        palette.inactive_selection_background,
        highlight_current_line.then_some(palette.caret_line_background),
    );
}

/// Every group's editor, the active group's first.
pub(super) fn all_editors(hwnd: HWND) -> Vec<Editor> {
    unsafe { app_ptr(hwnd) }
        .map(|app| {
            let app = unsafe { app.as_ref() };
            let active = app.tabs.active_group();
            let mut editors = app
                .active_group()
                .map(|group| group.editor.clone())
                .into_iter()
                .collect::<Vec<_>>();
            editors.extend(
                app.groups
                    .iter()
                    .filter(|group| group.id != active)
                    .map(|group| group.editor.clone()),
            );
            editors
        })
        .unwrap_or_default()
}

/// The window the content area's children go into: the editor group, or the main window before
/// the group exists.
pub(crate) fn content_parent(hwnd: HWND) -> HWND {
    group_hwnd(hwnd).unwrap_or(hwnd)
}

/// Lays out the tab strip, the find bar, then the preview and the editor below them, in `group`'s
/// client area.
pub(crate) fn layout_group(hwnd: HWND, group: HWND) {
    let Some(id) = group_id_of(hwnd, group) else {
        return;
    };
    let Some(editor_hwnd) = group_editor(hwnd, id).map(|editor| editor.hwnd()) else {
        return;
    };
    let mut client = RECT::default();
    unsafe {
        GetClientRect(group, &mut client);
    }
    let dpi = unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForWindow(group) }.max(96);
    let font = title_chrome(hwnd).1.text();
    let width = client.right;
    let strip_height = crate::window::group_strip::strip_height(dpi);
    // The menu band and the name box sit under the title row, over the top groups' content.
    let band = if is_top_group(hwnd, group) {
        menu_band_height(hwnd) + name_box_band_height(hwnd, dpi)
    } else {
        0
    };
    let find_top = strip_height + band;
    let find_bar_height = unsafe { app_ptr(hwnd) }
        .and_then(|app| {
            let bar = unsafe { app.as_ref() }.group(id)?.find_bar.as_ref()?;
            bar.layout(0, width, find_top, dpi, font);
            bar.is_visible().then(|| find_bar::find_bar_height(dpi))
        })
        .unwrap_or(0);
    let top = find_top + find_bar_height;
    let area = RECT {
        left: 0,
        top,
        right: width,
        bottom: client.bottom.max(top),
    };
    with_group_id(hwnd, id, |state| {
        state.content =
            crate::window::titlebar::Rect::new(area.left, area.top, area.right, area.bottom)
    });
    set_group_region(hwnd, group, &client, strip_height, band);
    let rects = crate::window::preview_host::layout(hwnd, id, area, dpi);
    crate::window::image_host::layout(hwnd, id, area);
    crate::window::preview_buttons::layout(hwnd, group, area, dpi);
    if let Some(editor_rect) = rects.editor {
        unsafe {
            MoveWindow(
                editor_hwnd,
                editor_rect.left,
                editor_rect.top,
                editor_rect.right - editor_rect.left,
                editor_rect.bottom - editor_rect.top,
                1,
            );
        }
    }
    unsafe {
        InvalidateRect(group, std::ptr::null(), 0);
    }
}

/// The visible name box's height, or 0.
fn name_box_band_height(hwnd: HWND, dpi: u32) -> i32 {
    unsafe { app_ptr(hwnd) }
        .filter(|app| {
            unsafe { app.as_ref() }
                .name_box
                .as_ref()
                .is_some_and(|name_box| name_box.is_visible())
        })
        .map_or(0, |_| crate::window::name_box::name_box_height(dpi))
}

/// Leaves out of the group window what the main window keeps in its area: the caption buttons at
/// the right end of the title row, and the band under it while the menu band or the name box
/// shows. The main window paints them and gets their input.
fn set_group_region(hwnd: HWND, group: HWND, client: &RECT, strip_height: i32, band: i32) {
    use windows_sys::Win32::Graphics::Gdi::{CombineRgn, CreateRectRgn, RGN_DIFF, SetWindowRgn};
    // Only the group under the caption buttons gives up the part of its strip they cover.
    let mut origin = windows_sys::Win32::Foundation::POINT { x: 0, y: 0 };
    unsafe { windows_sys::Win32::Graphics::Gdi::MapWindowPoints(group, hwnd, &mut origin, 1) };
    let caption_left = title_layout(hwnd).minimize.left - origin.x;
    let caption_left = if origin.y == 0 && caption_left < client.right {
        caption_left.max(0)
    } else {
        client.right
    };
    unsafe {
        let region = CreateRectRgn(0, 0, client.right, client.bottom);
        let cut = |left: i32, top: i32, right: i32, bottom: i32| {
            let part = CreateRectRgn(left, top, right, bottom);
            CombineRgn(region, region, part, RGN_DIFF);
            windows_sys::Win32::Graphics::Gdi::DeleteObject(part);
        };
        cut(caption_left, 0, client.right, strip_height);
        if band > 0 {
            cut(0, strip_height, client.right, strip_height + band);
        }
        // The system owns the region from here on.
        SetWindowRgn(group, region, 1);
    }
}

/// The group's answer to `WM_NCHITTEST`: its empty strip space, and a restored window's top
/// resize band, are the main window's caption (spec §4.1), so the window drags and resizes there.
pub(crate) fn group_hit_test(hwnd: HWND, group: HWND, lparam: LPARAM) -> LRESULT {
    use windows_sys::Win32::UI::WindowsAndMessaging::{HTCLIENT, HTTRANSPARENT, IsZoomed};
    let mut point = windows_sys::Win32::Foundation::POINT {
        x: (lparam as u32 & 0xffff) as u16 as i16 as i32,
        y: ((lparam as u32 >> 16) & 0xffff) as u16 as i16 as i32,
    };
    unsafe { windows_sys::Win32::Graphics::Gdi::ScreenToClient(group, &mut point) };
    let Some(layout) = group_id_of(hwnd, group).and_then(|id| strip_layout_of(hwnd, id)) else {
        return HTCLIENT as LRESULT;
    };
    let mut origin = windows_sys::Win32::Foundation::POINT { x: 0, y: 0 };
    unsafe { windows_sys::Win32::Graphics::Gdi::MapWindowPoints(group, hwnd, &mut origin, 1) };
    let in_title_row = origin.y == 0 && point.y >= 0 && point.y < layout.height;
    if !in_title_row {
        return HTCLIENT as LRESULT;
    }
    let resizes = unsafe { IsZoomed(hwnd) } == 0 && point.y < title_layout(hwnd).resize_border;
    let empty = layout.hit_test(crate::window::titlebar::Point::new(point.x, point.y))
        == crate::window::group_strip::StripTarget::Empty;
    if resizes || empty {
        HTTRANSPARENT as LRESULT
    } else {
        HTCLIENT as LRESULT
    }
}

/// A caption point (screen coordinates in `lparam`) over a title-row strip's empty space, as the
/// group window and its client coordinates.
pub(super) fn caption_strip_point(hwnd: HWND, lparam: LPARAM) -> Option<(HWND, i32, i32)> {
    let groups = unsafe { app_ptr(hwnd) }.map(|app| {
        unsafe { app.as_ref() }
            .groups
            .iter()
            .map(|group| (group.id, group.hwnd))
            .collect::<Vec<_>>()
    })?;
    groups.into_iter().find_map(|(id, group)| {
        if !is_top_group(hwnd, group) {
            return None;
        }
        let mut point = windows_sys::Win32::Foundation::POINT {
            x: (lparam as u32 & 0xffff) as u16 as i16 as i32,
            y: ((lparam as u32 >> 16) & 0xffff) as u16 as i16 as i32,
        };
        unsafe { windows_sys::Win32::Graphics::Gdi::ScreenToClient(group, &mut point) };
        let layout = strip_layout_of(hwnd, id)?;
        (point.x >= 0
            && point.x < layout.bounds().right
            && point.y >= 0
            && point.y < layout.height
            && layout.hit_test(crate::window::titlebar::Point::new(point.x, point.y))
                == crate::window::group_strip::StripTarget::Empty)
            .then_some((group, point.x, point.y))
    })
}

/// Paints what the group's children leave uncovered: the tab strip, the preview divider, and the
/// empty hint while no tab is open.
pub(crate) fn paint_group(hwnd: HWND, group: HWND) {
    let mut paint = windows_sys::Win32::Graphics::Gdi::PAINTSTRUCT::default();
    let dc = unsafe { windows_sys::Win32::Graphics::Gdi::BeginPaint(group, &mut paint) };
    if dc.is_null() {
        return;
    }
    let Some(id) = group_id_of(hwnd, group) else {
        unsafe { windows_sys::Win32::Graphics::Gdi::EndPaint(group, &paint) };
        return;
    };
    let dpi = unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForWindow(group) }.max(96);
    let (palette, fonts, _) = title_chrome(hwnd);
    let mut client = RECT::default();
    unsafe {
        GetClientRect(group, &mut client);
    }
    if let Some(layout) = strip_layout_of(hwnd, id)
        && paint.rcPaint.top < layout.height
    {
        let snapshot = tab_snapshot(hwnd, id);
        let titles = snapshot
            .titles
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        let (icon_set, light_theme) = current_icon_style(hwnd);
        let pointer = with_group_id(hwnd, id, |group| group.pointer).unwrap_or_default();
        let (is_active_group, group_count) = unsafe { app_ptr(hwnd) }
            .map(|app| {
                let app = unsafe { app.as_ref() };
                (app.tabs.active_group() == id, app.groups.len())
            })
            .unwrap_or((true, 1));
        unsafe {
            crate::window::group_strip::paint(
                dc,
                &layout,
                dpi,
                &crate::window::group_strip::StripPaint {
                    titles: &titles,
                    kinds: &snapshot.kinds,
                    icons: current_file_icons(hwnd),
                    icon_set,
                    light_theme,
                    active: snapshot.active,
                    preview_tab: snapshot.preview_tab,
                    palette,
                    fonts,
                    pointer,
                    accent: crate::window::group_strip::tab_accent(
                        is_active_group,
                        group_count,
                        &palette,
                    ),
                },
            );
        }
        client.top = layout.height + menu_band_height(hwnd) + name_box_band_height(hwnd, dpi);
    }
    if group_tab_count(hwnd, id) == 0 {
        unsafe {
            crate::window::titlebar::paint_empty_hint(
                dc,
                client,
                EMPTY_TABS_HINT,
                palette,
                fonts.text(),
                dpi,
            );
        }
    }
    if let Some(divider) =
        crate::window::preview_host::with_group_host(hwnd, id, |host| host.divider_rect()).flatten()
    {
        unsafe { crate::window::titlebar::paint_divider(dc, divider, palette) };
    }
    unsafe {
        windows_sys::Win32::Graphics::Gdi::EndPaint(group, &paint);
    }
}

/// Focus given to the group window goes on to its content, as focus given to the frame does. With
/// no tab open the frame keeps it, to take the menu keys.
pub(crate) fn focus_group_content(hwnd: HWND) {
    let target = (tab_count(hwnd) > 0)
        .then(|| content_focus_target(hwnd))
        .flatten()
        .unwrap_or(hwnd);
    unsafe {
        SetFocus(target);
    }
}
