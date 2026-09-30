//! Saving documents: Save, Save As, the save outcomes the library host completes, and reviewing
//! dirty documents before close.

use super::*;

/// Activates `id` (the prompt's modal loop can have activated another tab) and saves it. Reports
/// success only when that same document is the active, no-longer-dirty one afterwards.
pub(super) fn save_reviewed_document(hwnd: HWND, id: DocumentId) -> bool {
    if !activate_document_by_id(hwnd, id) || !save_active_document(hwnd) {
        return false;
    }
    unsafe { app_ptr(hwnd) }.is_some_and(|app| {
        let app = unsafe { app.as_ref() };
        app.tabs.active().is_some_and(|active| active.id == id)
            && app
                .tabs
                .document(id)
                .is_some_and(|document| !document.dirty)
    })
}

pub(in crate::window) fn save_active_document(hwnd: HWND) -> bool {
    if crate::window::image_host::active_is_image(hwnd) {
        return false;
    }
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return false;
    };
    let has_path = unsafe { app_ptr(hwnd) }
        .and_then(|app| Some(unsafe { app.as_ref() }.tabs.active()?.path.is_some()));
    match has_path {
        Some(true) => complete_save(hwnd, &identity, None),
        Some(false) => save_active_document_as(hwnd),
        None => false,
    }
}

pub(in crate::window) fn save_active_document_as(hwnd: HWND) -> bool {
    if crate::window::image_host::active_is_image(hwnd) {
        return false;
    }
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return false;
    };
    let Some((target, named, notes_mode)) = (unsafe { app_ptr(hwnd) }).and_then(|app| {
        let app = unsafe { app.as_ref() };
        let document = app.tabs.active()?;
        Some((
            document.id,
            document
                .path
                .as_deref()
                .and_then(std::path::Path::file_name)
                .map(|name| name.to_string_lossy().into_owned()),
            app.settings.notes_mode,
        ))
    }) else {
        return false;
    };
    // Only an untitled tab in notes mode starts in the notes folder under its label's name; every
    // other Save As keeps the dialog's usual suggestion and starting folder.
    let (suggested, folder) = match named {
        Some(name) => (name, None),
        // With no notebook open this is notes mode off's Save As.
        None if notes_mode && crate::window::library_host::folder(hwnd).is_some() => (
            crate::window::library_host::suggested_file_name(hwnd),
            crate::window::library_host::first_save_folder(hwnd),
        ),
        None => ("Untitled.txt".to_owned(), None),
    };
    // Modal Show reenters the window procedure. Only an owned identity crosses it.
    let selection = crate::window::modal::choose_save_path(hwnd, &suggested, folder.as_deref());
    if !identity.is_live_for(hwnd) {
        return false;
    }
    let path = match selection {
        Ok(Some(path)) => path,
        // Cancelling the dialog is not an error and says nothing.
        Ok(None) => return false,
        Err(error) => {
            push_notice(
                hwnd,
                format!("FastPad could not open the Save As dialog: {error}"),
            );
            return false;
        }
    };
    // The dialog's modal loop can have activated another tab; save the document that was chosen.
    if !activate_document_by_id(hwnd, target) {
        return false;
    }
    complete_save(hwnd, &identity, Some(path))
}

/// Test-only entry point that drives Save As with an explicit path, bypassing the native dialog.
/// The real dialog interaction is covered by `select_save_file`'s tests; this exists because the
/// shell's own "Confirm Save As" collision handling for an existing target could not be driven
/// reliably through synthetic window messages on this host, so the collision-rejection tail below
/// (identical production code `save_active_document_as` reaches after a real dialog selection) is
/// exercised directly instead, mirroring `open_path`'s existing non-dialog test entry point.
#[cfg(test)]
#[allow(
    dead_code,
    reason = "consumed by the source-linked save_file integration target"
)]
pub(crate) fn save_path_as(hwnd: HWND, path: &std::path::Path) {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    complete_save(hwnd, &identity, Some(path.to_path_buf()));
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::window) enum SaveOutcome {
    Saved,
    Failed,
    /// A first save found a file already at the chosen path and left it alone.
    NameTaken,
}

/// Shared tail of plain Save and Save As; see `save_active_to`.
pub(in crate::window) fn complete_save(
    hwnd: HWND,
    identity: &WindowIdentity,
    new_path: Option<std::path::PathBuf>,
) -> bool {
    save_active_to(hwnd, identity, new_path, false, true) == SaveOutcome::Saved
}

/// Plain Save for autosave: a failure pushes no generic notice, because the caller names the note.
pub(in crate::window) fn complete_autosave(hwnd: HWND, identity: &WindowIdentity) -> bool {
    save_active_to(hwnd, identity, None, false, false) == SaveOutcome::Saved
}

/// The first save of an untitled tab under a name picked in the name box. Never replaces a file:
/// one that appeared at `path` since the name was checked gives `NameTaken`, with no notice, the
/// tab still untitled and the file untouched.
pub(in crate::window) fn complete_first_save(
    hwnd: HWND,
    identity: &WindowIdentity,
    path: std::path::PathBuf,
) -> SaveOutcome {
    save_active_to(hwnd, identity, Some(path), true, true)
}

/// Shared tail of plain Save and Save As. `new_path` is `Some` only for Save As: the active
/// document's path is renamed (and checked against other open tabs' canonical paths) before the
/// write. Plain Save (`new_path: None`) writes to the document's existing path unchanged.
/// `create_new` refuses to replace an existing file (`SaveOutcome::NameTaken`). `report_failure`
/// pushes the generic "could not save" notice when the write fails.
///
/// For Save As, every failure after a successful rename (missing editor, a failed
/// `editor.text()` read, or a failed `save_atomic`) reverts the tab's path back to whatever it
/// held before this call: a failed write must never leave the tab claiming a path nothing was
/// actually written to, orphaning it from the path it was last genuinely saved at.
fn save_active_to(
    hwnd: HWND,
    identity: &WindowIdentity,
    new_path: Option<std::path::PathBuf>,
    create_new: bool,
    report_failure: bool,
) -> SaveOutcome {
    // Every save path ends here: an image tab's editor holds an empty placeholder, never its bytes.
    if crate::window::image_host::active_is_image(hwnd) {
        return SaveOutcome::Failed;
    }
    let is_save_as = new_path.is_some();
    let mut original_path: Option<std::path::PathBuf> = None;
    if let Some(path) = new_path {
        original_path = unsafe { app_ptr(hwnd) }
            .and_then(|app| unsafe { app.as_ref() }.tabs.active()?.path.clone());
        let outcome = unsafe { app_ptr(hwnd) }
            .map(|mut app| unsafe { app.as_mut() }.tabs.set_active_path(path));
        match outcome {
            Some(Ok(())) => {}
            Some(Err(_)) => {
                push_notice(
                    hwnd,
                    "This file is already open in another tab. Choose a different name.".to_owned(),
                );
                return SaveOutcome::Failed;
            }
            None => return SaveOutcome::Failed,
        }
    }
    if !identity.is_live_for(hwnd) {
        return SaveOutcome::Failed;
    }
    let Some((editor, path, encoding)) = (unsafe { app_ptr(hwnd) }).and_then(|app| {
        let app = unsafe { app.as_ref() };
        let editor = app.editor().cloned()?;
        let document = app.tabs.active()?;
        Some((editor, document.path.clone()?, document.encoding))
    }) else {
        if is_save_as {
            revert_active_path(hwnd, original_path);
        }
        return SaveOutcome::Failed;
    };
    let Ok(text) = editor.text() else {
        if is_save_as {
            revert_active_path(hwnd, original_path);
        }
        return SaveOutcome::Failed;
    };
    let bytes = crate::file::encoding::encode(&text, encoding);
    let result = if create_new {
        crate::file::saver::save_atomic_new(&path, &bytes)
    } else {
        crate::file::saver::save_atomic(&path, &bytes)
    };
    if !identity.is_live_for(hwnd) {
        return SaveOutcome::Failed;
    }
    match result {
        Ok(()) => {
            remove_saved_document_snapshots(hwnd);
            editor.set_save_point();
            // A recovered tab undone to the empty save point gets no save-point notification.
            let cleaned = identity.is_live_for(hwnd)
                && unsafe { app_ptr(hwnd) }
                    .is_some_and(|mut app| unsafe { app.as_mut() }.tabs.set_active_dirty(false));
            if cleaned && !is_save_as {
                invalidate_title_strip(hwnd);
            }
            if is_save_as {
                unsafe {
                    PostMessageW(hwnd, crate::window::WM_FASTPAD_APPLY_LANGUAGE, 0, 0);
                }
                invalidate_title_strip(hwnd);
            }
            if identity.is_live_for(hwnd) {
                crate::window::library_host::document_saved(hwnd);
            }
            SaveOutcome::Saved
        }
        Err(error) => {
            if is_save_as {
                revert_active_path(hwnd, original_path);
            }
            if create_new && crate::file::saver::is_already_exists(&error) {
                return SaveOutcome::NameTaken;
            }
            if !report_failure {
                return SaveOutcome::Failed;
            }
            push_notice(
                hwnd,
                "FastPad could not save this file. The previous version on disk was not modified."
                    .to_owned(),
            );
            SaveOutcome::Failed
        }
    }
}

fn revert_active_path(hwnd: HWND, original_path: Option<std::path::PathBuf>) {
    if let Some(mut app) = unsafe { app_ptr(hwnd) } {
        unsafe { app.as_mut() }
            .tabs
            .revert_active_path(original_path);
    }
}

/// Returns the documents explicitly discarded, or `None` when the close was cancelled.
pub(super) fn review_dirty_documents(hwnd: HWND) -> Option<Vec<DocumentId>> {
    let identity = unsafe { window_identity(hwnd) }?;
    let mut reviewed = Vec::<CloseReviewKey>::new();
    let mut discarded = Vec::<DocumentId>::new();
    loop {
        let pending = unsafe { app_ptr(hwnd) }.and_then(|app| {
            let app = unsafe { app.as_ref() };
            let review = app.tabs.next_dirty_review(&reviewed)?;
            let title = app.tabs.document(review.id)?.title();
            Some((review, title))
        });
        let Some((review, title)) = pending else {
            return Some(discarded);
        };
        let decision = prompt_close_decision(hwnd, &title);
        if decision == CloseDecision::Cancel || !identity.is_live_for(hwnd) {
            return None;
        }
        // A failed or cancelled save aborts the whole window close rather than losing the text.
        if decision == CloseDecision::Save {
            if !save_reviewed_document(hwnd, review.id) {
                return None;
            }
            continue;
        }
        let current = unsafe { app_ptr(hwnd) }
            .map(|app| unsafe { app.as_ref() }.tabs.dirty_review_is_current(review))
            .unwrap_or(false);
        if current {
            reviewed.push(review.key());
            discarded.retain(|id| *id != review.id);
            if decision == CloseDecision::Discard {
                discarded.push(review.id);
            }
        }
    }
}
