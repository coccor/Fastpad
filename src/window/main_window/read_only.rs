//! Read-only mode (issue #80): a tab opened for viewing only, from `--read-only` or File: Toggle
//! read-only. Scintilla keeps the flag on the document, so typing, pasting, the editing commands
//! and undo all stop in every view of it; Save and autosave leave the file alone. Changes made
//! outside FastPad still reload into it (`Editor::while_writable`).

use super::*;

/// Whether the active tab is open for viewing only.
pub(crate) fn active_is_read_only(hwnd: HWND) -> bool {
    unsafe { app_ptr(hwnd) }.is_some_and(|app| {
        unsafe { app.as_ref() }
            .tabs
            .active()
            .is_some_and(|document| document.read_only)
    })
}

/// File: Toggle read-only, for the active text tab.
pub(crate) fn toggle_read_only(hwnd: HWND) {
    let on = !active_is_read_only(hwnd);
    set_active_read_only(hwnd, on);
}

/// Makes the active text tab read-only, or editable again. A tab with unsaved edits is not made
/// read-only (they could then be neither saved nor changed), and a Compare with disk view stays
/// read-only: each says why in a notice. An image tab, which has no text, is left as it is.
/// Returns whether the tab is now as asked. Call it with nothing of the App borrowed.
pub(crate) fn set_active_read_only(hwnd: HWND, read_only: bool) -> bool {
    let Some((editor, refusal)) = unsafe { app_ptr(hwnd) }.and_then(|app| {
        let app = unsafe { app.as_ref() };
        let document = app.tabs.active()?;
        if document.is_image() {
            return None;
        }
        let refusal = if document.read_only == read_only {
            None
        } else if document.compare.is_some() {
            Some(format!("{} is always read-only.", document.title()))
        } else if read_only && document.dirty {
            Some(format!(
                "{} has unsaved changes. Save or undo them before making it read-only.",
                document.title()
            ))
        } else {
            None
        };
        Some((app.editor()?.clone(), refusal))
    }) else {
        return false;
    };
    if let Some(message) = refusal {
        push_notice(hwnd, message);
        return false;
    }
    if editor.set_read_only(read_only).is_err() {
        return false;
    }
    if let Some(mut app) = unsafe { app_ptr(hwnd) }
        && let Some(document) = unsafe { app.as_mut() }.tabs.active_mut()
    {
        document.read_only = read_only;
    }
    invalidate_status_bar(hwnd);
    true
}

/// "plan.md is read-only. Turn off File: Read-only to save it."
pub(crate) fn read_only_notice(title: &str, action: &str) -> String {
    format!("{title} is read-only. Turn off File: Read-only to {action}.")
}

/// `--read-only`: opens `path` as `open_path` does, then makes its tab read-only. A file already
/// open with unsaved edits is shown as it is, with a notice.
pub(crate) fn open_path_read_only(hwnd: HWND, path: &std::path::Path) -> Result<()> {
    open_path(hwnd, path)?;
    set_active_read_only(hwnd, true);
    Ok(())
}
