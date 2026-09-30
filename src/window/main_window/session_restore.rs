//! Restoring the last session at startup, one entry per pass, and recovering crash snapshots.

use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RestoreStep {
    Continue,
    Done,
}

/// One `WM_FASTPAD_RESTORE_SESSION` pass. The first pass takes the manifest. Each pass reopens
/// at most one entry, and the pass that finds none left finishes the restore.
pub(super) fn restore_session_step(hwnd: HWND) -> RestoreStep {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return RestoreStep::Done;
    };
    if file_population_active(hwnd) {
        return RestoreStep::Continue;
    }
    let started = unsafe { app_ptr(hwnd) }
        .is_some_and(|app| unsafe { app.as_ref() }.session_restore.is_some());
    if !started && !begin_session_restore(hwnd) {
        return RestoreStep::Done;
    }
    let next = unsafe { app_ptr(hwnd) }.and_then(|app| {
        let restore = unsafe { app.as_ref() }.session_restore.as_ref()?;
        let (index, entry) = restore.next_entry()?;
        // A group whose window could not be made reopens its entries in the first group.
        let group = restore.groups[index].id.or(restore.groups.first()?.id)?;
        Some((group, entry.clone()))
    });
    let Some((group, entry)) = next else {
        finish_session_restore(hwnd);
        return RestoreStep::Done;
    };
    activate_group(hwnd, group);
    let restored = restore_session_entry(hwnd, group, &entry);
    if !identity.is_live_for(hwnd) {
        return RestoreStep::Done;
    }
    if let Some(mut app) = unsafe { app_ptr(hwnd) }
        && let Some(restore) = unsafe { app.as_mut() }.session_restore.as_mut()
    {
        restore.record(restored);
    }
    RestoreStep::Continue
}

/// Takes the manifest, deleting it so a crash from here on is recovery's alone, and remembers
/// the empty startup tab so it can be closed. False when there is nothing to restore.
fn begin_session_restore(hwnd: HWND) -> bool {
    let Some(path) = session_path(hwnd) else {
        // With the setting off this primary never restores the manifest, and its snapshots come
        // back through crash recovery instead. Left in place, it would reopen them a second time
        // once the setting is turned back on, and keep hiding them from other windows' recovery.
        if let Some(stale) = disabled_session_path(hwnd) {
            crate::session::remove(&stale);
        }
        return false;
    };
    let Some(session) = crate::session::read(&path) else {
        return false;
    };
    crate::session::remove(&path);
    if session.is_empty() {
        return false;
    }
    let placeholder = empty_startup_tab(hwnd);
    // Restored snapshots are rewritten under this process's IDs, which only count as live while
    // it holds the owner mutex. `load_settings` has normally created it already.
    ensure_recovery_owner(hwnd);
    let Some(mut app) = (unsafe { app_ptr(hwnd) }) else {
        return false;
    };
    unsafe { app.as_mut() }.session_restore =
        Some(crate::session::SessionRestore::new(&session, placeholder));
    set_up_restored_groups(hwnd);
    bind_ipc_for_restore(hwnd);
    true
}

/// Makes a group window for every saved group after the first, which is the group already open,
/// and arranges them as the manifest's layout (split editors spec §8). A group whose window can't
/// be made has its entries reopened in the first group (spec §9).
fn set_up_restored_groups(hwnd: HWND) {
    let Some((first, saved)) = unsafe { app_ptr(hwnd) }.and_then(|app| {
        let app = unsafe { app.as_ref() };
        let restore = app.session_restore.as_ref()?;
        let numbers = restore
            .groups
            .iter()
            .map(|group| group.number)
            .collect::<Vec<_>>();
        Some((app.tabs.active_group(), (numbers, restore.layout.clone())))
    }) else {
        return;
    };
    let (numbers, layout) = saved;
    let mut ids = vec![Some(first)];
    for _ in 1..numbers.len() {
        ids.push(create_group(hwnd).ok());
    }
    // A failed group leaves the layout; its share goes to its neighbours.
    let layout = numbers
        .iter()
        .zip(&ids)
        .filter(|(_, id)| id.is_none())
        .try_fold(layout, |layout, (number, _)| layout.without(*number));
    let group_of = |number: usize| {
        numbers
            .iter()
            .position(|saved| *saved == number)
            .and_then(|index| ids[index])
    };
    let made = ids.iter().flatten().copied().collect::<Vec<_>>();
    let tree = layout
        .and_then(|layout| crate::window::split_tree::SplitTree::from_session(&layout, &group_of))
        .filter(|tree| tree.leaves().len() == made.len())
        .unwrap_or_else(|| {
            // A layout that doesn't name exactly the groups made: a row of them in order.
            let mut tree = crate::window::split_tree::SplitTree::new(first);
            for pair in made.windows(2) {
                tree.split(
                    pair[0],
                    crate::window::split_tree::Direction::Right,
                    pair[1],
                );
            }
            tree
        });
    if let Some(mut app) = unsafe { app_ptr(hwnd) } {
        let app = unsafe { app.as_mut() };
        app.layout = tree;
        if let Some(restore) = app.session_restore.as_mut() {
            for (group, id) in restore.groups.iter_mut().zip(&ids) {
                group.id = *id;
            }
        }
    }
    layout_editor_and_find_bar(hwnd);
}

/// A launch made while a long session is reopening must reach this window, not time out and
/// open a separate one. `handle_ipc_requests` holds what it forwards until the restore is done.
/// Tests never bind the real single-instance pipe.
fn bind_ipc_for_restore(hwnd: HWND) {
    #[cfg(not(test))]
    start_ipc_server_with(hwnd, crate::ipc::bind_session_server);
    #[cfg(test)]
    let _ = hwnd;
}

/// The active tab when it is still the empty, untouched untitled tab every launch starts with.
pub(super) fn empty_startup_tab(hwnd: HWND) -> Option<DocumentId> {
    use crate::editor::scintilla_constants::SCI_GETLENGTH;
    let app = unsafe { app_ptr(hwnd) }?;
    let app = unsafe { app.as_ref() };
    let active = app.tabs.active().filter(|document| {
        !document.dirty && document.path.is_none() && document.recovery_origin.is_none()
    })?;
    let editor = app.editor()?;
    let empty = unsafe { SendMessageW(editor.hwnd(), SCI_GETLENGTH, 0, 0) } == 0;
    empty.then_some(active.id)
}

/// Reopens one manifest entry in `group`, the active group, and applies its language while it is
/// the active tab. Returns its document, or `None` when the entry could not be reopened. A file
/// or snapshot already reopened for another group becomes a second view of the same document.
pub(super) fn restore_session_entry(
    hwnd: HWND,
    group: GroupId,
    entry: &crate::session::SessionEntry,
) -> Option<DocumentId> {
    match &entry.source {
        // An open file gets a view in the active group (split editors spec §5.3).
        crate::session::SessionSource::File(path) => open_path(hwnd, path).ok()?,
        crate::session::SessionSource::Snapshot(id) => {
            let reopened = unsafe { app_ptr(hwnd) }.and_then(|app| {
                let app = unsafe { app.as_ref() };
                let (_, document) = app
                    .session_restore
                    .as_ref()?
                    .snapshots
                    .iter()
                    .find(|(snapshot, _)| snapshot == id)?;
                app.tabs.document(*document).map(|_| *document)
            });
            match reopened {
                Some(document) => {
                    if !open_in_active_group(hwnd, document) {
                        return None;
                    }
                }
                None => {
                    let identity = unsafe { window_identity(hwnd) }?;
                    let root = recovery_root(hwnd)?;
                    let path = crate::recovery::snapshot::snapshot_path(&root, *id);
                    let snapshot =
                        crate::recovery::Snapshot::decode(&std::fs::read(&path).ok()?).ok()?;
                    let candidate = crate::recovery::SnapshotCandidate { path, snapshot };
                    open_snapshot_tab(hwnd, &identity, candidate, SnapshotTab::Session).ok()?;
                    // Recorded before the adoption below removes the file the id names.
                    if let Some(mut app) = unsafe { app_ptr(hwnd) } {
                        let app = unsafe { app.as_mut() };
                        let document = app.tabs.active().map(|document| document.id);
                        if let (Some(document), Some(restore)) =
                            (document, app.session_restore.as_mut())
                        {
                            restore.snapshots.push((*id, document));
                        }
                    }
                    adopt_restored_snapshot(hwnd, &identity, &root);
                }
            }
        }
    }
    apply_detected_language(hwnd);
    let (id, text) = unsafe { app_ptr(hwnd) }.and_then(|app| {
        let active = unsafe { app.as_ref() }.tabs.active()?;
        Some((active.id, !active.is_image()))
    })?;
    // Every tab lands where it was, not only the active one: the next entry's open records this
    // position for the tab as it takes the editor over (split editors spec §8).
    if text {
        apply_view_state(hwnd, group, entry);
    }
    Some(id)
}

/// The snapshot a session tab was just restored from belongs to the previous, exited process, so
/// every other FastPad process would take it for a crash leftover and offer it as "Recovered".
/// Rewriting the text under the tab's own ID, owned by this live process, and then removing the
/// source closes that gap. A failed write keeps the source, so the text is always in some file.
fn adopt_restored_snapshot(hwnd: HWND, identity: &WindowIdentity, root: &std::path::Path) {
    let job = unsafe { app_ptr(hwnd) }.and_then(|app| {
        let app = unsafe { app.as_ref() };
        let editor = app.editor().cloned()?;
        let document = app.tabs.active()?;
        let origin = document.recovery_origin.as_ref()?;
        Some((
            editor,
            document.id,
            document.generation,
            document.recovery_id,
            document
                .path
                .clone()
                .or_else(|| origin.original_path.clone()),
            document.encoding,
            origin.snapshot_path.clone(),
        ))
    });
    let Some((editor, id, generation, recovery_id, original_path, encoding, source)) = job else {
        return;
    };
    let Ok(text) = editor.text() else {
        return;
    };
    if !identity.is_live_for(hwnd) {
        return;
    }
    let snapshot = crate::recovery::Snapshot::new(recovery_id, original_path, encoding, text);
    let Ok(written) = crate::recovery::write_snapshot(root, &snapshot) else {
        return;
    };
    if let Some(mut app) = unsafe { app_ptr(hwnd) } {
        unsafe { app.as_mut() }
            .tabs
            .record_recovery_generation(id, generation);
    }
    if written != source {
        crate::recovery::remove_snapshot_files(&[source]);
    }
}

/// Closes the empty startup tab once something replaced it, shows the saved active tab with its
/// caret and scroll position, and reports every entry that failed in one notice.
pub(super) fn finish_session_restore(hwnd: HWND) {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    let Some(restore) =
        unsafe { app_ptr(hwnd) }.and_then(|mut app| unsafe { app.as_mut() }.session_restore.take())
    else {
        return;
    };
    let restored_any = restore.restored_any();
    if let Some(placeholder) = restore.placeholder
        && restored_any
        && still_empty_untitled(hwnd, placeholder)
        && activate_document_by_id(hwnd, placeholder)
        && identity.is_live_for(hwnd)
    {
        close_active_document(hwnd);
    }
    if !identity.is_live_for(hwnd) {
        return;
    }
    // Each group shows its saved active view where it was left.
    for (index, group) in restore.groups.iter().enumerate() {
        let Some(id) = group.id else {
            continue;
        };
        if let Some(active) = restore.active_view(index)
            && focus_view(hwnd, id, active)
            && identity.is_live_for(hwnd)
            && restore.saved_active_restored(index) == Some(active)
        {
            apply_view_state(hwnd, id, &group.entries[group.active]);
        }
        if !identity.is_live_for(hwnd) {
            return;
        }
    }
    // A group none of whose entries came back closes, unless it is the only one.
    for id in group_order(hwnd) {
        remove_empty_group(hwnd, id);
    }
    let active = restore
        .groups
        .get(restore.active_group)
        .and_then(|group| group.id)
        .filter(|id| group_order(hwnd).contains(id))
        .or_else(|| group_order(hwnd).first().copied());
    if let Some(active) = active {
        activate_group(hwnd, active);
    }
    layout_editor_and_find_bar(hwnd);
    refresh_tabs(hwnd);
    if !identity.is_live_for(hwnd) {
        return;
    }
    // Each restored tab entered the activation order at the front as it opened. Restart it from
    // the strip, the active tab first (quick-open spec §3.2).
    if let Some(mut app) = unsafe { app_ptr(hwnd) } {
        unsafe { app.as_mut() }.tabs.reset_activation_order();
    }
    if restore.failed > 0 {
        push_notice(hwnd, crate::session::restore_failure_notice(restore.failed));
    }
    // Launches forwarded during the restore were held in the queue. They open now, after every
    // restored tab, so the file the user just asked for ends up active.
    unsafe {
        PostMessageW(hwnd, crate::window::WM_FASTPAD_IPC_REQUEST, 0, 0);
    }
}

/// A reused startup tab now has a path, and a typed-in one is dirty. Neither may be closed.
fn still_empty_untitled(hwnd: HWND, id: DocumentId) -> bool {
    unsafe { app_ptr(hwnd) }.is_some_and(|app| {
        unsafe { app.as_ref() }
            .tabs
            .document(id)
            .is_some_and(|document| {
                !document.dirty && document.path.is_none() && document.recovery_origin.is_none()
            })
    })
}

/// Puts group `group`'s editor where `entry` was saved.
pub(super) fn apply_view_state(hwnd: HWND, group: GroupId, entry: &crate::session::SessionEntry) {
    let Some(editor) = group_editor(hwnd, group) else {
        return;
    };
    // `apply_view_state` clamps: the file may have shrunk since the session was saved.
    let _ = editor.apply_view_state(crate::editor::ViewState {
        caret: entry.caret,
        anchor: entry.anchor,
        first_line: entry.first_line,
        x_offset: 0,
    });
}

/// Snapshot files a saved `session.ini` still names. They are waiting for the primary window's
/// next restore, not left by a crash, so no window recovers them while session restore is on,
/// primary or not. With it off they come back through crash recovery like any other.
fn saved_session_snapshots(hwnd: HWND, root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let enabled = unsafe { app_ptr(hwnd) }
        .is_some_and(|app| unsafe { app.as_ref() }.settings.restore_session);
    let Some(session) = enabled
        .then(|| session_manifest_path(hwnd))
        .flatten()
        .and_then(|path| crate::session::read(&path))
    else {
        return Vec::new();
    };
    session
        .groups
        .iter()
        .flat_map(|group| &group.entries)
        .filter_map(|entry| match entry.source {
            crate::session::SessionSource::Snapshot(id) => {
                Some(crate::recovery::snapshot::snapshot_path(root, id))
            }
            crate::session::SessionSource::File(_) => None,
        })
        .collect()
}

/// Opens every valid foreign snapshot as a recovered tab and reports them with one notice.
pub(super) fn recover_snapshots(hwnd: HWND) {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    let Some(root) = recovery_root(hwnd) else {
        return;
    };
    // Every open posts the language unit, which continues into this one. While a session
    // restore is still reopening entries it owns their snapshots. Recovery runs again after
    // `WM_FASTPAD_OPEN_REQUEST`, which always posts the language unit.
    if unsafe { app_ptr(hwnd) }.is_some_and(|app| unsafe { app.as_ref() }.session_restore.is_some())
    {
        return;
    }
    let Ok(candidates) =
        crate::recovery::discover_snapshots_with(&root, crate::recovery::owner_is_alive)
    else {
        return;
    };
    let saved = saved_session_snapshots(hwnd, &root);
    let mut recovered = 0;
    for candidate in candidates {
        if saved.contains(&candidate.path) {
            continue;
        }
        // This process's own snapshots, and ones an open tab was already recovered or restored
        // from, are held by a live tab rather than left behind by a crash.
        let claimed = unsafe { app_ptr(hwnd) }.is_none_or(|app| {
            let app = unsafe { app.as_ref() };
            app.owns_recovery_id(candidate.snapshot.recovery_id)
                || app.tabs.documents().any(|document| {
                    document
                        .recovery_origin
                        .as_ref()
                        .is_some_and(|origin| origin.snapshot_path == candidate.path)
                })
        });
        if claimed {
            continue;
        }
        if open_snapshot_tab(hwnd, &identity, candidate, SnapshotTab::Recovered).is_ok() {
            recovered += 1;
        }
        if !identity.is_live_for(hwnd) {
            return;
        }
    }
    if recovered == 0 {
        return;
    }
    let chrome_built = unsafe { app_ptr(hwnd) }.is_some_and(|mut app| {
        let app = unsafe { app.as_mut() };
        app.notifications
            .push(crate::recovery::recovered_notice(recovered));
        app.status.is_some()
    });
    if chrome_built {
        layout_editor_and_find_bar(hwnd);
    }
    unsafe {
        InvalidateRect(hwnd, std::ptr::null(), 1);
    }
}
