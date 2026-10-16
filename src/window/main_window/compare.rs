//! Note: Compare with disk (second brain phase 1 spec §5.4): the file's text on disk, read-only,
//! beside the tab that conflicts with it. FastPad has no diff view; the two texts side by side
//! are enough to copy what is needed from either.

use super::*;

/// Opens the active tab's file as it is on disk in a read-only tab, in a new group to the right
/// (or in the active group when there is no room to split). The tab has no path, so it is never
/// saved; it is marked `compare`, so the session and recovery leave it out. Call it with nothing
/// of the App borrowed.
pub(crate) fn compare_with_disk(hwnd: HWND) {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    let Some((path, language, group)) = unsafe { app_ptr(hwnd) }.and_then(|app| {
        let app = unsafe { app.as_ref() };
        let active = app.tabs.active()?;
        if active.is_image() {
            return None;
        }
        Some((
            active.path.clone()?,
            active.language,
            app.tabs.active_group(),
        ))
    }) else {
        return;
    };
    let file = match crate::file::loader::load(&path).and_then(|file| {
        std::ffi::CString::new(file.text.as_str())
            .map_err(|_| crate::FastPadError::UnsupportedEncoding)?;
        Ok(file)
    }) {
        Ok(file) => file,
        Err(error) => {
            report_open_failure(hwnd, &path, &error);
            return;
        }
    };
    if let Some(right) = split_group(hwnd, group, crate::window::split_tree::Direction::Right)
        && identity.is_live_for(hwnd)
    {
        activate_group(hwnd, right);
    }
    if !identity.is_live_for(hwnd) || create_new_document(hwnd).is_err() {
        return;
    }
    let Some(editor) =
        unsafe { app_ptr(hwnd) }.and_then(|app| unsafe { app.as_ref() }.editor().cloned())
    else {
        return;
    };
    set_file_population(hwnd, true);
    let populated = editor.populate_clean(&file.text);
    if !identity.is_live_for(hwnd) {
        return;
    }
    set_file_population(hwnd, false);
    if populated.is_err() || editor.set_read_only(true).is_err() {
        return;
    }
    if let Some(mut app) = unsafe { app_ptr(hwnd) } {
        let tabs = &mut unsafe { app.as_mut() }.tabs;
        if let Some(document) = tabs.active_mut() {
            let name = path.file_name().map_or_else(
                || path.display().to_string(),
                |name| name.to_string_lossy().into_owned(),
            );
            document.compare = Some(format!("{name} (on disk)"));
        }
        tabs.refresh_views();
    }
    apply_language(hwnd, language);
    crate::window::preview_host::document_reloaded(hwnd);
    refresh_tabs(hwnd);
}
