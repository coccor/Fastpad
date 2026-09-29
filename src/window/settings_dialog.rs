//! Settings: a themed, owner-drawn modal popup listing every user-facing `fastpad.ini` setting
//! (settings dialog spec §3). Each change applies and saves at once through
//! `main_window::apply_settings_action`. Like About, it runs its own modal loop with the main
//! window disabled. All behaviour lives in `settings_model`; this module decodes input and
//! paints.

use super::dropdown_list::{
    DropdownList, ListKey, ListModel, ListOutcome, ListStyle, WM_LIST_PICKED,
};
use super::keymap::KeyStroke;
use super::modal::ModalScope;
use super::palette::Palette;
use super::panel::{inset, scale};
use super::settings_model::{
    Control, DialogModel, Effect, Focus, Key, Page, Row, Section, SettingsView, dropdown_action,
    dropdown_step, step_font_size,
};
use super::side_panel::paint_buffered;
use super::soft_paint::{
    Canvas, FOCUS_GAP_AT_96_DPI, FOCUS_WIDTH_AT_96_DPI, Frame, GLYPH_FONT, RADIUS_AT_96_DPI, Shape,
    TITLE_CLOSE_WIDTH_AT_96_DPI, TITLE_HEIGHT_AT_96_DPI, Tones, title_close,
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
    DT_SINGLELINE, DeleteObject, DrawTextW, FW_NORMAL, FW_SEMIBOLD, GetDC, GetMonitorInfoW, HBRUSH,
    HDC, HFONT, InvalidateRect, MONITOR_DEFAULTTONEAREST, MONITORINFO, MapWindowPoints,
    MonitorFromWindow, ReleaseDC, ScreenToClient, SelectObject, SetBkColor, SetTextColor,
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
/// Each setting sits on its own card, this tall with this gap under it: 13 cards and 3 headings
/// keep the dialog under 700 px at 96 DPI.
const CARD_HEIGHT_AT_96_DPI: i32 = 36;
const CARD_GAP_AT_96_DPI: i32 = 3;
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

/// Where everything sits. Headings and rows are in content coordinates, placed in the
/// scrolling `body` by `row_rect`/`heading_rect`. A row's rect is its card, without the gap
/// under it.
#[derive(Clone, Copy)]
pub(crate) struct Layout {
    pub width: i32,
    pub height: i32,
    pub title: RECT,
    pub title_close: RECT,
    pub body: RECT,
    pub nav: RECT,
    pub nav_items: [RECT; 2],
    pub headings: [RECT; 3],
    pub rows: [RECT; 13],
    pub content_height: i32,
    pub edit_ini: RECT,
    pub close: RECT,
    dpi: u32,
    link_width: i32,
}

impl Layout {
    /// The layout at `dpi` at its natural size, at most `max_width` wide and `max_height` tall
    /// (the work area), with the Edit fastpad.ini link `link_width` wide.
    pub(crate) fn calculate(dpi: u32, max_width: i32, max_height: i32, link_width: i32) -> Self {
        let width = scale(WIDTH_AT_96_DPI, dpi);
        // Only the content's height is read off this one.
        let probe = Self::sized(dpi, width, 0, link_width);
        let chrome = probe.title.bottom + scale(FOOTER_HEIGHT_AT_96_DPI, dpi);
        let natural = chrome + probe.content_height;
        let smallest = chrome + probe.row_pitch() * MIN_VISIBLE_ROWS;
        Self::sized(
            dpi,
            width.min(max_width),
            natural.min(max_height.max(smallest)),
            link_width,
        )
    }

    /// The smallest size the user can drag the dialog to.
    pub(crate) fn min_size(dpi: u32) -> (i32, i32) {
        (
            scale(MIN_WIDTH_AT_96_DPI, dpi),
            scale(MIN_HEIGHT_AT_96_DPI, dpi),
        )
    }

    /// The layout `width` by `height`, as the dialog opens or as the user sized it.
    pub(crate) fn sized(dpi: u32, width: i32, height: i32, link_width: i32) -> Self {
        let padding = scale(PADDING_AT_96_DPI, dpi);
        let title_height = scale(TITLE_HEIGHT_AT_96_DPI, dpi);
        let heading_height = scale(HEADING_HEIGHT_AT_96_DPI, dpi);
        let card_height = scale(CARD_HEIGHT_AT_96_DPI, dpi);
        let row_pitch = card_height + scale(CARD_GAP_AT_96_DPI, dpi);
        let footer_height = scale(FOOTER_HEIGHT_AT_96_DPI, dpi);

        // The × fills the title row's top-right corner, full height, like a caption button.
        let title_close_left = width - scale(TITLE_CLOSE_WIDTH_AT_96_DPI, dpi);
        let title = RECT {
            left: padding,
            top: 0,
            right: title_close_left,
            bottom: title_height,
        };
        let title_close = RECT {
            left: title_close_left,
            top: 0,
            right: width,
            bottom: title_height,
        };

        let nav_width = scale(NAV_WIDTH_AT_96_DPI, dpi).min(width / 3);
        let line = |top: i32, height: i32| RECT {
            left: nav_width + padding,
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
                rows[row as usize] = line(top, card_height);
                top += row_pitch;
            }
        }
        // The last card's gap separates it from the footer.
        let content_height = top;

        let body = RECT {
            left: nav_width,
            top: title_height,
            right: width,
            bottom: height - footer_height,
        };
        let nav = RECT {
            left: 0,
            top: title_height,
            right: nav_width,
            bottom: body.bottom,
        };
        let item_height = scale(NAV_ITEM_HEIGHT_AT_96_DPI, dpi);
        let nav_inset = scale(NAV_INSET_AT_96_DPI, dpi);
        let nav_items = std::array::from_fn(|index| {
            let top = nav.top + nav_inset + index as i32 * item_height;
            RECT {
                left: nav_inset,
                top,
                right: nav_width - nav_inset,
                bottom: top + item_height,
            }
        });
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
            nav,
            nav_items,
            headings,
            rows,
            content_height,
            edit_ini,
            close,
            dpi,
            link_width,
        }
    }

    pub(crate) fn max_scroll(&self) -> i32 {
        (self.content_height - (self.body.bottom - self.body.top)).max(0)
    }

    pub(crate) fn list_row_height(&self) -> i32 {
        scale(CONTROL_HEIGHT_AT_96_DPI, self.dpi)
    }

    /// The corner radius of cards and controls.
    pub(crate) fn radius(&self) -> i32 {
        scale(RADIUS_AT_96_DPI, self.dpi)
    }

    /// A card and the gap under it, as `calculate` stacks them; a wheel notch scrolls three.
    fn row_pitch(&self) -> i32 {
        scale(CARD_HEIGHT_AT_96_DPI, self.dpi) + scale(CARD_GAP_AT_96_DPI, self.dpi)
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

    /// The control of `row`, right-aligned inside the padding of its card `row_rect`.
    /// `segments` is how many a segmented row shows.
    pub(crate) fn control_rect(&self, row: Row, row_rect: RECT, segments: usize) -> RECT {
        let dpi = self.dpi;
        let (width, height) = match row.control() {
            Control::Dropdown => {
                let room = row_rect.right
                    - row_rect.left
                    - scale(CARD_PADDING_AT_96_DPI, dpi) * 2
                    - scale(LABEL_MIN_WIDTH_AT_96_DPI, dpi);
                (
                    scale(DROPDOWN_WIDTH_AT_96_DPI, dpi).min(room).max(0),
                    scale(CONTROL_HEIGHT_AT_96_DPI, dpi),
                )
            }
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
            Control::Check => (
                scale(TOGGLE_WIDTH_AT_96_DPI, dpi),
                scale(TOGGLE_HEIGHT_AT_96_DPI, dpi),
            ),
        };
        let top = row_rect.top + (row_rect.bottom - row_rect.top - height) / 2;
        let right = row_rect.right - scale(CARD_PADDING_AT_96_DPI, dpi);
        RECT {
            left: right - width,
            top,
            right,
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

    /// What client point `x`, `y` is on at `scroll`. A toggle row is hit anywhere on its card,
    /// label included. Other rows are hit only on their control.
    pub(crate) fn hit(
        &self,
        x: i32,
        y: i32,
        scroll: i32,
        view: &SettingsView,
        page: Page,
    ) -> Option<Hit> {
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
        if let Some(index) = self.nav_items.iter().position(&inside) {
            return Some(Hit::Nav(Page::ALL[index]));
        }
        if page != Page::General || !inside(&self.body) {
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

/// The round knob of a toggle switch whose track is `track`: inset on every side, at the right
/// when `on`, at the left when off.
pub(crate) fn toggle_knob(track: RECT, on: bool, dpi: u32) -> RECT {
    let inset = scale(KNOB_INSET_AT_96_DPI, dpi);
    let size = (track.bottom - track.top - 2 * inset).max(0);
    let left = if on {
        track.right - inset - size
    } else {
        track.left + inset
    };
    RECT {
        left,
        top: track.top + inset,
        right: left + size,
        bottom: track.top + inset + size,
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
    let title_font = create_ui_font(scale(18, dpi), "Segoe UI", FW_SEMIBOLD as i32, false);
    let heading_font = create_ui_font(scale(14, dpi), "Segoe UI", FW_SEMIBOLD as i32, false);
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
    let (max_width, max_height) = if work.bottom > work.top && work.right > work.left {
        (work.right - work.left, work.bottom - work.top)
    } else {
        (i32::MAX, i32::MAX)
    };
    let layout = Layout::calculate(
        dpi,
        max_width,
        max_height,
        measure(dialog, link_font, EDIT_INI_LABEL),
    );
    // Built here, as the dialog opens: nothing of the shortcuts page runs before.
    let page_layout = super::shortcuts_page::PageLayout::calculate(layout.body, dpi);
    let mut shortcuts = super::shortcuts_model::ShortcutsModel::new(
        super::main_window::keymap(owner),
        page_layout.visible_rows(),
    );
    shortcuts.lines = super::main_window::key_line_commands(owner);
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
        model: DialogModel::new(page),
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
            let action = state(hwnd)
                .and_then(|dialog| dropdown_step(row, forward, &dialog.view, &dialog.fonts));
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
        ListStyle {
            row_height: dialog.layout.list_row_height(),
            radius: dialog.layout.radius(),
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
        dropdown_action(row, index, &dialog.fonts)
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

/// A click released on the shortcuts page.
fn page_click(hwnd: HWND, hit: super::shortcuts_page::PageHit) {
    use super::shortcuts_model::ShortcutsEffect;
    use super::shortcuts_page::PageHit;
    let now = unsafe { GetMessageTime() } as u32;
    let double_click_time = unsafe { GetDoubleClickTime() };
    let effect = state(hwnd).map(|dialog| {
        let model = &mut dialog.shortcuts;
        match hit {
            PageHit::RecordToggle => model.toggle_record_keys(),
            PageHit::Row(index) => {
                let double = dialog.last_row_click.is_some_and(|(time, row)| {
                    row == index && now.wrapping_sub(time) <= double_click_time
                });
                dialog.last_row_click = (!double).then_some((now, index));
                model.select(index);
                if double {
                    model.start_change()
                } else {
                    ShortcutsEffect::Repaint
                }
            }
            PageHit::Pencil(index) => {
                model.select(index);
                model.start_change()
            }
            PageHit::ConflictLink => model.follow_conflicts(),
            PageHit::RecordBox => ShortcutsEffect::None,
            PageHit::OutsideRecordBox => model.cancel_recording(),
        }
    });
    if let Some(effect) = effect {
        run_shortcuts(hwnd, effect);
    }
    // The toggle, and the conflict link (which turns record-keys on), hand the field the keys.
    let record_keys = state(hwnd).is_some_and(|dialog| dialog.shortcuts.record_keys);
    if hit == PageHit::RecordToggle || (hit == PageHit::ConflictLink && record_keys) {
        run_shortcuts(hwnd, ShortcutsEffect::FocusSearch);
    }
}

/// Carries out a shortcuts page effect. As in `run`, no `Dialog` borrow may be alive: applying
/// keys runs main window code.
pub(crate) fn run_shortcuts(hwnd: HWND, effect: super::shortcuts_model::ShortcutsEffect) {
    use super::shortcuts_model::ShortcutsEffect;
    match effect {
        ShortcutsEffect::None => {}
        ShortcutsEffect::Repaint => invalidate(hwnd),
        ShortcutsEffect::SetKeys(command, keys) => {
            super::main_window::set_command_keys(owner(hwnd), command, keys);
            after_keymap_change(hwnd);
        }
        ShortcutsEffect::Reset(command) => {
            super::main_window::reset_command_keys(owner(hwnd), command);
            after_keymap_change(hwnd);
        }
        ShortcutsEffect::CopyId(id) => {
            if let Err(error) = crate::platform::clipboard::set_text(hwnd, id) {
                super::main_window::push_notice(
                    owner(hwnd),
                    format!("FastPad could not copy to the clipboard: {error}"),
                );
            }
        }
        ShortcutsEffect::FocusSearch | ShortcutsEffect::FocusTable => {
            let focus = if effect == ShortcutsEffect::FocusSearch {
                Focus::Search
            } else {
                Focus::Table
            };
            let repaint = state(hwnd).map(|dialog| dialog.model.set_focus(focus, &dialog.view));
            if let Some(repaint) = repaint {
                run(hwnd, repaint);
            }
        }
        ShortcutsEffect::SetSearchText(text) => {
            let search = state(hwnd).and_then(|dialog| dialog.search);
            if let Some(search) = search {
                unsafe {
                    SetWindowTextW(search, wide_null(&text).as_ptr());
                    // The cue follows the mode even when the text stays empty.
                    InvalidateRect(search, std::ptr::null(), 1);
                }
            }
            invalidate(hwnd);
        }
        ShortcutsEffect::Close => close(hwnd),
    }
}

const CHANGE: usize = 1;
const ADD: usize = 2;
const REMOVE: usize = 3;
const RESET: usize = 4;
const COPY_ID: usize = 5;

/// The selected row's menu (keyboard shortcuts spec §6.4), at client point `at`.
fn context_menu(hwnd: HWND, at: POINT) {
    let Some((user, has_key)) = state(hwnd).and_then(|dialog| {
        let shortcuts = &dialog.shortcuts;
        shortcuts
            .selected_row()
            .map(|row| (shortcuts.can_reset(), row.stroke.is_some()))
    }) else {
        return;
    };
    let mut items = vec![
        ("Change Keybinding\tEnter".to_owned(), CHANGE),
        ("Add Keybinding\tCtrl+Enter".to_owned(), ADD),
    ];
    if has_key {
        items.push(("Remove Keybinding\tDelete".to_owned(), REMOVE));
    }
    if user {
        items.push(("Reset Keybinding".to_owned(), RESET));
    }
    items.push((String::new(), 0));
    items.push(("Copy Command ID\tCtrl+C".to_owned(), COPY_ID));
    let choice = super::menus::track_choice(owner(hwnd), hwnd, &items, at);
    let effect = state(hwnd).map(|dialog| {
        let model = &mut dialog.shortcuts;
        match choice {
            Some(CHANGE) => model.start_change(),
            Some(ADD) => model.start_add(),
            Some(REMOVE) => model.remove(),
            Some(RESET) => model.reset(),
            Some(COPY_ID) => model.copy_id(),
            _ => super::shortcuts_model::ShortcutsEffect::None,
        }
    });
    if let Some(effect) = effect {
        run_shortcuts(hwnd, effect);
    }
}

/// Shift+F10 or the context-menu key (`WM_CONTEXTMENU` with no point): the selected row's
/// menu, under the row, when the table has the focus and no recording box is open.
fn keyboard_context_menu(hwnd: HWND) {
    let at = state(hwnd).and_then(|dialog| {
        if dialog.model.page != Page::Shortcuts
            || dialog.model.focus != Focus::Table
            || dialog.shortcuts.recording.is_some()
        {
            return None;
        }
        let selected = dialog.shortcuts.selected;
        dialog.shortcuts.selected_row()?;
        // Scrolled away, the row comes back into view first.
        dialog.shortcuts.select(selected);
        let rect = dialog.page_layout.row_rect(selected - dialog.shortcuts.top);
        Some(POINT {
            x: rect.left + (rect.bottom - rect.top) / 2,
            y: rect.bottom,
        })
    });
    if let Some(at) = at {
        invalidate(hwnd);
        context_menu(hwnd, at);
    }
}

/// Alt+K from outside the search field: toggles record-keys search, and when that turns it on
/// the field takes the focus so the next stroke is recorded there.
fn toggle_record_keys(hwnd: HWND) {
    use super::shortcuts_model::ShortcutsEffect;
    let Some(effect) = state(hwnd).map(|dialog| dialog.shortcuts.toggle_record_keys()) else {
        return;
    };
    run_shortcuts(hwnd, effect);
    if state(hwnd).is_some_and(|dialog| dialog.shortcuts.record_keys) {
        run_shortcuts(hwnd, ShortcutsEffect::FocusSearch);
    }
}

/// Re-reads the keymap after a change applied, keeping the selection.
fn after_keymap_change(hwnd: HWND) {
    let keymap = super::main_window::keymap(owner(hwnd));
    let lines = super::main_window::key_line_commands(owner(hwnd));
    if let Some(dialog) = state(hwnd) {
        dialog.shortcuts.refresh(keymap, lines);
    }
    if unsafe { GetFocus() } != hwnd {
        unsafe { SetFocus(hwnd) };
    }
    invalidate(hwnd);
}

/// The stroke `virtual_key` makes with the modifiers held now.
fn current_stroke(virtual_key: u16) -> Option<KeyStroke> {
    let held = |key: u16| unsafe { GetKeyState(i32::from(key)) } < 0;
    KeyStroke::from_key(virtual_key, held(VK_CONTROL), held(VK_SHIFT), held(VK_MENU))
}

/// What releasing the mouse on `hit` does.
fn click_effect(dialog: &mut Dialog, hit: Hit) -> Effect {
    let view = &dialog.view;
    match hit {
        Hit::Close | Hit::TitleClose => Effect::Close,
        Hit::EditIni => Effect::EditIni,
        Hit::Nav(page) => Effect::ShowPage(page),
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
        // Page clicks go through `page_click`.
        Hit::Page(_) => Effect::None,
    }
}

/// The focus a click on `hit` moves to.
fn focus_of(hit: Hit) -> Focus {
    match hit {
        Hit::Row(row, _) => Focus::Row(row),
        Hit::Nav(_) => Focus::Nav,
        Hit::EditIni => Focus::EditIni,
        Hit::Close | Hit::TitleClose => Focus::Close,
        Hit::Page(super::shortcuts_page::PageHit::RecordToggle) => Focus::Search,
        Hit::Page(_) => Focus::Table,
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

/// A key on the shortcuts page: the recording box takes every key, Alt+K toggles record-keys
/// search, and the focused table takes its keys. False when the key is not the page's: Shift+F10
/// then reaches DefWindowProc, which sends `WM_CONTEXTMENU` for the row menu.
fn shortcuts_key(hwnd: HWND, virtual_key: u16) -> bool {
    let stroke = current_stroke(virtual_key);
    let idle = state(hwnd).is_some_and(|dialog| {
        dialog.model.page == Page::Shortcuts && dialog.shortcuts.recording.is_none()
    });
    if idle && stroke == Some(KeyStroke::new(false, false, true, u16::from(b'K'))) {
        toggle_record_keys(hwnd);
        return true;
    }
    let routed = state(hwnd).and_then(|dialog| {
        if dialog.model.page != Page::Shortcuts {
            return None;
        }
        if dialog.shortcuts.recording.is_some() {
            // Every key goes to the box; a modifier alone records nothing.
            return Some(
                stroke.map_or(super::shortcuts_model::ShortcutsEffect::None, |stroke| {
                    dialog.shortcuts.record_key(stroke)
                }),
            );
        }
        let stroke = stroke?;
        if dialog.model.focus != Focus::Table {
            return None;
        }
        let effect = dialog.shortcuts.table_key(stroke);
        (effect != super::shortcuts_model::ShortcutsEffect::None).then_some(effect)
    });
    let Some(effect) = routed else {
        return false;
    };
    run_shortcuts(hwnd, effect);
    true
}

/// A key pressed in the search field; true when the dialog took it (keyboard shortcuts spec
/// §6.2).
pub(crate) fn search_key(dialog: HWND, virtual_key: u16) -> bool {
    use super::shortcuts_model::ShortcutsEffect;
    let stroke = current_stroke(virtual_key);
    let held = |key: u16| unsafe { GetKeyState(i32::from(key)) } < 0;
    let Some(record_keys) = state(dialog).map(|d| d.shortcuts.record_keys) else {
        return false;
    };
    let plain = |key: u16| stroke == Some(KeyStroke::new(false, false, false, key));
    let effect = if virtual_key == VK_TAB && !held(VK_CONTROL) && !held(VK_MENU) {
        let effect = state(dialog).map(|d| {
            d.model.key(
                Key::Tab {
                    back: held(VK_SHIFT),
                },
                &d.view,
            )
        });
        if let Some(effect) = effect {
            run(dialog, effect);
        }
        return true;
    } else if held(VK_CONTROL) && matches!(virtual_key, VK_PRIOR | VK_NEXT) {
        let effect = state(dialog).map(|d| {
            d.model.key(
                Key::NextPage {
                    back: virtual_key == VK_PRIOR,
                },
                &d.view,
            )
        });
        if let Some(effect) = effect {
            run(dialog, effect);
        }
        return true;
    } else if stroke == Some(KeyStroke::new(false, false, true, u16::from(b'K')))
        || (record_keys && plain(VK_ESCAPE))
    {
        state(dialog).map(|d| d.shortcuts.toggle_record_keys())
    } else if record_keys {
        // Every stroke becomes the filter; a modifier alone waits for its key.
        match stroke {
            Some(stroke) => state(dialog).map(|d| d.shortcuts.record_search_key(stroke)),
            None => return true,
        }
    } else if plain(VK_DOWN) || plain(VK_RETURN) {
        // Into the table on its first row, whatever was selected before the filter changed.
        state(dialog).map(|d| {
            d.shortcuts.select(0);
            ShortcutsEffect::FocusTable
        })
    } else if plain(VK_ESCAPE) {
        Some(ShortcutsEffect::Close)
    } else {
        return false;
    };
    if let Some(effect) = effect {
        run_shortcuts(dialog, effect);
    }
    true
}

/// Whether the field must not see a character: every one in record-keys mode (so a recorded
/// `S` never brings its `s` along), the Tab, Enter and Escape characters it would beep at, and
/// Alt+letters.
pub(crate) fn search_swallows_char(dialog: HWND, c: u32, sys: bool) -> bool {
    sys || matches!(c, 0x09 | 0x0d | 0x1b) || state(dialog).is_some_and(|d| d.shortcuts.record_keys)
}

/// The field took the focus (a click on it): the model follows. The field is outside the
/// recording box, so taking the focus while the box is open (it never asks for it) cancels
/// the box, as any click outside it does.
pub(crate) fn search_focused(dialog: HWND) {
    if let Some(d) = state(dialog) {
        d.shortcuts.cancel_recording();
        d.model.focus = Focus::Search;
    }
    invalidate(dialog);
}

/// The empty field's cue, in the muted colour.
pub(crate) fn paint_search_cue(dialog: HWND, edit: HWND) {
    use windows_sys::Win32::Graphics::Gdi::{
        BeginPaint, EndPaint, FillRect, PAINTSTRUCT, SetBkMode, TRANSPARENT,
    };
    let Some((brush, color, font, cue)) = state(dialog).map(|d| {
        let cue = if d.shortcuts.record_keys {
            "Press keys to search"
        } else {
            "Type to search in keybindings"
        };
        (d.search_brush, d.colors.muted_foreground, d.body_font, cue)
    }) else {
        return;
    };
    let mut paint = PAINTSTRUCT::default();
    unsafe {
        let dc = BeginPaint(edit, &mut paint);
        let mut client = RECT::default();
        windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect(edit, &mut client);
        FillRect(dc, &client, brush);
        let previous = SelectObject(dc, font as _);
        SetBkMode(dc, TRANSPARENT as i32);
        SetTextColor(dc, color);
        let mut text = wide_null(cue);
        DrawTextW(
            dc,
            text.as_mut_ptr(),
            -1,
            &mut client,
            DT_LEFT | DT_SINGLELINE | DT_NOPREFIX,
        );
        SelectObject(dc, previous);
        EndPaint(edit, &paint);
    }
}

/// `hwnd`'s text.
fn window_text(hwnd: HWND) -> String {
    let length = unsafe { GetWindowTextLengthW(hwnd) }.max(0) as usize;
    let mut buffer = vec![0u16; length + 1];
    let copied = unsafe { GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32) };
    String::from_utf16_lossy(&buffer[..copied.max(0) as usize])
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
    if shortcuts_key(hwnd, virtual_key) {
        return;
    }
    let ctrl = unsafe { GetKeyState(i32::from(VK_CONTROL)) } < 0;
    let key = if ctrl && matches!(virtual_key, VK_PRIOR | VK_NEXT) {
        Some(Key::NextPage {
            back: virtual_key == VK_PRIOR,
        })
    } else {
        model_key(virtual_key)
    };
    let Some(key) = key else {
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
        // On the shortcuts page Alt+K toggles record-keys search, and Alt combinations are
        // strokes for the recording box and the table; the rest (Alt+F4) keep their defaults.
        WM_SYSKEYDOWN if state(hwnd).is_some_and(|dialog| dialog.model.page == Page::Shortcuts) => {
            if shortcuts_key(hwnd, wparam as u16) {
                0
            } else {
                unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
            }
        }
        // No menu to open and no beep for Alt+letters on the shortcuts page.
        windows_sys::Win32::UI::WindowsAndMessaging::WM_SYSCHAR
            if state(hwnd).is_some_and(|dialog| dialog.model.page == Page::Shortcuts) =>
        {
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
                return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
            }
            // DefWindowProcW focuses the dialog itself; the search field takes it back.
            let result = unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
            sync_focus(hwnd);
            result
        }
        windows_sys::Win32::UI::WindowsAndMessaging::WM_CTLCOLOREDIT => {
            let Some((foreground, background, brush)) = state(hwnd).map(|dialog| {
                (
                    dialog.colors.editor_foreground,
                    Tones::new(&dialog.colors).control,
                    dialog.search_brush,
                )
            }) else {
                return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
            };
            let dc = wparam as HDC;
            unsafe {
                SetTextColor(dc, foreground);
                SetBkColor(dc, background);
            }
            brush as LRESULT
        }
        windows_sys::Win32::UI::WindowsAndMessaging::WM_COMMAND
            if (wparam & 0xffff) == super::shortcuts_page::SEARCH_CONTROL_ID
                && ((wparam >> 16) & 0xffff) as u32
                    == windows_sys::Win32::UI::WindowsAndMessaging::EN_CHANGE =>
        {
            let search = state(hwnd).and_then(|dialog| dialog.search);
            let text = search.map(window_text).unwrap_or_default();
            // In record-keys mode the dialog writes the field itself.
            let effect = state(hwnd).and_then(|dialog| {
                (!dialog.shortcuts.record_keys).then(|| dialog.shortcuts.set_text(&text))
            });
            if let Some(effect) = effect {
                run_shortcuts(hwnd, effect);
            }
            0
        }
        #[cfg(test)]
        WM_TEST_ANSWER => {
            if let Some(answer) = IN_LOOP.with(|answers| answers.borrow_mut().pop_front()) {
                answer(hwnd);
            }
            0
        }
        WM_MOUSEWHEEL => {
            let delta = ((wparam >> 16) & 0xffff) as i16;
            if let Some(dialog) = state(hwnd) {
                if let Some((_, list)) = &dialog.list {
                    list.wheel(delta);
                } else if dialog.model.page == Page::Shortcuts {
                    // Three rows a notch, small touchpad deltas adding up; General's scroll
                    // stays where it was.
                    let delta = i32::from(delta);
                    if dialog.wheel_rest.signum() == -delta.signum() {
                        dialog.wheel_rest = 0;
                    }
                    dialog.wheel_rest += delta * 3;
                    let rows = dialog.wheel_rest / 120;
                    dialog.wheel_rest -= rows * 120;
                    let top = dialog.shortcuts.top;
                    dialog.shortcuts.scroll(-rows as isize);
                    if dialog.shortcuts.top != top {
                        invalidate(hwnd);
                    }
                } else {
                    let pitch = dialog.layout.row_pitch();
                    let scroll = (dialog.scroll - i32::from(delta) * pitch / 40)
                        .clamp(0, dialog.layout.max_scroll());
                    if scroll != dialog.scroll {
                        dialog.scroll = scroll;
                        invalidate(hwnd);
                    }
                }
            }
            0
        }
        // The native frame stays hidden: the client is the whole window. Both forms leave the
        // proposed rect as it is.
        WM_NCCALCSIZE => 0,
        // A press on the title row is a non-client click that starts the move loop without a
        // WM_LBUTTONDOWN or a deactivation: close the list here, as any click elsewhere does.
        WM_NCLBUTTONDOWN => {
            close_list(hwnd);
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
        // The edges size the dialog (the native frame that would is hidden), and only the title
        // row drags it.
        WM_NCHITTEST => {
            let (x, y) = lparam_point(lparam);
            let mut point = POINT { x, y };
            unsafe { ScreenToClient(hwnd, &mut point) };
            let Some(layout) = state(hwnd).map(|dialog| dialog.layout) else {
                return HTCLIENT as LRESULT;
            };
            if unsafe { IsZoomed(hwnd) } == 0 {
                let border = unsafe {
                    GetSystemMetricsForDpi(SM_CXSIZEFRAME, layout.dpi)
                        + GetSystemMetricsForDpi(SM_CXPADDEDBORDER, layout.dpi)
                };
                if let Some(edge) =
                    sizing_edge(point.x, point.y, layout.width, layout.height, border)
                {
                    return edge as LRESULT;
                }
            }
            if point.y < layout.title.bottom && point.x < layout.title_close.left {
                HTCAPTION as LRESULT
            } else {
                HTCLIENT as LRESULT
            }
        }
        WM_SIZE => {
            let (width, height) = lparam_point(lparam);
            if width > 0 && height > 0 {
                resized(hwnd, width, height);
            }
            0
        }
        // No smaller than a usable minimum, unless the work area is (the dialog opens smaller
        // there); maximized, the work area, not the whole monitor (a popup would cover the
        // taskbar).
        WM_GETMINMAXINFO => {
            let dpi = state(hwnd).map_or(96, |dialog| dialog.layout.dpi);
            unsafe { super::titlebar::constrain_maximized_window(hwnd, lparam) };
            let (width, height) = Layout::min_size(dpi);
            let minmax = unsafe { &mut *(lparam as *mut MINMAXINFO) };
            minmax.ptMinTrackSize = POINT {
                x: width.min(minmax.ptMaxSize.x),
                y: height.min(minmax.ptMaxSize.y),
            };
            0
        }
        WM_SETCURSOR => {
            let mut point = POINT::default();
            unsafe {
                GetCursorPos(&mut point);
                ScreenToClient(hwnd, &mut point);
            }
            let on_link = state(hwnd).is_some_and(|dialog| {
                matches!(
                    hit_at(dialog, point.x, point.y),
                    Some(Hit::EditIni | Hit::Page(super::shortcuts_page::PageHit::ConflictLink))
                )
            });
            let cursor = if on_link { IDC_HAND } else { IDC_ARROW };
            unsafe { SetCursor(LoadCursorW(std::ptr::null_mut(), cursor) as HCURSOR) };
            1
        }
        WM_MOUSEMOVE => {
            let (x, y) = lparam_point(lparam);
            if let Some(dialog) = state(hwnd) {
                let hot = hit_at(dialog, x, y);
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
                let hit = hit_at(dialog, x, y)?;
                // A click on the dropdown whose list was open only closes that list.
                dialog.pressed =
                    (open_row.map(|row| Hit::Row(row, Part::Whole)) != Some(hit)).then_some(hit);
                // A greyed row takes no focus: it stays where it was; nor does the recording
                // box, which keeps the table's.
                Some(match hit {
                    Hit::Row(row, _) if !dialog.view.enabled(row) => Effect::None,
                    Hit::Page(
                        super::shortcuts_page::PageHit::RecordBox
                        | super::shortcuts_page::PageHit::OutsideRecordBox
                        | super::shortcuts_page::PageHit::ConflictLink,
                    ) => Effect::None,
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
            let released = state(hwnd).and_then(|dialog| {
                let pressed = dialog.pressed.take()?;
                (hit_at(dialog, x, y) == Some(pressed)).then_some(pressed)
            });
            invalidate(hwnd);
            match released {
                Some(Hit::Page(hit)) => page_click(hwnd, hit),
                Some(hit) => {
                    let effect = state(hwnd).map(|dialog| click_effect(dialog, hit));
                    if let Some(effect) = effect {
                        run(hwnd, effect);
                    }
                }
                None => {}
            }
            0
        }
        // Shift+F10 and the context-menu key: a context menu with no point (-1), for the selected
        // row, as in the notebook tree. The recording box keeps both keys (Shift+F10 is refused
        // there), so none comes while it is open.
        windows_sys::Win32::UI::WindowsAndMessaging::WM_CONTEXTMENU
            if lparam as u32 == u32::MAX =>
        {
            keyboard_context_menu(hwnd);
            0
        }
        // A right-click on a row selects it and opens its menu; while the recording box is open
        // no row is hit.
        windows_sys::Win32::UI::WindowsAndMessaging::WM_RBUTTONUP => {
            use super::shortcuts_page::PageHit;
            let (x, y) = lparam_point(lparam);
            let row = state(hwnd).and_then(|dialog| match hit_at(dialog, x, y) {
                Some(Hit::Page(PageHit::Row(index) | PageHit::Pencil(index))) => {
                    dialog.shortcuts.select(index);
                    dialog.model.focus = Focus::Table;
                    Some(index)
                }
                _ => None,
            });
            if row.is_some() {
                sync_focus(hwnd);
                invalidate(hwnd);
                context_menu(hwnd, POINT { x, y });
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
    let measure = |text: &str| text_width(dc, dialog.body_font, text);
    let mut frame = Frame::default();
    compose(&mut frame, client, dialog, &measure);
    frame.paint(dc, client, &dialog.canvas);
}

/// `text`'s width in `font` on `dc`.
fn text_width(dc: HDC, font: HFONT, text: &str) -> i32 {
    use windows_sys::Win32::Foundation::SIZE;
    use windows_sys::Win32::Graphics::Gdi::GetTextExtentPoint32W;
    let wide = text.encode_utf16().collect::<Vec<_>>();
    let mut size = SIZE::default();
    unsafe {
        let previous = SelectObject(dc, font as _);
        GetTextExtentPoint32W(dc, wide.as_ptr(), wide.len() as i32, &mut size);
        SelectObject(dc, previous);
    }
    size.cx
}

/// Everything the dialog paints. `measure` gives a text's width in the body font.
fn compose<'a>(
    frame: &mut Frame<'a>,
    client: RECT,
    dialog: &'a Dialog,
    measure: &dyn Fn(&str) -> i32,
) {
    let colors = &dialog.colors;
    let tones = Tones::new(colors);
    let layout = &dialog.layout;
    let view = &dialog.view;
    let dpi = layout.dpi;
    let radius = layout.radius();

    // No border line: the native shadow is the edge (square on Windows 10, rounded on 11).
    frame.shape(Shape::Fill {
        rect: client,
        color: colors.panel_background(),
    });

    // Title row: a strip-coloured band with the name and the × button.
    frame.shape(Shape::Fill {
        rect: RECT {
            bottom: layout.title.bottom,
            ..client
        },
        color: colors.strip_background,
    });
    frame.text(
        dialog.title_font,
        colors.editor_foreground,
        TITLE,
        layout.title,
        DT_LEFT,
    );
    title_close(
        frame,
        colors,
        dialog.glyph_font,
        layout.title_close,
        dialog.hot == Some(Hit::TitleClose),
    );
    // Subtle rules under the title and over the footer.
    for top in [layout.body.top - 1, layout.body.bottom] {
        frame.shape(Shape::Fill {
            rect: RECT {
                top,
                bottom: top + 1,
                ..client
            },
            color: tones.card,
        });
    }

    // The page list on the left.
    frame.shape(Shape::Fill {
        rect: layout.nav,
        color: colors.strip_background,
    });
    for page in Page::ALL {
        let item = layout.nav_items[page as usize];
        let current = dialog.model.page == page;
        let hot = dialog.hot == Some(Hit::Nav(page));
        if current || hot {
            let fill = if current {
                tones.control
            } else {
                tones.card_hot
            };
            tones.soft(frame, item, radius, fill);
        }
        if current {
            let bar = scale(NAV_BAR_WIDTH_AT_96_DPI, dpi);
            let middle = (item.top + item.bottom) / 2;
            frame.shape(Shape::Round {
                rect: RECT {
                    left: item.left,
                    top: middle - scale(8, dpi),
                    right: item.left + bar,
                    bottom: middle + scale(8, dpi),
                },
                radius: bar / 2,
                color: tones.accent,
            });
        }
        frame.text(
            dialog.body_font,
            colors.editor_foreground,
            page.title(),
            RECT {
                left: item.left + scale(12, dpi),
                ..item
            },
            DT_LEFT,
        );
    }

    // The scrolling body, clipped to its area.
    frame.clip(Some(layout.body));
    if dialog.model.page == Page::General {
        for section in Section::ALL {
            let heading = layout.heading_rect(section, dialog.scroll);
            frame.text(
                dialog.heading_font,
                colors.editor_foreground,
                section.title(),
                RECT {
                    top: heading.top + scale(HEADING_SPACE_ABOVE_AT_96_DPI, dpi),
                    ..heading
                },
                DT_LEFT,
            );
        }
        for row in Row::ALL {
            compose_row(frame, &tones, dialog, row);
        }
    }
    frame.clip(None);
    if dialog.model.page == Page::Shortcuts {
        let style = super::shortcuts_page::PageStyle {
            colors,
            tones: &tones,
            link_color: dialog.link_color,
            body_font: dialog.body_font,
            heading_font: dialog.heading_font,
            link_font: dialog.link_font,
            glyph_font: dialog.glyph_font,
            radius,
            hot: match dialog.hot {
                Some(Hit::Page(hit)) => Some(hit),
                _ => None,
            },
            table_focused: dialog.model.focus == Focus::Table,
        };
        super::shortcuts_page::compose(
            frame,
            measure,
            &dialog.page_layout,
            &dialog.shortcuts,
            &style,
        );
    }

    // Footer: the link and a filled accent Close.
    frame.text(
        dialog.link_font,
        dialog.link_color,
        EDIT_INI_LABEL,
        layout.edit_ini,
        DT_LEFT,
    );
    let button = match (dialog.pressed, dialog.hot) {
        (Some(Hit::Close), _) => tones.accent_down,
        (_, Some(Hit::Close)) => tones.accent_hot,
        _ => tones.accent,
    };
    tones.soft(frame, layout.close, radius, button);
    frame.text(
        dialog.body_font,
        tones.on_accent,
        CLOSE_LABEL,
        layout.close,
        DT_CENTER,
    );

    // The focus ring: a rounded accent stroke just outside the focused control, or on the
    // edge of a toggle's card, all of which is its hit area.
    let width = scale(FOCUS_WIDTH_AT_96_DPI, dpi);
    let outside = width + scale(FOCUS_GAP_AT_96_DPI, dpi);
    let ring = match dialog.model.focus {
        Focus::Nav => Some((
            inset(layout.nav_items[dialog.model.page as usize], -outside),
            radius + outside,
        )),
        Focus::Search => Some((inset(dialog.page_layout.search, -outside), radius + outside)),
        // The selected row's accent bar shows the table's focus.
        Focus::Table => None,
        Focus::EditIni => Some((inset(layout.edit_ini, -outside), radius + outside)),
        Focus::Close => Some((inset(layout.close, -outside), radius + outside)),
        Focus::Row(row) => {
            let card = layout.row_rect(row, dialog.scroll);
            let visible = card.top >= layout.body.top && card.bottom <= layout.body.bottom;
            visible.then(|| {
                if row.control() == Control::Check {
                    (card, radius)
                } else {
                    let control = layout.control_rect(row, card, view.segments(row).len());
                    (inset(control, -outside), radius + outside)
                }
            })
        }
    };
    if let Some((rect, radius)) = ring {
        frame.shape(Shape::Ring {
            rect,
            radius,
            width,
            color: tones.accent,
        });
    }
}

fn compose_row<'a>(frame: &mut Frame<'a>, tones: &Tones, dialog: &'a Dialog, row: Row) {
    let colors = &dialog.colors;
    let layout = &dialog.layout;
    let view = &dialog.view;
    let dpi = layout.dpi;
    let radius = layout.radius();
    let card = layout.row_rect(row, dialog.scroll);
    if card.bottom < layout.body.top || card.top > layout.body.bottom {
        return;
    }
    let enabled = view.enabled(row);
    let text = if enabled {
        colors.editor_foreground
    } else {
        colors.muted_foreground
    };
    let segments = view.segments(row);
    let control = layout.control_rect(row, card, segments.len());
    let hot = matches!(dialog.hot, Some(Hit::Row(hot_row, _)) if hot_row == row) && enabled;
    // A toggle's whole card is its hit area, so the card shows the hover.
    let card_fill = if hot && row.control() == Control::Check {
        tones.card_hot
    } else {
        tones.card
    };
    tones.soft(frame, card, radius, card_fill);
    let padding = scale(CARD_PADDING_AT_96_DPI, dpi);
    let label = RECT {
        left: card.left + padding,
        right: card.right - padding,
        ..card
    };
    frame.text(dialog.body_font, text, row.label(), label, DT_LEFT);
    // Pills inside a control's track.
    let pill_inset = scale(2, dpi);
    let pill_radius = (radius - pill_inset).max(scale(2, dpi));
    match row.control() {
        Control::Check => {
            let on = row.toggle().is_some_and(|toggle| view.checked(toggle));
            let state = RECT {
                left: control.left
                    - scale(
                        TOGGLE_STATE_GAP_AT_96_DPI + TOGGLE_STATE_WIDTH_AT_96_DPI,
                        dpi,
                    ),
                right: control.left - scale(TOGGLE_STATE_GAP_AT_96_DPI, dpi),
                ..card
            };
            frame.text(
                dialog.body_font,
                text,
                if on { "On" } else { "Off" },
                state,
                DT_RIGHT,
            );
            if !enabled {
                let hint = RECT {
                    left: label.left,
                    right: state.left - scale(12, dpi),
                    ..card
                };
                frame.text(
                    dialog.body_font,
                    colors.muted_foreground,
                    AUTOSAVE_HINT,
                    hint,
                    DT_RIGHT,
                );
            }
            let track_radius = (control.bottom - control.top) / 2;
            let knob = toggle_knob(control, on, dpi);
            let (track, knob_color) = match (enabled, on) {
                (true, true) => (
                    Some(if hot { tones.accent_hot } else { tones.accent }),
                    tones.on_accent,
                ),
                (true, false) => (None, colors.muted_foreground),
                // Greyed: a pale track and knob.
                (false, true) => (Some(colors.pressed_background), tones.card),
                (false, false) => (None, colors.pressed_background),
            };
            match track {
                Some(color) => frame.shape(Shape::Round {
                    rect: control,
                    radius: track_radius,
                    color,
                }),
                // Off: a pill outlined on the card, not filled.
                None => frame.shape(Shape::Ring {
                    rect: control,
                    radius: track_radius,
                    width: scale(1, dpi),
                    color: if enabled {
                        colors.muted_foreground
                    } else {
                        colors.pressed_background
                    },
                }),
            }
            frame.shape(Shape::Round {
                rect: knob,
                radius: (knob.bottom - knob.top) / 2,
                color: knob_color,
            });
        }
        Control::Segmented => {
            tones.soft(frame, control, radius, tones.control);
            for (index, (segment, rect)) in segments
                .iter()
                .zip(layout.segment_rects(control, segments.len()))
                .enumerate()
            {
                let pill = inset(rect, pill_inset);
                let hot_segment = dialog.hot == Some(Hit::Row(row, Part::Segment(index)));
                let (fill, foreground) = if segment.selected {
                    (Some(tones.accent), tones.on_accent)
                } else if hot_segment {
                    (Some(tones.control_hot), colors.editor_foreground)
                } else {
                    (None, colors.editor_foreground)
                };
                if let Some(color) = fill {
                    frame.shape(Shape::Round {
                        rect: pill,
                        radius: pill_radius,
                        color,
                    });
                }
                frame.text(
                    dialog.body_font,
                    foreground,
                    segment.label.clone(),
                    rect,
                    DT_CENTER,
                );
            }
        }
        Control::Dropdown => {
            let open = matches!(&dialog.list, Some((open_row, _)) if *open_row == row);
            let fill = if hot || open {
                tones.control_hot
            } else {
                tones.control
            };
            tones.soft(frame, control, radius, fill);
            let chevron = RECT {
                left: control.right - scale(26, dpi),
                ..control
            };
            let value = RECT {
                left: control.left + scale(8, dpi),
                right: chevron.left,
                ..control
            };
            frame.text(
                dialog.body_font,
                colors.editor_foreground,
                view.dropdown_text(row),
                value,
                DT_LEFT,
            );
            frame.text(
                dialog.glyph_font,
                colors.muted_foreground,
                GLYPH_CHEVRON_DOWN,
                chevron,
                DT_CENTER,
            );
        }
        Control::Stepper => {
            let [minus, value, plus] = layout.stepper_rects(control);
            tones.soft(frame, control, radius, tones.control);
            for (rect, glyph, part) in [
                (minus, GLYPH_REMOVE, Part::Minus),
                (plus, GLYPH_ADD, Part::Plus),
            ] {
                let fill = match (dialog.pressed, dialog.hot) {
                    (Some(pressed), _) if pressed == Hit::Row(row, part) => {
                        Some(tones.control_down)
                    }
                    (_, Some(hot)) if hot == Hit::Row(row, part) => Some(tones.control_hot),
                    _ => None,
                };
                if let Some(color) = fill {
                    frame.shape(Shape::Round {
                        rect: inset(rect, pill_inset),
                        radius: pill_radius,
                        color,
                    });
                }
                frame.text(
                    dialog.glyph_font,
                    colors.editor_foreground,
                    glyph,
                    rect,
                    DT_CENTER,
                );
            }
            // The typed value's field.
            frame.shape(Shape::Round {
                rect: inset(value, pill_inset),
                radius: pill_radius,
                color: colors.editor_background,
            });
            frame.text(
                dialog.body_font,
                colors.editor_foreground,
                dialog.model.font_size_text(view),
                value,
                DT_CENTER,
            );
        }
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
    fn cards_stack_under_their_headings_with_gaps_and_everything_fits_at_96_dpi() {
        // Break caught: cards touching or overlapping, a heading painted under its first card,
        // cards flush with the dialog's edges, or a dialog over about 700 px tall at 96 DPI
        // (soft-look addendum).
        let layout = Layout::calculate(96, 4000, 2000, 100);
        assert_eq!(layout.width, 860);
        assert_eq!(layout.max_scroll(), 0);
        let mut expected_top = 0;
        for section in Section::ALL {
            assert_eq!(layout.headings[section as usize].top, expected_top);
            expected_top = layout.headings[section as usize].bottom;
            for row in Row::ALL.into_iter().filter(|row| row.section() == section) {
                let card = layout.rows[row as usize];
                assert_eq!(card.top, expected_top, "{row:?}");
                assert_eq!(card.bottom - card.top, 36, "{row:?}");
                assert_eq!((card.left, card.right), (200, 840), "{row:?}");
                expected_top = card.bottom + 3;
            }
        }
        assert_eq!(layout.content_height, expected_top);
        assert_eq!(layout.height, 44 + layout.content_height + 56);
        assert!(layout.height <= 700, "{}", layout.height);
        assert!(layout.edit_ini.right <= layout.close.left);
    }

    #[test]
    fn the_nav_sits_left_of_the_cards_and_hits_its_pages() {
        // Break caught: cards painted under the nav, or a nav item that doesn't switch pages.
        let layout = Layout::calculate(96, 4000, 2000, 100);
        assert!(layout.nav.right <= layout.rows[0].left);
        assert_eq!(layout.body.left, layout.nav.right);
        for page in Page::ALL {
            let (x, y) = center(layout.nav_items[page as usize]);
            assert_eq!(
                layout.hit(x, y, 0, &view(), Page::General),
                Some(Hit::Nav(page))
            );
        }
        let (x, y) = center(layout.row_rect(Row::Theme, 0));
        assert_eq!(
            layout.hit(x, y, 0, &view(), Page::Shortcuts),
            None,
            "no General rows on the other page"
        );
    }

    #[test]
    fn hits_find_controls_checkbox_labels_segments_and_stepper_parts() {
        let layout = Layout::calculate(96, 4000, 2000, 100);
        let view = view();
        let hit = |rect: RECT| {
            let (x, y) = center(rect);
            layout.hit(x, y, 0, &view, Page::General)
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

        let gap = RECT {
            top: row(Row::WordWrap).bottom,
            bottom: row(Row::LineNumbers).top,
            ..row(Row::WordWrap)
        };
        assert_eq!(hit(gap), None, "the gap between two cards");

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
    fn a_sized_layout_fills_the_size_it_is_given() {
        // Break caught: a resized dialog painting its footer, cards or Close button where the
        // old size put them.
        let opened = Layout::calculate(96, 1920, 1080, 90);
        let sized = Layout::sized(96, 1000, 900, 90);
        assert_eq!((sized.width, sized.height), (1000, 900));
        assert_eq!(sized.body.bottom, 900 - 56);
        assert_eq!(sized.close.right, 1000 - 20);
        assert_eq!(sized.rows[0].right, 1000 - 20);
        assert_eq!(sized.title_close.right, 1000);
        // Taller than everything: nothing to scroll.
        assert_eq!(sized.max_scroll(), 0);
        // The opening size is a sized layout too.
        let again = Layout::sized(96, opened.width, opened.height, 90);
        let edges = |rect: RECT| (rect.left, rect.top, rect.right, rect.bottom);
        assert_eq!(edges(again.body), edges(opened.body));
        assert_eq!(edges(again.close), edges(opened.close));
    }

    #[test]
    fn the_edges_size_the_dialog_and_the_corners_size_both_ways() {
        // Break caught: a dialog that can't be sized because its hidden frame takes no drags, or
        // an edge band so wide it eats clicks meant for the nav or the cards.
        let edge = |x, y| sizing_edge(x, y, 800, 600, 8);
        assert_eq!(edge(2, 300), Some(HTLEFT));
        assert_eq!(edge(797, 300), Some(HTRIGHT));
        assert_eq!(edge(400, 3), Some(HTTOP));
        assert_eq!(edge(400, 595), Some(HTBOTTOM));
        assert_eq!(edge(1, 1), Some(HTTOPLEFT));
        assert_eq!(edge(799, 0), Some(HTTOPRIGHT));
        assert_eq!(edge(0, 599), Some(HTBOTTOMLEFT));
        assert_eq!(edge(799, 599), Some(HTBOTTOMRIGHT));
        assert_eq!(edge(8, 300), None);
        assert_eq!(edge(400, 300), None);
    }

    #[test]
    fn the_layout_scales_with_dpi() {
        let normal = Layout::calculate(96, 4000, 4000, 100);
        let double = Layout::calculate(192, 4000, 4000, 200);
        assert_eq!(double.width, normal.width * 2);
        assert_eq!(double.content_height, normal.content_height * 2);
        assert_eq!(double.rows[5].top, normal.rows[5].top * 2);
        assert_eq!(
            double.title_close.right - double.title_close.left,
            (normal.title_close.right - normal.title_close.left) * 2
        );
    }

    #[test]
    fn the_title_close_button_fills_the_title_row_corner_like_the_caption_close() {
        // Break caught: a × inset from the corner or shorter than the title row, unlike the
        // main window's caption close button, or a title that runs under it.
        let layout = Layout::calculate(96, 4000, 2000, 100);
        let close = layout.title_close;
        assert_eq!(
            (close.left, close.top, close.right, close.bottom),
            (860 - 46, 0, 860, 44)
        );
        assert_eq!(layout.title.bottom, close.bottom);
        assert_eq!(layout.title.right, close.left);
        assert_eq!(layout.body.top, close.bottom);
    }

    #[test]
    fn a_toggle_row_shows_a_40_by_20_switch_at_its_right_end() {
        // Break caught: the switch drawn in the 18-px square the checkbox had, squashed into a
        // square, off-centre in its row, or not DPI-scaled.
        for dpi in [96, 144, 192] {
            let layout = Layout::calculate(dpi, 4000, 4000, 100);
            for row in Row::ALL
                .into_iter()
                .filter(|row| row.control() == Control::Check)
            {
                let row_rect = layout.row_rect(row, 0);
                let control = layout.control_rect(row, row_rect, 0);
                assert_eq!(control.right - control.left, scale(40, dpi), "{row:?}");
                assert_eq!(control.bottom - control.top, scale(20, dpi), "{row:?}");
                assert_eq!(
                    control.right,
                    row_rect.right - scale(16, dpi),
                    "right-aligned inside the card's padding"
                );
                assert!(
                    (control.top - row_rect.top - (row_rect.bottom - control.bottom)).abs() <= 1,
                    "centred in the row"
                );
            }
        }
    }

    #[test]
    fn the_knob_is_a_circle_inside_the_track_on_the_right_when_on() {
        // Break caught: a knob that pokes out of its track, is not round, or sits on the same
        // side whether the switch is on or off.
        let track = RECT {
            left: 100,
            top: 10,
            right: 140,
            bottom: 30,
        };
        let off = toggle_knob(track, false, 96);
        let on = toggle_knob(track, true, 96);
        assert_eq!(
            (off.left, off.top, off.right, off.bottom),
            (104, 14, 116, 26)
        );
        assert_eq!((on.left, on.top, on.right, on.bottom), (124, 14, 136, 26));

        let layout = Layout::calculate(192, 4000, 4000, 100);
        let track = layout.control_rect(Row::WordWrap, layout.row_rect(Row::WordWrap, 0), 0);
        for on in [false, true] {
            let knob = toggle_knob(track, on, 192);
            assert_eq!(knob.right - knob.left, knob.bottom - knob.top, "round");
            assert!(knob.left > track.left && knob.right < track.right);
            assert!(knob.top > track.top && knob.bottom < track.bottom);
            let middle = (track.left + track.right) / 2;
            if on {
                assert!(knob.left > middle, "on: right");
            } else {
                assert!(knob.right < middle, "off: left");
            }
        }
    }

    fn painted_dialog(canvas: Canvas, theme: crate::platform::theme::Theme) -> Dialog {
        Dialog {
            colors: Palette::for_theme(theme, false),
            link_color: 0,
            layout: Layout::calculate(96, 4000, 2000, 100),
            view: view(),
            model: DialogModel::new(Page::General),
            fonts: Vec::new(),
            scroll: 0,
            title_font: std::ptr::null_mut(),
            heading_font: std::ptr::null_mut(),
            body_font: std::ptr::null_mut(),
            link_font: std::ptr::null_mut(),
            glyph_font: std::ptr::null_mut(),
            hot: None,
            pressed: None,
            tracking_leave: false,
            list: None,
            picked_at: None,
            swallowing: false,
            outcome: Rc::new(Cell::new(Outcome::Closed)),
            canvas,
            page_layout: super::super::shortcuts_page::PageLayout::calculate(
                Layout::calculate(96, 4000, 2000, 100).body,
                96,
            ),
            shortcuts: super::super::shortcuts_model::ShortcutsModel::new(
                crate::window::keymap::Keymap::defaults(),
                1,
            ),
            last_row_click: None,
            wheel_rest: 0,
            search: None,
            search_brush: std::ptr::null_mut(),
        }
    }

    #[test]
    fn toggles_show_their_state_whether_direct2d_or_the_gdi_fallback_paints() {
        // Break caught: a switch whose knob or track is missing, on the wrong side, or the same
        // on and off, in either theme, and above all when Direct2D can't load and GDI paints.
        use crate::platform::theme::Theme;
        use crate::window::soft_paint::TestSurface;
        for theme in [Theme::Light, Theme::Dark] {
            for direct2d in [false, true] {
                let canvas = if direct2d {
                    Canvas::load()
                } else {
                    Canvas::gdi()
                };
                assert_eq!(canvas.uses_direct2d(), direct2d);
                let dialog = painted_dialog(canvas, theme);
                let layout = dialog.layout;
                let colors = dialog.colors;
                let tones = Tones::new(&colors);
                let surface = TestSurface::new(layout.width, layout.height);
                let client = RECT {
                    left: 0,
                    top: 0,
                    right: layout.width,
                    bottom: layout.height,
                };
                paint_into(surface.dc, client, &dialog);
                let at = |rect: RECT| surface.pixel(center(rect).0, center(rect).1);
                let control = |row| layout.control_rect(row, layout.row_rect(row, 0), 0);
                let case = format!("{theme:?}, Direct2D {direct2d}");

                // Line numbers are on by default: an accent track, the knob on the right.
                let track = control(Row::LineNumbers);
                assert_eq!(at(toggle_knob(track, true, 96)), tones.on_accent, "{case}");
                assert_eq!(at(toggle_knob(track, false, 96)), tones.accent, "{case}");
                // Word wrap is off: the card shows through the outline, the knob on the left.
                let track = control(Row::WordWrap);
                assert_eq!(
                    at(toggle_knob(track, false, 96)),
                    colors.muted_foreground,
                    "{case}"
                );
                assert_eq!(at(toggle_knob(track, true, 96)), tones.card, "{case}");
                assert_eq!(
                    surface.pixel(center(track).0, track.top),
                    colors.muted_foreground,
                    "{case}: the off outline"
                );
                // Notebook autosave with no notebook: greyed.
                let track = control(Row::NotebookAutosave);
                assert_eq!(
                    at(toggle_knob(track, false, 96)),
                    colors.pressed_background,
                    "{case}"
                );
                // The card, and Close as a filled accent button.
                let card = layout.row_rect(Row::Theme, 0);
                assert_eq!(surface.pixel(card.left + 8, center(card).1), tones.card);
                assert_eq!(
                    surface.pixel(layout.close.left + 4, center(layout.close).1),
                    tones.accent,
                    "{case}"
                );
            }
        }
    }

    #[test]
    fn dropdowns_take_the_extra_width_and_the_other_controls_keep_their_natural_sizes() {
        // Break caught: long font names cut off in a dropdown that stayed 240 px in a wider
        // dialog, a dropdown grown over its label, or segments, steppers and toggles stretched
        // instead of right-aligned at their own sizes.
        let view = view();
        for dpi in [96, 144, 192] {
            let layout = Layout::calculate(dpi, 8000, 8000, 100);
            for row in Row::ALL {
                let card = layout.row_rect(row, 0);
                let control = layout.control_rect(row, card, view.segments(row).len());
                let width = control.right - control.left;
                assert_eq!(control.right, card.right - scale(16, dpi), "{row:?}");
                let natural = match row.control() {
                    Control::Dropdown => 360,
                    Control::Segmented if row == Row::TabWidth => 44 * 3,
                    Control::Segmented => 80 * view.segments(row).len() as i32,
                    Control::Stepper => 28 * 2 + 48,
                    Control::Check => 40,
                };
                assert_eq!(width, scale(natural, dpi), "{row:?} at {dpi} DPI");
            }
        }
    }

    #[test]
    fn a_narrow_work_area_caps_the_width_and_shrinks_the_dropdowns_not_their_labels() {
        // Break caught: a dialog wider than the screen, its × or Close off the right edge, or a
        // dropdown squeezed over its label instead of giving up its extra width.
        let layout = Layout::calculate(96, 450, 2000, 100);
        assert_eq!(layout.width, 450);
        assert_eq!(layout.title_close.right, 450);
        assert_eq!(layout.close.right, 450 - 20);
        let view = view();
        for row in [Row::Theme, Row::Font] {
            let card = layout.row_rect(row, 0);
            assert_eq!((card.left, card.right), (170, 430));
            let control = layout.control_rect(row, card, 0);
            assert_eq!(control.right, card.right - 16);
            assert!(
                control.left >= card.left + 16 + 120,
                "{row:?}: the label keeps 120 px"
            );
            assert!(control.right - control.left < 360, "{row:?} gave up width");
        }
        let track = layout.control_rect(
            Row::WordWrap,
            layout.row_rect(Row::WordWrap, 0),
            view.segments(Row::WordWrap).len(),
        );
        assert_eq!(track.right - track.left, 40, "a toggle keeps its size");
        assert_eq!(
            Layout::calculate(144, 4000, 2000, 100).width,
            scale(860, 144),
            "a wide work area leaves the scaled width alone"
        );
    }

    #[test]
    fn a_short_work_area_caps_the_height_and_scrolls_the_focused_row_into_view() {
        // Break caught: a dialog taller than a 1366×768 screen at 150%, with Close off-screen,
        // or Tab moving the focus to a row the body never scrolls to (review focus 4).
        let layout = Layout::calculate(144, 4000, 700, 150);
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
        let tiny = Layout::calculate(96, 4000, 100, 100);
        assert!(tiny.height > 100, "at least a few rows always show");
    }
}
