//! Where the primary window was when it closed: saved in `fastpad.ini` on close and put back
//! before the window is first shown.

use super::*;
use crate::config::WindowPlacement;
use windows_sys::Win32::Graphics::Gdi::{MONITOR_DEFAULTTONULL, MonitorFromRect};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetWindowPlacement, SHOW_WINDOW_CMD, SW_SHOW, SW_SHOWMAXIMIZED, SW_SHOWMINIMIZED,
    SetWindowPlacement, WINDOWPLACEMENT, WPF_RESTORETOMAXIMIZED,
};

/// Saves the window's restored frame and whether it is maximized, when this is the primary
/// window and either changed. A minimized window saves the state it would restore to.
pub(super) fn save_placement(hwnd: HWND) {
    let primary = unsafe { app_ptr(hwnd) }
        .is_some_and(|app| unsafe { app.as_ref() }.instance_mutex.is_some());
    if !primary {
        return;
    }
    let mut native = WINDOWPLACEMENT {
        length: size_of::<WINDOWPLACEMENT>() as u32,
        ..Default::default()
    };
    if unsafe { GetWindowPlacement(hwnd, &mut native) } == 0 {
        return;
    }
    let show = native.showCmd as SHOW_WINDOW_CMD;
    let maximized = show == SW_SHOWMAXIMIZED
        || (show == SW_SHOWMINIMIZED && native.flags & WPF_RESTORETOMAXIMIZED != 0);
    let frame = native.rcNormalPosition;
    let placement = WindowPlacement {
        x: frame.left,
        y: frame.top,
        width: frame.right - frame.left,
        height: frame.bottom - frame.top,
        maximized,
    };
    if placement.width <= 0 || placement.height <= 0 {
        return;
    }
    change_setting(hwnd, |settings| {
        (settings.window_placement != Some(placement)).then(|| {
            settings.window_placement = Some(placement);
            ("window_placement", placement.token())
        })
    });
}

/// Moves the not yet shown window to `placement` and returns how to show it. A placement on no
/// monitor (one since unplugged) is skipped, and the window opens where it was created.
pub(crate) fn restore_placement(hwnd: HWND, placement: Option<WindowPlacement>) -> SHOW_WINDOW_CMD {
    let Some(placement) = placement else {
        return SW_SHOW;
    };
    let frame = RECT {
        left: placement.x,
        top: placement.y,
        right: placement.x.saturating_add(placement.width),
        bottom: placement.y.saturating_add(placement.height),
    };
    if unsafe { MonitorFromRect(&frame, MONITOR_DEFAULTTONULL) }.is_null() {
        return SW_SHOW;
    }
    let native = WINDOWPLACEMENT {
        length: size_of::<WINDOWPLACEMENT>() as u32,
        // Hidden: the caller shows it once the editor is in place.
        showCmd: SW_HIDE as u32,
        rcNormalPosition: frame,
        ..Default::default()
    };
    if unsafe { SetWindowPlacement(hwnd, &native) } == 0 {
        return SW_SHOW;
    }
    if placement.maximized {
        SW_SHOWMAXIMIZED
    } else {
        SW_SHOW
    }
}
