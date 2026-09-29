//! The drop overlay (split editors spec §6): a layered, click-through popup over the rectangle a
//! dragged tab would end up in, or a thin bar at a strip's insertion point. The same technique
//! as `drag_label`, painted with one flat colour.

// Used by tab drags from Task 4.
#![allow(dead_code)]

use crate::platform::wide_null;
use std::sync::atomic::{AtomicBool, Ordering};
use windows_sys::Win32::Foundation::{
    ERROR_CLASS_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, RECT, WPARAM,
};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, DeleteObject, EndPaint, FillRect, InvalidateRect, PAINTSTRUCT,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GWLP_USERDATA, GetWindowLongPtrW,
    GetWindowRect, HTTRANSPARENT, LWA_ALPHA, RegisterClassW, SW_SHOWNOACTIVATE, SWP_NOACTIVATE,
    SWP_NOZORDER, SetLayeredWindowAttributes, SetWindowLongPtrW, SetWindowPos, ShowWindow,
    WM_ERASEBKGND, WM_NCHITTEST, WM_PAINT, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT, WS_POPUP,
};

const CLASS: &str = "FastPadDropOverlay";
/// An area's tint: the target shows through (PR 3 amendment 4).
pub(crate) const TINT_ALPHA: u8 = 80;
/// The strip's insertion bar: opaque.
pub(crate) const BAR_ALPHA: u8 = 255;

/// The overlay popup while a drag has a target.
#[derive(Clone, Copy)]
pub(crate) struct DropOverlay {
    hwnd: HWND,
}

impl std::fmt::Debug for DropOverlay {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DropOverlay")
            .finish_non_exhaustive()
    }
}

impl DropOverlay {
    /// Shows `color` over screen rectangle `rect` at `alpha`, owned by `owner`. `None` if the
    /// popup can't be made: the drag goes on without it. Call it with nothing of the App borrowed.
    pub(crate) fn show(owner: HWND, rect: RECT, color: u32, alpha: u8) -> Option<Self> {
        let instance = unsafe { GetModuleHandleW(std::ptr::null()) };
        let class = wide_null(CLASS);
        static REGISTERED: AtomicBool = AtomicBool::new(false);
        if !REGISTERED.load(Ordering::Relaxed) {
            let window_class = WNDCLASSW {
                lpfnWndProc: Some(overlay_proc),
                hInstance: instance,
                lpszClassName: class.as_ptr(),
                ..Default::default()
            };
            if unsafe { RegisterClassW(&window_class) } == 0
                && unsafe { GetLastError() } != ERROR_CLASS_ALREADY_EXISTS
            {
                return None;
            }
            REGISTERED.store(true, Ordering::Relaxed);
        }
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
                class.as_ptr(),
                std::ptr::null(),
                WS_POPUP,
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
                owner,
                std::ptr::null_mut(),
                instance,
                std::ptr::null(),
            )
        };
        if hwnd.is_null() {
            return None;
        }
        let overlay = Self { hwnd };
        overlay.place(rect, color, alpha);
        unsafe { ShowWindow(hwnd, SW_SHOWNOACTIVATE) };
        Some(overlay)
    }

    /// Moves the overlay to screen rectangle `rect` and repaints it in `color` at `alpha`.
    pub(crate) fn place(&self, rect: RECT, color: u32, alpha: u8) {
        unsafe {
            SetWindowLongPtrW(self.hwnd, GWLP_USERDATA, color as isize);
            SetLayeredWindowAttributes(self.hwnd, 0, alpha, LWA_ALPHA);
            SetWindowPos(
                self.hwnd,
                std::ptr::null_mut(),
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            InvalidateRect(self.hwnd, std::ptr::null(), 0);
        }
    }

    /// Where the overlay is, on the screen.
    pub(crate) fn rect(&self) -> RECT {
        let mut rect = RECT::default();
        unsafe { GetWindowRect(self.hwnd, &mut rect) };
        rect
    }

    /// Destroys the popup. Call it with nothing of the App borrowed.
    pub(crate) fn destroy(self) {
        unsafe { DestroyWindow(self.hwnd) };
    }

    #[cfg(test)]
    pub(crate) fn hwnd(&self) -> HWND {
        self.hwnd
    }
}

unsafe extern "system" fn overlay_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_NCHITTEST => HTTRANSPARENT as LRESULT,
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            let mut paint = PAINTSTRUCT::default();
            let dc = unsafe { BeginPaint(hwnd, &mut paint) };
            let color = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as u32;
            let brush = unsafe { CreateSolidBrush(color) };
            unsafe {
                FillRect(dc, &paint.rcPaint, brush);
                DeleteObject(brush);
                EndPaint(hwnd, &paint);
            }
            0
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}
