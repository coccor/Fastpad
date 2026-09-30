//! The find bar: opening and closing it, its panel pointer input and placeholders, and find,
//! find again and replace.

use super::*;
use windows_sys::Win32::UI::WindowsAndMessaging::{KillTimer, SetTimer};

/// The match counter recounts this long after the last change that affects it. Typing in the
/// query field or the document restarts the wait, so a burst of keys counts once.
const FIND_COUNT_DELAY_MS: u32 = 150;
pub(crate) const FIND_COUNT_TIMER_ID: usize = 0x4650_4643;

pub(super) fn open_find_bar(hwnd: HWND, mode: find_bar::FindBarMode) {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    // The two bars share the band above the editor; only one shows at a time.
    crate::window::library_host::close_name_box(hwnd);
    if !identity.is_live_for(hwnd) || !ensure_find_bar(hwnd) {
        return;
    }
    // A single-line selection is a reasonable query prefill; a multi-line one is not (the bar has
    // no way to display it), so it's left alone rather than truncated or rejected. With regex
    // on, it is escaped (as Search escapes it) so it matches only itself.
    let regex = unsafe { app_ptr(hwnd) }
        .and_then(|app| Some(unsafe { app.as_ref() }.find_bar()?.options().regex))
        .unwrap_or(false);
    let prefill = single_line_selection(hwnd).map(|text| {
        if regex {
            crate::search::escape(&text)
        } else {
            text
        }
    });
    let colors = title_chrome(hwnd).0;
    let pending = unsafe { app_ptr(hwnd) }.and_then(|mut app| {
        let bar = unsafe { app.as_mut() }.find_bar_mut()?;
        Some(bar.show(mode, prefill.as_deref(), colors))
    });
    let Some(pending) = pending else {
        return;
    };
    // Applied with nothing borrowed: the field's EN_CHANGE borrows the bar again.
    if let Some(pending) = pending {
        pending.apply();
    }
    if !identity.is_live_for(hwnd) {
        return;
    }
    layout_editor_and_find_bar(hwnd);
    if let Some(app) = unsafe { app_ptr(hwnd) }
        && let Some(bar) = unsafe { app.as_ref() }.find_bar()
    {
        bar.focus_query();
    }
    // A query kept from earlier is counted again in this document.
    schedule_find_count(hwnd);
}

/// Makes the find bar the first time it's needed, with nothing of the `App` borrowed, because
/// creating its controls sends messages. Returns false when there's no bar and none could be
/// made.
pub(super) fn ensure_find_bar(hwnd: HWND) -> bool {
    let Some(exists) =
        unsafe { app_ptr(hwnd) }.map(|app| unsafe { app.as_ref() }.find_bar().is_some())
    else {
        return false;
    };
    if exists {
        return true;
    }
    let Ok(bar) = find_bar::FindBar::create(content_parent(hwnd)) else {
        return false;
    };
    unsafe { app_ptr(hwnd) }.is_some_and(|mut app| {
        let Some(group) = unsafe { app.as_mut() }.active_group_mut() else {
            return false;
        };
        group.find_bar = Some(bar);
        true
    })
}

/// The active editor's selection as a query, when it is non-empty and on one line. A multi-line
/// selection can't be shown in a one-line box, so it is left alone rather than cut.
pub(crate) fn single_line_selection(hwnd: HWND) -> Option<String> {
    let editor =
        unsafe { app_ptr(hwnd) }.and_then(|app| unsafe { app.as_ref() }.editor().cloned())?;
    let text = editor.selected_text().ok()?;
    (!text.is_empty() && !text.contains(['\n', '\r'])).then_some(text)
}

/// Ctrl+Shift+F (spec §5) shows Search and focuses its box. A single-line selection in the active
/// editor replaces the box's text and searches at once. `show_with_query` escapes it while regex
/// is on.
pub(super) fn show_search_view(hwnd: HWND) {
    // Read before the box takes the focus.
    let prefill = single_line_selection(hwnd);
    crate::window::side_panel::show_view(hwnd, crate::config::SidebarView::Search, true);
    if let Some(text) = prefill {
        crate::window::search_view::show_with_query(hwnd, &text);
    }
}

/// A `SearchToggle*` palette command shows the Search view first if it is hidden, so the option it
/// flips is visible.
pub(super) fn toggle_search_option(hwnd: HWND, option: crate::search::SearchOption) {
    if crate::window::side_panel::current_view(hwnd) != crate::config::SidebarView::Search {
        crate::window::side_panel::show_view(hwnd, crate::config::SidebarView::Search, true);
    }
    crate::window::search_view::toggle_option(hwnd, option);
}

pub(crate) fn close_find_bar(hwnd: HWND) {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    let closed = unsafe { app_ptr(hwnd) }.is_some_and(|mut app| {
        let Some(bar) = unsafe { app.as_mut() }.find_bar_mut() else {
            return false;
        };
        bar.hide();
        true
    });
    if !closed || !identity.is_live_for(hwnd) {
        return;
    }
    layout_editor_and_find_bar(hwnd);
    focus_content(hwnd);
}

/// Mouse input on a panel: hovering over and clicking the find bar's close button and toggles.
pub(crate) fn panel_pointer(hwnd: HWND, panel: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) {
    if message == windows_sys::Win32::UI::WindowsAndMessaging::WM_MOUSEMOVE {
        ensure_find_tooltip(hwnd, panel, message, wparam, lparam);
    }
    let click = unsafe { app_ptr(hwnd) }.and_then(|app| {
        unsafe { app.as_ref() }
            .find_bar()
            .filter(|bar| bar.owns(panel))
            .and_then(|bar| bar.pointer(message, lparam))
    });
    match click {
        Some(find_bar::BarClick::Close) => close_find_bar(hwnd),
        Some(find_bar::BarClick::Toggle(option)) => toggle_find_option(hwnd, option),
        Some(find_bar::BarClick::Nav(find_bar::NavButton::Previous)) => find_previous(hwnd),
        Some(find_bar::BarClick::Nav(find_bar::NavButton::Next)) => find_next(hwnd),
        None => {}
    }
}

/// The first pointer move over the find bar makes its toggles' tooltip and hands it that move,
/// so the first hover starts the tip's timer like any later one. Nothing before that needs it.
fn ensure_find_tooltip(hwnd: HWND, panel: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) {
    let wanted = unsafe { app_ptr(hwnd) }.is_some_and(|app| {
        unsafe { app.as_ref() }
            .find_bar()
            .is_some_and(|bar| bar.owns(panel) && bar.wants_tooltip())
    });
    if !wanted {
        return;
    }
    // Made with nothing of the App borrowed: creating the control sends messages.
    let created = crate::window::tooltip::Tooltip::create(panel);
    let tools = unsafe { app_ptr(hwnd) }.and_then(|app| {
        let bar = unsafe { app.as_ref() }.find_bar()?;
        bar.set_tooltip(created);
        Some(bar.tooltip_tools())
    });
    match (created, tools) {
        (Some(_), Some(Some((tooltip, tools)))) => {
            for (index, (rect, text)) in tools.into_iter().enumerate() {
                tooltip.set_tool(index, rect, text);
            }
            tooltip.relay(message, wparam, lparam);
        }
        // The bar went while the tooltip was being made.
        (Some(tooltip), None) => tooltip.destroy(),
        _ => {}
    }
}

/// Flips a find bar option (a toggle click, or Alt+C, Alt+W or Alt+R in its fields) and tells
/// screen readers the check button's state changed, once the borrow is over.
pub(crate) fn toggle_find_option(hwnd: HWND, option: crate::search::SearchOption) {
    let panel = unsafe { app_ptr(hwnd) }.and_then(|mut app| {
        let bar = unsafe { app.as_mut() }.find_bar_mut()?;
        bar.toggle_option(option);
        Some(bar.panel_hwnd())
    });
    if let Some(panel) = panel {
        crate::window::sidebar_accessibility::notify(
            windows_sys::Win32::UI::WindowsAndMessaging::EVENT_OBJECT_STATECHANGE,
            panel,
            Some(find_bar::toggle_child(option)),
        );
    }
    schedule_find_count(hwnd);
}

/// `EN_CHANGE` from a find field. Its text is read with nothing of the `App` borrowed and kept
/// as the field's accessible value.
pub(super) fn find_field_changed(hwnd: HWND, control: HWND) {
    let text = find_bar::control_text(control);
    let query = unsafe { app_ptr(hwnd) }.is_some_and(|mut app| {
        unsafe { app.as_mut() }.find_bar_mut().is_some_and(|bar| {
            bar.field_changed(control, text);
            bar.is_query(control)
        })
    });
    if query {
        set_find_no_match(hwnd, false);
        // The old count no longer describes the query. The new one waits for a pause in typing.
        set_find_count(hwnd, None);
        schedule_find_count(hwnd);
    }
}

fn set_find_count(hwnd: HWND, count: Option<find_bar::MatchCount>) {
    if let Some(app) = unsafe { app_ptr(hwnd) }
        && let Some(bar) = unsafe { app.as_ref() }.find_bar()
    {
        bar.set_count(count);
    }
}

/// Starts (or restarts) the wait before the match counter recounts. Called after every change
/// that can change the count: the query, an option, the selection moving to another match, an edit
/// to the document, another tab. Does nothing (no timer) while the bar is closed or its query is
/// empty, so editing with the bar shut costs nothing.
pub(crate) fn schedule_find_count(hwnd: HWND) {
    let wanted = unsafe { app_ptr(hwnd) }.is_some_and(|app| {
        unsafe { app.as_ref() }
            .find_bar()
            .is_some_and(|bar| bar.is_visible() && bar.has_query())
    });
    if wanted {
        unsafe {
            SetTimer(hwnd, FIND_COUNT_TIMER_ID, FIND_COUNT_DELAY_MS, None);
        }
    }
}

/// `WM_TIMER` for `FIND_COUNT_TIMER_ID`: the wait ended. While a file is being populated the
/// document is not the one the bar's query belongs to, so it tries again at the next tick.
pub(crate) fn find_count_timer(hwnd: HWND) {
    if file_population_active(hwnd) {
        return;
    }
    unsafe {
        KillTimer(hwnd, FIND_COUNT_TIMER_ID);
    }
    refresh_find_count(hwnd);
}

/// Counts the matches of the bar's query in the active document now, with the current selection
/// as the current match, and shows the result.
pub(crate) fn refresh_find_count(hwnd: HWND) {
    let Some((editor, query, options)) = (unsafe { app_ptr(hwnd) }).and_then(|app| {
        let app = unsafe { app.as_ref() };
        let bar = app.find_bar().filter(|bar| bar.is_visible())?;
        Some((app.editor().cloned()?, bar.query_text(), bar.options()))
    }) else {
        return;
    };
    let count = editor
        .selection()
        .ok()
        .and_then(|selection| find_bar::count_matches(&editor, &query, options, &selection));
    set_find_count(hwnd, count);
}

/// `WM_PAINT` for an empty find field; false when there is no find bar to paint it.
pub(crate) fn paint_find_placeholder(hwnd: HWND, edit: HWND) -> bool {
    unsafe { app_ptr(hwnd) }.is_some_and(|app| {
        unsafe { app.as_ref() }
            .find_bar()
            .is_some_and(|bar| bar.paint_placeholder(edit))
    })
}

pub(crate) fn paint_palette_placeholder(hwnd: HWND, edit: HWND) -> bool {
    with_command_palette(hwnd, |palette| palette.paint_placeholder(edit)).unwrap_or(false)
}

pub(super) fn name_box_owns(hwnd: HWND, control: HWND) -> bool {
    unsafe { app_ptr(hwnd) }.is_some_and(|app| {
        unsafe { app.as_ref() }
            .name_box
            .as_ref()
            .is_some_and(|name_box| name_box.owns(control))
    })
}

pub(crate) fn find_bar_owns(hwnd: HWND, control: HWND) -> bool {
    unsafe { app_ptr(hwnd) }.is_some_and(|app| {
        unsafe { app.as_ref() }
            .find_bar()
            .is_some_and(|bar| bar.owns(control))
    })
}

pub(crate) fn find_next(hwnd: HWND) {
    navigate_to_match(hwnd, false);
}

pub(crate) fn find_previous(hwnd: HWND) {
    navigate_to_match(hwnd, true);
}

/// F3 and Shift+F3 step through the find bar's query, even while the bar is closed. With no
/// query yet, they open the bar.
pub(super) fn find_again(hwnd: HWND, backward: bool) {
    let has_query = unsafe { app_ptr(hwnd) }.is_some_and(|app| {
        unsafe { app.as_ref() }
            .find_bar()
            .is_some_and(|bar| !bar.query_text().is_empty())
    });
    if has_query {
        navigate_to_match(hwnd, backward);
    } else {
        open_find_bar(hwnd, find_bar::FindBarMode::Find);
    }
}

fn navigate_to_match(hwnd: HWND, backward: bool) {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    let Some((editor, query, options)) = (unsafe { app_ptr(hwnd) }).and_then(|app| {
        let app = unsafe { app.as_ref() };
        let editor = app.editor().cloned()?;
        let bar = app.find_bar()?;
        Some((editor, bar.query_text(), bar.options()))
    }) else {
        return;
    };
    if query.is_empty() {
        return;
    }
    let Ok(selection) = editor.selection() else {
        return;
    };
    let (origin, direction) = if backward {
        (selection.start, find_bar::SearchDirection::Backward)
    } else {
        (selection.end, find_bar::SearchDirection::Forward)
    };
    select_match(hwnd, &identity, &editor, &query, options, origin, direction);
}

/// Selects the next match of `query` under `options` from `origin`, wrapping once, and scrolls
/// it into view. When there is none, the selection stays and the find bar shows its no-match
/// state. A regex that doesn't compile, or matches empty text, counts as no match.
pub(super) fn select_match(
    hwnd: HWND,
    identity: &WindowIdentity,
    editor: &Editor,
    query: &str,
    options: crate::search::MatchOptions,
    origin: usize,
    direction: find_bar::SearchDirection,
) {
    let found = find_bar::find_in_editor(editor, query, options, origin, direction);
    if !identity.is_live_for(hwnd) {
        return;
    }
    if let Some(found) = found.clone() {
        let _ = editor.set_selection(found);
        editor.scroll_caret_into_view();
    }
    set_find_no_match(hwnd, found.is_none());
    schedule_find_count(hwnd);
}

fn set_find_no_match(hwnd: HWND, no_match: bool) {
    if let Some(app) = unsafe { app_ptr(hwnd) }
        && let Some(bar) = unsafe { app.as_ref() }.find_bar()
    {
        bar.set_no_match(no_match);
    }
}

pub(crate) fn replace_current(hwnd: HWND) {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    let Some((editor, query, replacement, options)) = (unsafe { app_ptr(hwnd) }).and_then(|app| {
        let app = unsafe { app.as_ref() };
        let editor = app.editor().cloned()?;
        let bar = app.find_bar()?;
        Some((editor, bar.query_text(), bar.replace_text(), bar.options()))
    }) else {
        return;
    };
    if query.is_empty() {
        return;
    }
    // Only replace when the selection is exactly a match under the options (a case-insensitive
    // "CAT" for "cat", a regex's match, a whole word); otherwise this Enter just moves to the
    // next match, as in a bare Find field. In regex mode the replacement expands `$1` with the
    // selected match's groups; in plain mode it is literal.
    if let Ok(selection) = editor.selection()
        && let Some(text) =
            find_bar::replacement_for(&editor, &query, &replacement, options, selection.clone())
    {
        let _ = editor.replace_target(selection, &text);
        if !identity.is_live_for(hwnd) {
            return;
        }
    }
    find_next(hwnd);
}

pub(crate) fn replace_all_matches(hwnd: HWND) {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    let Some((editor, query, replacement, options)) = (unsafe { app_ptr(hwnd) }).and_then(|app| {
        let app = unsafe { app.as_ref() };
        let editor = app.editor().cloned()?;
        let bar = app.find_bar()?;
        Some((editor, bar.query_text(), bar.replace_text(), bar.options()))
    }) else {
        return;
    };
    if query.is_empty() {
        return;
    }
    let replaced = find_bar::replace_all(&editor, &query, &replacement, options);
    if identity.is_live_for(hwnd) {
        editor.scroll_caret_into_view();
        set_find_no_match(hwnd, replaced == 0);
    }
}
