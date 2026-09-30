//! Where the session lives, and saving it when the window closes.

use super::*;

/// Where this window keeps its session, or `None` when it does not take part: session restore
/// is off, or this is not the primary instance. Tests never resolve the real path.
pub(super) fn session_path(hwnd: HWND) -> Option<std::path::PathBuf> {
    let app = unsafe { app_ptr(hwnd) }?;
    let app = unsafe { app.as_ref() };
    if !app.settings.restore_session || app.instance_mutex.is_none() {
        return None;
    }
    session_manifest_path(hwnd)
}

/// The primary window's manifest when session restore is off, so a stale one can be deleted.
pub(super) fn disabled_session_path(hwnd: HWND) -> Option<std::path::PathBuf> {
    let app = unsafe { app_ptr(hwnd) }?;
    let app = unsafe { app.as_ref() };
    if app.settings.restore_session || app.instance_mutex.is_none() {
        return None;
    }
    session_manifest_path(hwnd)
}

/// Where the manifest lives, whichever window asks. Only `session_path` and
/// `disabled_session_path` decide whether this window may change it. Tests never resolve the
/// real path, only a pre-seeded `App::session_path`.
pub(super) fn session_manifest_path(hwnd: HWND) -> Option<std::path::PathBuf> {
    let mut app = unsafe { app_ptr(hwnd) }?;
    let app = unsafe { app.as_mut() };
    #[cfg(not(test))]
    if app.session_path.is_none() {
        app.session_path = crate::session::session_file_path().ok();
    }
    app.session_path.clone()
}

/// With session restore on, records every tab in `session.ini` instead of asking about unsaved
/// changes. False sends the caller to the review prompts: the feature is off, or some unsaved
/// text could not be secured in a snapshot and must not close silently.
pub(super) fn save_session_for_close(hwnd: HWND) -> bool {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return false;
    };
    let Some(path) = session_path(hwnd) else {
        // Turned off during this session: a manifest from an earlier close must not outlive the
        // prompts this close shows instead.
        if let Some(stale) = disabled_session_path(hwnd) {
            crate::session::remove(&stale);
        }
        return false;
    };
    // Mid-restore, the manifest is gone and unrestored entries are in no tab. The review flow
    // keeps their snapshots on disk for recovery instead.
    if unsafe { app_ptr(hwnd) }.is_some_and(|app| unsafe { app.as_ref() }.session_restore.is_some())
    {
        return false;
    }
    let Some(root) = recovery_root(hwnd) else {
        return false;
    };
    let pending = unsafe { app_ptr(hwnd) }.map_or(0, |app| {
        unsafe { app.as_ref() }
            .tabs
            .documents()
            .filter(|document| crate::recovery::needs_snapshot(document))
            .count()
    });
    // Each call writes the next document still needing one, so `pending` calls cover them all.
    for _ in 0..pending {
        snapshot_next_document(hwnd);
        if !identity.is_live_for(hwnd) {
            return false;
        }
    }
    let Some(session) = build_session(hwnd, &root) else {
        return false;
    };
    let written = if session.is_empty() {
        crate::session::remove(&path);
        Ok(())
    } else {
        crate::session::write(&path, &session)
    };
    if written.is_err() {
        return false;
    }
    let files = unsafe { app_ptr(hwnd) }.map(|app| {
        unsafe { app.as_ref() }
            .tabs
            .documents()
            .flat_map(|document| {
                crate::recovery::snapshots_removed_on_session_close(&root, document)
            })
            .collect::<Vec<_>>()
    });
    crate::recovery::remove_snapshot_files(&files.unwrap_or_default());
    true
}

/// The manifest for the open tabs, or `None` when a dirty tab's text is in no snapshot file.
/// Clean untitled tabs are empty and skipped. Every text tab keeps its caret and scroll position:
/// the shown one from the editor, the others from where they were left (split editors spec §8).
///
/// Every group is written in layout order, numbered from 1, with its views in strip order and
/// the layout (split editors spec §8). A document in two groups is written under both with the
/// same source. A group left with no entries is dropped, and the layout with it.
pub(crate) fn build_session(hwnd: HWND, root: &std::path::Path) -> Option<crate::session::Session> {
    use crate::session::{Session, SessionEntry, SessionGroup, SessionLayout, SessionSource};
    let app = unsafe { app_ptr(hwnd) }?;
    let app = unsafe { app.as_ref() };
    let order = app.layout.leaves();
    // `None` for a document with nothing to write (a clean untitled tab); an error (`?`) for a
    // dirty one whose text is in no snapshot.
    let source_of = |document: &Document| -> Option<Option<SessionSource>> {
        if document.dirty {
            let file = crate::recovery::current_snapshot_file(root, document)?;
            let id = crate::recovery::snapshot::snapshot_file_id(&file)?;
            return Some(Some(SessionSource::Snapshot(id)));
        }
        Some(
            document
                .path
                .as_ref()
                .filter(|path| path.to_str().is_some())
                .map(|path| SessionSource::File(path.clone())),
        )
    };
    let mut groups = Vec::new();
    let mut dropped = Vec::new();
    for (position, group) in order.iter().enumerate() {
        let number = position + 1;
        let active_id = app.tabs.group(*group)?.active_document();
        let editor = app.group(*group).map(|state| &state.editor);
        let mut active = 0;
        let mut entries = Vec::new();
        for document in app.tabs.group_documents(*group) {
            let is_active = Some(document.id) == active_id;
            let Some(source) = source_of(document)? else {
                if is_active {
                    active = entries.len().saturating_sub(1);
                }
                continue;
            };
            if is_active {
                active = entries.len();
            }
            let mut entry = SessionEntry::new(source);
            // An image tab has no caret: the editor holds a placeholder then.
            if !document.is_image() {
                let state = if is_active {
                    editor
                        .and_then(|editor| editor.view_state().ok())
                        .unwrap_or_default()
                } else {
                    app.tabs.view_state_in(*group, document.id)
                };
                entry.caret = state.caret;
                entry.anchor = state.anchor;
                entry.first_line = state.first_line;
            }
            entries.push(entry);
        }
        if entries.is_empty() {
            dropped.push(number);
            continue;
        }
        groups.push(SessionGroup {
            number,
            active,
            entries,
        });
    }
    let number_of = |id: GroupId| {
        order
            .iter()
            .position(|group| *group == id)
            .map_or(1, |index| index + 1)
    };
    let layout = dropped
        .iter()
        .try_fold(app.layout.to_session(&number_of), |layout, number| {
            layout.without(*number)
        })
        .unwrap_or(SessionLayout::Leaf(1));
    let active_number = number_of(app.tabs.active_group());
    let active_group = groups
        .iter()
        .position(|group| group.number == active_number)
        .unwrap_or(0);
    Some(Session {
        layout,
        active_group,
        groups,
    })
}
