//! Live Markdown and the Markdown writing helpers in the window layer (live mode spec §4, §8).

use crate::document::{DocumentId, Language};
use crate::editor::markdown_edit::{
    EditPlan, enter_in_list, format_table, in_literal_block, indent_list_item, insert_link,
    line_bounds, next_cell, table_at, toggle_marker,
};
use crate::editor::{Editor, EditorHooks};
use crate::live::LIVE_MAX_BYTES;
use crate::window::commands::CommandId;
use crate::window::main_window as host_window;
use crate::window::split_tree::GroupId;
use std::collections::HashMap;
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_RETURN, VK_TAB};
use windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW;

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

/// Per-document Live Markdown state, and the groups waiting for deferred work.
#[derive(Default)]
pub(crate) struct LiveRegistry {
    docs: HashMap<DocumentId, DocState>,
    /// Groups whose selection changed since the deferred message was posted.
    pending: Vec<GroupId>,
    posted: bool,
}

impl std::fmt::Debug for LiveRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveRegistry")
            .field("docs", &self.docs)
            .field("pending", &self.pending)
            .field("posted", &self.posted)
            .finish()
    }
}

#[derive(Default)]
pub(crate) struct DocState {
    /// A byte inside a table the user edited; the table is formatted when the caret leaves it.
    dirty_table: Option<usize>,
    /// Set while FastPad itself rewrites a table, so that edit does not mark it dirty again.
    formatting: bool,
}

impl std::fmt::Debug for DocState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DocState")
            .field("dirty_table", &self.dirty_table)
            .field("formatting", &self.formatting)
            .finish()
    }
}

fn with_registry<R>(hwnd: HWND, run: impl FnOnce(&mut LiveRegistry) -> R) -> Option<R> {
    // SAFETY: the App pointer is used only inside `run`, which makes no Win32 call.
    unsafe { host_window::app_ptr(hwnd) }.map(|mut app| run(&mut unsafe { app.as_mut() }.live))
}

/// The group whose editor window is `editor`, its editor and active document.
pub(crate) fn group_of_editor(
    hwnd: HWND,
    editor: HWND,
) -> Option<(GroupId, Editor, DocumentId, Language)> {
    // SAFETY: the App reference lives only inside this function, which makes no Win32 call.
    let app = unsafe { host_window::app_ptr(hwnd)?.as_ref() };
    let group = app
        .groups
        .iter()
        .find(|group| group.editor.hwnd() == editor)?;
    let document = app.tabs.group(group.id)?.active_document()?;
    let language = app.tabs.document(document)?.language;
    Some((group.id, group.editor.clone(), document, language))
}

/// Every group editor's hooks: Enter continues lists, Tab nests list items and walks table
/// cells, in Markdown documents only (live mode spec §8.2, §8.3).
#[derive(Debug)]
pub(crate) struct GroupHooks {
    main: HWND,
    editor: HWND,
}

impl GroupHooks {
    pub(crate) fn new(main: HWND, editor: HWND) -> Self {
        Self { main, editor }
    }

    fn markdown_editor(&self) -> Option<Editor> {
        let (_, editor, _, language) = group_of_editor(self.main, self.editor)?;
        (language == Language::Markdown).then_some(editor)
    }
}

impl EditorHooks for GroupHooks {
    fn key_down(&self, vk: u16, ctrl: bool, shift: bool, alt: bool) -> bool {
        if ctrl || alt || !(vk == VK_RETURN || vk == VK_TAB) {
            return false;
        }
        let Some(editor) = self.markdown_editor() else {
            return false;
        };
        if editor
            .length()
            .map_or(true, |length| length > LIVE_MAX_BYTES)
        {
            return false;
        }
        let (Ok(selections), Ok(carets)) = (editor.selections(), editor.carets()) else {
            return false;
        };
        let ([selection], [caret]) = (selections.as_slice(), carets.as_slice()) else {
            return false; // multiple carets: Scintilla's own Enter / Tab
        };
        let caret = *caret;
        let fallback = editor.eol().unwrap_or("\r\n");
        let plan = editor.with_document_text(|text| {
            if in_literal_block(text, caret) {
                return None; // fenced code and front matter keep the default keys
            }
            if vk == VK_RETURN {
                if shift || !selection.is_empty() {
                    return None;
                }
                return enter_in_list(text, caret, fallback);
            }
            if line_bounds(text, selection.start) != line_bounds(text, selection.end) {
                return None; // a multi-line selection: Scintilla indents the lines
            }
            if let Some(cell) = next_cell(text, caret, shift) {
                return Some(EditPlan {
                    edits: Vec::new(),
                    selections: vec![cell],
                });
            }
            // A caret or a selection on one list item's line nests the item (spec §8.2).
            indent_list_item(text, caret, shift)
        });
        match plan {
            Ok(Some(plan)) => editor.apply_plan(&plan).is_ok(),
            _ => false,
        }
    }
}

/// SCN_MODIFIED for a Markdown document (live mode spec §8.3): remembers a table the user
/// edited. Reads only the edited line, never the whole document, and never edits inside the
/// notification.
pub(crate) fn text_changed(hwnd: HWND, editor: &Editor, document: DocumentId, position: usize) {
    let formatting = with_registry(hwnd, |registry| {
        registry
            .docs
            .get(&document)
            .is_some_and(|state| state.formatting)
    });
    if formatting != Some(false) {
        return;
    }
    if editor
        .length()
        .map_or(true, |length| length > LIVE_MAX_BYTES)
    {
        return;
    }
    let row = editor
        .line_from_position(position)
        .and_then(|line| editor.line_text(line))
        .is_ok_and(|line| line.contains('|'));
    if row {
        with_registry(hwnd, |registry| {
            registry.docs.entry(document).or_default().dirty_table = Some(position);
        });
    }
}

/// SCN_UPDATEUI with a selection change: the work runs from a posted message, never inside
/// Scintilla's notification.
pub(crate) fn selection_changed(hwnd: HWND, group: GroupId) {
    let post = with_registry(hwnd, |registry| {
        if !registry.pending.contains(&group) {
            registry.pending.push(group);
        }
        !std::mem::replace(&mut registry.posted, true)
    });
    if post == Some(true) {
        unsafe {
            PostMessageW(
                hwnd,
                crate::window::messages::WM_FASTPAD_LIVE_DEFERRED,
                0,
                0,
            )
        };
    }
}

pub(crate) fn run_deferred(hwnd: HWND) {
    let groups = with_registry(hwnd, |registry| {
        registry.posted = false;
        std::mem::take(&mut registry.pending)
    })
    .unwrap_or_default();
    for group in groups {
        format_left_table(hwnd, group);
    }
}

/// Whether every line from the caret's to the dirty byte's is a table row, so the caret is
/// still in the edited table. Walks from the caret, so a caret far away stops at once.
fn still_in_table(editor: &Editor, caret: usize, dirty: usize) -> crate::Result<bool> {
    let from = editor.line_from_position(caret)?;
    let to = editor.line_from_position(dirty)?;
    let lines: Box<dyn Iterator<Item = usize>> = if from <= to {
        Box::new(from..=to)
    } else {
        Box::new((to..=from).rev())
    };
    for line in lines {
        if !editor.line_text(line)?.contains('|') {
            return Ok(false);
        }
    }
    Ok(true)
}

fn format_left_table(hwnd: HWND, group: GroupId) {
    let Some(editor) = host_window::group_editor(hwnd, group) else {
        return;
    };
    let Some((_, _, document, language)) = group_of_editor(hwnd, editor.hwnd()) else {
        return;
    };
    let dirty = with_registry(hwnd, |registry| {
        registry
            .docs
            .get(&document)
            .and_then(|state| state.dirty_table)
    });
    let Some(dirty) = dirty.flatten() else {
        return;
    };
    if language != Language::Markdown {
        clear_dirty(hwnd, document);
        return;
    }
    let Ok(caret) = editor
        .carets()
        .map(|carets| carets.first().copied().unwrap_or(0))
    else {
        return;
    };
    match still_in_table(&editor, caret, dirty) {
        Ok(true) => return, // still inside: keep waiting
        Ok(false) => {}
        Err(_) => {
            clear_dirty(hwnd, document);
            return;
        }
    }
    let rewrite = editor.with_document_text(|text| {
        let table = table_at(text, dirty.min(text.len()))?;
        if in_literal_block(text, table.start) {
            return None; // a table-like block in fenced code or front matter is left alone
        }
        format_table(&text[table.clone()]).map(|formatted| (table, formatted))
    });
    clear_dirty(hwnd, document);
    if let Ok(Some((table, formatted))) = rewrite {
        set_formatting(hwnd, document, true);
        editor.begin_undo_action();
        let _ = editor.replace_target(table, &formatted);
        editor.end_undo_action();
        set_formatting(hwnd, document, false);
    }
}

fn set_formatting(hwnd: HWND, document: DocumentId, formatting: bool) {
    with_registry(hwnd, |registry| {
        registry.docs.entry(document).or_default().formatting = formatting;
    });
}

fn clear_dirty(hwnd: HWND, document: DocumentId) {
    with_registry(hwnd, |registry| {
        if let Some(state) = registry.docs.get_mut(&document) {
            state.dirty_table = None;
        }
    });
}
