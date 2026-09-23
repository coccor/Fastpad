//! A tooltip control (`TOOLTIPS_CLASS`) for rectangles of one painted window. The control
//! subclasses that window to see the pointer. Windows makes the popup owned by the window's
//! top-level ancestor, so destroying the painted child leaves it alive: whoever destroys the child
//! calls `Tooltip::destroy` too.

use crate::platform::wide_null;
use windows_sys::Win32::Foundation::{HWND, LPARAM, POINT, RECT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::{
    ICC_BAR_CLASSES, INITCOMMONCONTROLSEX, InitCommonControlsEx, TOOLTIPS_CLASS, TTF_SUBCLASS,
    TTM_ADDTOOLW, TTM_DELTOOLW, TTM_RELAYEVENT, TTS_ALWAYSTIP, TTS_NOPREFIX, TTTOOLINFOW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CW_USEDEFAULT, CreateWindowExW, DestroyWindow, GetCursorPos, HWND_TOPMOST, MSG, SWP_NOACTIVATE,
    SWP_NOMOVE, SWP_NOSIZE, SendMessageW, SetWindowPos, WS_EX_TOPMOST, WS_POPUP,
};

/// FastPad has no comctl32 v6 manifest. The v5 control rejects the full `TTTOOLINFOW` size, and
/// then every `TTM_ADDTOOLW` fails silently, so the structure is sized up to `lpReserved`.
const TOOL_INFO_SIZE: u32 = std::mem::offset_of!(TTTOOLINFOW, lpReserved) as u32;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Tooltip {
    hwnd: HWND,
    owner: HWND,
}

impl Tooltip {
    /// A tooltip for rectangles of `owner`, or `None` if the control can't be created.
    pub(crate) fn create(owner: HWND) -> Option<Tooltip> {
        static INITIALIZED: std::sync::Once = std::sync::Once::new();
        INITIALIZED.call_once(|| {
            let controls = INITCOMMONCONTROLSEX {
                dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
                dwICC: ICC_BAR_CLASSES,
            };
            unsafe { InitCommonControlsEx(&controls) };
        });
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOPMOST,
                TOOLTIPS_CLASS,
                std::ptr::null(),
                WS_POPUP | TTS_ALWAYSTIP | TTS_NOPREFIX,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                owner,
                std::ptr::null_mut(),
                GetModuleHandleW(std::ptr::null()),
                std::ptr::null(),
            )
        };
        if hwnd.is_null() {
            return None;
        }
        unsafe {
            SetWindowPos(
                hwnd,
                HWND_TOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
        Some(Self { hwnd, owner })
    }

    /// Shows `text` while the pointer rests on `rect` (owner client coordinates). A tool with the
    /// same `id` is replaced, and an empty `text` removes it, so no empty tip ever shows.
    pub(crate) fn set_tool(&self, id: usize, rect: RECT, text: &str) {
        let mut wide = wide_null(text);
        let mut info = TTTOOLINFOW {
            cbSize: TOOL_INFO_SIZE,
            uFlags: TTF_SUBCLASS,
            hwnd: self.owner,
            uId: id,
            rect,
            lpszText: wide.as_mut_ptr(),
            ..Default::default()
        };
        unsafe {
            SendMessageW(
                self.hwnd,
                TTM_DELTOOLW,
                0,
                &info as *const TTTOOLINFOW as LPARAM,
            );
        }
        if text.is_empty() {
            return;
        }
        unsafe {
            SendMessageW(
                self.hwnd,
                TTM_ADDTOOLW,
                0,
                &mut info as *mut TTTOOLINFOW as LPARAM,
            );
        }
    }

    /// Hands the control a mouse message of `owner` that its subclass did not see: the one that
    /// made it, so the first hover starts the tip's timer like any later one.
    pub(crate) fn relay(&self, message: u32, wparam: WPARAM, lparam: LPARAM) {
        let mut point = POINT::default();
        unsafe { GetCursorPos(&mut point) };
        let msg = MSG {
            hwnd: self.owner,
            message,
            wParam: wparam,
            lParam: lparam,
            pt: point,
            ..Default::default()
        };
        unsafe {
            SendMessageW(self.hwnd, TTM_RELAYEVENT, 0, &msg as *const MSG as LPARAM);
        }
    }

    /// Destroys the control. The painted window it watches does not take it along.
    pub(crate) fn destroy(self) {
        unsafe { DestroyWindow(self.hwnd) };
    }

    #[cfg(test)]
    #[allow(
        dead_code,
        reason = "only the in-process window tests read the handle, not the source-linked targets"
    )]
    pub(crate) fn hwnd(&self) -> HWND {
        self.hwnd
    }

    #[cfg(test)]
    #[allow(
        dead_code,
        reason = "only the in-process window tests count tools, not the source-linked targets"
    )]
    pub(crate) fn tool_count(&self) -> usize {
        (unsafe {
            SendMessageW(
                self.hwnd,
                windows_sys::Win32::UI::Controls::TTM_GETTOOLCOUNT,
                0,
                0,
            )
        }) as usize
    }
}
