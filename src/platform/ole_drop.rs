//! The raw OLE drag-and-drop plumbing the drop targets share: the `IDropTarget` and `IDataObject`
//! layouts, the interface IDs, and reading the files a drag offers as `CF_HDROP`.

use std::cell::Cell;
use std::ffi::c_void;
use std::path::PathBuf;
use windows_sys::Win32::Foundation::{POINTL, S_OK};
use windows_sys::Win32::System::Com::{DVASPECT_CONTENT, FORMATETC, STGMEDIUM, TYMED_HGLOBAL};
use windows_sys::Win32::System::Ole::{CF_HDROP, OleInitialize, ReleaseStgMedium};
use windows_sys::core::{GUID, HRESULT};

pub(crate) const IID_IUNKNOWN: GUID = GUID::from_u128(0x00000000_0000_0000_c000_000000000046);
pub(crate) const IID_IDROPTARGET: GUID = GUID::from_u128(0x00000122_0000_0000_c000_000000000046);

pub(crate) type Unknown = *mut c_void;

#[repr(C)]
pub(crate) struct DropTargetVtbl {
    pub(crate) query_interface:
        unsafe extern "system" fn(Unknown, *const GUID, *mut Unknown) -> HRESULT,
    pub(crate) add_ref: unsafe extern "system" fn(Unknown) -> u32,
    pub(crate) release: unsafe extern "system" fn(Unknown) -> u32,
    pub(crate) drag_enter:
        unsafe extern "system" fn(Unknown, Unknown, u32, POINTL, *mut u32) -> HRESULT,
    pub(crate) drag_over: unsafe extern "system" fn(Unknown, u32, POINTL, *mut u32) -> HRESULT,
    pub(crate) drag_leave: unsafe extern "system" fn(Unknown) -> HRESULT,
    pub(crate) drop: unsafe extern "system" fn(Unknown, Unknown, u32, POINTL, *mut u32) -> HRESULT,
}

/// The leading `IDataObject` slots the drop targets call; the rest are never read.
#[repr(C)]
pub(crate) struct DataObjectVtbl {
    pub(crate) query_interface: usize,
    pub(crate) add_ref: usize,
    pub(crate) release: usize,
    pub(crate) get_data:
        unsafe extern "system" fn(Unknown, *const FORMATETC, *mut STGMEDIUM) -> HRESULT,
    pub(crate) get_data_here: usize,
    pub(crate) query_get_data: unsafe extern "system" fn(Unknown, *const FORMATETC) -> HRESULT,
}

/// Whether `iid` is one a drop target answers to: `IUnknown` or `IDropTarget`.
pub(crate) fn is_drop_target_iid(iid: &GUID) -> bool {
    let known = |other: &GUID| {
        iid.data1 == other.data1
            && iid.data2 == other.data2
            && iid.data3 == other.data3
            && iid.data4 == other.data4
    };
    known(&IID_IUNKNOWN) || known(&IID_IDROPTARGET)
}

/// Initialises OLE on this thread, once: `RegisterDragDrop` needs it. It is never uninitialised,
/// since the UI thread lives as long as the process.
pub(crate) fn ensure_ole() {
    thread_local! {
        static INITIALIZED: Cell<bool> = const { Cell::new(false) };
    }
    INITIALIZED.with(|initialized| {
        if !initialized.replace(true) {
            unsafe { OleInitialize(std::ptr::null()) };
        }
    });
}

pub(crate) fn hdrop_format() -> FORMATETC {
    FORMATETC {
        cfFormat: CF_HDROP,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT,
        lindex: -1,
        tymed: TYMED_HGLOBAL as u32,
    }
}

pub(crate) fn offers_files(data: Unknown) -> bool {
    if data.is_null() {
        return false;
    }
    let data_vtbl = unsafe { &**(data as *const *const DataObjectVtbl) };
    unsafe { (data_vtbl.query_get_data)(data, &hdrop_format()) == S_OK }
}

pub(crate) fn dropped_files(data: Unknown) -> Vec<PathBuf> {
    let data_vtbl = unsafe { &**(data as *const *const DataObjectVtbl) };
    let mut medium = STGMEDIUM::default();
    if unsafe { (data_vtbl.get_data)(data, &hdrop_format(), &mut medium) } < 0 {
        return Vec::new();
    }
    let paths = if medium.tymed == TYMED_HGLOBAL as u32 {
        crate::platform::win32::dropped_paths(unsafe { medium.u.hGlobal })
    } else {
        Vec::new()
    };
    unsafe { ReleaseStgMedium(&mut medium) };
    paths
}
