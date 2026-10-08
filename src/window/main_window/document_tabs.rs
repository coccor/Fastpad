//! Creating, activating, cycling and closing documents and their tabs, including the close
//! review.

use super::*;

pub(crate) fn create_new_document(hwnd: HWND) -> Result<()> {
    let identity = unsafe { window_identity(hwnd) }.ok_or(crate::FastPadError::Invariant(
        "main window app state was not available",
    ))?;
    crate::window::library_host::autosave_active(hwnd);
    if !identity.is_live_for(hwnd) {
        return Err(crate::FastPadError::Invariant(
            "main window was destroyed while creating a document",
        ));
    }
    let (editor, id, recovery_id) = {
        let Some(mut app) = (unsafe { app_ptr(hwnd) }) else {
            return Err(crate::FastPadError::Invariant(
                "main window app state was not available",
            ));
        };
        let app = unsafe { app.as_mut() };
        let editor = app
            .editor()
            .cloned()
            .ok_or(crate::FastPadError::Invariant("editor was not initialized"))?;
        let (id, recovery_id) = app.allocate_document_identity();
        (editor, id, recovery_id)
    };

    remember_active_view(hwnd);
    let document = Document::untitled(id, recovery_id, editor.create_document()?);
    document
        .expect_text()
        .and_then(|handle| editor.use_document(handle))?;
    if !identity.is_live_for(hwnd) {
        return Err(crate::FastPadError::Invariant(
            "main window was destroyed while creating a document",
        ));
    }
    let Some(mut app) = (unsafe { app_ptr(hwnd) }) else {
        return Err(crate::FastPadError::Invariant(
            "main window app state was not available",
        ));
    };
    let app = unsafe { app.as_mut() };
    app.tabs
        .push(document)
        .map_err(|_| crate::FastPadError::Invariant("duplicate document path"))?;
    refresh_tabs(hwnd);
    Ok(())
}

pub(super) fn activate_tab(hwnd: HWND, index: usize) {
    let target = unsafe { app_ptr(hwnd) }.and_then(|app| {
        let view = unsafe { app.as_ref() }.tabs.view().snapshot();
        view.tabs.get(index).map(|tab| (tab.id, view.revision))
    });
    if let Some((id, revision)) = target {
        let _ = activate_document(hwnd, id, revision);
    }
}

/// Activates the tab after (or before) the active one, wrapping around the ends of the strip.
pub(super) fn cycle_tab(hwnd: HWND, forward: bool) {
    let Some(active) =
        (unsafe { app_ptr(hwnd) }).map(|app| unsafe { app.as_ref() }.tabs.active_index())
    else {
        return;
    };
    let count = tab_count(hwnd);
    if count < 2 {
        return;
    }
    let target = if forward {
        (active + 1) % count
    } else {
        (active + count - 1) % count
    };
    activate_tab(hwnd, target);
}

fn activate_document(hwnd: HWND, id: DocumentId, revision: u64) -> bool {
    let Some(group) =
        unsafe { app_ptr(hwnd) }.map(|app| unsafe { app.as_ref() }.tabs.active_group())
    else {
        return false;
    };
    activate_document_in(hwnd, group, id, revision)
}

/// Shows `id`'s view in `group`, provided `group`'s strip is still at `revision`.
pub(crate) fn activate_document_in(
    hwnd: HWND,
    group: GroupId,
    id: DocumentId,
    revision: u64,
) -> bool {
    if file_population_active(hwnd) {
        return false;
    }
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return false;
    };
    let leaving = unsafe { app_ptr(hwnd) }.and_then(|app| {
        let app = unsafe { app.as_ref() };
        let tabs = app.tabs.group(group)?;
        (tabs.view().snapshot().revision == revision).then(|| {
            (
                tabs.active_document() != Some(id),
                app.tabs.active_group() == group,
            )
        })
    });
    let Some((leaving, active)) = leaving else {
        return false;
    };
    // Saving the tab being left bumps the view revision itself, so the caller's revision is
    // checked before it; afterwards `activate_in` still refuses an `id` that has gone.
    if leaving && active {
        crate::window::library_host::autosave_active(hwnd);
        if !identity.is_live_for(hwnd) {
            return false;
        }
    }
    if leaving {
        remember_view(hwnd, group);
    }
    let activated = unsafe { app_ptr(hwnd) }
        .is_some_and(|mut app| unsafe { app.as_mut() }.tabs.activate_in(group, id));
    if !activated || !show_group_view(hwnd, group) || !identity.is_live_for(hwnd) {
        return false;
    }
    if active {
        refresh_tabs(hwnd);
        crate::window::image_host::check_disk(hwnd);
        schedule_find_count(hwnd);
    } else if let Some(window) = with_group_id(hwnd, group, |state| state.hwnd) {
        unsafe { InvalidateRect(window, std::ptr::null(), 0) };
        crate::window::notebook_view::editors_changed(hwnd);
    }
    true
}

/// `remember_view` for the active group.
pub(super) fn remember_active_view(hwnd: HWND) {
    if let Some(group) =
        unsafe { app_ptr(hwnd) }.map(|app| unsafe { app.as_ref() }.tabs.active_group())
    {
        remember_view(hwnd, group);
    }
}

/// Selects a tab for the strip provider of the group window `wparam`.
pub(super) fn handle_accessible_select(hwnd: HWND, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if lparam == 0 {
        return 0;
    }
    let request = unsafe { *(lparam as *const AccessibleSelectRequest) };
    let Some(group) = group_id_of(hwnd, wparam as HWND) else {
        return 0;
    };
    isize::from(activate_document_in(
        hwnd,
        group,
        request.document_id,
        request.revision,
    ))
}

pub(super) fn close_active_document(hwnd: HWND) {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    // Another group still shows the document: only this view closes, without asking (split
    // editors spec §5.4). Its text and dirty state stay with the other view.
    let shared = unsafe { app_ptr(hwnd) }.and_then(|app| {
        let app = unsafe { app.as_ref() };
        let review = app.tabs.active_close_review()?;
        (app.tabs.views_of(review.id).len() > 1).then(|| (review, app.editor().cloned()))
    });
    if let Some((review, Some(editor))) = shared {
        close_reviewed_document(hwnd, &identity, &editor, review, CloseDecision::Discard);
        return;
    }
    // A saved note is clean now and closes without a prompt; paused or failed ones still ask.
    crate::window::library_host::autosave_active(hwnd);
    if !identity.is_live_for(hwnd) {
        return;
    }
    let snapshot = unsafe { app_ptr(hwnd) }.and_then(|app| {
        let app = unsafe { app.as_ref() };
        let review = app.tabs.active_close_review()?;
        let document = app.tabs.document(review.id)?;
        Some((
            review,
            document.dirty,
            document.title(),
            app.editor().cloned(),
        ))
    });
    let Some((review, dirty, title, Some(editor))) = snapshot else {
        return;
    };
    let decision = if dirty {
        prompt_close_decision(hwnd, &title)
    } else {
        CloseDecision::Discard
    };
    if decision == CloseDecision::Cancel || !identity.is_live_for(hwnd) {
        return;
    }
    // Saving clears the dirty flag, which advances the generation the prompt reviewed.
    let review = if decision == CloseDecision::Save {
        if !save_reviewed_document(hwnd, review.id) {
            return;
        }
        match unsafe { app_ptr(hwnd) }
            .and_then(|app| unsafe { app.as_ref() }.tabs.active_close_review())
        {
            Some(saved) if saved.id == review.id => saved,
            _ => return,
        }
    } else {
        review
    };
    close_reviewed_document(hwnd, &identity, &editor, review, decision);
}

/// Closes tab `id` as a middle-click on it does (open editors spec §3.2), if it is still open.
pub(crate) fn close_document_tab(hwnd: HWND, id: DocumentId) {
    let index = unsafe { app_ptr(hwnd) }.and_then(|app| {
        let tabs = &unsafe { app.as_ref() }.tabs;
        tabs.group_documents(tabs.active_group())
            .into_iter()
            .position(|document| document.id == id)
    });
    if let Some(index) = index {
        close_tab_at(hwnd, index);
    }
}

/// Closes the tab at strip `index` (quick-open spec §5). A clean tab that isn't the active one
/// closes where it is, and the active tab stays. Any other tab is activated first, so a save
/// prompt asks about the tab on screen, and is then closed as Close tab closes it.
pub(super) fn close_tab_at(hwnd: HWND, index: usize) {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    let target = unsafe { app_ptr(hwnd) }.and_then(|app| {
        let tabs = &unsafe { app.as_ref() }.tabs;
        let document = *tabs.group_documents(tabs.active_group()).get(index)?;
        let background = tabs.active().is_some_and(|active| active.id != document.id);
        Some((
            crate::window::tabs::CloseReview {
                id: document.id,
                generation: document.generation,
            },
            background && !document.dirty,
        ))
    });
    let Some((review, clean_background)) = target else {
        return;
    };
    if clean_background {
        close_background_document(hwnd, &identity, review);
    } else if activate_document_by_id(hwnd, review.id) && identity.is_live_for(hwnd) {
        execute_command(hwnd, CommandId::CloseTab);
    }
    // A middle-click moves no focus, so the palette can stay open in the QuickOpen picker while
    // a tab behind it closes; its empty-query rows (the open tabs) must drop the closed one.
    refresh_quick_open_after_close(hwnd);
}

/// Rebuilds an open quick-open picker's rows after `close_tab_at` closes a tab, so a tab closed
/// behind the palette does not linger in its "open tabs" rows.
fn refresh_quick_open_after_close(hwnd: HWND) {
    let showing_quick_open = with_command_palette(hwnd, |palette| {
        palette.is_visible()
            && palette
                .picker()
                .is_some_and(|picker| picker.kind == command_palette::PickerKind::QuickOpen)
    })
    .unwrap_or(false);
    if showing_quick_open {
        refilter_command_palette(hwnd);
    }
}

/// The document shown by tab `index` of the strip.
pub(super) fn tab_id_at(hwnd: HWND, index: usize) -> Option<DocumentId> {
    unsafe { app_ptr(hwnd) }.and_then(|app| {
        let tabs = &unsafe { app.as_ref() }.tabs;
        tabs.group_documents(tabs.active_group())
            .get(index)
            .map(|document| document.id)
    })
}

/// Closes `id` without asking, discarding any unsaved edits, e.g. once its file is deleted.
/// Every group's view of `id` closes, each without asking (split editors spec §5.6).
pub(in crate::window) fn close_document_without_prompt(hwnd: HWND, id: DocumentId) {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    let previous = unsafe { app_ptr(hwnd) }.map(|app| unsafe { app.as_ref() }.tabs.active_group());
    close_every_view(hwnd, &identity, id);
    // Closing a view elsewhere activated its group; the one the user was in stays active.
    if let Some(previous) = previous
        && identity.is_live_for(hwnd)
        && with_group_id(hwnd, previous, |_| ()).is_some()
    {
        activate_group(hwnd, previous);
    }
}

fn close_every_view(hwnd: HWND, identity: &WindowIdentity, id: DocumentId) {
    let views = || {
        unsafe { app_ptr(hwnd) }.map_or(0, |app| unsafe { app.as_ref() }.tabs.views_of(id).len())
    };
    while let before @ 1.. = views() {
        if !activate_document_by_id(hwnd, id) || !identity.is_live_for(hwnd) {
            return;
        }
        let reviewed = unsafe { app_ptr(hwnd) }.and_then(|app| {
            let app = unsafe { app.as_ref() };
            Some((app.tabs.active_close_review()?, app.editor().cloned()?))
        });
        let Some((review, editor)) = reviewed else {
            return;
        };
        if review.id != id {
            return;
        }
        close_reviewed_document(hwnd, identity, &editor, review, CloseDecision::Discard);
        if !identity.is_live_for(hwnd) || views() >= before {
            return;
        }
    }
}

/// The close itself, once `decision` is settled: closes the reviewed tab, removes its recovery
/// snapshots and shows whichever tab takes its place.
fn close_reviewed_document(
    hwnd: HWND,
    identity: &WindowIdentity,
    editor: &Editor,
    review: crate::window::tabs::CloseReview,
    decision: CloseDecision,
) {
    let group = unsafe { app_ptr(hwnd) }.map(|app| unsafe { app.as_ref() }.tabs.active_group());
    let switched = unsafe { app_ptr(hwnd) }.and_then(|mut app| {
        let app = unsafe { app.as_mut() };
        // `None` when the document is still shown in another group: it stays open.
        let closed = app.tabs.close_reviewed(review, decision).ok()?;
        let snapshots = app
            .recovery_root
            .as_deref()
            .zip(closed.as_ref())
            .map(|(root, closed)| {
                crate::recovery::snapshots_removed_on_close(
                    root,
                    closed,
                    decision == CloseDecision::Discard,
                )
            })
            .unwrap_or_default();
        let view_state = app
            .tabs
            .active()
            .map(|document| app.tabs.view_state(document.id))
            .unwrap_or_default();
        Some((
            closed,
            app.tabs.active_handle().cloned(),
            view_state,
            snapshots,
        ))
    });
    let Some((closed, active, view_state, snapshots)) = switched else {
        return;
    };
    // The view keeps its own reference to whatever it shows, so the last closed document is
    // swapped for an empty placeholder rather than lingering in the hidden editor.
    match active {
        Some(active) => {
            if editor.use_document(&active).is_ok() {
                let _ = editor.apply_view_state(view_state);
            }
        }
        None => {
            if let Ok(blank) = editor.create_document() {
                let _ = editor.use_document(&blank);
            }
        }
    }
    if let Some(closed) = closed {
        crate::window::markdown_host::forget(hwnd, closed.id);
    }
    crate::recovery::remove_snapshot_files(&snapshots);
    if identity.is_live_for(hwnd) {
        refresh_tabs(hwnd);
        // A group closes with its last tab, unless it is the only one (spec §5.4).
        if let Some(group) = group {
            remove_empty_group(hwnd, group);
        }
    }
}

/// `close_reviewed_document` for a clean tab that isn't active: it closes where it is, its
/// recovery snapshots go, and the editor keeps showing the active tab (no document swap).
fn close_background_document(
    hwnd: HWND,
    identity: &WindowIdentity,
    review: crate::window::tabs::CloseReview,
) {
    let closed = unsafe { app_ptr(hwnd) }.and_then(|mut app| {
        let app = unsafe { app.as_mut() };
        let closed = app.tabs.close_clean_background(review).ok()?;
        let snapshots = app
            .recovery_root
            .as_deref()
            .zip(closed.as_ref())
            .map(|(root, closed)| crate::recovery::snapshots_removed_on_close(root, closed, true))
            .unwrap_or_default();
        Some((closed, snapshots))
    });
    let Some((closed, snapshots)) = closed else {
        return;
    };
    if let Some(closed) = closed {
        crate::window::markdown_host::forget(hwnd, closed.id);
    }
    crate::recovery::remove_snapshot_files(&snapshots);
    if identity.is_live_for(hwnd) {
        refresh_tabs(hwnd);
    }
}

/// Closes tabs one at a time, reviewing each dirty one, until none remain or a close is refused.
/// Close all tabs closes the active group's tabs (split editors plan amendment 10). The group goes
/// with its last tab, which ends the loop, unless it is the only one.
pub(super) fn close_all_documents(hwnd: HWND) {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    let active_group =
        || unsafe { app_ptr(hwnd) }.map(|app| unsafe { app.as_ref() }.tabs.active_group());
    let Some(group) = active_group() else {
        return;
    };
    loop {
        let before = tab_count(hwnd);
        if before == 0 || active_group() != Some(group) {
            return;
        }
        close_active_document(hwnd);
        if !identity.is_live_for(hwnd)
            || (active_group() == Some(group) && tab_count(hwnd) >= before)
        {
            return;
        }
    }
}

/// Makes `id` the active document, or reports false when it no longer exists.
/// Shows open document `id` in the active group: its view there, or a new view when only another
/// group has one (split editors spec §5.3). A second view makes a preview tab normal.
pub(super) fn open_in_active_group(hwnd: HWND, id: DocumentId) -> bool {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return false;
    };
    let target = unsafe { app_ptr(hwnd) }.and_then(|app| {
        let tabs = &unsafe { app.as_ref() }.tabs;
        tabs.document(id)?;
        let group = tabs.active_group();
        let state = tabs.group(group)?;
        Some((group, state.contains(id), state.view().snapshot().revision))
    });
    let Some((group, here, revision)) = target else {
        return false;
    };
    if here {
        return activate_document(hwnd, id, revision);
    }
    // The tab being left saves first, as switching tabs does.
    crate::window::library_host::autosave_active(hwnd);
    if !identity.is_live_for(hwnd) {
        return false;
    }
    remember_view(hwnd, group);
    let added = unsafe { app_ptr(hwnd) }.is_some_and(|mut app| {
        unsafe { app.as_mut() }
            .tabs
            .add_view(group, id, crate::editor::ViewState::default())
    });
    if !added || !show_group_view(hwnd, group) {
        return false;
    }
    refresh_tabs(hwnd);
    crate::window::image_host::check_disk(hwnd);
    true
}

/// Makes `id` the active document, or reports false when it no longer exists: its view in the
/// active group, else its view in the first group in layout order that has one, which becomes
/// the active group.
pub(in crate::window) fn activate_document_by_id(hwnd: HWND, id: DocumentId) -> bool {
    let group = unsafe { app_ptr(hwnd) }.and_then(|app| {
        let app = unsafe { app.as_ref() };
        let active = app.tabs.active_group();
        let views = app.tabs.views_of(id);
        if views.contains(&active) {
            return Some(active);
        }
        app.layout
            .leaves()
            .into_iter()
            .chain(views.iter().copied())
            .find(|group| views.contains(group))
    });
    let Some(group) = group else {
        return false;
    };
    activate_group(hwnd, group);
    let target = unsafe { app_ptr(hwnd) }.and_then(|app| {
        let app = unsafe { app.as_ref() };
        if app.tabs.active().is_some_and(|active| active.id == id) {
            return Some(None);
        }
        app.tabs.document(id)?;
        Some(Some(app.tabs.view().snapshot().revision))
    });
    match target {
        Some(None) => true,
        Some(Some(revision)) => activate_document(hwnd, id, revision),
        None => false,
    }
}
