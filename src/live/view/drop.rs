//! Text dropped on the Live view (phase 4 plan, Decision 6): an OLE drop target that takes
//! `CF_UNICODETEXT`, shows a drop caret where the text would land, and inserts it there as one
//! undo step. The window's file drops are the group's: `library_host::wrap_live_drop_target`
//! wraps this target, as it wraps Scintilla's, so a drag offering files never reaches it.

use super::*;
use crate::platform::ole_drop::{
    DropTargetVtbl, Unknown, dropped_text, ensure_ole, is_drop_target_iid, offers_text,
};
use std::cell::Cell;
use std::ffi::c_void;
use windows_sys::Win32::Foundation::{E_NOINTERFACE, E_POINTER, POINT, POINTL, S_OK};
use windows_sys::Win32::Graphics::Gdi::ScreenToClient;
use windows_sys::Win32::System::Ole::{
    DROPEFFECT_COPY, DROPEFFECT_MOVE, DROPEFFECT_NONE, RegisterDragDrop, RevokeDragDrop,
};
/// `MK_SHIFT`: Shift held during the drag.
const MK_SHIFT: u32 = 0x0004;
use windows_sys::core::{GUID, HRESULT};

#[repr(C)]
struct TextDropTarget {
    vtbl: &'static DropTargetVtbl,
    refs: Cell<u32>,
    hwnd: HWND,
    /// Whether the drag in progress offers text.
    text: Cell<bool>,
}

static TEXT_DROP_VTBL: DropTargetVtbl = DropTargetVtbl {
    query_interface,
    add_ref,
    release,
    drag_enter,
    drag_over,
    drag_leave,
    drop: drop_on,
};

/// Registers the text drop target on the Live window `hwnd`.
pub(super) fn register(hwnd: HWND) {
    ensure_ole();
    let target = Box::into_raw(Box::new(TextDropTarget {
        vtbl: &TEXT_DROP_VTBL,
        refs: Cell::new(1),
        hwnd,
        text: Cell::new(false),
    }))
    .cast::<c_void>();
    unsafe {
        RegisterDragDrop(hwnd, target);
        // OLE holds its own reference when registration succeeded.
        release(target);
    }
}

/// Revokes whatever target is registered on `hwnd` (this one, or the file wrapper around it),
/// which releases it.
pub(super) fn revoke(hwnd: HWND) {
    unsafe { RevokeDragDrop(hwnd) };
}

fn this<'a>(object: Unknown) -> &'a TextDropTarget {
    unsafe { &*(object as *const TextDropTarget) }
}

/// Copy, so another app's text is never taken from it unasked; Shift moves when the source
/// allows it.
fn choose_effect(keys: u32, effect: *mut u32) {
    if effect.is_null() {
        return;
    }
    let allowed = unsafe { *effect };
    let wanted = if keys & MK_SHIFT != 0 && allowed & DROPEFFECT_MOVE != 0 {
        DROPEFFECT_MOVE
    } else if allowed & DROPEFFECT_COPY != 0 {
        DROPEFFECT_COPY
    } else {
        allowed & DROPEFFECT_MOVE
    };
    unsafe { *effect = wanted };
}

fn refuse(effect: *mut u32) {
    if !effect.is_null() {
        unsafe { *effect = DROPEFFECT_NONE };
    }
}

/// The document position under screen point `point`, which becomes the drop caret.
fn hover(hwnd: HWND, point: POINTL) -> Option<usize> {
    let mut client = POINT {
        x: point.x,
        y: point.y,
    };
    unsafe { ScreenToClient(hwnd, &mut client) };
    let position = with_state(hwnd, |state| {
        let position = super::mouse::position_at(state, (client.x, client.y));
        state.drop_at = Some(position);
        position
    });
    invalidate(hwnd);
    position
}

fn clear(hwnd: HWND) {
    with_state(hwnd, |state| state.drop_at = None);
    invalidate(hwnd);
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
        drop(unsafe { Box::from_raw(object.cast::<TextDropTarget>()) });
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
    target.text.set(offers_text(data));
    if target.text.get() {
        choose_effect(keys, effect);
        hover(target.hwnd, point);
    } else {
        refuse(effect);
    }
    S_OK
}

unsafe extern "system" fn drag_over(
    object: Unknown,
    keys: u32,
    point: POINTL,
    effect: *mut u32,
) -> HRESULT {
    let target = this(object);
    if target.text.get() {
        choose_effect(keys, effect);
        hover(target.hwnd, point);
    } else {
        refuse(effect);
    }
    S_OK
}

unsafe extern "system" fn drag_leave(object: Unknown) -> HRESULT {
    let target = this(object);
    if target.text.replace(false) {
        clear(target.hwnd);
    }
    S_OK
}

unsafe extern "system" fn drop_on(
    object: Unknown,
    data: Unknown,
    keys: u32,
    point: POINTL,
    effect: *mut u32,
) -> HRESULT {
    let target = this(object);
    if !target.text.replace(false) {
        refuse(effect);
        return S_OK;
    }
    choose_effect(keys, effect);
    let at = hover(target.hwnd, point);
    clear(target.hwnd);
    match (at, dropped_text(data)) {
        (Some(at), Some(text)) if !text.is_empty() => insert(target.hwnd, at, &text),
        _ => refuse(effect),
    }
    S_OK
}

/// Inserts `text` at `at` as one undo step, in the document's line endings, and selects it.
fn insert(hwnd: HWND, at: usize, text: &str) {
    let Some(editor) = with_state(hwnd, |state| state.editor.clone()) else {
        return;
    };
    let eol = editor.eol().unwrap_or("\r\n");
    let text = text
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\n', eol);
    editor.begin_undo_action();
    let inserted = editor.replace_target(at..at, &text);
    editor.end_undo_action();
    if let Ok(range) = inserted {
        let _ = editor.set_selection(range);
    }
    post_changed(hwnd, false);
}

/// Runs a text drop of `text` at client point `client` through the target registered on
/// `hwnd`, as OLE does; returns the effect the drop answered.
#[cfg(test)]
pub(crate) fn test_drop_text(hwnd: HWND, text: &str, client: (i32, i32)) -> u32 {
    use crate::platform::ole_drop::DataObjectVtbl;
    use windows_sys::Win32::Foundation::DV_E_FORMATETC;
    use windows_sys::Win32::Graphics::Gdi::ClientToScreen;
    use windows_sys::Win32::System::Com::{FORMATETC, STGMEDIUM, TYMED_HGLOBAL};
    use windows_sys::Win32::System::Memory::{
        GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock,
    };

    #[repr(C)]
    struct FakeText {
        vtbl: &'static DataObjectVtbl,
        text: Vec<u16>,
    }
    static FAKE_VTBL: DataObjectVtbl = DataObjectVtbl {
        query_interface: 0,
        add_ref: 0,
        release: 0,
        get_data,
        get_data_here: 0,
        query_get_data,
    };
    const CF_UNICODETEXT: u16 = 13;
    unsafe extern "system" fn query_get_data(_: Unknown, format: *const FORMATETC) -> HRESULT {
        if unsafe { (*format).cfFormat } == CF_UNICODETEXT {
            S_OK
        } else {
            DV_E_FORMATETC
        }
    }
    unsafe extern "system" fn get_data(
        object: Unknown,
        format: *const FORMATETC,
        medium: *mut STGMEDIUM,
    ) -> HRESULT {
        if unsafe { (*format).cfFormat } != CF_UNICODETEXT {
            return DV_E_FORMATETC;
        }
        let data = unsafe { &*(object as *const FakeText) };
        unsafe {
            let global = GlobalAlloc(GMEM_MOVEABLE, data.text.len() * 2);
            let target = GlobalLock(global).cast::<u16>();
            std::ptr::copy_nonoverlapping(data.text.as_ptr(), target, data.text.len());
            GlobalUnlock(global);
            *medium = STGMEDIUM::default();
            (*medium).tymed = TYMED_HGLOBAL as u32;
            (*medium).u.hGlobal = global;
        }
        S_OK
    }

    let mut data = FakeText {
        vtbl: &FAKE_VTBL,
        text: text.encode_utf16().chain([0]).collect(),
    };
    let data = (&mut data as *mut FakeText).cast::<c_void>();
    let target = crate::editor::file_drop::registered_target(hwnd);
    assert!(!target.is_null(), "no drop target is registered");
    let vtbl = unsafe { &**(target as *const *const DropTargetVtbl) };
    let mut screen = POINT {
        x: client.0,
        y: client.1,
    };
    unsafe { ClientToScreen(hwnd, &mut screen) };
    let point = POINTL {
        x: screen.x,
        y: screen.y,
    };
    let mut effects = [DROPEFFECT_COPY | DROPEFFECT_MOVE; 3];
    unsafe {
        let _ = (vtbl.drag_enter)(target, data, 0, point, &mut effects[0]);
        let _ = (vtbl.drag_over)(target, 0, point, &mut effects[1]);
        let _ = (vtbl.drop)(target, data, 0, point, &mut effects[2]);
    }
    effects[2]
}
