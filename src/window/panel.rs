//! A painted child window that hosts native controls for the command palette, the find bar and
//! the name box.
//! It draws its own background through the main window (which owns both widgets) and passes its
//! controls' notifications on to the main window unchanged, so they are handled exactly as if the
//! controls were the main window's own children.

use crate::platform::{last_error, wide_null};
use windows_sys::Win32::Foundation::{
    ERROR_CLASS_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, RECT, WPARAM,
};
use windows_sys::Win32::Graphics::Gdi::{
    DC_BRUSH, FillRect, GetDC, GetStockObject, GetTextMetricsW, HDC, HFONT, ReleaseDC,
    SelectObject, SetDCBrushColor, TEXTMETRICW,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::WM_MOUSELEAVE;
use windows_sys::Win32::UI::WindowsAndMessaging::HMENU;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetParent, IDC_ARROW, LoadCursorW, OBJID_CLIENT,
    RegisterClassW, SendMessageW, WM_COMMAND, WM_CTLCOLORBTN, WM_CTLCOLOREDIT, WM_CTLCOLORLISTBOX,
    WM_DRAWITEM, WM_ERASEBKGND, WM_GETOBJECT, WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT, WNDCLASSW,
    WS_CHILD, WS_CLIPCHILDREN, WS_CLIPSIBLINGS,
};

pub(crate) const fn scale(value: i32, dpi: u32) -> i32 {
    let dpi = if dpi == 0 { 96 } else { dpi };
    ((value as i64 * dpi as i64 + 48) / 96) as i32
}

/// Creates a hidden panel over the main window's other children; the editor clips against it.
pub(crate) fn create_panel(parent: HWND) -> crate::Result<HWND> {
    create_child(
        parent,
        register_panel_class()?,
        WS_CHILD | WS_CLIPSIBLINGS | WS_CLIPCHILDREN,
    )
}

pub(crate) unsafe fn fill(dc: HDC, rect: RECT, color: u32) {
    unsafe {
        SetDCBrushColor(dc, color);
        FillRect(dc, &rect, GetStockObject(DC_BRUSH));
    }
}

pub(crate) const fn inset(rect: RECT, by: i32) -> RECT {
    RECT {
        left: rect.left + by,
        top: rect.top + by,
        right: rect.right - by,
        bottom: rect.bottom - by,
    }
}

/// The pixel height of a line of `font` text, as the `Edit` will draw it.
pub(crate) fn text_height(control: HWND, font: HFONT) -> i32 {
    unsafe {
        let dc = GetDC(control);
        if dc.is_null() {
            return 0;
        }
        let previous = (!font.is_null()).then(|| SelectObject(dc, font as _));
        let mut metrics = TEXTMETRICW::default();
        let height = if GetTextMetricsW(dc, &mut metrics) != 0 {
            metrics.tmHeight
        } else {
            0
        };
        if let Some(previous) = previous {
            SelectObject(dc, previous);
        }
        ReleaseDC(control, dc);
        height
    }
}

/// Registers the panel window class once per process and returns its name.
fn register_panel_class() -> crate::Result<&'static [u16]> {
    static CLASS_NAME: std::sync::OnceLock<Vec<u16>> = std::sync::OnceLock::new();
    static REGISTERED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let name = CLASS_NAME.get_or_init(|| wide_null("FastPadCommandPalette"));
    let registered = *REGISTERED.get_or_init(|| {
        let class = WNDCLASSW {
            lpfnWndProc: Some(panel_proc),
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
            "the command palette window class could not be registered",
        ))
    }
}

/// The panel paints itself and passes its controls' notifications on to the main window.
unsafe extern "system" fn panel_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let main = unsafe { GetParent(hwnd) };
    match message {
        WM_PAINT => {
            super::main_window::paint_panel(main, hwnd);
            0
        }
        WM_ERASEBKGND => 1,
        WM_MOUSEMOVE | WM_MOUSELEAVE | WM_LBUTTONUP => {
            super::main_window::panel_pointer(main, hwnd, message, wparam, lparam);
            0
        }
        WM_COMMAND | WM_CTLCOLORBTN | WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX | WM_DRAWITEM => unsafe {
            SendMessageW(main, message, wparam, lparam)
        },
        // Only the find bar's panel answers with its own object. The palette and the name box
        // keep the system's, which lists their native controls. `find_bar_owns` lets go of the
        // App before `object_result` runs, as its contract requires.
        WM_GETOBJECT
            if lparam as i32 == OBJID_CLIENT && super::main_window::find_bar_owns(main, hwnd) =>
        unsafe {
            super::sidebar_accessibility::object_result(
                hwnd,
                &super::find_bar::FIND_BAR_ACCESSIBLE,
                wparam,
            )
        },
        super::sidebar_accessibility::WM_FASTPAD_SIDEBAR_ACCESSIBLE => unsafe {
            super::sidebar_accessibility::answer(hwnd, lparam)
        },
        super::sidebar_accessibility::WM_FASTPAD_SIDEBAR_ACTION
            if super::main_window::find_bar_owns(main, hwnd) =>
        {
            super::sidebar_accessibility::run_action(
                hwnd,
                &super::find_bar::FIND_BAR_ACCESSIBLE,
                wparam,
                lparam,
            );
            0
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

pub(crate) fn create_child(parent: HWND, class: &[u16], style: u32) -> crate::Result<HWND> {
    create_child_with_menu(parent, class, style, std::ptr::null_mut())
}

/// Like `create_child`, but with a control ID so `WM_COMMAND` can tell the child apart.
pub(crate) fn create_child_with_id(
    parent: HWND,
    class: &[u16],
    style: u32,
    id: u16,
) -> crate::Result<HWND> {
    create_child_with_menu(parent, class, style, id as usize as HMENU)
}

fn create_child_with_menu(
    parent: HWND,
    class: &[u16],
    style: u32,
    menu: HMENU,
) -> crate::Result<HWND> {
    let hwnd = unsafe {
        CreateWindowExW(
            0,
            class.as_ptr(),
            std::ptr::null(),
            style,
            0,
            0,
            0,
            0,
            parent,
            menu,
            std::ptr::null_mut(),
            std::ptr::null(),
        )
    };
    if hwnd.is_null() {
        Err(last_error())
    } else {
        Ok(hwnd)
    }
}
