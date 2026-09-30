//! The Search view's input: Tab focus stops, opening results, replace requests, tooltips and
//! the panel's message handler.

use super::*;
use crate::window::main_window::OpenMode;
use crate::window::option_toggles;
use crate::window::panel::scale;
use crate::window::row_list::{self, ListKey};
use crate::window::text_search_host;
use crate::window::tooltip::Tooltip;
use std::path::Path;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::UI::Controls::{EM_SETSEL, WM_MOUSELEAVE};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, SetFocus, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent,
    VIRTUAL_KEY, VK_CONTROL, VK_RETURN, VK_SHIFT, VK_TAB, VK_UP,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    SendMessageW, WM_CAPTURECHANGED, WM_CHAR, WM_KEYDOWN, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL,
};

pub(super) fn key_down(key: VIRTUAL_KEY) -> bool {
    (unsafe { GetKeyState(i32::from(key)) }) < 0
}

/// Where Tab and Shift+Tab move the caret in the Search view: the search box, the replace field
/// while it is open, and the results, in that order, wrapping.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum FocusStop {
    Box,
    Replace,
    Results,
}

/// The stop after `from` (before it with `back`). The replace field is skipped while it is
/// closed, and the results while there are none.
pub(super) fn next_stop(
    from: FocusStop,
    back: bool,
    replace_open: bool,
    has_results: bool,
) -> FocusStop {
    let order = [FocusStop::Box, FocusStop::Replace, FocusStop::Results];
    let usable = |stop: FocusStop| match stop {
        FocusStop::Box => true,
        FocusStop::Replace => replace_open,
        FocusStop::Results => has_results,
    };
    let start = order.iter().position(|stop| *stop == from).unwrap_or(0);
    (1..=order.len())
        .map(|step| {
            let index = if back {
                (start + order.len() * 2 - step) % order.len()
            } else {
                (start + step) % order.len()
            };
            order[index]
        })
        .find(|stop| usable(*stop))
        .unwrap_or(FocusStop::Box)
}

/// Opens the selected result, or the first one, as the preview tab or a normal tab.
pub(crate) fn open_selected(hwnd: HWND, mode: OpenMode, focus_editor: bool) {
    let Some(path) = with_view(hwnd, |view| {
        let index = view.list.selected.unwrap_or(0);
        view.results.get(index).map(|result| result.path.clone())
    })
    .flatten() else {
        return;
    };
    open_result(hwnd, &path, mode, focus_editor);
}

pub(super) fn open_result(hwnd: HWND, relative: &Path, mode: OpenMode, focus_editor: bool) {
    // A replace is changing these notes: a result opens again once its report is in.
    if with_view(hwnd, |view| view.replacing).unwrap_or(false) {
        return;
    }
    crate::window::main_window::open_search_result(hwnd, relative, mode, focus_editor);
}

/// Down from the box: the focus moves into the results.
pub(super) fn enter_results(hwnd: HWND, panel: HWND) {
    let (client, dpi) = geometry(panel);
    let has_results = with_view(hwnd, |view| {
        if view.results.is_empty() {
            return false;
        }
        let area = view.list_area(client, dpi);
        let index = view.list.selected.unwrap_or(0);
        view.list.select(index, height(area));
        true
    })
    .unwrap_or(false);
    if has_results {
        unsafe {
            SetFocus(panel);
        }
        invalidate(panel);
    }
}

/// Tab (Shift+Tab with `back`) from `from`: the caret moves to the next of the search box, the
/// replace field while it is open, and the results while there are any, wrapping. Focused with
/// nothing of the App borrowed.
pub(super) fn cycle_focus(hwnd: HWND, panel: HWND, from: FocusStop, back: bool) {
    let Some((edit, replace, has_results)) = with_view(hwnd, |view| {
        (
            view.edit,
            view.replace_edit.filter(|_| view.replace_open),
            !view.results.is_empty(),
        )
    }) else {
        return;
    };
    let to = next_stop(from, back, replace.is_some(), has_results);
    if to == from {
        return;
    }
    let field = match to {
        FocusStop::Box => edit,
        FocusStop::Replace => replace,
        FocusStop::Results => {
            enter_results(hwnd, panel);
            return;
        }
    };
    if let Some(field) = field {
        // As a dialog's Tab does, the field's text comes selected.
        unsafe {
            SetFocus(field);
            SendMessageW(field, EM_SETSEL, 0, -1);
        }
    }
}

/// Replace all (spec §11): its button, or Ctrl+Alt+Enter in either field. It runs only while the
/// replace field is open and a search finished with results (`SearchView::replace_all_enabled`).
pub(super) fn replace_all_requested(hwnd: HWND) {
    let ready =
        with_view(hwnd, |view| view.replace_open && view.replace_all_enabled()).unwrap_or(false);
    if ready {
        #[cfg(test)]
        with_view(hwnd, |view| view.replace_all_requests += 1);
        text_search_host::replace_all(hwnd);
    }
}

/// A result's replace button, or Ctrl+Shift+1 on the selected result: replaces in that note only
/// (spec §11). It runs under the same rule as Replace all, while the replace field is open.
pub(super) fn row_replace_requested(hwnd: HWND, index: usize) {
    let path = with_view(hwnd, |view| {
        (view.replace_open && view.replace_all_enabled())
            .then(|| view.results.get(index).map(|hit| hit.path.clone()))
            .flatten()
    })
    .flatten();
    if let Some(path) = path {
        #[cfg(test)]
        with_view(hwnd, |view| view.row_replace_requests.push(index));
        text_search_host::replace_in(hwnd, &path);
    }
}

/// Whether panel point (`x`, `y`) is on the chevron or the painted search field, which are client
/// area, not window caption. Both show once the box exists.
pub(crate) fn header_hit(hwnd: HWND, panel: HWND, x: i32, y: i32) -> bool {
    let (client, dpi) = geometry(panel);
    let point = POINT { x, y };
    with_view(hwnd, |view| view.edit.is_some()).unwrap_or(false)
        && (inside(SearchView::field_rect(client, dpi), point)
            || inside(SearchView::chevron_rect(client, dpi), point))
}

/// A press on the field's padding, outside the box itself, puts the caret in the box. Reports
/// whether the press was on the field.
pub(super) fn field_pressed(hwnd: HWND, panel: HWND, at: POINT) -> bool {
    if !header_hit(hwnd, panel, at.x, at.y) {
        return false;
    }
    // Focused with nothing of the App borrowed: SetFocus sends focus messages.
    if let Some(edit) = with_view(hwnd, |view| view.edit).flatten() {
        unsafe {
            SetFocus(edit);
        }
    }
    true
}

/// A press on the replace field's padding, outside the field's Edit, puts the caret in it.
/// Reports whether the press was on the field.
pub(super) fn replace_field_pressed(hwnd: HWND, panel: HWND, at: POINT) -> bool {
    let (client, dpi) = geometry(panel);
    let Some(replace) =
        with_view(hwnd, |view| view.replace_edit.filter(|_| view.replace_open)).flatten()
    else {
        return false;
    };
    if !inside(SearchView::replace_field_rect(client, dpi), at) {
        return false;
    }
    // Focused with nothing of the App borrowed.
    unsafe {
        SetFocus(replace);
    }
    true
}

/// Gives the tooltip the tools the view has now, making the tooltip on the first pointer move and
/// handing it that move. Runs with nothing of the App borrowed: creating the control and adding
/// tools send messages.
pub(super) fn update_tooltips(
    hwnd: HWND,
    panel: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) {
    let (client, dpi) = geometry(panel);
    let Some((tools, existing, failed, changed)) = with_view(hwnd, |view| {
        let tools = view.tooltip_tools(client, dpi);
        let changed = tools != view.tools_shown;
        (tools, view.tooltip, view.tooltip_failed, changed)
    }) else {
        return;
    };
    let (tooltip, created) = match existing {
        Some(tooltip) => (tooltip, false),
        None if failed => return,
        None => {
            let created = Tooltip::create(panel);
            let kept = with_view(hwnd, |view| {
                view.tooltip = created;
                view.tooltip_failed = created.is_none();
            });
            match (created, kept) {
                (Some(tooltip), Some(())) => {
                    tooltip.set_max_width(scale(TOOLTIP_WIDTH_AT_96_DPI, dpi));
                    (tooltip, true)
                }
                (Some(tooltip), None) => {
                    tooltip.destroy();
                    return;
                }
                (None, _) => return,
            }
        }
    };
    if changed || created {
        for (id, rect, text) in &tools {
            tooltip.set_tool(*id, rect_of(*rect), text);
        }
        with_view(hwnd, |view| view.tools_shown = tools);
    }
    if created {
        tooltip.relay(message, wparam, lparam);
    }
}

/// Input for the Search view's field and result list. `None` leaves the message to the panel.
pub(crate) fn handle(
    hwnd: HWND,
    panel: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> Option<LRESULT> {
    // Alt+C, Alt+W and Alt+R in the results flip the toggles (spec §4), and the character that
    // follows is swallowed before the menu band sees it.
    if let Some(option) = option_toggles::alt_option(message, wparam, lparam) {
        toggle_option(hwnd, option);
        return Some(0);
    }
    if option_toggles::is_toggle_char(message, wparam, lparam) {
        return Some(0);
    }
    let (client, dpi) = geometry(panel);
    match message {
        WM_MOUSEMOVE => {
            let mut track = TRACKMOUSEEVENT {
                cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: panel,
                dwHoverTime: 0,
            };
            unsafe {
                TrackMouseEvent(&mut track);
            }
            let at = point(lparam);
            let changed = with_view(hwnd, |view| {
                if let Some(grab) = view.thumb_grab {
                    let area = view.list_area(client, dpi);
                    return view.list.drag_thumb(grab, at.y - area.top, height(area));
                }
                let toggle = view.toggle_at(at, client, dpi);
                let toggle_changed = std::mem::replace(&mut view.toggle_hover, toggle) != toggle;
                let header = view.header_button_at(at, client, dpi);
                let header_changed = std::mem::replace(&mut view.header_hover, header) != header;
                let hover = view.row_under(at, client, dpi);
                let row_changed = view.list.set_hover(hover);
                // After the hover moved: the button shows on the hovered row.
                let button = view.row_button_at(at, client, dpi);
                let button_changed =
                    std::mem::replace(&mut view.row_hover_button, button) != button;
                toggle_changed || header_changed || row_changed || button_changed
            })
            .unwrap_or(false);
            if changed {
                invalidate(panel);
            }
            update_tooltips(hwnd, panel, message, wparam, lparam);
            Some(0)
        }
        WM_MOUSELEAVE => {
            let changed = with_view(hwnd, |view| {
                let toggle_changed = view.toggle_hover.take().is_some();
                let header_changed = view.header_hover.take().is_some();
                let button_changed = view.row_hover_button.take().is_some();
                let row_changed = view.list.set_hover(None);
                toggle_changed || header_changed || button_changed || row_changed
            })
            .unwrap_or(false);
            if changed {
                invalidate(panel);
            }
            Some(0)
        }
        WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
            let at = point(lparam);
            if let Some(option) = with_view(hwnd, |view| view.toggle_at(at, client, dpi)).flatten()
            {
                toggle_option(hwnd, option);
                return Some(0);
            }
            // A double click on a button is only its first press again.
            let pressed = message == WM_LBUTTONDOWN;
            match with_view(hwnd, |view| view.header_button_at(at, client, dpi)).flatten() {
                Some(HeaderButton::Chevron) => {
                    if pressed {
                        toggle_replace(hwnd);
                    }
                    return Some(0);
                }
                Some(HeaderButton::Clear) => {
                    if pressed {
                        clear_search(hwnd);
                    }
                    return Some(0);
                }
                Some(HeaderButton::ReplaceAll) => {
                    if pressed {
                        replace_all_requested(hwnd);
                    }
                    return Some(0);
                }
                None => {}
            }
            if replace_field_pressed(hwnd, panel, at) || field_pressed(hwnd, panel, at) {
                return Some(0);
            }
            if let Some(index) =
                with_view(hwnd, |view| view.row_button_at(at, client, dpi)).flatten()
            {
                if pressed {
                    row_replace_requested(hwnd, index);
                }
                return Some(0);
            }
            // A press on the scroll thumb drags it, as in the Notebook view.
            let grabbed = message == WM_LBUTTONDOWN
                && with_view(hwnd, |view| {
                    view.thumb_grab = view.thumb_at(at, client, dpi);
                    view.thumb_grab.is_some()
                })
                .unwrap_or(false);
            if grabbed {
                unsafe {
                    SetCapture(panel);
                }
                return Some(0);
            }
            let path = with_view(hwnd, |view| {
                let index = view.row_under(at, client, dpi)?;
                let area = view.list_area(client, dpi);
                view.list.select(index, height(area));
                Some(view.results[index].path.clone())
            })
            .flatten();
            invalidate(panel);
            if let Some(path) = path {
                let mode = if message == WM_LBUTTONDBLCLK {
                    OpenMode::Permanent
                } else {
                    OpenMode::Preview
                };
                // A mouse click moves the focus to the editor (spec §6.4).
                open_result(hwnd, &path, mode, true);
            }
            Some(0)
        }
        WM_LBUTTONUP => {
            if with_view(hwnd, |view| view.thumb_grab.take().is_some()).unwrap_or(false) {
                unsafe {
                    ReleaseCapture();
                }
            }
            Some(0)
        }
        WM_CAPTURECHANGED => {
            with_view(hwnd, |view| view.thumb_grab = None);
            Some(0)
        }
        WM_MOUSEWHEEL => {
            let delta = i32::from((wparam >> 16) as u16 as i16);
            let lines = row_list::wheel_lines();
            let scrolled = with_view(hwnd, |view| {
                let area = view.list_area(client, dpi);
                view.list.wheel(delta, lines, height(area))
            })
            .unwrap_or(false);
            if scrolled {
                invalidate(panel);
            }
            Some(0)
        }
        WM_KEYDOWN => {
            let key = wparam as u16;
            let ctrl = key_down(VK_CONTROL);
            let shift = key_down(VK_SHIFT);
            // Tab and Shift+Tab move on to the search box or the replace field.
            if key == VK_TAB && !ctrl {
                cycle_focus(hwnd, panel, FocusStop::Results, shift);
                return Some(0);
            }
            // Ctrl+Shift+1 is the selected row's replace button (VS Code's binding).
            if key == u16::from(b'1') && ctrl && shift {
                if let Some(index) = with_view(hwnd, |view| view.list.selected).flatten() {
                    row_replace_requested(hwnd, index);
                }
                return Some(0);
            }
            // Enter opens a normal tab: the preview tab is the mouse's (spec §6.4).
            if key == VK_RETURN {
                open_selected(hwnd, OpenMode::Permanent, true);
                return Some(0);
            }
            let at_top = with_view(hwnd, |view| {
                view.list.selected.is_none_or(|index| index == 0)
            })
            .unwrap_or(true);
            if key == VK_UP && at_top {
                if let Some(edit) = with_view(hwnd, |view| view.edit).flatten() {
                    unsafe {
                        SetFocus(edit);
                    }
                }
                return Some(0);
            }
            let movement = ListKey::from_virtual_key(u32::from(key))?;
            with_view(hwnd, |view| {
                let area = view.list_area(client, dpi);
                view.list.move_selection(movement, height(area))
            });
            invalidate(panel);
            Some(0)
        }
        // Typing in the list goes on in the box.
        WM_CHAR if (wparam as u32) >= 0x20 && wparam as u32 != 0x7f => {
            let edit = with_view(hwnd, |view| view.edit).flatten()?;
            unsafe {
                SetFocus(edit);
                SendMessageW(edit, WM_CHAR, wparam, lparam);
            }
            Some(0)
        }
        _ => None,
    }
}
