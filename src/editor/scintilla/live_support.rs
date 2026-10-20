//! `Editor`'s operations for the Live Markdown view (2026-10-18 Live editor design spec §7.2):
//! the selection with its direction, word ranges for double-click, the installed hooks, and
//! forwarding the Live view's typing to Scintilla's own input path.

use super::*;
use crate::editor::scintilla_constants::{
    SCI_GETANCHOR, SCI_GETCURRENTPOS, SCI_GETSELECTIONS, SCI_SELECTIONISRECTANGLE,
    SCI_WORDENDPOSITION, SCI_WORDSTARTPOSITION,
};
use std::ops::Range;

impl Editor {
    fn live_send(&self, message: u32, wparam: usize, lparam: isize) -> Result<isize> {
        self.endpoint.send_direct_checked(message, wparam, lparam)
    }

    /// The main selection as (anchor, caret), keeping its direction.
    pub fn anchor_and_caret(&self) -> Result<(usize, usize)> {
        let anchor = self.live_send(SCI_GETANCHOR, 0, 0)?.max(0) as usize;
        let caret = self.live_send(SCI_GETCURRENTPOS, 0, 0)?.max(0) as usize;
        Ok((anchor, caret))
    }

    /// How many selections Scintilla holds; 1 for a plain caret.
    pub fn selection_count(&self) -> Result<usize> {
        Ok(self.live_send(SCI_GETSELECTIONS, 0, 0)?.max(1) as usize)
    }

    /// The selection is a rectangle (Alt+Shift+arrows, Alt+drag), even one row tall.
    pub fn is_rectangular_selection(&self) -> Result<bool> {
        Ok(self.live_send(SCI_SELECTIONISRECTANGLE, 0, 0)? != 0)
    }

    /// The word around `position` by Scintilla's word rules; empty when `position` is not in or
    /// next to a word.
    pub fn word_range_at(&self, position: usize) -> Result<Range<usize>> {
        let start = self.live_send(SCI_WORDSTARTPOSITION, position, 1)?.max(0) as usize;
        let end = self.live_send(SCI_WORDENDPOSITION, position, 1)?.max(0) as usize;
        Ok(start..end.max(start))
    }
}

// The Live view's window (phase 1 plan, Task 6) is the first caller outside tests.
#[allow(dead_code)]
impl Editor {
    /// The hooks installed with `set_hooks`.
    pub(crate) fn hooks(&self) -> Option<Rc<dyn crate::editor::EditorHooks>> {
        self.endpoint.hooks.borrow().clone()
    }

    /// Sends a `WM_CHAR` to the Scintilla window, through FastPad's subclass, as if typed there.
    pub(crate) fn forward_char(&self, wparam: usize, lparam: isize) -> isize {
        self.forward(
            windows_sys::Win32::UI::WindowsAndMessaging::WM_CHAR,
            wparam,
            lparam,
        )
    }

    /// Sends a key message (`WM_KEYDOWN`, `WM_KEYUP`, `WM_SYSKEYDOWN`, `WM_SYSKEYUP`) to the
    /// Scintilla window, through FastPad's subclass and its hooks.
    pub(crate) fn forward_key(&self, message: u32, wparam: usize, lparam: isize) -> isize {
        self.forward(message, wparam, lparam)
    }

    /// Answers `WM_GETTEXT` / `WM_GETTEXTLENGTH` with Scintilla's own answer.
    pub(crate) fn forward_get_text(&self, message: u32, wparam: usize, lparam: isize) -> isize {
        self.forward(message, wparam, lparam)
    }

    #[cfg(windows)]
    fn forward(&self, message: u32, wparam: usize, lparam: isize) -> isize {
        if self.endpoint.destroyed.load(Ordering::Acquire) || self.endpoint.hwnd.is_null() {
            return 0;
        }
        unsafe { SendMessageW(self.endpoint.hwnd, message, wparam, lparam) }
    }

    #[cfg(not(windows))]
    fn forward(&self, message: u32, wparam: usize, lparam: isize) -> isize {
        let _ = (message, wparam, lparam);
        0
    }
}
