//! Reading, replacing and reloading the text of documents no visible editor shows.

use super::*;

/// Runs `f` on the document host showing `target`, a document no visible editor shows (split
/// editors spec §3.1). The host is never painted and its notifications reach no window, so the
/// visible editor's view and the notification handler see none of this; callers record edits
/// themselves (`Tabs::note_background_edit`).
/// The editor of a group whose active view shows `id`: the active group's if it does, else the
/// first in layout order.
fn editor_showing(app: &App, id: DocumentId) -> Option<Editor> {
    let active = app.tabs.active_group();
    let showing = groups_showing(app, id);
    let group = showing
        .iter()
        .find(|group| **group == active)
        .or_else(|| showing.first())?;
    app.group(*group).map(|state| state.editor.clone())
}

pub(super) fn with_background_document<R>(
    hwnd: HWND,
    target: &crate::editor::EditorDocument,
    f: impl FnOnce(&Editor) -> Result<R>,
) -> Result<R> {
    let host = unsafe { app_ptr(hwnd) }
        .and_then(|app| unsafe { app.as_ref() }.document_host.clone())
        .ok_or(crate::FastPadError::Invariant(
            "the document host is missing",
        ))?;
    host.use_document(target)?;
    let result = f(&host);
    // Leave the host on a document of its own, so it never keeps a closed tab's text alive.
    if let Ok(blank) = host.create_document() {
        let _ = host.use_document(&blank);
    }
    result
}

/// The text of tab `id` as the editor has it, for the Search view's overlays. A background tab is
/// read through the document host (`with_background_document`). `None` without an editor or that
/// tab, while a file is being populated, or when Scintilla can't be read. Call it with nothing of
/// the App borrowed.
pub(crate) fn document_text(hwnd: HWND, id: DocumentId) -> Option<String> {
    let (editor, inactive) = unsafe { app_ptr(hwnd) }.and_then(|app| {
        let app = unsafe { app.as_ref() };
        // While a file is populated the editor may show a document that is not the active tab's.
        if app.populating_file {
            return None;
        }
        let editor = app.editor().cloned()?;
        let active = app.tabs.active()?;
        let target = app.tabs.document(id)?;
        if target.id == active.id {
            return Some((editor, None));
        }
        Some((editor, Some(target.text_handle()?.clone())))
    })?;
    match inactive {
        None => editor.text().ok(),
        Some(target) => with_background_document(hwnd, &target, Editor::text).ok(),
    }
}

/// Replaces every match of `matcher` in tab `id`'s live text with `template` (expanded in regex
/// mode), in the editor, as one undo action (note-search spec §12). The tab is not saved. The
/// active tab's edit raises Scintilla's notifications as typing does. A background tab is
/// edited through the document host (`with_background_document`), whose notifications reach no
/// window, and then marked edited by hand (`Tabs::note_background_edit`). Returns how many matches were replaced, or `None`
/// without an editor or that tab, while a file is being populated, or when Scintilla fails. Call
/// it with nothing of the App borrowed.
pub(crate) fn replace_in_document(
    hwnd: HWND,
    id: DocumentId,
    matcher: &crate::search::Matcher,
    template: &str,
) -> Option<usize> {
    let identity = unsafe { window_identity(hwnd) }?;
    let (editor, inactive) = unsafe { app_ptr(hwnd) }.and_then(|app| {
        let app = unsafe { app.as_ref() };
        // While a file is populated the editor may show a document that is not the active tab's.
        if app.populating_file {
            return None;
        }
        let target = app.tabs.document(id)?;
        // A group showing the document edits it, so its notifications record the change once.
        if let Some(editor) = editor_showing(app, id) {
            return Some((editor, None));
        }
        let editor = app.editor().cloned()?;
        Some((editor, Some(target.text_handle()?.clone())))
    })?;
    // Set once Scintilla is asked to change the text: from then on it may have changed, even if
    // a replacement then fails partway.
    let touched = std::cell::Cell::new(false);
    let replace = |editor: &Editor| -> Result<usize> {
        let edits = editor.with_document_text(|text| matcher.replacements(text, template))?;
        if edits.is_empty() {
            return Ok(0);
        }
        touched.set(true);
        editor.replace_ranges_with(&edits)
    };
    match inactive {
        None => replace(&editor).ok(),
        Some(target) => {
            // What the edit replaced, even if a later step fails.
            let done = std::cell::Cell::new(None);
            let result = with_background_document(hwnd, &target, |e| {
                let replaced = replace(e)?;
                done.set(Some(replaced));
                Ok(replaced)
            });
            // The tab's text may have changed (a replacement that failed partway, or a restore
            // that failed after it), so it is marked edited whatever the result.
            if touched.get() && identity.is_live_for(hwnd) {
                let changed = unsafe { app_ptr(hwnd) }
                    .is_some_and(|mut app| unsafe { app.as_mut() }.tabs.note_background_edit(id));
                if changed {
                    invalidate_title_strip(hwnd);
                }
            }
            done.get().or(result.ok())
        }
    }
}

/// A tab's text generation and disk stamp, taken when a reload of it begins.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TabMark {
    pub generation: u64,
    pub disk_stamp: Option<crate::library::DiskStamp>,
}

/// Shows `loaded` (read on a worker) in tab `id`, as a file open populates a tab: no undo
/// history and no notifications, the tab left clean, and its encoding and disk stamp taken from
/// the read. The caret and scroll position stay where they were, as far as the new text allows.
/// Only a tab still open on `path`, still clean, and with the same generation and disk stamp as
/// `mark` is changed: an edit since (saved or not) keeps its text, and its old disk stamp pauses
/// its autosave. Returns whether the tab was reloaded. Call it with nothing of the App borrowed.
pub(crate) fn reload_clean_document(
    hwnd: HWND,
    id: DocumentId,
    path: &std::path::Path,
    mark: TabMark,
    loaded: &crate::file::loader::LoadedFile,
    stamp: Option<crate::library::DiskStamp>,
) -> bool {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return false;
    };
    let Some((editor, inactive)) = unsafe { app_ptr(hwnd) }.and_then(|app| {
        let app = unsafe { app.as_ref() };
        if app.populating_file {
            return None;
        }
        let target = app.tabs.document(id)?;
        let unchanged = TabMark {
            generation: target.generation,
            disk_stamp: target.disk_stamp,
        } == mark;
        if target.dirty || target.path.as_deref() != Some(path) || !unchanged {
            return None;
        }
        if let Some(editor) = editor_showing(app, id) {
            return Some((editor, None));
        }
        let editor = app.editor().cloned()?;
        Some((editor, Some(target.text_handle()?.clone())))
    }) else {
        return false;
    };
    let populate = |editor: &Editor| editor.populate_clean(&loaded.text);
    let populated = match &inactive {
        Some(target) => with_background_document(hwnd, target, populate),
        None => {
            use crate::editor::scintilla_constants::{
                SCI_GETFIRSTVISIBLELINE, SCI_SETFIRSTVISIBLELINE,
            };
            let selection = editor.selection();
            let first_line = unsafe { SendMessageW(editor.hwnd(), SCI_GETFIRSTVISIBLELINE, 0, 0) };
            set_file_population(hwnd, true);
            let populated = populate(&editor);
            if identity.is_live_for(hwnd) {
                if let Ok(selection) = selection {
                    let _ = editor.set_selection(selection);
                }
                unsafe {
                    SendMessageW(
                        editor.hwnd(),
                        SCI_SETFIRSTVISIBLELINE,
                        first_line as usize,
                        0,
                    );
                }
                set_file_population(hwnd, false);
            }
            populated
        }
    };
    if populated.is_err() || !identity.is_live_for(hwnd) {
        return false;
    }
    if let Some(mut app) = unsafe { app_ptr(hwnd) }
        && let Some(document) = unsafe { app.as_mut() }.tabs.document_mut(id)
    {
        document.encoding = loaded.encoding;
        document.disk_stamp = stamp;
        document.autosave_paused = false;
    }
    if inactive.is_none() {
        // Population suppressed SCN_MODIFIED: the preview reads the new text.
        crate::window::preview_host::document_reloaded(hwnd);
    }
    true
}

pub(super) fn set_file_population(hwnd: HWND, active: bool) {
    if let Some(mut app) = unsafe { app_ptr(hwnd) } {
        unsafe { app.as_mut() }.populating_file = active;
    }
}
