//! File operations the note library needs that `std` does not offer safely: a rename that never
//! replaces its target, and deleting to the Recycle Bin.

use crate::Result;
use crate::platform::last_error;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::Storage::FileSystem::{
    COPY_FILE_FAIL_IF_EXISTS, CopyFileExW, MOVEFILE_COPY_ALLOWED, MOVEFILE_WRITE_THROUGH,
    MoveFileExW,
};
use windows_sys::Win32::UI::Shell::{
    FO_DELETE, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, FOF_WANTNUKEWARNING,
    SHFILEOPSTRUCTW, SHFileOperationW,
};

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
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

/// Moves `from` to `to`, never over an existing file. Across volumes it copies and then deletes
/// (`MOVEFILE_COPY_ALLOWED`). `MOVEFILE_WRITE_THROUGH` returns only once the copy is on disk, so
/// the source is never deleted before the copy exists.
pub fn move_file(from: &Path, to: &Path) -> Result<()> {
    let (from_wide, to_wide) = (wide(from), wide(to));
    let flags = MOVEFILE_COPY_ALLOWED | MOVEFILE_WRITE_THROUGH;
    if unsafe { MoveFileExW(from_wide.as_ptr(), to_wide.as_ptr(), flags) } == 0 {
        return Err(last_error());
    }
    Ok(())
}

/// Sends one file to the Recycle Bin without any shell UI, except a warning if the file cannot be
/// recycled (a network share, the bin disabled, over quota, ...) and would be deleted outright
/// instead: `FOF_NOCONFIRMATION` alone would auto-answer that "delete permanently?" prompt with
/// Yes, silently destroying the file rather than recycling it. `owner` owns that warning, so it is
/// modal to the window instead of floating free of it.
pub fn recycle(owner: HWND, path: &Path) -> Result<()> {
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
    shell_recycle(owner, path)
}

/// Sends one folder, with everything in it, to the Recycle Bin, the way `recycle` sends a file
/// (notebook folders spec §4.3). A relative path, or anything but a directory, is refused before
/// anything is sent to the shell.
pub fn recycle_folder(owner: HWND, path: &Path) -> Result<()> {
    if !path.is_absolute() {
        return Err(crate::FastPadError::Invariant(
            "recycle requires an absolute path",
        ));
    }
    if !path.is_dir() {
        return Err(crate::FastPadError::Invariant(
            "the folder to delete does not exist",
        ));
    }
    shell_recycle(owner, path)
}

/// `SHFileOperationW`'s delete to the Recycle Bin, shared by `recycle` and `recycle_folder`.
fn shell_recycle(owner: HWND, path: &Path) -> Result<()> {
    // SHFileOperationW takes a list ending in two NULs.
    let mut from = wide(path);
    from.push(0);
    let mut operation: SHFILEOPSTRUCTW = unsafe { std::mem::zeroed() };
    operation.hwnd = owner;
    operation.wFunc = FO_DELETE;
    operation.pFrom = from.as_ptr();
    operation.fFlags =
        (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_SILENT | FOF_NOERRORUI | FOF_WANTNUKEWARNING)
            as _;
    let status = unsafe { SHFileOperationW(&mut operation) };
    if operation.fAnyOperationsAborted != 0 {
        return Err(crate::FastPadError::Invariant("recycle aborted"));
    }
    if status != 0 {
        return Err(crate::FastPadError::Win32(status as u32));
    }
    Ok(())
}

/// `CopyFileExW` with `COPY_FILE_FAIL_IF_EXISTS`: fails if `to` already exists.
pub fn copy_file_no_replace(from: &Path, to: &Path) -> Result<()> {
    let (from_wide, to_wide) = (wide(from), wide(to));
    let copied = unsafe {
        CopyFileExW(
            from_wide.as_ptr(),
            to_wide.as_ptr(),
            None,
            std::ptr::null(),
            std::ptr::null_mut(),
            COPY_FILE_FAIL_IF_EXISTS,
        )
    };
    if copied == 0 {
        return Err(last_error());
    }
    Ok(())
}

/// What `copy_tree` did: the files copied, and the first error with the path it happened at.
#[derive(Debug)]
pub struct Copied {
    pub files: usize,
    pub error: Option<(std::path::PathBuf, crate::FastPadError)>,
}

/// Copies the file or folder `from` to `to`, which must not exist: a folder is created with
/// everything in it, never merged into one that is there. Stops at the first error, and after
/// the file in hand once `cancel` is set. An explicit stack, so a deep folder can't overflow.
pub fn copy_tree(from: &Path, to: &Path, cancel: &std::sync::atomic::AtomicBool) -> Copied {
    use std::sync::atomic::Ordering;
    let mut copied = Copied {
        files: 0,
        error: None,
    };
    let fail = |copied: &mut Copied, path: &Path, error: crate::FastPadError| {
        copied.error = Some((path.to_path_buf(), error));
    };
    if !from.is_dir() {
        match copy_file_no_replace(from, to) {
            Ok(()) => copied.files = 1,
            Err(error) => fail(&mut copied, from, error),
        }
        return copied;
    }
    let mut stack = vec![(from.to_path_buf(), to.to_path_buf())];
    while let Some((source, target)) = stack.pop() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        if let Err(error) = std::fs::create_dir(&target) {
            fail(&mut copied, &target, error.into());
            break;
        }
        let entries = match std::fs::read_dir(&source) {
            Ok(entries) => entries,
            Err(error) => {
                fail(&mut copied, &source, error.into());
                break;
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    fail(&mut copied, &source, error.into());
                    return copied;
                }
            };
            let (inner, outer) = (entry.path(), target.join(entry.file_name()));
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                stack.push((inner, outer));
            } else {
                if cancel.load(Ordering::Relaxed) {
                    return copied;
                }
                match copy_file_no_replace(&inner, &outer) {
                    Ok(()) => copied.files += 1,
                    Err(error) => {
                        fail(&mut copied, &inner, error);
                        return copied;
                    }
                }
            }
        }
    }
    copied
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(label: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("fastpad-files-{label}-{}", std::process::id()));
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
        recycle(std::ptr::null_mut(), &path).unwrap();
        assert!(!path.exists());
        assert!(recycle(std::ptr::null_mut(), &dir.join("never-existed.md")).is_err());
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
        assert!(recycle(std::ptr::null_mut(), std::path::Path::new("keep.md")).is_err());
        assert!(path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn recycle_rejects_a_directory() {
        // Break caught: recycle's contract is one file; SHFileOperationW would happily recycle a
        // whole directory tree if `is_file` were relaxed back to `exists`.
        let dir = scratch("recycle-directory");
        assert!(recycle(std::ptr::null_mut(), &dir).is_err());
        assert!(dir.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn recycling_a_folder_removes_it_with_everything_in_it_and_refuses_anything_else() {
        // Break caught: a folder delete refused because `recycle` takes only files, contents left
        // behind, or a file or relative path accepted as a folder.
        let dir = scratch("recycle-folder");
        let folder = dir.join("old");
        std::fs::create_dir_all(folder.join("inner")).unwrap();
        std::fs::write(folder.join(r"inner\a.md"), "a").unwrap();
        recycle_folder(std::ptr::null_mut(), &folder).unwrap();
        assert!(!folder.exists());
        assert!(dir.exists());
        let file = dir.join("file.md");
        std::fs::write(&file, "x").unwrap();
        assert!(recycle_folder(std::ptr::null_mut(), &file).is_err());
        assert!(file.exists());
        assert!(recycle_folder(std::ptr::null_mut(), std::path::Path::new("old")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn moving_a_file_never_replaces_the_target_and_lands_in_another_folder() {
        // Break caught: a move onto a same-named note in the destination destroying that note, or
        // a "move" that leaves the source behind.
        let dir = scratch("move");
        std::fs::create_dir_all(dir.join("other")).unwrap();
        std::fs::write(dir.join("a.md"), "a").unwrap();
        std::fs::write(dir.join("other").join("b.md"), "b").unwrap();
        assert!(move_file(&dir.join("a.md"), &dir.join("other").join("b.md")).is_err());
        assert_eq!(
            std::fs::read_to_string(dir.join("other").join("b.md")).unwrap(),
            "b"
        );
        assert!(dir.join("a.md").exists());
        move_file(&dir.join("a.md"), &dir.join("other").join("a.md")).unwrap();
        assert!(!dir.join("a.md").exists());
        assert_eq!(
            std::fs::read_to_string(dir.join("other").join("a.md")).unwrap(),
            "a"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn copying_a_file_never_replaces_and_a_folder_copies_whole() {
        // Break caught: a copy over an existing note, a nested folder copied flat or partly, or
        // non-note files left behind (open editors spec §4.5).
        let dir = scratch("copy");
        std::fs::write(dir.join("a.md"), "a").unwrap();
        std::fs::write(dir.join("taken.md"), "keep").unwrap();
        assert!(copy_file_no_replace(&dir.join("a.md"), &dir.join("taken.md")).is_err());
        assert_eq!(
            std::fs::read_to_string(dir.join("taken.md")).unwrap(),
            "keep"
        );
        copy_file_no_replace(&dir.join("a.md"), &dir.join("b.md")).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("b.md")).unwrap(), "a");

        let from = dir.join("pics");
        std::fs::create_dir_all(from.join(r"deep\er")).unwrap();
        std::fs::write(from.join("x.png"), [1u8, 2]).unwrap();
        std::fs::write(from.join(r"deep\er\y.md"), "y").unwrap();
        std::fs::create_dir_all(from.join("empty")).unwrap();
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let copied = copy_tree(&from, &dir.join("copy"), &cancel);
        assert!(copied.error.is_none());
        assert_eq!(copied.files, 2);
        assert_eq!(std::fs::read(dir.join(r"copy\x.png")).unwrap(), [1, 2]);
        assert_eq!(
            std::fs::read_to_string(dir.join(r"copy\deep\er\y.md")).unwrap(),
            "y"
        );
        assert!(dir.join(r"copy\empty").is_dir());
        let again = copy_tree(&from, &dir.join("copy"), &cancel);
        assert!(
            again.error.is_some() && again.files == 0,
            "an existing folder is never merged into"
        );
        let single = copy_tree(&dir.join("a.md"), &dir.join("c.md"), &cancel);
        assert_eq!((single.files, single.error.is_none()), (1, true));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_default_notes_folder_is_under_documents() {
        let documents = crate::platform::paths::documents_dir().unwrap();
        assert!(documents.is_dir());
        assert_eq!(
            crate::platform::paths::notes_folder_in(&documents),
            documents.join("FastPad")
        );
    }
}
