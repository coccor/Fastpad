//! The quick capture box (second brain phase 1 spec §10.2, §10.4): a small top-most window with
//! a multi-line edit, a strip of attachments, and keys to save, close, paste images and snip.
//! It files nothing itself: a `Sink` does, so the main window and a capture-only FastPad share
//! it. The first half of the file is pure layout and key logic.

use super::design::metrics::scale;
use super::design::type_ramp::{self, TextStyle};
use super::palette::Palette;
use crate::library::capture_file::{LARGE_FILE_BYTES, Source};
use crate::library::notebook_ini::CaptureTarget;
use crate::platform::wide_null;
use std::path::{Path, PathBuf};
use windows_sys::Win32::Foundation::{
    ERROR_CLASS_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows_sys::Win32::Graphics::Dwm::{
    DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND, DWMWCP_ROUND, DwmExtendFrameIntoClientArea,
    DwmSetWindowAttribute,
};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_RIGHT, DT_SINGLELINE,
    DT_VCENTER, DeleteObject, DrawTextW, EndPaint, FillRect, FrameRect, GetDC, GetMonitorInfoW,
    HBRUSH, HDC, HFONT, InvalidateRect, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint,
    PAINTSTRUCT, ReleaseDC, ScreenToClient, SelectObject, SetBkColor, SetBkMode, SetTextColor,
    TRANSPARENT,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::MARGINS;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetFocus, GetKeyState, SetFocus, VK_BACK, VK_CONTROL, VK_DELETE, VK_ESCAPE, VK_LEFT, VK_MENU,
    VK_RETURN, VK_RIGHT, VK_SHIFT, VK_SPACE, VK_TAB,
};
use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, ES_AUTOVSCROLL, ES_MULTILINE, ES_WANTRETURN,
    FindWindowW, GWLP_USERDATA, GetCursorPos, GetWindowLongPtrW, GetWindowTextLengthW,
    GetWindowTextW, HTCAPTION, HTCLIENT, IDC_ARROW, IsWindow, KillTimer, LoadCursorW,
    RegisterClassW, SW_HIDE, SW_SHOW, SWP_NOACTIVATE, SWP_NOZORDER, SetForegroundWindow, SetTimer,
    SetWindowLongPtrW, SetWindowPos, SetWindowTextW, ShowWindow, WM_APP, WM_CHAR, WM_CLOSE,
    WM_CTLCOLOREDIT, WM_DESTROY, WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONDOWN, WM_NCCALCSIZE,
    WM_NCDESTROY, WM_NCHITTEST, WM_PAINT, WM_PASTE, WM_SETFOCUS, WM_SETFONT, WM_TIMER, WNDCLASSW,
    WS_CAPTION, WS_CHILD, WS_CLIPCHILDREN, WS_EX_TOPMOST, WS_POPUP, WS_TABSTOP, WS_VISIBLE,
    WS_VSCROLL,
};

const WIDTH_AT_96_DPI: i32 = 520;
const PAD_AT_96_DPI: i32 = 12;
const TITLE_AT_96_DPI: i32 = 34;
const EDIT_AT_96_DPI: i32 = 92;
const CHIP_HEIGHT_AT_96_DPI: i32 = 30;
const CHIP_GAP_AT_96_DPI: i32 = 8;
const CHIP_MAX_AT_96_DPI: i32 = 220;
const CHIP_REMOVE_AT_96_DPI: i32 = 24;
const ERROR_AT_96_DPI: i32 = 22;
const FOOTER_AT_96_DPI: i32 = 30;

/// Posted to the box when the capture hotkey (or another `--capture`) arrives while it is open:
/// it cycles the target when the box is in front, and comes to the front otherwise.
pub(crate) const WM_CAPTURE_AGAIN: u32 = WM_APP + 0x60;
/// Files dropped on the box, as a `Box<Vec<PathBuf>>` in lparam.
const WM_CAPTURE_DROPPED: u32 = WM_APP + 0x61;
const SNIP_TIMER: usize = 0x4350_534E;
const SNIP_WAIT_MS: u32 = 30_000;
const EDIT_HOOK: usize = 0x4350_4544;
/// What the clipboard listener sends (`WM_CLIPBOARDUPDATE`).
const WM_CLIPBOARDUPDATE: u32 = 0x031D;

/// Where a capture goes, in the order Ctrl+Alt+N cycles through them.
pub(crate) fn next_target(target: CaptureTarget) -> CaptureTarget {
    match target {
        CaptureTarget::Inbox => CaptureTarget::Daily,
        CaptureTarget::Daily => CaptureTarget::Folder,
        CaptureTarget::Folder => CaptureTarget::Inbox,
    }
}

/// What a key in the box's edit does.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Action {
    /// Ctrl+Enter, or Ctrl+Shift+Enter to open the target afterwards.
    Submit {
        open: bool,
    },
    Close,
    /// Ctrl+Shift+V: the clipboard's image even when it has text too.
    AttachImage,
    Snip,
    CycleTarget,
    /// Tab: to the attachment strip.
    ToStrip,
}

pub(crate) fn key_action(key: u16, ctrl: bool, shift: bool, alt: bool) -> Option<Action> {
    match key {
        VK_RETURN if ctrl && !alt => Some(Action::Submit { open: shift }),
        VK_ESCAPE => Some(Action::Close),
        k if k == u16::from(b'V') && ctrl && shift && !alt => Some(Action::AttachImage),
        k if k == u16::from(b'S') && ctrl && shift && !alt => Some(Action::Snip),
        k if k == u16::from(b'N') && ctrl && alt => Some(Action::CycleTarget),
        VK_TAB if !ctrl && !alt => Some(Action::ToStrip),
        _ => None,
    }
}

/// What Ctrl+V does (spec §10.4): files copied in Explorer are attached; an image with no text
/// beside it is attached; anything else is pasted as text.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Paste {
    Files,
    Image,
    Text,
}

pub(crate) fn paste_kind(files: bool, image: bool, text: bool) -> Paste {
    if files {
        Paste::Files
    } else if image && !text {
        Paste::Image
    } else {
        Paste::Text
    }
}

/// The box's parts, in client pixels.
#[derive(Clone, Default)]
pub(crate) struct Layout {
    pub width: i32,
    pub height: i32,
    pub title: RECT,
    pub close: RECT,
    pub edit: RECT,
    /// One per attachment, wrapped onto as many rows as they need.
    pub chips: Vec<RECT>,
    pub error: Option<RECT>,
    pub footer: RECT,
    pub snip: RECT,
}

/// Lays the box out for chips `chip_widths` pixels wide (their text), with an error line or not.
pub(crate) fn layout(dpi: u32, chip_widths: &[i32], has_error: bool, snip_width: i32) -> Layout {
    let s = |v: i32| scale(v, dpi);
    let width = s(WIDTH_AT_96_DPI);
    let pad = s(PAD_AT_96_DPI);
    let title = RECT {
        left: 0,
        top: 0,
        right: width,
        bottom: s(TITLE_AT_96_DPI),
    };
    let close = RECT {
        left: width - s(TITLE_AT_96_DPI),
        ..title
    };
    let edit = RECT {
        left: pad,
        top: title.bottom,
        right: width - pad,
        bottom: title.bottom + s(EDIT_AT_96_DPI),
    };
    let mut y = edit.bottom + s(CHIP_GAP_AT_96_DPI);
    let mut x = pad;
    let mut chips = Vec::new();
    for text in chip_widths {
        let chip = (text + s(CHIP_REMOVE_AT_96_DPI) + s(PAD_AT_96_DPI)).min(s(CHIP_MAX_AT_96_DPI));
        if x > pad && x + chip > width - pad {
            x = pad;
            y += s(CHIP_HEIGHT_AT_96_DPI) + s(CHIP_GAP_AT_96_DPI);
        }
        chips.push(RECT {
            left: x,
            top: y,
            right: x + chip,
            bottom: y + s(CHIP_HEIGHT_AT_96_DPI),
        });
        x += chip + s(CHIP_GAP_AT_96_DPI);
    }
    if !chips.is_empty() {
        y += s(CHIP_HEIGHT_AT_96_DPI) + s(CHIP_GAP_AT_96_DPI);
    }
    let error = has_error.then(|| {
        let line = RECT {
            left: pad,
            top: y,
            right: width - pad,
            bottom: y + s(ERROR_AT_96_DPI),
        };
        y = line.bottom;
        line
    });
    let footer = RECT {
        left: pad,
        top: y,
        right: width - pad,
        bottom: y + s(FOOTER_AT_96_DPI),
    };
    let snip = RECT {
        left: pad,
        top: footer.top,
        right: pad + snip_width,
        bottom: footer.bottom,
    };
    Layout {
        width,
        height: footer.bottom + s(4),
        title,
        close,
        edit,
        chips,
        error,
        footer,
        snip,
    }
}

/// The text the title band shows, and the window's accessible name.
pub(crate) fn title_text(target_name: &str) -> String {
    format!("Capture \u{2192} {target_name}")
}

pub(crate) fn accessible_name(target_name: &str) -> String {
    format!("Quick capture to {target_name}")
}

/// An attachment's label in the strip.
pub(crate) fn chip_label(source: &Source, index: usize) -> String {
    match source {
        Source::Png(_) => format!("Image {}", index + 1),
        Source::File(path) => path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
    }
}

/// A capture ready to file.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Submitted {
    pub text: String,
    pub attachments: Vec<Source>,
    pub target: CaptureTarget,
    /// Ctrl+Shift+Enter: open the target with the caret on the new entry.
    pub open: bool,
}

/// Whoever files the box's captures.
pub(crate) trait Sink {
    /// What the title shows for `target` ("inbox.md", "today's note", "inbox\").
    fn target_name(&self, target: CaptureTarget) -> String;
    /// Files the capture; the error is shown in the box, which stays open.
    fn file(&mut self, capture: &Submitted) -> Result<(), String>;
    /// The box closed (saved or not).
    fn closed(&mut self) {}
}

// ---------------------------------------------------------------------------------------------
// Drafts (spec §10.2): text and images kept when the box closes unsaved.

/// `%LocalAppData%\FastPad\capture-draft`.
fn draft_dir() -> Option<PathBuf> {
    crate::platform::fastpad_data_dir()
        .ok()
        .map(|dir| dir.join("capture-draft"))
}

/// Keeps `text` and `attachments` for the next time the box opens. Dropped files are kept as
/// their paths.
pub(crate) fn save_draft(text: &str, attachments: &[Source]) {
    let Some(dir) = draft_dir() else {
        return;
    };
    clear_draft();
    if text.trim().is_empty() && attachments.is_empty() {
        return;
    }
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let _ = std::fs::write(dir.join("draft.txt"), text);
    let mut files = String::new();
    for (index, source) in attachments.iter().enumerate() {
        match source {
            Source::Png(bytes) => {
                let _ = std::fs::write(dir.join(format!("image-{index:03}.png")), bytes);
            }
            Source::File(path) => {
                files.push_str(&format!("{index:03}\t{}\n", path.display()));
            }
        }
    }
    let _ = std::fs::write(dir.join("files.txt"), files);
}

/// The draft left by the last box, if any.
pub(crate) fn load_draft() -> (String, Vec<Source>) {
    let Some(dir) = draft_dir() else {
        return (String::new(), Vec::new());
    };
    let text = std::fs::read_to_string(dir.join("draft.txt")).unwrap_or_default();
    let mut numbered: Vec<(String, Source)> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if let Some(index) = name
                .strip_prefix("image-")
                .and_then(|rest| rest.strip_suffix(".png"))
                && let Ok(bytes) = std::fs::read(entry.path())
            {
                numbered.push((index.to_owned(), Source::Png(bytes)));
            }
        }
    }
    if let Ok(files) = std::fs::read_to_string(dir.join("files.txt")) {
        for line in files.lines() {
            if let Some((index, path)) = line.split_once('\t') {
                numbered.push((index.to_owned(), Source::File(PathBuf::from(path))));
            }
        }
    }
    numbered.sort_by(|a, b| a.0.cmp(&b.0));
    (
        text,
        numbered.into_iter().map(|(_, source)| source).collect(),
    )
}

pub(crate) fn clear_draft() {
    if let Some(dir) = draft_dir() {
        let _ = std::fs::remove_dir_all(dir);
    }
}

// ---------------------------------------------------------------------------------------------
// The window.

struct CaptureBox {
    sink: Box<dyn Sink>,
    colors: Palette,
    dpi: u32,
    edit: HWND,
    edit_brush: HBRUSH,
    title_font: HFONT,
    body_font: HFONT,
    items: Vec<Source>,
    /// The strip's keyboard selection: an attachment, or `items.len()` for the Snip button.
    selected: usize,
    target: CaptureTarget,
    error: Option<String>,
    layout: Layout,
    snipping: bool,
    /// The WM_CHAR of a Ctrl+Shift+V that attached an image must not paste text too.
    swallow_paste_char: bool,
}

const SNIP_LABEL: &str = "+ Snip (Ctrl+Shift+S)";
const HINT: &str = "Ctrl+Enter save \u{00B7} Esc close";

fn class_name() -> &'static [u16] {
    static NAME: std::sync::OnceLock<Vec<u16>> = std::sync::OnceLock::new();
    NAME.get_or_init(|| wide_null("FastPadCapture"))
}

fn register_class() -> bool {
    static REGISTERED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *REGISTERED.get_or_init(|| {
        let class = WNDCLASSW {
            lpfnWndProc: Some(box_proc),
            hInstance: unsafe { GetModuleHandleW(std::ptr::null()) },
            hCursor: unsafe { LoadCursorW(std::ptr::null_mut(), IDC_ARROW) },
            hIcon: unsafe {
                windows_sys::Win32::UI::WindowsAndMessaging::LoadIconW(
                    GetModuleHandleW(std::ptr::null()),
                    1 as _,
                )
            },
            lpszClassName: class_name().as_ptr(),
            ..Default::default()
        };
        unsafe { RegisterClassW(&class) != 0 || GetLastError() == ERROR_CLASS_ALREADY_EXISTS }
    })
}

/// The open capture box in this session, if any (this process's or a capture-only one's).
pub(crate) fn find() -> Option<HWND> {
    let hwnd = unsafe { FindWindowW(class_name().as_ptr(), std::ptr::null()) };
    (!hwnd.is_null()).then_some(hwnd)
}

/// Whether `hwnd` is a capture box or one of its controls.
pub(crate) fn owns(hwnd: HWND) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GA_ROOT, GetAncestor};
    if hwnd.is_null() {
        return false;
    }
    let root = unsafe { GetAncestor(hwnd, GA_ROOT) };
    !root.is_null() && unsafe { GetWindowLongPtrW(root, GWLP_USERDATA) } != 0 && is_box(root)
}

fn is_box(hwnd: HWND) -> bool {
    let mut name = [0_u16; 32];
    let length = unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::GetClassNameW(
            hwnd,
            name.as_mut_ptr(),
            name.len() as i32,
        )
    }
    .max(0) as usize;
    name[..length] == class_name()[..class_name().len() - 1]
}

fn state(hwnd: HWND) -> Option<&'static mut CaptureBox> {
    let pointer = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut CaptureBox;
    (!pointer.is_null()).then(|| unsafe { &mut *pointer })
}

/// Opens the box centered on the monitor under the mouse, with the last draft, filing through
/// `sink` to `target` first. Returns the window.
pub(crate) fn open(colors: Palette, target: CaptureTarget, sink: Box<dyn Sink>) -> Option<HWND> {
    if !register_class() {
        return None;
    }
    let mut cursor = POINT::default();
    unsafe { GetCursorPos(&mut cursor) };
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_TOPMOST,
            class_name().as_ptr(),
            std::ptr::null(),
            // WS_CAPTION gives the DWM shadow; WM_NCCALCSIZE hides the frame, as prompts do.
            WS_POPUP | WS_CAPTION | WS_CLIPCHILDREN,
            cursor.x,
            cursor.y,
            1,
            1,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            GetModuleHandleW(std::ptr::null()),
            std::ptr::null(),
        )
    };
    if hwnd.is_null() {
        return None;
    }
    let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
    let edit = unsafe {
        CreateWindowExW(
            0,
            wide_null("EDIT").as_ptr(),
            std::ptr::null(),
            WS_CHILD
                | WS_VISIBLE
                | WS_TABSTOP
                | WS_VSCROLL
                | (ES_MULTILINE | ES_AUTOVSCROLL | ES_WANTRETURN) as u32,
            0,
            0,
            0,
            0,
            hwnd,
            std::ptr::null_mut(),
            GetModuleHandleW(std::ptr::null()),
            std::ptr::null(),
        )
    };
    let body_font = type_ramp::create(TextStyle::Body, dpi);
    let title_font = type_ramp::create(TextStyle::Heading, dpi);
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::SendMessageW(
            edit,
            WM_SETFONT,
            body_font as usize,
            0,
        );
        SetWindowSubclass(edit, Some(edit_proc), EDIT_HOOK, hwnd as usize);
    }
    crate::window::titlebar::apply_scrollbar_theme(edit, colors.dark_frame);
    let (text, items) = load_draft();
    let state = Box::new(CaptureBox {
        sink,
        colors,
        dpi,
        edit,
        edit_brush: unsafe { CreateSolidBrush(colors.editor_background) },
        title_font,
        body_font,
        items,
        selected: 0,
        target,
        error: None,
        layout: Layout::default(),
        snipping: false,
        swallow_paste_char: false,
    });
    unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(state) as isize) };
    set_edit_text(edit, &text);
    let dropper = hwnd as isize;
    let _ = crate::editor::file_drop::accept_file_drops_on(
        hwnd,
        move |paths, _| {
            let payload = Box::into_raw(Box::new(paths));
            if unsafe {
                windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(
                    dropper as HWND,
                    WM_CAPTURE_DROPPED,
                    0,
                    payload as isize,
                )
            } == 0
            {
                drop(unsafe { Box::from_raw(payload) });
            }
        },
        |_| {},
        || {},
    );
    relayout(hwnd);
    place(hwnd, cursor);
    unsafe {
        let corners = if colors.high_contrast {
            DWMWCP_DONOTROUND
        } else {
            DWMWCP_ROUND
        };
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE as u32,
            (&raw const corners).cast(),
            std::mem::size_of_val(&corners) as u32,
        );
        let margins = MARGINS {
            cxLeftWidth: 1,
            cxRightWidth: 1,
            cyTopHeight: 1,
            cyBottomHeight: 1,
        };
        DwmExtendFrameIntoClientArea(hwnd, &margins);
    }
    crate::window::titlebar::apply_frame_theme(hwnd, colors.dark_frame);
    show(hwnd);
    let _ = dpi;
    Some(hwnd)
}

fn show(hwnd: HWND) {
    unsafe {
        ShowWindow(hwnd, SW_SHOW);
        SetForegroundWindow(hwnd);
    }
    if let Some(capture) = state(hwnd) {
        let edit = capture.edit;
        unsafe {
            SetFocus(edit);
            // The caret goes to the end of a restored draft.
            windows_sys::Win32::UI::WindowsAndMessaging::SendMessageW(
                edit,
                windows_sys::Win32::UI::Controls::EM_SETSEL,
                usize::MAX,
                -1,
            );
        }
    }
}

/// Centers the box on the work area of the monitor at `point`.
fn place(hwnd: HWND, point: POINT) {
    let Some(capture) = state(hwnd) else {
        return;
    };
    let (width, height) = (capture.layout.width, capture.layout.height);
    let mut monitor = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    unsafe {
        GetMonitorInfoW(
            MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST),
            &mut monitor,
        )
    };
    let work = monitor.rcWork;
    let left = work.left + ((work.right - work.left) - width).max(0) / 2;
    let top = work.top + ((work.bottom - work.top) - height).max(0) / 3;
    unsafe {
        SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            left,
            top,
            width,
            height,
            SWP_NOZORDER | SWP_NOACTIVATE,
        )
    };
}

fn text_width(hwnd: HWND, font: HFONT, text: &str) -> i32 {
    let wide: Vec<u16> = text.encode_utf16().collect();
    let mut rect = RECT::default();
    unsafe {
        let dc = GetDC(hwnd);
        let old = SelectObject(dc, font);
        DrawTextW(
            dc,
            wide.as_ptr(),
            wide.len() as i32,
            &mut rect,
            windows_sys::Win32::Graphics::Gdi::DT_CALCRECT | DT_SINGLELINE | DT_NOPREFIX,
        );
        SelectObject(dc, old);
        ReleaseDC(hwnd, dc);
    }
    rect.right - rect.left
}

/// Lays the box out again for its attachments and error, keeping its top-left corner.
fn relayout(hwnd: HWND) {
    let Some(capture) = state(hwnd) else {
        return;
    };
    let widths: Vec<i32> = capture
        .items
        .iter()
        .enumerate()
        .map(|(index, item)| text_width(hwnd, capture.body_font, &chip_label(item, index)))
        .collect();
    let snip = text_width(hwnd, capture.body_font, SNIP_LABEL) + scale(8, capture.dpi);
    capture.layout = layout(capture.dpi, &widths, capture.error.is_some(), snip);
    capture.selected = capture.selected.min(capture.items.len());
    let edit = capture.layout.edit;
    let (width, height) = (capture.layout.width, capture.layout.height);
    let name = capture.sink.target_name(capture.target);
    unsafe {
        SetWindowPos(
            capture.edit,
            std::ptr::null_mut(),
            edit.left,
            edit.top,
            edit.right - edit.left,
            edit.bottom - edit.top,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
        let mut frame = RECT::default();
        windows_sys::Win32::UI::WindowsAndMessaging::GetWindowRect(hwnd, &mut frame);
        SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            frame.left,
            frame.top,
            width,
            height,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
        SetWindowTextW(hwnd, wide_null(&accessible_name(&name)).as_ptr());
        InvalidateRect(hwnd, std::ptr::null(), 1);
    }
}

fn set_edit_text(edit: HWND, text: &str) {
    unsafe { SetWindowTextW(edit, wide_null(text).as_ptr()) };
}

fn edit_text(edit: HWND) -> String {
    let length = unsafe { GetWindowTextLengthW(edit) }.max(0) as usize;
    let mut buffer = vec![0_u16; length + 1];
    let copied = unsafe { GetWindowTextW(edit, buffer.as_mut_ptr(), buffer.len() as i32) };
    String::from_utf16_lossy(&buffer[..copied.max(0) as usize])
}

/// Shows `error` in the box (or clears it).
fn set_error(hwnd: HWND, error: Option<String>) {
    if let Some(capture) = state(hwnd) {
        capture.error = error;
    }
    relayout(hwnd);
}

/// Attaches `source`; a file over 20 MB asks first.
fn attach(hwnd: HWND, source: Source) {
    if let Source::File(path) = &source {
        if path.is_dir() {
            set_error(
                hwnd,
                Some(format!(
                    "{} is a folder. Only files can be attached.",
                    path.display()
                )),
            );
            return;
        }
        let size = std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0);
        if size > LARGE_FILE_BYTES && !confirm_large(hwnd, path, size) {
            return;
        }
    }
    if let Some(capture) = state(hwnd) {
        capture.items.push(source);
        capture.error = None;
    }
    relayout(hwnd);
}

fn confirm_large(hwnd: HWND, path: &Path, size: u64) -> bool {
    let Some(colors) = state(hwnd).map(|capture| capture.colors) else {
        return false;
    };
    let message = format!(
        "{} is {} MB. Copy it into the notebook anyway?",
        path.file_name().unwrap_or_default().to_string_lossy(),
        size.div_ceil(1024 * 1024)
    );
    let spec = super::prompt::Spec {
        message: &message,
        buttons: &["Copy it", "Leave it out"],
        quick_keys: &[],
    };
    super::prompt::show(hwnd, colors, &spec) == 0
}

/// Ctrl+V and the edit's Paste: files and lone images are attached (spec §10.4). Returns
/// whether the paste was taken, so the edit doesn't paste text too.
fn paste(hwnd: HWND, force_image: bool) -> bool {
    use crate::platform::clipboard;
    let kind = if force_image {
        if clipboard::has_image() {
            Paste::Image
        } else {
            return false;
        }
    } else {
        paste_kind(
            clipboard::has_files(),
            clipboard::has_image(),
            clipboard::has_text(),
        )
    };
    match kind {
        Paste::Text => false,
        Paste::Files => {
            for path in clipboard::files(hwnd) {
                attach(hwnd, Source::File(path));
            }
            true
        }
        Paste::Image => {
            match clipboard::image_png(hwnd) {
                Some(Ok(png)) => attach(hwnd, Source::Png(png)),
                Some(Err(error)) => set_error(hwnd, Some(error)),
                None => {}
            }
            true
        }
    }
}

fn remove_item(hwnd: HWND, index: usize) {
    if let Some(capture) = state(hwnd)
        && index < capture.items.len()
    {
        capture.items.remove(index);
    }
    relayout(hwnd);
}

fn cycle_target(hwnd: HWND) {
    if let Some(capture) = state(hwnd) {
        capture.target = next_target(capture.target);
    }
    relayout(hwnd);
}

#[cfg(test)]
thread_local! {
    static SNIPS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// How many snips tests started.
#[cfg(test)]
#[allow(
    dead_code,
    reason = "read by the lib window tests, not by the source-linked integration targets"
)]
pub(crate) fn take_snips() -> usize {
    SNIPS.with(|snips| snips.replace(0))
}

/// Snip (spec §10.4): the box hides, Windows' own snipping overlay starts, and the next image
/// on the clipboard is attached when the box comes back. 30 seconds without one brings it back
/// empty-handed.
fn start_snip(hwnd: HWND) {
    if let Some(capture) = state(hwnd) {
        capture.snipping = true;
    }
    unsafe {
        ShowWindow(hwnd, SW_HIDE);
        AddClipboardFormatListener(hwnd);
        SetTimer(hwnd, SNIP_TIMER, SNIP_WAIT_MS, None);
    }
    #[cfg(test)]
    SNIPS.with(|snips| snips.set(snips.get() + 1));
    #[cfg(not(test))]
    unsafe {
        windows_sys::Win32::UI::Shell::ShellExecuteW(
            std::ptr::null_mut(),
            wide_null("open").as_ptr(),
            wide_null("ms-screenclip:").as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL,
        );
    }
}

fn end_snip(hwnd: HWND) {
    let was = state(hwnd).is_some_and(|capture| std::mem::take(&mut capture.snipping));
    if !was {
        return;
    }
    unsafe {
        KillTimer(hwnd, SNIP_TIMER);
        RemoveClipboardFormatListener(hwnd);
    }
    show(hwnd);
}

#[link(name = "user32")]
unsafe extern "system" {
    fn AddClipboardFormatListener(hwnd: HWND) -> i32;
    fn RemoveClipboardFormatListener(hwnd: HWND) -> i32;
}

/// Files the capture; on success the draft goes and the box closes.
pub(crate) fn submit(hwnd: HWND, open: bool) {
    let Some(capture) = state(hwnd) else {
        return;
    };
    let text = edit_text(capture.edit);
    if text.trim().is_empty() && capture.items.is_empty() {
        close(hwnd);
        return;
    }
    let submitted = Submitted {
        text,
        attachments: capture.items.clone(),
        target: capture.target,
        open,
    };
    match capture.sink.file(&submitted) {
        Ok(()) => {
            clear_draft();
            if let Some(capture) = state(hwnd) {
                capture.items.clear();
                set_edit_text(capture.edit, "");
            }
            unsafe { DestroyWindow(hwnd) };
        }
        Err(error) => {
            save_draft(&submitted.text, &submitted.attachments);
            set_error(hwnd, Some(error));
        }
    }
}

/// Esc: closes the box, keeping what it holds as a draft.
pub(crate) fn close(hwnd: HWND) {
    if let Some(capture) = state(hwnd) {
        save_draft(&edit_text(capture.edit), &capture.items);
    }
    unsafe { DestroyWindow(hwnd) };
}

fn modifiers() -> (bool, bool, bool) {
    let down = |key: u16| unsafe { GetKeyState(i32::from(key)) } < 0;
    (down(VK_CONTROL), down(VK_SHIFT), down(VK_MENU))
}

fn run(hwnd: HWND, action: Action) {
    match action {
        Action::Submit { open } => submit(hwnd, open),
        Action::Close => close(hwnd),
        Action::AttachImage => {
            if paste(hwnd, true)
                && let Some(capture) = state(hwnd)
            {
                capture.swallow_paste_char = true;
            }
        }
        Action::Snip => start_snip(hwnd),
        Action::CycleTarget => cycle_target(hwnd),
        Action::ToStrip => {
            if let Some(capture) = state(hwnd) {
                capture.selected = 0;
            }
            unsafe { SetFocus(hwnd) };
            invalidate(hwnd);
        }
    }
}

fn invalidate(hwnd: HWND) {
    unsafe { InvalidateRect(hwnd, std::ptr::null(), 0) };
}

unsafe extern "system" fn edit_proc(
    edit: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    owner: usize,
) -> LRESULT {
    let hwnd = owner as HWND;
    match message {
        WM_KEYDOWN => {
            let (ctrl, shift, alt) = modifiers();
            if let Some(action) = key_action(wparam as u16, ctrl, shift, alt) {
                run(hwnd, action);
                return 0;
            }
        }
        WM_CHAR => {
            let ch = wparam as u32;
            // Ctrl+Enter's line feed, Esc and Tab never reach the text.
            if ch == 0x0A || ch == 0x1B || ch == 0x09 {
                return 0;
            }
            if ch == 0x16
                && let Some(capture) = state(hwnd)
                && std::mem::take(&mut capture.swallow_paste_char)
            {
                return 0;
            }
        }
        WM_PASTE => {
            if paste(hwnd, false) {
                return 0;
            }
        }
        WM_NCDESTROY => unsafe {
            RemoveWindowSubclass(edit, Some(edit_proc), EDIT_HOOK);
        },
        _ => {}
    }
    unsafe { DefSubclassProc(edit, message, wparam, lparam) }
}

fn draw_text(dc: HDC, text: &str, rect: &RECT, format: u32) {
    let wide: Vec<u16> = text.encode_utf16().collect();
    let mut rect = *rect;
    unsafe {
        DrawTextW(
            dc,
            wide.as_ptr(),
            wide.len() as i32,
            &mut rect,
            format | DT_NOPREFIX | DT_SINGLELINE | DT_VCENTER,
        )
    };
}

fn fill(dc: HDC, rect: &RECT, color: u32) {
    unsafe {
        let brush = CreateSolidBrush(color);
        FillRect(dc, rect, brush);
        DeleteObject(brush);
    }
}

fn frame(dc: HDC, rect: &RECT, color: u32) {
    unsafe {
        let brush = CreateSolidBrush(color);
        FrameRect(dc, rect, brush);
        DeleteObject(brush);
    }
}

fn paint(hwnd: HWND) {
    let Some(capture) = state(hwnd) else {
        return;
    };
    let mut ps = PAINTSTRUCT::default();
    let dc = unsafe { BeginPaint(hwnd, &mut ps) };
    let colors = capture.colors;
    let layout = capture.layout.clone();
    let client = RECT {
        left: 0,
        top: 0,
        right: layout.width,
        bottom: layout.height,
    };
    let strip_focused = unsafe { GetFocus() } == hwnd;
    fill(dc, &client, colors.strip_background);
    unsafe { SetBkMode(dc, TRANSPARENT as i32) };
    let pad = scale(PAD_AT_96_DPI, capture.dpi);
    let old = unsafe { SelectObject(dc, capture.title_font) };
    unsafe { SetTextColor(dc, colors.editor_foreground) };
    let name = capture.sink.target_name(capture.target);
    draw_text(
        dc,
        &title_text(&name),
        &RECT {
            left: pad,
            ..layout.title
        },
        DT_LEFT | DT_END_ELLIPSIS,
    );
    unsafe { SelectObject(dc, capture.body_font) };
    unsafe { SetTextColor(dc, colors.muted_foreground) };
    draw_text(
        dc,
        "\u{2715}",
        &layout.close,
        windows_sys::Win32::Graphics::Gdi::DT_CENTER,
    );
    for (index, chip) in layout.chips.iter().enumerate() {
        let selected = strip_focused && capture.selected == index;
        fill(
            dc,
            chip,
            if selected {
                colors.selection_background
            } else {
                colors.editor_background
            },
        );
        frame(
            dc,
            chip,
            if selected {
                colors.accent
            } else {
                colors.stroke
            },
        );
        let remove = scale(CHIP_REMOVE_AT_96_DPI, capture.dpi);
        unsafe { SetTextColor(dc, colors.editor_foreground) };
        if let Some(item) = capture.items.get(index) {
            draw_text(
                dc,
                &chip_label(item, index),
                &RECT {
                    left: chip.left + pad / 2,
                    right: chip.right - remove,
                    ..*chip
                },
                DT_LEFT | DT_END_ELLIPSIS,
            );
        }
        unsafe { SetTextColor(dc, colors.muted_foreground) };
        draw_text(
            dc,
            "\u{2715}",
            &RECT {
                left: chip.right - remove,
                ..*chip
            },
            windows_sys::Win32::Graphics::Gdi::DT_CENTER,
        );
    }
    if let (Some(rect), Some(error)) = (layout.error, capture.error.as_deref()) {
        unsafe { SetTextColor(dc, colors.error_foreground) };
        draw_text(dc, error, &rect, DT_LEFT | DT_END_ELLIPSIS);
    }
    let snip_selected = strip_focused && capture.selected == capture.items.len();
    if snip_selected {
        frame(dc, &layout.snip, colors.accent);
    }
    unsafe { SetTextColor(dc, colors.accent) };
    draw_text(
        dc,
        SNIP_LABEL,
        &RECT {
            left: layout.snip.left + scale(4, capture.dpi),
            ..layout.snip
        },
        DT_LEFT,
    );
    unsafe { SetTextColor(dc, colors.muted_foreground) };
    draw_text(dc, HINT, &layout.footer, DT_RIGHT);
    unsafe {
        SelectObject(dc, old);
        EndPaint(hwnd, &ps);
    }
}

fn contains(rect: &RECT, x: i32, y: i32) -> bool {
    x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom
}

fn clicked(hwnd: HWND, x: i32, y: i32) {
    let Some(capture) = state(hwnd) else {
        return;
    };
    let layout = capture.layout.clone();
    if contains(&layout.close, x, y) {
        close(hwnd);
        return;
    }
    if contains(&layout.snip, x, y) {
        start_snip(hwnd);
        return;
    }
    let remove = scale(CHIP_REMOVE_AT_96_DPI, capture.dpi);
    if let Some(index) = layout.chips.iter().position(|chip| {
        contains(
            &RECT {
                left: chip.right - remove,
                ..*chip
            },
            x,
            y,
        )
    }) {
        remove_item(hwnd, index);
    }
}

/// Keys while the strip (the box itself) has the focus.
fn strip_key(hwnd: HWND, key: u16) {
    let Some(capture) = state(hwnd) else {
        return;
    };
    let count = capture.items.len();
    let (ctrl, shift, _) = modifiers();
    match key {
        VK_TAB => unsafe {
            SetFocus(capture.edit);
        },
        VK_LEFT => capture.selected = capture.selected.saturating_sub(1),
        VK_RIGHT => capture.selected = (capture.selected + 1).min(count),
        VK_DELETE | VK_BACK if capture.selected < count => remove_item(hwnd, capture.selected),
        VK_RETURN if ctrl => submit(hwnd, shift),
        VK_RETURN | VK_SPACE if capture.selected == count => start_snip(hwnd),
        VK_ESCAPE => close(hwnd),
        _ => {}
    }
    if unsafe { IsWindow(hwnd) } != 0 {
        invalidate(hwnd);
    }
}

unsafe extern "system" fn box_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } == 0 {
        return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
    }
    match message {
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            paint(hwnd);
            0
        }
        WM_NCCALCSIZE => 0,
        WM_NCHITTEST => {
            let mut point = POINT {
                x: (lparam & 0xFFFF) as i16 as i32,
                y: ((lparam >> 16) & 0xFFFF) as i16 as i32,
            };
            unsafe { ScreenToClient(hwnd, &mut point) };
            let drags = state(hwnd).is_some_and(|capture| {
                contains(&capture.layout.title, point.x, point.y)
                    && !contains(&capture.layout.close, point.x, point.y)
            });
            if drags {
                HTCAPTION as LRESULT
            } else {
                HTCLIENT as LRESULT
            }
        }
        WM_LBUTTONDOWN => {
            clicked(
                hwnd,
                (lparam & 0xFFFF) as i16 as i32,
                ((lparam >> 16) & 0xFFFF) as i16 as i32,
            );
            0
        }
        WM_KEYDOWN => {
            strip_key(hwnd, wparam as u16);
            0
        }
        WM_SETFOCUS => {
            invalidate(hwnd);
            0
        }
        WM_CTLCOLOREDIT => {
            let Some(capture) = state(hwnd) else {
                return 0;
            };
            let dc = wparam as HDC;
            unsafe {
                SetTextColor(dc, capture.colors.editor_foreground);
                SetBkColor(dc, capture.colors.editor_background);
            }
            capture.edit_brush as LRESULT
        }
        WM_CAPTURE_AGAIN => {
            let snipping = state(hwnd).is_some_and(|capture| capture.snipping);
            if snipping {
                // The hotkey during a snip brings the box back without an image.
                end_snip(hwnd);
            } else if unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow() }
                == hwnd
            {
                cycle_target(hwnd);
            } else {
                show(hwnd);
            }
            0
        }
        WM_CAPTURE_DROPPED => {
            if lparam != 0 {
                let paths = unsafe { Box::from_raw(lparam as *mut Vec<PathBuf>) };
                for path in *paths {
                    attach(hwnd, Source::File(path));
                }
            }
            0
        }
        WM_CLIPBOARDUPDATE => {
            if state(hwnd).is_some_and(|capture| capture.snipping)
                && crate::platform::clipboard::has_image()
            {
                end_snip(hwnd);
                let _ = paste(hwnd, true);
            }
            0
        }
        WM_TIMER if wparam == SNIP_TIMER => {
            end_snip(hwnd);
            0
        }
        WM_CLOSE => {
            close(hwnd);
            0
        }
        WM_DESTROY => {
            crate::editor::file_drop::revoke_file_drops(hwnd);
            unsafe {
                KillTimer(hwnd, SNIP_TIMER);
                RemoveClipboardFormatListener(hwnd);
            }
            if let Some(capture) = state(hwnd) {
                capture.sink.closed();
            }
            0
        }
        WM_NCDESTROY => {
            let pointer = unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) } as *mut CaptureBox;
            if !pointer.is_null() {
                let capture = unsafe { Box::from_raw(pointer) };
                unsafe {
                    DeleteObject(capture.edit_brush);
                    DeleteObject(capture.title_font);
                    DeleteObject(capture.body_font);
                }
            }
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

// ---------------------------------------------------------------------------------------------
// For tests.

#[cfg(test)]
#[allow(
    dead_code,
    reason = "read by the lib window tests, not by the source-linked integration targets"
)]
pub(crate) fn edit_hwnd(hwnd: HWND) -> Option<HWND> {
    state(hwnd).map(|capture| capture.edit)
}

#[cfg(test)]
#[allow(
    dead_code,
    reason = "read by the lib window tests, not by the source-linked integration targets"
)]
pub(crate) fn set_text_for_test(hwnd: HWND, text: &str) {
    if let Some(capture) = state(hwnd) {
        set_edit_text(capture.edit, text);
    }
}

#[cfg(test)]
#[allow(
    dead_code,
    reason = "read by the lib window tests, not by the source-linked integration targets"
)]
pub(crate) fn attach_for_test(hwnd: HWND, source: Source) {
    attach(hwnd, source);
}

#[cfg(test)]
#[allow(
    dead_code,
    reason = "read by the lib window tests, not by the source-linked integration targets"
)]
pub(crate) fn shown(hwnd: HWND) -> (String, usize, Option<String>, CaptureTarget) {
    state(hwnd).map_or_else(
        || (String::new(), 0, None, CaptureTarget::Inbox),
        |capture| {
            (
                edit_text(capture.edit),
                capture.items.len(),
                capture.error.clone(),
                capture.target,
            )
        },
    )
}

#[cfg(test)]
#[allow(
    dead_code,
    reason = "read by the lib window tests, not by the source-linked integration targets"
)]
pub(crate) fn again_for_test(hwnd: HWND) {
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::SendMessageW(hwnd, WM_CAPTURE_AGAIN, 0, 0)
    };
}

#[cfg(test)]
#[allow(
    dead_code,
    reason = "read by the lib window tests, not by the source-linked integration targets"
)]
pub(crate) fn key_for_test(hwnd: HWND, action: Action) {
    run(hwnd, action);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_map_to_the_box_actions() {
        assert_eq!(
            key_action(VK_RETURN, true, false, false),
            Some(Action::Submit { open: false })
        );
        assert_eq!(
            key_action(VK_RETURN, true, true, false),
            Some(Action::Submit { open: true })
        );
        assert_eq!(
            key_action(VK_RETURN, false, false, false),
            None,
            "Enter is a new line"
        );
        assert_eq!(
            key_action(VK_ESCAPE, false, false, false),
            Some(Action::Close)
        );
        assert_eq!(
            key_action(u16::from(b'V'), true, true, false),
            Some(Action::AttachImage)
        );
        assert_eq!(
            key_action(u16::from(b'V'), true, false, false),
            None,
            "the edit pastes"
        );
        assert_eq!(
            key_action(u16::from(b'S'), true, true, false),
            Some(Action::Snip)
        );
        assert_eq!(
            key_action(u16::from(b'N'), true, false, true),
            Some(Action::CycleTarget)
        );
        assert_eq!(
            key_action(VK_TAB, false, false, false),
            Some(Action::ToStrip)
        );
    }

    #[test]
    fn ctrl_v_attaches_files_and_lone_images_and_pastes_text_otherwise() {
        assert_eq!(paste_kind(true, false, true), Paste::Files);
        assert_eq!(paste_kind(false, true, false), Paste::Image);
        assert_eq!(
            paste_kind(false, true, true),
            Paste::Text,
            "Word puts a picture beside copied text"
        );
        assert_eq!(paste_kind(false, false, true), Paste::Text);
    }

    #[test]
    fn targets_cycle_inbox_daily_folder() {
        assert_eq!(next_target(CaptureTarget::Inbox), CaptureTarget::Daily);
        assert_eq!(next_target(CaptureTarget::Daily), CaptureTarget::Folder);
        assert_eq!(next_target(CaptureTarget::Folder), CaptureTarget::Inbox);
    }

    #[test]
    fn the_strip_appears_only_with_attachments_and_wraps() {
        let plain = layout(96, &[], false, 120);
        assert_eq!(plain.width, 520);
        assert!(plain.chips.is_empty());
        assert_eq!(plain.footer.top, plain.edit.bottom + 8);

        let with = layout(96, &[200, 200, 200], true, 120);
        assert_eq!(with.chips.len(), 3);
        assert_eq!(with.chips[0].top, with.chips[1].top);
        assert!(with.chips[2].top > with.chips[1].top, "the third wraps");
        assert!(with.chips.iter().all(|chip| chip.right <= 520 - 12));
        let error = with.error.unwrap();
        assert!(error.top >= with.chips[2].bottom);
        assert!(with.height > plain.height);

        let big = layout(192, &[], false, 240);
        assert_eq!(big.width, 1040);
    }

    #[test]
    fn the_title_and_name_say_where_the_capture_goes() {
        assert_eq!(title_text("inbox.md"), "Capture \u{2192} inbox.md");
        assert_eq!(accessible_name("inbox.md"), "Quick capture to inbox.md");
        assert_eq!(chip_label(&Source::Png(Vec::new()), 1), "Image 2");
        assert_eq!(
            chip_label(&Source::File(PathBuf::from(r"C:\logs\ledger.log")), 0),
            "ledger.log"
        );
    }
}
