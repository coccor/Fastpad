//! The `FastPadLive` window procedure: paint, size, focus, the system caret, keys, IME, mouse,
//! scrolling and text queries.

use super::paint::{PaintOutcome, paint};
use super::*;
use windows::Win32::Graphics::Direct2D::Common::D2D_SIZE_U;
use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::ValidateRect;
use windows_sys::Win32::UI::Controls::SetScrollInfo;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_CONTROL;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateCaret, DLGC_WANTALLKEYS, DefWindowProcW, DestroyCaret, KillTimer, SB_VERT,
    WM_CAPTURECHANGED, WM_CHAR, WM_DPICHANGED_AFTERPARENT, WM_ERASEBKGND, WM_GETDLGCODE,
    WM_GETTEXT, WM_GETTEXTLENGTH, WM_IME_STARTCOMPOSITION, WM_KEYDOWN, WM_KEYUP, WM_KILLFOCUS,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCDESTROY, WM_PAINT, WM_SETFOCUS,
    WM_SIZE, WM_TIMER, WM_VSCROLL,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{HTCLIENT, WM_SETCURSOR};

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
                // The system caret is never shown; it tells screen readers, magnifiers and IMEs
                // where the drawn caret is. Paint moves it.
                let height = with_state(hwnd, |state| {
                    (state.layout.fonts().line_height * dpi_scale(hwnd)).round() as i32
                })
                .unwrap_or(16);
                unsafe { CreateCaret(hwnd, std::ptr::null_mut(), 1, height) };
                crate::window::post_content_focus(hwnd);
                super::keys::restart_blink(hwnd);
            } else {
                unsafe {
                    KillTimer(hwnd, super::keys::CARET_TIMER);
                    DestroyCaret();
                }
                super::mouse::release(hwnd, true);
            }
            invalidate(hwnd);
            0
        }
        WM_SETCURSOR if (lparam & 0xFFFF) as u32 == HTCLIENT => {
            super::mouse::update_cursor(hwnd);
            1
        }
        WM_KEYDOWN | WM_KEYUP if wparam == usize::from(VK_CONTROL) => {
            super::mouse::update_cursor(hwnd);
            super::keys::handle(hwnd, message, wparam, lparam)
                .unwrap_or_else(|| unsafe { DefWindowProcW(hwnd, message, wparam, lparam) })
        }
        WM_KEYDOWN | WM_KEYUP | WM_CHAR => super::keys::handle(hwnd, message, wparam, lparam)
            .unwrap_or_else(|| unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }),
        WM_VSCROLL => {
            super::scrolling::vscroll(hwnd, wparam);
            0
        }
        WM_MOUSEWHEEL => {
            super::scrolling::wheel(hwnd, wparam);
            0
        }
        WM_TIMER if wparam == super::keys::CARET_TIMER => {
            super::keys::blink(hwnd);
            0
        }
        WM_TIMER if wparam == super::mouse::AUTOSCROLL_TIMER => {
            super::mouse::autoscroll(hwnd);
            0
        }
        WM_LBUTTONDOWN => {
            super::mouse::press(hwnd, lparam);
            0
        }
        WM_MOUSEMOVE => {
            super::mouse::moved(hwnd, lparam);
            0
        }
        WM_LBUTTONUP | WM_CAPTURECHANGED => {
            super::mouse::release(hwnd, message == WM_LBUTTONUP);
            0
        }
        WM_IME_STARTCOMPOSITION => {
            let place = with_state(hwnd, |state| {
                let (x, y, height) = state.caret_px?;
                let fonts = state.layout.fonts();
                Some(super::ime::CaretPlace {
                    x,
                    y,
                    height,
                    font_pixels: (fonts.size * dpi_scale(hwnd)).round() as i32,
                    family: fonts.family.clone(),
                })
            })
            .flatten();
            if let Some(place) = place {
                super::ime::place(hwnd, &place);
            }
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
        crate::window::WM_FASTPAD_LIVE_IMAGE => {
            super::images_ready(hwnd);
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
