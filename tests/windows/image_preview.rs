#![cfg(windows)]
mod support;

// Build the crate in-process with cfg(test), as json_commands.rs does, so tests can reach App.
include!("../../src/lib.rs");

use crate::window::commands::CommandId;
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, IsWindowVisible, MSG, PM_QS_INPUT, PM_REMOVE, PeekMessageW, SendMessageW,
    TranslateMessage, WM_COMMAND, WM_LBUTTONDBLCLK, WM_PAINT,
};

struct TestMain {
    hwnd: HWND,
    editor: HWND,
    identity: app::WindowIdentity,
    _class: window::MainWindowClass,
}

impl TestMain {
    fn new() -> Self {
        let app = app::App::new(
            launch::LaunchOptions::default(),
            perf::StartupMetrics::begin().unwrap(),
        );
        let identity = app.window_identity();
        let instance = unsafe {
            windows_sys::Win32::System::LibraryLoader::GetModuleHandleW(std::ptr::null())
        };
        let class = window::MainWindowClass::register(instance).unwrap();
        let mut context = window::WindowCreateContext::new(Box::new(app));
        let hwnd = class.create(&mut context).unwrap();
        let editor = unsafe {
            window::initialize_editor_with(hwnd, &identity, editor::Editor::create).unwrap()
        };
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::ShowWindow(
                hwnd,
                windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOW,
            );
        }
        Self {
            hwnd,
            editor,
            identity,
            _class: class,
        }
    }

    fn with_app<R>(&self, run: impl FnOnce(&mut app::App) -> R) -> R {
        assert!(self.identity.is_live_for(self.hwnd));
        let raw = unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::GetWindowLongPtrW(
                self.hwnd,
                windows_sys::Win32::UI::WindowsAndMessaging::GWLP_USERDATA,
            )
        } as *mut app::App;
        assert!(!raw.is_null());
        run(unsafe { &mut *raw })
    }

    fn command(&self, command: CommandId) {
        unsafe { SendMessageW(self.hwnd, WM_COMMAND, command as usize, 0) };
    }

    fn set_text(&self, text: &str) {
        let bytes = std::ffi::CString::new(text).unwrap();
        unsafe {
            SendMessageW(
                self.editor,
                crate::editor::scintilla_constants::SCI_SETTEXT,
                0,
                bytes.as_ptr() as isize,
            )
        };
    }

    fn notices(&self) -> Vec<String> {
        self.with_app(|app| {
            app.notifications
                .pending()
                .iter()
                .map(|notice| notice.message.clone())
                .collect()
        })
    }
}

impl Drop for TestMain {
    fn drop(&mut self) {
        if self.identity.is_live_for(self.hwnd) {
            unsafe { windows_sys::Win32::UI::WindowsAndMessaging::DestroyWindow(self.hwnd) };
        }
    }
}

fn pump_until(what: &str, timeout: Duration, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + timeout;
    loop {
        pump_pending();
        if condition() {
            return;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Dispatches everything queued, draining input before each other message as FastPad's own loop
/// prioritizes it. Posted messages are retrieved ahead of input, so a plain `PeekMessageW` loop
/// livelocks when real mouse or keyboard input reaches the (foreground) test window while a
/// deferred startup unit is queued: the unit sees input pending, reposts itself, and is retrieved
/// again before the input ever is.
fn pump_pending() {
    let mut msg = MSG::default();
    loop {
        while unsafe {
            PeekMessageW(
                &mut msg,
                std::ptr::null_mut(),
                0,
                0,
                PM_REMOVE | PM_QS_INPUT,
            )
        } != 0
        {
            unsafe {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        if unsafe { PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) } == 0 {
            return;
        }
        unsafe {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

fn visible(hwnd: HWND) -> bool {
    unsafe { IsWindowVisible(hwnd) != 0 }
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("fastpad-image-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Writes a 24-bit bottom-up BMP of `width`×`height` grey pixels.
fn write_bmp(path: &std::path::Path, width: u32, height: u32) {
    let row = (width * 3).div_ceil(4) * 4;
    let size = 54 + row * height;
    let mut bytes = Vec::with_capacity(size as usize);
    bytes.extend_from_slice(b"BM");
    bytes.extend_from_slice(&size.to_le_bytes());
    bytes.extend_from_slice(&[0; 4]);
    bytes.extend_from_slice(&54_u32.to_le_bytes());
    bytes.extend_from_slice(&40_u32.to_le_bytes());
    bytes.extend_from_slice(&(width as i32).to_le_bytes());
    bytes.extend_from_slice(&(height as i32).to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&24_u16.to_le_bytes());
    bytes.extend_from_slice(&[0; 24]);
    bytes.resize(size as usize, 0x80);
    std::fs::write(path, bytes).unwrap();
}

impl TestMain {
    fn open(&self, path: &std::path::Path) -> crate::Result<()> {
        window::open_path(self.hwnd, path)
    }

    fn image_view(&self) -> image_view::ImageView {
        self.with_app(|app| app.active_group().unwrap().image.view)
            .expect("image view")
    }

    fn wait_ready(&self) -> image_view::ImageStats {
        let view = self.image_view();
        pump_until("image decoded", Duration::from_secs(5), || {
            matches!(
                view.stats().phase,
                image_view::Phase::Ready | image_view::Phase::Failed
            )
        });
        view.stats()
    }
}

#[test]
fn a_bmp_opens_in_an_image_tab_that_fits_the_window() {
    // Break caught: images refused with "unsupported text encoding", shown in the editor, or opened
    // at 100% overflowing the window.
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    let dir = scratch("fit");
    let path = dir.join("big.bmp");
    write_bmp(&path, 4000, 3000);
    main.open(&path).unwrap();
    assert!(main.with_app(|app| app.tabs.active().unwrap().is_image()));
    assert!(!visible(main.editor));
    let view = main.image_view();
    assert!(visible(view.hwnd()));
    let stats = main.wait_ready();
    assert_eq!(stats.phase, image_view::Phase::Ready);
    assert!(stats.scale < 1.0);
    assert_eq!(view.status().size, Some((4000, 3000)));
    assert_eq!(view.status().format, Some("BMP"));
    assert!(main.notices().is_empty());
}

#[test]
fn a_small_image_shows_at_100_percent_and_zoom_commands_step_and_reset() {
    // Break caught: small images enlarged to fit, Ctrl+plus zooming the hidden editor, or Ctrl+0
    // not returning to fit.
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    let dir = scratch("zoom");
    let path = dir.join("small.bmp");
    write_bmp(&path, 64, 32);
    main.open(&path).unwrap();
    let view = main.image_view();
    assert_eq!(main.wait_ready().scale, 1.0);
    main.command(CommandId::ZoomIn);
    assert_eq!(view.stats().scale, 1.5);
    main.command(CommandId::ZoomOut);
    main.command(CommandId::ZoomOut);
    assert_eq!(view.stats().scale, 0.67);
    main.command(CommandId::ZoomReset);
    assert_eq!(view.stats().scale, 1.0);
    unsafe { SendMessageW(view.hwnd(), WM_LBUTTONDBLCLK, 0, 0) };
    assert_eq!(view.stats().scale, 1.0); // fit is already 100% for a small image
}

#[test]
fn saving_an_image_tab_never_writes_the_file() {
    // Break caught: Ctrl+S writing the empty placeholder document over the image.
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    let dir = scratch("save");
    let path = dir.join("keep.png");
    std::fs::write(&path, preview::images::PNG_2X2).unwrap();
    main.open(&path).unwrap();
    main.wait_ready();
    for command in [
        CommandId::Save,
        CommandId::SaveAs,
        CommandId::Undo,
        CommandId::Paste,
        CommandId::Find,
    ] {
        main.command(command);
    }
    assert!(platform::dialogs::take_dialog_events().is_empty());
    assert_eq!(std::fs::read(&path).unwrap(), preview::images::PNG_2X2);
    assert!(!main.with_app(|app| app.tabs.active().unwrap().dirty));
    assert!(main.with_app(|app| app.find_bar().is_none()));
    main.command(CommandId::CloseTab);
    assert_eq!(main.with_app(|app| app.tabs.len()), 0);
    assert_eq!(std::fs::read(&path).unwrap(), preview::images::PNG_2X2);
}

#[test]
fn a_corrupt_png_shows_the_failed_state_without_a_notice() {
    // Break caught: a damaged image freezing the window, raising a notice, or closing its tab.
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    let dir = scratch("corrupt");
    let path = dir.join("broken.png");
    std::fs::write(&path, b"\x89PNG\r\n\x1a\nnot really").unwrap();
    main.open(&path).unwrap();
    assert_eq!(main.wait_ready().phase, image_view::Phase::Failed);
    assert!(main.notices().is_empty());
    assert!(
        main.image_view()
            .accessible_text()
            .0
            .contains("can't display")
    );
}

#[test]
fn a_misnamed_image_opens_by_signature_and_other_binary_still_gets_the_notice() {
    // Break caught: a PNG saved as .dat refused, or every binary file opening as a broken image.
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    let dir = scratch("sniff");
    let png = dir.join("photo.dat");
    std::fs::write(&png, preview::images::PNG_2X2).unwrap();
    main.open(&png).unwrap();
    assert!(main.with_app(|app| app.tabs.active().unwrap().is_image()));
    let other = dir.join("blob.bin");
    std::fs::write(&other, [0xFF_u8, 0xFE, 0x00, 0xC3, 0x28]).unwrap();
    assert!(matches!(
        main.open(&other),
        Err(FastPadError::UnsupportedEncoding)
    ));
}

#[test]
fn an_image_replaces_the_empty_start_tab() {
    // Break caught: "Untitled" left beside the first image opened.
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    assert_eq!(main.with_app(|app| app.tabs.len()), 1, "the start tab");
    let dir = scratch("reuse");
    let path = dir.join("a.png");
    std::fs::write(&path, preview::images::PNG_2X2).unwrap();
    main.open(&path).unwrap();
    assert_eq!(main.with_app(|app| app.tabs.len()), 1);
    assert!(main.with_app(|app| app.tabs.active().unwrap().is_image()));
}

#[test]
fn opening_an_open_image_again_activates_its_tab() {
    // Break caught: a second open failing with "duplicate document path".
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    let dir = scratch("twice");
    let path = dir.join("a.png");
    std::fs::write(&path, preview::images::PNG_2X2).unwrap();
    main.open(&path).unwrap();
    main.command(CommandId::New);
    main.open(&dir.join(".").join("a.png")).unwrap();
    assert_eq!(main.with_app(|app| app.tabs.len()), 2);
    assert!(main.with_app(|app| app.tabs.active().unwrap().is_image()));
}

#[test]
fn switching_between_image_and_text_tabs_keeps_each_tab_intact() {
    // Break caught: the image tab turning dirty, the text tab losing its edits, the view keeping
    // its render target while hidden, or switching back decoding again.
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    let dir = scratch("switch");
    let path = dir.join("a.png");
    std::fs::write(&path, preview::images::PNG_2X2).unwrap();
    main.command(CommandId::New);
    main.set_text("draft");
    main.open(&path).unwrap();
    let view = main.image_view();
    main.wait_ready();
    unsafe { SendMessageW(view.hwnd(), WM_PAINT, 0, 0) };
    assert!(view.stats().has_target);
    main.command(CommandId::PreviousTab);
    assert!(visible(main.editor) && !visible(view.hwnd()));
    assert!(!view.stats().has_target);
    assert_eq!(
        support::win32::scintilla_text(main.editor).unwrap(),
        "draft"
    );
    let decodes = view.stats().decodes;
    main.command(CommandId::NextTab);
    assert!(visible(view.hwnd()));
    assert_eq!(view.stats().decodes, decodes);
    assert!(!main.with_app(|app| app.tabs.active().unwrap().dirty));
}

#[test]
fn a_changed_or_deleted_file_is_noticed_when_its_tab_is_activated() {
    // Break caught: a stale image after the file was edited elsewhere, or a deleted file still shown.
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    let dir = scratch("disk");
    let path = dir.join("a.bmp");
    write_bmp(&path, 10, 10);
    main.open(&path).unwrap();
    let view = main.image_view();
    main.wait_ready();
    main.command(CommandId::New);
    write_bmp(&path, 20, 10);
    main.command(CommandId::PreviousTab);
    pump_until("redecode", Duration::from_secs(5), || {
        view.status().size == Some((20, 10))
    });
    main.command(CommandId::NextTab);
    std::fs::remove_file(&path).unwrap();
    main.command(CommandId::PreviousTab);
    assert_eq!(view.stats().phase, image_view::Phase::Failed);
}

#[test]
fn an_image_tab_restores_from_the_session_as_its_file() {
    // Break caught: an image tab dropped from the session, or saved as a snapshot of empty text.
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    let dir = scratch("session");
    let path = dir.join("a.png");
    std::fs::write(&path, preview::images::PNG_2X2).unwrap();
    main.open(&path).unwrap();
    let session = window::build_session(main.hwnd, &dir).unwrap();
    assert!(matches!(
        &session.groups[0].entries[0].source,
        session::SessionSource::File(file) if file.ends_with("a.png")
    ));
}

#[test]
fn an_svg_opens_as_text_and_the_preview_renders_and_follows_edits() {
    // Break caught: SVG opening in the image view (user chose text), the preview not rendering
    // it, or a typing mistake blanking the last good render.
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    let dir = scratch("svg");
    let path = dir.join("logo.svg");
    std::fs::write(
        &path,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20"><rect width="40" height="20"/></svg>"#,
    )
    .unwrap();
    main.open(&path).unwrap();
    pump_pending();
    assert!(!main.with_app(|app| app.tabs.active().unwrap().is_image()));
    assert!(window::preview_host::buttons_visible(main.hwnd));
    main.command(CommandId::MarkdownPreviewCycle);
    let view = main
        .with_app(|app| app.active_group().unwrap().preview.svg_view)
        .expect("svg view");
    pump_until("svg rendered", Duration::from_secs(5), || {
        view.stats().phase == image_view::Phase::Ready
    });
    assert_eq!(view.status().size, Some((40, 20)));
    main.set_text(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="80" height="20"><rect width="80" height="20"/></svg>"#,
    );
    pump_until("svg re-rendered", Duration::from_secs(5), || {
        view.status().size == Some((80, 20))
    });
    main.set_text("<svg");
    pump_until("svg error bar", Duration::from_secs(5), || {
        view.stats().svg_error
    });
    assert_eq!(view.status().size, Some((80, 20)));
}

#[test]
fn the_image_view_names_itself_and_its_zoom_for_screen_readers() {
    // Break caught: Narrator reading "Markdown preview" or nothing for an image.
    let _scintilla = support::win32::WindowHarness::new().unwrap();
    let main = TestMain::new();
    let dir = scratch("a11y");
    let path = dir.join("shot.bmp");
    write_bmp(&path, 64, 32);
    main.open(&path).unwrap();
    main.wait_ready();
    let (name, value) = main.image_view().accessible_text();
    assert_eq!(name, "shot.bmp, image, 64 by 32 pixels");
    assert_eq!(value, "Zoom 100 percent");
    main.command(CommandId::ZoomIn);
    assert_eq!(main.image_view().accessible_text().1, "Zoom 150 percent");
}

#[test]
fn launching_with_a_text_file_loads_no_preview_graphics_library() {
    use support::process::{FastPadProcess, process_has_module_loaded};
    /// Removes the temporary folder even when an assertion fails.
    struct TempRoot(std::path::PathBuf);
    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let root =
        TempRoot(std::env::temp_dir().join(format!("fastpad-startup-{}", std::process::id())));
    let root = &root.0;
    let local_app_data = root.join("LocalAppData");
    std::fs::create_dir_all(&local_app_data).unwrap();
    let path = root.join("startup.txt");
    std::fs::write(&path, "plain text\n").unwrap();
    let mut process = FastPadProcess::spawn_with_local_app_data(
        [std::ffi::OsStr::new("--new-window"), path.as_os_str()],
        &local_app_data,
    )
    .unwrap();
    process
        .wait_for_main_window(Duration::from_secs(10))
        .unwrap();
    std::thread::sleep(Duration::from_millis(500));
    for module in ["d2d1.dll", "dwrite.dll", "windowscodecs.dll"] {
        assert!(
            !process_has_module_loaded(process.id(), module).unwrap(),
            "{module} was loaded before any preview was opened"
        );
    }
    process.close().unwrap();
    let _ = std::fs::remove_dir_all(root);
}
