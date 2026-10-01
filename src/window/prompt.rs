//! The themed prompt that replaces `MessageBoxW` for the close and confirm questions: an owned
//! popup in the theme's colors with action-named buttons, running its own modal loop like
//! `about.rs`. This file's first half is pure layout and key logic.

use super::design::metrics::{CONTROL_RADIUS, FOCUS_GAP, FOCUS_RING, scale};
use super::design::text_scale::scale_text;
use super::design::type_ramp::{self, TextStyle};
use super::palette::Palette;
use super::panel::{inset, text_height};
use super::side_panel::paint_buffered;
use super::soft_paint::{
    Canvas, Frame, Shape, TITLE_CLOSE_WIDTH_AT_96_DPI, TITLE_HEIGHT_AT_96_DPI, Tones,
    glyph_font_face, title_close,
};
use super::titlebar::create_ui_font;
use crate::platform::wide_null;
use std::cell::Cell;
use windows_sys::Win32::Foundation::{
    ERROR_CLASS_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows_sys::Win32::Graphics::Dwm::{
    DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND, DWMWCP_ROUND, DwmExtendFrameIntoClientArea,
    DwmSetWindowAttribute,
};
use windows_sys::Win32::Graphics::Gdi::{
    DT_CALCRECT, DT_CENTER, DT_EDITCONTROL, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_SINGLELINE,
    DT_WORDBREAK, DeleteObject, DrawTextW, FW_NORMAL, GetDC, GetMonitorInfoW, HDC, HFONT,
    InvalidateRect, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow, ReleaseDC,
    ScreenToClient, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::{MARGINS, WM_MOUSELEAVE};
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    EnableWindow, GetKeyState, IsWindowEnabled, ReleaseCapture, SetCapture, SetFocus, TME_LEAVE,
    TRACKMOUSEEVENT, TrackMouseEvent, VK_ESCAPE, VK_RETURN, VK_SHIFT, VK_SPACE, VK_TAB,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GW_OWNER, GWLP_USERDATA,
    GetMessageW, GetWindow, GetWindowLongPtrW, GetWindowRect, HCURSOR, HTCAPTION, HTCLIENT,
    IDC_ARROW, IsIconic, IsWindow, LoadCursorW, MSG, PostQuitMessage, RegisterClassW, SW_SHOW,
    SWP_NOACTIVATE, SWP_NOZORDER, SetCursor, SetWindowLongPtrW, SetWindowPos, ShowWindow,
    TranslateMessage, WM_CLOSE, WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MOUSEMOVE, WM_NCCALCSIZE, WM_NCDESTROY, WM_NCHITTEST, WM_PAINT, WM_SETCURSOR, WNDCLASSW,
    WS_CAPTION, WS_CLIPCHILDREN, WS_EX_TOOLWINDOW, WS_POPUP,
};

const WIDTH_AT_96_DPI: i32 = 400;
const PADDING_AT_96_DPI: i32 = 20;
const TITLE_PAD_AT_96_DPI: i32 = 10;
const MESSAGE_GAP_AT_96_DPI: i32 = 12;
const FOOTER_GAP_AT_96_DPI: i32 = 20;
const FOOTER_HEIGHT_AT_96_DPI: i32 = 56;
const BUTTON_MIN_WIDTH_AT_96_DPI: i32 = 88;
const BUTTON_HEIGHT_AT_96_DPI: i32 = 30;
const BUTTON_PAD_AT_96_DPI: i32 = 16;
const BUTTON_GAP_AT_96_DPI: i32 = 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Target {
    Button(usize),
    TitleClose,
}

fn button_widths(dpi: u32, label_widths: &[i32]) -> Vec<i32> {
    label_widths
        .iter()
        .map(|width| {
            scale(BUTTON_MIN_WIDTH_AT_96_DPI, dpi).max(width + 2 * scale(BUTTON_PAD_AT_96_DPI, dpi))
        })
        .collect()
}

fn buttons_total(dpi: u32, widths: &[i32]) -> i32 {
    widths.iter().sum::<i32>() + scale(BUTTON_GAP_AT_96_DPI, dpi) * (widths.len() as i32 - 1).max(0)
}

/// The dialog's width: 400px at 96 DPI, or wider when the buttons need more.
pub(crate) fn dialog_width(dpi: u32, label_widths: &[i32]) -> i32 {
    let needed =
        buttons_total(dpi, &button_widths(dpi, label_widths)) + 2 * scale(PADDING_AT_96_DPI, dpi);
    scale(WIDTH_AT_96_DPI, dpi).max(needed)
}

/// The width the message wraps to.
pub(crate) fn content_width(dpi: u32, label_widths: &[i32]) -> i32 {
    dialog_width(dpi, label_widths) - 2 * scale(PADDING_AT_96_DPI, dpi)
}

#[derive(Clone)]
pub(crate) struct Layout {
    pub width: i32,
    pub height: i32,
    pub title_band: RECT,
    pub title: RECT,
    pub title_close: RECT,
    pub message: RECT,
    pub footer: RECT,
    pub buttons: Vec<RECT>,
    pub(crate) dpi: u32,
}

impl Layout {
    /// `width` is `dialog_width`; `title_height` and `message_height` are the measured text heights.
    pub(crate) fn calculate(
        dpi: u32,
        width: i32,
        title_height: i32,
        message_height: i32,
        label_widths: &[i32],
    ) -> Self {
        let padding = scale(PADDING_AT_96_DPI, dpi);
        let band_height = scale_text(TITLE_HEIGHT_AT_96_DPI, dpi)
            .max(title_height + 2 * scale(TITLE_PAD_AT_96_DPI, dpi));
        let title_band = RECT {
            left: 0,
            top: 0,
            right: width,
            bottom: band_height,
        };
        let title_close = RECT {
            left: width - scale(TITLE_CLOSE_WIDTH_AT_96_DPI, dpi),
            top: 0,
            right: width,
            bottom: scale_text(TITLE_HEIGHT_AT_96_DPI, dpi).min(band_height),
        };
        let title_top = (band_height - title_height) / 2;
        let title = RECT {
            left: padding,
            top: title_top,
            right: title_close.left,
            bottom: title_top + title_height,
        };
        let message_top = band_height + scale(MESSAGE_GAP_AT_96_DPI, dpi);
        let message = RECT {
            left: padding,
            top: message_top,
            right: width - padding,
            bottom: message_top + message_height,
        };
        let footer_top = message.bottom + scale(FOOTER_GAP_AT_96_DPI, dpi);
        let footer = RECT {
            left: 0,
            top: footer_top,
            right: width,
            bottom: footer_top + scale_text(FOOTER_HEIGHT_AT_96_DPI, dpi),
        };
        let button_height = scale_text(BUTTON_HEIGHT_AT_96_DPI, dpi);
        let button_top = footer.top + (footer.bottom - footer.top - button_height) / 2;
        let gap = scale(BUTTON_GAP_AT_96_DPI, dpi);
        let mut right = width - padding;
        let mut buttons: Vec<RECT> = button_widths(dpi, label_widths)
            .into_iter()
            .rev()
            .map(|button_width| {
                let rect = RECT {
                    left: right - button_width,
                    top: button_top,
                    right,
                    bottom: button_top + button_height,
                };
                right = rect.left - gap;
                rect
            })
            .collect();
        buttons.reverse();
        Self {
            width,
            height: footer.bottom,
            title_band,
            title,
            title_close,
            message,
            footer,
            buttons,
            dpi,
        }
    }

    pub(crate) fn target_at(&self, x: i32, y: i32) -> Option<Target> {
        let inside =
            |rect: &RECT| x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom;
        if inside(&self.title_close) {
            return Some(Target::TitleClose);
        }
        self.buttons.iter().position(inside).map(Target::Button)
    }
}

/// The tallest the message may be for the whole prompt to fit a work area `work_height` tall:
/// what is left once the title band, the gaps and the footer are taken.
pub(crate) fn message_cap(dpi: u32, title_height: i32, work_height: i32) -> i32 {
    let chrome = Layout::calculate(dpi, 0, title_height, 0, &[]).height;
    (work_height - chrome).max(0)
}

/// Where a `width` x `height` prompt goes: centered on the owner's `frame`, or on the monitor's
/// `work` area when the owner is minimized (its frame is parked off-screen), then moved so the
/// whole prompt is inside `work`. Without a work area it is just centered on the frame.
pub(crate) fn placement(
    frame: RECT,
    work: Option<RECT>,
    iconic: bool,
    width: i32,
    height: i32,
) -> (i32, i32) {
    let center_on = match work {
        Some(work) if iconic => work,
        _ => frame,
    };
    let left = center_on.left + (center_on.right - center_on.left - width) / 2;
    let top = center_on.top + (center_on.bottom - center_on.top - height) / 2;
    match work {
        Some(work) => (
            left.min(work.right - width).max(work.left),
            top.min(work.bottom - height).max(work.top),
        ),
        None => (left, top),
    }
}

/// The focus after Tab (`forward`) or Shift+Tab, wrapping.
pub(crate) fn next_focus(current: usize, count: usize, forward: bool) -> usize {
    if count == 0 {
        return 0;
    }
    if forward {
        (current + 1) % count
    } else {
        (current + count - 1) % count
    }
}

/// The button a key chooses, if any: Esc is the last (Cancel), Enter and Space the focused one,
/// and `quick` maps letters to buttons.
pub(crate) fn key_choice(
    key: u16,
    focus: usize,
    count: usize,
    quick: &[(u16, usize)],
) -> Option<usize> {
    match key {
        VK_ESCAPE => count.checked_sub(1),
        VK_RETURN | VK_SPACE => (count > 0).then(|| focus.min(count - 1)),
        _ => quick
            .iter()
            .find(|(letter, index)| *letter == key && *index < count)
            .map(|(_, index)| *index),
    }
}

// The window half: an owned popup with About's look and modal loop.

const TITLE: &str = "FastPad";

/// What to ask: the message, the button labels with the primary first and Cancel last, and the
/// letters (virtual-key codes) that pick a button.
pub(crate) struct Spec<'a> {
    pub message: &'a str,
    pub buttons: &'a [&'a str],
    pub quick_keys: &'a [(u16, usize)],
}

/// The open prompt's state, owned by its window through `GWLP_USERDATA`.
struct Prompt {
    colors: Palette,
    layout: Layout,
    message: String,
    labels: Vec<String>,
    quick: Vec<(u16, usize)>,
    title_font: HFONT,
    body_font: HFONT,
    glyph_font: HFONT,
    /// The focused button's index.
    focus: usize,
    hot: Option<Target>,
    pressed: Option<Target>,
    tracking_leave: bool,
    /// Direct2D for the rounded shapes, loaded as the prompt opens.
    canvas: Canvas,
    /// The message was cut to fit the work area, so its last line ends in an ellipsis.
    capped: bool,
    /// The owner was enabled when the prompt opened, so closing it enables the owner again.
    owner_was_enabled: bool,
}

impl Drop for Prompt {
    fn drop(&mut self) {
        unsafe {
            for font in [self.title_font, self.body_font, self.glyph_font] {
                DeleteObject(font as _);
            }
        }
    }
}

thread_local! {
    /// The button the open prompt was answered with, set just before it closes.
    static CHOICE: Cell<Option<usize>> = const { Cell::new(None) };
}

/// Shows the prompt over `owner` and returns the chosen button's index once it is closed: Cancel,
/// the last, for Esc, the ×, `WM_CLOSE`, or a window that could not be created.
pub(crate) fn show(owner: HWND, colors: Palette, spec: &Spec) -> usize {
    let last = spec.buttons.len().saturating_sub(1);
    CHOICE.with(|choice| choice.set(None));
    // An owner already disabled (by an outer modal) stays disabled: only what this prompt
    // disabled is enabled again.
    let owner_was_enabled = unsafe { IsWindowEnabled(owner) } != 0;
    let Some(dialog) = create(owner, colors, spec, owner_was_enabled) else {
        return last;
    };
    unsafe {
        if owner_was_enabled {
            EnableWindow(owner, 0);
        }
        ShowWindow(dialog, SW_SHOW);
        SetFocus(dialog);
    }
    #[cfg(test)]
    {
        let answer = ANSWERS
            .with(|answers| answers.borrow_mut().pop_front())
            .expect("a prompt opened in a test without answer_next");
        answer(dialog);
    }
    let mut message = MSG::default();
    while unsafe { IsWindow(dialog) } != 0 {
        match unsafe { GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) } {
            0 => {
                // WM_QUIT belongs to the outer loop: put it back for that loop to see.
                unsafe { PostQuitMessage(message.wParam as i32) };
                break;
            }
            -1 => break,
            _ => unsafe {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            },
        }
    }
    close(dialog);
    if owner_was_enabled {
        unsafe { EnableWindow(owner, 1) };
    }
    CHOICE.with(Cell::take).unwrap_or(last)
}

/// The work area of the monitor nearest `owner` (for a minimized owner, the one it restores to).
fn work_area(owner: HWND) -> Option<RECT> {
    let mut monitor = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    let found = unsafe {
        GetMonitorInfoW(
            MonitorFromWindow(owner, MONITOR_DEFAULTTONEAREST),
            &mut monitor,
        )
    } != 0;
    let work = monitor.rcWork;
    (found && work.right > work.left && work.bottom > work.top).then_some(work)
}

fn create(owner: HWND, colors: Palette, spec: &Spec, owner_was_enabled: bool) -> Option<HWND> {
    let class = register_class()?;
    // Not drawn (the title band draws `TITLE`), but read out as the prompt activates, so a
    // screen reader announces the question.
    let title = wide_null(&format!("{TITLE}: {}", spec.message));
    let dialog = unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW,
            class.as_ptr(),
            title.as_ptr(),
            // WS_CAPTION gives it a native frame, hidden by WM_NCCALCSIZE, so DWM draws the
            // window shadow; no system menu and no sizing border.
            WS_POPUP | WS_CAPTION | WS_CLIPCHILDREN,
            0,
            0,
            0,
            0,
            owner,
            std::ptr::null_mut(),
            GetModuleHandleW(std::ptr::null()),
            std::ptr::null(),
        )
    };
    if dialog.is_null() {
        return None;
    }
    let dpi = unsafe { GetDpiForWindow(owner) }.max(96);
    // The title matches Settings' and About's.
    let title_font = type_ramp::create(TextStyle::Title, dpi);
    let body_font = type_ramp::create(TextStyle::Body, dpi);
    let glyph_font = create_ui_font(scale(11, dpi), glyph_font_face(), FW_NORMAL as i32, false);
    let label_widths: Vec<i32> = spec
        .buttons
        .iter()
        .map(|label| measure(dialog, body_font, label))
        .collect();
    let width = dialog_width(dpi, &label_widths);
    let title_height = text_height(dialog, title_font);
    let work = work_area(owner);
    let measured = measure_wrapped(
        dialog,
        body_font,
        spec.message,
        content_width(dpi, &label_widths),
    );
    let cap = work.map_or(i32::MAX, |work| {
        message_cap(dpi, title_height, work.bottom - work.top)
    });
    let capped = measured > cap;
    let layout = Layout::calculate(dpi, width, title_height, measured.min(cap), &label_widths);
    let (width, height) = (layout.width, layout.height);
    let state = Box::new(Prompt {
        colors,
        layout,
        message: spec.message.to_owned(),
        labels: spec
            .buttons
            .iter()
            .map(|label| (*label).to_owned())
            .collect(),
        quick: spec.quick_keys.to_vec(),
        title_font,
        body_font,
        glyph_font,
        focus: 0,
        hot: None,
        pressed: None,
        tracking_leave: false,
        canvas: Canvas::load(),
        capped,
        owner_was_enabled,
    });
    unsafe { SetWindowLongPtrW(dialog, GWLP_USERDATA, Box::into_raw(state) as isize) };

    // Centered over the owner (or its monitor when minimized) inside the work area, with
    // rounded corners where Windows 11 draws them (square in high contrast) and the DWM shadow
    // of the hidden frame (a 1-px frame margin keeps DWM drawing it though the client covers
    // the whole window).
    let mut frame = RECT::default();
    unsafe { GetWindowRect(owner, &mut frame) };
    let iconic = unsafe { IsIconic(owner) } != 0;
    let (left, top) = placement(frame, work, iconic, width, height);
    unsafe {
        SetWindowPos(
            dialog,
            std::ptr::null_mut(),
            left,
            top,
            width,
            height,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
        let corners = if colors.high_contrast {
            DWMWCP_DONOTROUND
        } else {
            DWMWCP_ROUND
        };
        DwmSetWindowAttribute(
            dialog,
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
        DwmExtendFrameIntoClientArea(dialog, &margins);
    }
    Some(dialog)
}

/// Records `index` as the answer and closes the prompt.
fn finish(dialog: HWND, index: usize) {
    CHOICE.with(|choice| choice.set(Some(index)));
    close(dialog);
}

/// Re-enables the owner (if the prompt disabled it) before the prompt goes, so Windows hands
/// activation back to it rather than to some other application.
fn close(dialog: HWND) {
    if unsafe { IsWindow(dialog) } == 0 {
        return;
    }
    let reenable = state(dialog).is_some_and(|prompt| prompt.owner_was_enabled);
    unsafe {
        if reenable {
            EnableWindow(GetWindow(dialog, GW_OWNER), 1);
        }
        DestroyWindow(dialog);
    }
}

fn register_class() -> Option<&'static [u16]> {
    static CLASS_NAME: std::sync::OnceLock<Vec<u16>> = std::sync::OnceLock::new();
    static REGISTERED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let name = CLASS_NAME.get_or_init(|| wide_null("FastPadPrompt"));
    let registered = *REGISTERED.get_or_init(|| {
        // No CS_DROPSHADOW: the hidden native frame's DWM shadow replaces it.
        let class = WNDCLASSW {
            lpfnWndProc: Some(prompt_proc),
            hInstance: unsafe { GetModuleHandleW(std::ptr::null()) },
            hCursor: unsafe { LoadCursorW(std::ptr::null_mut(), IDC_ARROW) },
            lpszClassName: name.as_ptr(),
            ..Default::default()
        };
        unsafe { RegisterClassW(&class) != 0 || GetLastError() == ERROR_CLASS_ALREADY_EXISTS }
    });
    registered.then_some(name.as_slice())
}

/// The pixel extent of `text` in `font` as `DrawTextW` measures it with `format` in `rect`.
fn calc_rect(hwnd: HWND, font: HFONT, text: &str, mut rect: RECT, format: u32) -> RECT {
    unsafe {
        let dc = GetDC(hwnd);
        if dc.is_null() {
            return RECT::default();
        }
        let previous = SelectObject(dc, font as _);
        let mut text = wide_null(text);
        DrawTextW(
            dc,
            text.as_mut_ptr(),
            -1,
            &mut rect,
            DT_CALCRECT | DT_NOPREFIX | format,
        );
        SelectObject(dc, previous);
        ReleaseDC(hwnd, dc);
        rect
    }
}

/// The single-line pixel width of `text` in `font`.
fn measure(hwnd: HWND, font: HFONT, text: &str) -> i32 {
    let rect = calc_rect(hwnd, font, text, RECT::default(), DT_SINGLELINE);
    rect.right - rect.left
}

/// The height of `text` in `font` wrapped to `width`, breaking inside a word too long for a line
/// (a spaceless file name) as `paint_into` draws it.
fn measure_wrapped(hwnd: HWND, font: HFONT, text: &str, width: i32) -> i32 {
    let bounds = RECT {
        right: width,
        ..RECT::default()
    };
    let rect = calc_rect(hwnd, font, text, bounds, DT_WORDBREAK | DT_EDITCONTROL);
    rect.bottom - rect.top
}

fn state<'a>(hwnd: HWND) -> Option<&'a mut Prompt> {
    let pointer = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut Prompt;
    // SAFETY: set once in `create` from `Box::into_raw` and cleared in WM_NCDESTROY. This thread
    // only; callers end one borrow before anything that can re-enter the window procedure
    // (closing, capturing the mouse).
    unsafe { pointer.as_mut() }
}

fn invalidate(hwnd: HWND) {
    unsafe { InvalidateRect(hwnd, std::ptr::null(), 0) };
}

/// The number of buttons, read without holding a borrow.
fn button_count(hwnd: HWND) -> usize {
    state(hwnd).map_or(0, |prompt| prompt.labels.len())
}

/// Picks a button, or Cancel for the ×.
fn activate(hwnd: HWND, target: Target) {
    let index = match target {
        Target::Button(index) => index,
        Target::TitleClose => button_count(hwnd).saturating_sub(1),
    };
    finish(hwnd, index);
}

fn lparam_point(lparam: LPARAM) -> (i32, i32) {
    (
        (lparam & 0xffff) as i16 as i32,
        ((lparam >> 16) & 0xffff) as i16 as i32,
    )
}

unsafe extern "system" fn prompt_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // Checked without forming a reference: `paint` holds a `&Prompt` across `BeginPaint`, which
    // sends WM_ERASEBKGND back here, so neither the check nor that arm may borrow.
    if unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } == 0 {
        return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
    }
    match message {
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            if let Some(prompt) = state(hwnd) {
                paint(hwnd, prompt);
            }
            0
        }
        WM_CLOSE => {
            finish(hwnd, button_count(hwnd).saturating_sub(1));
            0
        }
        WM_KEYDOWN => {
            let key = wparam as u16;
            if key == VK_TAB {
                let back = unsafe { GetKeyState(i32::from(VK_SHIFT)) } < 0;
                if let Some(prompt) = state(hwnd) {
                    prompt.focus = next_focus(prompt.focus, prompt.labels.len(), !back);
                }
                invalidate(hwnd);
            } else if (lparam >> 30) & 1 != 0 && key != VK_ESCAPE {
                // An auto-repeat of a key held down when the prompt opened must not answer it:
                // the primary can be Delete or Replace.
            } else {
                let choice = state(hwnd).and_then(|prompt| {
                    key_choice(key, prompt.focus, prompt.labels.len(), &prompt.quick)
                });
                if let Some(index) = choice {
                    finish(hwnd, index);
                }
            }
            0
        }
        // The native frame stays hidden: the client is the whole window. Both forms leave the
        // proposed rect as it is.
        WM_NCCALCSIZE => 0,
        // Only the title band drags the prompt, as About's header does; not its ×.
        WM_NCHITTEST => {
            let (x, y) = lparam_point(lparam);
            let mut point = POINT { x, y };
            unsafe { ScreenToClient(hwnd, &mut point) };
            let drags = state(hwnd).is_some_and(|prompt| {
                let band = prompt.layout.title_band;
                point.y >= band.top
                    && point.y < band.bottom
                    && prompt.layout.target_at(point.x, point.y).is_none()
            });
            if drags {
                HTCAPTION as LRESULT
            } else {
                HTCLIENT as LRESULT
            }
        }
        WM_SETCURSOR => {
            unsafe { SetCursor(LoadCursorW(std::ptr::null_mut(), IDC_ARROW) as HCURSOR) };
            1
        }
        WM_MOUSEMOVE => {
            let (x, y) = lparam_point(lparam);
            if let Some(prompt) = state(hwnd) {
                let hot = prompt.layout.target_at(x, y);
                if !prompt.tracking_leave {
                    let mut track = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    prompt.tracking_leave = unsafe { TrackMouseEvent(&mut track) } != 0;
                }
                if hot != prompt.hot {
                    prompt.hot = hot;
                    invalidate(hwnd);
                }
            }
            0
        }
        WM_MOUSELEAVE => {
            if let Some(prompt) = state(hwnd) {
                prompt.tracking_leave = false;
                if prompt.hot.take().is_some() {
                    invalidate(hwnd);
                }
            }
            0
        }
        WM_LBUTTONDOWN => {
            let (x, y) = lparam_point(lparam);
            let pressed = state(hwnd).and_then(|prompt| {
                prompt.pressed = prompt.layout.target_at(x, y);
                // A button takes the focus; the × focuses Cancel, as About's focuses OK.
                prompt.focus = match prompt.pressed? {
                    Target::Button(index) => index,
                    Target::TitleClose => prompt.labels.len().saturating_sub(1),
                };
                prompt.pressed
            });
            if pressed.is_some() {
                unsafe { SetCapture(hwnd) };
                invalidate(hwnd);
            }
            0
        }
        WM_LBUTTONUP => {
            let (x, y) = lparam_point(lparam);
            let clicked = state(hwnd).and_then(|prompt| {
                let pressed = prompt.pressed.take()?;
                (prompt.layout.target_at(x, y) == Some(pressed)).then_some(pressed)
            });
            unsafe { ReleaseCapture() };
            invalidate(hwnd);
            if let Some(target) = clicked {
                activate(hwnd, target);
            }
            0
        }
        WM_NCDESTROY => {
            let pointer = unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) } as *mut Prompt;
            // SAFETY: the pointer came from `Box::into_raw` in `create` and is released only here.
            drop(unsafe { Box::from_raw(pointer) });
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

/// Paints through an off-screen bitmap so a hover repaint never shows the erase.
fn paint(hwnd: HWND, prompt: &Prompt) {
    paint_buffered(hwnd, |dc, client| paint_into(dc, client, prompt));
}

fn paint_into(dc: HDC, client: RECT, prompt: &Prompt) {
    let mut frame = Frame::default();
    compose(&mut frame, client, prompt);
    frame.paint(dc, client, &prompt.canvas);
    // The message wraps, which `Frame`'s single-line text can't, so it goes on last with GDI
    // once Direct2D has let go of the DC, as About's icon does.
    let mut text = wide_null(&prompt.message);
    let mut rect = prompt.layout.message;
    // The same breaking as `measure_wrapped`; a message cut to the work area ends in "...".
    let cut = if prompt.capped { DT_END_ELLIPSIS } else { 0 };
    unsafe {
        let previous = SelectObject(dc, prompt.body_font as _);
        SetBkMode(dc, TRANSPARENT as i32);
        SetTextColor(dc, prompt.colors.editor_foreground);
        DrawTextW(
            dc,
            text.as_mut_ptr(),
            -1,
            &mut rect,
            DT_LEFT | DT_WORDBREAK | DT_EDITCONTROL | DT_NOPREFIX | cut,
        );
        SelectObject(dc, previous);
    }
}

fn compose<'a>(frame: &mut Frame<'a>, client: RECT, prompt: &'a Prompt) {
    let colors = &prompt.colors;
    let tones = Tones::new(colors);
    let layout = &prompt.layout;
    let radius = scale(CONTROL_RADIUS, layout.dpi);

    // No border line: the native shadow is the edge (square on Windows 10, rounded on 11).
    frame.shape(Shape::Fill {
        rect: client,
        color: colors.panel_background(),
    });

    // The title band: a strip-coloured band like About's header, with the name and the ×.
    frame.shape(Shape::Fill {
        rect: layout.title_band,
        color: colors.strip_background,
    });
    frame.text(
        prompt.title_font,
        colors.editor_foreground,
        TITLE,
        layout.title,
        DT_LEFT,
    );
    title_close(
        frame,
        colors,
        prompt.glyph_font,
        layout.title_close,
        prompt.hot == Some(Target::TitleClose),
    );
    // Subtle rules under the band and over the footer.
    for top in [layout.title_band.bottom - 1, layout.footer.top] {
        frame.shape(Shape::Fill {
            rect: RECT {
                top,
                bottom: top + 1,
                ..client
            },
            color: tones.card,
        });
    }

    // The primary (first) button is a filled accent one, like About's OK; the others are
    // Settings' secondary controls.
    for (index, (label, rect)) in prompt.labels.iter().zip(&layout.buttons).enumerate() {
        let target = Some(Target::Button(index));
        let (pressed, hot) = (prompt.pressed == target, prompt.hot == target);
        let (fill, foreground) = if index == 0 {
            let fill = if pressed {
                tones.accent_down
            } else if hot {
                tones.accent_hot
            } else {
                tones.accent
            };
            (fill, tones.on_accent)
        } else {
            let fill = if pressed {
                tones.control_down
            } else if hot {
                tones.control_hot
            } else {
                tones.control
            };
            (fill, colors.editor_foreground)
        };
        tones.soft(frame, *rect, radius, fill);
        frame.text(
            prompt.body_font,
            foreground,
            label.as_str(),
            *rect,
            DT_CENTER,
        );
    }

    // The focus ring: a rounded accent stroke just outside the focused button.
    if let Some(focused) = layout.buttons.get(prompt.focus) {
        let width = scale(FOCUS_RING, layout.dpi);
        let outside = width + scale(FOCUS_GAP, layout.dpi);
        frame.shape(Shape::Ring {
            rect: inset(*focused, -outside),
            radius: radius + outside,
            width,
            color: tones.accent,
        });
    }
}

#[cfg(test)]
type Answer = Box<dyn FnOnce(HWND)>;

#[cfg(test)]
thread_local! {
    static ANSWERS: std::cell::RefCell<std::collections::VecDeque<Answer>> =
        const { std::cell::RefCell::new(std::collections::VecDeque::new()) };
}

/// Runs `answer` with the next prompt's window once it is shown, before its modal loop starts;
/// the answer posts the input that closes it.
#[cfg(test)]
pub(crate) fn answer_next(answer: impl FnOnce(HWND) + 'static) {
    ANSWERS.with(|answers| answers.borrow_mut().push_back(Box::new(answer)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_ESCAPE, VK_RETURN, VK_SPACE};

    fn inside(outer: RECT, inner: RECT) -> bool {
        inner.left >= outer.left
            && inner.top >= outer.top
            && inner.right <= outer.right
            && inner.bottom <= outer.bottom
    }

    #[test]
    fn everything_fits_at_every_dpi_and_text_size() {
        // Break caught: a long message or a long button label clipping or overlapping at 192 DPI
        // and 225 % text.
        let _factor = crate::window::design::text_scale::FactorGuard::new();
        for factor in [100, 225] {
            crate::window::design::text_scale::set_factor_for_test(factor);
            for dpi in [96, 120, 144, 192] {
                for labels in [vec![88, 60], vec![60, 120, 60], vec![420, 70]] {
                    let width = dialog_width(dpi, &labels);
                    let layout =
                        Layout::calculate(dpi, width, 24 * factor as i32 / 100, 300, &labels);
                    let client = RECT {
                        left: 0,
                        top: 0,
                        right: layout.width,
                        bottom: layout.height,
                    };
                    for rect in [
                        layout.title_band,
                        layout.title,
                        layout.message,
                        layout.footer,
                    ]
                    .into_iter()
                    .chain(layout.buttons.iter().copied())
                    {
                        assert!(inside(client, rect), "{factor} {dpi} {labels:?}");
                    }
                    assert!(layout.title_band.bottom <= layout.message.top);
                    assert!(layout.message.bottom <= layout.footer.top);
                    for button in &layout.buttons {
                        assert!(inside(layout.footer, *button));
                    }
                    for pair in layout.buttons.windows(2) {
                        assert!(pair[0].right < pair[1].left, "buttons never overlap");
                    }
                }
            }
        }
    }

    #[test]
    fn the_dialog_is_400px_and_grows_only_for_wide_buttons() {
        assert_eq!(dialog_width(96, &[60, 70]), 400);
        assert!(dialog_width(96, &[420, 70]) > 400);
        assert_eq!(dialog_width(192, &[60, 70]), 800);
    }

    #[test]
    fn the_message_height_grows_the_dialog() {
        let short = Layout::calculate(96, 400, 24, 20, &[60, 70]);
        let long = Layout::calculate(96, 400, 24, 120, &[60, 70]);
        assert_eq!(long.height - short.height, 100);
    }

    #[test]
    fn buttons_are_right_aligned_with_the_last_at_the_padding() {
        let layout = Layout::calculate(96, 400, 24, 40, &[60, 70, 60]);
        assert_eq!(layout.buttons.len(), 3);
        assert_eq!(layout.buttons[2].right, 400 - 20);
        assert!(layout.buttons[0].left < layout.buttons[1].left);
        assert_eq!(
            layout.target_at(layout.buttons[1].left + 1, layout.buttons[1].top + 1),
            Some(Target::Button(1))
        );
        assert_eq!(
            layout.target_at(layout.title_close.left + 1, layout.title_close.top + 1),
            Some(Target::TitleClose)
        );
        assert_eq!(layout.target_at(1, layout.message.top + 1), None);
    }

    #[test]
    fn keys_choose_cancel_on_escape_the_focus_on_enter_and_quick_letters() {
        // Break caught: Esc or a stray key choosing a destructive button; Enter ignoring the focus.
        let quick = [(u16::from(b'S'), 0), (u16::from(b'D'), 1)];
        assert_eq!(key_choice(VK_ESCAPE, 0, 3, &quick), Some(2));
        assert_eq!(key_choice(VK_RETURN, 0, 3, &quick), Some(0));
        assert_eq!(key_choice(VK_RETURN, 1, 3, &quick), Some(1));
        assert_eq!(key_choice(VK_SPACE, 2, 3, &quick), Some(2));
        assert_eq!(key_choice(u16::from(b'S'), 2, 3, &quick), Some(0));
        assert_eq!(key_choice(u16::from(b'D'), 0, 3, &quick), Some(1));
        assert_eq!(key_choice(u16::from(b'X'), 0, 3, &quick), None);
        assert_eq!(
            key_choice(u16::from(b'S'), 0, 2, &[]),
            None,
            "no quick keys, no choice"
        );
        assert_eq!(key_choice(VK_ESCAPE, 0, 2, &[]), Some(1));
    }

    #[test]
    fn tab_cycles_forward_and_back_and_wraps() {
        assert_eq!(next_focus(0, 3, true), 1);
        assert_eq!(next_focus(2, 3, true), 0);
        assert_eq!(next_focus(0, 3, false), 2);
        assert_eq!(next_focus(0, 1, true), 0);
    }

    use crate::platform::theme::Theme;
    use crate::window::soft_paint::TestSurface;
    use windows_sys::Win32::UI::WindowsAndMessaging::{PostMessageW, WS_OVERLAPPEDWINDOW};

    /// A plain top-level window to own the prompt, destroyed on drop.
    struct Owner(HWND);

    impl Owner {
        fn new() -> Self {
            let class = wide_null("STATIC");
            let hwnd = unsafe {
                CreateWindowExW(
                    0,
                    class.as_ptr(),
                    class.as_ptr(),
                    WS_OVERLAPPEDWINDOW,
                    100,
                    100,
                    800,
                    600,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null(),
                )
            };
            assert!(!hwnd.is_null(), "the test owner window");
            Self(hwnd)
        }
    }

    impl Drop for Owner {
        fn drop(&mut self) {
            unsafe { DestroyWindow(self.0) };
        }
    }

    const DELETE: Spec<'static> = Spec {
        message: "Delete it?",
        buttons: &["Delete", "Cancel"],
        quick_keys: &[],
    };

    fn key(dialog: HWND, key: u16) {
        unsafe { PostMessageW(dialog, WM_KEYDOWN, usize::from(key), 0) };
    }

    /// Shows `spec` over a fresh owner with `answer` posting its input, and returns the choice.
    fn answered(spec: &Spec, answer: impl FnOnce(HWND) + 'static) -> usize {
        let owner = Owner::new();
        answer_next(answer);
        show(owner.0, Palette::neutral(), spec)
    }

    #[test]
    fn a_held_enter_does_not_answer_but_the_next_press_does() {
        // Break caught: Enter held down while the prompt opens auto-repeating into it and
        // choosing a destructive primary before anyone has read it.
        assert_eq!(
            answered(&DELETE, |dialog| unsafe {
                // Bit 30 of lparam is the previous key state: set on an auto-repeat.
                PostMessageW(dialog, WM_KEYDOWN, usize::from(VK_RETURN), 1 << 30);
                PostMessageW(dialog, WM_KEYDOWN, usize::from(VK_ESCAPE), 1 << 30);
            }),
            1,
            "the repeat was ignored, so only the Esc repeat (always Cancel) answered"
        );
        assert_eq!(
            answered(&DELETE, |dialog| unsafe {
                PostMessageW(dialog, WM_KEYDOWN, usize::from(VK_RETURN), 1 << 30);
                PostMessageW(dialog, WM_KEYDOWN, usize::from(VK_RETURN), 0);
            }),
            0,
            "a fresh press still answers"
        );
    }

    #[test]
    fn show_returns_the_primary_on_enter() {
        // Break caught: Enter doing nothing, or choosing Cancel instead of the focused primary.
        assert_eq!(answered(&DELETE, |dialog| key(dialog, VK_RETURN)), 0);
    }

    #[test]
    fn escape_and_the_title_close_return_cancel() {
        // Break caught: Esc, the × or Alt+F4 (WM_CLOSE) choosing anything but Cancel, the last.
        assert_eq!(answered(&DELETE, |dialog| key(dialog, VK_ESCAPE)), 1);
        assert_eq!(
            answered(&DELETE, |dialog| {
                let close = state(dialog)
                    .map(|prompt| prompt.layout.title_close)
                    .unwrap();
                let x = (close.left + close.right) / 2;
                let y = (close.top + close.bottom) / 2;
                let point = ((y as isize) << 16) | (x as isize & 0xffff);
                unsafe {
                    PostMessageW(dialog, WM_LBUTTONDOWN, 0, point);
                    PostMessageW(dialog, WM_LBUTTONUP, 0, point);
                }
            }),
            1
        );
        assert_eq!(
            answered(&DELETE, |dialog| unsafe {
                PostMessageW(dialog, WM_CLOSE, 0, 0);
            }),
            1
        );
    }

    #[test]
    fn tab_then_enter_picks_the_second_button() {
        let spec = Spec {
            message: "Save changes to notes.txt?",
            buttons: &["Save", "Don't save", "Cancel"],
            quick_keys: &[],
        };
        assert_eq!(
            answered(&spec, |dialog| {
                key(dialog, VK_TAB);
                key(dialog, VK_RETURN);
            }),
            1
        );
    }

    #[test]
    fn a_quick_key_picks_its_button() {
        let spec = Spec {
            message: "Save changes to notes.txt?",
            buttons: &["Save", "Don't save", "Cancel"],
            quick_keys: &[(b'S' as u16, 0), (b'D' as u16, 1)],
        };
        assert_eq!(answered(&spec, |dialog| key(dialog, u16::from(b'S'))), 0);
        assert_eq!(answered(&spec, |dialog| key(dialog, u16::from(b'D'))), 1);
    }

    /// The layout `create` gives `message` over `owner`.
    fn created_layout(owner: HWND, message: &str) -> Layout {
        let spec = Spec { message, ..DELETE };
        let dialog = create(owner, Palette::neutral(), &spec, true).expect("the prompt window");
        let layout = state(dialog).map(|prompt| prompt.layout.clone()).unwrap();
        unsafe { DestroyWindow(dialog) };
        layout
    }

    #[test]
    fn a_long_message_grows_the_window() {
        // Break caught: a message measured on one line, clipping a long question.
        let owner = Owner::new();
        let long = ["word"; 40].join(" ");
        assert!(
            created_layout(owner.0, &long).height > created_layout(owner.0, "Delete it?").height
        );
    }

    #[test]
    fn a_long_unbroken_word_wraps_onto_more_lines() {
        // Break caught: a spaceless file name measured as one line and clipped at the right
        // edge, hiding which file the Delete applies to.
        let owner = Owner::new();
        let line = |layout: &Layout| layout.message.bottom - layout.message.top;
        let one = created_layout(owner.0, "Delete it?");
        let word = created_layout(owner.0, &"x".repeat(300));
        assert!(
            line(&word) > line(&one),
            "{} vs {}",
            line(&word),
            line(&one)
        );
    }

    #[test]
    fn a_capped_message_keeps_the_prompt_inside_the_work_area() {
        // Break caught: a long message growing the prompt past the bottom of the screen.
        let _factor = crate::window::design::text_scale::FactorGuard::new();
        for factor in [100, 225] {
            crate::window::design::text_scale::set_factor_for_test(factor);
            for dpi in [96, 144, 192] {
                let title_height = 24 * factor as i32 / 100;
                for work_height in [768, 1040, 2160] {
                    let cap = message_cap(dpi, title_height, work_height);
                    assert!(cap > 0, "{factor} {dpi} {work_height}");
                    let labels = [60, 70];
                    let layout = Layout::calculate(
                        dpi,
                        dialog_width(dpi, &labels),
                        title_height,
                        100_000.min(cap),
                        &labels,
                    );
                    assert!(layout.height <= work_height, "{factor} {dpi} {work_height}");
                    assert_eq!(layout.height, work_height, "the cap uses all the room");
                }
            }
        }
        assert_eq!(message_cap(96, 24, 10), 0, "never negative");
    }

    #[test]
    fn the_prompt_is_placed_inside_the_work_area() {
        // Break caught: a prompt over a minimized owner opening at -32000, off-screen; or one
        // over an owner partly off-screen hanging off the edge.
        let work = RECT {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1040,
        };
        let owner = RECT {
            left: 100,
            top: 100,
            right: 900,
            bottom: 700,
        };
        assert_eq!(placement(owner, Some(work), false, 400, 200), (300, 300));
        let parked = RECT {
            left: -32000,
            top: -32000,
            right: -31840,
            bottom: -31972,
        };
        assert_eq!(
            placement(parked, Some(work), true, 400, 200),
            (760, 420),
            "centered on the work area"
        );
        let hanging = RECT {
            left: 1700,
            top: 900,
            right: 2500,
            bottom: 1500,
        };
        assert_eq!(placement(hanging, Some(work), false, 400, 200), (1520, 840));
        let left_of = RECT {
            left: -700,
            top: -500,
            right: 100,
            bottom: 100,
        };
        assert_eq!(placement(left_of, Some(work), false, 400, 200), (0, 0));
        assert_eq!(placement(owner, None, false, 400, 200), (300, 300));
    }

    #[test]
    fn the_window_text_carries_the_question() {
        // Break caught: a screen reader announcing only "FastPad" as the prompt activates.
        use windows_sys::Win32::UI::WindowsAndMessaging::GetWindowTextW;
        let text = std::rc::Rc::new(std::cell::RefCell::new(String::new()));
        let seen = text.clone();
        answered(&DELETE, move |dialog| {
            let mut buffer = [0u16; 128];
            let length = unsafe { GetWindowTextW(dialog, buffer.as_mut_ptr(), 128) };
            *seen.borrow_mut() = String::from_utf16_lossy(&buffer[..length as usize]);
            key(dialog, VK_ESCAPE);
        });
        assert_eq!(*text.borrow(), "FastPad: Delete it?");
    }

    #[test]
    fn only_an_owner_the_prompt_disabled_is_enabled_again() {
        // Break caught: a prompt opened under an outer modal enabling the owner as it closes,
        // so the outer prompt's owner takes input again.
        let owner = Owner::new();
        answer_next(|dialog| key(dialog, VK_ESCAPE));
        show(owner.0, Palette::neutral(), &DELETE);
        assert_ne!(unsafe { IsWindowEnabled(owner.0) }, 0, "enabled again");

        unsafe { EnableWindow(owner.0, 0) };
        answer_next(|dialog| key(dialog, VK_RETURN));
        show(owner.0, Palette::neutral(), &DELETE);
        assert_eq!(
            unsafe { IsWindowEnabled(owner.0) },
            0,
            "still disabled after Enter"
        );
        answer_next(|dialog| key(dialog, VK_ESCAPE));
        show(owner.0, Palette::neutral(), &DELETE);
        assert_eq!(
            unsafe { IsWindowEnabled(owner.0) },
            0,
            "still disabled after Esc"
        );
    }

    fn painted_prompt(canvas: Canvas, colors: Palette) -> Prompt {
        let labels = [60, 60];
        let layout = Layout::calculate(96, dialog_width(96, &labels), 24, 34, &labels);
        Prompt {
            colors,
            layout,
            message: "Delete it?".to_owned(),
            labels: vec!["Delete".to_owned(), "Cancel".to_owned()],
            quick: Vec::new(),
            title_font: std::ptr::null_mut(),
            body_font: std::ptr::null_mut(),
            glyph_font: std::ptr::null_mut(),
            focus: 0,
            hot: None,
            pressed: None,
            tracking_leave: false,
            canvas,
            capped: false,
            owner_was_enabled: true,
        }
    }

    fn paint_prompt(prompt: &Prompt) -> TestSurface {
        let layout = &prompt.layout;
        let surface = TestSurface::new(layout.width, layout.height);
        let client = RECT {
            left: 0,
            top: 0,
            right: layout.width,
            bottom: layout.height,
        };
        paint_into(surface.dc, client, prompt);
        surface
    }

    fn middle(rect: RECT) -> (i32, i32) {
        ((rect.left + rect.right) / 2, (rect.top + rect.bottom) / 2)
    }

    #[test]
    fn the_primary_button_is_accent_filled_and_the_second_is_not() {
        // Break caught: every button painted alike, so the destructive or default action has no
        // emphasis; or the soft rounded look lost under Direct2D.
        let colors = Palette::for_theme(Theme::ALL[1], false);
        let tones = Tones::new(&colors);
        for direct2d in [false, true] {
            let canvas = if direct2d {
                Canvas::load()
            } else {
                Canvas::gdi()
            };
            assert_eq!(canvas.uses_direct2d(), direct2d);
            let prompt = painted_prompt(canvas, colors);
            let surface = paint_prompt(&prompt);
            let [primary, second] = [prompt.layout.buttons[0], prompt.layout.buttons[1]];
            let (_, y) = middle(primary);
            // Left of the centred label, clear of its text.
            assert_eq!(
                surface.pixel(primary.left + 4, y),
                tones.accent,
                "{direct2d}"
            );
            let (_, y) = middle(second);
            assert_ne!(
                surface.pixel(second.left + 4, y),
                tones.accent,
                "{direct2d}"
            );
            assert_eq!(
                surface.pixel(second.left + 4, y),
                tones.control,
                "{direct2d}"
            );
            let corner = surface.pixel(primary.left, primary.top);
            if direct2d {
                assert_ne!(corner, tones.accent, "rounded corner");
            } else {
                assert_eq!(corner, tones.accent, "square corner under GDI");
            }
        }
    }

    #[test]
    fn high_contrast_fills_the_primary_with_the_highlight_pair() {
        // Break caught: a themed accent (not a system color) on the primary in high contrast.
        let colors = Palette::for_theme(Theme::ALL[1], true);
        let prompt = painted_prompt(Canvas::gdi(), colors);
        let surface = paint_prompt(&prompt);
        let [primary, second] = [prompt.layout.buttons[0], prompt.layout.buttons[1]];
        let (_, y) = middle(primary);
        assert_eq!(surface.pixel(primary.left + 4, y), colors.accent);
        // Square, with high contrast's system-color outline on its edge.
        assert_eq!(
            surface.pixel(primary.left, primary.top),
            colors.muted_foreground
        );
        let (_, y) = middle(second);
        assert_eq!(surface.pixel(second.left + 4, y), colors.strip_background);
    }
}
