//! Walks a folder for notes. Each directory is listed with `FileIdBothDirectoryInfo` queries,
//! which return names, sizes, write times and file IDs in bulk without opening any file.

use crate::Result;
use crate::platform::{OwnedHandle, last_error, wide_null};
use std::os::windows::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use windows_sys::Win32::Foundation::{ERROR_NO_MORE_FILES, GetLastError};
use windows_sys::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, CreateFileW, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_HIDDEN,
    FILE_ATTRIBUTE_OFFLINE, FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS, FILE_ATTRIBUTE_REPARSE_POINT,
    FILE_ATTRIBUTE_SYSTEM, FILE_FLAG_BACKUP_SEMANTICS, FILE_ID_BOTH_DIR_INFO, FILE_LIST_DIRECTORY,
    FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FileIdBothDirectoryInfo,
    GetFileInformationByHandle, GetFileInformationByHandleEx, OPEN_EXISTING,
};

pub const NOTE_LIMIT: usize = 10_000;
const SKIPPED: [&str; 4] = ["node_modules", "target", "bin", "obj"];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScanEntry {
    /// Relative to the scanned folder.
    pub path: PathBuf,
    pub size: u64,
    /// Last write time in FILETIME ticks.
    pub mtime: u64,
    /// 0 when the file system has no stable IDs (FAT, some network shares).
    pub file_id: u64,
    pub online_only: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Scan {
    pub volume: u32,
    pub entries: Vec<ScanEntry>,
    pub truncated: bool,
}

pub fn skip_directory(name: &str) -> bool {
    name.starts_with('.')
        || SKIPPED
            .iter()
            .any(|skipped| skipped.eq_ignore_ascii_case(name))
}

fn open_directory(path: &Path) -> Result<OwnedHandle> {
    let wide = wide_null(&path.to_string_lossy());
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            FILE_LIST_DIRECTORY,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            std::ptr::null_mut(),
        )
    };
    // CreateFileW returns INVALID_HANDLE_VALUE on failure, which from_raw_owned rejects.
    unsafe { OwnedHandle::from_raw_owned(handle) }.map_err(|_| last_error())
}

pub fn scan(folder: &Path, limit: usize) -> Result<Scan> {
    let root = open_directory(folder)?;
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    if unsafe { GetFileInformationByHandle(root.as_raw(), &mut info) } == 0 {
        return Err(last_error());
    }
    let mut scan = Scan {
        volume: info.dwVolumeSerialNumber,
        ..Scan::default()
    };
    let mut pending = vec![(PathBuf::new(), Some(root))];
    // 64 KiB, 8-byte aligned as FILE_ID_BOTH_DIR_INFO requires.
    let mut buffer = vec![0_u64; 8 * 1024];
    while let Some((relative, handle)) = pending.pop() {
        let handle = match handle {
            Some(handle) => handle,
            // A subfolder that cannot be opened is skipped, not fatal.
            None => match open_directory(&folder.join(&relative)) {
                Ok(handle) => handle,
                Err(_) => continue,
            },
        };
        loop {
            let ok = unsafe {
                GetFileInformationByHandleEx(
                    handle.as_raw(),
                    FileIdBothDirectoryInfo,
                    buffer.as_mut_ptr().cast(),
                    (buffer.len() * 8) as u32,
                )
            };
            if ok == 0 {
                let error = unsafe { GetLastError() };
                if error != ERROR_NO_MORE_FILES && relative.as_os_str().is_empty() {
                    return Err(crate::FastPadError::Win32(error));
                }
                break;
            }
            let mut offset = 0_usize;
            loop {
                // Entries are packed back to back; NextEntryOffset is 8-byte aligned.
                let entry = unsafe {
                    &*(buffer
                        .as_ptr()
                        .cast::<u8>()
                        .add(offset)
                        .cast::<FILE_ID_BOTH_DIR_INFO>())
                };
                let name = unsafe {
                    std::slice::from_raw_parts(
                        std::ptr::addr_of!(entry.FileName).cast::<u16>(),
                        (entry.FileNameLength / 2) as usize,
                    )
                };
                let name = std::ffi::OsString::from_wide(name)
                    .to_string_lossy()
                    .into_owned();
                let attributes = entry.FileAttributes;
                let skipped_attributes = FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM;
                if name != "." && name != ".." && attributes & skipped_attributes == 0 {
                    let path = relative.join(&name);
                    if attributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
                        if attributes & FILE_ATTRIBUTE_REPARSE_POINT == 0 && !skip_directory(&name)
                        {
                            pending.push((path, None));
                        }
                    } else if Path::new(&name)
                        .extension()
                        .is_some_and(|ext| super::title::is_note_extension(&ext.to_string_lossy()))
                    {
                        if scan.entries.len() >= limit {
                            scan.truncated = true;
                            return Ok(scan);
                        }
                        scan.entries.push(ScanEntry {
                            path,
                            size: entry.EndOfFile as u64,
                            mtime: entry.LastWriteTime as u64,
                            file_id: entry.FileId as u64,
                            online_only: attributes
                                & (FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS | FILE_ATTRIBUTE_OFFLINE)
                                != 0,
                        });
                    }
                }
                if entry.NextEntryOffset == 0 {
                    break;
                }
                offset += entry.NextEntryOffset as usize;
            }
        }
    }
    Ok(scan)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(label: &str) -> Self {
            let root =
                std::env::temp_dir().join(format!("fastpad-scan-{label}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).unwrap();
            Self(root)
        }

        fn file(&self, relative: &str, text: &str) {
            let path = self.0.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn paths(scan: &Scan) -> Vec<String> {
        let mut paths: Vec<_> = scan
            .entries
            .iter()
            .map(|e| e.path.to_string_lossy().into_owned())
            .collect();
        paths.sort();
        paths
    }

    #[test]
    fn notes_are_found_in_subfolders_and_other_files_and_folders_are_skipped() {
        // Break caught: a repo folder listing node_modules, build output or images as notes.
        let scratch = Scratch::new("rules");
        scratch.file("a.md", "a");
        scratch.file(r"sub\deeper\b.TXT", "bb");
        scratch.file("picture.png", "x");
        scratch.file(r".git\c.md", "x");
        scratch.file(r"node_modules\d.md", "x");
        scratch.file(r"target\e.json", "x");
        scratch.file(r"Bin\f.md", "x");
        scratch.file(r"hidden\g.md", "x");
        let hidden = crate::platform::wide_null(&scratch.0.join("hidden").to_string_lossy());
        unsafe {
            windows_sys::Win32::Storage::FileSystem::SetFileAttributesW(
                hidden.as_ptr(),
                windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_HIDDEN,
            );
        }
        let scan = scan(&scratch.0, NOTE_LIMIT).unwrap();
        assert_eq!(paths(&scan), [r"a.md", r"sub\deeper\b.TXT"]);
        let b = scan
            .entries
            .iter()
            .find(|e| e.path.ends_with("b.TXT"))
            .unwrap();
        assert_eq!(b.size, 2);
        assert_ne!(b.file_id, 0, "NTFS reports file IDs");
        assert_ne!(b.mtime, 0);
        assert!(!scan.truncated);
    }

    #[test]
    fn the_scan_stops_at_the_limit() {
        let scratch = Scratch::new("limit");
        for index in 0..5 {
            scratch.file(&format!("{index}.md"), "x");
        }
        let scan = scan(&scratch.0, 3).unwrap();
        assert_eq!(scan.entries.len(), 3);
        assert!(scan.truncated);
    }

    #[test]
    fn reparse_points_are_not_followed() {
        // Break caught: a junction back to the folder itself making the scan loop until the limit
        // with the same notes over and over.
        let scratch = Scratch::new("junction");
        scratch.file("a.md", "a");
        let link = scratch.0.join("loop");
        let made = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(&scratch.0)
            .output()
            .unwrap();
        assert!(made.status.success(), "mklink /J failed: {made:?}");
        let scan = scan(&scratch.0, NOTE_LIMIT).unwrap();
        assert_eq!(paths(&scan), ["a.md"]);
    }

    #[test]
    fn a_missing_folder_is_an_error_and_skip_rules_ignore_case() {
        assert!(scan(Path::new(r"Z:\fastpad-does-not-exist\x"), NOTE_LIMIT).is_err());
        assert!(skip_directory("Node_Modules"));
        assert!(skip_directory(".obsidian"));
        assert!(!skip_directory("notes"));
    }
}
