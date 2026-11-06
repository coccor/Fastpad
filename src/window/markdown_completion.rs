//! The lists that complete what is typed in a Markdown note (Markdown typing speed, 2026-11-04):
//! `[[` notes to link to (second brain phase 1 spec §7.4), `#` tags the notebook uses, and `/`
//! commands at a line's start. FastPad ranks each list itself, best match first, and shows it
//! again after every typed or deleted character; in the Live view the list opens at Live's caret.

// A plan's selections are ranges; a one-caret plan is legitimately a one-element Vec of them.
#![allow(clippy::single_range_in_vec_init)]

use crate::editor::Editor;
use crate::editor::markdown_edit::{EditPlan, TextEdit, line_bounds};
use crate::editor::markdown_typing::{
    rank_names, rank_slash, slash_query, slash_snippet, tag_query, wiki_query,
};
use crate::window::main_window as host_window;
use windows_sys::Win32::Foundation::{HWND, LPARAM, POINT, RECT};

/// What an open list completes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Kind {
    Wiki,
    Tag,
    Slash,
}

/// The list open in one editor, and every item it can offer (ranked again on each keystroke).
#[derive(Debug)]
pub(crate) struct Completion {
    editor: HWND,
    kind: Kind,
    items: Vec<String>,
}

/// Wikilink and tag lists show at most this many of the best matches.
const LIMIT: usize = 50;

fn with_state<R>(hwnd: HWND, run: impl FnOnce(&mut Option<Completion>) -> R) -> Option<R> {
    crate::window::markdown_host::with_registry(hwnd, |registry| run(&mut registry.completion))
}

/// The kind of the list FastPad opened in `editor`, while one is open there.
fn open_kind(hwnd: HWND, editor: &Editor) -> Option<Kind> {
    if !editor.completion_active() {
        with_state(hwnd, |state| *state = None);
        return None;
    }
    with_state(hwnd, |state| {
        state
            .as_ref()
            .filter(|open| open.editor == editor.hwnd())
            .map(|open| open.kind)
    })
    .flatten()
}

/// The caret's line up to the caret, and where that line starts.
fn line_before_caret(editor: &Editor) -> Option<(String, usize)> {
    let caret = editor.caret().ok()?;
    let line = editor.line_from_position(caret).ok()?;
    let start = editor.line_start(line).ok()?;
    let bytes = editor.range_bytes(start..caret).ok()?;
    Some((String::from_utf8_lossy(bytes).into_owned(), start))
}

/// `SCN_CHARADDED` in a note: a second `[`, a `/` at a line's start, or a letter after a `#`
/// opens its list; with a list open, it is ranked again.
pub(crate) fn char_added(hwnd: HWND, editor: &Editor, character: i32) {
    if let Some(kind) = open_kind(hwnd, editor) {
        refresh(hwnd, editor, kind);
        return;
    }
    let Some(typed) = char::from_u32(character as u32) else {
        return;
    };
    let Some((before, _)) = line_before_caret(editor) else {
        return;
    };
    let kind = if typed == '[' && before.ends_with("[[") {
        Kind::Wiki
    } else if typed == '/' && slash_query(&before).is_some() {
        Kind::Slash
    } else if tag_query(&before).is_some_and(|start| before.len() - start == typed.len_utf8()) {
        Kind::Tag
    } else {
        return;
    };
    open(hwnd, editor, kind);
}

/// `SCN_AUTOCCHARDELETED`: Backspace inside the typed text ranks the list again.
pub(crate) fn char_deleted(hwnd: HWND, editor: &Editor) {
    if let Some(kind) = open_kind(hwnd, editor) {
        refresh(hwnd, editor, kind);
    }
}

/// `SCN_AUTOCCANCELLED`: the list closed without a choice.
pub(crate) fn cancelled(hwnd: HWND, editor: &Editor) {
    with_state(hwnd, |state| {
        if state
            .as_ref()
            .is_some_and(|open| open.editor == editor.hwnd())
        {
            *state = None;
        }
    });
}

/// Opens a `kind` list in `editor` with every item it can offer.
pub(crate) fn open(hwnd: HWND, editor: &Editor, kind: Kind) {
    let items = match kind {
        Kind::Wiki => {
            let from = unsafe { host_window::app_ptr(hwnd) }
                .and_then(|app| unsafe { app.as_ref() }.tabs.active()?.path.clone());
            crate::window::library_host::completion_items(hwnd, from.as_deref())
        }
        Kind::Tag => {
            let from = unsafe { host_window::app_ptr(hwnd) }
                .and_then(|app| unsafe { app.as_ref() }.tabs.active()?.path.clone());
            tag_names(hwnd, from.as_deref())
        }
        Kind::Slash => Vec::new(),
    };
    with_state(hwnd, |state| {
        *state = Some(Completion {
            editor: editor.hwnd(),
            kind,
            items,
        });
    });
    refresh(hwnd, editor, kind);
}

/// Every tag in the index of the notebook that owns the note at `from` (the default notebook
/// for an untitled tab), by its full name (`project/fastpad`), sorted.
fn tag_names(hwnd: HWND, from: Option<&std::path::Path>) -> Vec<String> {
    fn walk(nodes: &[crate::library::index::TagNode], names: &mut Vec<String>) {
        for node in nodes {
            names.push(node.full.clone());
            walk(&node.children, names);
        }
    }
    let mut names = Vec::new();
    let index = match from {
        Some(from) => crate::window::library_host::note_index_for(hwnd, from),
        // An untitled tab's links resolve in the default notebook (`link_notebook`).
        None => crate::window::library_host::link_notebook(hwnd, None)
            .map(|id| crate::window::library_host::note_index_of(hwnd, id))
            .unwrap_or_default(),
    };
    walk(&index.tags(), &mut names);
    names.sort_by(|a, b| crate::library::tree::natural_cmp(a, b));
    names.dedup();
    names
}

/// The open list's query (the text typed since it opened), or `None` when the caret has left
/// what it completes.
fn query(kind: Kind, before: &str) -> Option<usize> {
    match kind {
        Kind::Wiki => wiki_query(before),
        Kind::Tag => tag_query(before),
        Kind::Slash => slash_query(before).map(|slash| slash + 1),
    }
}

/// Ranks the open list for what is typed now and shows it, or closes it when nothing matches.
fn refresh(hwnd: HWND, editor: &Editor, kind: Kind) {
    let ranked = line_before_caret(editor).and_then(|(before, _)| {
        let start = query(kind, &before)?;
        let typed = &before[start..];
        let ranked = match kind {
            Kind::Slash => rank_slash(typed).into_iter().map(String::from).collect(),
            Kind::Tag => with_state(hwnd, |state| {
                rank_names(
                    state.as_ref().map_or(&[][..], |open| &open.items),
                    typed,
                    LIMIT,
                )
            })?,
            Kind::Wiki => with_state(hwnd, |state| {
                rank_notes(state.as_ref().map_or(&[][..], |open| &open.items), typed)
            })?,
        };
        Some((typed.len(), ranked))
    });
    match ranked {
        Some((typed, ranked)) if !ranked.is_empty() => {
            let _ = editor.show_completion(typed, &ranked);
            place_in_live(hwnd, editor);
        }
        _ => {
            with_state(hwnd, |state| *state = None);
            let _ = editor.cancel_completion();
        }
    }
}

/// Link targets for `query`: the quick-open matcher's best (spec §7.4), or the first ones in
/// order before anything is typed.
fn rank_notes(items: &[String], query: &str) -> Vec<String> {
    if query.trim().is_empty() {
        return items.iter().take(LIMIT).cloned().collect();
    }
    crate::library::quick_open::search(items.iter().map(String::as_str), query, LIMIT)
        .into_iter()
        .map(|found| found.path.to_string_lossy().into_owned())
        .collect()
}

/// `SCN_AUTOCSELECTION`: a `/` command was picked. Scintilla's own insertion is cancelled and the
/// command's text replaces the `/` and what was typed after it, as one undo step.
pub(crate) fn selected(hwnd: HWND, editor: &Editor, text: *const u8) {
    if open_kind(hwnd, editor) != Some(Kind::Slash) || text.is_null() {
        return;
    }
    let label = unsafe { std::ffi::CStr::from_ptr(text.cast()) }
        .to_string_lossy()
        .into_owned();
    let _ = editor.cancel_completion();
    with_state(hwnd, |state| *state = None);
    let Some((before, line_start)) = line_before_caret(editor) else {
        return;
    };
    let Some(slash) = slash_query(&before) else {
        return;
    };
    let (date, time) = crate::platform::clock::local_now();
    let date = crate::library::dates::format(date, "YYYY-MM-DD");
    let time = format!("{:02}:{:02}", time.hour, time.minute);
    let eol = editor.eol().unwrap_or("\r\n");
    let plan = editor.with_document_text(|document| {
        let at = line_start + slash;
        // The line above has text: a divider must not underline it as a heading.
        let after_text = line_start > 0
            && !document[line_bounds(document, line_start - 1)]
                .trim()
                .is_empty();
        let (snippet, selection) = slash_snippet(&label, eol, &date, &time, after_text)?;
        Some(EditPlan {
            edits: vec![TextEdit {
                range: at..line_start + before.len(),
                text: snippet,
            }],
            selections: vec![at + selection.start..at + selection.end],
        })
    });
    if let Ok(Some(plan)) = plan
        && editor.apply_plan(&plan).is_ok()
        && label == "Note link"
    {
        open(hwnd, editor, Kind::Wiki);
    }
}

/// `SCN_AUTOCCOMPLETED`: a note or tag was picked; a link closes after its note unless it already
/// does.
pub(crate) fn completed(hwnd: HWND, editor: &Editor) {
    let kind = with_state(hwnd, |state| state.take().map(|open| open.kind)).flatten();
    if kind != Some(Kind::Wiki) {
        return;
    }
    let Ok(caret) = editor.caret() else {
        return;
    };
    let closed = editor.range_bytes(caret..caret + 2).ok() == Some(b"]]".as_slice());
    if !closed {
        let _ = editor.replace_target(caret..caret, "]]");
        let _ = editor.set_selection(caret + 2..caret + 2);
    }
}

/// In the Live view the hidden editor's list would open at the editor's caret: it moves under
/// Live's caret instead, or above it when the screen ends below.
fn place_in_live(hwnd: HWND, editor: &Editor) {
    use windows_sys::Win32::Graphics::Gdi::{
        ClientToScreen, GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowRect, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, SetWindowPos,
    };
    let Some((group, ..)) = crate::window::markdown_host::group_of_editor(hwnd, editor.hwnd())
    else {
        return;
    };
    let Ok(caret) = editor.caret() else {
        return;
    };
    let Some((view, (x, top, bottom))) =
        crate::window::live_host::caret_client_rect(hwnd, group, caret)
    else {
        return;
    };
    let Some(list) = completion_window() else {
        return;
    };
    let mut below = POINT { x, y: bottom };
    let mut above = POINT { x, y: top };
    unsafe {
        ClientToScreen(view, &mut below);
        ClientToScreen(view, &mut above);
    }
    let mut rect = RECT::default();
    unsafe { GetWindowRect(list, &mut rect) };
    let height = rect.bottom - rect.top;
    let mut info: MONITORINFO = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
    let monitor = unsafe { MonitorFromPoint(below, MONITOR_DEFAULTTONEAREST) };
    let fits = unsafe { GetMonitorInfoW(monitor, &mut info) } == 0
        || below.y + height <= info.rcWork.bottom;
    let y = if fits { below.y } else { above.y - height };
    unsafe {
        SetWindowPos(
            list,
            std::ptr::null_mut(),
            below.x,
            y,
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        )
    };
}

/// Scintilla's shown completion list: a top-level `ListBoxX` window of this thread.
fn completion_window() -> Option<HWND> {
    use windows_sys::Win32::System::Threading::GetCurrentThreadId;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumThreadWindows, GetClassNameW, IsWindowVisible,
    };
    unsafe extern "system" fn visit(window: HWND, found: LPARAM) -> i32 {
        let mut class = [0u16; 16];
        let length = unsafe { GetClassNameW(window, class.as_mut_ptr(), class.len() as i32) };
        let name: Vec<u16> = "ListBoxX".encode_utf16().collect();
        if length as usize == name.len()
            && class[..name.len()] == name[..]
            && unsafe { IsWindowVisible(window) } != 0
        {
            unsafe { *(found as *mut HWND) = window };
            return 0;
        }
        1
    }
    let mut found: HWND = std::ptr::null_mut();
    unsafe {
        EnumThreadWindows(
            GetCurrentThreadId(),
            Some(visit),
            &mut found as *mut HWND as LPARAM,
        )
    };
    (!found.is_null()).then_some(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_targets_rank_by_the_quick_open_matcher() {
        let items: Vec<String> = ["Archive/Old plans", "Plans", "Meeting notes", "Planning/Q4"]
            .map(String::from)
            .to_vec();
        assert_eq!(rank_notes(&items, ""), items);
        let ranked = rank_notes(&items, "plan");
        assert_eq!(ranked.first().map(String::as_str), Some("Plans"));
        assert!(!ranked.contains(&"Meeting notes".to_owned()));
        assert_eq!(rank_notes(&items, "mtg nt"), vec!["Meeting notes"]);
    }

    #[test]
    fn a_query_ends_where_its_list_stops_completing() {
        assert_eq!(query(Kind::Wiki, "[[Pla"), Some(2));
        assert_eq!(query(Kind::Wiki, "[[Pla]]"), None);
        assert_eq!(query(Kind::Slash, "/ta"), Some(1));
        assert_eq!(query(Kind::Tag, "#to do"), None);
    }
}
