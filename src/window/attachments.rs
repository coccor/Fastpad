//! Images and files inside Markdown notes (second brain phase 1 spec §10.5): Ctrl+V of a lone
//! image or of files copied in Explorer, Ctrl+drop, and Notes: Insert attachment… save into the
//! attachments folder and insert links at the caret, as one undo step.

use super::main_window::{app_ptr, push_notice};
use crate::document::DocumentId;
use crate::editor::Editor;
use crate::library::capture::{self, Attachment};
use crate::library::capture_file::{self, LARGE_FILE_BYTES, Source};
use std::path::{Path, PathBuf};
use windows_sys::Win32::Foundation::HWND;

/// What an untitled tab says when asked to take an attachment.
pub(crate) const SAVE_FIRST: &str = "Save the note first, so the image has a place to go.";

/// Where a note's attachments go: the folder, absolute, and the note's own path.
struct Place {
    directory: PathBuf,
    note: PathBuf,
    /// The notebook it lives in, when it is in the open one.
    root: Option<PathBuf>,
}

/// The attachments folder for the note at `note` (absolute): the open notebook's (from
/// `notebook.ini`, else Obsidian's setting, else `attachments`), or `attachments` beside a
/// Markdown file outside it.
fn place_for(hwnd: HWND, note: &Path) -> Place {
    if let Some(root) = super::library_host::folder(hwnd)
        && crate::library::is_inside(&root, note)
    {
        let settings = super::library_host::notebook_settings(hwnd);
        let obsidian = capture_file::obsidian_attachments_of(&root);
        let relative = crate::library::record_path(&root, note);
        let folder = capture::attachments_folder(
            settings.attachments_folder.as_deref(),
            obsidian.as_ref(),
            &relative,
        );
        return Place {
            directory: root.join(folder),
            note: note.to_path_buf(),
            root: Some(root),
        };
    }
    Place {
        directory: note.parent().unwrap_or(Path::new("")).join("attachments"),
        note: note.to_path_buf(),
        root: None,
    }
}

/// A Markdown tab's path, or `None` for an untitled one.
fn document_path(hwnd: HWND, document: DocumentId) -> Option<PathBuf> {
    let app = unsafe { app_ptr(hwnd) }?;
    unsafe { app.as_ref() }
        .tabs
        .document(document)?
        .path
        .clone()
}

/// A pasted image's name (spec §10.5): `<note stem> YYYY-MM-DD HHmmss`.
pub(crate) fn image_stem(note: &Path) -> String {
    let (date, time) = super::library_host::now();
    let second = if cfg!(test) {
        0
    } else {
        crate::platform::clock::local_second()
    };
    format!(
        "{} {} {:02}{:02}{second:02}",
        note.file_stem().unwrap_or_default().to_string_lossy(),
        crate::library::dates::format(date, "YYYY-MM-DD"),
        time.hour,
        time.minute
    )
}

/// Saves `sources` for the note in `document` and inserts one link per file at `at` (the
/// caret when `None`), as one undo step. Files over 20 MB ask first. Returns whether anything
/// was inserted.
pub(crate) fn attach(
    hwnd: HWND,
    editor: &Editor,
    document: DocumentId,
    sources: Vec<Source>,
    at: Option<usize>,
) -> bool {
    let Some(note) = document_path(hwnd, document) else {
        push_notice(hwnd, SAVE_FIRST.to_owned());
        return false;
    };
    let place = place_for(hwnd, &note);
    let stem = image_stem(&note);
    let mut saved = Vec::new();
    for source in sources {
        if let Source::File(path) = &source {
            if path.is_dir() {
                continue;
            }
            let size = std::fs::metadata(path).map_or(0, |meta| meta.len());
            if size > LARGE_FILE_BYTES {
                let question = format!(
                    "{} is {} MB. Copy it into the notebook anyway?",
                    path.file_name().unwrap_or_default().to_string_lossy(),
                    size.div_ceil(1024 * 1024)
                );
                if super::modal::choose(hwnd, &question, &["Copy it", "Leave it out"]) != 0 {
                    continue;
                }
            }
        }
        let stored = capture_file::store(&place.directory, &source, &stem);
        match stored {
            Ok(name) => {
                let path = place.directory.join(&name);
                // The watcher's report of the new file is FastPad's own, not an outside change.
                super::library_host::begin_own_writes(hwnd, [path.as_path()]);
                super::library_host::end_own_writes(hwnd, [path.as_path()]);
                saved.push(Attachment {
                    image: matches!(source, Source::Png(_)) || capture_file::is_image(&path),
                    path,
                });
            }
            Err(error) => push_notice(hwnd, error),
        }
    }
    if saved.is_empty() {
        return false;
    }
    if place.root.is_some() {
        for attachment in &saved {
            super::library_host::with_state(hwnd, |state| state.add_note(&attachment.path));
        }
        super::side_panel::refresh(hwnd);
    }
    let eol = editor.eol().unwrap_or("\r\n");
    let text = capture::link_lines(&place.note, &saved).join(eol);
    insert(editor, &text, at)
}

/// Inserts `text` at `at` (else over the selection) as one undo step, leaving the caret after
/// it.
fn insert(editor: &Editor, text: &str, at: Option<usize>) -> bool {
    let Ok(range) = at.map_or_else(|| editor.selection(), |at| Ok(at..at)) else {
        return false;
    };
    editor.begin_undo_action();
    let inserted = editor.replace_target(range, text);
    editor.end_undo_action();
    match inserted {
        Ok(range) => {
            let _ = editor.set_selection(range.end..range.end);
            editor.scroll_caret_into_view();
            true
        }
        Err(_) => false,
    }
}

/// Ctrl+V in a Markdown tab (spec §10.5): files copied in Explorer, or an image with no text
/// beside it (`force_image` takes the image anyway), become attachments. Returns whether the
/// paste was taken; otherwise the editor pastes as before.
pub(crate) fn paste(hwnd: HWND, editor: &Editor, document: DocumentId, force_image: bool) -> bool {
    use crate::platform::clipboard;
    if !super::markdown_host::has_links(hwnd, document) {
        return false;
    }
    let files = clipboard::has_files();
    let image = clipboard::has_image();
    if !(files || (image && (force_image || !clipboard::has_text()))) {
        if force_image {
            push_notice(hwnd, "The clipboard has no image.".to_owned());
            return true;
        }
        return false;
    }
    if document_path(hwnd, document).is_none() {
        push_notice(hwnd, SAVE_FIRST.to_owned());
        return true;
    }
    let sources = if files && !force_image {
        clipboard::files(hwnd)
            .into_iter()
            .map(Source::File)
            .collect()
    } else {
        match clipboard::image_png(hwnd) {
            Some(Ok(png)) => vec![Source::Png(png)],
            Some(Err(error)) => {
                push_notice(hwnd, error);
                return true;
            }
            None => return false,
        }
    };
    attach(hwnd, editor, document, sources, None);
    true
}

/// The active tab's editor and document, when it is a Markdown note.
fn active_markdown(hwnd: HWND) -> Option<(Editor, DocumentId)> {
    let (editor, document) = unsafe { app_ptr(hwnd) }.and_then(|app| {
        let app = unsafe { app.as_ref() };
        Some((app.editor().cloned()?, app.tabs.active()?.id))
    })?;
    super::markdown_host::has_links(hwnd, document).then_some((editor, document))
}

/// Notes: Paste image as attachment: the clipboard's image even when it holds text too.
pub(crate) fn paste_image_command(hwnd: HWND) {
    match active_markdown(hwnd) {
        Some((editor, document)) => {
            paste(hwnd, &editor, document, true);
        }
        None => push_notice(
            hwnd,
            "Attachments go into Markdown notes. Open one first.".to_owned(),
        ),
    }
}

/// Notes: Insert attachment…: a file picked in the Open dialog.
pub(crate) fn insert_attachment_command(hwnd: HWND) {
    let Some((editor, document)) = active_markdown(hwnd) else {
        push_notice(
            hwnd,
            "Attachments go into Markdown notes. Open one first.".to_owned(),
        );
        return;
    };
    if document_path(hwnd, document).is_none() {
        push_notice(hwnd, SAVE_FIRST.to_owned());
        return;
    }
    match crate::platform::dialogs::show_open_dialog(hwnd) {
        Ok(Some(path)) => {
            attach(hwnd, &editor, document, vec![Source::File(path)], None);
        }
        Ok(None) => {}
        Err(error) => push_notice(
            hwnd,
            format!("FastPad could not open the file picker: {error}"),
        ),
    }
}

/// Files dropped with Ctrl held on the editor at client point `point` (or on its group's
/// strips, at the caret): attached to the Markdown note there. Returns false when the drop is
/// not for a Markdown note, so the files open as tabs as before.
pub(crate) fn dropped(
    hwnd: HWND,
    editor: &Editor,
    document: DocumentId,
    paths: Vec<PathBuf>,
    point: Option<(i32, i32)>,
) -> bool {
    if !super::markdown_host::has_links(hwnd, document) {
        return false;
    }
    let at = point.and_then(|(x, y)| editor.position_at(x, y));
    attach(
        hwnd,
        editor,
        document,
        paths.into_iter().map(Source::File).collect(),
        at,
    );
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pasted_image_is_named_after_its_note_and_the_time() {
        crate::window::library_host::set_now(
            crate::library::dates::Date::new(2026, 10, 19).unwrap(),
            crate::library::dates::Time { hour: 9, minute: 5 },
        );
        assert_eq!(
            image_stem(Path::new(r"C:\notes\Atlas plan.md")),
            "Atlas plan 2026-10-19 090500"
        );
    }
}
