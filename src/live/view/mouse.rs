//! The mouse in the Live view (phase 1 plan, Task 10): a press places the caret, Shift extends,
//! a double press selects a word and a triple press a line, and a drag extends by the same unit,
//! scrolling while the pointer is above or below the view. Ctrl+click follows a link.

use super::*;
use std::ops::Range;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetDoubleClickTime, GetKeyState, ReleaseCapture, SetCapture, SetFocus, VK_CONTROL, VK_SHIFT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetMessageTime, GetSystemMetrics, KillTimer, SM_CXDOUBLECLK, SM_CYDOUBLECLK, SendMessageW,
    SetTimer,
};

/// The drag autoscroll timer's id.
pub(super) const AUTOSCROLL_TIMER: usize = 2;
const AUTOSCROLL_MS: u32 = 30;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Unit {
    Character,
    Word,
    Line,
}

pub(super) struct Drag {
    unit: Unit,
    /// What the press selected: the selection always keeps it.
    anchor: Range<usize>,
    /// The pointer's last client point.
    point: (i32, i32),
}

#[derive(Clone, Copy)]
pub(super) struct Click {
    time: u32,
    point: (i32, i32),
    count: u8,
}

fn point(lparam: isize) -> (i32, i32) {
    (
        (lparam & 0xFFFF) as u16 as i16 as i32,
        ((lparam >> 16) & 0xFFFF) as u16 as i16 as i32,
    )
}

fn down(key: u16) -> bool {
    (unsafe { GetKeyState(i32::from(key)) }) < 0
}

/// The document position under client point (`x`, `y`).
fn position_at(state: &mut ViewState, (x, y): (i32, i32)) -> usize {
    prepare(state);
    let scale = dpi_scale(state.hwnd);
    let doc_x = x as f32 / scale - text_left(state);
    let doc_y = y as f32 / scale + state.scroll_y;
    let ViewState {
        editor,
        layout,
        document,
        revealed,
        ..
    } = state;
    let source = EditorText(editor);
    let text = Styled {
        text: &source,
        document,
        revealed: revealed.clone(),
    };
    layout.position_at(&text, doc_x, doc_y).unwrap_or(0)
}

/// The `unit` around `position`.
fn unit_at(text: &impl LiveText, unit: Unit, position: usize) -> Range<usize> {
    match unit {
        Unit::Character => position..position,
        Unit::Word => {
            let word = text.word_at(position);
            if word.is_empty() {
                position..position
            } else {
                word
            }
        }
        Unit::Line => {
            let line = text.line_of(position);
            text.line_start(line)..text.line_start(line + 1)
        }
    }
}

/// `WM_LBUTTONDOWN`.
pub(super) fn press(hwnd: HWND, lparam: isize) {
    unsafe { SetFocus(hwnd) };
    let at = point(lparam);
    if down(VK_CONTROL) {
        let position = with_state(hwnd, |state| position_at(state, at)).unwrap_or(0);
        // The main window follows a link under the pointer; the click is then taken.
        let followed = unsafe {
            SendMessageW(
                crate::platform::win32::root_window(hwnd),
                crate::window::WM_FASTPAD_LIVE_LINK,
                hwnd as usize,
                position as isize,
            )
        };
        if followed != 0 {
            return;
        }
    }
    let time = unsafe { GetMessageTime() } as u32;
    let extend = down(VK_SHIFT);
    with_state(hwnd, |state| {
        let count = match state.clicks {
            Some(last)
                if time.wrapping_sub(last.time) <= unsafe { GetDoubleClickTime() }
                    && (at.0 - last.point.0).abs()
                        <= unsafe { GetSystemMetrics(SM_CXDOUBLECLK) } / 2
                    && (at.1 - last.point.1).abs()
                        <= unsafe { GetSystemMetrics(SM_CYDOUBLECLK) } / 2 =>
            {
                last.count % 3 + 1
            }
            _ => 1,
        };
        state.clicks = Some(Click {
            time,
            point: at,
            count,
        });
        let unit = match count {
            1 => Unit::Character,
            2 => Unit::Word,
            _ => Unit::Line,
        };
        let position = position_at(state, at);
        let text = EditorText(&state.editor);
        let (old_anchor, _) = text.selection();
        let anchor = if unit == Unit::Character && extend {
            old_anchor..old_anchor
        } else {
            unit_at(&text, unit, position)
        };
        state.goal = None;
        state.drag = Some(Drag {
            unit,
            anchor,
            point: at,
        });
        extend_to(state, position);
    });
    unsafe { SetCapture(hwnd) };
    super::keys::restart_blink(hwnd);
    post_changed(hwnd, false);
}

/// The selection from the drag's anchor to the `unit` around `position`.
fn extend_to(state: &mut ViewState, position: usize) {
    let Some(drag) = &state.drag else {
        return;
    };
    let text = EditorText(&state.editor);
    let around = unit_at(&text, drag.unit, position);
    let (anchor, caret) = if around.start < drag.anchor.start {
        (drag.anchor.end, around.start)
    } else {
        (drag.anchor.start, around.end.max(drag.anchor.end))
    };
    text.set_selection(anchor, caret);
}

/// `WM_MOUSEMOVE`: extends a drag, and scrolls while the pointer is outside the view.
pub(super) fn moved(hwnd: HWND, lparam: isize) {
    let at = point(lparam);
    let dragging = with_state(hwnd, |state| {
        let Some(drag) = &mut state.drag else {
            return false;
        };
        drag.point = at;
        let position = position_at(state, at);
        extend_to(state, position);
        true
    })
    .unwrap_or(false);
    if !dragging {
        return;
    }
    let mut client = RECT::default();
    unsafe { GetClientRect(hwnd, &mut client) };
    if at.1 < client.top || at.1 >= client.bottom {
        unsafe { SetTimer(hwnd, AUTOSCROLL_TIMER, AUTOSCROLL_MS, None) };
    } else {
        unsafe { KillTimer(hwnd, AUTOSCROLL_TIMER) };
    }
    post_changed(hwnd, false);
}

/// `WM_TIMER` while a drag holds the pointer above or below the view: scrolls toward it, by
/// more the farther away it is, and extends the selection to what scrolled in.
pub(super) fn autoscroll(hwnd: HWND) {
    let mut client = RECT::default();
    unsafe { GetClientRect(hwnd, &mut client) };
    let (_, height) = view_size(hwnd);
    let scrolled = with_state(hwnd, |state| {
        let point = state.drag.as_ref()?.point;
        let distance = if point.1 < client.top {
            point.1 - client.top
        } else if point.1 >= client.bottom {
            point.1 - client.bottom + 1
        } else {
            return None;
        };
        let line = state.layout.fonts().line_height;
        let step = (distance as f32 / dpi_scale(hwnd)).clamp(-4.0 * line, 4.0 * line);
        let total = state.layout.total_height();
        state.scroll_y = scroll::clamp(state.scroll_y + step, height, total);
        let position = position_at(state, point);
        extend_to(state, position);
        Some(())
    })
    .flatten();
    if scrolled.is_none() {
        unsafe { KillTimer(hwnd, AUTOSCROLL_TIMER) };
        return;
    }
    invalidate(hwnd);
    post_changed(hwnd, true);
}

/// `WM_LBUTTONUP` and `WM_CAPTURECHANGED`: the drag ends.
pub(super) fn release(hwnd: HWND, button_up: bool) {
    let dragging = with_state(hwnd, |state| state.drag.take().is_some()).unwrap_or(false);
    unsafe { KillTimer(hwnd, AUTOSCROLL_TIMER) };
    if dragging && button_up {
        unsafe { ReleaseCapture() };
    }
}
