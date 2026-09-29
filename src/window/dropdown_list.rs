//! The Settings dialog's dropdown list: a themed popup under a dropdown that never takes the
//! activation, so the keyboard stays with the dialog, which forwards keys here (settings dialog
//! spec §3.5). `ListModel` is the pure part: selection, scrolling and type-ahead.

use super::palette::Palette;
use super::panel::{fill, inset};
use crate::platform::wide_null;
use windows_sys::Win32::Foundation::{
    ERROR_CLASS_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, ClientToScreen, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER,
    DrawTextW, EndPaint, GetMonitorInfoW, HFONT, InvalidateRect, MONITOR_DEFAULTTONEAREST,
    MONITORINFO, MonitorFromRect, PAINTSTRUCT, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CS_DROPSHADOW, CreateWindowExW, DefWindowProcW, DestroyWindow, GWLP_USERDATA, GetClientRect,
    GetWindowLongPtrW, IDC_ARROW, LoadCursorW, MA_NOACTIVATE, PostMessageW, RegisterClassW,
    SW_SHOWNA, SetWindowLongPtrW, ShowWindow, WM_APP, WM_ERASEBKGND, WM_LBUTTONUP,
    WM_MOUSEACTIVATE, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCDESTROY, WM_PAINT, WNDCLASSW,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

pub(crate) const VISIBLE_ROWS: usize = 10;
/// Letters typed within this long of each other build up one type-ahead prefix.
const TYPE_AHEAD_PAUSE_MS: u32 = 1000;

/// Posted to the owner (the Settings dialog) when a row is clicked; `wparam` is the item index
/// and `lparam` the click's screen point, packed like a mouse message's. `WM_APP + 1` is the
/// dialog's own message space, not the main window's.
pub(crate) const WM_LIST_PICKED: u32 = WM_APP + 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ListKey {
    Up,
    Down,
    PageUp,
    PageDown,
    Home,
    End,
    Enter,
    Escape,
    Char(char),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ListOutcome {
    Ignored,
    Moved,
    Picked(usize),
    Dismissed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ListModel {
    pub items: Vec<String>,
    pub selected: usize,
    /// The first visible item.
    pub top: usize,
    typed: String,
    last_typed_at: u32,
    /// Wheel travel not yet worth a whole row, in 1/120ths of a row (precision touchpads send
    /// deltas far smaller than a notch).
    wheel_remainder: i32,
}

impl ListModel {
    /// A list with `selected` (or the first item) selected and scrolled into view.
    pub(crate) fn new(items: Vec<String>, selected: Option<usize>) -> Self {
        let selected = selected.unwrap_or(0).min(items.len().saturating_sub(1));
        let mut model = Self {
            items,
            selected,
            top: 0,
            typed: String::new(),
            last_typed_at: 0,
            wheel_remainder: 0,
        };
        model.scroll_into_view();
        model
    }

    pub(crate) fn visible_rows(&self) -> usize {
        self.items.len().min(VISIBLE_ROWS)
    }

    /// The item shown in visible row `row`.
    pub(crate) fn item_at_row(&self, row: usize) -> Option<usize> {
        let index = self.top + row;
        (row < VISIBLE_ROWS && index < self.items.len()).then_some(index)
    }

    /// Scrolls by `rows` without moving the selection; true when the view moved.
    pub(crate) fn scroll(&mut self, rows: isize) -> bool {
        let max_top = self.items.len().saturating_sub(VISIBLE_ROWS);
        let top = self.top.saturating_add_signed(rows).min(max_top);
        let moved = top != self.top;
        self.top = top;
        moved
    }

    /// A wheel turn of `delta` (120 per notch): three rows per notch, keeping what a small
    /// delta adds up to until it makes a whole row. True when the view moved.
    pub(crate) fn wheel(&mut self, delta: i16) -> bool {
        let travel = self.wheel_remainder + i32::from(delta) * 3;
        let rows = travel / 120;
        self.wheel_remainder = travel - rows * 120;
        rows != 0 && self.scroll(-rows as isize)
    }

    pub(crate) fn key(&mut self, key: ListKey, now_ms: u32) -> ListOutcome {
        if self.items.is_empty() {
            return match key {
                ListKey::Enter | ListKey::Escape => ListOutcome::Dismissed,
                _ => ListOutcome::Ignored,
            };
        }
        let last = self.items.len() - 1;
        let page = VISIBLE_ROWS - 1;
        match key {
            ListKey::Up => self.select(self.selected.saturating_sub(1)),
            ListKey::Down => self.select((self.selected + 1).min(last)),
            ListKey::PageUp => self.select(self.selected.saturating_sub(page)),
            ListKey::PageDown => self.select((self.selected + page).min(last)),
            ListKey::Home => self.select(0),
            ListKey::End => self.select(last),
            ListKey::Enter => ListOutcome::Picked(self.selected),
            ListKey::Escape => ListOutcome::Dismissed,
            ListKey::Char(c) => self.type_ahead(c, now_ms),
        }
    }

    fn select(&mut self, index: usize) -> ListOutcome {
        if index == self.selected {
            return ListOutcome::Ignored;
        }
        self.selected = index;
        self.scroll_into_view();
        ListOutcome::Moved
    }

    fn scroll_into_view(&mut self) {
        if self.selected < self.top {
            self.top = self.selected;
        } else if self.selected >= self.top + VISIBLE_ROWS {
            self.top = self.selected + 1 - VISIBLE_ROWS;
        }
    }

    /// Jumps to the next item that starts with the typed text. Typing one letter again cycles
    /// through the items starting with it; a longer prefix stays put while it still matches.
    fn type_ahead(&mut self, c: char, now_ms: u32) -> ListOutcome {
        if c.is_control() {
            return ListOutcome::Ignored;
        }
        if now_ms.wrapping_sub(self.last_typed_at) > TYPE_AHEAD_PAUSE_MS {
            self.typed.clear();
        }
        self.last_typed_at = now_ms;
        self.typed.extend(c.to_lowercase());
        let start = if self.typed.chars().count() == 1 {
            self.selected + 1
        } else {
            self.selected
        };
        let count = self.items.len();
        let found = (0..count)
            .map(|offset| (start + offset) % count)
            .find(|&index| self.items[index].to_lowercase().starts_with(&self.typed));
        // A prefix that still matches the selected item leaves it selected: `Ignored`.
        found.map_or(ListOutcome::Ignored, |index| self.select(index))
    }
}

/// The open list popup. Dropping it destroys the window.
pub(crate) struct DropdownList {
    hwnd: HWND,
}

struct ListState {
    owner: HWND,
    model: ListModel,
    colors: Palette,
    /// Borrowed from the dialog, which outlives the list.
    font: HFONT,
    row_height: i32,
    hot: Option<usize>,
}

impl DropdownList {
    /// Shows `model` directly under `anchor` (screen coordinates), as wide as it, or above it when
    /// the monitor's work area has no room below. It never takes the activation from `owner`.
    pub(crate) fn show(
        owner: HWND,
        anchor: RECT,
        row_height: i32,
        font: HFONT,
        colors: Palette,
        model: ListModel,
    ) -> Option<Self> {
        let class = register_class()?;
        let height = model.visible_rows().max(1) as i32 * row_height + 2;
        let mut monitor = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let top = unsafe {
            if GetMonitorInfoW(
                MonitorFromRect(&anchor, MONITOR_DEFAULTTONEAREST),
                &mut monitor,
            ) != 0
                && anchor.bottom + height > monitor.rcWork.bottom
            {
                anchor.top - height
            } else {
                anchor.bottom
            }
        };
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST,
                class.as_ptr(),
                std::ptr::null(),
                WS_POPUP,
                anchor.left,
                top,
                anchor.right - anchor.left,
                height,
                owner,
                std::ptr::null_mut(),
                GetModuleHandleW(std::ptr::null()),
                std::ptr::null(),
            )
        };
        if hwnd.is_null() {
            return None;
        }
        let state = Box::new(ListState {
            owner,
            model,
            colors,
            font,
            row_height,
            hot: None,
        });
        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(state) as isize);
            ShowWindow(hwnd, SW_SHOWNA);
        }
        Some(Self { hwnd })
    }

    /// A key the dialog forwarded; repaints when the selection moved.
    pub(crate) fn key(&self, key: ListKey, now_ms: u32) -> ListOutcome {
        let Some(state) = state(self.hwnd) else {
            return ListOutcome::Dismissed;
        };
        let outcome = state.model.key(key, now_ms);
        if outcome == ListOutcome::Moved {
            invalidate(self.hwnd);
        }
        outcome
    }

    /// A mouse wheel turn the dialog forwarded.
    pub(crate) fn wheel(&self, delta: i16) {
        wheel(self.hwnd, delta);
    }
}

impl Drop for DropdownList {
    fn drop(&mut self) {
        unsafe { DestroyWindow(self.hwnd) };
    }
}

fn register_class() -> Option<&'static [u16]> {
    static CLASS_NAME: std::sync::OnceLock<Vec<u16>> = std::sync::OnceLock::new();
    static REGISTERED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let name = CLASS_NAME.get_or_init(|| wide_null("FastPadSettingsList"));
    let registered = *REGISTERED.get_or_init(|| {
        let class = WNDCLASSW {
            style: CS_DROPSHADOW,
            lpfnWndProc: Some(list_proc),
            hInstance: unsafe { GetModuleHandleW(std::ptr::null()) },
            hCursor: unsafe { LoadCursorW(std::ptr::null_mut(), IDC_ARROW) },
            lpszClassName: name.as_ptr(),
            ..Default::default()
        };
        unsafe { RegisterClassW(&class) != 0 || GetLastError() == ERROR_CLASS_ALREADY_EXISTS }
    });
    registered.then_some(name.as_slice())
}

fn state<'a>(hwnd: HWND) -> Option<&'a mut ListState> {
    let pointer = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut ListState;
    // SAFETY: set once in `show` from `Box::into_raw` and cleared in WM_NCDESTROY; this thread
    // only, and never two at once.
    unsafe { pointer.as_mut() }
}

fn invalidate(hwnd: HWND) {
    unsafe { InvalidateRect(hwnd, std::ptr::null(), 0) };
}

/// Scrolls the list for a wheel turn of `delta`, whichever window the wheel reached.
fn wheel(hwnd: HWND, delta: i16) {
    if state(hwnd).is_some_and(|list| list.model.wheel(delta)) {
        invalidate(hwnd);
    }
}

/// The item under client `y`, one pixel of border above the first row.
fn item_at(state: &ListState, y: i32) -> Option<usize> {
    let row = (y - 1).max(0) / state.row_height.max(1);
    state.model.item_at_row(row as usize)
}

unsafe extern "system" fn list_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // Checked without forming a reference, and each arm borrows only for itself: `paint` holds
    // a `&ListState` across `BeginPaint`, which sends WM_ERASEBKGND back here.
    if unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } == 0 {
        return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
    }
    match message {
        WM_ERASEBKGND => 1,
        WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
        WM_PAINT => {
            if let Some(list) = state(hwnd) {
                paint(hwnd, list);
            }
            0
        }
        WM_MOUSEMOVE => {
            if let Some(list) = state(hwnd) {
                let hot = item_at(list, ((lparam >> 16) & 0xffff) as i16 as i32);
                if hot != list.hot {
                    list.hot = hot;
                    invalidate(hwnd);
                }
            }
            0
        }
        // With "scroll inactive windows when I hover over them" the wheel comes here rather
        // than to the dialog, and DefWindowProc would not pass it on.
        WM_MOUSEWHEEL => {
            wheel(hwnd, ((wparam >> 16) & 0xffff) as i16);
            0
        }
        WM_LBUTTONUP => {
            let mut point = POINT {
                x: (lparam & 0xffff) as i16 as i32,
                y: ((lparam >> 16) & 0xffff) as i16 as i32,
            };
            let picked = state(hwnd)
                .and_then(|list| item_at(list, point.y).map(|index| (list.owner, index)));
            if let Some((owner, index)) = picked {
                unsafe { ClientToScreen(hwnd, &mut point) };
                let at = (point.x as u16 as usize | ((point.y as u16 as usize) << 16)) as LPARAM;
                // Posted: the dialog destroys this window when it handles the pick.
                unsafe { PostMessageW(owner, WM_LIST_PICKED, index, at) };
            }
            0
        }
        WM_NCDESTROY => {
            let pointer = unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) } as *mut ListState;
            // SAFETY: from `Box::into_raw` in `show`, released only here.
            drop(unsafe { Box::from_raw(pointer) });
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

fn paint(hwnd: HWND, list: &ListState) {
    let mut paint = PAINTSTRUCT::default();
    let dc = unsafe { BeginPaint(hwnd, &mut paint) };
    if dc.is_null() {
        return;
    }
    let colors = list.colors;
    let mut client = RECT::default();
    unsafe {
        GetClientRect(hwnd, &mut client);
        fill(dc, client, colors.muted_foreground);
        fill(dc, inset(client, 1), colors.panel_background());
        SetBkMode(dc, TRANSPARENT as i32);
        let previous = SelectObject(dc, list.font as _);
        let text_inset = list.row_height / 3;
        for row in 0..list.model.visible_rows() {
            let Some(index) = list.model.item_at_row(row) else {
                break;
            };
            let top = 1 + row as i32 * list.row_height;
            let rect = RECT {
                left: 1,
                top,
                right: client.right - 1,
                bottom: top + list.row_height,
            };
            let foreground = if index == list.model.selected {
                fill(dc, rect, colors.selection_background);
                colors
                    .selection_foreground
                    .unwrap_or(colors.editor_foreground)
            } else {
                if list.hot == Some(index) {
                    fill(dc, rect, colors.hover_background);
                }
                colors.editor_foreground
            };
            SetTextColor(dc, foreground);
            let mut text = wide_null(&list.model.items[index]);
            let mut text_rect = RECT {
                left: rect.left + text_inset,
                right: rect.right - text_inset,
                ..rect
            };
            DrawTextW(
                dc,
                text.as_mut_ptr(),
                -1,
                &mut text_rect,
                DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX | DT_END_ELLIPSIS,
            );
        }
        // A thin thumb shows where the view is in a list longer than it.
        let count = list.model.items.len();
        if count > VISIBLE_ROWS {
            let track = client.bottom - 2;
            let thumb = (track * VISIBLE_ROWS as i32 / count as i32).max(8);
            let top =
                1 + (track - thumb) * list.model.top as i32 / (count - VISIBLE_ROWS).max(1) as i32;
            fill(
                dc,
                RECT {
                    left: client.right - 5,
                    top,
                    right: client.right - 2,
                    bottom: top + thumb,
                },
                colors.muted_foreground,
            );
        }
        SelectObject(dc, previous);
        EndPaint(hwnd, &paint);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn the_current_value_opens_selected_and_scrolled_into_view() {
        let model = ListModel::new((0..30).map(|i| format!("Font {i}")).collect(), Some(25));
        assert_eq!(model.selected, 25);
        assert_eq!(model.top, 16, "the selected row is the last visible one");
        assert_eq!(ListModel::new(Vec::new(), Some(3)).selected, 0);
        assert_eq!(ListModel::new(items(&["a"]), None).selected, 0);
    }

    #[test]
    fn arrows_pages_and_ends_move_the_selection_and_the_view() {
        let mut model = ListModel::new((0..30).map(|i| format!("Font {i}")).collect(), Some(0));
        assert_eq!(model.key(ListKey::Up, 0), ListOutcome::Ignored);
        assert_eq!(model.key(ListKey::Down, 0), ListOutcome::Moved);
        assert_eq!(model.selected, 1);
        model.key(ListKey::PageDown, 0);
        assert_eq!(model.selected, 10);
        assert_eq!(model.top, 1);
        model.key(ListKey::End, 0);
        assert_eq!((model.selected, model.top), (29, 20));
        model.key(ListKey::Home, 0);
        assert_eq!((model.selected, model.top), (0, 0));
        assert_eq!(model.key(ListKey::Enter, 0), ListOutcome::Picked(0));
        assert_eq!(model.key(ListKey::Escape, 0), ListOutcome::Dismissed);
        assert!(model.scroll(3));
        assert_eq!(model.top, 3);
        assert!(model.scroll(-10));
        assert_eq!(model.top, 0);
        assert!(!model.scroll(-1), "already at the top");
        assert_eq!(model.item_at_row(2), Some(2));
        assert_eq!(model.item_at_row(VISIBLE_ROWS), None);
    }

    #[test]
    fn small_wheel_deltas_add_up_to_whole_rows() {
        // Break caught: a precision touchpad's deltas of a few units each truncating to zero
        // rows, so the list never scrolls under two-finger scrolling (final review 4a).
        let mut model = ListModel::new((0..30).map(|i| format!("Font {i}")).collect(), Some(0));
        assert!(!model.wheel(-30), "a quarter notch is less than a row");
        assert_eq!(model.top, 0);
        assert!(model.wheel(-30), "half a notch: 1.5 rows, one whole row");
        assert_eq!(model.top, 1);
        assert!(model.wheel(-60), "the kept half row plus 1.5 rows");
        assert_eq!(model.top, 3);
        assert!(model.wheel(-120), "a notch is three rows");
        assert_eq!(model.top, 6);
        assert!(model.wheel(120));
        assert_eq!(model.top, 3);
    }

    #[test]
    fn type_ahead_builds_a_prefix_cycles_one_letter_and_resets_after_a_pause() {
        // Break caught: typing "cas" in a list of hundreds of fonts landing on "Calibri", or a
        // prefix that never resets so the list stops responding to letters.
        let mut model = ListModel::new(
            items(&[
                "Arial",
                "Calibri",
                "Cascadia Code",
                "Cascadia Mono",
                "Consolas",
            ]),
            Some(0),
        );
        model.key(ListKey::Char('c'), 5_000);
        assert_eq!(model.selected, 1, "Calibri");
        model.key(ListKey::Char('a'), 5_100);
        model.key(ListKey::Char('s'), 5_200);
        assert_eq!(model.selected, 2, "Cascadia Code");
        model.key(ListKey::Char('c'), 9_000);
        assert_eq!(
            model.selected, 3,
            "after a pause a single c moves on to the next C item"
        );
        assert_eq!(model.key(ListKey::Char('c'), 9_100), ListOutcome::Ignored);
        assert_eq!(model.selected, 3, "\"cc\" matches nothing and stays put");
        model.key(ListKey::Char('c'), 11_000);
        assert_eq!(model.selected, 4, "after another pause: Consolas");
        assert_eq!(model.key(ListKey::Char('z'), 20_000), ListOutcome::Ignored);
        assert_eq!(
            model.key(ListKey::Char('\u{8}'), 21_000),
            ListOutcome::Ignored
        );
    }
}
