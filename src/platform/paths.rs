use crate::{FastPadError, Result};
use std::path::PathBuf;

/// FastPad's per-user data directory: `%LocalAppData%\FastPad`. Nothing under this directory is
/// created by this helper — callers create whatever files/subdirectories they need (settings,
/// crash-recovery snapshots, IPC session state, ...) on demand.
///
/// Resolved via the `LOCALAPPDATA` environment variable rather than `SHGetKnownFolderPath`: Windows
/// sets it for every interactive user session, and — unlike a `KNOWNFOLDERID` lookup — a spawned
/// child process can trivially be pointed at an isolated scratch directory for tests by overriding
/// this one environment variable, without touching the real user profile.
#[cfg(windows)]
pub fn fastpad_data_dir() -> Result<PathBuf> {
    let value = std::env::var_os("LOCALAPPDATA").ok_or(FastPadError::Invariant(
        "the LOCALAPPDATA environment variable was not set",
    ))?;
    if value.is_empty() {
        return Err(FastPadError::Invariant(
            "the LOCALAPPDATA environment variable was empty",
        ));
    }
    Ok(PathBuf::from(value).join("FastPad"))
}

#[cfg(not(windows))]
pub fn fastpad_data_dir() -> Result<PathBuf> {
    Err(FastPadError::Invariant(
        "FastPad's per-user data directory is only defined on Windows",
    ))
}

/// The user's Documents folder, from the known-folder database.
#[cfg(windows)]
pub fn documents_dir() -> Result<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::System::Com::CoTaskMemFree;
    use windows_sys::Win32::UI::Shell::{FOLDERID_Documents, SHGetKnownFolderPath};
    let mut raw = std::ptr::null_mut();
    let status =
        unsafe { SHGetKnownFolderPath(&FOLDERID_Documents, 0, std::ptr::null_mut(), &mut raw) };
    if status < 0 || raw.is_null() {
        if !raw.is_null() {
            unsafe { CoTaskMemFree(raw.cast()) };
        }
        return Err(FastPadError::Win32(status as u32));
    }
    let mut len = 0;
    // The shell owns this terminated UTF-16 allocation until CoTaskMemFree.
    unsafe {
        while *raw.add(len) != 0 {
            len += 1;
        }
    }
    let path = PathBuf::from(std::ffi::OsString::from_wide(unsafe {
        std::slice::from_raw_parts(raw, len)
    }));
    unsafe { CoTaskMemFree(raw.cast()) };
    Ok(path)
}

/// Where Ctrl+N notes go before any folder was opened: `Documents\FastPad`.
#[cfg(windows)]
pub fn default_notes_folder() -> Result<PathBuf> {
    Ok(documents_dir()?.join("FastPad"))
}

#[cfg(test)]
mod tests {
    use super::fastpad_data_dir;

    #[cfg(windows)]
    #[test]
    fn resolves_a_fastpad_subdirectory_under_local_app_data() {
        // Break caught: resolving to LOCALAPPDATA itself (or some other path) instead of FastPad's
        // own subdirectory would let settings/recovery/IPC state collide with unrelated files.
        let local_app_data = std::env::var("LOCALAPPDATA").expect(
            "LOCALAPPDATA is set for every interactive Windows session this test suite runs under",
        );
        let resolved = fastpad_data_dir().unwrap();
        assert_eq!(
            resolved,
            std::path::Path::new(&local_app_data).join("FastPad")
        );
    }
}
