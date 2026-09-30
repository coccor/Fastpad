//! Notes mode on and off, untitled tabs' labels from their first lines, and where and how an
//! untitled tab's first save goes (Ctrl+N, Ctrl+S, Save As).

use super::*;
use crate::library;
use crate::library::title;
use crate::window::main_window::{app_ptr, push_notice};
use crate::window::name_box::NamePurpose;
use std::path::PathBuf;
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::KillTimer;

/// Notes mode was toggled: load the last folder, or flush and forget the library.
pub(crate) fn notes_mode_changed(hwnd: HWND, enabled: bool) {
    if enabled {
        if folder(hwnd).is_none() {
            open_library_step(hwnd);
        }
        return;
    }
    // A name box left open would do nothing on Enter once the library is gone.
    close_name_box(hwnd);
    // The setting change applies either way: the user asked for it. The pending operations are
    // tried twice, and a loss is said out loud.
    let flushed = (0..2).any(|_| {
        matches!(
            try_flush(hwnd, false),
            Ok(library::Flushed::Wrote | library::Flushed::Nothing)
        )
    });
    let folder = folder(hwnd);
    let lost = !flushed && with_state(hwnd, |state| !state.pending.is_empty()).unwrap_or(false);
    if lost && let Some(folder) = folder {
        push_notice(
            hwnd,
            format!(
                "Metadata changes could not be written to {}",
                crate::library::store::library_file(&folder).display()
            ),
        );
    }
    unsafe {
        KillTimer(hwnd, LIBRARY_WRITE_TIMER_ID);
    }
    // The Search view goes with the sidebar: its search stops and its record is dropped.
    crate::window::text_search_host::forget(hwnd);
    host(hwnd, |host| {
        host.state = None;
        host.folder = None;
        host.generation = host.generation.wrapping_add(1);
        host.scanning = false;
    });
}

/// Recomputes the active untitled tab's label from its first lines. It is kept with notes mode
/// off too (only edits near the top recompute it), and shown only with notes mode on.
pub(crate) fn refresh_label(hwnd: HWND) {
    let shown = notes_mode(hwnd);
    let changed = unsafe { app_ptr(hwnd) }.and_then(|mut app| {
        let app = unsafe { app.as_mut() };
        let editor = app.editor()?;
        let active = app.tabs.active()?;
        if active.path.is_some() {
            return None;
        }
        let id = active.id;
        let count = editor
            .line_count()
            .unwrap_or(0)
            .min(title::LABEL_SCAN_LINES);
        let lines: Vec<String> = (0..count)
            .map(|line| editor.line_text(line).unwrap_or_default())
            .collect();
        let label = title::untitled_label(lines.iter().map(String::as_str));
        let document = app.tabs.document_mut(id)?;
        document.label_watch = label.watch_through;
        document.first_line_label = label.text;
        if !shown || document.untitled_label == document.first_line_label {
            return None;
        }
        document.untitled_label = document.first_line_label.clone();
        Some((id, crate::window::open_editors::unsaved_label(document)))
    });
    if changed.is_some() {
        crate::window::main_window::refresh_tab_view(hwnd);
        crate::window::notebook_view::editors_changed(hwnd);
    }
}

/// The label a restored or recovered untitled document's text gives, set before it is shown.
pub(crate) fn label_restored_document(
    hwnd: HWND,
    document: &mut crate::document::Document,
    text: &str,
) {
    if document.path.is_some() {
        return;
    }
    let label = title::untitled_label(text.lines());
    document.label_watch = label.watch_through;
    document.first_line_label = label.text;
    if notes_mode(hwnd) {
        document.untitled_label = document.first_line_label.clone();
    }
}

/// Notes mode turned on: every untitled tab shows its label, without switching tabs.
pub(crate) fn show_labels(hwnd: HWND) {
    if let Some(mut app) = unsafe { app_ptr(hwnd) } {
        let app = unsafe { app.as_mut() };
        let ids: Vec<_> = app
            .tabs
            .documents()
            .filter(|document| document.path.is_none())
            .map(|document| document.id)
            .collect();
        for id in ids {
            if let Some(document) = app.tabs.document_mut(id) {
                document.untitled_label = document.first_line_label.clone();
            }
        }
    }
    crate::window::main_window::refresh_tab_view(hwnd);
}

/// `SCN_MODIFIED`: only edits at or above the label's line can change it.
/// A text change at `position` in `group`'s editor; its active tab's label may follow.
pub(crate) fn text_changed(hwnd: HWND, group: crate::window::split_tree::GroupId, position: usize) {
    let relevant = unsafe { app_ptr(hwnd) }.is_some_and(|app| {
        let app = unsafe { app.as_ref() };
        let Some(editor) = app.group(group).map(|state| &state.editor) else {
            return false;
        };
        let Some(active) = app
            .tabs
            .group(group)
            .and_then(|tabs| tabs.active_document())
            .and_then(|id| app.tabs.document(id))
        else {
            return false;
        };
        active.path.is_none()
            && editor
                .line_from_position(position)
                .is_ok_and(|line| line <= active.label_watch)
    });
    if relevant {
        refresh_label(hwnd);
    }
}

/// Notes mode turned off: untitled tabs go back to "Untitled".
pub(crate) fn clear_labels(hwnd: HWND) {
    if let Some(mut app) = unsafe { app_ptr(hwnd) } {
        let app = unsafe { app.as_mut() };
        let ids: Vec<_> = app.tabs.documents().map(|document| document.id).collect();
        for id in ids {
            if let Some(document) = app.tabs.document_mut(id) {
                document.untitled_label = None;
            }
        }
    }
    crate::window::main_window::refresh_tab_view(hwnd);
}

pub(super) fn active_untitled(hwnd: HWND) -> Option<crate::document::DocumentId> {
    unsafe { app_ptr(hwnd) }.and_then(|app| {
        let active = unsafe { app.as_ref() }.tabs.active()?;
        active.path.is_none().then_some(active.id)
    })
}

/// "<sanitized label>.<extension for the tab's language>".
pub(crate) fn suggested_file_name(hwnd: HWND) -> String {
    unsafe { app_ptr(hwnd) }
        .and_then(|app| {
            let active = unsafe { app.as_ref() }.tabs.active()?;
            let stem = title::sanitize_stem(active.untitled_label.as_deref().unwrap_or("Untitled"));
            Some(format!(
                "{stem}.{}",
                title::default_extension(active.language)
            ))
        })
        .unwrap_or_else(|| "Untitled.md".to_owned())
}

/// Ctrl+N and File → New tab. The new untitled tab remembers where its first save goes: the
/// folder of the sidebar's selected row, else the notebook root. With no notebook open it is a
/// plain new tab.
pub(crate) fn new_note_in(hwnd: HWND) {
    let destination = self::folder(hwnd).filter(|_| notes_mode(hwnd)).map(|root| {
        crate::window::notebook_view::selected_folder(hwnd)
            .filter(|candidate| {
                library::model::same_path(candidate, &root) || library::is_inside(&root, candidate)
            })
            .unwrap_or(root)
    });
    if let Err(error) = crate::window::main_window::create_new_document(hwnd) {
        push_notice(hwnd, format!("FastPad could not create a new tab: {error}"));
        return;
    }
    if let Some(mut app) = unsafe { app_ptr(hwnd) } {
        let app = unsafe { app.as_mut() };
        if let Some(id) = app.tabs.active().map(|document| document.id)
            && let Some(document) = app.tabs.document_mut(id)
        {
            document.save_folder = destination;
        }
    }
    // `create_new_document` already showed the new unsaved row: its tab switch rebuilt the
    // Notebook view's rows and selected it (`side_panel::active_tab_changed`).
}

/// Where tab `id`'s first save goes: its remembered folder while that is still a folder of the
/// open notebook, else the notebook root (spec §6.7).
pub(super) fn save_folder_for(hwnd: HWND, id: crate::document::DocumentId) -> Option<PathBuf> {
    let root = folder(hwnd)?;
    let remembered = unsafe { app_ptr(hwnd) }.and_then(|app| {
        unsafe { app.as_ref() }
            .tabs
            .document(id)?
            .save_folder
            .clone()
    });
    Some(
        remembered
            .filter(|folder| library::is_inside(&root, folder) && folder.is_dir())
            .unwrap_or(root),
    )
}

/// The active tab's first-save folder, for the name box and the Save As dialog.
pub(crate) fn first_save_folder(hwnd: HWND) -> Option<PathBuf> {
    let id = active_untitled(hwnd)?;
    save_folder_for(hwnd, id)
}

/// Ctrl+S: an untitled tab in notes mode is named in the name box; everything else as before.
pub(crate) fn save_command(hwnd: HWND) {
    // The editor holds an empty placeholder for an image tab: saving would write it over the image.
    if crate::window::image_host::active_is_image(hwnd) {
        return;
    }
    if notes_mode(hwnd)
        && folder(hwnd).is_some()
        && let Some(id) = active_untitled(hwnd)
    {
        // Ctrl+S again while the box is open for this tab keeps what was typed.
        if name_box_purpose(hwnd) == Some(NamePurpose::FirstSave(id)) {
            focus_name_box(hwnd);
            return;
        }
        refresh_label(hwnd);
        let suffix = format!(
            "in {}",
            first_save_folder(hwnd)
                .as_deref()
                .map_or_else(|| folder_display_name(hwnd), notebook_name)
        );
        open_name_box(
            hwnd,
            NamePurpose::FirstSave(id),
            &suggested_file_name(hwnd),
            suffix,
            true,
        );
        return;
    }
    let _ = crate::window::main_window::save_active_document(hwnd);
}

pub(crate) fn save_as_command(hwnd: HWND) {
    if crate::window::image_host::active_is_image(hwnd) {
        return;
    }
    if notes_mode(hwnd) && folder(hwnd).is_some() && active_untitled(hwnd).is_some() {
        save_command(hwnd);
        return;
    }
    let _ = crate::window::main_window::save_active_document_as(hwnd);
}

pub(super) fn folder_display_name(hwnd: HWND) -> String {
    folder(hwnd)
        .map(|f| notebook_name(&f))
        .unwrap_or_else(|| "the notebook".to_owned())
}
