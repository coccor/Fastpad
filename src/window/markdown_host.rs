//! The Markdown writing helpers in the window layer (Markdown design spec §8): the format
//! commands, Enter and Tab in Markdown documents, and tables formatted when the caret leaves them.

use crate::document::{DocumentId, Language};
use crate::editor::markdown_edit::{
    EditPlan, FenceCache, MARKDOWN_HELPER_MAX_BYTES, enter_in_list, format_table,
    in_literal_block_cached, indent_list_item, insert_link, is_list_item, line_bounds, next_cell,
    table_at, toggle_marker,
};
use crate::editor::scintilla_constants::{SC_MOD_DELETETEXT, SC_PERFORMED_REDO, SC_PERFORMED_UNDO};
use crate::editor::{Editor, EditorHooks, ScintillaNotification};
use crate::window::commands::CommandId;
use crate::window::main_window as host_window;
use crate::window::split_tree::GroupId;
use std::collections::HashMap;
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetFocus, VK_RETURN, VK_TAB};
use windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW;

/// The format commands (spec §8.1): toggle a marker around each selection, or insert
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

/// The helpers' per-document state, and the groups waiting for deferred work.
#[derive(Default)]
pub(crate) struct MarkdownRegistry {
    docs: HashMap<DocumentId, HelperState>,
    /// Groups whose selection changed since the deferred message was posted.
    pending: Vec<GroupId>,
    posted: bool,
}

impl std::fmt::Debug for MarkdownRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MarkdownRegistry")
            .field("docs", &self.docs)
            .field("pending", &self.pending)
            .field("posted", &self.posted)
            .finish()
    }
}

#[derive(Default)]
pub(crate) struct HelperState {
    /// A line of a table the user typed in; the table is formatted when the caret leaves it.
    /// Kept on its row as lines are added or removed above it.
    dirty_line: Option<usize>,
    /// Set while FastPad itself rewrites a table, so that edit does not mark it dirty again.
    formatting: bool,
    /// Where fenced code blocks stand, so the block check need not rescan the document.
    fences: FenceCache,
}

impl std::fmt::Debug for HelperState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HelperState")
            .field("dirty_line", &self.dirty_line)
            .field("formatting", &self.formatting)
            .field("fences", &self.fences)
            .finish()
    }
}

/// Whether `at` is in fenced code or front matter, resuming from `document`'s fence states.
fn in_literal_block(hwnd: HWND, document: DocumentId, text: &str, at: usize) -> bool {
    let mut cache = with_registry(hwnd, |registry| {
        registry
            .docs
            .get_mut(&document)
            .map(|state| std::mem::take(&mut state.fences))
    })
    .flatten()
    .unwrap_or_default();
    let literal = in_literal_block_cached(text, at, &mut cache);
    with_registry(hwnd, |registry| {
        registry.docs.entry(document).or_default().fences = cache;
    });
    literal
}

/// Drops what the helpers remember about `document` (its dirty table and fence states): it
/// closed, or its text or language changed without SCN_MODIFIED reaching `text_changed` (a
/// reload, a background Replace, a language change).
pub(crate) fn forget(hwnd: HWND, document: DocumentId) {
    with_registry(hwnd, |registry| registry.docs.remove(&document));
}

fn with_registry<R>(hwnd: HWND, run: impl FnOnce(&mut MarkdownRegistry) -> R) -> Option<R> {
    // SAFETY: the App pointer is used only inside `run`, which makes no Win32 call.
    unsafe { host_window::app_ptr(hwnd) }.map(|mut app| run(&mut unsafe { app.as_mut() }.markdown))
}

/// Whether the helpers hold any state for `document`.
#[cfg(test)]
pub(crate) fn has_state(hwnd: HWND, document: DocumentId) -> bool {
    with_registry(hwnd, |registry| registry.docs.contains_key(&document)).unwrap_or(false)
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
/// cells, in Markdown documents only (spec §8.2, §8.3).
#[derive(Debug)]
pub(crate) struct GroupHooks {
    main: HWND,
    editor: HWND,
}

impl GroupHooks {
    pub(crate) fn new(main: HWND, editor: HWND) -> Self {
        Self { main, editor }
    }

    fn markdown_editor(&self) -> Option<(Editor, DocumentId)> {
        let (_, editor, document, language) = group_of_editor(self.main, self.editor)?;
        (language == Language::Markdown).then_some((editor, document))
    }
}

impl EditorHooks for GroupHooks {
    fn key_down(&self, vk: u16, ctrl: bool, shift: bool, alt: bool) -> bool {
        if ctrl || alt || !(vk == VK_RETURN || vk == VK_TAB) {
            return false;
        }
        let Some((editor, document)) = self.markdown_editor() else {
            return false;
        };
        if editor
            .length()
            .map_or(true, |length| length > MARKDOWN_HELPER_MAX_BYTES)
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
        // The caret's line alone rules the helpers out, so a plain Enter or Tab in prose never
        // reads the document.
        let line_of = |at: usize| editor.line_from_position(at).ok();
        let Some(line_text) = line_of(caret).and_then(|line| editor.line_text(line).ok()) else {
            return false;
        };
        let list = is_list_item(&line_text);
        if vk == VK_RETURN {
            if shift || !selection.is_empty() || !list {
                return false;
            }
        } else if line_of(selection.start) != line_of(selection.end) {
            return false; // a multi-line selection: Scintilla indents the lines
        } else if !list && !line_text.contains('|') {
            return false;
        }
        let fallback = editor.eol().unwrap_or("\r\n");
        if fallback == "\r" {
            return false; // the helpers split lines on LF only
        }
        let plan = editor.with_document_text(|text| {
            // A lone CR on the caret's line: a CR-only document, read as one line by the helpers.
            if splits_on_cr(text, caret) {
                return None;
            }
            let plan = if vk == VK_RETURN {
                enter_in_list(text, caret, fallback)
            } else if let Some(cell) = next_cell(text, caret, shift) {
                Some(EditPlan {
                    edits: Vec::new(),
                    selections: vec![cell],
                })
            } else {
                // A caret or a selection on one list item's line nests the item (spec §8.2).
                indent_list_item(text, caret, shift).or_else(|| {
                    // No cell to move to (the last one, or the delimiter row): Tab never puts a
                    // tab character into a table.
                    // The selection is kept as it was, its caret end included.
                    let kept = if caret == selection.start {
                        selection.end..selection.start
                    } else {
                        selection.clone()
                    };
                    table_at(text, caret).map(|_| EditPlan {
                        edits: Vec::new(),
                        selections: vec![kept],
                    })
                })
            };
            // Fenced code and front matter keep the default keys; checked only with a plan.
            plan.filter(|_| !in_literal_block(self.main, document, text, caret))
        });
        match plan {
            Ok(Some(plan)) => editor.apply_plan(&plan).is_ok(),
            _ => false,
        }
    }
}

/// SCN_MODIFIED for a Markdown document (spec §8.3): keeps a dirty table's line on
/// its row, and remembers a table the user typed in at the focused editor's caret. Undo, redo,
/// Replace All and other views' edits are not typing. Reads only the edited line, never the
/// whole document, and never edits inside the notification.
pub(crate) fn text_changed(
    hwnd: HWND,
    editor: &Editor,
    document: DocumentId,
    modification: &ScintillaNotification,
) {
    let Ok(length) = editor.length() else {
        return;
    };
    let position = modification.position.max(0) as usize;
    let delta = if modification.modification_type as u32 & SC_MOD_DELETETEXT != 0 {
        -modification.length
    } else {
        modification.length
    };
    let formatting = with_registry(hwnd, |registry| {
        registry.docs.get_mut(&document).is_some_and(|state| {
            state.fences.edited(position, delta, length);
            if length > MARKDOWN_HELPER_MAX_BYTES {
                state.dirty_line = None; // past the limit no table is tracked
            }
            state.formatting
        })
    });
    if formatting != Some(false) || length > MARKDOWN_HELPER_MAX_BYTES {
        return;
    }
    let Ok(line) = editor.line_from_position(position) else {
        return;
    };
    let lines_added = modification.lines_added;
    if lines_added != 0 {
        // Lines inserted at the very start of a row push that row down too.
        let at_line_start = editor.line_start(line).is_ok_and(|start| start == position);
        with_registry(hwnd, |registry| {
            if let Some(dirty) = registry
                .docs
                .get_mut(&document)
                .and_then(|state| state.dirty_line.as_mut())
                && (line < *dirty || (line == *dirty && lines_added > 0 && at_line_start))
            {
                *dirty = (*dirty as isize + lines_added).max(line as isize) as usize;
            }
        });
    }
    let performed = SC_PERFORMED_UNDO | SC_PERFORMED_REDO;
    if modification.modification_type as u32 & performed != 0 {
        return;
    }
    if !typed_at_caret(hwnd, document, line, lines_added.max(0) as usize) {
        return;
    }
    if editor.line_text(line).is_ok_and(|text| text.contains('|')) {
        with_registry(hwnd, |registry| {
            registry.docs.entry(document).or_default().dirty_line = Some(line);
        });
    }
}

/// Whether an edit on `line` (adding `lines_added`) is typing: the focused editor shows
/// `document` and its main caret is on the edited lines.
fn typed_at_caret(hwnd: HWND, document: DocumentId, line: usize, lines_added: usize) -> bool {
    let focus = unsafe { GetFocus() };
    let Some((_, editor, shown, _)) = group_of_editor(hwnd, focus) else {
        return false;
    };
    shown == document
        && editor
            .caret()
            .and_then(|caret| editor.line_from_position(caret))
            .is_ok_and(|caret_line| (line..=line + lines_added).contains(&caret_line))
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
                crate::window::messages::WM_FASTPAD_MARKDOWN_DEFERRED,
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

/// Whether every line from the caret's to the dirty one is a table row, so the caret is still
/// in the edited table. Walks from the caret, so a caret far away stops at once.
fn still_in_table(editor: &Editor, dirty: usize) -> crate::Result<bool> {
    let from = editor.line_from_position(editor.caret()?)?;
    let lines: Box<dyn Iterator<Item = usize>> = if from <= dirty {
        Box::new(from..=dirty)
    } else {
        Box::new((dirty..=from).rev())
    };
    for line in lines {
        if !editor.line_text(line)?.contains('|') {
            return Ok(false);
        }
    }
    Ok(true)
}

/// The editors of every group whose active view shows `document`.
fn editors_showing(hwnd: HWND, document: DocumentId) -> Vec<Editor> {
    // SAFETY: the App reference lives only inside this function, which makes no Win32 call.
    let Some(app) = (unsafe { host_window::app_ptr(hwnd) }) else {
        return Vec::new();
    };
    let app = unsafe { app.as_ref() };
    app.groups
        .iter()
        .filter(|group| {
            app.tabs
                .group(group.id)
                .is_some_and(|tabs| tabs.active_document() == Some(document))
        })
        .map(|group| group.editor.clone())
        .collect()
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
            .and_then(|state| state.dirty_line)
    });
    let Some(dirty) = dirty.flatten() else {
        return;
    };
    if language != Language::Markdown {
        clear_dirty(hwnd, document);
        return;
    }
    // Every view of the document counts: its table waits while any caret is still in it.
    for view in editors_showing(hwnd, document) {
        match still_in_table(&view, dirty) {
            Ok(true) => return,
            Ok(false) => {}
            Err(_) => {
                clear_dirty(hwnd, document);
                return;
            }
        }
    }
    clear_dirty(hwnd, document);
    let Ok(start) = editor.line_start(dirty) else {
        return;
    };
    let rewrite = editor.with_document_text(|text| {
        let start = start.min(text.len());
        if splits_on_cr(text, start) {
            return None; // a CR-only document: the helpers would read it as one line
        }
        let table = table_at(text, start)?;
        if in_literal_block(hwnd, document, text, table.start) {
            return None; // a table-like block in fenced code or front matter is left alone
        }
        format_table(&text[table.clone()]).map(|formatted| (table, formatted))
    });
    if let Ok(Some((table, formatted))) = rewrite {
        set_formatting(hwnd, document, true);
        editor.begin_undo_action();
        let _ = editor.replace_target(table, &formatted);
        editor.end_undo_action();
        set_formatting(hwnd, document, false);
    }
}

/// Whether the line around `at`, split on LF as the helpers split lines, holds a lone CR: the
/// document (or that part of it) ends its lines with CR alone. Reads that one LF-delimited span.
fn splits_on_cr(text: &str, at: usize) -> bool {
    text[line_bounds(text, at)].contains('\r')
}

fn set_formatting(hwnd: HWND, document: DocumentId, formatting: bool) {
    with_registry(hwnd, |registry| {
        registry.docs.entry(document).or_default().formatting = formatting;
    });
}

fn clear_dirty(hwnd: HWND, document: DocumentId) {
    with_registry(hwnd, |registry| {
        if let Some(state) = registry.docs.get_mut(&document) {
            state.dirty_line = None;
        }
    });
}
