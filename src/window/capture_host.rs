//! Quick capture in the running FastPad (second brain phase 1 spec §10): the global hotkey, the
//! capture box's filing into the open notebook (through an open tab when the target is in one),
//! and captures forwarded by `fastpad --capture`.

use super::capture_box::{self, Sink, Submitted};
use super::main_window::{app_ptr, push_notice};
use crate::document::DocumentId;
use crate::library::capture_file::{self, OpenTabs, Request, Source};
use crate::library::notebook_ini::{CaptureTarget, NotebookSettings};
use std::path::{Path, PathBuf};
use windows_sys::Win32::Foundation::HWND;

/// The id the capture hotkey is registered under.
const HOTKEY_ID: i32 = 0x4350;

/// The notebook a capture goes to, and its conventions: the open notebook, else the most recent
/// one, else `Documents\FastPad`.
fn capture_root(hwnd: HWND) -> Option<(PathBuf, NotebookSettings)> {
    if let Some(root) = super::library_host::folder(hwnd) {
        return Some((root, super::library_host::notebook_settings(hwnd)));
    }
    let data = crate::platform::fastpad_data_dir().ok();
    let fallback = crate::platform::paths::default_notes_folder().ok();
    let root = capture_file::capture_root(data.as_deref(), fallback)?;
    let settings = capture_file::notebook_settings_of(&root);
    Some((root, settings))
}

/// Tabs of the running FastPad, for appends to a note that is open (spec §10.3).
struct Tabs {
    hwnd: HWND,
    /// The tabs that took an append, and whether each was dirty before.
    appended: Vec<(DocumentId, bool)>,
}

impl Tabs {
    fn find(&self, path: &Path) -> Option<(DocumentId, bool)> {
        let app = unsafe { app_ptr(self.hwnd) }?;
        let tabs = &unsafe { app.as_ref() }.tabs;
        let id = tabs.find_stored_path(path)?;
        Some((id, tabs.document(id)?.dirty))
    }
}

impl OpenTabs for Tabs {
    fn text(&self, path: &Path) -> Option<String> {
        let (id, _) = self.find(path)?;
        super::main_window::document_text(self.hwnd, id)
    }

    fn append(&mut self, path: &Path, text: &str) -> Result<(), String> {
        let (id, dirty) = self
            .find(path)
            .ok_or_else(|| "The tab closed while FastPad added the capture.".to_owned())?;
        if !super::main_window::apply_disk_edit(
            self.hwnd,
            id,
            super::main_window::DiskEdit::Append(text),
        ) {
            return Err(format!(
                "FastPad could not add the capture to {}.",
                path.display()
            ));
        }
        self.appended.push((id, dirty));
        Ok(())
    }
}

/// Saves `id` without leaving it active: a clean tab that took a capture is saved, as is a
/// dirty one when autosave is on (spec §10.3).
fn save_tab(hwnd: HWND, id: DocumentId) -> bool {
    let active =
        unsafe { app_ptr(hwnd) }.and_then(|app| Some(unsafe { app.as_ref() }.tabs.active()?.id));
    let saved = super::main_window::activate_document_by_id(hwnd, id)
        && super::main_window::save_active_document(hwnd);
    if let Some(active) = active
        && active != id
    {
        super::main_window::activate_document_by_id(hwnd, active);
    }
    saved
}

/// Files `capture` into the notebook: the shared filing, then the tabs, the note list and the
/// index catch up. Returns the note's absolute path and the entry's line.
fn file(hwnd: HWND, capture: &Submitted, source: Option<&str>) -> Result<(PathBuf, usize), String> {
    let (root, settings) =
        capture_root(hwnd).ok_or_else(|| "FastPad has no notebook to capture into.".to_owned())?;
    let (date, time) = super::library_host::now();
    let request = Request {
        root: &root,
        settings: &settings,
        target: capture.target,
        date,
        time,
        text: &capture.text,
        attachments: &capture.attachments,
        source,
    };
    let mut tabs = Tabs {
        hwnd,
        appended: Vec::new(),
    };
    let filed = capture_file::file_capture(&request, &mut tabs)?;
    let path = root.join(&filed.note);
    let autosave = super::library_host::notebook_autosave(hwnd).unwrap_or(false);
    for (id, dirty) in std::mem::take(&mut tabs.appended) {
        if (!dirty || autosave) && save_tab(hwnd, id) {
            continue;
        }
        push_notice(
            hwnd,
            format!(
                "The capture is in the unsaved tab {}.",
                crate::library::title::note_title(&path)
            ),
        );
    }
    let open = super::library_host::folder(hwnd)
        .is_some_and(|folder| crate::library::model::same_path(&folder, &root));
    if open {
        super::library_host::with_state(hwnd, |state| state.add_note(&path));
        super::library_host::request_index(hwnd);
        super::side_panel::refresh(hwnd);
    }
    Ok((path, filed.line))
}

/// Opens `path` with the caret on 0-based `line` (Ctrl+Shift+Enter).
fn open_at(hwnd: HWND, path: &Path, line: usize) {
    if let Err(error) =
        super::main_window::open_note(hwnd, path, super::main_window::OpenMode::Permanent, true)
    {
        super::main_window::report_open_failure(hwnd, path, &error);
        return;
    }
    super::main_window::go_to_line(hwnd, line as u32 + 1);
    unsafe { windows_sys::Win32::UI::WindowsAndMessaging::SetForegroundWindow(hwnd) };
}

/// The box's sink in the running FastPad.
struct MainSink {
    hwnd: HWND,
    source: Option<String>,
}

impl Sink for MainSink {
    fn target_name(&self, target: CaptureTarget) -> String {
        let settings = capture_root(self.hwnd)
            .map(|(_, settings)| settings)
            .unwrap_or_default();
        capture_file::target_name(&settings, target)
    }

    fn file(&mut self, capture: &Submitted) -> Result<(), String> {
        let (path, line) = file(self.hwnd, capture, self.source.as_deref())?;
        if capture.open {
            open_at(self.hwnd, &path, line);
        } else {
            push_notice(
                self.hwnd,
                format!(
                    "Captured to {}.",
                    path.file_name().unwrap_or_default().to_string_lossy()
                ),
            );
        }
        Ok(())
    }
}

/// The notebook's default capture target.
fn default_target(hwnd: HWND) -> CaptureTarget {
    capture_root(hwnd).map_or(CaptureTarget::Inbox, |(_, settings)| {
        settings.capture_target
    })
}

/// Opens the capture box, or brings the open one forward (cycling its target when it is already
/// in front). `source` is the window that was in front, when recorded.
pub(crate) fn open_box(hwnd: HWND, source: Option<String>) -> Option<HWND> {
    if let Some(open) = capture_box::find() {
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(
                open,
                capture_box::WM_CAPTURE_AGAIN,
                0,
                0,
            )
        };
        return Some(open);
    }
    let colors = super::main_window::current_palette(hwnd);
    capture_box::open(
        colors,
        default_target(hwnd),
        Box::new(MainSink { hwnd, source }),
    )
}

/// The title of the window in front, for `capture_source` (spec §10.4).
fn foreground_title(hwnd: HWND) -> Option<String> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowTextLengthW, GetWindowTextW,
    };
    let front = unsafe { GetForegroundWindow() };
    if front.is_null() || front == hwnd {
        return None;
    }
    let length = unsafe { GetWindowTextLengthW(front) }.max(0) as usize;
    let mut buffer = vec![0_u16; length + 1];
    let copied = unsafe { GetWindowTextW(front, buffer.as_mut_ptr(), buffer.len() as i32) };
    let title = String::from_utf16_lossy(&buffer[..copied.max(0) as usize]);
    (!title.trim().is_empty()).then(|| title.trim().to_owned())
}

/// `WM_HOTKEY`: the capture hotkey.
pub(crate) fn hotkey(hwnd: HWND, id: usize) {
    if id != HOTKEY_ID as usize {
        return;
    }
    let record =
        unsafe { app_ptr(hwnd) }.is_some_and(|app| unsafe { app.as_ref() }.settings.capture_source);
    let source = if record && capture_box::find().is_none() {
        foreground_title(hwnd)
    } else {
        None
    };
    open_box(hwnd, source);
}

/// Registers `capture_hotkey` (spec §10.1) for this window. An empty setting turns it off; a
/// key another app holds gets one notice.
pub(crate) fn register_hotkey(hwnd: HWND) {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, UnregisterHotKey,
    };
    let Some(text) = (unsafe { app_ptr(hwnd) })
        .map(|app| unsafe { app.as_ref() }.settings.capture_hotkey.clone())
    else {
        return;
    };
    unsafe { UnregisterHotKey(hwnd, HOTKEY_ID) };
    if text.trim().is_empty() {
        return;
    }
    let Some(stroke) = super::keymap::KeyStroke::parse(&text) else {
        push_notice(
            hwnd,
            format!("capture_hotkey \"{text}\" is not a key FastPad knows."),
        );
        return;
    };
    let mut modifiers = MOD_NOREPEAT;
    if stroke.ctrl {
        modifiers |= MOD_CONTROL;
    }
    if stroke.shift {
        modifiers |= MOD_SHIFT;
    }
    if stroke.alt {
        modifiers |= MOD_ALT;
    }
    #[cfg(test)]
    let registered = {
        let _ = modifiers;
        true
    };
    #[cfg(not(test))]
    let registered = unsafe {
        windows_sys::Win32::UI::Input::KeyboardAndMouse::RegisterHotKey(
            hwnd,
            HOTKEY_ID,
            modifiers,
            u32::from(stroke.vk),
        )
    } != 0;
    if !registered {
        push_notice(
            hwnd,
            format!(
                "Another app is using {} for itself, so quick capture has no hotkey. Change capture_hotkey in fastpad.ini.",
                stroke.text()
            ),
        );
    }
}

pub(crate) fn unregister_hotkey(hwnd: HWND) {
    unsafe { windows_sys::Win32::UI::Input::KeyboardAndMouse::UnregisterHotKey(hwnd, HOTKEY_ID) };
}

/// A capture forwarded by `fastpad --capture` (spec §10.1): with nothing to file, the box opens;
/// otherwise the text and files are filed with no window, and a notice says where they went.
pub(crate) fn forwarded(hwnd: HWND, text: Option<String>, attachments: Vec<PathBuf>) {
    if text.is_none() && attachments.is_empty() {
        open_box(hwnd, None);
        return;
    }
    let capture = Submitted {
        text: text.unwrap_or_default(),
        attachments: attachments.into_iter().map(Source::File).collect(),
        target: default_target(hwnd),
        open: false,
    };
    match file(hwnd, &capture, None) {
        Ok((path, _)) => push_notice(
            hwnd,
            format!(
                "Captured to {}.",
                path.file_name().unwrap_or_default().to_string_lossy()
            ),
        ),
        Err(error) => push_notice(hwnd, error),
    }
}
