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
///
/// In-process tests (`cfg(test)`) get a per-process scratch directory instead; see
/// `test_profile_dir`.
#[cfg(all(windows, not(test)))]
pub fn fastpad_data_dir() -> Result<PathBuf> {
    data_dir_under(std::env::var_os("LOCALAPPDATA"))
}

#[cfg(all(windows, test))]
pub fn fastpad_data_dir() -> Result<PathBuf> {
    test_profile_dir("FastPad")
}

/// `<local_app_data>\FastPad`, from the value of `LOCALAPPDATA`.
#[cfg(windows)]
fn data_dir_under(local_app_data: Option<std::ffi::OsString>) -> Result<PathBuf> {
    let value = local_app_data.ok_or(FastPadError::Invariant(
        "the LOCALAPPDATA environment variable was not set",
    ))?;
    if value.is_empty() {
        return Err(FastPadError::Invariant(
            "the LOCALAPPDATA environment variable was empty",
        ));
    }
    Ok(PathBuf::from(value).join("FastPad"))
}

/// `%TEMP%\fastpad-libtest-<pid>\<name>`, created on demand: in-process tests run with the
/// user's real environment, so nothing they save may resolve into the real profile or Documents.
#[cfg(all(windows, test))]
fn test_profile_dir(name: &str) -> Result<PathBuf> {
    let dir = std::env::temp_dir()
        .join(format!("fastpad-libtest-{}", std::process::id()))
        .join(name);
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
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

/// Where Ctrl+N notes go before any folder was opened: `Documents\FastPad`. In-process tests
/// (`cfg(test)`) get a per-process scratch directory instead.
#[cfg(all(windows, not(test)))]
pub fn default_notes_folder() -> Result<PathBuf> {
    Ok(notes_folder_in(&documents_dir()?))
}

#[cfg(all(windows, test))]
pub fn default_notes_folder() -> Result<PathBuf> {
    test_profile_dir("Notes")
}

/// The default notes folder inside a Documents folder.
#[cfg(windows)]
pub fn notes_folder_in(documents: &std::path::Path) -> PathBuf {
    documents.join("FastPad")
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    #[test]
    fn resolves_a_fastpad_subdirectory_under_local_app_data() {
        // Break caught: resolving to LOCALAPPDATA itself (or some other path) instead of FastPad's
        // own subdirectory would let settings/recovery/IPC state collide with unrelated files.
        let local_app_data = std::env::var_os("LOCALAPPDATA").expect(
            "LOCALAPPDATA is set for every interactive Windows session this test suite runs under",
        );
        assert_eq!(
            super::data_dir_under(Some(local_app_data.clone())).unwrap(),
            std::path::Path::new(&local_app_data).join("FastPad")
        );
        assert!(super::data_dir_under(None).is_err());
        assert!(super::data_dir_under(Some(std::ffi::OsString::new())).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn in_process_tests_never_resolve_into_the_real_profile_or_documents() {
        // Break caught: an in-process test writing settings or a folder's local state into the
        // user's real %LOCALAPPDATA%\FastPad or Documents\FastPad.
        let data = super::fastpad_data_dir().unwrap();
        let notes = super::default_notes_folder().unwrap();
        let scratch = std::env::temp_dir().join(format!("fastpad-libtest-{}", std::process::id()));
        assert_eq!(data, scratch.join("FastPad"));
        assert_eq!(notes, scratch.join("Notes"));
        assert!(data.is_dir() && notes.is_dir());
    }
}
