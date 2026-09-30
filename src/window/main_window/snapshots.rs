//! Recovery snapshots: the idle snapshot timer, writing snapshots, reopening them as tabs and
//! removing them.

use super::*;

pub(super) fn start_recovery_timer(hwnd: HWND) {
    ensure_recovery_owner(hwnd);
    let Some(interval) = (unsafe { app_ptr(hwnd) })
        .map(|app| unsafe { app.as_ref() }.settings.recovery_interval_seconds)
    else {
        return;
    };
    unsafe {
        SetTimer(
            hwnd,
            crate::recovery::RECOVERY_TIMER_ID,
            crate::recovery::timer_period_ms(interval),
            None,
        );
    }
}

/// Holds the named mutex that tells other FastPad processes this one's snapshots are live, not
/// crash leftovers. Creating it again is a no-op once held.
pub(super) fn ensure_recovery_owner(hwnd: HWND) {
    if let Some(mut app) = unsafe { app_ptr(hwnd) } {
        let app = unsafe { app.as_mut() };
        if app.recovery_owner.is_none() {
            app.recovery_owner = crate::recovery::create_owner_mutex(app.recovery_owner_id()).ok();
        }
    }
}

/// Resolves (once) and returns the Recovery directory; tests pre-seed `App::recovery_root`.
pub(super) fn recovery_root(hwnd: HWND) -> Option<std::path::PathBuf> {
    let mut app = unsafe { app_ptr(hwnd) }?;
    let app = unsafe { app.as_mut() };
    if app.recovery_root.is_none() {
        app.recovery_root = crate::recovery::recovery_root().ok();
    }
    app.recovery_root.clone()
}

pub(super) fn snapshot_when_idle(hwnd: HWND) {
    let mut info = LASTINPUTINFO {
        cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
        dwTime: 0,
    };
    if unsafe { GetLastInputInfo(&mut info) } == 0
        || !crate::recovery::input_idle(info.dwTime, unsafe { GetTickCount() })
    {
        return;
    }
    snapshot_next_document(hwnd);
}

/// Writes at most one dirty document whose generation has not been recorded yet.
pub(super) fn snapshot_next_document(hwnd: HWND) {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    if file_population_active(hwnd) || crate::window::modal::modal_active(hwnd) {
        return;
    }
    let Some(root) = recovery_root(hwnd) else {
        return;
    };
    let job = unsafe { app_ptr(hwnd) }.and_then(|app| {
        let app = unsafe { app.as_ref() };
        let editor = app.editor().cloned()?;
        let active = app.tabs.active()?;
        let document = crate::recovery::next_snapshot_document(
            app.tabs.documents(),
            app.last_snapshot_attempt,
        )?;
        let origin = document.recovery_origin.as_ref();
        let inactive = if document.id == active.id {
            None
        } else {
            Some(document.text_handle()?.clone())
        };
        Some(SnapshotJob {
            editor,
            id: document.id,
            generation: document.generation,
            recovery_id: document.recovery_id,
            original_path: document
                .path
                .clone()
                .or_else(|| origin.and_then(|origin| origin.original_path.clone())),
            encoding: document.encoding,
            source_snapshot: origin.map(|origin| origin.snapshot_path.clone()),
            inactive,
        })
    });
    let Some(job) = job else {
        return;
    };
    // Recorded before the write so a document that keeps failing still yields the next tick.
    if let Some(mut app) = unsafe { app_ptr(hwnd) } {
        unsafe { app.as_mut() }.last_snapshot_attempt = Some(job.id);
    }
    let started = std::time::Instant::now();
    let text = match &job.inactive {
        None => job.editor.text(),
        Some(target) => with_background_document(hwnd, target, Editor::text),
    };
    let Ok(text) = text else {
        return;
    };
    if !identity.is_live_for(hwnd) {
        return;
    }
    let snapshot =
        crate::recovery::Snapshot::new(job.recovery_id, job.original_path, job.encoding, text);
    let written = crate::recovery::write_snapshot(&root, &snapshot);
    drop(snapshot);
    let elapsed = started.elapsed();
    if let Some(mut app) = unsafe { app_ptr(hwnd) } {
        let app = unsafe { app.as_mut() };
        app.last_snapshot_duration = Some(elapsed);
        if written.is_ok() {
            app.tabs.record_recovery_generation(job.id, job.generation);
        }
    }
    // Once a recovered tab has its own snapshot, its source would only resurrect a stale duplicate.
    if let (Ok(written), Some(source)) = (written, job.source_snapshot)
        && written != source
    {
        crate::recovery::remove_snapshot_files(&[source]);
    }
}

struct SnapshotJob {
    editor: Editor,
    id: DocumentId,
    generation: u64,
    recovery_id: RecoveryId,
    original_path: Option<std::path::PathBuf>,
    encoding: crate::file::encoding::Encoding,
    source_snapshot: Option<std::path::PathBuf>,
    /// A background tab's document, read through the document host.
    inactive: Option<crate::editor::EditorDocument>,
}

/// How a snapshot comes back as a tab: after a crash (untitled, titled "Recovered: ...") or from
/// the last session (bound to its file again, so Ctrl+S saves where it came from).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SnapshotTab {
    Recovered,
    Session,
}

pub(super) fn open_snapshot_tab(
    hwnd: HWND,
    identity: &WindowIdentity,
    candidate: crate::recovery::SnapshotCandidate,
    kind: SnapshotTab,
) -> Result<()> {
    if file_population_active(hwnd) {
        return Err(crate::FastPadError::Invariant(
            "file population is already active",
        ));
    }
    let crate::recovery::SnapshotCandidate { path, snapshot } = candidate;
    let from_session = kind == SnapshotTab::Session;
    // A session tab edits its file again, unless another tab already has that file open.
    let bound_path = snapshot.original_path.clone().filter(|original| {
        from_session
            && unsafe { app_ptr(hwnd) }
                .is_some_and(|app| unsafe { app.as_ref() }.tabs.find_path(original).is_none())
    });
    let (editor, id, recovery_id) = {
        let mut app = unsafe { app_ptr(hwnd) }.ok_or(crate::FastPadError::Invariant(
            "main window app state was not available",
        ))?;
        let app = unsafe { app.as_mut() };
        let editor = app
            .editor()
            .cloned()
            .ok_or(crate::FastPadError::Invariant("editor was not initialized"))?;
        let (id, recovery_id) = app.allocate_document_identity();
        (editor, id, recovery_id)
    };
    remember_active_view(hwnd);
    let previous = editor.current_document()?;
    let mut document = Document::untitled(id, recovery_id, editor.create_document()?);
    document.path = bound_path;
    document.encoding = snapshot.encoding;
    document.dirty = true;
    document.recovery_generation = Some(document.generation);
    document.recovery_origin = Some(crate::document::RecoveryOrigin {
        snapshot_path: path,
        original_path: snapshot.original_path,
        from_session,
    });
    // Only the active tab's label follows its edits, so a background tab gets its label here.
    crate::window::library_host::label_restored_document(hwnd, &mut document, &snapshot.text);
    if !identity.is_live_for(hwnd) {
        return Err(crate::FastPadError::Invariant(
            "main window was destroyed during recovery",
        ));
    }
    set_file_population(hwnd, true);
    // Undo collection stays on so the loaded text leaves the save point: the tab starts dirty.
    let result = document
        .expect_text()
        .and_then(|handle| editor.use_document(handle))
        .and_then(|_| editor.set_text(&snapshot.text));
    drop(snapshot.text);
    if result.is_err() && identity.is_live_for(hwnd) {
        let _ = editor.use_document(&previous);
    }
    if !identity.is_live_for(hwnd) {
        return Err(crate::FastPadError::Invariant(
            "main window was destroyed during recovery",
        ));
    }
    set_file_population(hwnd, false);
    result?;
    let pushed = unsafe { app_ptr(hwnd) }
        .ok_or(crate::FastPadError::Invariant(
            "main window app state was not available",
        ))
        .and_then(|mut app| {
            unsafe { app.as_mut() }
                .tabs
                .push(document)
                .map_err(|_| crate::FastPadError::Invariant("duplicate document path"))
        });
    if pushed.is_err() {
        let _ = editor.use_document(&previous);
    }
    pushed?;
    refresh_tabs(hwnd);
    Ok(())
}

/// A successful save supersedes both the document's own snapshot and any recovery source.
pub(in crate::window) fn remove_saved_document_snapshots(hwnd: HWND) {
    let files = unsafe { app_ptr(hwnd) }.map(|mut app| {
        let app = unsafe { app.as_mut() };
        let own = app.recovery_root.as_deref().map(|root| {
            let active = app.tabs.active()?;
            Some(crate::recovery::snapshot::snapshot_path(
                root,
                active.recovery_id,
            ))
        });
        let source = app
            .tabs
            .take_active_recovery_origin()
            .map(|origin| origin.snapshot_path);
        own.flatten().into_iter().chain(source).collect::<Vec<_>>()
    });
    crate::recovery::remove_snapshot_files(&files.unwrap_or_default());
}

pub(super) fn remove_session_snapshots(hwnd: HWND, discarded: &[DocumentId]) {
    let files = unsafe { app_ptr(hwnd) }.and_then(|app| {
        let app = unsafe { app.as_ref() };
        let root = app.recovery_root.as_deref()?;
        Some(
            app.tabs
                .documents()
                .flat_map(|document| {
                    crate::recovery::snapshots_removed_on_close(
                        root,
                        document,
                        discarded.contains(&document.id),
                    )
                })
                .collect::<Vec<_>>(),
        )
    });
    crate::recovery::remove_snapshot_files(&files.unwrap_or_default());
}
