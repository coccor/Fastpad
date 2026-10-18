//! Quick capture in the running FastPad (second brain phase 1 spec §10).

use super::*;
use crate::library::capture_file::Source;
use crate::library::dates::{Date, Time};
use crate::library::notebook_ini::CaptureTarget;
use crate::window::capture_box::{self, Action};

/// Closes any box a test left open, and starts without a draft.
struct Clean;

impl Clean {
    fn new() -> Self {
        Self::close_all();
        capture_box::clear_draft();
        crate::window::library_host::set_now(
            Date::new(2026, 10, 19).unwrap(),
            Time {
                hour: 10,
                minute: 30,
            },
        );
        Self
    }

    fn close_all() {
        while let Some(open) = capture_box::find() {
            unsafe { windows_sys::Win32::UI::WindowsAndMessaging::DestroyWindow(open) };
        }
    }
}

impl Drop for Clean {
    fn drop(&mut self) {
        Self::close_all();
        capture_box::clear_draft();
    }
}

fn notebook(label: &str) -> (LibraryScratch, ProductionWindow) {
    let scratch = LibraryScratch::new(label);
    let window = ProductionWindow::new(make_app());
    install_test_editor(&window);
    app_mut(window.hwnd).library.data_dir = Some(scratch.data());
    open_note(&window, &scratch, "a.md", "");
    app_mut(window.hwnd).notifications.dismiss_all();
    (scratch, window)
}

fn open_box(hwnd: HWND) -> HWND {
    crate::window::capture_host::open_box(hwnd, None).expect("the box opens")
}

#[test]
fn ctrl_enter_appends_to_the_inbox_and_closes_the_box() {
    // Break caught: a capture lost, filed somewhere else, or the box left open after saving.
    let _scintilla = load_native_scintilla();
    let _clean = Clean::new();
    let (scratch, window) = notebook("capture-inbox");

    let capture = open_box(window.hwnd);
    let (text, items, error, target) = capture_box::shown(capture);
    assert_eq!(
        (text.as_str(), items, error, target),
        ("", 0, None, CaptureTarget::Inbox)
    );
    capture_box::set_text_for_test(capture, "Call Dana about the budget #atlas\r\nsoon");
    capture_box::key_for_test(capture, Action::Submit { open: false });

    assert!(capture_box::find().is_none(), "the box closed");
    assert_eq!(
        std::fs::read_to_string(scratch.folder().join("inbox.md")).unwrap(),
        "# Inbox\n\n- 10:30 Call Dana about the budget #atlas\n  soon\n"
    );
    assert_eq!(notices(window.hwnd), ["Captured to inbox.md."]);
    crate::window::library_host::with_state(window.hwnd, |state| {
        assert!(state.is_listed(std::path::Path::new("inbox.md")));
    });
}

#[test]
fn esc_keeps_the_text_as_a_draft_for_the_next_box() {
    let _scintilla = load_native_scintilla();
    let _clean = Clean::new();
    let (scratch, window) = notebook("capture-draft");

    let capture = open_box(window.hwnd);
    capture_box::set_text_for_test(capture, "half a thought");
    capture_box::attach_for_test(capture, Source::Png(b"png".to_vec()));
    capture_box::key_for_test(capture, Action::Close);
    assert!(capture_box::find().is_none());
    assert!(
        !scratch.folder().join("inbox.md").exists(),
        "nothing was filed"
    );

    let again = open_box(window.hwnd);
    let (text, items, _, _) = capture_box::shown(again);
    assert_eq!(text, "half a thought");
    assert_eq!(items, 1, "the pasted image came back too");
}

#[test]
fn an_inbox_open_in_a_clean_tab_takes_the_capture_and_is_saved() {
    // Break caught: the file written under an open tab, which then holds stale text.
    let _scintilla = load_native_scintilla();
    let _clean = Clean::new();
    let (scratch, window) = notebook("capture-tab");
    let inbox = scratch.note("inbox.md", "# Inbox\n");
    super::super::open_path(window.hwnd, &inbox).unwrap();

    let capture = open_box(window.hwnd);
    capture_box::set_text_for_test(capture, "idea");
    capture_box::key_for_test(capture, Action::Submit { open: false });

    assert_eq!(
        std::fs::read_to_string(&inbox).unwrap(),
        "# Inbox\n\n- 10:30 idea\n"
    );
    let tab = app_mut(window.hwnd).tabs.active().unwrap();
    assert_eq!(tab.path.as_deref(), Some(inbox.as_path()));
    assert!(!tab.dirty, "the clean tab was saved");
}

#[test]
fn ctrl_alt_n_cycles_the_target_and_a_one_file_capture_links_its_image() {
    let _scintilla = load_native_scintilla();
    let _clean = Clean::new();
    let (scratch, window) = notebook("capture-folder");

    let capture = open_box(window.hwnd);
    capture_box::key_for_test(capture, Action::CycleTarget);
    assert_eq!(capture_box::shown(capture).3, CaptureTarget::Daily);
    capture_box::key_for_test(capture, Action::CycleTarget);
    assert_eq!(capture_box::shown(capture).3, CaptureTarget::Folder);
    capture_box::set_text_for_test(capture, "lag spikes");
    capture_box::attach_for_test(capture, Source::Png(b"png".to_vec()));
    capture_box::key_for_test(capture, Action::Submit { open: true });

    let note = scratch
        .folder()
        .join(r"inbox\2026-10-19 1030 lag spikes.md");
    assert_eq!(
        std::fs::read_to_string(&note).unwrap(),
        "---\ncaptured: 2026-10-19T10:30\n---\n\nlag spikes\n\n\
         ![](../attachments/2026-10-19%201030%20lag%20spikes.png)\n"
    );
    assert!(
        scratch
            .folder()
            .join(r"attachments\2026-10-19 1030 lag spikes.png")
            .exists()
    );
    assert_eq!(
        app_mut(window.hwnd).tabs.active().unwrap().path.as_deref(),
        Some(note.as_path()),
        "Ctrl+Shift+Enter opens the capture"
    );
}

#[test]
fn a_failed_capture_keeps_the_box_open_with_the_error() {
    let _scintilla = load_native_scintilla();
    let _clean = Clean::new();
    let (scratch, window) = notebook("capture-fail");

    let capture = open_box(window.hwnd);
    capture_box::set_text_for_test(capture, "keep me");
    capture_box::attach_for_test(capture, Source::File(scratch.folder().join("gone.pdf")));
    capture_box::key_for_test(capture, Action::Submit { open: false });

    assert_eq!(capture_box::find(), Some(capture));
    let (text, _, error, _) = capture_box::shown(capture);
    assert_eq!(text, "keep me");
    assert!(error.is_some_and(|error| error.contains("gone.pdf")));
    assert!(!scratch.folder().join("inbox.md").exists());
}

#[test]
fn snipping_hides_the_box_until_the_hotkey_brings_it_back() {
    use windows_sys::Win32::UI::WindowsAndMessaging::IsWindowVisible;
    let _scintilla = load_native_scintilla();
    let _clean = Clean::new();
    let (_scratch, window) = notebook("capture-snip");

    let capture = open_box(window.hwnd);
    capture_box::take_snips();
    capture_box::key_for_test(capture, Action::Snip);
    assert_eq!(capture_box::take_snips(), 1);
    assert_eq!(
        unsafe { IsWindowVisible(capture) },
        0,
        "hidden for the snip"
    );

    capture_box::again_for_test(capture);
    assert_ne!(unsafe { IsWindowVisible(capture) }, 0);
}

#[test]
fn a_forwarded_capture_with_text_is_filed_with_no_window() {
    // Break caught: `fastpad --capture "text"` opening a box, or bringing FastPad forward.
    let _scintilla = load_native_scintilla();
    let _clean = Clean::new();
    let (scratch, window) = notebook("capture-forwarded");
    let log = scratch.note("ledger.log", "log");

    app_mut(window.hwnd)
        .ipc_requests
        .push(crate::ipc::IpcRequest::Capture {
            text: Some("batch failed".to_owned()),
            attachments: vec![log.clone()],
        });
    unsafe {
        SendMessageW(window.hwnd, crate::window::WM_FASTPAD_IPC_REQUEST, 0, 0);
    }

    assert!(capture_box::find().is_none());
    assert_eq!(
        std::fs::read_to_string(scratch.folder().join("inbox.md")).unwrap(),
        "# Inbox\n\n- 10:30 batch failed\n  [ledger.log](attachments/ledger.log)\n"
    );
    assert_eq!(notices(window.hwnd), ["Captured to inbox.md."]);

    app_mut(window.hwnd)
        .ipc_requests
        .push(crate::ipc::IpcRequest::Capture {
            text: None,
            attachments: Vec::new(),
        });
    unsafe {
        SendMessageW(window.hwnd, crate::window::WM_FASTPAD_IPC_REQUEST, 0, 0);
    }
    assert!(
        capture_box::find().is_some(),
        "with nothing to file, the box opens"
    );
}

#[test]
fn an_unknown_capture_hotkey_is_named_in_a_notice() {
    let window = ProductionWindow::new(make_app());
    app_mut(window.hwnd).settings.capture_hotkey = "Ctrl+Banana".to_owned();
    crate::window::capture_host::register_hotkey(window.hwnd);
    assert_eq!(
        notices(window.hwnd),
        ["capture_hotkey \"Ctrl+Banana\" is not a key FastPad knows."]
    );
}
