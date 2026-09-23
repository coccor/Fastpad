//! Showing a file in Explorer.

use crate::platform::wide_null;
use std::path::Path;
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

/// Explorer's argument that opens `path`'s folder with `path` selected. The path is quoted:
/// Explorer splits its arguments on commas.
pub fn select_argument(path: &Path) -> String {
    format!("/select,\"{}\"", path.display())
}

#[cfg(test)]
thread_local! {
    static REVEALED: std::cell::RefCell<Vec<std::path::PathBuf>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// The paths `reveal_in_explorer` was asked to show on this thread. Tests never start Explorer.
#[cfg(test)]
pub fn take_revealed() -> Vec<std::path::PathBuf> {
    REVEALED.with(|revealed| std::mem::take(&mut *revealed.borrow_mut()))
}

#[cfg(test)]
fn recorded_for_test(path: &Path) -> bool {
    REVEALED.with(|revealed| revealed.borrow_mut().push(path.to_path_buf()));
    true
}

#[cfg(not(test))]
fn recorded_for_test(_path: &Path) -> bool {
    false
}

/// Opens an Explorer window on `path`'s folder with `path` (a file or a folder) selected.
/// `SHOpenFolderAndSelectItems` would reuse an open window, but windows-sys gates it behind
/// `Win32_UI_Shell_Common`, which FastPad does not enable. `explorer.exe /select` needs only the
/// features already on.
pub fn reveal_in_explorer(path: &Path) -> crate::Result<()> {
    if recorded_for_test(path) {
        return Ok(());
    }
    let operation = wide_null("open");
    let file = wide_null("explorer.exe");
    let arguments = wide_null(&select_argument(path));
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            operation.as_ptr(),
            file.as_ptr(),
            arguments.as_ptr(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    // ShellExecuteW reports failure as a value of 32 or less (the SE_ERR_* codes).
    if result as usize <= 32 {
        return Err(crate::FastPadError::Invariant(
            "Explorer could not be started",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explorer_is_asked_to_select_the_quoted_path() {
        // Break caught: a path with spaces or commas split by Explorer's argument parser, which
        // then opens Documents instead of the note's folder.
        assert_eq!(
            select_argument(Path::new(r"C:\My Notes\a, b.md")),
            r#"/select,"C:\My Notes\a, b.md""#
        );
    }
}
