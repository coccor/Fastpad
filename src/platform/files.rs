//! File operations the note library needs that `std` does not offer safely: a rename that never
//! replaces its target, and deleting to the Recycle Bin.

use crate::Result;
use crate::platform::last_error;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use windows_sys::Win32::Storage::FileSystem::MoveFileExW;
use windows_sys::Win32::UI::Shell::{
    FO_DELETE, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, SHFILEOPSTRUCTW,
    SHFileOperationW,
};

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(std::iter::once(0)).collect()
}

/// `MoveFileExW` without `MOVEFILE_REPLACE_EXISTING`: fails if `new` already exists (a change of
/// letter case only is allowed, because it names the same file).
pub fn rename_no_replace(old: &Path, new: &Path) -> Result<()> {
    let (old_wide, new_wide) = (wide(old), wide(new));
    if unsafe { MoveFileExW(old_wide.as_ptr(), new_wide.as_ptr(), 0) } == 0 {
        return Err(last_error());
    }
    Ok(())
}

/// Sends one file to the Recycle Bin without any shell UI.
pub fn recycle(path: &Path) -> Result<()> {
    if !path.exists() {
        return Err(crate::FastPadError::Invariant("the file to delete does not exist"));
    }
    // SHFileOperationW takes a list ending in two NULs.
    let mut from = wide(path);
    from.push(0);
    let mut operation: SHFILEOPSTRUCTW = unsafe { std::mem::zeroed() };
    operation.wFunc = FO_DELETE;
    operation.pFrom = from.as_ptr();
    operation.fFlags = (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_SILENT | FOF_NOERRORUI) as _;
    let status = unsafe { SHFileOperationW(&mut operation) };
    if status != 0 || operation.fAnyOperationsAborted != 0 {
        return Err(crate::FastPadError::Win32(status as u32));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(label: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("fastpad-files-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn rename_never_replaces_an_existing_file_but_allows_a_case_change() {
        // Break caught: std::fs::rename's MOVEFILE_REPLACE_EXISTING silently destroying a note
        // that appeared under the new name after the clash check.
        let dir = scratch("rename");
        std::fs::write(dir.join("a.md"), "a").unwrap();
        std::fs::write(dir.join("b.md"), "b").unwrap();
        assert!(rename_no_replace(&dir.join("a.md"), &dir.join("b.md")).is_err());
        assert_eq!(std::fs::read_to_string(dir.join("b.md")).unwrap(), "b");
        rename_no_replace(&dir.join("a.md"), &dir.join("A.md")).unwrap();
        rename_no_replace(&dir.join("A.md"), &dir.join("c.md")).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("c.md")).unwrap(), "a");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn recycling_removes_the_file_from_its_folder() {
        let dir = scratch("recycle");
        let path = dir.join("gone.md");
        std::fs::write(&path, "x").unwrap();
        recycle(&path).unwrap();
        assert!(!path.exists());
        assert!(recycle(&dir.join("never-existed.md")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_default_notes_folder_is_under_documents() {
        let documents = crate::platform::paths::documents_dir().unwrap();
        assert!(documents.is_dir());
        assert_eq!(crate::platform::paths::default_notes_folder().unwrap(), documents.join("FastPad"));
    }
}
