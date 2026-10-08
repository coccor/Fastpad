//! Settings: a themed, owner-drawn modal popup listing every user-facing `fastpad.ini` setting
//! (settings dialog spec §3). Each change applies and saves at once through
//! `main_window::apply_settings_action`. Like About, it runs its own modal loop with the main
//! window disabled. All behaviour lives in `settings_model`; this module decodes input and
//! paints.

use super::design::metrics::scale;
use super::design::metrics::{CONTROL_RADIUS, FOCUS_GAP, FOCUS_RING};
use super::design::text_scale::scale_text;
use super::design::type_ramp::{self, TextStyle};
use super::dropdown_list::{
    DropdownList, ListKey, ListModel, ListOutcome, ListStyle, WM_LIST_PICKED,
};
use super::keymap::KeyStroke;
use super::modal::ModalScope;
use super::palette::Palette;
use super::panel::inset;
use super::settings_model::{
    Control, DialogModel, Effect, Focus, Key, Page, Row, Section, SettingsView, dropdown_action,
    dropdown_step, step_effect, step_value, stepper_value,
};
use super::side_panel::paint_buffered;
use super::soft_paint::{
    Canvas, Frame, Shape, TITLE_CLOSE_WIDTH_AT_96_DPI, TITLE_HEIGHT_AT_96_DPI, Tones,
    glyph_font_face, title_close,
};
use super::titlebar::create_ui_font;
use crate::platform::wide_null;
use std::cell::Cell;
use std::rc::Rc;
use windows_sys::Win32::Foundation::{
    ERROR_CLASS_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows_sys::Win32::Graphics::Dwm::{
    DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmExtendFrameIntoClientArea,
    DwmSetWindowAttribute,
};
use windows_sys::Win32::Graphics::Gdi::{
    ClientToScreen, CreateSolidBrush, DT_CALCRECT, DT_CENTER, DT_LEFT, DT_NOPREFIX, DT_RIGHT,
    DT_SINGLELINE, DeleteObject, DrawTextW, FW_NORMAL, GetDC, GetMonitorInfoW, HBRUSH, HDC, HFONT,
    InvalidateRect, MONITOR_DEFAULTTONEAREST, MONITORINFO, MapWindowPoints, MonitorFromWindow,
    ReleaseDC, ScreenToClient, SelectObject, SetBkColor, SetTextColor,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::SystemInformation::GetTickCount;
use windows_sys::Win32::UI::Controls::MARGINS;
use windows_sys::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    EnableWindow, GetDoubleClickTime, GetFocus, GetKeyState, ReleaseCapture, SetCapture, SetFocus,
    TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent, VK_CONTROL, VK_DOWN, VK_END, VK_ESCAPE, VK_HOME,
    VK_LEFT, VK_MENU, VK_NEXT, VK_PRIOR, VK_RETURN, VK_RIGHT, VK_SHIFT, VK_SPACE, VK_TAB, VK_UP,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, FindWindowExW, GW_OWNER,
    GWLP_USERDATA, GetCursorPos, GetMessageTime, GetMessageW, GetSystemMetrics, GetWindow,
    GetWindowLongPtrW, GetWindowRect, GetWindowTextLengthW, GetWindowTextW, HCURSOR, HTBOTTOM,
    HTBOTTOMLEFT, HTBOTTOMRIGHT, HTCAPTION, HTCLIENT, HTLEFT, HTRIGHT, HTTOP, HTTOPLEFT,
    HTTOPRIGHT, IDC_ARROW, IDC_HAND, IsWindow, IsZoomed, LoadCursorW, MINMAXINFO, MSG,
    PostMessageW, PostQuitMessage, RegisterClassW, SM_CXDOUBLECLK, SM_CXPADDEDBORDER,
    SM_CXSIZEFRAME, SM_CYDOUBLECLK, SW_HIDE, SW_SHOW, SWP_NOACTIVATE, SWP_NOZORDER, SetCursor,
    SetWindowLongPtrW, SetWindowPos, SetWindowTextW, ShowWindow, TranslateMessage, WA_INACTIVE,
    WM_ACTIVATE, WM_CHAR, WM_CLOSE, WM_ERASEBKGND, WM_GETMINMAXINFO, WM_KEYDOWN, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCCALCSIZE, WM_NCDESTROY, WM_NCHITTEST,
    WM_NCLBUTTONDOWN, WM_PAINT, WM_SETCURSOR, WM_SIZE, WM_SYSKEYDOWN, WNDCLASSW, WS_CAPTION,
    WS_CLIPCHILDREN, WS_EX_TOOLWINDOW, WS_MAXIMIZEBOX, WS_POPUP, WS_THICKFRAME,
};

mod layout;
pub(crate) use layout::*;
mod shortcuts_input;
pub(crate) use shortcuts_input::*;
mod input;
use input::*;
mod painting;
use painting::*;

const TITLE: &str = "Settings";
const EDIT_INI_LABEL: &str = "Edit fastpad.ini";
const CLOSE_LABEL: &str = "Close";
const AUTOSAVE_HINT: &str = "Open a notebook to change this";
const GLYPH_CHEVRON_DOWN: &str = "\u{E70D}";
const GLYPH_ADD: &str = "\u{E710}";
const GLYPH_REMOVE: &str = "\u{E738}";

/// The width, unless the work area is narrower.
const WIDTH_AT_96_DPI: i32 = 860;
/// The smallest the user can size the dialog to: the nav, a usable card column and a few rows.
const MIN_WIDTH_AT_96_DPI: i32 = 600;
const MIN_HEIGHT_AT_96_DPI: i32 = 360;
const NAV_WIDTH_AT_96_DPI: i32 = 180;
const NAV_ITEM_HEIGHT_AT_96_DPI: i32 = 32;
const NAV_INSET_AT_96_DPI: i32 = 8;
const NAV_BAR_WIDTH_AT_96_DPI: i32 = 3;
const PADDING_AT_96_DPI: i32 = 20;
const HEADING_HEIGHT_AT_96_DPI: i32 = 30;
/// A heading's text sits this far below the top of its space, closer to its first card.
const HEADING_SPACE_ABOVE_AT_96_DPI: i32 = 6;
/// Each setting sits on its own card, this tall with this gap under it: 14 cards and 3 headings
/// keep the dialog under 700 px at 96 DPI.
const CARD_HEIGHT_AT_96_DPI: i32 = 34;
const CARD_GAP_AT_96_DPI: i32 = 2;
/// Between a card's sides and its label or control.
const CARD_PADDING_AT_96_DPI: i32 = 16;
const CONTROL_HEIGHT_AT_96_DPI: i32 = 26;
/// A dropdown's width, room for long font names; in a dialog narrowed by its work area it gives
/// up width before its label's `LABEL_MIN_WIDTH_AT_96_DPI`.
const DROPDOWN_WIDTH_AT_96_DPI: i32 = 360;
const LABEL_MIN_WIDTH_AT_96_DPI: i32 = 120;
const SEGMENT_WIDTH_AT_96_DPI: i32 = 80;
const TAB_SEGMENT_WIDTH_AT_96_DPI: i32 = 44;
const STEP_BUTTON_AT_96_DPI: i32 = 28;
const STEP_VALUE_AT_96_DPI: i32 = 48;
const TOGGLE_WIDTH_AT_96_DPI: i32 = 40;
const TOGGLE_HEIGHT_AT_96_DPI: i32 = 20;
/// Between a toggle's track and its round knob.
const KNOB_INSET_AT_96_DPI: i32 = 4;
/// The On/Off text left of a toggle: its width and its gap to the switch.
const TOGGLE_STATE_WIDTH_AT_96_DPI: i32 = 28;
const TOGGLE_STATE_GAP_AT_96_DPI: i32 = 10;
const FOOTER_HEIGHT_AT_96_DPI: i32 = 56;
const BUTTON_WIDTH_AT_96_DPI: i32 = 88;
const BUTTON_HEIGHT_AT_96_DPI: i32 = 30;
/// However short the screen, at least this many cards stay visible.
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
    Nav(Page),
    EditIni,
    Close,
    TitleClose,
    /// Something on the Keyboard Shortcuts page.
    Page(super::shortcuts_page::PageHit),
}

/// The open dialog's state, owned by its window through `GWLP_USERDATA`.
struct Dialog {
    colors: Palette,
    link_color: u32,
    layout: Layout,
    view: SettingsView,
    model: DialogModel,
    fonts: Vec<String>,
    /// The Preview font row's list: every family, sorted together.
    preview_fonts: Vec<String>,
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
    /// Direct2D for the rounded shapes, loaded as the dialog opens.
    canvas: Canvas,
    page_layout: super::shortcuts_page::PageLayout,
    shortcuts: super::shortcuts_model::ShortcutsModel,
    /// When and on which row the last click in the table was, for double-clicks.
    last_row_click: Option<(u32, usize)>,
    /// Wheel travel on the shortcuts table not yet a whole row, in 1/120ths of a row (a notch,
    /// 120, is three rows). Dropped when the wheel turns the other way.
    wheel_rest: i32,
    /// The shortcuts page's native search field, created with the dialog and destroyed with it;
    /// `None` if the EDIT could not be created.
    search: Option<HWND>,
    /// The search field's background (`WM_CTLCOLOREDIT`).
    search_brush: HBRUSH,
}

impl Dialog {
    /// The font list `row`'s dropdown shows.
    fn fonts_for(&self, row: Row) -> &[String] {
        if row == Row::PreviewFont {
            &self.preview_fonts
        } else {
            &self.fonts
        }
    }
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
            DeleteObject(self.search_brush as _);
        }
    }
}

/// The page the open dialog shows.
#[cfg(test)]
pub(crate) fn current_page(dialog: HWND) -> Option<Page> {
    state(dialog).map(|dialog| dialog.model.page)
}

/// Shows Settings over `owner` and returns once it is closed.
pub(crate) fn show(owner: HWND, colors: Palette, link_color: u32, page: Page) -> Outcome {
    let _modal = ModalScope::enter(owner);
    let outcome = Rc::new(Cell::new(Outcome::Closed));
    let Some(dialog) = create(owner, colors, link_color, page, outcome.clone()) else {
        return Outcome::Closed;
    };
    let search = state(dialog)
        .filter(|state| state.model.page == Page::Shortcuts)
        .and_then(|state| state.search);
    unsafe {
        EnableWindow(owner, 0);
        if let Some(search) = search {
            ShowWindow(search, SW_SHOW);
        }
        ShowWindow(dialog, SW_SHOW);
        SetFocus(search.unwrap_or(dialog));
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
    page: Page,
    outcome: Rc<Cell<Outcome>>,
) -> Option<HWND> {
    let class = register_class()?;
    let title = wide_null(TITLE);
    let dialog = unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW,
            class.as_ptr(),
            title.as_ptr(),
            // WS_CAPTION gives it a native frame, hidden by WM_NCCALCSIZE, so DWM draws the
            // window shadow; no system menu. It sizes like a window: WS_THICKFRAME for dragging
            // its edges (hit-tested in WM_NCHITTEST, as the frame is hidden), snapping and
            // maximizing, WS_MAXIMIZEBOX for a double-click on the title row.
            WS_POPUP | WS_CAPTION | WS_THICKFRAME | WS_MAXIMIZEBOX | WS_CLIPCHILDREN,
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
    let title_font = type_ramp::create(TextStyle::Title, dpi);
    let heading_font = type_ramp::create(TextStyle::Heading, dpi);
    let body_font = type_ramp::create(TextStyle::Body, dpi);
    let link_font = create_underlined_font(crate::window::design::text_scale::scale_text(13, dpi));
    let glyph_font = create_ui_font(scale(11, dpi), glyph_font_face(), FW_NORMAL as i32, false);

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
    let (max_width, max_height) = if work.bottom > work.top && work.right > work.left {
        (work.right - work.left, work.bottom - work.top)
    } else {
        (i32::MAX, i32::MAX)
    };
    let view = super::main_window::settings_view(owner);
    let link_width = measure(dialog, link_font, EDIT_INI_LABEL);
    let layout = match view.settings.settings_size {
        Some(saved) => {
            let (width, height) = opening_size(saved, dpi, max_width, max_height);
            Layout::sized(dpi, width, height, link_width)
        }
        None => Layout::calculate(dpi, max_width, max_height, link_width),
    };
    // Built here, as the dialog opens: nothing of the shortcuts page runs before.
    let page_layout = super::shortcuts_page::PageLayout::calculate(layout.body, dpi);
    let mut shortcuts = super::shortcuts_model::ShortcutsModel::new(
        super::main_window::keymap(owner),
        page_layout.visible_rows(),
    );
    shortcuts.lines = super::main_window::key_line_commands(owner);
    let families = crate::platform::fonts::installed_font_families();
    let preview_fonts = crate::platform::fonts::preview_dropdown_names(
        families.clone(),
        &view.settings.preview_font,
    );
    let fonts = crate::platform::fonts::dropdown_names(families, &view.settings.font_face);
    let state = Box::new(Dialog {
        colors,
        link_color,
        layout,
        view,
        model: DialogModel::new(page),
        fonts,
        preview_fonts,
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
        canvas: Canvas::load(),
        page_layout,
        shortcuts,
        last_row_click: None,
        wheel_rest: 0,
        search: None,
        search_brush: unsafe { CreateSolidBrush(Tones::new(&colors).control) },
    });
    unsafe { SetWindowLongPtrW(dialog, GWLP_USERDATA, Box::into_raw(state) as isize) };
    // Created here, with the dialog: nothing of the page exists before it opens.
    let field = search_field(&page_layout, dpi);
    let search = super::shortcuts_page::create_search(dialog, field, body_font);
    if let Some(created) = self::state(dialog) {
        created.search = Some(search).filter(|search| !search.is_null());
    }

    // Centered over the owner, kept inside the work area, with rounded corners where Windows
    // 11 draws them and the DWM shadow of the hidden frame (a 1-px frame margin keeps DWM
    // drawing it though the client covers the whole window).
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

/// The size, in pixels at `dpi`, to open at for a `saved` size in 96-DPI pixels: no smaller
/// than the minimum and no larger than the `max_width` by `max_height` work area.
pub(crate) fn opening_size(
    saved: (u16, u16),
    dpi: u32,
    max_width: i32,
    max_height: i32,
) -> (i32, i32) {
    let (min_width, min_height) = Layout::min_size(dpi);
    let fit = |value: u16, min: i32, max: i32| scale(i32::from(value), dpi).max(min).min(max);
    (
        fit(saved.0, min_width, max_width),
        fit(saved.1, min_height, max_height),
    )
}

/// `layout`'s size in 96-DPI pixels, as `settings_size` saves it.
pub(crate) fn saved_size(layout: &Layout) -> (u16, u16) {
    let unscale = |value: i32| {
        let dpi = layout.dpi.max(1) as i32;
        u16::try_from((value * 96 + dpi / 2) / dpi).unwrap_or(u16::MAX)
    };
    (unscale(layout.width), unscale(layout.height))
}

/// Saves the size the user dragged the dialog to, unless it is maximized (that is no size to
/// reopen at) or the drag only moved it.
fn save_size(hwnd: HWND) {
    if unsafe { IsZoomed(hwnd) } != 0 {
        return;
    }
    let Some(size) = state(hwnd).map(|dialog| saved_size(&dialog.layout)) else {
        return;
    };
    super::main_window::change_setting(owner(hwnd), |settings| {
        (settings.settings_size != Some(size)).then(|| {
            settings.settings_size = Some(size);
            ("settings_size", format!("{}x{}", size.0, size.1))
        })
    });
}

/// Where the search EDIT sits in the page's search box.
fn search_field(page_layout: &super::shortcuts_page::PageLayout, dpi: u32) -> RECT {
    super::shortcuts_page::field_rect(page_layout.search, scale(18, dpi), dpi)
}

/// Lays the dialog out again at its new client size, `width` by `height`.
fn resized(hwnd: HWND, width: i32, height: i32) {
    let moved = state(hwnd).and_then(|dialog| {
        let layout = &dialog.layout;
        if (layout.width, layout.height) == (width, height) {
            return None;
        }
        let dpi = layout.dpi;
        dialog.layout = Layout::sized(dpi, width, height, layout.link_width);
        dialog.scroll = dialog
            .layout
            .scroll_to_show(dialog.model.focus, dialog.scroll)
            .clamp(0, dialog.layout.max_scroll());
        dialog.page_layout = super::shortcuts_page::PageLayout::calculate(dialog.layout.body, dpi);
        dialog
            .shortcuts
            .set_visible(dialog.page_layout.visible_rows());
        dialog.hot = None;
        Some((dialog.search, search_field(&dialog.page_layout, dpi)))
    });
    let Some((search, field)) = moved else {
        return;
    };
    // An open dropdown hangs off a row that has moved.
    close_list(hwnd);
    if let Some(search) = search {
        unsafe {
            SetWindowPos(
                search,
                std::ptr::null_mut(),
                field.left,
                field.top,
                field.right - field.left,
                field.bottom - field.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }
    invalidate(hwnd);
}

/// The sizing edge or corner under client point `x`, `y` of a `width` by `height` dialog, within
/// `border` of its edges.
pub(crate) fn sizing_edge(x: i32, y: i32, width: i32, height: i32, border: i32) -> Option<u32> {
    let left = x < border;
    let right = x >= width - border;
    let top = y < border;
    let bottom = y >= height - border;
    Some(match (left, right, top, bottom) {
        (true, _, true, _) => HTTOPLEFT,
        (_, true, true, _) => HTTOPRIGHT,
        (true, _, _, true) => HTBOTTOMLEFT,
        (_, true, _, true) => HTBOTTOMRIGHT,
        (true, ..) => HTLEFT,
        (_, true, ..) => HTRIGHT,
        (_, _, true, _) => HTTOP,
        (.., true) => HTBOTTOM,
        _ => return None,
    })
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
        // No CS_DROPSHADOW: the hidden native frame's DWM shadow replaces it.
        let class = WNDCLASSW {
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
    let face = wide_null(crate::window::design::faces::current().text);
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
            sync_focus(hwnd);
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
        Effect::StepDropdown(row, forward) => {
            // The borrow ends before the change applies, as for a pick from the list.
            let action = state(hwnd).and_then(|dialog| {
                dropdown_step(row, forward, &dialog.view, dialog.fonts_for(row))
            });
            if let Some(action) = action {
                run(hwnd, Effect::Apply(action));
            }
        }
        Effect::ShowPage(page) => {
            let search = state(hwnd).and_then(|dialog| {
                dialog.model.show_page(page);
                dialog.search
            });
            if let Some(search) = search {
                let show = if page == Page::Shortcuts {
                    SW_SHOW
                } else {
                    SW_HIDE
                };
                unsafe { ShowWindow(search, show) };
            }
            invalidate(hwnd);
            sync_focus(hwnd);
        }
        Effect::EditIni => {
            if let Some(dialog) = state(hwnd) {
                dialog.outcome.set(Outcome::EditIni);
            }
            close(hwnd);
        }
        Effect::Close => close(hwnd),
    }
}

/// Puts the keyboard focus where the model has it: in the search field, or on the dialog.
fn sync_focus(hwnd: HWND) {
    let Some((wants_search, search)) =
        state(hwnd).map(|dialog| (dialog.model.focus == Focus::Search, dialog.search))
    else {
        return;
    };
    // Without a field the focus stays on the dialog.
    let Some(search) = search else {
        return;
    };
    let focus = unsafe { GetFocus() };
    if wants_search && focus != search {
        unsafe { SetFocus(search) };
    } else if !wants_search && focus == search {
        unsafe { SetFocus(hwnd) };
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
    let mut repaint_search = None;
    if let Some(dialog) = state(hwnd) {
        if dialog.colors != colors {
            unsafe {
                DeleteObject(dialog.search_brush as _);
                dialog.search_brush = CreateSolidBrush(Tones::new(&colors).control);
            }
            repaint_search = dialog.search;
        }
        dialog.view = view;
        dialog.colors = colors;
        dialog.link_color = link_color;
        dialog.scroll = dialog
            .layout
            .scroll_to_show(dialog.model.focus, dialog.scroll);
    }
    if let Some(search) = repaint_search {
        unsafe { InvalidateRect(search, std::ptr::null(), 1) };
    }
    invalidate(hwnd);
}

fn open_list(hwnd: HWND, row: Row) {
    let Some(dialog) = state(hwnd) else {
        return;
    };
    dialog.list = None;
    let (items, selected) = dialog.view.dropdown(row, dialog.fonts_for(row));
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
        ListStyle {
            row_height: dialog.layout.list_row_height(),
            radius: dialog.layout.radius(),
            dpi: dialog.layout.dpi,
            font: dialog.body_font,
            colors: dialog.colors,
            canvas: dialog.canvas.clone(),
        },
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
        dropdown_action(row, index, dialog.fonts_for(row))
    });
    invalidate(hwnd);
    if let Some(action) = action {
        run(hwnd, Effect::Apply(action));
    }
}

/// What client point `x`, `y` is on, on the page shown. The recording box takes every click.
fn hit_at(dialog: &Dialog, x: i32, y: i32) -> Option<Hit> {
    let shortcuts = dialog.model.page == Page::Shortcuts;
    if shortcuts && dialog.shortcuts.recording.is_some() {
        return dialog
            .page_layout
            .hit(x, y, &dialog.shortcuts)
            .map(Hit::Page);
    }
    dialog
        .layout
        .hit(x, y, dialog.scroll, &dialog.view, dialog.model.page)
        .or_else(|| {
            shortcuts
                .then(|| dialog.page_layout.hit(x, y, &dialog.shortcuts))
                .flatten()
                .map(Hit::Page)
        })
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

/// The client centre of the shortcuts table's `slot`th visible row.
#[cfg(test)]
pub(crate) fn page_row_point(dialog: HWND, slot: usize) -> (i32, i32) {
    let rect = state(dialog)
        .map(|dialog| dialog.page_layout.row_rect(slot))
        .unwrap_or_default();
    ((rect.left + rect.right) / 2, (rect.top + rect.bottom) / 2)
}

/// A copy of the open dialog's shortcuts page state.
#[cfg(test)]
pub(crate) fn shortcuts_model(dialog: HWND) -> Option<super::shortcuts_model::ShortcutsModel> {
    state(dialog).map(|dialog| dialog.shortcuts.clone())
}

#[cfg(test)]
const WM_TEST_ANSWER: u32 = windows_sys::Win32::UI::WindowsAndMessaging::WM_APP + 3;

#[cfg(test)]
thread_local! {
    static IN_LOOP: std::cell::RefCell<std::collections::VecDeque<Answer>> =
        const { std::cell::RefCell::new(std::collections::VecDeque::new()) };
}

/// Runs `answer` from the dialog's loop after the input posted so far.
#[cfg(test)]
pub(crate) fn answer_in_loop(dialog: HWND, answer: impl FnOnce(HWND) + 'static) {
    IN_LOOP.with(|answers| answers.borrow_mut().push_back(Box::new(answer)));
    unsafe { PostMessageW(dialog, WM_TEST_ANSWER, 0, 0) };
}

/// The shortcuts page's search field.
#[cfg(test)]
pub(crate) fn search_hwnd(dialog: HWND) -> HWND {
    state(dialog)
        .and_then(|dialog| dialog.search)
        .unwrap_or(std::ptr::null_mut())
}

/// What has the keyboard focus, as the dialog's model sees it.
#[cfg(test)]
pub(crate) fn current_focus(dialog: HWND) -> Option<Focus> {
    state(dialog).map(|dialog| dialog.model.focus)
}

/// Alt+K's path from outside the search field, without a held Alt: toggles record-keys search.
#[cfg(test)]
pub(crate) fn toggle_record_keys_for_test(dialog: HWND) {
    toggle_record_keys(dialog);
}

/// Whether the dialog kept the focus after each change applied since the last call.
#[cfg(test)]
pub(crate) fn take_focus_checks() -> Vec<bool> {
    FOCUS_AFTER_APPLY.with(|checks| std::mem::take(&mut *checks.borrow_mut()))
}

#[cfg(test)]
mod tests;
