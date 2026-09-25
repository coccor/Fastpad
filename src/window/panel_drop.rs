//! Files dragged from Explorer onto the sidebar panel (open editors spec §4.1, §4.3): the panel's
//! OLE drop target. The drag's files are read once, at DragEnter; `notebook_view` says what the
//! point under the pointer does with them, and a drop is posted, never handled inside Drop, so
//! Explorer never waits on a prompt (spec §6).
//!
//! `register` runs after first paint. The target is revoked before the panel is destroyed, which
//! releases it.

use crate::platform::ole_drop::{
    DropTargetVtbl, Unknown, dropped_files, ensure_ole, is_drop_target_iid, offers_files,
};
use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::path::PathBuf;
use windows_sys::Win32::Foundation::{E_NOINTERFACE, E_POINTER, HWND, POINT, POINTL, S_OK};
use windows_sys::Win32::Graphics::Gdi::ScreenToClient;
use windows_sys::Win32::System::Ole::{
    DROPEFFECT_COPY, DROPEFFECT_NONE, RegisterDragDrop, RevokeDragDrop,
};
use windows_sys::core::{GUID, HRESULT};

#[repr(C)]
struct PanelTarget {
    vtbl: &'static DropTargetVtbl,
    refs: Cell<u32>,
    /// The main window, whose notebook view the panel shows.
    main: HWND,
    panel: HWND,
    /// The files the drag in progress offers; empty for a drag that offers none.
    files: RefCell<Vec<PathBuf>>,
}

static PANEL_VTBL: DropTargetVtbl = DropTargetVtbl {
    query_interface,
    add_ref,
    release,
    drag_enter,
    drag_over,
    drag_leave,
    drop: drop_on,
};

/// Registers the panel's drop target. Calling it again on the same panel does nothing.
pub(crate) fn register(main: HWND, panel: HWND) -> crate::Result<()> {
    if !crate::editor::file_drop::registered_target(panel).is_null() {
        return Ok(());
    }
    ensure_ole();
    let target = Box::into_raw(Box::new(PanelTarget {
        vtbl: &PANEL_VTBL,
        refs: Cell::new(1),
        main,
        panel,
        files: RefCell::new(Vec::new()),
    }))
    .cast::<c_void>();
    let result = unsafe { RegisterDragDrop(panel, target) };
    // OLE holds its own reference when registration succeeded; this drops the creation reference.
    unsafe { release(target) };
    if result < 0 {
        return Err(crate::FastPadError::Win32(result as u32));
    }
    Ok(())
}

/// Revokes the panel's drop target, if it has one, which releases it. Called before the panel
/// is destroyed.
pub(crate) fn revoke(panel: HWND) {
    if !crate::editor::file_drop::registered_target(panel).is_null() {
        unsafe { RevokeDragDrop(panel) };
    }
}

fn this<'a>(object: Unknown) -> &'a PanelTarget {
    unsafe { &*(object as *const PanelTarget) }
}

/// Screen point `point` in the panel's client coordinates.
fn panel_point(panel: HWND, point: POINTL) -> (i32, i32) {
    let mut client = POINT {
        x: point.x,
        y: point.y,
    };
    unsafe { ScreenToClient(panel, &mut client) };
    (client.x, client.y)
}

/// Answers COPY when `accepted` and the source allows a copy, NONE otherwise.
fn answer(effect: *mut u32, accepted: bool) {
    if !effect.is_null() {
        unsafe {
            *effect = if accepted && *effect & DROPEFFECT_COPY != 0 {
                DROPEFFECT_COPY
            } else {
                DROPEFFECT_NONE
            };
        }
    }
}

unsafe extern "system" fn query_interface(
    object: Unknown,
    iid: *const GUID,
    out: *mut Unknown,
) -> HRESULT {
    if iid.is_null() || out.is_null() {
        return E_POINTER;
    }
    if is_drop_target_iid(unsafe { &*iid }) {
        unsafe {
            add_ref(object);
            *out = object;
        }
        S_OK
    } else {
        unsafe { *out = std::ptr::null_mut() };
        E_NOINTERFACE
    }
}

unsafe extern "system" fn add_ref(object: Unknown) -> u32 {
    let target = this(object);
    target.refs.set(target.refs.get() + 1);
    target.refs.get()
}

unsafe extern "system" fn release(object: Unknown) -> u32 {
    let target = this(object);
    let refs = target.refs.get() - 1;
    target.refs.set(refs);
    if refs == 0 {
        drop(unsafe { Box::from_raw(object.cast::<PanelTarget>()) });
    }
    refs
}

unsafe extern "system" fn drag_enter(
    object: Unknown,
    data: Unknown,
    keys: u32,
    point: POINTL,
    effect: *mut u32,
) -> HRESULT {
    let target = this(object);
    let files = if offers_files(data) {
        dropped_files(data)
    } else {
        Vec::new()
    };
    let offered = !files.is_empty();
    target.files.replace(files);
    if !offered {
        answer(effect, false);
        return S_OK;
    }
    unsafe { drag_over(object, keys, point, effect) }
}

unsafe extern "system" fn drag_over(
    object: Unknown,
    _keys: u32,
    point: POINTL,
    effect: *mut u32,
) -> HRESULT {
    let target = this(object);
    // A copy, so nothing of the target stays borrowed while the view runs.
    let files = target.files.borrow().clone();
    if files.is_empty() {
        answer(effect, false);
        return S_OK;
    }
    let (x, y) = panel_point(target.panel, point);
    let accepted = super::notebook_view::external_over(target.main, x, y, &files);
    answer(effect, accepted);
    S_OK
}

unsafe extern "system" fn drag_leave(object: Unknown) -> HRESULT {
    let target = this(object);
    target.files.take();
    super::notebook_view::external_leave(target.main);
    S_OK
}

unsafe extern "system" fn drop_on(
    object: Unknown,
    _data: Unknown,
    _keys: u32,
    point: POINTL,
    effect: *mut u32,
) -> HRESULT {
    let target = this(object);
    let files = target.files.take();
    let copy_allowed = !effect.is_null() && unsafe { *effect } & DROPEFFECT_COPY != 0;
    if files.is_empty() || !copy_allowed {
        super::notebook_view::external_leave(target.main);
        answer(effect, false);
        return S_OK;
    }
    let (x, y) = panel_point(target.panel, point);
    let accepted = super::notebook_view::external_drop(target.main, x, y, files);
    answer(effect, accepted);
    S_OK
}
