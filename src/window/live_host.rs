//! Live Markdown and the Markdown writing helpers in the window layer (live mode spec §4, §8).

use crate::document::Language;
use crate::editor::markdown_edit::{insert_link, toggle_marker};
use crate::window::commands::CommandId;
use crate::window::main_window as host_window;
use windows_sys::Win32::Foundation::HWND;

/// Whether the active document is shown in Live Markdown.
pub(crate) fn is_live(_hwnd: HWND) -> bool {
    false
}

/// The format commands (live mode spec §8.1): toggle a marker around each selection, or insert
/// a link, as one undo step.
pub(crate) fn format(hwnd: HWND, command: CommandId) {
    if host_window::active_language(hwnd) != Language::Markdown {
        return;
    }
    host_window::with_editor(hwnd, |editor| {
        let Ok(selections) = editor.selections() else {
            return;
        };
        let plan = editor.with_document_text(|text| match command {
            CommandId::MarkdownBold => toggle_marker(text, &selections, "**"),
            CommandId::MarkdownItalic => toggle_marker(text, &selections, "*"),
            CommandId::MarkdownCode => toggle_marker(text, &selections, "`"),
            _ => insert_link(text, &selections),
        });
        if let Ok(plan) = plan {
            let _ = editor.apply_plan(&plan);
        }
    });
}
