use crate::{FastPadError, Result};
use windows_sys::Win32::Foundation::{HANDLE, HMODULE, INVALID_HANDLE_VALUE};

#[cfg(windows)]
use windows_sys::Win32::Foundation::{CloseHandle, FreeLibrary, GetLastError};

pub fn wide_null(value: &str) -> Vec<u16> {
    let mut units: Vec<u16> = value.encode_utf16().collect();
    while units.last().copied() == Some(0) {
        units.pop();
    }
    units.push(0);
    units
}

pub fn last_error() -> FastPadError {
    FastPadError::Win32(raw_last_error())
}

pub(crate) fn require_module(raw: HMODULE) -> Result<HMODULE> {
    if module_is_valid(raw) {
        Ok(raw)
    } else {
        Err(last_error())
    }
}

pub(crate) fn require_handle(raw: HANDLE) -> Result<HANDLE> {
    if handle_is_valid(raw) {
        Ok(raw)
    } else {
        Err(last_error())
    }
}

pub(crate) fn module_is_valid(raw: HMODULE) -> bool {
    !raw.is_null()
}

pub(crate) fn handle_is_valid(raw: HANDLE) -> bool {
    !raw.is_null() && raw != INVALID_HANDLE_VALUE
}

#[cfg(windows)]
pub(crate) fn free_library(module: HMODULE) {
    unsafe {
        FreeLibrary(module);
    }
}

#[cfg(not(windows))]
pub(crate) fn free_library(_module: HMODULE) {}

#[cfg(windows)]
pub(crate) fn close_handle(handle: HANDLE) {
    unsafe {
        CloseHandle(handle);
    }
}

#[cfg(not(windows))]
pub(crate) fn close_handle(_handle: HANDLE) {}

#[cfg(windows)]
fn raw_last_error() -> u32 {
    unsafe { GetLastError() }
}

#[cfg(not(windows))]
fn raw_last_error() -> u32 {
    0
}

/// The paths in a shell `HDROP` (from `WM_DROPFILES` or an OLE `CF_HDROP`), skipping empty
/// entries. Does not free the handle.
#[cfg(windows)]
pub(crate) fn dropped_paths(drop: windows_sys::Win32::UI::Shell::HDROP) -> Vec<std::path::PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::UI::Shell::DragQueryFileW;
    let count = unsafe { DragQueryFileW(drop, u32::MAX, std::ptr::null_mut(), 0) };
    let mut paths = Vec::new();
    for index in 0..count {
        let length = unsafe { DragQueryFileW(drop, index, std::ptr::null_mut(), 0) } as usize;
        if length == 0 {
            continue;
        }
        let mut buffer = vec![0_u16; length + 1];
        let copied =
            unsafe { DragQueryFileW(drop, index, buffer.as_mut_ptr(), buffer.len() as u32) }
                as usize;
        if copied == 0 {
            continue;
        }
        buffer.truncate(copied.min(length));
        paths.push(std::path::PathBuf::from(std::ffi::OsString::from_wide(
            &buffer,
        )));
    }
    paths
}

/// A movable `HGLOBAL` holding a wide `DROPFILES` list, as Explorer builds for a file drop. The
/// caller frees it (`GlobalFree`, `DragFinish` or `ReleaseStgMedium`).
#[cfg(all(test, windows))]
pub(crate) fn test_hdrop(paths: &[&std::path::Path]) -> windows_sys::Win32::Foundation::HGLOBAL {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::System::Memory::{
        GMEM_MOVEABLE, GMEM_ZEROINIT, GlobalAlloc, GlobalLock, GlobalUnlock,
    };
    use windows_sys::Win32::UI::Shell::DROPFILES;
    let mut list = Vec::<u16>::new();
    for path in paths {
        list.extend(path.as_os_str().encode_wide());
        list.push(0);
    }
    list.push(0);
    let header = std::mem::size_of::<DROPFILES>();
    let global = unsafe { GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, header + list.len() * 2) };
    assert!(!global.is_null());
    unsafe {
        let base = GlobalLock(global).cast::<u8>();
        let files = DROPFILES {
            pFiles: header as u32,
            fWide: 1,
            ..Default::default()
        };
        std::ptr::write_unaligned(base.cast::<DROPFILES>(), files);
        std::ptr::copy_nonoverlapping(list.as_ptr().cast::<u8>(), base.add(header), list.len() * 2);
        GlobalUnlock(global);
    }
    global
}

#[cfg(test)]
mod tests {
    use super::wide_null;

    #[test]
    fn dropped_paths_lists_every_file_in_a_drop() {
        // Break caught: a truncated or ANSI read of Explorer's DROPFILES list.
        let a = std::path::Path::new(r"C:\notes\zăpadă.md");
        let b = std::path::Path::new(r"D:\Notes");
        let global = super::test_hdrop(&[a, b]);
        let paths = super::dropped_paths(global);
        unsafe { windows_sys::Win32::Foundation::GlobalFree(global) };
        assert_eq!(paths, vec![a.to_path_buf(), b.to_path_buf()]);
    }

    #[test]
    fn wide_null_appends_exactly_one_terminator() {
        // Break caught: forgetting the trailing NUL or appending more than one terminator.
        assert_eq!(wide_null("FastPad"), vec![70, 97, 115, 116, 80, 97, 100, 0]);
    }
}
