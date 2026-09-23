//! File operations the note library needs that `std` does not offer safely: a rename that never
//! replaces its target, and deleting to the Recycle Bin.

use crate::Result;
use crate::platform::last_error;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use windows_sys::Win32::Storage::FileSystem::MoveFileExW;
use windows_sys::Win32::UI::Shell::{
    FO_DELETE, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, FOF_WANTNUKEWARNING,
    SHFILEOPSTRUCTW, SHFileOperationW,
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

/// Sends one file to the Recycle Bin without any shell UI, except a warning if the file cannot be
/// recycled (a network share, the bin disabled, over quota, ...) and would be deleted outright
/// instead: `FOF_NOCONFIRMATION` alone would auto-answer that "delete permanently?" prompt with
/// Yes, silently destroying the file rather than recycling it.
pub fn recycle(path: &Path) -> Result<()> {
    if !path.is_absolute() {
        return Err(crate::FastPadError::Invariant(
            "recycle requires an absolute path",
        ));
    }
    if !path.is_file() {
        return Err(crate::FastPadError::Invariant(
            "the file to delete does not exist",
        ));
    }
    // SHFileOperationW takes a list ending in two NULs.
    let mut from = wide(path);
    from.push(0);
    let mut operation: SHFILEOPSTRUCTW = unsafe { std::mem::zeroed() };
    operation.wFunc = FO_DELETE;
    operation.pFrom = from.as_ptr();
    operation.fFlags = (FOF_ALLOWUNDO
        | FOF_NOCONFIRMATION
        | FOF_SILENT
        | FOF_NOERRORUI
        | FOF_WANTNUKEWARNING) as _;
    let status = unsafe { SHFileOperationW(&mut operation) };
    if operation.fAnyOperationsAborted != 0 {
        return Err(crate::FastPadError::Invariant("recycle aborted"));
    }
    if status != 0 {
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
    fn recycle_rejects_a_relative_path_and_deletes_nothing() {
        // Break caught: a relative path handed to SHFileOperationW resolves against the process's
        // current directory, which is never what a caller building a path from a note's folder
        // intends, and could delete an unrelated same-named file elsewhere.
        let dir = scratch("recycle-relative");
        let path = dir.join("keep.md");
        std::fs::write(&path, "x").unwrap();
        assert!(recycle(std::path::Path::new("keep.md")).is_err());
        assert!(path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn recycle_rejects_a_directory() {
        // Break caught: recycle's contract is one file; SHFileOperationW would happily recycle a
        // whole directory tree if `is_file` were relaxed back to `exists`.
        let dir = scratch("recycle-directory");
        assert!(recycle(&dir).is_err());
        assert!(dir.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_default_notes_folder_is_under_documents() {
        let documents = crate::platform::paths::documents_dir().unwrap();
        assert!(documents.is_dir());
        assert_eq!(crate::platform::paths::default_notes_folder().unwrap(), documents.join("FastPad"));
    }
}
