//! Running searches: the box's query changes, library changes, the search's batches and pattern
//! errors, and the match options.

use super::*;
use crate::config::SidebarView;
use crate::library::model::same_path;
use crate::platform::wide_null;
use crate::search::{MatchOptions, SearchOption};
use crate::window::library_host;
use crate::window::side_panel;
use crate::window::text_search_host::{self, SearchBatch};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::Graphics::Gdi::InvalidateRect;
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
            view.box_text.clone_from(&query);
            (view.search == SearchState::TooShort).then(|| {
                view.search = SearchState::Idle;
                view.panel
            })
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

/// Part of `side_panel::refresh`. A new notebook cancels the search and clears the query and the
/// results. The same notebook runs the query again if its notes changed, or, while the view is
/// hidden, checks once it shows again.
pub(crate) fn library_changed(hwnd: HWND) {
    let notebook = library_host::folder(hwnd);
    let loaded = library_host::with_state(hwnd, |_| ()).is_some();
    let failed = library_host::load_failed(hwnd);
    let Some((edit, changed)) = with_view(hwnd, |view| {
        let changed = match (&view.notebook, &notebook) {
            (Some(old), Some(new)) => !same_path(old, new),
            (None, None) => false,
            _ => true,
        };
        view.loaded = loaded;
        view.failed = failed;
        if changed {
            view.notebook = notebook.clone();
            view.placeholder = placeholder(notebook.as_deref());
            view.clear_results();
            view.query.clear();
            view.search = SearchState::Idle;
            view.stale = false;
        }
        (view.edit, changed)
    }) else {
        return;
    };
    if changed {
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
