//! Dynamic annotation through `IAccPropServices` (oleacc): a native control's accessible name
//! and description, kept for its window by the system's own proxy, so screen readers hear them
//! from the control itself (inline naming spec §6).

use crate::{FastPadError, Result};
use std::ffi::c_void;
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    CoUninitialize,
};
use windows_sys::Win32::UI::Accessibility::{
    CAccPropServices, PROPID_ACC_DESCRIPTION, PROPID_ACC_NAME,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{CHILDID_SELF, OBJID_CLIENT};
use windows_sys::core::{GUID, HRESULT};

const IID_IACC_PROP_SERVICES: GUID = GUID::from_u128(0x6e26e776_04f0_495d_80e4_3330352e3169);
/// COM is already initialized on this thread in the other apartment model: usable, not ours
/// to uninitialize.
const RPC_E_CHANGED_MODE: HRESULT = 0x8001_0106_u32 as HRESULT;

// The SDK's IUnknown -> IAccPropServices order, up to the one method used.
#[repr(C)]
struct AccPropServicesVtable {
    query_interface: usize,
    add_ref: usize,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    set_prop_value: usize,
    set_prop_server: usize,
    clear_props: usize,
    set_hwnd_prop: usize,
    set_hwnd_prop_str:
        unsafe extern "system" fn(*mut c_void, HWND, u32, u32, GUID, *const u16) -> HRESULT,
}

fn check(status: HRESULT) -> Result<()> {
    if status < 0 {
        Err(FastPadError::Win32(status as u32))
    } else {
        Ok(())
    }
}

/// Gives `hwnd` (a native control) the accessible `name` and `description`. The annotation
/// stays with the window until it is destroyed; calling again replaces it.
pub fn annotate(hwnd: HWND, name: &str, description: &str) -> Result<()> {
    let initialized = unsafe { CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32) };
    if initialized < 0 && initialized != RPC_E_CHANGED_MODE {
        return Err(FastPadError::Win32(initialized as u32));
    }
    let result = set_strings(hwnd, name, description);
    if initialized >= 0 {
        // S_FALSE too: every successful CoInitializeEx is balanced.
        unsafe { CoUninitialize() };
    }
    result
}

fn set_strings(hwnd: HWND, name: &str, description: &str) -> Result<()> {
    let mut services: *mut c_void = std::ptr::null_mut();
    check(unsafe {
        CoCreateInstance(
            &CAccPropServices,
            std::ptr::null_mut(),
            CLSCTX_INPROC_SERVER,
            &IID_IACC_PROP_SERVICES,
            &mut services,
        )
    })?;
    if services.is_null() {
        return Err(FastPadError::Invariant("IAccPropServices was not created"));
    }
    let vtable = unsafe { &**(services as *const *const AccPropServicesVtable) };
    let set = |property: GUID, text: &str| {
        let wide = crate::platform::wide_null(text);
        check(unsafe {
            (vtable.set_hwnd_prop_str)(
                services,
                hwnd,
                OBJID_CLIENT as u32,
                CHILDID_SELF,
                property,
                wide.as_ptr(),
            )
        })
    };
    let result = set(PROPID_ACC_NAME, name).and_then(|()| set(PROPID_ACC_DESCRIPTION, description));
    unsafe { (vtable.release)(services) };
    result
}
