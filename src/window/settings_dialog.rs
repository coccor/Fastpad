//! Settings: a themed, owner-drawn modal popup listing every user-facing `fastpad.ini` setting
//! (settings dialog spec §3). Each change applies and saves at once through
//! `main_window::apply_settings_action`. Like About, it runs its own modal loop with the main
//! window disabled. All behaviour lives in `settings_model`; this module decodes input and
//! paints.

use super::dropdown_list::{DropdownList, ListKey, ListModel, ListOutcome, WM_LIST_PICKED};
use super::modal::ModalScope;
use super::palette::Palette;
use super::panel::{fill, inset, scale};
use super::settings_model::{
    Control, DialogModel, Effect, Focus, Key, Row, Section, SettingsView, dropdown_action,
    step_font_size,
};
use super::side_panel::paint_buffered;
use super::titlebar::create_ui_font;
use crate::platform::wide_null;
use std::cell::Cell;
use std::rc::Rc;
use windows_sys::Win32::Foundation::{
    ERROR_CLASS_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows_sys::Win32::Graphics::Dwm::{
    DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute,
};
use windows_sys::Win32::Graphics::Gdi::{
    ClientToScreen, DT_CALCRECT, DT_CENTER, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_RIGHT,
    DT_SINGLELINE, DT_VCENTER, DeleteObject, DrawFocusRect, DrawTextW, FW_NORMAL, FW_SEMIBOLD,
    GetDC, GetMonitorInfoW, HDC, HFONT, IntersectClipRect, InvalidateRect,
    MONITOR_DEFAULTTONEAREST, MONITORINFO, MapWindowPoints, MonitorFromWindow, ReleaseDC,
    RestoreDC, SaveDC, ScreenToClient, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::SystemInformation::GetTickCount;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    EnableWindow, GetDoubleClickTime, GetFocus, GetKeyState, ReleaseCapture, SetCapture, SetFocus,
    TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent, VK_DOWN, VK_END, VK_ESCAPE, VK_HOME, VK_LEFT,
    VK_NEXT, VK_PRIOR, VK_RETURN, VK_RIGHT, VK_SHIFT, VK_SPACE, VK_TAB, VK_UP,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CS_DROPSHADOW, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, FindWindowExW,
    GW_OWNER, GWLP_USERDATA, GetCursorPos, GetMessageTime, GetMessageW, GetSystemMetrics,
    GetWindow, GetWindowLongPtrW, GetWindowRect, HCURSOR, HTCAPTION, HTCLIENT, IDC_ARROW, IDC_HAND,
    IsWindow, LoadCursorW, MSG, PostMessageW, PostQuitMessage, RegisterClassW, SM_CXDOUBLECLK,
    SM_CYDOUBLECLK, SW_SHOW, SWP_NOACTIVATE, SWP_NOZORDER, SetCursor, SetWindowLongPtrW,
    SetWindowPos, ShowWindow, TranslateMessage, WA_INACTIVE, WM_ACTIVATE, WM_CHAR, WM_CLOSE,
    WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL,
    WM_NCDESTROY, WM_NCHITTEST, WM_NCLBUTTONDOWN, WM_PAINT, WM_SETCURSOR, WM_SYSKEYDOWN, WNDCLASSW,
    WS_CLIPCHILDREN, WS_EX_TOOLWINDOW, WS_POPUP,
};

const TITLE: &str = "Settings";
const EDIT_INI_LABEL: &str = "Edit fastpad.ini";
const CLOSE_LABEL: &str = "Close";
const AUTOSAVE_HINT: &str = "Open a notebook to change this";
const GLYPH_FONT: &str = "Segoe MDL2 Assets";
const GLYPH_CHECK: &str = "\u{E73E}";
const GLYPH_CHEVRON_DOWN: &str = "\u{E70D}";
const GLYPH_ADD: &str = "\u{E710}";
const GLYPH_REMOVE: &str = "\u{E738}";
const GLYPH_CLOSE: &str = "\u{E8BB}";

const WIDTH_AT_96_DPI: i32 = 520;
const PADDING_AT_96_DPI: i32 = 20;
const TITLE_HEIGHT_AT_96_DPI: i32 = 44;
const HEADING_HEIGHT_AT_96_DPI: i32 = 30;
const ROW_HEIGHT_AT_96_DPI: i32 = 32;
const CONTROL_HEIGHT_AT_96_DPI: i32 = 26;
const DROPDOWN_WIDTH_AT_96_DPI: i32 = 240;
const SEGMENT_WIDTH_AT_96_DPI: i32 = 80;
const TAB_SEGMENT_WIDTH_AT_96_DPI: i32 = 44;
const STEP_BUTTON_AT_96_DPI: i32 = 28;
const STEP_VALUE_AT_96_DPI: i32 = 48;
const CHECK_AT_96_DPI: i32 = 18;
const CONTENT_BOTTOM_GAP_AT_96_DPI: i32 = 8;
const FOOTER_HEIGHT_AT_96_DPI: i32 = 56;
const BUTTON_WIDTH_AT_96_DPI: i32 = 88;
const BUTTON_HEIGHT_AT_96_DPI: i32 = 30;
/// However short the screen, at least this many rows stay visible.
const MIN_VISIBLE_ROWS: i32 = 3;

/// How the dialog was closed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Outcome {
    Closed,
    /// The Edit fastpad.ini link: the caller opens the file now that the dialog is gone.
    EditIni,
}

/// Which part of a row the pointer is on.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Part {
    Whole,
    Segment(usize),
    Minus,
    Value,
    Plus,
}

/// What the pointer is on.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Hit {
    Row(Row, Part),
    EditIni,
    Close,
    TitleClose,
}

/// Where everything sits. Headings and rows are in content coordinates, placed in the
/// scrolling `body` by `row_rect`/`heading_rect`.
#[derive(Clone, Copy)]
pub(crate) struct Layout {
    pub width: i32,
    pub height: i32,
    pub title: RECT,
    pub title_close: RECT,
    pub body: RECT,
    pub headings: [RECT; 3],
    pub rows: [RECT; 13],
    pub content_height: i32,
    pub edit_ini: RECT,
    pub close: RECT,
    dpi: u32,
}

impl Layout {
    /// The layout at `dpi`, at most `max_height` tall (the work area), with the Edit fastpad.ini
    /// link `link_width` wide.
    pub(crate) fn calculate(dpi: u32, max_height: i32, link_width: i32) -> Self {
        let width = scale(WIDTH_AT_96_DPI, dpi);
        let padding = scale(PADDING_AT_96_DPI, dpi);
        let title_height = scale(TITLE_HEIGHT_AT_96_DPI, dpi);
        let heading_height = scale(HEADING_HEIGHT_AT_96_DPI, dpi);
        let row_height = scale(ROW_HEIGHT_AT_96_DPI, dpi);
        let footer_height = scale(FOOTER_HEIGHT_AT_96_DPI, dpi);

        let title = RECT {
            left: padding,
            top: 0,
            right: width - title_height,
            bottom: title_height,
        };
        let title_close = RECT {
            left: width - title_height,
            top: 0,
            right: width,
            bottom: title_height,
        };

        let line = |top: i32, height: i32| RECT {
            left: padding,
            top,
            right: width - padding,
            bottom: top + height,
        };
        let mut headings = [RECT::default(); 3];
        let mut rows = [RECT::default(); 13];
        let mut top = 0;
        for section in Section::ALL {
            headings[section as usize] = line(top, heading_height);
            top += heading_height;
            for row in Row::ALL.into_iter().filter(|row| row.section() == section) {
                rows[row as usize] = line(top, row_height);
                top += row_height;
            }
        }
        let content_height = top + scale(CONTENT_BOTTOM_GAP_AT_96_DPI, dpi);

        let smallest = title_height + row_height * MIN_VISIBLE_ROWS + footer_height;
        let height = (title_height + content_height + footer_height).min(max_height.max(smallest));
        let body = RECT {
            left: 0,
            top: title_height,
            right: width,
            bottom: height - footer_height,
        };
        let button_height = scale(BUTTON_HEIGHT_AT_96_DPI, dpi);
        let button_top = body.bottom + (footer_height - button_height) / 2;
        let close = RECT {
            left: width - padding - scale(BUTTON_WIDTH_AT_96_DPI, dpi),
            top: button_top,
            right: width - padding,
            bottom: button_top + button_height,
        };
        let edit_ini = RECT {
            left: padding,
            top: button_top,
            right: (padding + link_width).min(close.left),
            bottom: button_top + button_height,
        };
        Self {
            width,
            height,
            title,
            title_close,
            body,
            headings,
            rows,
            content_height,
            edit_ini,
            close,
            dpi,
        }
    }

    pub(crate) fn max_scroll(&self) -> i32 {
        (self.content_height - (self.body.bottom - self.body.top)).max(0)
    }

    pub(crate) fn list_row_height(&self) -> i32 {
        scale(CONTROL_HEIGHT_AT_96_DPI, self.dpi)
    }

    fn place(&self, rect: RECT, scroll: i32) -> RECT {
        let offset = self.body.top - scroll;
        RECT {
            top: rect.top + offset,
            bottom: rect.bottom + offset,
            ..rect
        }
    }

    /// Row `row`'s full rect in client coordinates at `scroll`.
    pub(crate) fn row_rect(&self, row: Row, scroll: i32) -> RECT {
        self.place(self.rows[row as usize], scroll)
    }

    pub(crate) fn heading_rect(&self, section: Section, scroll: i32) -> RECT {
        self.place(self.headings[section as usize], scroll)
    }

    /// The control of `row`, right-aligned in `row_rect`. `segments` is how many a segmented
    /// row shows.
    pub(crate) fn control_rect(&self, row: Row, row_rect: RECT, segments: usize) -> RECT {
        let dpi = self.dpi;
        let (width, height) = match row.control() {
            Control::Dropdown => (
                scale(DROPDOWN_WIDTH_AT_96_DPI, dpi),
                scale(CONTROL_HEIGHT_AT_96_DPI, dpi),
            ),
            Control::Segmented => {
                let each = if row == Row::TabWidth {
                    TAB_SEGMENT_WIDTH_AT_96_DPI
                } else {
                    SEGMENT_WIDTH_AT_96_DPI
                };
                (
                    segments as i32 * scale(each, dpi),
                    scale(CONTROL_HEIGHT_AT_96_DPI, dpi),
                )
            }
            Control::Stepper => (
                scale(STEP_BUTTON_AT_96_DPI, dpi) * 2 + scale(STEP_VALUE_AT_96_DPI, dpi),
                scale(CONTROL_HEIGHT_AT_96_DPI, dpi),
            ),
            Control::Check => (scale(CHECK_AT_96_DPI, dpi), scale(CHECK_AT_96_DPI, dpi)),
        };
        let top = row_rect.top + (row_rect.bottom - row_rect.top - height) / 2;
        RECT {
            left: row_rect.right - width,
            top,
            right: row_rect.right,
            bottom: top + height,
        }
    }

    /// A segmented control's segments, left to right, sharing its width.
    pub(crate) fn segment_rects(&self, control: RECT, segments: usize) -> Vec<RECT> {
        let count = segments.max(1) as i32;
        let each = (control.right - control.left) / count;
        (0..count)
            .map(|index| RECT {
                left: control.left + index * each,
                right: if index == count - 1 {
                    control.right
                } else {
                    control.left + (index + 1) * each
                },
                ..control
            })
            .collect()
    }

    /// The stepper's –, value and + parts.
    pub(crate) fn stepper_rects(&self, control: RECT) -> [RECT; 3] {
        let button = scale(STEP_BUTTON_AT_96_DPI, self.dpi);
        [
            RECT {
                right: control.left + button,
                ..control
            },
            RECT {
                left: control.left + button,
                right: control.right - button,
                ..control
            },
            RECT {
                left: control.right - button,
                ..control
            },
        ]
    }

    /// What client point `x`, `y` is on at `scroll`. A checkbox row is hit anywhere, label
    /// included. Other rows are hit only on their control.
    pub(crate) fn hit(&self, x: i32, y: i32, scroll: i32, view: &SettingsView) -> Option<Hit> {
        let inside =
            |rect: &RECT| x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom;
        if inside(&self.title_close) {
            return Some(Hit::TitleClose);
        }
        if inside(&self.close) {
            return Some(Hit::Close);
        }
        if inside(&self.edit_ini) {
            return Some(Hit::EditIni);
        }
        if !inside(&self.body) {
            return None;
        }
        let row = Row::ALL
            .into_iter()
            .find(|row| inside(&self.row_rect(*row, scroll)))?;
        let segments = view.segments(row).len();
        let control = self.control_rect(row, self.row_rect(row, scroll), segments);
        let part = match row.control() {
            Control::Check => Some(Part::Whole),
            Control::Dropdown => inside(&control).then_some(Part::Whole),
            Control::Segmented => self
                .segment_rects(control, segments)
                .iter()
                .position(&inside)
                .map(Part::Segment),
            Control::Stepper => {
                let [minus, value, plus] = self.stepper_rects(control);
                [
                    (minus, Part::Minus),
                    (value, Part::Value),
                    (plus, Part::Plus),
                ]
                .into_iter()
                .find(|(rect, _)| inside(rect))
                .map(|(_, part)| part)
            }
        };
        part.map(|part| Hit::Row(row, part))
    }

    /// The scroll from `scroll` that shows `focus`'s row whole. A section's first row brings its
    /// heading into view too.
    pub(crate) fn scroll_to_show(&self, focus: Focus, scroll: i32) -> i32 {
        let Focus::Row(row) = focus else {
            return scroll;
        };
        let rect = self.rows[row as usize];
        let first_in_section = Row::ALL
            .into_iter()
            .find(|candidate| candidate.section() == row.section())
            == Some(row);
        let top = if first_in_section {
            self.headings[row.section() as usize].top
        } else {
            rect.top
        };
        let visible = self.body.bottom - self.body.top;
        let scroll = if top < scroll {
            top
        } else if rect.bottom > scroll + visible {
            rect.bottom - visible
        } else {
            scroll
        };
        scroll.clamp(0, self.max_scroll())
    }
}

/// The open dialog's state, owned by its window through `GWLP_USERDATA`.
struct Dialog {
    colors: Palette,
    link_color: u32,
    layout: Layout,
    view: SettingsView,
    model: DialogModel,
    fonts: Vec<String>,
    scroll: i32,
    title_font: HFONT,
    heading_font: HFONT,
    body_font: HFONT,
    link_font: HFONT,
    glyph_font: HFONT,
    hot: Option<Hit>,
    pressed: Option<Hit>,
    tracking_leave: bool,
    /// The open dropdown and its row. Dropped before the fonts it borrows.
    list: Option<(Row, DropdownList)>,
    /// When and where (screen) the last mouse pick in a dropdown was: the second click of a
    /// double-click on an item lands here once the list is gone and must not change a setting.
    picked_at: Option<(u32, POINT)>,
    /// A press swallowed as the second click of such a double-click: its release is swallowed
    /// too.
    swallowing: bool,
    outcome: Rc<Cell<Outcome>>,
}

impl Drop for Dialog {
    fn drop(&mut self) {
        self.list = None;
        unsafe {
            for font in [
                self.title_font,
                self.heading_font,
                self.body_font,
                self.link_font,
                self.glyph_font,
            ] {
                DeleteObject(font as _);
            }
        }
    }
}

/// Shows Settings over `owner` and returns once it is closed.
pub(crate) fn show(owner: HWND, colors: Palette, link_color: u32) -> Outcome {
    let _modal = ModalScope::enter(owner);
    let outcome = Rc::new(Cell::new(Outcome::Closed));
    let Some(dialog) = create(owner, colors, link_color, outcome.clone()) else {
        return Outcome::Closed;
    };
    unsafe {
        EnableWindow(owner, 0);
        ShowWindow(dialog, SW_SHOW);
        SetFocus(dialog);
    }
    #[cfg(test)]
    {
        let answer = ANSWERS
            .with(|answers| answers.borrow_mut().pop_front())
            .expect("a Settings dialog opened in a test without answer_next");
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
    unsafe { EnableWindow(owner, 1) };
    outcome.get()
}

fn create(
    owner: HWND,
    colors: Palette,
    link_color: u32,
    outcome: Rc<Cell<Outcome>>,
) -> Option<HWND> {
    let class = register_class()?;
    let title = wide_null(TITLE);
    let dialog = unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW,
            class.as_ptr(),
            title.as_ptr(),
            WS_POPUP | WS_CLIPCHILDREN,
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
    let title_font = create_ui_font(scale(18, dpi), "Segoe UI", FW_SEMIBOLD as i32, false);
    let heading_font = create_ui_font(scale(12, dpi), "Segoe UI", FW_SEMIBOLD as i32, false);
    let body_font = create_ui_font(scale(13, dpi), "Segoe UI", FW_NORMAL as i32, false);
    let link_font = create_underlined_font(scale(13, dpi));
    let glyph_font = create_ui_font(scale(11, dpi), GLYPH_FONT, FW_NORMAL as i32, false);

    let mut monitor = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    let mut frame = RECT::default();
    unsafe {
        GetMonitorInfoW(
            MonitorFromWindow(owner, MONITOR_DEFAULTTONEAREST),
            &mut monitor,
        );
        GetWindowRect(owner, &mut frame);
    }
    let work = monitor.rcWork;
    let max_height = if work.bottom > work.top {
        work.bottom - work.top
    } else {
        i32::MAX
    };
    let layout = Layout::calculate(dpi, max_height, measure(dialog, link_font, EDIT_INI_LABEL));
    let view = super::main_window::settings_view(owner);
    let fonts = crate::platform::fonts::dropdown_names(
        crate::platform::fonts::installed_font_families(),
        &view.settings.font_face,
    );
    let state = Box::new(Dialog {
        colors,
        link_color,
        layout,
        view,
        model: DialogModel::new(),
        fonts,
        scroll: 0,
        title_font,
        heading_font,
        body_font,
        link_font,
        glyph_font,
        hot: None,
        pressed: None,
        tracking_leave: false,
        list: None,
        picked_at: None,
        swallowing: false,
        outcome,
    });
    unsafe { SetWindowLongPtrW(dialog, GWLP_USERDATA, Box::into_raw(state) as isize) };

    // Centered over the owner, kept inside the work area, with rounded corners where Windows
    // 11 draws them.
    let mut left = frame.left + (frame.right - frame.left - layout.width) / 2;
    let mut top = frame.top + (frame.bottom - frame.top - layout.height) / 2;
    if work.bottom > work.top {
        left = left.clamp(work.left, (work.right - layout.width).max(work.left));
        top = top.clamp(work.top, (work.bottom - layout.height).max(work.top));
    }
    unsafe {
        SetWindowPos(
            dialog,
            std::ptr::null_mut(),
            left,
            top,
            layout.width,
            layout.height,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
        let corners = DWMWCP_ROUND;
        DwmSetWindowAttribute(
            dialog,
            DWMWA_WINDOW_CORNER_PREFERENCE as u32,
            (&raw const corners).cast(),
            std::mem::size_of_val(&corners) as u32,
        );
    }
    Some(dialog)
}

/// Re-enables the owner before the dialog goes, so Windows hands activation back to it rather
/// than to some other application.
fn close(dialog: HWND) {
    if unsafe { IsWindow(dialog) } == 0 {
        return;
    }
    unsafe {
        EnableWindow(owner(dialog), 1);
        DestroyWindow(dialog);
    }
}

fn register_class() -> Option<&'static [u16]> {
    static CLASS_NAME: std::sync::OnceLock<Vec<u16>> = std::sync::OnceLock::new();
    static REGISTERED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let name = CLASS_NAME.get_or_init(|| wide_null("FastPadSettings"));
    let registered = *REGISTERED.get_or_init(|| {
        let class = WNDCLASSW {
            style: CS_DROPSHADOW,
            lpfnWndProc: Some(dialog_proc),
            hInstance: unsafe { GetModuleHandleW(std::ptr::null()) },
            hCursor: unsafe { LoadCursorW(std::ptr::null_mut(), IDC_ARROW) },
            lpszClassName: name.as_ptr(),
            ..Default::default()
        };
        unsafe { RegisterClassW(&class) != 0 || GetLastError() == ERROR_CLASS_ALREADY_EXISTS }
    });
    registered.then_some(name.as_slice())
}

fn create_underlined_font(pixel_height: i32) -> HFONT {
    use windows_sys::Win32::Graphics::Gdi::{
        CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateFontW, DEFAULT_CHARSET, DEFAULT_PITCH,
        OUT_DEFAULT_PRECIS,
    };
    let face = wide_null("Segoe UI");
    unsafe {
        CreateFontW(
            -pixel_height,
            0,
            0,
            0,
            FW_NORMAL as i32,
            0,
            1,
            0,
            u32::from(DEFAULT_CHARSET),
            u32::from(OUT_DEFAULT_PRECIS),
            u32::from(CLIP_DEFAULT_PRECIS),
            u32::from(CLEARTYPE_QUALITY),
            u32::from(DEFAULT_PITCH),
            face.as_ptr(),
        )
    }
}

fn measure(hwnd: HWND, font: HFONT, text: &str) -> i32 {
    unsafe {
        let dc = GetDC(hwnd);
        if dc.is_null() {
            return 0;
        }
        let previous = SelectObject(dc, font as _);
        let mut text = wide_null(text);
        let mut rect = RECT::default();
        DrawTextW(
            dc,
            text.as_mut_ptr(),
            -1,
            &mut rect,
            DT_CALCRECT | DT_SINGLELINE | DT_NOPREFIX,
        );
        SelectObject(dc, previous);
        ReleaseDC(hwnd, dc);
        rect.right - rect.left
    }
}

/// The main window; read from the window rather than `Dialog`, which the caller may be
/// borrowing.
fn owner(dialog: HWND) -> HWND {
    unsafe { GetWindow(dialog, GW_OWNER) }
}

fn state<'a>(hwnd: HWND) -> Option<&'a mut Dialog> {
    let pointer = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut Dialog;
    // SAFETY: set once in `create` from `Box::into_raw` and cleared in WM_NCDESTROY. This thread
    // only; callers end one borrow before anything that can re-enter the window procedure
    // (see `run`).
    unsafe { pointer.as_mut() }
}

fn invalidate(hwnd: HWND) {
    unsafe { InvalidateRect(hwnd, std::ptr::null(), 0) };
}

fn lparam_point(lparam: LPARAM) -> (i32, i32) {
    (
        (lparam & 0xffff) as i16 as i32,
        ((lparam >> 16) & 0xffff) as i16 as i32,
    )
}

/// Carries out `effect`. No `Dialog` borrow may be alive here: applying a setting runs main
/// window code that can move the focus and re-enter this window's procedure.
fn run(hwnd: HWND, effect: Effect) {
    match effect {
        Effect::None => {}
        Effect::Repaint => {
            if let Some(dialog) = state(hwnd) {
                dialog.scroll = dialog
                    .layout
                    .scroll_to_show(dialog.model.focus, dialog.scroll);
            }
            invalidate(hwnd);
        }
        Effect::Apply(action) => {
            super::main_window::apply_settings_action(owner(hwnd), action);
            // Some changes move the focus (notes mode rebuilds the sidebar); the keyboard stays
            // here.
            if unsafe { GetFocus() } != hwnd {
                unsafe { SetFocus(hwnd) };
            }
            #[cfg(test)]
            FOCUS_AFTER_APPLY
                .with(|checks| checks.borrow_mut().push(unsafe { GetFocus() } == hwnd));
            refresh(hwnd);
        }
        Effect::OpenDropdown(row) => open_list(hwnd, row),
        Effect::EditIni => {
            if let Some(dialog) = state(hwnd) {
                dialog.outcome.set(Outcome::EditIni);
            }
            close(hwnd);
        }
        Effect::Close => close(hwnd),
    }
}

/// Posted to an open dialog when something it shows changed outside it (the notebook finished
/// loading, so its autosave switch is known): it calls `refresh`.
const WM_SETTINGS_REFRESH: u32 = windows_sys::Win32::UI::WindowsAndMessaging::WM_APP + 2;

/// The Settings dialog `owner` has open, if any.
pub(crate) fn open_dialog(owner: HWND) -> Option<HWND> {
    let class = register_class()?;
    let mut after: HWND = std::ptr::null_mut();
    loop {
        after = unsafe {
            FindWindowExW(
                std::ptr::null_mut(),
                after,
                class.as_ptr(),
                std::ptr::null(),
            )
        };
        if after.is_null() {
            return None;
        }
        if unsafe { GetWindow(after, GW_OWNER) } == owner {
            return Some(after);
        }
    }
}

/// Has `owner`'s open dialog, if any, re-read what it shows. Posted, so the dialog does it from
/// its own loop with nothing borrowed.
pub(crate) fn refresh_open(owner: HWND) {
    if let Some(dialog) = open_dialog(owner) {
        unsafe { PostMessageW(dialog, WM_SETTINGS_REFRESH, 0, 0) };
    }
}

/// Whether a press at `at` (screen) at `now` is the second click of a double-click whose first
/// click picked a dropdown item at `picked`: within the double-click time and rectangle
/// (`limits`: milliseconds, width, height, as Windows reports them).
pub(crate) fn completes_double_click(
    picked: (u32, POINT),
    now: u32,
    at: POINT,
    limits: (u32, i32, i32),
) -> bool {
    let (time, point) = picked;
    let (max_ms, width, height) = limits;
    now.wrapping_sub(time) <= max_ms
        && (at.x - point.x).abs() <= width / 2
        && (at.y - point.y).abs() <= height / 2
}

/// Re-reads what the dialog shows after a change: the settings, the notebook's switch, and the
/// theme's colours, which a theme change replaced.
fn refresh(hwnd: HWND) {
    let owner = owner(hwnd);
    let view = super::main_window::settings_view(owner);
    let colors = super::main_window::current_palette(owner);
    let link_color = super::main_window::link_color(owner);
    if let Some(dialog) = state(hwnd) {
        dialog.view = view;
        dialog.colors = colors;
        dialog.link_color = link_color;
        dialog.scroll = dialog
            .layout
            .scroll_to_show(dialog.model.focus, dialog.scroll);
    }
    invalidate(hwnd);
}

fn open_list(hwnd: HWND, row: Row) {
    let Some(dialog) = state(hwnd) else {
        return;
    };
    dialog.list = None;
    let (items, selected) = dialog.view.dropdown(row, &dialog.fonts);
    let control = dialog
        .layout
        .control_rect(row, dialog.layout.row_rect(row, dialog.scroll), 0);
    let mut anchor = control;
    unsafe {
        MapWindowPoints(
            hwnd,
            std::ptr::null_mut(),
            (&raw mut anchor).cast::<POINT>(),
            2,
        );
    }
    dialog.list = DropdownList::show(
        hwnd,
        anchor,
        dialog.layout.list_row_height(),
        dialog.body_font,
        dialog.colors,
        ListModel::new(items, selected),
    )
    .map(|list| (row, list));
    invalidate(hwnd);
}

/// Closes the open dropdown, if any; true when one was open.
fn close_list(hwnd: HWND) -> bool {
    let closed = state(hwnd).and_then(|dialog| dialog.list.take()).is_some();
    if closed {
        invalidate(hwnd);
    }
    closed
}

/// Picks item `index` of the open dropdown.
fn pick(hwnd: HWND, index: usize) {
    let action = state(hwnd).and_then(|dialog| {
        let (row, _) = dialog.list.take()?;
        dropdown_action(row, index, &dialog.fonts)
    });
    invalidate(hwnd);
    if let Some(action) = action {
        run(hwnd, Effect::Apply(action));
    }
}

/// What releasing the mouse on `hit` does.
fn click_effect(dialog: &mut Dialog, hit: Hit) -> Effect {
    let view = &dialog.view;
    match hit {
        Hit::Close | Hit::TitleClose => Effect::Close,
        Hit::EditIni => Effect::EditIni,
        Hit::Row(row, _) if !view.enabled(row) => Effect::None,
        Hit::Row(row, Part::Whole) => match (row.control(), row.toggle()) {
            (Control::Check, Some(toggle)) => {
                Effect::Apply(super::settings_model::SettingsAction::Toggle(toggle))
            }
            (Control::Dropdown, _) => Effect::OpenDropdown(row),
            _ => Effect::None,
        },
        Hit::Row(row, Part::Segment(index)) => view
            .segment_action(row, index)
            .map_or(Effect::None, Effect::Apply),
        Hit::Row(_, part @ (Part::Minus | Part::Plus)) => {
            dialog.model.typed = None;
            let current = view.settings.font_size;
            let size = step_font_size(current, part == Part::Plus);
            if size == current {
                Effect::Repaint
            } else {
                Effect::Apply(super::settings_model::SettingsAction::SetFontSize(size))
            }
        }
        Hit::Row(_, Part::Value) => Effect::Repaint,
    }
}

/// The focus a click on `hit` moves to.
fn focus_of(hit: Hit) -> Focus {
    match hit {
        Hit::Row(row, _) => Focus::Row(row),
        Hit::EditIni => Focus::EditIni,
        Hit::Close | Hit::TitleClose => Focus::Close,
    }
}

fn list_key(virtual_key: u16) -> Option<ListKey> {
    Some(match virtual_key {
        VK_UP => ListKey::Up,
        VK_DOWN => ListKey::Down,
        VK_PRIOR => ListKey::PageUp,
        VK_NEXT => ListKey::PageDown,
        VK_HOME => ListKey::Home,
        VK_END => ListKey::End,
        VK_RETURN => ListKey::Enter,
        VK_ESCAPE => ListKey::Escape,
        _ => return None,
    })
}

fn model_key(virtual_key: u16) -> Option<Key> {
    Some(match virtual_key {
        VK_TAB => Key::Tab {
            back: unsafe { GetKeyState(i32::from(VK_SHIFT)) } < 0,
        },
        VK_SPACE => Key::Space,
        VK_RETURN => Key::Enter,
        VK_LEFT => Key::Left,
        VK_RIGHT => Key::Right,
        VK_UP => Key::Up,
        VK_DOWN => Key::Down,
        VK_ESCAPE => Key::Escape,
        _ => return None,
    })
}

fn key_down(hwnd: HWND, virtual_key: u16) {
    // With a dropdown open, its keys go to the list; Tab closes it and moves on.
    let list_outcome = state(hwnd).and_then(|dialog| {
        let (_, list) = dialog.list.as_ref()?;
        let key = list_key(virtual_key)?;
        Some(list.key(key, unsafe { GetTickCount() }))
    });
    match list_outcome {
        Some(ListOutcome::Picked(index)) => return pick(hwnd, index),
        Some(ListOutcome::Dismissed) => {
            close_list(hwnd);
            return;
        }
        Some(_) => return,
        None => {
            if virtual_key == VK_TAB {
                close_list(hwnd);
            }
        }
    }
    let Some(key) = model_key(virtual_key) else {
        return;
    };
    let effect = state(hwnd).map(|dialog| dialog.model.key(key, &dialog.view));
    if let Some(effect) = effect {
        run(hwnd, effect);
    }
}

fn char_typed(hwnd: HWND, c: char) {
    let now = unsafe { GetTickCount() };
    let in_list = state(hwnd).and_then(|dialog| {
        let (_, list) = dialog.list.as_ref()?;
        Some(list.key(ListKey::Char(c), now))
    });
    if in_list.is_some() {
        return;
    }
    let key = match c {
        '\u{8}' => Key::Backspace,
        c if c.is_control() => return,
        c => Key::Char(c),
    };
    let effect = state(hwnd).map(|dialog| dialog.model.key(key, &dialog.view));
    if let Some(effect) = effect {
        run(hwnd, effect);
    }
}

unsafe extern "system" fn dialog_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // Checked without forming a reference: `paint` holds a `&Dialog` across `BeginPaint`,
    // which sends WM_ERASEBKGND back here, so neither the check nor that arm may borrow.
    if unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } == 0 {
        return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
    }
    match message {
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            if let Some(dialog) = state(hwnd) {
                paint(hwnd, dialog);
            }
            0
        }
        WM_CLOSE => {
            close(hwnd);
            0
        }
        WM_KEYDOWN => {
            key_down(hwnd, wparam as u16);
            0
        }
        // Alt+Down opens a dropdown, as in a combo box.
        WM_SYSKEYDOWN if wparam as u16 == VK_DOWN => {
            let effect = state(hwnd).and_then(|dialog| {
                dialog
                    .list
                    .is_none()
                    .then(|| dialog.model.key(Key::AltDown, &dialog.view))
            });
            if let Some(effect) = effect {
                run(hwnd, effect);
            }
            0
        }
        WM_CHAR => {
            if let Some(c) = char::from_u32(wparam as u32) {
                char_typed(hwnd, c);
            }
            0
        }
        WM_LIST_PICKED => {
            let (x, y) = lparam_point(lparam);
            let time = unsafe { GetMessageTime() } as u32;
            if let Some(dialog) = state(hwnd) {
                dialog.picked_at = Some((time, POINT { x, y }));
            }
            pick(hwnd, wparam);
            0
        }
        WM_SETTINGS_REFRESH => {
            refresh(hwnd);
            0
        }
        WM_ACTIVATE => {
            if (wparam & 0xffff) as u32 == WA_INACTIVE {
                close_list(hwnd);
            }
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
        WM_MOUSEWHEEL => {
            let delta = ((wparam >> 16) & 0xffff) as i16;
            if let Some(dialog) = state(hwnd) {
                if let Some((_, list)) = &dialog.list {
                    list.wheel(delta);
                } else {
                    let row_height = scale(ROW_HEIGHT_AT_96_DPI, dialog.layout.dpi);
                    let scroll = (dialog.scroll - i32::from(delta) * row_height / 40)
                        .clamp(0, dialog.layout.max_scroll());
                    if scroll != dialog.scroll {
                        dialog.scroll = scroll;
                        invalidate(hwnd);
                    }
                }
            }
            0
        }
        // A press on the title row is a non-client click that starts the move loop without a
        // WM_LBUTTONDOWN or a deactivation: close the list here, as any click elsewhere does.
        WM_NCLBUTTONDOWN => {
            close_list(hwnd);
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
        // Only the title row drags the dialog.
        WM_NCHITTEST => {
            let (x, y) = lparam_point(lparam);
            let mut point = POINT { x, y };
            unsafe { ScreenToClient(hwnd, &mut point) };
            let caption = state(hwnd).is_some_and(|dialog| {
                point.y < dialog.layout.title.bottom && point.x < dialog.layout.title_close.left
            });
            if caption {
                HTCAPTION as LRESULT
            } else {
                HTCLIENT as LRESULT
            }
        }
        WM_SETCURSOR => {
            let mut point = POINT::default();
            unsafe {
                GetCursorPos(&mut point);
                ScreenToClient(hwnd, &mut point);
            }
            let on_link = state(hwnd).is_some_and(|dialog| {
                dialog
                    .layout
                    .hit(point.x, point.y, dialog.scroll, &dialog.view)
                    == Some(Hit::EditIni)
            });
            let cursor = if on_link { IDC_HAND } else { IDC_ARROW };
            unsafe { SetCursor(LoadCursorW(std::ptr::null_mut(), cursor) as HCURSOR) };
            1
        }
        WM_MOUSEMOVE => {
            let (x, y) = lparam_point(lparam);
            if let Some(dialog) = state(hwnd) {
                let hot = dialog.layout.hit(x, y, dialog.scroll, &dialog.view);
                if !dialog.tracking_leave {
                    let mut track = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    dialog.tracking_leave = unsafe { TrackMouseEvent(&mut track) } != 0;
                }
                if hot != dialog.hot {
                    dialog.hot = hot;
                    invalidate(hwnd);
                }
            }
            0
        }
        windows_sys::Win32::UI::Controls::WM_MOUSELEAVE => {
            if let Some(dialog) = state(hwnd) {
                dialog.tracking_leave = false;
                if dialog.hot.take().is_some() {
                    invalidate(hwnd);
                }
            }
            0
        }
        WM_LBUTTONDOWN => {
            let (x, y) = lparam_point(lparam);
            let mut at = POINT { x, y };
            unsafe { ClientToScreen(hwnd, &mut at) };
            let now = unsafe { GetMessageTime() } as u32;
            let limits = unsafe {
                (
                    GetDoubleClickTime(),
                    GetSystemMetrics(SM_CXDOUBLECLK),
                    GetSystemMetrics(SM_CYDOUBLECLK),
                )
            };
            let swallow = state(hwnd).is_some_and(|dialog| {
                let swallow = dialog
                    .picked_at
                    .take()
                    .is_some_and(|picked| completes_double_click(picked, now, at, limits));
                dialog.swallowing = swallow;
                swallow
            });
            if swallow {
                return 0;
            }
            let open_row = state(hwnd).and_then(|dialog| dialog.list.as_ref().map(|(row, _)| *row));
            close_list(hwnd);
            let effect = state(hwnd).and_then(|dialog| {
                dialog.pressed = None;
                let hit = dialog.layout.hit(x, y, dialog.scroll, &dialog.view)?;
                // A click on the dropdown whose list was open only closes that list.
                dialog.pressed =
                    (open_row.map(|row| Hit::Row(row, Part::Whole)) != Some(hit)).then_some(hit);
                // A greyed row takes no focus: it stays where it was.
                Some(match hit {
                    Hit::Row(row, _) if !dialog.view.enabled(row) => Effect::None,
                    _ => dialog.model.set_focus(focus_of(hit), &dialog.view),
                })
            });
            if let Some(effect) = effect {
                unsafe { SetCapture(hwnd) };
                run(hwnd, effect);
            }
            0
        }
        WM_LBUTTONUP => {
            if state(hwnd).is_some_and(|dialog| std::mem::take(&mut dialog.swallowing)) {
                return 0;
            }
            let (x, y) = lparam_point(lparam);
            unsafe { ReleaseCapture() };
            let effect = state(hwnd).and_then(|dialog| {
                let pressed = dialog.pressed.take()?;
                (dialog.layout.hit(x, y, dialog.scroll, &dialog.view) == Some(pressed))
                    .then(|| click_effect(dialog, pressed))
            });
            invalidate(hwnd);
            if let Some(effect) = effect {
                run(hwnd, effect);
            }
            0
        }
        WM_NCDESTROY => {
            let pointer = unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) } as *mut Dialog;
            // SAFETY: from `Box::into_raw` in `create`, released only here.
            drop(unsafe { Box::from_raw(pointer) });
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

/// Paints through an off-screen bitmap so a hover repaint never shows the erase.
fn paint(hwnd: HWND, dialog: &Dialog) {
    paint_buffered(hwnd, |dc, client| paint_into(dc, client, dialog));
}

fn paint_into(dc: HDC, client: RECT, dialog: &Dialog) {
    let colors = dialog.colors;
    let layout = &dialog.layout;
    let view = &dialog.view;
    unsafe {
        fill(dc, client, colors.muted_foreground);
        fill(dc, inset(client, 1), colors.panel_background());
        SetBkMode(dc, TRANSPARENT as i32);

        // Title row: the name and the × button.
        draw_text(
            dc,
            dialog.title_font,
            colors.editor_foreground,
            TITLE,
            layout.title,
            DT_LEFT,
        );
        let close_hot = dialog.hot == Some(Hit::TitleClose);
        if close_hot {
            fill(
                dc,
                inset(layout.title_close, 1),
                colors.close_hover_background,
            );
        }
        draw_text(
            dc,
            dialog.glyph_font,
            if close_hot {
                colors.close_hover_foreground
            } else {
                colors.muted_foreground
            },
            GLYPH_CLOSE,
            layout.title_close,
            DT_CENTER,
        );
        let rule = |top: i32| RECT {
            left: 1,
            top,
            right: client.right - 1,
            bottom: top + 1,
        };
        fill(dc, rule(layout.body.top - 1), colors.strip_background);
        fill(dc, rule(layout.body.bottom), colors.strip_background);

        // The scrolling body, clipped to its area.
        let saved = SaveDC(dc);
        IntersectClipRect(
            dc,
            layout.body.left,
            layout.body.top,
            layout.body.right,
            layout.body.bottom,
        );
        for section in Section::ALL {
            draw_text(
                dc,
                dialog.heading_font,
                colors.muted_foreground,
                &section.title().to_uppercase(),
                layout.heading_rect(section, dialog.scroll),
                DT_LEFT,
            );
        }
        for row in Row::ALL {
            paint_row(dc, dialog, row);
        }
        RestoreDC(dc, saved);

        // Footer: the link and Close.
        draw_text(
            dc,
            dialog.link_font,
            dialog.link_color,
            EDIT_INI_LABEL,
            layout.edit_ini,
            DT_LEFT,
        );
        let button = match (dialog.pressed, dialog.hot) {
            (Some(Hit::Close), _) => colors.pressed_background,
            (_, Some(Hit::Close)) => colors.hover_background,
            _ => colors.strip_background,
        };
        fill(dc, layout.close, colors.muted_foreground);
        fill(dc, inset(layout.close, 1), button);
        draw_text(
            dc,
            dialog.body_font,
            colors.editor_foreground,
            CLOSE_LABEL,
            layout.close,
            DT_CENTER,
        );

        // The focus ring.
        let ring = match dialog.model.focus {
            Focus::EditIni => Some(inset(layout.edit_ini, -2)),
            Focus::Close => Some(inset(layout.close, scale(3, layout.dpi))),
            Focus::Row(row) => {
                let row_rect = layout.row_rect(row, dialog.scroll);
                let visible =
                    row_rect.top >= layout.body.top && row_rect.bottom <= layout.body.bottom;
                visible.then(|| {
                    if row.control() == Control::Check {
                        inset(row_rect, 1)
                    } else {
                        inset(
                            layout.control_rect(row, row_rect, view.segments(row).len()),
                            -2,
                        )
                    }
                })
            }
        };
        if let Some(ring) = ring {
            SetTextColor(dc, colors.editor_foreground);
            DrawFocusRect(dc, &ring);
        }
    }
}

unsafe fn paint_row(dc: HDC, dialog: &Dialog, row: Row) {
    let colors = dialog.colors;
    let layout = &dialog.layout;
    let view = &dialog.view;
    let row_rect = layout.row_rect(row, dialog.scroll);
    if row_rect.bottom < layout.body.top || row_rect.top > layout.body.bottom {
        return;
    }
    let enabled = view.enabled(row);
    let text = if enabled {
        colors.editor_foreground
    } else {
        colors.muted_foreground
    };
    let segments = view.segments(row);
    let control = layout.control_rect(row, row_rect, segments.len());
    unsafe {
        draw_text(dc, dialog.body_font, text, row.label(), row_rect, DT_LEFT);
        let hot = matches!(dialog.hot, Some(Hit::Row(hot_row, _)) if hot_row == row) && enabled;
        match row.control() {
            Control::Check => {
                let checked = row.toggle().is_some_and(|toggle| view.checked(toggle));
                if !enabled {
                    let hint = RECT {
                        right: control.left - scale(12, layout.dpi),
                        ..row_rect
                    };
                    draw_text(
                        dc,
                        dialog.body_font,
                        colors.muted_foreground,
                        AUTOSAVE_HINT,
                        hint,
                        DT_RIGHT,
                    );
                }
                fill(
                    dc,
                    control,
                    if enabled {
                        colors.muted_foreground
                    } else {
                        colors.strip_background
                    },
                );
                let inner = inset(control, 1);
                if checked {
                    fill(dc, inner, colors.selection_background);
                    draw_text(
                        dc,
                        dialog.glyph_font,
                        colors
                            .selection_foreground
                            .unwrap_or(colors.editor_foreground),
                        GLYPH_CHECK,
                        control,
                        DT_CENTER,
                    );
                } else {
                    fill(
                        dc,
                        inner,
                        if hot {
                            colors.hover_background
                        } else {
                            colors.panel_background()
                        },
                    );
                }
            }
            Control::Segmented => {
                fill(dc, control, colors.muted_foreground);
                for (index, (segment, rect)) in segments
                    .iter()
                    .zip(layout.segment_rects(control, segments.len()))
                    .enumerate()
                {
                    let inner = RECT {
                        left: rect.left + i32::from(index == 0),
                        ..inset(rect, 1)
                    };
                    let hot_segment = dialog.hot == Some(Hit::Row(row, Part::Segment(index)));
                    let (background, foreground) = if segment.selected {
                        (
                            colors.selection_background,
                            colors
                                .selection_foreground
                                .unwrap_or(colors.editor_foreground),
                        )
                    } else if hot_segment {
                        (colors.hover_background, colors.editor_foreground)
                    } else {
                        (colors.strip_background, colors.editor_foreground)
                    };
                    fill(dc, inner, background);
                    draw_text(
                        dc,
                        dialog.body_font,
                        foreground,
                        &segment.label,
                        rect,
                        DT_CENTER,
                    );
                }
            }
            Control::Dropdown => {
                let open = matches!(&dialog.list, Some((open_row, _)) if *open_row == row);
                fill(dc, control, colors.muted_foreground);
                fill(
                    dc,
                    inset(control, 1),
                    if hot || open {
                        colors.hover_background
                    } else {
                        colors.strip_background
                    },
                );
                let pad = scale(8, layout.dpi);
                let chevron = RECT {
                    left: control.right - scale(26, layout.dpi),
                    ..control
                };
                let value = RECT {
                    left: control.left + pad,
                    right: chevron.left,
                    ..control
                };
                draw_text(
                    dc,
                    dialog.body_font,
                    colors.editor_foreground,
                    &view.dropdown_text(row),
                    value,
                    DT_LEFT,
                );
                draw_text(
                    dc,
                    dialog.glyph_font,
                    colors.muted_foreground,
                    GLYPH_CHEVRON_DOWN,
                    chevron,
                    DT_CENTER,
                );
            }
            Control::Stepper => {
                let [minus, value, plus] = layout.stepper_rects(control);
                fill(dc, control, colors.muted_foreground);
                for (rect, glyph, part) in [
                    (minus, GLYPH_REMOVE, Part::Minus),
                    (plus, GLYPH_ADD, Part::Plus),
                ] {
                    let background = match (dialog.pressed, dialog.hot) {
                        (Some(pressed), _) if pressed == Hit::Row(row, part) => {
                            colors.pressed_background
                        }
                        (_, Some(hot)) if hot == Hit::Row(row, part) => colors.hover_background,
                        _ => colors.strip_background,
                    };
                    fill(dc, inset(rect, 1), background);
                    draw_text(
                        dc,
                        dialog.glyph_font,
                        colors.editor_foreground,
                        glyph,
                        rect,
                        DT_CENTER,
                    );
                }
                fill(
                    dc,
                    RECT {
                        top: value.top + 1,
                        bottom: value.bottom - 1,
                        ..value
                    },
                    colors.editor_background,
                );
                draw_text(
                    dc,
                    dialog.body_font,
                    colors.editor_foreground,
                    &dialog.model.font_size_text(view),
                    value,
                    DT_CENTER,
                );
            }
        }
    }
}

unsafe fn draw_text(dc: HDC, font: HFONT, color: u32, text: &str, rect: RECT, align: u32) {
    let mut text = wide_null(text);
    let mut rect = rect;
    unsafe {
        let previous = SelectObject(dc, font as _);
        SetTextColor(dc, color);
        DrawTextW(
            dc,
            text.as_mut_ptr(),
            -1,
            &mut rect,
            align | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX | DT_END_ELLIPSIS,
        );
        SelectObject(dc, previous);
    }
}

#[cfg(test)]
type Answer = Box<dyn FnOnce(HWND)>;

#[cfg(test)]
thread_local! {
    static ANSWERS: std::cell::RefCell<std::collections::VecDeque<Answer>> =
        const { std::cell::RefCell::new(std::collections::VecDeque::new()) };
    /// After each applied change: whether the dialog still had the keyboard focus.
    static FOCUS_AFTER_APPLY: std::cell::RefCell<Vec<bool>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Runs `answer` with the next dialog this thread opens, right after it shows, before its loop.
/// Tests post their input from here.
#[cfg(test)]
pub(crate) fn answer_next(answer: impl FnOnce(HWND) + 'static) {
    ANSWERS.with(|answers| answers.borrow_mut().push_back(Box::new(answer)));
}

/// Whether the dialog kept the focus after each change applied since the last call.
#[cfg(test)]
pub(crate) fn take_focus_checks() -> Vec<bool> {
    FOCUS_AFTER_APPLY.with(|checks| std::mem::take(&mut *checks.borrow_mut()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view() -> SettingsView {
        SettingsView {
            settings: crate::config::default_settings(),
            notebook_autosave: None,
        }
    }

    fn center(rect: RECT) -> (i32, i32) {
        ((rect.left + rect.right) / 2, (rect.top + rect.bottom) / 2)
    }

    #[test]
    fn rows_stack_under_their_headings_and_everything_fits_at_96_dpi() {
        // Break caught: rows overlapping, a heading painted under its first row, or a dialog
        // that scrolls on an ordinary screen.
        let layout = Layout::calculate(96, 2000, 100);
        assert_eq!(layout.width, 520);
        assert_eq!(layout.max_scroll(), 0);
        let mut expected_top = 0;
        for section in Section::ALL {
            assert_eq!(layout.headings[section as usize].top, expected_top);
            expected_top = layout.headings[section as usize].bottom;
            for row in Row::ALL.into_iter().filter(|row| row.section() == section) {
                assert_eq!(layout.rows[row as usize].top, expected_top, "{row:?}");
                expected_top = layout.rows[row as usize].bottom;
            }
        }
        assert_eq!(layout.height, 44 + layout.content_height + 56);
        assert!(layout.edit_ini.right <= layout.close.left);
    }

    #[test]
    fn hits_find_controls_checkbox_labels_segments_and_stepper_parts() {
        let layout = Layout::calculate(96, 2000, 100);
        let view = view();
        let hit = |rect: RECT| {
            let (x, y) = center(rect);
            layout.hit(x, y, 0, &view)
        };
        let row = |row| layout.row_rect(row, 0);
        let control = |r: Row| layout.control_rect(r, row(r), view.segments(r).len());

        assert_eq!(
            hit(control(Row::Theme)),
            Some(Hit::Row(Row::Theme, Part::Whole))
        );
        let label = RECT {
            right: row(Row::Theme).left + 40,
            ..row(Row::Theme)
        };
        assert_eq!(hit(label), None, "a dropdown's label is not the dropdown");
        let label = RECT {
            right: row(Row::WordWrap).left + 40,
            ..row(Row::WordWrap)
        };
        assert_eq!(
            hit(label),
            Some(Hit::Row(Row::WordWrap, Part::Whole)),
            "a checkbox's label toggles it"
        );

        let segments = layout.segment_rects(control(Row::TabWidth), 3);
        assert_eq!(
            hit(segments[2]),
            Some(Hit::Row(Row::TabWidth, Part::Segment(2)))
        );
        let [minus, value, plus] = layout.stepper_rects(control(Row::FontSize));
        assert_eq!(hit(minus), Some(Hit::Row(Row::FontSize, Part::Minus)));
        assert_eq!(hit(value), Some(Hit::Row(Row::FontSize, Part::Value)));
        assert_eq!(hit(plus), Some(Hit::Row(Row::FontSize, Part::Plus)));

        assert_eq!(hit(layout.close), Some(Hit::Close));
        assert_eq!(hit(layout.edit_ini), Some(Hit::EditIni));
        assert_eq!(hit(layout.title_close), Some(Hit::TitleClose));
        assert_eq!(hit(layout.title), None);
    }

    #[test]
    fn only_a_quick_nearby_press_after_a_pick_completes_a_double_click() {
        // Break caught: the second click of a double-click on a dropdown item toggling the
        // checkbox row under it, or every later click after a pick being swallowed (final
        // review 3).
        let picked = (1_000, POINT { x: 100, y: 200 });
        let limits = (500, 4, 4);
        let near = POINT { x: 102, y: 198 };
        assert!(completes_double_click(picked, 1_300, near, limits));
        assert!(
            !completes_double_click(picked, 1_600, near, limits),
            "too late"
        );
        assert!(
            !completes_double_click(picked, 1_300, POINT { x: 103, y: 200 }, limits),
            "too far"
        );
        assert!(
            completes_double_click((u32::MAX - 10, picked.1), 100, near, limits),
            "the tick count wrapping between the clicks"
        );
    }

    #[test]
    fn the_layout_scales_with_dpi() {
        let normal = Layout::calculate(96, 4000, 100);
        let double = Layout::calculate(192, 4000, 200);
        assert_eq!(double.width, normal.width * 2);
        assert_eq!(double.content_height, normal.content_height * 2);
        assert_eq!(double.rows[5].top, normal.rows[5].top * 2);
    }

    #[test]
    fn a_short_work_area_caps_the_height_and_scrolls_the_focused_row_into_view() {
        // Break caught: a dialog taller than a 1366×768 screen at 150%, with Close off-screen,
        // or Tab moving the focus to a row the body never scrolls to (review focus 4).
        let layout = Layout::calculate(144, 700, 150);
        assert_eq!(layout.height, 700);
        assert!(layout.max_scroll() > 0);
        let visible = layout.body.bottom - layout.body.top;
        let scroll = layout.scroll_to_show(Focus::Row(Row::NotebookAutosave), 0);
        let bottom = layout.rows[Row::NotebookAutosave as usize].bottom;
        assert_eq!(
            scroll,
            bottom - visible,
            "just enough to show the last row whole"
        );
        assert!(scroll <= layout.max_scroll());
        assert_eq!(
            layout.scroll_to_show(Focus::Row(Row::Theme), scroll),
            0,
            "the first row brings its heading back"
        );
        assert_eq!(layout.scroll_to_show(Focus::Close, 37), 37);
        let tiny = Layout::calculate(96, 100, 100);
        assert!(tiny.height > 100, "at least a few rows always show");
    }
}
