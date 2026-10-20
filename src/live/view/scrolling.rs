//! Scrolling the Live view: the scroll bar and the mouse wheel. Ctrl+wheel zooms, through the
//! main window's zoom commands, so the Live view and every editor keep one zoom.

use super::*;
use crate::window::commands::CommandId;

use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetScrollInfo, PostMessageW, SB_BOTTOM, SB_LINEDOWN, SB_LINEUP, SB_PAGEDOWN, SB_PAGEUP,
    SB_THUMBPOSITION, SB_THUMBTRACK, SB_TOP, SB_VERT, SCROLLINFO, SIF_TRACKPOS,
    SPI_GETWHEELSCROLLLINES, SystemParametersInfoW, WM_COMMAND,
};

/// `MK_CONTROL` from WinUser.h; its windows-sys home needs a feature FastPad does not enable.
const MK_CONTROL: usize = 0x0008;

/// Moves the view to `scroll_y` (clamped) and tells the main window it scrolled.
fn scroll_to(hwnd: HWND, scroll_y: impl FnOnce(&mut ViewState, f32) -> f32) {
    let (_, height) = view_size(hwnd);
    let moved = with_state(hwnd, |state| {
        let wanted = scroll_y(state, height);
        let clamped = scroll::clamp(wanted, height, state.layout.total_height());
        let moved = (clamped - state.scroll_y).abs() > 0.01;
        state.scroll_y = clamped;
        moved
    })
    .unwrap_or(false);
    if moved {
        invalidate(hwnd);
        post_changed(hwnd, true);
    }
}

/// `WM_VSCROLL`.
pub(super) fn vscroll(hwnd: HWND, wparam: usize) {
    let request = (wparam & 0xFFFF) as i32;
    let track = matches!(request, SB_THUMBTRACK | SB_THUMBPOSITION).then(|| {
        let mut info = SCROLLINFO {
            cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
            fMask: SIF_TRACKPOS,
            ..Default::default()
        };
        unsafe { GetScrollInfo(hwnd, SB_VERT, &mut info) };
        info.nTrackPos as f32 / dpi_scale(hwnd)
    });
    scroll_to(hwnd, |state, height| {
        let line = state.layout.fonts().line_height;
        match request {
            SB_LINEUP => state.scroll_y - line,
            SB_LINEDOWN => state.scroll_y + line,
            SB_PAGEUP => state.scroll_y - (height - line).max(line),
            SB_PAGEDOWN => state.scroll_y + (height - line).max(line),
            SB_TOP => 0.0,
            SB_BOTTOM => f32::MAX,
            _ => track.unwrap_or(state.scroll_y),
        }
    });
}

/// `WM_MOUSEWHEEL`: the system's lines per notch; with Ctrl, one zoom step per notch.
pub(super) fn wheel(hwnd: HWND, wparam: usize) {
    let delta = ((wparam >> 16) & 0xFFFF) as u16 as i16;
    if wparam & MK_CONTROL != 0 {
        let command = if delta > 0 {
            CommandId::ZoomIn
        } else {
            CommandId::ZoomOut
        };
        unsafe {
            PostMessageW(
                crate::platform::win32::root_window(hwnd),
                WM_COMMAND,
                command as usize,
                0,
            )
        };
        return;
    }
    let mut lines = 3_u32;
    unsafe {
        SystemParametersInfoW(
            SPI_GETWHEELSCROLLLINES,
            0,
            (&mut lines as *mut u32).cast(),
            0,
        )
    };
    let (_, height) = view_size(hwnd);
    scroll_to(hwnd, |state, _| {
        let line = state.layout.fonts().line_height;
        // WHEEL_PAGESCROLL (u32::MAX) scrolls a page per notch.
        let step = if lines == u32::MAX {
            height
        } else {
            lines as f32 * line
        };
        state.scroll_y - step * f32::from(delta) / 120.0
    });
}
