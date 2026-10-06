//! The deferred startup chain: posting and handling the deferred startup messages, the input
//! queue checks it yields to, input priority, startup milestones, and the editor's creation.

use super::*;

pub(crate) unsafe fn maybe_post_deferred_start(hwnd: HWND, identity: &WindowIdentity) {
    // SAFETY: The caller guarantees `hwnd` is the live FastPad main window. The raw App pointer is
    // used only for the immediate pending-flag transition before posting the deferred message.
    if !identity.is_live_for(hwnd) {
        return;
    }
    let should_post = unsafe { take_deferred_start_pending(hwnd) };
    if should_post {
        // Posted startup steps outrank WM_PAINT, so the tab strip paints now or only after them.
        if let Some(group) = group_hwnd(hwnd) {
            unsafe { windows_sys::Win32::Graphics::Gdi::UpdateWindow(group) };
        }
        unsafe {
            PostMessageW(hwnd, deferred_start_message(), 0, 0);
        }
    }
}

pub(super) fn handle_deferred(hwnd: HWND, action: DeferredAction) -> LRESULT {
    // Only `WM_FASTPAD_OPEN_REQUEST` processed with no input pending produces this action, so the
    // launch file opens on exactly the same input-readiness gate as the rest of the chain. The
    // open posts the language continuation (and records FileLoaded) itself.
    if action == DeferredAction::PostNext(crate::window::WM_FASTPAD_APPLY_LANGUAGE) {
        return handle_open_request(hwnd);
    }
    // Each of these actions is produced only by its own deferred message with no input pending:
    // `PostNext(WM_FASTPAD_RESTORE_SESSION)` by `WM_FASTPAD_LOAD_SETTINGS`, `RecordFullyReady` by
    // `WM_FASTPAD_BUILD_CHROME`. Running them before the milestone keeps the milestone honest.
    if action == DeferredAction::PostNext(crate::window::WM_FASTPAD_RESTORE_SESSION) {
        load_settings(hwnd);
    }
    // Only `WM_FASTPAD_RESTORE_SESSION` processed with no input pending produces this action.
    // Each pass reopens at most one session entry and reposts the unit until none remain.
    if action == DeferredAction::PostNext(crate::window::WM_FASTPAD_OPEN_LIBRARY)
        && restore_session_step(hwnd) == RestoreStep::Continue
    {
        unsafe {
            PostMessageW(hwnd, crate::window::WM_FASTPAD_RESTORE_SESSION, 0, 0);
        }
        return 0;
    }
    // Only `WM_FASTPAD_OPEN_LIBRARY` processed with no input pending produces this action.
    if action == DeferredAction::PostNext(crate::window::WM_FASTPAD_OPEN_REQUEST) {
        crate::window::library_host::open_library_step(hwnd);
    }
    if action == DeferredAction::RecordFullyReady {
        build_chrome(hwnd);
    }
    if let Some(milestone) = completed_milestone(action) {
        unsafe {
            let _ = record_milestone(hwnd, milestone);
        }
    }
    // `WM_FASTPAD_APPLY_LANGUAGE` is the only message that classifies into
    // `PostNext(WM_FASTPAD_RECOVERY)`, so this is exactly the point where that message has just
    // been processed (input was not pending). It serves double duty: advancing the deferred
    // startup chain (above/below) and, here, detecting and applying the active document's
    // language after every successful Open/Save As/launch load that posts it.
    if action == DeferredAction::PostNext(crate::window::WM_FASTPAD_RECOVERY) {
        apply_detected_language(hwnd);
        // A tab reopened mid-restore must not start recovery, IPC and chrome ahead of the rest of
        // the session. After the restore, `WM_FASTPAD_OPEN_REQUEST` always posts
        // `WM_FASTPAD_APPLY_LANGUAGE` again, so the chain resumes in order from there.
        if unsafe { app_ptr(hwnd) }
            .is_some_and(|app| unsafe { app.as_ref() }.session_restore.is_some())
        {
            return 0;
        }
    }
    // Only `WM_FASTPAD_RECOVERY` processed with no input pending produces this action.
    if action == DeferredAction::PostNext(crate::window::WM_FASTPAD_START_IPC) {
        recover_snapshots(hwnd);
    }
    // Only `WM_FASTPAD_START_IPC` processed with no input pending produces this action. A session
    // restore has already bound the pipe, and binding again is a no-op.
    if action == DeferredAction::PostNext(crate::window::WM_FASTPAD_BUILD_CHROME) {
        start_ipc_server_with(hwnd, crate::ipc::bind_session_server);
    }
    match action {
        DeferredAction::RepostSelf(message) => {
            unsafe {
                request_input_priority(hwnd);
            }
            unsafe {
                PostMessageW(hwnd, message, 0, 0);
            }
            0
        }
        DeferredAction::PostNext(message) => unsafe {
            PostMessageW(hwnd, message, 0, 0);
            0
        },
        DeferredAction::RecordFullyReady => 0,
    }
}

pub(crate) fn input_pending() -> bool {
    const STATUS_SHIFT: u32 = 16;
    let queue_status = input_queue_status_mask();
    let pending = unsafe {
        ((windows_sys::Win32::UI::WindowsAndMessaging::GetQueueStatus(queue_status)
            >> STATUS_SHIFT)
            & queue_status)
            != 0
    };
    #[cfg(test)]
    if pending && queue_status != QS_INPUT {
        let mut message = MSG::default();
        let found = unsafe {
            PeekMessageW(
                &mut message,
                std::ptr::null_mut(),
                INPUT_MESSAGE_FIRST,
                INPUT_MESSAGE_LAST,
                PM_NOREMOVE | (queue_status << STATUS_SHIFT),
            )
        } != 0;
        return found && message.message != WM_QUIT;
    }
    pending
}

pub(crate) fn input_queue_status_mask() -> u32 {
    #[cfg(test)]
    {
        TEST_INPUT_QUEUE_STATUS.with(Cell::get)
    }
    #[cfg(not(test))]
    {
        QS_INPUT
    }
}

#[cfg(test)]
thread_local! {
    pub(super) static TEST_INPUT_QUEUE_STATUS: Cell<u32> = const { Cell::new(QS_INPUT) };
}

#[cfg(test)]
pub(crate) fn with_test_input_queue_status<R>(queue_status: u32, run: impl FnOnce() -> R) -> R {
    struct ResetQueueStatus(u32);

    impl Drop for ResetQueueStatus {
        fn drop(&mut self) {
            TEST_INPUT_QUEUE_STATUS.with(|status| status.set(self.0));
        }
    }

    TEST_INPUT_QUEUE_STATUS.with(|status| {
        let reset = ResetQueueStatus(status.replace(queue_status));
        let result = run();
        drop(reset);
        result
    })
}

pub(super) unsafe fn take_app(hwnd: HWND) -> Option<Box<App>> {
    let raw = unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) as *mut App };
    (!raw.is_null()).then(|| unsafe { Box::from_raw(raw) })
}

pub(crate) unsafe fn initialize_editor_with<F>(
    hwnd: HWND,
    identity: &WindowIdentity,
    create_editor: F,
) -> Result<HWND>
where
    F: FnOnce(HWND) -> Result<Editor>,
{
    // SAFETY: The caller guarantees `hwnd` is the live FastPad main window whose `GWLP_USERDATA`
    // owns an App. No App reference is held across the reentrant editor creation callback.
    if !identity.is_live_for(hwnd) {
        return Err(crate::FastPadError::Invariant(
            "main window identity was not live during editor initialization",
        ));
    }
    unsafe {
        record_milestone(hwnd, Milestone::WindowCreated)?;
    }

    // Every document comes from the host, so any editor can show any of them (split editors
    // spec §3.1).
    let host = Editor::create_document_host()?;
    // The editor, find bar, preview and image view live in the editor group (split editors spec
    // §4.2), which the main window lays out.
    let group = crate::window::editor_group::create(hwnd)?;
    let editor = create_editor(group)?.with_document_host(&host);
    install_group_hooks(hwnd, &editor);
    let editor_hwnd = editor.hwnd();
    if !identity.is_live_for(hwnd) {
        return Err(crate::FastPadError::Invariant(
            "main window was destroyed during editor initialization",
        ));
    }

    let recovery_id = unsafe { app_ptr(hwnd) }
        .map(|app| unsafe { app.as_ref() }.allocate_recovery_id())
        .ok_or(crate::FastPadError::Invariant(
            "main window app state was not available",
        ))?;
    let initial = editor.create_document()?;
    editor.use_document(&initial)?;
    let document = Document::untitled(DocumentId(1), recovery_id, initial);
    if let Some(mut app) = unsafe { app_ptr(hwnd) } {
        unsafe { app.as_mut() }.document_host = Some(host);
    }
    if !identity.is_live_for(hwnd) {
        return Err(crate::FastPadError::Invariant(
            "main window was destroyed while adopting the initial document",
        ));
    }

    unsafe {
        install_editor(hwnd, group, editor, document)?;
        record_milestone(hwnd, Milestone::EditorCreated)?;
    }
    // The sidebar comes with the window, before first paint, from the settings bootstrap read.
    crate::window::side_panel::create_for_first_frame(hwnd, notes_mode_enabled(hwnd));
    Ok(editor_hwnd)
}

pub(crate) unsafe fn input_priority_requested(hwnd: HWND, identity: &WindowIdentity) -> bool {
    // SAFETY: This helper reads a raw App pointer stored in `GWLP_USERDATA` and copies a boolean
    // flag without returning references across the FFI boundary.
    if !identity.is_live_for(hwnd) {
        return false;
    }
    unsafe { app_ptr(hwnd) }
        .map(|app| unsafe { app.as_ref() }.prioritizes_input())
        .unwrap_or(false)
}

pub(crate) unsafe fn clear_input_priority(hwnd: HWND, identity: &WindowIdentity) {
    // SAFETY: This helper mutates a boolean flag through the window-owned App pointer and does not
    // retain any reference across reentrant Win32 calls.
    if !identity.is_live_for(hwnd) {
        return;
    }
    if let Some(mut app) = unsafe { app_ptr(hwnd) } {
        unsafe { app.as_mut() }.clear_input_priority();
    }
}

pub(super) unsafe fn record_milestone(hwnd: HWND, milestone: Milestone) -> Result<()> {
    // SAFETY: The App pointer is owned by the window and is only used for an immediate milestone
    // write before returning to the caller.
    let Some(mut app) = (unsafe { app_ptr(hwnd) }) else {
        return Err(crate::FastPadError::Invariant(
            "main window app state was not available",
        ));
    };
    unsafe { app.as_mut() }.startup.record_now(milestone)
}

pub(super) unsafe fn mark_first_paint_complete(hwnd: HWND) {
    // SAFETY: The App pointer is used only for an immediate state transition after painting.
    if let Some(mut app) = unsafe { app_ptr(hwnd) } {
        unsafe { app.as_mut() }.mark_first_paint_complete();
    }
}

pub(super) unsafe fn take_deferred_start_pending(hwnd: HWND) -> bool {
    // SAFETY: The App pointer is used only for an immediate flag read-reset before returning.
    unsafe { app_ptr(hwnd) }
        .map(|mut app| unsafe { app.as_mut() }.take_deferred_start_pending())
        .unwrap_or(false)
}

unsafe fn request_input_priority(hwnd: HWND) {
    // SAFETY: The App pointer is used only for an immediate flag write before returning.
    if let Some(mut app) = unsafe { app_ptr(hwnd) } {
        unsafe { app.as_mut() }.request_input_priority();
    }
}

pub(crate) unsafe fn editor_hwnd(hwnd: HWND) -> Option<HWND> {
    // SAFETY: The App pointer is used only to copy out the child HWND; no reference crosses into
    // any subsequent Win32 call.
    let app = unsafe { app_ptr(hwnd) }?;
    unsafe { app.as_ref() }.editor().map(Editor::hwnd)
}

pub(crate) fn with_editor(hwnd: HWND, action: impl FnOnce(&Editor)) {
    let Some(app) = (unsafe { app_ptr(hwnd) }) else {
        return;
    };
    let Some(editor) = unsafe { app.as_ref() }.editor() else {
        return;
    };
    action(editor);
}
