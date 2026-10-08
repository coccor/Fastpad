//! Callbacks the editor subclass makes into the window layer: the Markdown helpers take Enter
//! and Tab (2026-10-06 Markdown design spec §8). The editor knows nothing about documents, so
//! this is the only way in.

/// The subclass calls these while it handles the editor's own window message, and an
/// implementation may call back into the editor (edit text, set the selection). So an
/// implementer must not hold a `RefCell` borrow (or any other exclusive access) across a call
/// into the editor window: that call can re-enter the hook or the window layer.
pub trait EditorHooks: std::fmt::Debug {
    /// A `WM_KEYDOWN` no accelerator took; true consumes it and the `WM_CHAR` that follows.
    fn key_down(&self, _vk: u16, _ctrl: bool, _shift: bool, _alt: bool) -> bool {
        false
    }
}
