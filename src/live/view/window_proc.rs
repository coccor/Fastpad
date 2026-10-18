//! The `FastPadLive` window procedure: paint, size, focus and text queries. Keyboard, mouse,
//! scrolling and IME arrive in later tasks of the phase 1 plan.

use super::paint::{PaintOutcome, paint};
use super::*;
use windows::Win32::Graphics::Direct2D::Common::D2D_SIZE_U;
use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::ValidateRect;
use windows_sys::Win32::UI::Controls::SetScrollInfo;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DLGC_WANTALLKEYS, DefWindowProcW, SB_VERT, WM_DPICHANGED_AFTERPARENT, WM_ERASEBKGND,
    WM_GETDLGCODE, WM_GETTEXT, WM_GETTEXTLENGTH, WM_KILLFOCUS, WM_NCDESTROY, WM_PAINT, WM_SETFOCUS,
    WM_SIZE,
};

pub(super) unsafe extern "system" fn live_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_NCDESTROY => {
            let pointer = unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) } as *mut ViewState;
            if !pointer.is_null() {
                drop(unsafe { Box::from_raw(pointer) });
            }
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            let outcome = with_state(hwnd, |state| match paint(state) {
                Ok(outcome) => {
                    state.paint_retried = outcome.repaint && state.paint_retried;
                    outcome
                }
                Err(_) => {
                    state.target = None;
                    state.brushes = None;
                    // Retry a failed frame once; a device that keeps failing must not spin.
                    PaintOutcome {
                        scroll: None,
                        repaint: !std::mem::replace(&mut state.paint_retried, true),
                    }
                }
            });
            unsafe { ValidateRect(hwnd, std::ptr::null()) };
            // The state borrow has ended: these calls may send messages back to this window.
            if let Some(outcome) = outcome {
                if let Some(info) = outcome.scroll {
                    unsafe { SetScrollInfo(hwnd, SB_VERT, &info, 1) };
                }
                if outcome.repaint {
                    invalidate(hwnd);
                }
            }
            0
        }
        WM_SIZE => {
            let width = (lparam & 0xFFFF) as u32;
            let height = ((lparam >> 16) & 0xFFFF) as u32;
            with_state(hwnd, |state| {
                if let Some(target) = &state.target
                    && unsafe {
                        target.Resize(&D2D_SIZE_U {
                            width: width.max(1),
                            height: height.max(1),
                        })
                    }
                    .is_err()
                {
                    state.target = None;
                    state.brushes = None;
                }
            });
            invalidate(hwnd);
            0
        }
        WM_GETDLGCODE => DLGC_WANTALLKEYS as LRESULT,
        WM_SETFOCUS | WM_KILLFOCUS => {
            with_state(hwnd, |state| {
                state.focused = message == WM_SETFOCUS;
                state.caret_on = true;
            });
            if message == WM_SETFOCUS {
                crate::window::post_content_focus(hwnd);
            }
            invalidate(hwnd);
            0
        }
        WM_DPICHANGED_AFTERPARENT => {
            invalidate(hwnd);
            0
        }
        // Screen readers and tests read the document as the source view's text.
        WM_GETTEXT | WM_GETTEXTLENGTH => with_state(hwnd, |state| state.editor.clone())
            .map_or(0, |editor| editor.forward_get_text(message, wparam, lparam)),
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}
