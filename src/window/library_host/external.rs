//! Changes made to files outside FastPad (second brain phase 1 spec §5): checking a tab against
//! its file, then reloading it, taking in an append, or raising a conflict. Applies to every text
//! tab with a path, notes mode or not.

use super::*;
use crate::document::DocumentId;
use crate::file::loader::LoadedFile;
use crate::library::DiskStamp;
use crate::library::external::{
    self, ConflictKind, DiskChange, ReloadPolicy, TabState, TabVerdict, TextBase,
};
use crate::library::title;
use crate::window::main_window::{
    DiskEdit, apply_disk_edit, invalidate_title_strip, push_notice, show_status_hint,
    window_identity,
};

fn policy(hwnd: HWND) -> ReloadPolicy {
    let auto = unsafe { app_ptr(hwnd) }
        .is_none_or(|app| unsafe { app.as_ref() }.settings.reload_changed_files);
    if auto {
        ReloadPolicy::Auto
    } else {
        ReloadPolicy::Ask
    }
}

/// Reads `path` as a tab would open it. A NUL byte cannot round-trip through Scintilla's UTF-8
/// buffer, so such a file is unsupported, as when opening it.
fn load_text(path: &std::path::Path) -> crate::Result<LoadedFile> {
    let loaded = crate::file::loader::load(path)?;
    std::ffi::CString::new(loaded.text.as_str())
        .map_err(|_| crate::FastPadError::UnsupportedEncoding)?;
    Ok(loaded)
}

/// What a check needs to know about tab `id`, or `None` for a tab that has no file to compare
/// with: untitled, an image (the image view checks its own), or a Compare with disk view.
fn tab_state(hwnd: HWND, id: DocumentId) -> Option<(PathBuf, TabState, Option<TextBase>)> {
    let app = unsafe { app_ptr(hwnd) }?;
    let document = unsafe { app.as_ref() }.tabs.document(id)?;
    if document.is_image() || document.compare.is_some() {
        return None;
    }
    Some((
        document.path.clone()?,
        TabState {
            dirty: document.dirty,
            known: document.disk_stamp,
            deleted_on_disk: document.deleted_on_disk,
        },
        document.text_base,
    ))
}

/// Checks tab `id` against its file and does what the verdict says (spec §5.2). The file is read
/// only when its stamp differs from the one the tab knows. Call it with nothing of the App
/// borrowed.
pub(crate) fn check_tab(hwnd: HWND, id: DocumentId) -> TabVerdict {
    if crate::window::main_window::file_population_active(hwnd) {
        return TabVerdict::Unchanged;
    }
    let Some((path, tab, base)) = tab_state(hwnd, id) else {
        return TabVerdict::Unchanged;
    };
    let now = library::disk_stamp(&path);
    let mut loaded = None;
    let mut unreadable = false;
    let verdict = external::verdict(tab, now, policy(hwnd), || match load_text(&path) {
        Ok(file) => {
            let change = external::classify(base, &file.text);
            loaded = Some(file);
            change
        }
        Err(_) => {
            unreadable = true;
            DiskChange::Other
        }
    });
    if unreadable {
        // Locked mid-write, or no longer text: the tab is left as it is, and the next check
        // tries again.
        return TabVerdict::Unchanged;
    }
    match verdict {
        TabVerdict::Unchanged => {
            // Rewritten with the same text (or read before a change that never came): only the
            // stamp moves, so the file isn't read again next time.
            if loaded.is_some() {
                update(hwnd, id, |document| document.disk_stamp = now);
            }
        }
        TabVerdict::Reload => {
            let Some(file) = loaded else {
                return TabVerdict::Unchanged;
            };
            if !take_disk_text(hwnd, id, &file, now) {
                return TabVerdict::Unchanged;
            }
            show_status_hint(
                hwnd,
                &format!(
                    "Reloaded {}: changed outside FastPad",
                    title::note_title(&path)
                ),
            );
        }
        TabVerdict::AppendMerge { at } => {
            let Some(file) = loaded else {
                return TabVerdict::Unchanged;
            };
            if !apply_disk_edit(hwnd, id, DiskEdit::Append(&file.text[at..])) {
                return TabVerdict::Unchanged;
            }
            update(hwnd, id, |document| {
                document.disk_stamp = now;
                document.text_base = Some(TextBase::of(&file.text));
            });
            show_status_hint(
                hwnd,
                &format!(
                    "Added text to {} from outside FastPad",
                    title::note_title(&path)
                ),
            );
        }
        TabVerdict::Conflict(kind) => raise_conflict(hwnd, id, &path, kind),
        TabVerdict::Deleted | TabVerdict::Restored => {
            let deleted = verdict == TabVerdict::Deleted;
            update(hwnd, id, |document| document.deleted_on_disk = deleted);
        }
    }
    verdict
}

/// Puts `file`'s text in tab `id` as one undoable edit, leaves the tab clean, and takes the
/// file's encoding and stamp. Returns whether it was done.
fn take_disk_text(hwnd: HWND, id: DocumentId, file: &LoadedFile, stamp: Option<DiskStamp>) -> bool {
    if !apply_disk_edit(hwnd, id, DiskEdit::Reload(&file.text)) {
        return false;
    }
    update(hwnd, id, |document| {
        document.encoding = file.encoding;
        document.disk_stamp = stamp;
        document.text_base = Some(TextBase::of(&file.text));
        document.autosave_paused = false;
        document.conflict = None;
        document.deleted_on_disk = false;
    });
    true
}

fn raise_conflict(hwnd: HWND, id: DocumentId, path: &std::path::Path, kind: ConflictKind) {
    let mut already = false;
    update(hwnd, id, |document| {
        already = document.conflict == Some(kind);
        document.conflict = Some(kind);
        document.autosave_paused = true;
        if kind == ConflictKind::Deleted {
            document.deleted_on_disk = true;
        }
    });
    if already {
        return;
    }
    let name = title::note_title(path);
    push_notice(
        hwnd,
        match kind {
            ConflictKind::Changed => format!(
                "{name} changed on disk while you had unsaved edits. Use Note: Reload from disk, Note: Keep my version or Note: Compare with disk."
            ),
            ConflictKind::Deleted => format!(
                "{name} was deleted on disk while you had unsaved edits. Use Note: Keep my version to save it again."
            ),
        },
    );
}

/// Changes tab `id`'s document and refreshes the strips, whose titles may follow.
fn update(hwnd: HWND, id: DocumentId, f: impl FnOnce(&mut crate::document::Document)) {
    let Some(mut app) = (unsafe { app_ptr(hwnd) }) else {
        return;
    };
    let tabs = &mut unsafe { app.as_mut() }.tabs;
    let Some(document) = tabs.document_mut(id) else {
        return;
    };
    f(document);
    tabs.refresh_views();
    invalidate_title_strip(hwnd);
}

/// Checks every text tab with a file: when the window comes back to the front, and after a
/// watcher overflow.
pub(crate) fn check_all_tabs(hwnd: HWND) {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    let ids = unsafe { app_ptr(hwnd) }
        .map(|app| {
            unsafe { app.as_ref() }
                .tabs
                .documents()
                .filter(|document| document.path.is_some() && !document.is_image())
                .map(|document| document.id)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    for id in ids {
        if !identity.is_live_for(hwnd) {
            return;
        }
        check_tab(hwnd, id);
    }
}

/// Checks the tabs open on any of `paths`, compared without regard to case.
pub(crate) fn check_paths(hwnd: HWND, paths: &[PathBuf]) {
    let ids = unsafe { app_ptr(hwnd) }
        .map(|app| {
            unsafe { app.as_ref() }
                .tabs
                .documents()
                .filter(|document| {
                    document.path.as_deref().is_some_and(|open| {
                        paths
                            .iter()
                            .any(|path| crate::library::model::same_path(open, path))
                    })
                })
                .map(|document| document.id)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    for id in ids {
        check_tab(hwnd, id);
    }
}

/// Note: Reload from disk: the active tab takes its file's text, as one undoable edit, whether
/// or not it has unsaved edits.
pub(crate) fn reload_from_disk(hwnd: HWND) {
    let Some((id, path)) = unsafe { app_ptr(hwnd) }.and_then(|app| {
        let active = unsafe { app.as_ref() }.tabs.active()?;
        Some((active.id, active.path.clone()?))
    }) else {
        return;
    };
    // Read before the load: a change that lands during it is still noticed next time.
    let stamp = library::disk_stamp(&path);
    match load_text(&path) {
        Ok(file) => {
            if take_disk_text(hwnd, id, &file, stamp) {
                crate::window::main_window::remove_saved_document_snapshots(hwnd);
            }
        }
        Err(error) => crate::window::main_window::report_open_failure(hwnd, &path, &error),
    }
}
