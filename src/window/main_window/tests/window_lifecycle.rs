//! Window creation and destruction, App ownership, and the IPC server.

use super::*;

#[test]
fn the_editor_reads_single_lines_without_their_line_endings() {
    // Break caught: a line reader that keeps the CR/LF, misreads Scintilla's line count, or
    // panics instead of returning empty text past the last line.
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    editor.set_text("first\r\nsecond\nthird").unwrap();
    assert_eq!(editor.line_count().unwrap(), 3);
    assert_eq!(editor.line_text(0).unwrap(), "first");
    assert_eq!(editor.line_text(1).unwrap(), "second");
    assert_eq!(editor.line_text(2).unwrap(), "third");
    assert_eq!(editor.line_text(9).unwrap(), "");
}

#[test]
fn the_editor_reads_multi_byte_utf8_lines_without_their_line_endings() {
    // Break caught: a byte-length-based line reader splitting or corrupting a multi-byte
    // UTF-8 character at the line boundary instead of returning the line whole.
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    editor.set_text("h\u{e9}llo \u{1f600}\r\nsecond").unwrap();
    assert_eq!(editor.line_text(0).unwrap(), "h\u{e9}llo \u{1f600}");
}

#[test]
fn create_context_drops_untransferred_value_on_pre_window_failure() {
    // Break caught: bootstrap manually reclaiming a create-time App allocation is unsafe once
    // ownership can also transfer through WM_NCCREATE.
    let drops = Arc::new(AtomicUsize::new(0));
    {
        let _context = WindowCreateContext::new(Box::new(DropProbe::new(Arc::clone(&drops))));
    }

    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[test]
fn production_nc_create_transfers_app_and_nc_destroy_clears_the_window() {
    // Break caught: bypassing the real WM_NCCREATE/WM_NCDESTROY ownership path can leave the
    // production window without App state or leave the HWND alive after teardown.
    let window = ProductionWindow::new(make_app());
    assert_ne!(unsafe { GetWindowLongPtrW(window.hwnd, GWLP_USERDATA) }, 0);
    unsafe {
        DestroyWindow(window.hwnd);
    }
    assert_eq!(unsafe { IsWindow(window.hwnd) }, 0);
}

#[test]
fn initial_editor_installation_updates_a_retained_empty_tab_view() {
    // Break caught: accessibility requested before editor creation retains an obsolete view.
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let view = unsafe { super::super::app_ptr(window.hwnd).unwrap().as_ref() }
        .tabs
        .view();
    assert!(view.snapshot().tabs.is_empty());
    let identity = unsafe { super::super::window_identity(window.hwnd).unwrap() };
    unsafe {
        super::super::initialize_editor_with(window.hwnd, &identity, crate::editor::Editor::create)
    }
    .unwrap();
    assert_eq!(view.snapshot().tabs.len(), 1);
}

#[test]
fn native_wm_close_releases_all_owned_documents_before_editor_destruction() {
    // Break caught: clearing tabs after DestroyWindow skips real releases at the dead endpoint.
    if std::env::var_os("FASTPAD_REQUIRE_APPVERIF").is_some() {
        let verifier = crate::platform::wide_null("verifier.dll");
        assert!(
            !unsafe { GetModuleHandleW(verifier.as_ptr()) }.is_null(),
            "Application Verifier must actually be loaded for a claimed verifier run"
        );
    }
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let identity = unsafe { super::super::window_identity(window.hwnd).unwrap() };
    let editor = unsafe {
        super::super::initialize_editor_with(window.hwnd, &identity, crate::editor::Editor::create)
    }
    .unwrap();
    super::super::create_new_document(window.hwnd).unwrap();
    // Documents belong to the document host, so their releases go through its endpoint.
    let host = unsafe { super::super::app_ptr(window.hwnd).unwrap().as_ref() }
        .document_host
        .as_ref()
        .unwrap()
        .hwnd();
    let (_, releases) = crate::editor::scintilla::release_observation::during(|| unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::SendMessageW(
            window.hwnd,
            windows_sys::Win32::UI::WindowsAndMessaging::WM_CLOSE,
            0,
            0,
        )
    });
    assert_eq!(releases.len(), 2);
    assert_ne!(releases[0].document, releases[1].document);
    assert!(
        releases
            .iter()
            .all(|release| release.hwnd == host && release.window_was_live)
    );
    assert_eq!(unsafe { IsWindow(editor) }, 0);
    assert_eq!(unsafe { IsWindow(host) }, 0);
    assert_eq!(unsafe { IsWindow(window.hwnd) }, 0);
}

#[test]
fn original_window_identity_stays_invalid_after_replacement_creation() {
    // Break caught: an IsWindow-only liveness check can accept a recycled HWND and read the
    // replacement window's GWLP_USERDATA as the original App.
    let original_app = make_app();
    let original_identity = original_app.window_identity();
    let original = ProductionWindow::new(original_app);
    assert!(original_identity.is_live_for(original.hwnd));

    unsafe {
        DestroyWindow(original.hwnd);
    }
    assert!(original_identity.is_invalidated());
    drop(original);

    let replacement_app = make_app();
    let replacement_identity = replacement_app.window_identity();
    let replacement = ProductionWindow::new(replacement_app);
    assert!(replacement_identity.is_live_for(replacement.hwnd));
    assert!(original_identity.is_invalidated());
    assert!(!original_identity.is_live_for(replacement.hwnd));
}

#[test]
fn reentrant_paint_completion_does_not_mutate_replacement_app() {
    // Break caught: removing the post-DefWindowProc identity gate lets an old WM_PAINT
    // completion mutate the App found in a recycled HWND's replacement GWLP_USERDATA slot.
    const PAINT_RESULT: LRESULT = 73;
    let mut original = Some(ProductionWindow::new(make_app()));
    let original_hwnd = original.as_ref().unwrap().hwnd;
    let replacement = RefCell::new(None::<ProductionWindow>);

    let default_window_proc = |hwnd, _, _, _| {
        assert_ne!(unsafe { DestroyWindow(hwnd) }, 0);
        drop(original.take());
        replacement.replace(Some(ProductionWindow::new(make_app())));
        PAINT_RESULT
    };
    let complete_first_paint = |_| {
        let replacement = replacement.borrow();
        let replacement = replacement.as_ref().unwrap();
        unsafe {
            mark_first_paint_complete(replacement.hwnd);
        }
    };
    let result = unsafe {
        handle_paint_with(
            original_hwnd,
            WM_PAINT,
            0,
            0,
            default_window_proc,
            complete_first_paint,
        )
    };

    assert_eq!(result, PAINT_RESULT);
    let replacement = replacement.borrow();
    let replacement = replacement.as_ref().unwrap();
    assert!(!unsafe { take_deferred_start_pending(replacement.hwnd) });
}

#[test]
fn ipc_bind_failure_releases_the_instance_mutex_and_notifies_exactly_once() {
    // Break caught: keeping the mutex after a failed bind makes every later launch wait on a
    // pipe that will never exist; retrying or re-notifying spams the status line.
    let window = ProductionWindow::new(make_app());
    unsafe { super::super::app_ptr(window.hwnd).unwrap().as_mut() }.instance_mutex =
        Some(unnamed_mutex());

    super::super::start_ipc_server_with(window.hwnd, || {
        Err(crate::FastPadError::Ipc("simulated bind failure"))
    });
    super::super::start_ipc_server_with(window.hwnd, || unreachable!("no mutex means no server"));

    let app = unsafe { super::super::app_ptr(window.hwnd).unwrap().as_ref() };
    assert!(app.ipc.is_none());
    assert!(app.instance_mutex.is_none());
    assert_eq!(app.notifications.len(), 1);
}

#[test]
fn process_without_instance_mutex_never_binds_a_server() {
    // Break caught: a --new-window or fallback process squats the primary's pipe name.
    let window = ProductionWindow::new(make_app());
    super::super::start_ipc_server_with(window.hwnd, || unreachable!("no mutex means no server"));
    let app = unsafe { super::super::app_ptr(window.hwnd).unwrap().as_ref() };
    assert!(app.ipc.is_none());
    assert_eq!(app.notifications.len(), 0);
}

fn deliver_frame(window: &ProductionWindow, names: &crate::ipc::InstanceNames, frame: Vec<u8>) {
    use std::time::{Duration, Instant};
    use windows_sys::Win32::Foundation::WAIT_OBJECT_0;
    use windows_sys::Win32::System::Threading::WaitForSingleObject;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, MSG, PM_REMOVE, PeekMessageW, TranslateMessage,
    };
    let pipe = names.clone();
    let client = std::thread::spawn(move || {
        crate::ipc::client::send_frame(&pipe, &frame, Duration::from_secs(2))
    });
    let identity = unsafe { super::super::window_identity(window.hwnd).unwrap() };
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut quiet_since = None;
    while Instant::now() < deadline {
        let event = super::super::ipc_wait_handle(window.hwnd, &identity).unwrap();
        if unsafe { WaitForSingleObject(event, 20) } == WAIT_OBJECT_0 {
            super::super::service_ipc(window.hwnd, &identity);
            quiet_since = None;
        } else if client.is_finished() {
            let since = *quiet_since.get_or_insert_with(Instant::now);
            if since.elapsed() >= Duration::from_millis(150) {
                break;
            }
        }
        let mut message = MSG::default();
        while unsafe { PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_REMOVE) } != 0 {
            unsafe {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
    }
    client.join().unwrap().unwrap();
}

#[test]
fn ipc_requests_reach_the_window_as_tabs_and_malformed_frames_change_nothing() {
    // Break caught: decoded requests never leave the pipe, duplicate opens add tabs, Activate
    // mutates tabs, or a malformed frame reaches application state.
    use crate::ipc::{IpcRequest, encode_frame};
    let _scintilla = load_native_scintilla();
    let window = ProductionWindow::new(make_app());
    let editor = install_test_editor(&window);
    unsafe {
        SendMessageW(
            editor.hwnd(),
            windows_sys::Win32::UI::WindowsAndMessaging::WM_CHAR,
            b'x' as usize,
            0,
        );
    }
    unsafe { super::super::app_ptr(window.hwnd).unwrap().as_mut() }.instance_mutex =
        Some(unnamed_mutex());
    let names = crate::ipc::server::tests::unique_names();
    super::super::start_ipc_server_with(window.hwnd, || {
        crate::ipc::IpcServer::bind(&names, &crate::ipc::CurrentUserAcl::current()?)
    });
    let scratch = RecoveryScratch::new("ipc-open");
    let file = scratch.path().join("forwarded.txt");
    std::fs::write(&file, b"forwarded text").unwrap();
    let tabs = || {
        unsafe { super::super::app_ptr(window.hwnd).unwrap().as_ref() }
            .tabs
            .len()
    };

    let open = encode_frame(&IpcRequest::Open(file.clone())).unwrap();
    deliver_frame(&window, &names, open.clone());
    assert_eq!(tabs(), 2);
    assert_eq!(editor.text().unwrap(), "forwarded text");
    deliver_frame(&window, &names, open);
    assert_eq!(tabs(), 2);
    deliver_frame(
        &window,
        &names,
        encode_frame(&IpcRequest::Activate).unwrap(),
    );
    assert_eq!(tabs(), 2);
    deliver_frame(&window, &names, b"FPI1\x09\0\0\0\0".to_vec());
    assert_eq!(tabs(), 2);
    deliver_frame(&window, &names, encode_frame(&IpcRequest::New).unwrap());
    assert_eq!(tabs(), 3);
    assert!(
        unsafe { super::super::app_ptr(window.hwnd).unwrap().as_ref() }
            .ipc_requests
            .is_empty()
    );
}

struct DropProbe {
    drops: Arc<AtomicUsize>,
}

impl DropProbe {
    fn new(drops: Arc<AtomicUsize>) -> Self {
        Self { drops }
    }
}

impl Drop for DropProbe {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}
