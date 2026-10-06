//! Callbacks the editor subclass makes into the window layer (live mode spec §7): Live Markdown
//! paints decorations after Scintilla and takes checkbox and link clicks; the Markdown helpers
//! take Enter and Tab. The editor knows nothing about documents, so this is the only way in.

use windows_sys::Win32::Foundation::{HWND, RECT};

/// The subclass calls these while it handles the editor's own window message, and an
/// implementation may call back into the editor (edit text, set the selection). So an
/// implementer must not hold a `RefCell` borrow (or any other exclusive access) across a call
/// into the editor window: that call can re-enter the hook or the window layer.
pub trait EditorHooks: std::fmt::Debug {
    /// After Scintilla painted `update` (client coordinates).
    fn after_paint(&self, _hwnd: HWND, _update: RECT) {}
    /// A left-button press; true consumes it (Scintilla never sees it).
    fn mouse_down(&self, _x: i32, _y: i32, _ctrl: bool) -> bool {
        false
    }
    /// `WM_SETCURSOR` over the text area; true means the hook set the cursor.
    fn set_cursor(&self, _x: i32, _y: i32, _ctrl: bool) -> bool {
        false
    }
    /// A `WM_KEYDOWN` no accelerator took; true consumes it and the `WM_CHAR` that follows.
    fn key_down(&self, _vk: u16, _ctrl: bool, _shift: bool, _alt: bool) -> bool {
        false
    }
}
