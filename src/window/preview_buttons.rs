//! The view mode control (2026-10-22 view modes spec §2): one segment per mode the active tab can
//! show, floating at the top-right of an editor group's content (split editors spec §4.1). A small
//! child window of the group, above the editor and preview in z-order, shown only while the active
//! tab is Markdown or SVG.

use crate::platform::wide_null;
use crate::preview::ViewMode;
use crate::window::design::metrics::scale;
use crate::window::titlebar::{
    GLYPH_VIEW_LIVE, GLYPH_VIEW_PREVIEW, GLYPH_VIEW_SOURCE, GLYPH_VIEW_SPLIT, Point, Rect,
    create_ui_font, draw_text, fill, restore_font, select_font,
};
use crate::window::tooltip::Tooltip;
use windows_sys::Win32::Foundation::{
    ERROR_CLASS_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, DT_CENTER, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, DeleteObject, EndPaint,
    FW_NORMAL, InvalidateRect, MapWindowPoints, PAINTSTRUCT, SetBkMode, SetTextColor, TRANSPARENT,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::WM_MOUSELEAVE;
use windows_sys::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DefWindowProcW, FindWindowExW, GetClientRect, HWND_TOP, IDC_ARROW, IsWindowVisible,
    LoadCursorW, RegisterClassW, SM_CXVSCROLL, SW_HIDE, SWP_NOACTIVATE, SWP_SHOWWINDOW,
    SetWindowPos, ShowWindow, WM_ERASEBKGND, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MOUSEMOVE, WM_PAINT, WNDCLASSW, WS_CHILD, WS_CLIPSIBLINGS,
};

const CLASS_NAME: &str = "FastPadPreviewButtons";

/// The floating control's window, once created, and its pointer state; kept on its group.
#[derive(Debug)]
pub(crate) struct PreviewButtons {
    pub(crate) hwnd: HWND,
    /// The modes shown, in order: every mode for Markdown, no Live for SVG.
    segments: &'static [ViewMode],
    hovered: Option<ViewMode>,
    pressed: Option<ViewMode>,
    /// Made on the first pointer move over the buttons; destroyed with the group.
    tooltip: Option<Tooltip>,
    /// The tooltip could not be made, and that is not tried again.
    tooltip_failed: bool,
}

impl Drop for PreviewButtons {
    fn drop(&mut self) {
        if let Some(tooltip) = self.tooltip.take() {
            tooltip.destroy();
        }
    }
}

impl Default for PreviewButtons {
    fn default() -> Self {
        Self {
            hwnd: std::ptr::null_mut(),
            segments: &ViewMode::ALL,
            hovered: None,
            pressed: None,
            tooltip: None,
            tooltip_failed: false,
        }
    }
}

fn button_size(dpi: u32) -> i32 {
    scale(32, dpi)
}

/// The active group's floating preview buttons, once created.
#[cfg(test)]
pub(crate) fn hwnd(main: HWND) -> Option<HWND> {
    crate::window::main_window::with_group(main, |group| group.preview_buttons.hwnd)
        .filter(|hwnd| !hwnd.is_null())
}

/// The modes group `id`'s active tab can be shown in, in the control's order; empty for a tab that
/// is neither Markdown nor SVG.
pub(crate) fn segments_for(
    main: HWND,
    id: crate::window::split_tree::GroupId,
) -> &'static [ViewMode] {
    match crate::window::preview_host::group_document(main, id).map(|(_, language, _)| language) {
        Some(crate::document::Language::Markdown) => &ViewMode::ALL,
        Some(crate::document::Language::Svg) => &ViewMode::SVG,
        _ => &[],
    }
}

/// The modes the control in `buttons` shows.
fn segments(main: HWND, buttons: HWND) -> &'static [ViewMode] {
    crate::window::main_window::with_group(main, |group| {
        (group.preview_buttons.hwnd == buttons).then_some(group.preview_buttons.segments)
    })
    .flatten()
    .unwrap_or(&[])
}

/// The rectangle of the segment at `index`, in the floating window's client coordinates.
pub(crate) fn button_rect(buttons: HWND, index: usize) -> Rect {
    let size = button_size(unsafe { GetDpiForWindow(buttons) }.max(96));
    let left = index as i32 * size;
    Rect::new(left, 0, left + size, size)
}

fn button_at(main: HWND, buttons: HWND, x: i32, y: i32) -> Option<ViewMode> {
    segments(main, buttons)
        .iter()
        .enumerate()
        .find(|(index, _)| button_rect(buttons, *index).contains(Point::new(x, y)))
        .map(|(_, mode)| *mode)
}

/// The shown floating window inside `group`, for the group's accessibility provider, which has
/// only the group's window to go on.
pub(crate) fn find(group: HWND) -> Option<HWND> {
    let class = wide_null(CLASS_NAME);
    let buttons = unsafe {
        FindWindowExW(
            group,
            std::ptr::null_mut(),
            class.as_ptr(),
            std::ptr::null(),
        )
    };
    (!buttons.is_null() && unsafe { IsWindowVisible(buttons) } != 0).then_some(buttons)
}

/// The rectangle of the segment at `index` in `group`'s client coordinates while the control is
/// shown.
pub(crate) fn rect_in_group(group: HWND, index: usize) -> Option<Rect> {
    let buttons = find(group)?;
    let mut origin = POINT { x: 0, y: 0 };
    unsafe { MapWindowPoints(buttons, group, &mut origin, 1) };
    let rect = button_rect(buttons, index);
    Some(Rect::new(
        rect.left + origin.x,
        rect.top + origin.y,
        rect.right + origin.x,
        rect.bottom + origin.y,
    ))
}

/// Shows `group`'s control at the top-right of `area` (the group's content, in its client
/// coordinates), clear of a vertical scroll bar, while `group` is the active group and its active
/// tab is Markdown or SVG; hides it otherwise.
pub(crate) fn layout(main: HWND, group: HWND, area: RECT, dpi: u32) {
    let Some(id) = crate::window::main_window::group_id_of(main, group) else {
        return;
    };
    let existing =
        crate::window::main_window::with_group_id(main, id, |state| state.preview_buttons.hwnd)
            .filter(|hwnd| !hwnd.is_null());
    let active = unsafe { crate::window::main_window::app_ptr(main) }
        .is_some_and(|app| unsafe { app.as_ref() }.tabs.active_group() == id);
    let segments = if active { segments_for(main, id) } else { &[] };
    if segments.is_empty() {
        if let Some(buttons) = existing {
            unsafe { ShowWindow(buttons, SW_HIDE) };
        }
        return;
    }
    let Some(buttons) = existing.or_else(|| create(main, id, group)) else {
        return;
    };
    // Other segments mean other tooltip rectangles: the tooltip is made again on the next hover.
    let stale = crate::window::main_window::with_group_id(main, id, |state| {
        let control = &mut state.preview_buttons;
        if control.segments == segments {
            return None;
        }
        control.segments = segments;
        control.hovered = None;
        control.pressed = None;
        control.tooltip_failed = false;
        control.tooltip.take()
    })
    .flatten();
    if let Some(tooltip) = stale {
        tooltip.destroy();
    }
    let size = button_size(dpi);
    let width = segments.len() as i32 * size;
    let margin = scale(8, dpi);
    let scroll_bar = unsafe { GetSystemMetricsForDpi(SM_CXVSCROLL, dpi) };
    let x = (area.right - scroll_bar - margin - width).max(area.left);
    unsafe {
        SetWindowPos(
            buttons,
            HWND_TOP,
            x,
            area.top + margin,
            width,
            size,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
        InvalidateRect(buttons, std::ptr::null(), 0);
    }
}

fn create(main: HWND, id: crate::window::split_tree::GroupId, group: HWND) -> Option<HWND> {
    let buttons = crate::window::panel::create_child(
        group,
        register_class().ok()?,
        WS_CHILD | WS_CLIPSIBLINGS,
    )
    .ok()?;
    crate::window::main_window::with_group_id(main, id, |state| {
        state.preview_buttons.hwnd = buttons;
    });
    Some(buttons)
}

fn register_class() -> crate::Result<&'static [u16]> {
    static CLASS: std::sync::OnceLock<Vec<u16>> = std::sync::OnceLock::new();
    static REGISTERED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let name = CLASS.get_or_init(|| wide_null(CLASS_NAME));
    let registered = *REGISTERED.get_or_init(|| {
        let class = WNDCLASSW {
            lpfnWndProc: Some(buttons_proc),
            hInstance: unsafe { GetModuleHandleW(std::ptr::null()) },
            hCursor: unsafe { LoadCursorW(std::ptr::null_mut(), IDC_ARROW) },
            lpszClassName: name.as_ptr(),
            ..Default::default()
        };
        unsafe { RegisterClassW(&class) != 0 || GetLastError() == ERROR_CLASS_ALREADY_EXISTS }
    });
    if registered {
        Ok(name)
    } else {
        Err(crate::FastPadError::Invariant(
            "the preview buttons window class could not be registered",
        ))
    }
}

/// The first pointer move over the buttons makes their tooltip and hands it that move, so the
/// first hover starts the tip's timer like any later one. Nothing before that needs it.
fn ensure_tooltip(main: HWND, buttons: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) {
    let wanted = crate::window::main_window::with_group(main, |group| {
        group.preview_buttons.hwnd == buttons
            && group.preview_buttons.tooltip.is_none()
            && !group.preview_buttons.tooltip_failed
    })
    .unwrap_or(false);
    if !wanted {
        return;
    }
    // Made with nothing of the App borrowed: creating the control sends messages.
    let created = Tooltip::create(buttons);
    let stored = crate::window::main_window::with_group(main, |group| {
        let state = &mut group.preview_buttons;
        let stored = state.hwnd == buttons && state.tooltip.is_none();
        if stored {
            state.tooltip = created;
            state.tooltip_failed = created.is_none();
        }
        stored
    })
    .unwrap_or(false);
    match created {
        Some(tooltip) if stored => {
            for (id, mode) in segments(main, buttons).iter().enumerate() {
                let rect = button_rect(buttons, id);
                let rect = RECT {
                    left: rect.left,
                    top: rect.top,
                    right: rect.right,
                    bottom: rect.bottom,
                };
                tooltip.set_tool(
                    id,
                    rect,
                    &crate::window::preview_host::button_text(main, *mode),
                );
            }
            tooltip.relay(message, wparam, lparam);
        }
        // The group went or the buttons were replaced while the tooltip was being made.
        Some(tooltip) => tooltip.destroy(),
        None => {}
    }
}

fn update(main: HWND, buttons: HWND, change: impl FnOnce(&mut PreviewButtons)) {
    crate::window::main_window::with_group(main, |group| change(&mut group.preview_buttons));
    unsafe { InvalidateRect(buttons, std::ptr::null(), 0) };
}

unsafe extern "system" fn buttons_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let main = crate::platform::win32::root_window(hwnd);
    let (x, y) = (
        (lparam as u32 & 0xffff) as u16 as i16 as i32,
        ((lparam as u32 >> 16) & 0xffff) as u16 as i16 as i32,
    );
    match message {
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            paint(main, hwnd);
            0
        }
        WM_MOUSEMOVE => {
            ensure_tooltip(main, hwnd, message, wparam, lparam);
            let mut track = TRACKMOUSEEVENT {
                cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: hwnd,
                dwHoverTime: 0,
            };
            unsafe { TrackMouseEvent(&mut track) };
            let over = button_at(main, hwnd, x, y);
            update(main, hwnd, |state| state.hovered = over);
            crate::window::preview_host::button_hover(main, over);
            0
        }
        WM_MOUSELEAVE => {
            update(main, hwnd, |state| {
                state.hovered = None;
                state.pressed = None;
            });
            crate::window::preview_host::button_hover(main, None);
            0
        }
        WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
            let over = button_at(main, hwnd, x, y);
            update(main, hwnd, |state| {
                state.hovered = over;
                state.pressed = over;
            });
            0
        }
        // A click acts only when released over the button it was pressed on.
        WM_LBUTTONUP => {
            let over = button_at(main, hwnd, x, y);
            let mut pressed = None;
            update(main, hwnd, |state| pressed = state.pressed.take());
            if let Some(mode) = over.filter(|&mode| pressed == Some(mode)) {
                crate::window::preview_host::set_mode(main, mode);
            }
            0
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

fn paint(main: HWND, buttons: HWND) {
    let mut paint = PAINTSTRUCT::default();
    let dc = unsafe { BeginPaint(buttons, &mut paint) };
    if dc.is_null() {
        return;
    }
    let dpi = unsafe { GetDpiForWindow(buttons) }.max(96);
    let (palette, _, _) = crate::window::main_window::title_chrome(main);
    let mode = crate::window::preview_host::mode(main);
    let (hovered, pressed, segments) = crate::window::main_window::with_group(main, |group| {
        let control = &group.preview_buttons;
        (control.hovered, control.pressed, control.segments)
    })
    .unwrap_or_default();
    let mut client = RECT::default();
    unsafe { GetClientRect(buttons, &mut client) };
    let centered = DT_SINGLELINE | DT_VCENTER | DT_CENTER | DT_NOPREFIX;
    unsafe {
        fill(
            dc,
            Rect::new(0, 0, client.right, client.bottom),
            palette.strip_background,
        );
        SetBkMode(dc, TRANSPARENT as i32);
    }
    // Larger than the title bar's caption glyphs: these are the buttons' whole content.
    let font = create_ui_font(
        scale(17, dpi),
        crate::window::design::faces::current().icons,
        FW_NORMAL as i32,
        false,
    );
    let previous = unsafe { select_font(dc, font) };
    for (index, &button) in segments.iter().enumerate() {
        let glyph = match button {
            ViewMode::Source => GLYPH_VIEW_SOURCE,
            ViewMode::Live => GLYPH_VIEW_LIVE,
            ViewMode::Split => GLYPH_VIEW_SPLIT,
            ViewMode::Preview => GLYPH_VIEW_PREVIEW,
        };
        let active = mode == button;
        let rect = button_rect(buttons, index);
        let over = hovered == Some(button);
        let background = if (over && pressed == Some(button)) || active {
            Some(palette.pressed_background)
        } else if over {
            Some(palette.hover_background)
        } else {
            None
        };
        unsafe {
            if let Some(background) = background {
                fill(dc, rect.centered_square(scale(28, dpi)), background);
            }
            SetTextColor(
                dc,
                if over || active {
                    palette.hover_foreground
                } else {
                    palette.muted_foreground
                },
            );
            draw_text(dc, glyph, rect, centered);
        }
    }
    unsafe {
        restore_font(dc, previous);
        DeleteObject(font as _);
        EndPaint(buttons, &paint);
    }
}
