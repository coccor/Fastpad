//! The name box: first-save naming of an untitled note and renaming a note, with its errors
//! and the Browse… fallback to the Save As dialog.

use super::*;
use crate::library;
use crate::library::title;
use crate::window::main_window::{app_ptr, push_notice, window_identity};
use crate::window::name_box::{NameBox, NamePurpose};
use std::path::Path;
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW;

pub(crate) fn open_name_box(
    hwnd: HWND,
    purpose: NamePurpose,
    text: &str,
    suffix: String,
    browse: bool,
) {
    crate::window::main_window::close_find_bar(hwnd);
    let colors = crate::window::main_window::current_palette(hwnd);
    let shown = unsafe { app_ptr(hwnd) }.is_some_and(|mut app| {
        let app = unsafe { app.as_mut() };
        if app.name_box.is_none() {
            app.name_box = NameBox::create(hwnd).ok();
        }
        match app.name_box.as_mut() {
            Some(name_box) => {
                name_box.show(purpose, text, suffix, browse, colors);
                true
            }
            None => false,
        }
    });
    if !shown {
        push_notice(hwnd, "FastPad could not show the name box.".to_owned());
        return;
    }
    crate::window::main_window::layout_editor_and_find_bar(hwnd);
    focus_name_box(hwnd);
}

pub(super) fn focus_name_box(hwnd: HWND) {
    if let Some(app) = unsafe { app_ptr(hwnd) }
        && let Some(name_box) = unsafe { app.as_ref() }.name_box.as_ref()
    {
        name_box.focus();
    }
}

/// The purpose of the visible name box, if one is showing.
pub(super) fn name_box_purpose(hwnd: HWND) -> Option<NamePurpose> {
    unsafe { app_ptr(hwnd) }.and_then(|app| {
        let name_box = unsafe { app.as_ref() }.name_box.as_ref()?;
        if !name_box.is_visible() {
            return None;
        }
        name_box.purpose().cloned()
    })
}

/// Closes a name box whose tab is gone or no longer active, or whose first save already happened.
pub(crate) fn close_stale_name_box(hwnd: HWND) {
    let Some(purpose) = name_box_purpose(hwnd) else {
        return;
    };
    let id = purpose.document();
    let stale = unsafe { app_ptr(hwnd) }.is_none_or(|app| {
        let tabs = &unsafe { app.as_ref() }.tabs;
        let Some(document) = tabs.document(id) else {
            return true;
        };
        tabs.active().is_none_or(|active| active.id != id)
            || (matches!(purpose, NamePurpose::FirstSave(_)) && document.path.is_some())
    });
    if stale {
        close_name_box(hwnd);
    }
}

pub(crate) fn close_name_box(hwnd: HWND) {
    let hidden = unsafe { app_ptr(hwnd) }.is_some_and(|mut app| {
        unsafe { app.as_mut() }
            .name_box
            .as_mut()
            .is_some_and(|name_box| {
                let was = name_box.is_visible();
                name_box.hide();
                was
            })
    });
    if hidden {
        crate::window::main_window::layout_editor_and_find_bar(hwnd);
        crate::window::main_window::focus_content(hwnd);
    }
}

pub(super) fn name_box_state(hwnd: HWND) -> Option<(NamePurpose, String)> {
    unsafe { app_ptr(hwnd) }.and_then(|app| {
        let name_box = unsafe { app.as_ref() }.name_box.as_ref()?;
        Some((name_box.purpose()?.clone(), name_box.text()))
    })
}

pub(super) fn name_box_error(hwnd: HWND, error: String) {
    let stored = unsafe { app_ptr(hwnd) }.is_some_and(|mut app| {
        unsafe { app.as_mut() }
            .name_box
            .as_mut()
            .map(|name_box| name_box.set_error(Some(error)))
            .is_some()
    });
    // The error is wider than the suffix, so the field shrinks to make room.
    if stored {
        crate::window::main_window::layout_editor_and_find_bar(hwnd);
    }
}

/// "<name> already exists. Try <first free name>."
pub(crate) fn name_taken_error(folder: &Path, stem: &str, extension: &str) -> String {
    let free = title::free_name(stem, extension, |candidate| folder.join(candidate).exists());
    format!(
        "{} already exists. Try {free}.",
        title::file_name(stem, extension)
    )
}

/// Enter or Save in the name box.
pub(crate) fn name_box_submit(hwnd: HWND) {
    let Some((purpose, text)) = name_box_state(hwnd) else {
        return;
    };
    match purpose {
        NamePurpose::FirstSave(id) => submit_first_save(hwnd, id, &text),
        NamePurpose::RenameNote(_) if !ready_library(hwnd) => close_name_box(hwnd),
        NamePurpose::RenameNote(id) => submit_rename(hwnd, id, &text),
    }
}

pub(super) fn submit_first_save(hwnd: HWND, id: crate::document::DocumentId, text: &str) {
    let Some(folder) = save_folder_for(hwnd, id) else {
        return;
    };
    let language = unsafe { app_ptr(hwnd) }
        .and_then(|app| Some(unsafe { app.as_ref() }.tabs.document(id)?.language))
        .unwrap_or(crate::document::Language::Markdown);
    let (stem, extension) = title::split_typed_name(text, title::default_extension(language));
    let name = title::file_name(&stem, &extension);
    let target = folder.join(&name);
    if target.exists() {
        name_box_error(hwnd, name_taken_error(&folder, &stem, &extension));
        return;
    }
    if let Err(error) = std::fs::create_dir_all(&folder) {
        name_box_error(
            hwnd,
            format!("FastPad could not create {}: {error}", folder.display()),
        );
        return;
    }
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    if !crate::window::main_window::activate_document_by_id(hwnd, id) {
        return;
    }
    let outcome = crate::window::main_window::complete_first_save(hwnd, &identity, target);
    if !identity.is_live_for(hwnd) {
        return;
    }
    match outcome {
        crate::window::main_window::SaveOutcome::Saved => close_name_box(hwnd),
        // A file took the name after the check above; it is left alone.
        crate::window::main_window::SaveOutcome::NameTaken => {
            name_box_error(hwnd, name_taken_error(&folder, &stem, &extension));
        }
        crate::window::main_window::SaveOutcome::Failed => {}
    }
}

/// Note: Rename… on a file with no row in the Notebook tree (outside the open notebook, or not
/// a note): the name bar, prefilled with the file's current name.
pub(crate) fn rename_note(hwnd: HWND) {
    let Some(path) = active_file(hwnd) else {
        return;
    };
    let Some(id) =
        unsafe { app_ptr(hwnd) }.and_then(|app| Some(unsafe { app.as_ref() }.tabs.active()?.id))
    else {
        return;
    };
    // The tab is about to be renamed: a preview would be replaced by the next click in the tree
    // and take the name box with it, so it becomes a normal tab.
    promote_tab_for(hwnd, &path);
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    open_name_box(
        hwnd,
        NamePurpose::RenameNote(id),
        &name,
        "Rename".to_owned(),
        false,
    );
}

/// Renames the tab's file in place, never over another file, then follows it in the tab and the
/// library.
pub(super) fn submit_rename(hwnd: HWND, id: crate::document::DocumentId, text: &str) {
    let Some(old) = unsafe { app_ptr(hwnd) }
        .and_then(|app| unsafe { app.as_ref() }.tabs.document(id)?.path.clone())
    else {
        close_name_box(hwnd);
        return;
    };
    let current_extension = old.extension().map(|e| e.to_string_lossy().into_owned());
    let (stem, extension) = title::split_rename(text, current_extension.as_deref());
    let extension = extension.unwrap_or_default();
    let new = old.with_file_name(title::file_name(&stem, &extension));
    if new == old {
        close_name_box(hwnd);
        return;
    }
    // A change of letter case only names the same file, so it is not a clash.
    let case_only = library::model::same_path(&new, &old);
    let parent = old.parent().map(Path::to_path_buf).unwrap_or_default();
    if new.exists() && !case_only {
        name_box_error(hwnd, name_taken_error(&parent, &stem, &extension));
        return;
    }
    if let Err(error) = crate::platform::files::rename_no_replace(&old, &new) {
        // A file may have taken the name after the check above; it is left alone.
        let error = if new.exists() && !case_only {
            name_taken_error(&parent, &stem, &extension)
        } else {
            format!("FastPad could not rename the file: {error}")
        };
        name_box_error(hwnd, error);
        return;
    }
    let rebound = unsafe { app_ptr(hwnd) }.is_some_and(|mut app| {
        unsafe { app.as_mut() }
            .tabs
            .rebind_path(id, new.clone())
            .is_ok()
    });
    if !rebound {
        // Undo, so the tab and the disk agree.
        let _ = crate::platform::files::rename_no_replace(&new, &old);
        name_box_error(hwnd, "Another tab already has that file open.".to_owned());
        return;
    }
    with_state(hwnd, |state| state.rename_note(&old, &new));
    schedule_write(hwnd);
    close_name_box(hwnd);
    crate::window::main_window::invalidate_title_strip(hwnd);
    crate::window::side_panel::refresh(hwnd);
    // The extension may have changed, and with it the language.
    unsafe {
        PostMessageW(hwnd, crate::window::WM_FASTPAD_APPLY_LANGUAGE, 0, 0);
    }
}

/// Browse… in the name box: the system Save As dialog, starting in the folder.
pub(crate) fn name_box_browse(hwnd: HWND) {
    let Some((NamePurpose::FirstSave(id), _)) = name_box_state(hwnd) else {
        return;
    };
    close_name_box(hwnd);
    if crate::window::main_window::activate_document_by_id(hwnd, id) {
        let _ = crate::window::main_window::save_active_document_as(hwnd);
    }
}
