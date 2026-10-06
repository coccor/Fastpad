//! Live Markdown and the Markdown writing helpers in the window layer (live mode spec §4, §8).

use windows_sys::Win32::Foundation::HWND;

/// Whether the active document is shown in Live Markdown.
pub(crate) fn is_live(_hwnd: HWND) -> bool {
    false
}
