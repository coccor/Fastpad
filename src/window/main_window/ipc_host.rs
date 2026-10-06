//! The single-instance IPC server: binding it, servicing forwarded launches, and shutting it
//! down.

use super::*;

const IPC_UNAVAILABLE_NOTICE: &str =
    "FastPad could not start its single-instance listener; later launches open separate windows.";

/// Binds the pipe server only for the process that owns the session instance mutex.
pub(super) fn start_ipc_server_with(
    hwnd: HWND,
    bind: impl FnOnce() -> Result<crate::ipc::IpcServer>,
) {
    let Some(mut app) = (unsafe { app_ptr(hwnd) }) else {
        return;
    };
    // Binding makes no window calls, so this App borrow cannot be re-entered.
    let app = unsafe { app.as_mut() };
    if app.instance_mutex.is_none() || app.ipc.is_some() {
        return;
    }
    match bind() {
        Ok(server) => app.ipc = Some(server),
        Err(_) => stop_ipc(app),
    }
}

fn stop_ipc(app: &mut App) {
    app.ipc = None;
    // Releasing the mutex sends later launches to independent processes instead of a dead pipe.
    app.instance_mutex = None;
    app.notifications.push(IPC_UNAVAILABLE_NOTICE);
}

/// Copies the pipe event out of App for one wait; callers must not dispatch while using it.
pub(crate) fn ipc_wait_handle(
    hwnd: HWND,
    identity: &WindowIdentity,
) -> Option<windows_sys::Win32::Foundation::HANDLE> {
    if !identity.is_live_for(hwnd) {
        return None;
    }
    let app = unsafe { app_ptr(hwnd) }?;
    unsafe { app.as_ref() }
        .ipc
        .as_ref()
        .map(crate::ipc::IpcServer::event)
}

/// Services a signaled pipe event: queues decoded requests and posts one drain message.
pub(crate) fn service_ipc(hwnd: HWND, identity: &WindowIdentity) {
    if !identity.is_live_for(hwnd) {
        return;
    }
    let Some(mut app) = (unsafe { app_ptr(hwnd) }) else {
        return;
    };
    let queued = {
        let app = unsafe { app.as_mut() };
        let Some(server) = app.ipc.as_mut() else {
            return;
        };
        match server.poll() {
            Ok(requests) => {
                let queued = !requests.is_empty();
                app.ipc_requests.extend(requests);
                Some(queued)
            }
            Err(_) => {
                stop_ipc(app);
                None
            }
        }
    };
    match queued {
        Some(true) => unsafe {
            PostMessageW(hwnd, crate::window::WM_FASTPAD_IPC_REQUEST, 0, 0);
        },
        Some(false) => {}
        None => refresh_notifications(hwnd),
    }
}

pub(super) fn handle_ipc_requests(hwnd: HWND) -> LRESULT {
    // The pipe is bound as soon as a session restore starts. A forwarded file opened now would
    // be buried under the tabs still to be restored, so requests wait in the queue until
    // `finish_session_restore` posts this message again.
    if unsafe { app_ptr(hwnd) }.is_some_and(|app| unsafe { app.as_ref() }.session_restore.is_some())
    {
        return 0;
    }
    open_ipc_requests(hwnd)
}

/// Handles every queued forwarded launch now, restore or not.
fn open_ipc_requests(hwnd: HWND) -> LRESULT {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return 0;
    };
    let requests = unsafe { app_ptr(hwnd) }
        .map(|mut app| std::mem::take(&mut unsafe { app.as_mut() }.ipc_requests))
        .unwrap_or_default();
    for request in requests {
        if !identity.is_live_for(hwnd) {
            return 0;
        }
        match request {
            crate::ipc::IpcRequest::Open(path) => {
                if let Err(error) = App::open_path(hwnd, &path) {
                    report_open_failure(hwnd, &path, &error);
                }
            }
            crate::ipc::IpcRequest::New => execute_command(hwnd, CommandId::New),
            crate::ipc::IpcRequest::Activate => {}
            crate::ipc::IpcRequest::OpenFolder(path) => {
                crate::window::library_host::open_folder(hwnd, &path)
            }
        }
        if identity.is_live_for(hwnd) {
            bring_to_foreground(hwnd);
        }
    }
    0
}

fn bring_to_foreground(hwnd: HWND) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        IsIconic, SW_RESTORE, SetForegroundWindow, ShowWindow,
    };
    unsafe {
        if IsIconic(hwnd) != 0 {
            ShowWindow(hwnd, SW_RESTORE);
        }
        SetForegroundWindow(hwnd);
    }
}

/// Services the pipe and handles everything already queued, before a close review begins. This
/// ignores the mid-restore hold: a close then uses the review, which must see a forwarded file
/// as a tab rather than drop it with the window. If the review is cancelled, the restore goes on
/// and the extra tab is simply one more open tab.
pub(super) fn drain_ipc_requests(hwnd: HWND) {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    service_ipc(hwnd, &identity);
    if identity.is_live_for(hwnd) {
        open_ipc_requests(hwnd);
    }
}

/// Stops accepting forwarded launches before releasing the mutex that invites the next primary to
/// bind the pipe; the reverse order would let it bind while this server still exists.
pub(super) fn shutdown_ipc(hwnd: HWND) {
    let Some(mut app) = (unsafe { app_ptr(hwnd) }) else {
        return;
    };
    let app = unsafe { app.as_mut() };
    drop(app.ipc.take());
    drop(app.instance_mutex.take());
}

pub(super) fn clear_documents_for_shutdown(hwnd: HWND) {
    if let Some(mut app) = unsafe { app_ptr(hwnd) } {
        let app = unsafe { app.as_mut() };
        app.tabs.clear_for_shutdown();
        // The Markdown helpers' per-document state goes with the documents.
        app.markdown = Default::default();
    }
}
