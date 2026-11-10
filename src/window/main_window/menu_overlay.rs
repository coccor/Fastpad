//! The popup that shows the Alt/F10 menu column in the window's top-left corner, under the title
//! strip. It floats over the sidebar and the editors, so menu mode moves nothing, and it never
//! takes the activation: the frame keeps the focus for the menu keys, and a click elsewhere ends
//! menu mode as before. Pointer input over it goes to the frame's menu handling.

use super::*;
use windows_sys::Win32::Foundation::ERROR_CLASS_ALREADY_EXISTS;
use windows_sys::Win32::Foundation::GetLastError;
use windows_sys::Win32::Graphics::Gdi::ClientToScreen;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CS_DROPSHADOW, CreateWindowExW, DefWindowProcW, GW_OWNER, GetWindow, HWND_TOP, IDC_ARROW,
    LoadCursorW, MA_NOACTIVATE, RegisterClassW, SW_SHOWNA, SWP_NOACTIVATE, SetWindowPos,
    ShowWindow, WM_ERASEBKGND, WM_MOUSEACTIVATE, WNDCLASSW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_POPUP,
};

/// Shows, moves or removes the column's popup to match menu mode and the headings' place.
pub(super) fn sync(hwnd: HWND) {
    let Some(mut app) = (unsafe { app_ptr(hwnd) }) else {
        return;
    };
    let current = unsafe { app.as_ref() }.menu_overlay;
    let dpi = unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForWindow(hwnd) }.max(96);
    let column = menu_mode(hwnd)
        .and_then(|_| crate::window::menu_band::column_rect(&menu_headings(hwnd), dpi));
    let Some(column) = column else {
        if !current.is_null() {
            unsafe { app.as_mut() }.menu_overlay = std::ptr::null_mut();
            unsafe { DestroyWindow(current) };
        }
        return;
    };
    let mut origin = windows_sys::Win32::Foundation::POINT {
        x: column.left,
        y: column.top,
    };
    unsafe { ClientToScreen(hwnd, &mut origin) };
    let (width, height) = (column.right - column.left, column.bottom - column.top);
    if current.is_null() {
        let Some(overlay) = create(hwnd, origin, width, height) else {
            return;
        };
        unsafe { app.as_mut() }.menu_overlay = overlay;
        return;
    }
    unsafe {
        SetWindowPos(
            current,
            HWND_TOP,
            origin.x,
            origin.y,
            width,
            height,
            SWP_NOACTIVATE,
        );
        InvalidateRect(current, std::ptr::null(), 0);
    }
}

/// The column's popup, or null outside menu mode.
#[cfg(test)]
pub(crate) fn overlay(hwnd: HWND) -> HWND {
    unsafe { app_ptr(hwnd) }
        .map(|app| unsafe { app.as_ref() }.menu_overlay)
        .unwrap_or(std::ptr::null_mut())
}

fn create(
    owner: HWND,
    origin: windows_sys::Win32::Foundation::POINT,
    width: i32,
    height: i32,
) -> Option<HWND> {
    let class = register_class()?;
    let overlay = unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            class.as_ptr(),
            std::ptr::null(),
            WS_POPUP,
            origin.x,
            origin.y,
            width,
            height,
            owner,
            std::ptr::null_mut(),
            GetModuleHandleW(std::ptr::null()),
            std::ptr::null(),
        )
    };
    if overlay.is_null() {
        return None;
    }
    unsafe {
        ShowWindow(overlay, SW_SHOWNA);
    }
    Some(overlay)
}

fn register_class() -> Option<&'static [u16]> {
    static CLASS_NAME: std::sync::OnceLock<Vec<u16>> = std::sync::OnceLock::new();
    static REGISTERED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let name = CLASS_NAME.get_or_init(|| crate::platform::wide_null("FastPadMenuColumn"));
    let registered = *REGISTERED.get_or_init(|| {
        let class = WNDCLASSW {
            style: CS_DROPSHADOW,
            lpfnWndProc: Some(overlay_proc),
            hInstance: unsafe { GetModuleHandleW(std::ptr::null()) },
            hCursor: unsafe { LoadCursorW(std::ptr::null_mut(), IDC_ARROW) },
            lpszClassName: name.as_ptr(),
            ..Default::default()
        };
        unsafe { RegisterClassW(&class) != 0 || GetLastError() == ERROR_CLASS_ALREADY_EXISTS }
    });
    registered.then_some(name.as_slice())
}

/// `lparam`'s point, from the popup's client coordinates into its owner's, packed the same way.
fn to_owner(overlay: HWND, owner: HWND, lparam: LPARAM) -> (i32, i32, LPARAM) {
    let mut point = windows_sys::Win32::Foundation::POINT {
        x: (lparam as u32 & 0xffff) as u16 as i16 as i32,
        y: ((lparam as u32 >> 16) & 0xffff) as u16 as i16 as i32,
    };
    unsafe {
        ClientToScreen(overlay, &mut point);
        windows_sys::Win32::Graphics::Gdi::ScreenToClient(owner, &mut point);
    }
    let packed = (point.x as u16 as u32 | ((point.y as u16 as u32) << 16)) as LPARAM;
    (point.x, point.y, packed)
}

unsafe extern "system" fn overlay_proc(
    overlay: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let owner = unsafe { GetWindow(overlay, GW_OWNER) };
    match message {
        WM_ERASEBKGND => 1,
        WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
        WM_PAINT if !owner.is_null() => {
            let headings = menu_headings(owner);
            let mode = menu_mode(owner).unwrap_or_default();
            let (palette, fonts, _) = title_chrome(owner);
            let dpi = unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForWindow(owner) }.max(96);
            crate::window::side_panel::paint_buffered(overlay, |dc, _| unsafe {
                crate::window::menu_band::paint(dc, &headings, mode, palette, fonts.text(), dpi);
            });
            0
        }
        WM_MOUSEMOVE if !owner.is_null() => {
            let (_, _, packed) = to_owner(overlay, owner, lparam);
            hover_menu_heading(owner, packed);
            0
        }
        WM_LBUTTONDOWN if !owner.is_null() => {
            let (x, y, _) = to_owner(overlay, owner, lparam);
            if let Some(index) = menu_band::heading_at(&menu_headings(owner), x, y) {
                open_menu(owner, index);
            }
            0
        }
        _ => unsafe { DefWindowProcW(overlay, message, wparam, lparam) },
    }
}
