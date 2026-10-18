//! `fastpad --capture` (second brain phase 1 spec §10.1): forwarded to the running FastPad when
//! there is one; otherwise this process files the capture itself, showing only the capture box
//! when it has nothing to file yet. No session, recovery, library scan or main window.

use super::capture_box::{self, Sink, Submitted};
use crate::ipc::IpcRequest;
use crate::ipc::client::{Forward, forward_if_running};
use crate::launch::{CaptureRequest, CaptureText};
use crate::library::capture_file::{self, LARGE_FILE_BYTES, NoTabs, Request, Source};
use crate::library::notebook_ini::{CaptureTarget, NotebookSettings};
use std::path::{Path, PathBuf};

/// Filed (or the box closed).
pub const EXIT_OK: i32 = 0;
/// The capture could not be written.
pub const EXIT_FAILED: i32 = 1;
/// The request was refused: text past the 64 KB frame, or an attachment that is missing, a
/// folder, or over 20 MB.
pub const EXIT_REFUSED: i32 = 2;

/// Checks `--attach` paths: each an existing file of at most 20 MB, made absolute.
pub fn checked_attachments(paths: &[std::ffi::OsString]) -> Result<Vec<PathBuf>, String> {
    paths
        .iter()
        .map(|path| {
            let path = std::path::absolute(path).map_err(|error| error.to_string())?;
            let meta =
                std::fs::metadata(&path).map_err(|error| format!("{}: {error}", path.display()))?;
            if !meta.is_file() {
                return Err(format!("{} is not a file", path.display()));
            }
            if meta.len() > LARGE_FILE_BYTES {
                return Err(format!("{} is over 20 MB", path.display()));
            }
            Ok(path)
        })
        .collect()
}

/// The notebook a capture-only process files into, and its conventions.
fn root() -> Option<(PathBuf, NotebookSettings)> {
    let data = crate::platform::fastpad_data_dir().ok();
    let fallback = crate::platform::paths::default_notes_folder().ok();
    let root = capture_file::capture_root(data.as_deref(), fallback)?;
    let settings = capture_file::notebook_settings_of(&root);
    Some((root, settings))
}

/// Files `text` and `attachments` with no window.
fn file_now(
    root: &Path,
    settings: &NotebookSettings,
    target: CaptureTarget,
    text: &str,
    attachments: &[Source],
) -> Result<(PathBuf, usize), String> {
    let (date, time) = crate::platform::clock::local_now();
    let request = Request {
        root,
        settings,
        target,
        date,
        time,
        text,
        attachments,
        source: None,
    };
    let filed = capture_file::file_capture(&request, &mut NoTabs)?;
    Ok((root.join(filed.note), filed.line))
}

/// The box's sink in a capture-only process.
struct OnlySink {
    root: PathBuf,
    settings: NotebookSettings,
}

impl Sink for OnlySink {
    fn target_name(&self, target: CaptureTarget) -> String {
        capture_file::target_name(&self.settings, target)
    }

    fn file(&mut self, capture: &Submitted) -> Result<(), String> {
        let (path, _) = file_now(
            &self.root,
            &self.settings,
            capture.target,
            &capture.text,
            &capture.attachments,
        )?;
        if capture.open {
            // Ctrl+Shift+Enter: FastPad starts normally with the note.
            if let Ok(exe) = std::env::current_exe() {
                let _ = std::process::Command::new(exe).arg(&path).spawn();
            }
        }
        Ok(())
    }

    fn closed(&mut self) {
        unsafe { windows_sys::Win32::UI::WindowsAndMessaging::PostQuitMessage(0) };
    }
}

/// Runs `fastpad --capture …` and returns the exit code.
pub fn run(request: CaptureRequest) -> i32 {
    let text = match &request.text {
        None => None,
        Some(CaptureText::Text(text)) => Some(text.clone()),
        Some(CaptureText::Stdin) => {
            let mut text = String::new();
            let _ = std::io::Read::read_to_string(&mut std::io::stdin(), &mut text);
            Some(text)
        }
    };
    let Ok(attachments) = checked_attachments(&request.attachments) else {
        return EXIT_REFUSED;
    };
    let forwarded = IpcRequest::Capture {
        text: text.clone(),
        attachments: attachments.clone(),
    };
    if crate::ipc::encode_frame(&forwarded).is_err() {
        return EXIT_REFUSED;
    }
    match forward_if_running(&forwarded) {
        Forward::Sent => return EXIT_OK,
        Forward::NotRunning | Forward::Failed => {}
    }
    let Some((root, settings)) = root() else {
        return EXIT_FAILED;
    };
    if text.is_some() || !attachments.is_empty() {
        let sources: Vec<Source> = attachments.into_iter().map(Source::File).collect();
        return match file_now(
            &root,
            &settings,
            settings.capture_target,
            text.as_deref().unwrap_or_default(),
            &sources,
        ) {
            Ok(_) => EXIT_OK,
            Err(_) => EXIT_FAILED,
        };
    }
    show_box(root, settings)
}

/// The capture box alone, until it closes.
fn show_box(root: PathBuf, settings: NotebookSettings) -> i32 {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, GetMessageW, MSG, PostMessageW, TranslateMessage,
    };
    // A second `--capture` while a capture-only box is open brings that one forward instead.
    if let Some(open) = capture_box::find() {
        unsafe { PostMessageW(open, capture_box::WM_CAPTURE_AGAIN, 0, 0) };
        return EXIT_OK;
    }
    unsafe {
        windows_sys::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(
            windows_sys::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        );
    }
    let (preferences, _) = crate::config::load();
    let system = crate::platform::theme::SystemTheme::detect();
    let colors = super::palette::Palette::for_theme(
        system.effective_theme(preferences.theme),
        system.high_contrast,
    );
    let target = settings.capture_target;
    if capture_box::open(colors, target, Box::new(OnlySink { root, settings })).is_none() {
        return EXIT_FAILED;
    }
    let mut message = MSG::default();
    while unsafe { GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) } > 0 {
        unsafe {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    EXIT_OK
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attachments_must_be_files_within_the_size_limit() {
        let dir = std::env::temp_dir().join(format!("fastpad-capture-only-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.log");
        std::fs::write(&file, "x").unwrap();
        assert_eq!(
            checked_attachments(&[file.clone().into_os_string()]),
            Ok(vec![file.clone()])
        );
        assert!(checked_attachments(&[dir.clone().into_os_string()]).is_err());
        assert!(checked_attachments(&[dir.join("missing").into_os_string()]).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
