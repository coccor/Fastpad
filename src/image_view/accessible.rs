//! MSAA for the image view (image preview spec §6.6): one graphic object with no children. Its name
//! says what the image is ("shot.png, image, 1920 by 1080 pixels") and its value is the zoom. It reads
//! a text snapshot the view keeps current, so it is safe to call from any thread.

use crate::window::accessibility::{
    AccessibleVtable, IID_IACCESSIBLE, IID_IDISPATCH, IID_IUNKNOWN, RawVariant, VariantValue,
    accessible_get_help_topic, accessible_get_ids_of_names, accessible_get_parent,
    accessible_get_type_info, accessible_get_type_info_count, accessible_invoke, allocate_bstr,
    guid_eq,
};
use std::ffi::c_void;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, RwLock};
use windows_sys::Win32::Foundation::{
    DISP_E_MEMBERNOTFOUND, E_INVALIDARG, E_NOINTERFACE, E_NOTIMPL, HWND, LRESULT, POINT, RECT,
    S_FALSE, S_OK, WPARAM,
};
use windows_sys::Win32::Graphics::Gdi::ScreenToClient;
use windows_sys::Win32::UI::Accessibility::{LresultFromObject, ROLE_SYSTEM_GRAPHIC};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GUITHREADINFO, GetClientRect, GetGUIThreadInfo, GetWindowRect, GetWindowThreadProcessId,
};
use windows_sys::core::{BSTR, GUID, HRESULT};

const STATE_SYSTEM_FOCUSED: u32 = 0x0000_0004;
const STATE_SYSTEM_READONLY: u32 = 0x0000_0040;
const STATE_SYSTEM_FOCUSABLE: u32 = 0x0010_0000;

const EMPTY_RECT: RECT = RECT {
    left: 0,
    top: 0,
    right: 0,
    bottom: 0,
};

/// The accessible name and value, shared between the view and its MSAA objects.
pub type AccessibleText = Arc<RwLock<(String, String)>>;

#[repr(C)]
struct ImageAccessible {
    vtable: &'static AccessibleVtable,
    references: AtomicU32,
    hwnd: HWND,
    text: AccessibleText,
}

pub(crate) static IMAGE_VTABLE: AccessibleVtable = AccessibleVtable {
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

fn create_provider(hwnd: HWND, text: AccessibleText) -> *mut c_void {
    Box::into_raw(Box::new(ImageAccessible {
        vtable: &IMAGE_VTABLE,
        references: AtomicU32::new(1),
        hwnd,
        text,
    }))
    .cast()
}

/// Answers `WM_GETOBJECT(OBJID_CLIENT)`. Call it with no view state borrowed: a client may call
/// back into the view while `LresultFromObject` runs.
pub fn object_result(hwnd: HWND, text: AccessibleText, wparam: WPARAM) -> LRESULT {
    let provider = create_provider(hwnd, text);
    let result = unsafe { LresultFromObject(&IID_IACCESSIBLE, wparam, provider) };
    unsafe { release(provider) };
    result
}

unsafe fn item<'a>(this: *mut c_void) -> &'a ImageAccessible {
    unsafe { &*this.cast::<ImageAccessible>() }
}

/// Only the object itself (`CHILDID_SELF`) exists.
fn is_self(child: &RawVariant) -> bool {
    child.child_id() == Some(0)
}

/// Asks the view's own thread: MSAA clients call in on RPC threads, where `GetFocus` reports that
/// thread's (empty) focus.
fn has_focus(item: &ImageAccessible) -> bool {
    if item.hwnd.is_null() {
        return false;
    }
    let thread = unsafe { GetWindowThreadProcessId(item.hwnd, std::ptr::null_mut()) };
    let mut info = GUITHREADINFO {
        cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
        ..Default::default()
    };
    thread != 0
        && unsafe { GetGUIThreadInfo(thread, &mut info) } != 0
        && info.hwndFocus == item.hwnd
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
        drop(unsafe { Box::from_raw(this.cast::<ImageAccessible>()) });
    }
    remaining
}

unsafe extern "system" fn child_count(_this: *mut c_void, count: *mut i32) -> HRESULT {
    if count.is_null() {
        return E_INVALIDARG;
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
    let text = unsafe { item(this) }
        .text
        .read()
        .unwrap_or_else(|error| error.into_inner())
        .0
        .clone();
    unsafe { allocate_bstr(&text, output) }
}

unsafe extern "system" fn value(
    this: *mut c_void,
    child: RawVariant,
    output: *mut BSTR,
) -> HRESULT {
    if !is_self(&child) {
        return E_INVALIDARG;
    }
    let text = unsafe { item(this) }
        .text
        .read()
        .unwrap_or_else(|error| error.into_inner())
        .1
        .clone();
    unsafe { allocate_bstr(&text, output) }
}

unsafe extern "system" fn empty_text(
    _this: *mut c_void,
    child: RawVariant,
    output: *mut BSTR,
) -> HRESULT {
    if !is_self(&child) {
        return E_INVALIDARG;
    }
    unsafe { allocate_bstr("", output) }
}

unsafe extern "system" fn role(
    _this: *mut c_void,
    child: RawVariant,
    output: *mut RawVariant,
) -> HRESULT {
    if output.is_null() || !is_self(&child) {
        return E_INVALIDARG;
    }
    unsafe { *output = RawVariant::integer(ROLE_SYSTEM_GRAPHIC as i32) };
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
    let focused = if has_focus(unsafe { item(this) }) {
        STATE_SYSTEM_FOCUSED
    } else {
        0
    };
    let state = STATE_SYSTEM_READONLY | STATE_SYSTEM_FOCUSABLE | focused;
    unsafe { *output = RawVariant::integer(state as i32) };
    S_OK
}

unsafe extern "system" fn focus(this: *mut c_void, output: *mut RawVariant) -> HRESULT {
    if output.is_null() {
        return E_INVALIDARG;
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
    if left.is_null() || top.is_null() || width.is_null() || height.is_null() {
        return E_INVALIDARG;
    }
    if !is_self(&child) {
        return E_INVALIDARG;
    }
    let item = unsafe { item(this) };
    let mut rect = EMPTY_RECT;
    if item.hwnd.is_null() || unsafe { GetWindowRect(item.hwnd, &mut rect) } == 0 {
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
    let item = unsafe { item(this) };
    let mut point = POINT { x, y };
    let mut client = EMPTY_RECT;
    let inside = !item.hwnd.is_null()
        && unsafe { ScreenToClient(item.hwnd, &mut point) } != 0
        && unsafe { GetClientRect(item.hwnd, &mut client) } != 0
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

unsafe extern "system" fn put_text(
    _this: *mut c_void,
    _child: RawVariant,
    _value: BSTR,
) -> HRESULT {
    E_NOTIMPL
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::Foundation::{SysFreeString, SysStringLen};

    fn read(
        this: *mut c_void,
        getter: unsafe extern "system" fn(*mut c_void, RawVariant, *mut BSTR) -> HRESULT,
    ) -> String {
        let mut output: BSTR = std::ptr::null_mut();
        assert_eq!(
            unsafe { getter(this, RawVariant::integer(0), &mut output) },
            S_OK
        );
        let length = unsafe { SysStringLen(output) } as usize;
        let text = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(output, length) });
        unsafe { SysFreeString(output) };
        text
    }

    #[test]
    fn the_graphic_reads_its_name_and_zoom_and_has_no_children() {
        // Break caught: a screen reader hearing nothing for an image, or walking into a child that
        // does not exist.
        let text: AccessibleText = Arc::new(RwLock::new((
            "a.png, image, 2 by 2 pixels".to_owned(),
            "Zoom 100 percent".to_owned(),
        )));
        let provider = create_provider(std::ptr::null_mut(), text);
        assert_eq!(read(provider, name), "a.png, image, 2 by 2 pixels");
        assert_eq!(read(provider, value), "Zoom 100 percent");
        let mut count = -1;
        assert_eq!(unsafe { child_count(provider, &mut count) }, S_OK);
        assert_eq!(count, 0);
        let mut output: BSTR = std::ptr::null_mut();
        assert_eq!(
            unsafe { name(provider, RawVariant::integer(1), &mut output) },
            E_INVALIDARG
        );
        let mut role_value = RawVariant::empty();
        assert_eq!(
            unsafe { role(provider, RawVariant::integer(0), &mut role_value) },
            S_OK
        );
        unsafe { release(provider) };
    }
}
