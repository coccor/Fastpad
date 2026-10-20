//! MSAA for the Live view (2026-10-18 Live editor design spec §9; phase 4 plan, Decision 8): one
//! editable-text object with no children, named "Live Markdown", whose value is the document's
//! text. It holds only the window: the text comes from `WM_GETTEXT`, which a client's RPC thread
//! may send, and once the window is gone every call answers `CO_E_OBJNOTCONNECTED`. The system
//! caret, which the view keeps at its drawn caret, gives the caret's place.

use crate::window::accessibility::{
    AccessibleVtable, IID_IACCESSIBLE, IID_IDISPATCH, IID_IUNKNOWN, RawVariant, VariantValue,
    accessible_get_help_topic, accessible_get_ids_of_names, accessible_get_parent,
    accessible_get_type_info, accessible_get_type_info_count, accessible_invoke, allocate_bstr,
    guid_eq,
};
use std::ffi::c_void;
use std::sync::atomic::{AtomicU32, Ordering};
use windows_sys::Win32::Foundation::{
    CO_E_OBJNOTCONNECTED, DISP_E_MEMBERNOTFOUND, E_INVALIDARG, E_NOINTERFACE, E_NOTIMPL, HWND,
    LRESULT, POINT, RECT, S_FALSE, S_OK, WPARAM,
};
use windows_sys::Win32::Graphics::Gdi::ScreenToClient;
use windows_sys::Win32::UI::Accessibility::{LresultFromObject, ROLE_SYSTEM_TEXT};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GUITHREADINFO, GetClientRect, GetGUIThreadInfo, GetWindowRect, GetWindowThreadProcessId,
    IsWindow, SendMessageW, WM_GETTEXT, WM_GETTEXTLENGTH,
};
use windows_sys::core::{BSTR, GUID, HRESULT};

const STATE_SYSTEM_FOCUSED: u32 = 0x0000_0004;
const STATE_SYSTEM_FOCUSABLE: u32 = 0x0010_0000;

/// What a screen reader announces for the view.
pub const LIVE_ACCESSIBLE_NAME: &str = "Live Markdown";

const EMPTY_RECT: RECT = RECT {
    left: 0,
    top: 0,
    right: 0,
    bottom: 0,
};

#[repr(C)]
struct LiveAccessible {
    vtable: &'static AccessibleVtable,
    references: AtomicU32,
    hwnd: HWND,
}

static LIVE_VTABLE: AccessibleVtable = AccessibleVtable {
    query_interface,
    add_ref,
    release,
    get_type_info_count: accessible_get_type_info_count,
    get_type_info: accessible_get_type_info,
    get_ids_of_names: accessible_get_ids_of_names,
    invoke: accessible_invoke,
    get_acc_parent: accessible_get_parent,
    get_acc_child_count: child_count,
    get_acc_child: child,
    get_acc_name: name,
    get_acc_value: value,
    get_acc_description: empty_text,
    get_acc_role: role,
    get_acc_state: state,
    get_acc_help: empty_text,
    get_acc_help_topic: accessible_get_help_topic,
    get_acc_keyboard_shortcut: empty_text,
    get_acc_focus: focus,
    get_acc_selection: empty_variant,
    get_acc_default_action: empty_text,
    acc_select: select,
    acc_location: location,
    acc_navigate: navigate,
    acc_hit_test: hit_test,
    acc_do_default_action: do_default_action,
    put_acc_name: put_text,
    put_acc_value: put_text,
};

/// Answers `WM_GETOBJECT(OBJID_CLIENT)`. Call it with no view state borrowed: a client may call
/// back into the view while `LresultFromObject` runs.
pub(super) fn object_result(hwnd: HWND, wparam: WPARAM) -> LRESULT {
    let provider = Box::into_raw(Box::new(LiveAccessible {
        vtable: &LIVE_VTABLE,
        references: AtomicU32::new(1),
        hwnd,
    }))
    .cast::<c_void>();
    let result = unsafe { LresultFromObject(&IID_IACCESSIBLE, wparam, provider) };
    unsafe { release(provider) };
    result
}

unsafe fn item<'a>(this: *mut c_void) -> &'a LiveAccessible {
    unsafe { &*this.cast::<LiveAccessible>() }
}

/// The window still exists: a client may keep the object after Live is turned off.
fn connected(this: *mut c_void) -> bool {
    (unsafe { IsWindow(item(this).hwnd) }) != 0
}

/// Only the object itself (`CHILDID_SELF`) exists.
fn is_self(child: &RawVariant) -> bool {
    child.child_id() == Some(0)
}

/// Asks the view's own thread: MSAA clients call in on RPC threads, where `GetFocus` reports that
/// thread's (empty) focus.
fn has_focus(item: &LiveAccessible) -> bool {
    let thread = unsafe { GetWindowThreadProcessId(item.hwnd, std::ptr::null_mut()) };
    let mut info = GUITHREADINFO {
        cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
        ..Default::default()
    };
    thread != 0
        && unsafe { GetGUIThreadInfo(thread, &mut info) } != 0
        && info.hwndFocus == item.hwnd
}

/// The document's text, as the view answers `WM_GETTEXT`.
fn document_text(hwnd: HWND) -> String {
    let length = unsafe { SendMessageW(hwnd, WM_GETTEXTLENGTH, 0, 0) }.max(0) as usize;
    let mut buffer = vec![0u16; length + 1];
    let copied = unsafe {
        SendMessageW(hwnd, WM_GETTEXT, buffer.len(), buffer.as_mut_ptr() as isize)
    }
    .max(0) as usize;
    String::from_utf16_lossy(&buffer[..copied.min(length)])
}

unsafe extern "system" fn query_interface(
    this: *mut c_void,
    iid: *const GUID,
    output: *mut *mut c_void,
) -> HRESULT {
    if iid.is_null() || output.is_null() {
        return E_INVALIDARG;
    }
    let requested = unsafe { *iid };
    if guid_eq(&requested, &IID_IUNKNOWN)
        || guid_eq(&requested, &IID_IDISPATCH)
        || guid_eq(&requested, &IID_IACCESSIBLE)
    {
        unsafe {
            *output = this;
            add_ref(this);
        }
        S_OK
    } else {
        unsafe { *output = std::ptr::null_mut() };
        E_NOINTERFACE
    }
}

unsafe extern "system" fn add_ref(this: *mut c_void) -> u32 {
    unsafe { item(this) }
        .references
        .fetch_add(1, Ordering::Relaxed)
        + 1
}

unsafe extern "system" fn release(this: *mut c_void) -> u32 {
    let remaining = unsafe { item(this) }
        .references
        .fetch_sub(1, Ordering::Release)
        - 1;
    if remaining == 0 {
        drop(unsafe { Box::from_raw(this.cast::<LiveAccessible>()) });
    }
    remaining
}

unsafe extern "system" fn child_count(this: *mut c_void, count: *mut i32) -> HRESULT {
    if count.is_null() {
        return E_INVALIDARG;
    }
    if !connected(this) {
        return CO_E_OBJNOTCONNECTED;
    }
    unsafe { *count = 0 };
    S_OK
}

unsafe extern "system" fn child(
    _this: *mut c_void,
    _child: RawVariant,
    output: *mut *mut c_void,
) -> HRESULT {
    if !output.is_null() {
        unsafe { *output = std::ptr::null_mut() };
    }
    E_INVALIDARG
}

unsafe extern "system" fn name(this: *mut c_void, child: RawVariant, output: *mut BSTR) -> HRESULT {
    if !is_self(&child) {
        return E_INVALIDARG;
    }
    if !connected(this) {
        return CO_E_OBJNOTCONNECTED;
    }
    unsafe { allocate_bstr(LIVE_ACCESSIBLE_NAME, output) }
}

unsafe extern "system" fn value(
    this: *mut c_void,
    child: RawVariant,
    output: *mut BSTR,
) -> HRESULT {
    if !is_self(&child) {
        return E_INVALIDARG;
    }
    if !connected(this) {
        return CO_E_OBJNOTCONNECTED;
    }
    let text = document_text(unsafe { item(this) }.hwnd);
    unsafe { allocate_bstr(&text, output) }
}

unsafe extern "system" fn empty_text(
    this: *mut c_void,
    child: RawVariant,
    output: *mut BSTR,
) -> HRESULT {
    if !is_self(&child) {
        return E_INVALIDARG;
    }
    if !connected(this) {
        return CO_E_OBJNOTCONNECTED;
    }
    unsafe { allocate_bstr("", output) }
}

unsafe extern "system" fn role(
    this: *mut c_void,
    child: RawVariant,
    output: *mut RawVariant,
) -> HRESULT {
    if output.is_null() || !is_self(&child) {
        return E_INVALIDARG;
    }
    if !connected(this) {
        return CO_E_OBJNOTCONNECTED;
    }
    unsafe { *output = RawVariant::integer(ROLE_SYSTEM_TEXT as i32) };
    S_OK
}

unsafe extern "system" fn state(
    this: *mut c_void,
    child: RawVariant,
    output: *mut RawVariant,
) -> HRESULT {
    if output.is_null() || !is_self(&child) {
        return E_INVALIDARG;
    }
    if !connected(this) {
        return CO_E_OBJNOTCONNECTED;
    }
    let focused = if has_focus(unsafe { item(this) }) {
        STATE_SYSTEM_FOCUSED
    } else {
        0
    };
    unsafe { *output = RawVariant::integer((STATE_SYSTEM_FOCUSABLE | focused) as i32) };
    S_OK
}

unsafe extern "system" fn focus(this: *mut c_void, output: *mut RawVariant) -> HRESULT {
    if output.is_null() {
        return E_INVALIDARG;
    }
    if !connected(this) {
        return CO_E_OBJNOTCONNECTED;
    }
    if has_focus(unsafe { item(this) }) {
        unsafe { *output = RawVariant::integer(0) };
        S_OK
    } else {
        unsafe { *output = RawVariant::empty() };
        S_FALSE
    }
}

unsafe extern "system" fn empty_variant(_this: *mut c_void, output: *mut RawVariant) -> HRESULT {
    if output.is_null() {
        return E_INVALIDARG;
    }
    unsafe { *output = RawVariant::empty() };
    S_FALSE
}

unsafe extern "system" fn select(_this: *mut c_void, _flags: i32, _child: RawVariant) -> HRESULT {
    DISP_E_MEMBERNOTFOUND
}

unsafe extern "system" fn location(
    this: *mut c_void,
    left: *mut i32,
    top: *mut i32,
    width: *mut i32,
    height: *mut i32,
    child: RawVariant,
) -> HRESULT {
    if left.is_null() || top.is_null() || width.is_null() || height.is_null() || !is_self(&child) {
        return E_INVALIDARG;
    }
    if !connected(this) {
        return CO_E_OBJNOTCONNECTED;
    }
    let mut rect = EMPTY_RECT;
    if unsafe { GetWindowRect(item(this).hwnd, &mut rect) } == 0 {
        return S_FALSE;
    }
    unsafe {
        *left = rect.left;
        *top = rect.top;
        *width = rect.right - rect.left;
        *height = rect.bottom - rect.top;
    }
    S_OK
}

unsafe extern "system" fn navigate(
    _this: *mut c_void,
    _direction: i32,
    _start: RawVariant,
    output: *mut RawVariant,
) -> HRESULT {
    if !output.is_null() {
        unsafe { *output = RawVariant::empty() };
    }
    S_FALSE
}

unsafe extern "system" fn hit_test(
    this: *mut c_void,
    x: i32,
    y: i32,
    output: *mut RawVariant,
) -> HRESULT {
    if output.is_null() {
        return E_INVALIDARG;
    }
    if !connected(this) {
        return CO_E_OBJNOTCONNECTED;
    }
    let hwnd = unsafe { item(this) }.hwnd;
    let mut point = POINT { x, y };
    let mut client = EMPTY_RECT;
    let inside = unsafe { ScreenToClient(hwnd, &mut point) } != 0
        && unsafe { GetClientRect(hwnd, &mut client) } != 0
        && point.x >= client.left
        && point.x < client.right
        && point.y >= client.top
        && point.y < client.bottom;
    if inside {
        unsafe { *output = RawVariant::integer(0) };
        S_OK
    } else {
        unsafe { *output = RawVariant::empty() };
        S_FALSE
    }
}

unsafe extern "system" fn do_default_action(_this: *mut c_void, _child: RawVariant) -> HRESULT {
    DISP_E_MEMBERNOTFOUND
}

/// Edits go through typing, as in Scintilla's own object.
unsafe extern "system" fn put_text(
    _this: *mut c_void,
    _child: RawVariant,
    _value: BSTR,
) -> HRESULT {
    E_NOTIMPL
}
