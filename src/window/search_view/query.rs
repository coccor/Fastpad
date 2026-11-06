//! Running searches: the box's query changes, library changes, the search's batches and pattern
//! errors, and the match options.

use super::*;
use crate::config::SidebarView;
use crate::platform::wide_null;
use crate::search::{MatchOptions, SearchOption};
use crate::window::library_host;
use crate::window::side_panel;
use crate::window::text_search_host::{self, SearchBatch};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::Graphics::Gdi::InvalidateRect;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows_sys::Win32::UI::WindowsAndMessaging::SetWindowTextW;

/// `EN_CHANGE` from the box. A query that can run restarts the debounce and nothing else. One
/// that can't (empty, all white space or one character) cancels the search and clears the list.
pub(crate) fn query_changed(hwnd: HWND) {
    let Some(edit) = with_view(hwnd, |view| view.edit).flatten() else {
        return;
    };
    // Read with nothing of the App borrowed, and kept for screen readers.
    let query = window_text(edit);
    if text_search_host::searchable(&query) {
        text_search_host::schedule(hwnd);
        // "Type at least 2 characters." goes at once, not when the debounce ends.
        let panel = with_view(hwnd, |view| {
            // The clear button appears with the first text.
            let first = std::mem::replace(&mut view.box_text, query.clone()).is_empty();
            let too_short = view.search == SearchState::TooShort;
            if too_short {
                view.search = SearchState::Idle;
            }
            (first || too_short).then_some(view.panel)
        })
        .flatten();
        if let Some(panel) = panel {
            invalidate(panel);
            announce_lines(hwnd, true);
        }
        return;
    }
    text_search_host::cancel(hwnd);
    let state = if query.trim().is_empty() {
        SearchState::Idle
    } else {
        SearchState::TooShort
    };
    let Some(panel) = with_view(hwnd, |view| {
        view.clear_results();
        view.box_text.clone_from(&query);
        view.query = query;
        view.search = state;
        view.panel
    }) else {
        return;
    };
    invalidate(panel);
    announce_lines(hwnd, true);
}

/// The clear button: empties the box, which clears the query and the results (`query_changed`),
/// and puts the caret back in it. Runs with nothing of the App borrowed: setting the text sends
/// `EN_CHANGE` and focusing sends focus messages.
pub(super) fn clear_search(hwnd: HWND) {
    let Some(edit) = with_view(hwnd, |view| view.edit).flatten() else {
        return;
    };
    let empty = wide_null("");
    unsafe {
        SetWindowTextW(edit, empty.as_ptr());
        SetFocus(edit);
    }
}

/// Part of `side_panel::refresh`. The view searches every loaded open notebook, or the scoped
/// one (multi-root search spec §4); switching tabs changes nothing. A notebook opening, closing
/// or finishing its load drops the rows of notebooks no longer searched at once and runs the
/// query again (through the note lists' marks, `text_search_host::library_changed`), or, while
/// the view is hidden, once it shows again. With no notebook open at all, the query and the
/// results are cleared.
pub(crate) fn library_changed(hwnd: HWND) {
    let open = library_host::notebooks(hwnd);
    let several = open.len() > 1;
    let all = library_host::searchable_notebooks(hwnd);
    let failed = !open.is_empty()
        && open
            .iter()
            .all(|(id, _)| library_host::load_failed_of(hwnd, *id));
    // A scope whose notebook closed goes back to all notebooks (spec §4.1).
    let scope = with_view(hwnd, |view| view.scope)
        .flatten()
        .filter(|id| several && open.iter().any(|(open, _)| open == id));
    let searched: Vec<Searchable> = all
        .iter()
        .filter(|s| scope.is_none_or(|id| s.id == id))
        .cloned()
        .collect();
    let name = match scope {
        Some(id) => all.iter().find(|s| s.id == id).map(|s| s.label.clone()),
        None if several => Some("all notebooks".to_owned()),
        None => open
            .first()
            .and_then(|(id, _)| library_host::notebook_display_name(hwnd, *id)),
    };
    let suffix = if several && scope.is_none() {
        library_host::skipped_suffix(library_host::skipped_notebooks(hwnd))
    } else {
        String::new()
    };
    let header = format!("{}{suffix}", header_text(name.as_deref()));
    let Some((edit, closed_all, refolded)) = with_view(hwnd, |view| {
        let closed_all = view.open && open.is_empty();
        // A notebook whose folder changed under the same id: its rows, and any search reading
        // it, are another folder's.
        let refolded: Vec<library_host::NotebookId> = view
            .notebooks
            .iter()
            .filter(|old| {
                searched
                    .iter()
                    .any(|new| new.id == old.id && new.folder != old.folder)
            })
            .map(|old| old.id)
            .collect();
        view.results
            .retain(|result| !refolded.contains(&result.notebook));
        view.open = !open.is_empty();
        view.loaded = !all.is_empty();
        view.failed = failed;
        view.scope = scope;
        if view.header != header {
            view.header = header;
            invalidate(view.panel);
        }
        view.placeholder = placeholder(name.as_deref());
        let moved = view.several != several
            || view.notebooks.len() != searched.len()
            || view.notebooks.iter().zip(&searched).any(|(a, b)| a != b);
        view.several = several;
        view.notebooks = searched;
        if moved {
            view.relist();
            invalidate(view.panel);
        }
        if closed_all {
            view.clear_results();
            view.query.clear();
            view.search = SearchState::Idle;
            view.stale = false;
        }
        (view.edit, closed_all, !refolded.is_empty())
    }) else {
        return;
    };
    if refolded {
        text_search_host::forget(hwnd);
    }
    if closed_all {
        text_search_host::forget(hwnd);
        // Clearing the box sends EN_CHANGE, which leaves the view idle.
        if let Some(edit) = edit {
            let empty = wide_null("");
            unsafe {
                SetWindowTextW(edit, empty.as_ptr());
                InvalidateRect(edit, std::ptr::null(), 1);
            }
        }
        layout(hwnd);
    } else if side_panel::current_view(hwnd) == SidebarView::Search {
        text_search_host::library_changed(hwnd);
    } else {
        with_view(hwnd, |view| view.stale = true);
    }
}

/// Limits the search to the open notebook `scope`, or to none for all notebooks (spec §4.1),
/// and runs the query again at once.
pub(crate) fn set_scope(hwnd: HWND, scope: Option<library_host::NotebookId>) {
    if with_view(hwnd, |view| view.scope = scope).is_none() {
        return;
    }
    library_changed(hwnd);
    text_search_host::run_now(hwnd);
}

/// The notebook the search is limited to, `None` for all.
#[cfg(test)]
pub(crate) fn scope(hwnd: HWND) -> Option<library_host::NotebookId> {
    with_view(hwnd, |view| view.scope).flatten()
}

/// The notebooks the view searches, in sidebar order (spec §4.3).
pub(crate) fn searched_notebooks(hwnd: HWND) -> Vec<Searchable> {
    with_view(hwnd, |view| view.notebooks.clone()).unwrap_or_default()
}

/// The box's text and the options, or `None` before the box exists.
pub(crate) fn current_query(hwnd: HWND) -> Option<(String, MatchOptions)> {
    let (edit, options) = with_view(hwnd, |view| Some((view.edit?, view.options))).flatten()?;
    // Read with nothing of the App borrowed: WM_GETTEXT goes through the box's subclass.
    Some((window_text(edit), options))
}

/// The query and options the shown results ran with, or `None` without a view.
pub(crate) fn run_query(hwnd: HWND) -> Option<(String, MatchOptions)> {
    with_view(hwnd, |view| (view.query.clone(), view.run_options))
}

/// `text_search_host::run_now` started a search for `query` over `total` notes. Replace all
/// waits for it, and says so.
pub(crate) fn begin_search(hwnd: HWND, query: &str, total: usize) {
    let Some((panel, flipped)) = with_view(hwnd, |view| {
        let enabled = view.replace_all_enabled();
        view.begin(query, total);
        (view.panel, enabled != view.replace_all_enabled())
    }) else {
        return;
    };
    invalidate(panel);
    if flipped {
        announce_state(hwnd, SearchChild::ReplaceAll);
    }
    announce_lines(hwnd, false);
}

/// A batch of the current search (`text_search_host::batch_arrived`). The panel repaints only if
/// something it shows changed. Replace all says when it became available.
pub(crate) fn apply_batch(hwnd: HWND, batch: SearchBatch) {
    let settled = batch.end.is_some();
    let Some((panel, enabled)) = with_view(hwnd, |view| (view.panel, view.replace_all_enabled()))
    else {
        return;
    };
    let (client, dpi) = geometry(panel);
    let (changed, flipped) = with_view(hwnd, |view| {
        let area = view.list_area(client, dpi);
        let changed = view.apply(batch, height(area));
        (changed, view.replace_all_enabled() != enabled)
    })
    .unwrap_or((false, false));
    if changed || flipped {
        invalidate(panel);
    }
    if flipped {
        announce_state(hwnd, SearchChild::ReplaceAll);
    }
    announce_lines(hwnd, settled);
}

/// Shows a pattern's error in place of the summary, keeping the results, or clears it.
pub(crate) fn set_pattern_error(hwnd: HWND, error: Option<String>) {
    let Some(panel) = with_view(hwnd, |view| {
        match error {
            Some(message) => view.search = SearchState::PatternError(message),
            None if matches!(view.search, SearchState::PatternError(_)) => {
                view.search = SearchState::Idle;
            }
            None => {}
        }
        view.panel
    }) else {
        return;
    };
    invalidate(panel);
    announce_lines(hwnd, true);
}

pub(crate) fn options(hwnd: HWND) -> MatchOptions {
    with_view(hwnd, |view| view.options).unwrap_or_default()
}

/// Flips `option` and runs the query again at once.
pub(crate) fn toggle_option(hwnd: HWND, option: SearchOption) {
    let Some(panel) = with_view(hwnd, |view| {
        view.options = view.options.toggled(option);
        view.panel
    }) else {
        return;
    };
    invalidate(panel);
    announce_toggle(hwnd, option);
    text_search_host::run_now(hwnd);
}
