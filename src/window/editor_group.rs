//! An editor group: the child window that holds a group's editor, find bar, Markdown/SVG preview
//! and image view (split editors spec §4.2). Its children's notifications go on to the main
//! window unchanged, so they are handled as if the children were the main window's own. The
//! main window lays the group out; the group lays out its children in its own client area.

use crate::platform::wide_null;
use windows_sys::Win32::Foundation::{
    ERROR_CLASS_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, WPARAM,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CS_DBLCLKS, DefWindowProcW, IDC_ARROW, IDC_SIZEWE, LoadCursorW, RegisterClassW, SendMessageW,
    SetCursor, WM_CAPTURECHANGED, WM_COMMAND, WM_CTLCOLORBTN, WM_CTLCOLOREDIT,
    WM_CTLCOLORLISTBOX, WM_CTLCOLORSTATIC, WM_DRAWITEM, WM_ERASEBKGND, WM_LBUTTONDBLCLK,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_NOTIFY, WM_PAINT, WM_SETCURSOR, WM_SETFOCUS,
    WM_SIZE, WNDCLASSW, WS_CHILD, WS_CLIPCHILDREN, WS_CLIPSIBLINGS, WS_VISIBLE,
};

/// The window of one editor group. PR 1 of the split editors work has exactly one.
#[derive(Debug)]
pub(crate) struct GroupWindow {
    pub(crate) hwnd: HWND,
}

/// Creates the group window under `main`. It is empty until the editor and its companions are
/// created inside it.
pub(crate) fn create(main: HWND) -> crate::Result<HWND> {
    super::panel::create_child(
        main,
        register_class()?,
        WS_CHILD | WS_VISIBLE | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
    )
}

fn register_class() -> crate::Result<&'static [u16]> {
    static CLASS_NAME: std::sync::OnceLock<Vec<u16>> = std::sync::OnceLock::new();
    static REGISTERED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let name = CLASS_NAME.get_or_init(|| wide_null("FastPadEditorGroup"));
    let registered = *REGISTERED.get_or_init(|| {
        let class = WNDCLASSW {
            style: CS_DBLCLKS,
            lpfnWndProc: Some(group_proc),
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
            "the editor group window class could not be registered",
        ))
    }
}

fn point(lparam: LPARAM) -> (i32, i32) {
    (
        (lparam as u32 & 0xffff) as u16 as i16 as i32,
        ((lparam as u32 >> 16) & 0xffff) as u16 as i16 as i32,
    )
}

unsafe extern "system" fn group_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let main = crate::platform::win32::root_window(hwnd);
    match message {
        // Scintilla's notifications and the controls' commands and colors are the main window's
        // to handle, exactly as before the editor moved into the group.
        WM_NOTIFY | WM_COMMAND | WM_CTLCOLOREDIT | WM_CTLCOLORSTATIC | WM_CTLCOLORBTN
        | WM_CTLCOLORLISTBOX | WM_DRAWITEM => unsafe { SendMessageW(main, message, wparam, lparam) },
        WM_SIZE => {
            super::main_window::layout_group(main, hwnd);
            0
        }
        WM_SETFOCUS => {
            super::main_window::focus_group_content(main);
            0
        }
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            super::main_window::paint_group(main, hwnd);
            0
        }
        WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
            let (x, y) = point(lparam);
            super::preview_host::begin_divider_drag(main, hwnd, x, y);
            0
        }
        WM_MOUSEMOVE => {
            super::preview_host::drag_divider(main, point(lparam).0);
            0
        }
        WM_LBUTTONUP => {
            super::preview_host::end_divider_drag(main);
            0
        }
        WM_CAPTURECHANGED => {
            super::preview_host::cancel_divider_drag(main);
            0
        }
        WM_SETCURSOR if super::preview_host::cursor_over_divider(main, hwnd) => {
            unsafe { SetCursor(LoadCursorW(std::ptr::null_mut(), IDC_SIZEWE)) };
            1
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}
