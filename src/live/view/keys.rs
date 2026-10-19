//! Keyboard input in the Live view (phase 1 plan, Task 8). Keys that move over wrapped rows
//! (Up/Down, PgUp/PgDn, Home/End) move the caret here, over the Live layout; every other key and
//! every character goes to the group's Scintilla through FastPad's subclass, so typing, the
//! writing helpers, undo grouping and Scintilla's own keys behave exactly as in the source view.

use super::*;
use crate::live::motion::{self, Move};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, VK_CONTROL, VK_DOWN, VK_END, VK_HOME, VK_MENU, VK_NEXT, VK_PRIOR, VK_SHIFT, VK_UP,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetCaretBlinkTime, KillTimer, SetTimer, WM_CHAR, WM_KEYDOWN, WM_KEYUP,
};

/// The caret blink timer's id.
pub(super) const CARET_TIMER: usize = 1;

fn key_down(key: u16) -> bool {
    (unsafe { GetKeyState(i32::from(key)) }) < 0
}

/// The row movement `vk` asks for, when it is one the Live view makes itself: plain or with
/// Shift, never with Ctrl or Alt.
fn row_move(vk: u16) -> Option<Move> {
    if key_down(VK_CONTROL) || key_down(VK_MENU) {
        return None;
    }
    Some(match vk {
        VK_UP => Move::Up,
        VK_DOWN => Move::Down,
        VK_PRIOR => Move::PageUp,
        VK_NEXT => Move::PageDown,
        VK_HOME => Move::RowStart,
        VK_END => Move::RowEnd,
        _ => return None,
    })
}

/// `WM_KEYDOWN`, `WM_KEYUP` and `WM_CHAR`. Returns `None` for messages it leaves to
/// `DefWindowProcW`.
pub(super) fn handle(hwnd: HWND, message: u32, wparam: usize, lparam: isize) -> Option<isize> {
    let editor = with_state(hwnd, |state| {
        state.goal_reset_unless_vertical(message, wparam);
        state.editor.clone()
    })?;
    match message {
        WM_KEYDOWN => {
            if let Some(movement) = row_move(wparam as u16) {
                with_state(hwnd, |state| {
                    move_caret(state, movement, key_down(VK_SHIFT))
                });
            } else {
                editor.forward_key(message, wparam, lparam);
            }
        }
        WM_KEYUP => {
            editor.forward_key(message, wparam, lparam);
        }
        WM_CHAR => {
            editor.forward_char(wparam, lparam);
        }
        _ => return None,
    }
    // What changed repaints from the edit and selection notifications; the caret shows now.
    restart_blink(hwnd);
    post_changed(hwnd, false);
    Some(0)
}

impl ViewState {
    /// Any key but a vertical move ends a run of Up/Down.
    fn goal_reset_unless_vertical(&mut self, message: u32, wparam: usize) {
        let vertical = message == WM_KEYDOWN
            && matches!(
                row_move(wparam as u16),
                Some(Move::Up | Move::Down | Move::PageUp | Move::PageDown)
            );
        if !vertical && message != WM_KEYUP {
            self.goal = None;
        }
    }
}

fn move_caret(state: &mut ViewState, movement: Move, extend: bool) {
    prepare(state);
    let (_, height) = view_size(state.hwnd);
    let ViewState {
        editor,
        layout,
        document,
        revealed,
        goal,
        scroll_y,
        pictures,
        ..
    } = state;
    let source = EditorText(editor);
    // Rows as drawn now: a target on a rendered line is hit-tested rendered, and the reveal
    // that follows keeps the caret where it landed (phase 2 plan, Decision 1).
    let text = Styled {
        text: &source,
        document,
        revealed: revealed.clone(),
        images: &|dest: &str| pictures.resolve(dest),
    };
    let (anchor, caret) = text.selection();
    let goal_x = goal.filter(|&(_, at)| at == caret).map(|(x, _)| x);
    let before = layout.caret_rect(&text, caret).ok();
    let moved = {
        let mut geometry = layout.geometry(&text, height);
        motion::apply(&mut geometry, caret, goal_x, movement)
    };
    *goal = moved.goal_x.map(|x| (x, moved.caret));
    text.set_selection(if extend { anchor } else { moved.caret }, moved.caret);
    // A page move scrolls the view by as much as the caret moved, as editors do.
    if matches!(movement, Move::PageUp | Move::PageDown)
        && let (Some(before), Ok(after)) = (before, layout.caret_rect(&text, moved.caret))
    {
        let total = layout.total_height();
        *scroll_y = scroll::clamp(*scroll_y + (after.top - before.top), height, total);
    }
    scroll_to_caret(state);
}

/// Shows the caret and starts its blink over, as every key does.
pub(super) fn restart_blink(hwnd: HWND) {
    let focused = with_state(hwnd, |state| {
        if !std::mem::replace(&mut state.caret_on, true)
            && let Some((top, bottom)) = state.caret_span
        {
            invalidate_span(state, top, bottom);
        }
        state.focused
    })
    .unwrap_or(false);
    let blink = unsafe { GetCaretBlinkTime() };
    if focused && blink != u32::MAX && blink != 0 {
        unsafe { SetTimer(hwnd, CARET_TIMER, blink, None) };
    } else {
        unsafe { KillTimer(hwnd, CARET_TIMER) };
    }
}

/// `WM_TIMER` for the caret blink.
pub(super) fn blink(hwnd: HWND) {
    with_state(hwnd, |state| {
        state.caret_on = !state.caret_on;
        if let Some((top, bottom)) = state.caret_span {
            invalidate_span(state, top, bottom);
        }
    });
}
