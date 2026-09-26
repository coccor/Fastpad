//! Files dropped on the editor. Scintilla registers its own OLE drop target on its window, and that
//! target accepts only text, so a file dragged from Explorer onto the editor is refused and never
//! becomes the parent's `WM_DROPFILES`. `accept_file_drops` wraps Scintilla's target: a drag that
//! offers `CF_HDROP` is taken here and its paths go to a callback, and every other drag (text
//! dragged within or into the editor) goes to Scintilla's own target unchanged.
//!
//! Scintilla revokes whatever target is registered when its window is destroyed, which releases the
//! wrapper, and the wrapper releases Scintilla's target and the callback.

use crate::platform::ole_drop::{
    DropTargetVtbl, Unknown, dropped_files, is_drop_target_iid, offers_files,
};
use crate::{FastPadError, Result};
use std::cell::Cell;
use std::ffi::c_void;
use std::path::PathBuf;
use windows_sys::Win32::Foundation::{E_NOINTERFACE, E_POINTER, HWND, POINTL, S_OK};
use windows_sys::Win32::System::Ole::{
    DROPEFFECT_COPY, DROPEFFECT_NONE, RegisterDragDrop, RevokeDragDrop,
};
use windows_sys::Win32::UI::WindowsAndMessaging::GetPropW;
use windows_sys::core::{GUID, HRESULT};

/// The window property where OLE keeps the `IDropTarget` that `RegisterDragDrop` registered.
const DROP_TARGET_PROPERTY: &str = "OleDropTargetInterface";

#[repr(C)]
struct FileDropTarget {
    vtbl: &'static DropTargetVtbl,
    refs: Cell<u32>,
    /// Scintilla's own target, held with a reference of ours.
    inner: Unknown,
    /// Whether the drag in progress offers files and so is ours rather than Scintilla's.
    files: Cell<bool>,
    on_files: Box<dyn Fn(Vec<PathBuf>)>,
}

static FILE_DROP_VTBL: DropTargetVtbl = DropTargetVtbl {
    query_interface,
    add_ref,
    release,
    drag_enter,
    drag_over,
    drag_leave,
    drop: drop_on,
};

/// Replaces the drop target Scintilla registered on `scintilla` with one that sends dropped files
/// to `on_files` and everything else to Scintilla. Calling it again on the same window does nothing.
pub(crate) fn accept_file_drops(
    scintilla: HWND,
    on_files: impl Fn(Vec<PathBuf>) + 'static,
) -> Result<()> {
    let inner = registered_target(scintilla);
    if inner.is_null() {
        return Err(FastPadError::Invariant(
            "the editor window has no registered drop target",
        ));
    }
    if is_file_drop_target(inner) {
        return Ok(());
    }
    unsafe { (vtbl(inner).add_ref)(inner) };
    let target = Box::into_raw(Box::new(FileDropTarget {
        vtbl: &FILE_DROP_VTBL,
        refs: Cell::new(1),
        inner,
        files: Cell::new(false),
        on_files: Box::new(on_files),
    }))
    .cast::<c_void>();
    let result = unsafe {
        RevokeDragDrop(scintilla);
        RegisterDragDrop(scintilla, target)
    };
    if result < 0 {
        // Put Scintilla's own target back so text drag-and-drop keeps working.
        unsafe { RegisterDragDrop(scintilla, inner) };
    }
    // OLE holds its own reference when registration succeeded; this drops the creation reference.
    unsafe { release(target) };
    if result < 0 {
        return Err(FastPadError::Win32(result as u32));
    }
    Ok(())
}

/// The `IDropTarget` registered on `hwnd`, or null.
pub(crate) fn registered_target(hwnd: HWND) -> *mut c_void {
    let name = crate::platform::wide_null(DROP_TARGET_PROPERTY);
    unsafe { GetPropW(hwnd, name.as_ptr()) }
}

pub(crate) fn is_file_drop_target(target: *mut c_void) -> bool {
    !target.is_null() && std::ptr::eq(vtbl(target), &FILE_DROP_VTBL)
}

fn vtbl<'a>(object: Unknown) -> &'a DropTargetVtbl {
    unsafe { &**(object as *const *const DropTargetVtbl) }
}

fn this<'a>(object: Unknown) -> &'a FileDropTarget {
    unsafe { &*(object as *const FileDropTarget) }
}

/// Copy is the only effect a file drop offers, and only when the source allows it.
fn copy_effect(effect: *mut u32) {
    if !effect.is_null() {
        unsafe {
            *effect = if *effect & DROPEFFECT_COPY != 0 {
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
        let target = unsafe { Box::from_raw(object.cast::<FileDropTarget>()) };
        unsafe { (vtbl(target.inner).release)(target.inner) };
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
    target.files.set(offers_files(data));
    if target.files.get() {
        copy_effect(effect);
        return S_OK;
    }
    unsafe { (vtbl(target.inner).drag_enter)(target.inner, data, keys, point, effect) }
}

unsafe extern "system" fn drag_over(
    object: Unknown,
    keys: u32,
    point: POINTL,
    effect: *mut u32,
) -> HRESULT {
    let target = this(object);
    if target.files.get() {
        copy_effect(effect);
        return S_OK;
    }
    unsafe { (vtbl(target.inner).drag_over)(target.inner, keys, point, effect) }
}

unsafe extern "system" fn drag_leave(object: Unknown) -> HRESULT {
    let target = this(object);
    if target.files.replace(false) {
        return S_OK;
    }
    unsafe { (vtbl(target.inner).drag_leave)(target.inner) }
}

unsafe extern "system" fn drop_on(
    object: Unknown,
    data: Unknown,
    keys: u32,
    point: POINTL,
    effect: *mut u32,
) -> HRESULT {
    let target = this(object);
    if !target.files.replace(false) {
        return unsafe { (vtbl(target.inner).drop)(target.inner, data, keys, point, effect) };
    }
    copy_effect(effect);
    let paths = dropped_files(data);
    if !paths.is_empty() {
        (target.on_files)(paths);
    }
    S_OK
}

/// A minimal `IDataObject` for tests: it offers `CF_HDROP` for `paths`, or nothing when `paths`
/// is empty.
#[cfg(test)]
pub(crate) mod test_support {
    use super::vtbl;
    use crate::platform::ole_drop::{DataObjectVtbl, Unknown};
    use std::path::{Path, PathBuf};
    use windows_sys::Win32::Foundation::{DV_E_FORMATETC, POINTL, S_OK};
    use windows_sys::Win32::System::Com::{FORMATETC, STGMEDIUM, TYMED_HGLOBAL};
    use windows_sys::Win32::System::Ole::CF_HDROP;
    use windows_sys::core::HRESULT;

    #[repr(C)]
    struct FakeData {
        vtbl: &'static DataObjectVtbl,
        paths: Vec<PathBuf>,
    }

    static FAKE_VTBL: DataObjectVtbl = DataObjectVtbl {
        query_interface: 0,
        add_ref: 0,
        release: 0,
        get_data,
        get_data_here: 0,
        query_get_data,
    };

    fn offers(object: Unknown, format: *const FORMATETC) -> bool {
        let data = unsafe { &*(object as *const FakeData) };
        let format = unsafe { &*format };
        !data.paths.is_empty() && format.cfFormat == CF_HDROP
    }

    unsafe extern "system" fn query_get_data(object: Unknown, format: *const FORMATETC) -> HRESULT {
        if offers(object, format) {
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
        if !offers(object, format) {
            return DV_E_FORMATETC;
        }
        let data = unsafe { &*(object as *const FakeData) };
        let paths = data.paths.iter().map(PathBuf::as_path).collect::<Vec<_>>();
        let global = crate::platform::win32::test_hdrop(&paths);
        unsafe {
            *medium = STGMEDIUM::default();
            (*medium).tymed = TYMED_HGLOBAL as u32;
            (*medium).u.hGlobal = global;
        }
        S_OK
    }

    /// Runs DragEnter, DragOver and Drop on the target registered on `hwnd`, as OLE does for a
    /// drop, and returns the effect each step answered.
    pub(crate) fn drag_and_drop(
        hwnd: windows_sys::Win32::Foundation::HWND,
        paths: &[&Path],
    ) -> [u32; 3] {
        drag_and_drop_at(hwnd, paths, POINTL { x: 1, y: 1 })
    }

    /// `drag_and_drop` with the pointer at screen point `point`. A refused DragOver ends in
    /// DragLeave, as OLE's does when the button goes up there.
    pub(crate) fn drag_and_drop_at(
        hwnd: windows_sys::Win32::Foundation::HWND,
        paths: &[&Path],
        point: POINTL,
    ) -> [u32; 3] {
        let target = super::registered_target(hwnd);
        assert!(!target.is_null(), "no drop target is registered");
        let mut data = FakeData {
            vtbl: &FAKE_VTBL,
            paths: paths.iter().map(|path| path.to_path_buf()).collect(),
        };
        let data = (&mut data as *mut FakeData).cast::<std::ffi::c_void>();
        let target_vtbl = vtbl(target);
        let mut effects = [super::DROPEFFECT_COPY; 3];
        unsafe {
            let _ = (target_vtbl.drag_enter)(target, data, 0, point, &mut effects[0]);
            let _ = (target_vtbl.drag_over)(target, 0, point, &mut effects[1]);
            if effects[1] == super::DROPEFFECT_NONE {
                let _ = (target_vtbl.drag_leave)(target);
                effects[2] = super::DROPEFFECT_NONE;
            } else {
                let _ = (target_vtbl.drop)(target, data, 0, point, &mut effects[2]);
            }
        }
        effects
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::drag_and_drop;
    use super::{accept_file_drops, is_file_drop_target, registered_target};
    use std::cell::RefCell;
    use std::path::{Path, PathBuf};
    use std::rc::Rc;
    use windows_sys::Win32::System::Ole::{DROPEFFECT_COPY, DROPEFFECT_NONE};

    fn load_native_scintilla() -> crate::platform::OwnedModule {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("native/out/x64/Scintilla.dll");
        let path = crate::platform::wide_null(path.to_str().unwrap());
        let module = unsafe {
            windows_sys::Win32::System::LibraryLoader::LoadLibraryExW(
                path.as_ptr(),
                std::ptr::null_mut(),
                windows_sys::Win32::System::LibraryLoader::LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR
                    | windows_sys::Win32::System::LibraryLoader::LOAD_LIBRARY_SEARCH_SYSTEM32,
            )
        };
        unsafe { crate::platform::OwnedModule::from_raw_owned(module) }.unwrap()
    }

    fn message_only_parent() -> windows_sys::Win32::Foundation::HWND {
        use windows_sys::Win32::UI::WindowsAndMessaging::{CreateWindowExW, HWND_MESSAGE};
        let class = crate::platform::wide_null("STATIC");
        let hwnd = unsafe {
            CreateWindowExW(
                0,
                class.as_ptr(),
                std::ptr::null(),
                0,
                0,
                0,
                0,
                0,
                HWND_MESSAGE,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        };
        assert!(!hwnd.is_null());
        hwnd
    }

    #[test]
    fn scintilla_registers_a_text_only_drop_target_that_refuses_files() {
        // Pins why the wrapper exists: Scintilla's own OLE target wins over the parent's
        // WM_DROPFILES and answers DROPEFFECT_NONE for Explorer's CF_HDROP.
        let _scintilla = load_native_scintilla();
        let parent = message_only_parent();
        let editor = crate::editor::Editor::create(parent).unwrap();
        assert!(!registered_target(editor.hwnd()).is_null());
        assert!(!is_file_drop_target(registered_target(editor.hwnd())));
        let effects = drag_and_drop(editor.hwnd(), &[Path::new(r"C:\notes\a.md")]);
        assert_eq!(effects[0], DROPEFFECT_NONE);
        drop(editor);
        unsafe { windows_sys::Win32::UI::WindowsAndMessaging::DestroyWindow(parent) };
    }

    #[test]
    fn the_wrapper_takes_file_drops_passes_other_drags_to_scintilla_and_is_released_with_the_editor()
     {
        // Break caught: files dropped on the editor refused, text drags no longer reaching
        // Scintilla, or the wrapper (and what its callback holds) leaking past the editor.
        let _scintilla = load_native_scintilla();
        let parent = message_only_parent();
        let editor = crate::editor::Editor::create(parent).unwrap();
        let received = Rc::new(RefCell::new(Vec::<PathBuf>::new()));
        let sink = Rc::clone(&received);
        accept_file_drops(editor.hwnd(), move |paths| sink.borrow_mut().extend(paths)).unwrap();
        accept_file_drops(editor.hwnd(), |_| panic!("wrapped twice")).unwrap();
        assert!(is_file_drop_target(registered_target(editor.hwnd())));

        let a = Path::new(r"C:\notes\a.md");
        let folder = Path::new(r"D:\Notes");
        assert_eq!(
            drag_and_drop(editor.hwnd(), &[a, folder]),
            [DROPEFFECT_COPY; 3]
        );
        assert_eq!(
            *received.borrow(),
            vec![a.to_path_buf(), folder.to_path_buf()]
        );

        // No CF_HDROP and no text: Scintilla's own target answers, and refuses it.
        assert_eq!(drag_and_drop(editor.hwnd(), &[])[0], DROPEFFECT_NONE);
        assert_eq!(received.borrow().len(), 2);

        assert_eq!(Rc::strong_count(&received), 2);
        drop(editor);
        assert_eq!(Rc::strong_count(&received), 1, "the wrapper was released");
        unsafe { windows_sys::Win32::UI::WindowsAndMessaging::DestroyWindow(parent) };
    }
}
