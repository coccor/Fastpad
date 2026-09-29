//! An editor group: the child window that holds a group's tab strip, editor, find bar,
//! Markdown/SVG preview and image view (split editors spec §4.2). Its children's notifications go
//! on to the main window unchanged, so they are handled as if the children were the main window's
//! own. The main window lays the group out; the group lays out its children in its own client
//! area and paints the strip.

use crate::platform::wide_null;
use windows_sys::Win32::Foundation::{
    ERROR_CLASS_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, WPARAM,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CS_DBLCLKS, DefWindowProcW, IDC_ARROW, IDC_SIZEWE, LoadCursorW, OBJID_CLIENT, RegisterClassW,
    SendMessageW, SetCursor, WM_CAPTURECHANGED, WM_COMMAND, WM_CTLCOLORBTN, WM_CTLCOLOREDIT,
    WM_CTLCOLORLISTBOX, WM_CTLCOLORSTATIC, WM_DRAWITEM, WM_ERASEBKGND, WM_GETOBJECT,
    WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MOUSEMOVE, WM_NCHITTEST,
    WM_NOTIFY, WM_PAINT, WM_RBUTTONDOWN, WM_SETCURSOR, WM_SETFOCUS, WM_SIZE, WNDCLASSW, WS_CHILD,
    WS_CLIPCHILDREN, WS_CLIPSIBLINGS, WS_VISIBLE,
};

/// The window of one editor group, what it shows, and its tab strip's pointer state. One per
/// editor group; the main window keeps them in `App.groups`.
#[derive(Debug)]
pub(crate) struct GroupWindow {
    pub(crate) id: super::split_tree::GroupId,
    pub(crate) hwnd: HWND,
    pub(crate) editor: crate::editor::Editor,
    /// Created the first time Find or Replace opens in this group.
    pub(crate) find_bar: Option<super::find_bar::FindBar>,
    /// Declared before `preview`, whose Direct2D objects the image view's may share.
    pub(crate) image: super::image_host::ImageHost,
    pub(crate) preview: super::preview_host::PreviewHost,
    pub(crate) pointer: super::group_strip::StripPointer,
    /// While the tab scroll thumb is dragged: where along the thumb the pointer grabbed it.
    pub(crate) thumb_grab: Option<i32>,
    /// Between a middle-button press on a tab and its release: the tab's strip index and the
    /// document it showed then (quick-open spec §5).
    pub(crate) middle_press: Option<(usize, crate::document::DocumentId)>,
    /// The content area (below the strip, band and find bar), in group-client coordinates, as
    /// last laid out.
    pub(crate) content: super::titlebar::Rect,
    /// The last tab click (its document and message time), so a second click on the same tab
    /// within the double-click time keeps a preview tab.
    pub(crate) last_tab_click: Option<(crate::document::DocumentId, u32)>,
    /// The tab strip's accessibility provider, created on the first `WM_GETOBJECT`.
    pub(crate) accessibility: super::accessibility::AccessibilityState,
    /// The Markdown/SVG preview buttons floating over the content.
    pub(crate) preview_buttons: super::preview_buttons::PreviewButtons,
}

impl GroupWindow {
    pub(crate) fn new(
        id: super::split_tree::GroupId,
        hwnd: HWND,
        editor: crate::editor::Editor,
    ) -> Self {
        Self {
            id,
            hwnd,
            editor,
            find_bar: None,
            image: Default::default(),
            preview: Default::default(),
            pointer: Default::default(),
            thumb_grab: None,
            middle_press: None,
            content: Default::default(),
            last_tab_click: None,
            accessibility: Default::default(),
            preview_buttons: Default::default(),
        }
    }
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
    // A press anywhere in a group makes it the active group before anything handles the press.
    if matches!(
        message,
        WM_LBUTTONDOWN | WM_LBUTTONDBLCLK | WM_RBUTTONDOWN | WM_MBUTTONDOWN
    ) {
        super::main_window::press_group_window(main, hwnd);
    }
    match message {
        // Scintilla's notifications and the controls' commands and colors are the main window's
        // to handle, exactly as before the editor moved into the group.
        WM_NOTIFY | WM_COMMAND | WM_CTLCOLOREDIT | WM_CTLCOLORSTATIC | WM_CTLCOLORBTN
        | WM_CTLCOLORLISTBOX | WM_DRAWITEM => unsafe {
            SendMessageW(main, message, wparam, lparam)
        },
        // The strip's accessible selection goes to the main window too, which activates the tab
        // in this group: the group window rides along as `wparam`.
        super::accessibility::WM_FASTPAD_ACCESSIBLE_SELECT => unsafe {
            SendMessageW(main, message, hwnd as WPARAM, lparam)
        },
        WM_NCHITTEST => super::main_window::group_hit_test(main, hwnd, lparam),
        WM_GETOBJECT if lparam as i32 == OBJID_CLIENT => {
            super::main_window::group_accessible_object(main, hwnd, wparam)
        }
        WM_SIZE => {
            super::main_window::layout_group(main, hwnd);
            0
        }
        WM_SETFOCUS => {
            super::main_window::activate_group_window(main, hwnd);
            super::main_window::focus_group_content(main);
            0
        }
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            super::main_window::paint_group(main, hwnd);
            0
        }
        // The preview divider takes a press first; everything else is the tab strip's.
        WM_LBUTTONDOWN | WM_LBUTTONDBLCLK
            if {
                let (x, y) = point(lparam);
                super::preview_host::begin_divider_drag(main, hwnd, x, y)
            } =>
        {
            0
        }
        WM_MOUSEMOVE if super::preview_host::drag_divider(main, point(lparam).0) => 0,
        WM_LBUTTONUP if super::preview_host::end_divider_drag(main) => 0,
        WM_CAPTURECHANGED => {
            super::preview_host::cancel_divider_drag(main);
            super::main_window::group_strip_message(main, hwnd, message, wparam, lparam);
            0
        }
        WM_SETCURSOR if super::preview_host::cursor_over_divider(main, hwnd) => {
            unsafe { SetCursor(LoadCursorW(std::ptr::null_mut(), IDC_SIZEWE)) };
            1
        }
        _ => super::main_window::group_strip_message(main, hwnd, message, wparam, lparam)
            .unwrap_or_else(|| unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }),
    }
}
