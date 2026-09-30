//! The Search view's edit controls: the search box and replace field, opening replace, the
//! box's layout, showing and hiding the view, and the subclass that paints placeholders and
//! routes keys.

use super::*;
use crate::config::SidebarView;
use crate::platform::{last_error, wide_null};
use crate::search::escape;
use crate::window::main_window::OpenMode;
use crate::window::option_toggles;
use crate::window::panel::{create_child, fill, scale, text_height};
use crate::window::side_panel::{self, draw_text};
use crate::window::text_search_host;
use std::path::PathBuf;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, DT_END_ELLIPSIS, DT_NOPREFIX, DT_SINGLELINE, EndPaint, HFONT, InvalidateRect,
    PAINTSTRUCT,
};
use windows_sys::Win32::UI::Controls::{EM_GETMARGINS, EM_REPLACESEL, EM_SETSEL, EM_UNDO};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetFocus, SetFocus, VK_CONTROL, VK_DOWN, VK_ESCAPE, VK_MENU, VK_NEXT, VK_RETURN, VK_SHIFT,
    VK_TAB, VK_UP,
};
use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DestroyWindow, ES_AUTOHSCROLL, GetClientRect, GetParent, GetWindowTextLengthW, MoveWindow,
    SW_HIDE, SW_SHOWNA, SendMessageW, SetWindowTextW, ShowWindow, WM_CHAR, WM_CLEAR, WM_CUT,
    WM_GETFONT, WM_KEYDOWN, WM_NCDESTROY, WM_PAINT, WM_PASTE, WM_SETFONT, WM_SETTEXT,
    WM_SYSKEYDOWN, WM_UNDO, WS_CHILD,
};

/// Ctrl+Shift+F with a one-line selection: `text` replaces the box's text (escaped first when
/// regex is on), all of it selected, and the search runs at once. The caller shows the view.
pub(crate) fn show_with_query(hwnd: HWND, text: &str) {
    let text = if options(hwnd).regex {
        escape(text)
    } else {
        text.to_owned()
    };
    let Some(edit) = ensure_edit(hwnd) else {
        return;
    };
    let wide = wide_null(&text);
    // Setting the text sends EN_CHANGE, which starts the debounce; `run_now` replaces it.
    unsafe {
        SetWindowTextW(edit, wide.as_ptr());
        SendMessageW(edit, EM_SETSEL, 0, -1);
    }
    text_search_host::run_now(hwnd);
}

/// Ctrl+Shift+H (spec §11): shows Search with the replace field open and the caret in it. A
/// one-line selection in the active editor fills the search box and searches at once, as
/// Ctrl+Shift+F's does (`show_with_query` escapes it while regex is on). Pressed with the caret
/// already in the open replace field, it closes the field instead and the caret goes back to the
/// search box: the keyboard's way to close it (spec §17), as the chevron takes no focus.
pub(crate) fn show_replace(hwnd: HWND) {
    let field = with_view(hwnd, |view| view.replace_open.then_some(view.replace_edit))
        .flatten()
        .flatten();
    // Asked with nothing of the App borrowed.
    if field.is_some_and(|field| unsafe { GetFocus() } == field) {
        side_panel::with_accessible_events(hwnd, || set_replace_open(hwnd, false));
        return;
    }
    // Read before the view takes the focus.
    let prefill = crate::window::main_window::single_line_selection(hwnd);
    side_panel::show_view(hwnd, SidebarView::Search, false);
    if let Some(text) = prefill {
        show_with_query(hwnd, &text);
    }
    let replace = side_panel::with_accessible_events(hwnd, || set_replace_open(hwnd, true));
    let edit = with_view(hwnd, |view| view.edit).flatten();
    // Focused with nothing of the App borrowed.
    unsafe {
        match (replace, edit) {
            (Some(replace), _) => {
                SetFocus(replace);
                SendMessageW(replace, EM_SETSEL, 0, -1);
            }
            // No replace field could be made (reported once): the box takes the caret instead.
            (None, Some(edit)) => {
                SetFocus(edit);
            }
            (None, None) => {}
        }
    }
}

/// The listed notes' paths, relative to the notebook, in list order.
pub(crate) fn result_paths(hwnd: HWND) -> Vec<PathBuf> {
    with_view(hwnd, |view| {
        view.results
            .iter()
            .map(|result| result.path.clone())
            .collect()
    })
    .unwrap_or_default()
}

/// The listed notes in list order, as a replace takes them (spec §12a: never a note that isn't
/// listed), and whether the results were capped.
pub(crate) fn replace_candidates(hwnd: HWND) -> (Vec<text_search_host::Candidate>, bool) {
    with_view(hwnd, |view| {
        let candidates = view
            .results
            .iter()
            .map(|hit| text_search_host::Candidate {
                path: hit.path.clone(),
                name: hit.name.clone(),
                stamp: hit.stamp,
            })
            .collect();
        let capped = matches!(view.search, SearchState::Done { capped: true, .. });
        (candidates, capped)
    })
    .unwrap_or_default()
}

/// A replace started or ended (`text_search_host`): while one runs the summary says so, no
/// result opens, and Replace all and the row buttons are unavailable (drawn dim, and so reported),
/// which screen readers are told when it flips.
pub(crate) fn set_replacing(hwnd: HWND, replacing: bool) {
    let Some((panel, flipped, row)) = with_view(hwnd, |view| {
        (view.replacing != replacing).then(|| {
            let enabled = view.replace_all_enabled();
            view.replacing = replacing;
            (
                view.panel,
                enabled != view.replace_all_enabled(),
                view.row_replace_child(),
            )
        })
    })
    .flatten() else {
        return;
    };
    invalidate(panel);
    if flipped {
        announce_state(hwnd, SearchChild::ReplaceAll);
        if let Some(row) = row {
            announce_state(hwnd, SearchChild::RowReplace(row));
        }
    }
    announce_lines(hwnd, false);
}

/// The search box, made now if the view has none yet. A failure is reported once.
pub(super) fn ensure_edit(hwnd: HWND) -> Option<HWND> {
    let (panel, edit, failed) = with_view(hwnd, |view| (view.panel, view.edit, view.edit_failed))?;
    if edit.is_some() || failed {
        return edit;
    }
    // Made with nothing of the App borrowed: creating the Edit sends messages to the panel.
    match create_edit(panel, SEARCH_HOOK_ID) {
        Ok(edit) => {
            if with_view(hwnd, |view| view.edit = Some(edit)).is_none() {
                unsafe { DestroyWindow(edit) };
                return None;
            }
            Some(edit)
        }
        Err(error) => {
            with_view(hwnd, |view| view.edit_failed = true);
            crate::window::main_window::push_notice(
                hwnd,
                format!("FastPad could not show the search box: {error}"),
            );
            None
        }
    }
}

/// The replace field, made now if the view has none yet. A failure is reported once.
pub(super) fn ensure_replace_edit(hwnd: HWND) -> Option<HWND> {
    let (panel, edit, failed) = with_view(hwnd, |view| {
        (view.panel, view.replace_edit, view.replace_edit_failed)
    })?;
    if edit.is_some() || failed {
        return edit;
    }
    // Made with nothing of the App borrowed: creating the Edit sends messages to the panel.
    match create_edit(panel, REPLACE_HOOK_ID) {
        Ok(edit) => {
            if with_view(hwnd, |view| view.replace_edit = Some(edit)).is_none() {
                unsafe { DestroyWindow(edit) };
                return None;
            }
            Some(edit)
        }
        Err(error) => {
            with_view(hwnd, |view| view.replace_edit_failed = true);
            crate::window::main_window::push_notice(
                hwnd,
                format!("FastPad could not show the replace field: {error}"),
            );
            None
        }
    }
}

/// Opens or closes the replace field, making it the first time it opens, and lays the view out
/// again. Returns the field while it is open; `None` when it is closed or could not be made (then
/// it stays closed). Closing it with the caret inside moves the caret to the search box.
pub(super) fn set_replace_open(hwnd: HWND, open: bool) -> Option<HWND> {
    // Without the search box `layout` never places or shows the field, so it never opens: the
    // caret must not go into a hidden control.
    if open && with_view(hwnd, |view| view.edit.is_none()).unwrap_or(true) {
        return None;
    }
    let replace = if open {
        Some(ensure_replace_edit(hwnd)?)
    } else {
        with_view(hwnd, |view| view.replace_edit).flatten()
    };
    let (panel, edit, changed) = with_view(hwnd, |view| {
        let changed = view.replace_open != open;
        if changed {
            view.replace_open = open;
            // The replace children come and go, and the rows move down or up.
            view.order = view.order.wrapping_add(1);
            view.row_hover_button = None;
            view.header_hover = None;
        }
        (view.panel, view.edit, changed)
    })?;
    // Focused with nothing of the App borrowed: SetFocus sends focus messages.
    if !open
        && let (Some(replace), Some(edit)) = (replace, edit)
        && unsafe { GetFocus() } == replace
    {
        unsafe {
            SetFocus(edit);
        }
    }
    layout(hwnd);
    invalidate(panel);
    if changed {
        announce_state(hwnd, SearchChild::Chevron);
    }
    if open { replace } else { None }
}

/// The chevron (spec §11): opens the replace field with the caret in it, or closes it.
pub(crate) fn toggle_replace(hwnd: HWND) {
    let Some(open) = with_view(hwnd, |view| !view.replace_open) else {
        return;
    };
    let replace = side_panel::with_accessible_events(hwnd, || set_replace_open(hwnd, open));
    if let Some(replace) = replace {
        unsafe {
            SetFocus(replace);
            SendMessageW(replace, EM_SETSEL, 0, -1);
        }
    }
}

/// `EN_CHANGE` from one of the view's fields (`side_panel` passes the control).
pub(crate) fn edit_changed(hwnd: HWND, edit: HWND) {
    if with_view(hwnd, |view| view.replace_edit == Some(edit)).unwrap_or(false) {
        replace_changed(hwnd, edit);
    } else {
        query_changed(hwnd);
    }
}

/// `EN_CHANGE` from the replace field: keeps its text. No search runs.
pub(super) fn replace_changed(hwnd: HWND, edit: HWND) {
    // Read with nothing of the App borrowed.
    let text = window_text(edit);
    with_view(hwnd, |view| view.replace_text = text);
}

/// The replace field's text as `EN_CHANGE` last kept it; empty before it is first opened.
pub(crate) fn replace_text(hwnd: HWND) -> String {
    with_view(hwnd, |view| view.replace_text.clone()).unwrap_or_default()
}

/// Makes the search box fail to be made, as `ensure_edit` records a failure. Returns whether
/// there was a view.
#[cfg(test)]
pub(crate) fn fail_search_box(hwnd: HWND) -> bool {
    with_view(hwnd, |view| view.edit_failed = true).is_some()
}

/// Whether Replace all can run now (`SearchView::replace_all_enabled`).
pub(crate) fn replace_all_enabled(hwnd: HWND) -> bool {
    with_view(hwnd, |view| view.replace_all_enabled()).unwrap_or(false)
}

/// A hidden field inside `panel`: the search box (`SEARCH_HOOK_ID`) or the replace field
/// (`REPLACE_HOOK_ID`). The subclass tells them apart by its ID.
pub(super) fn create_edit(panel: HWND, hook: usize) -> crate::Result<HWND> {
    let edit = create_child(panel, &wide_null("Edit"), WS_CHILD | ES_AUTOHSCROLL as u32)?;
    if unsafe { SetWindowSubclass(edit, Some(search_edit_proc), hook, 0) } == 0 {
        let error = last_error();
        unsafe {
            DestroyWindow(edit);
        }
        return Err(error);
    }
    Ok(edit)
}

/// Places the box in the header, short of the toggles, and the replace field in its row while it
/// is open, and shows them while the Search view shows, making the box the first time the view
/// shows a notebook. Part of `side_panel::layout`, and run whenever the view, the notebook or the
/// replace field's state changes. It never makes the box without a notebook: at startup that
/// would come before the first paint. `shown` makes it once the user opens the view.
pub(crate) fn layout(hwnd: HWND) {
    let Some(has_notebook) = with_view(hwnd, |view| view.notebook.is_some()) else {
        return;
    };
    let show = side_panel::current_view(hwnd) == SidebarView::Search;
    let edit = if show && has_notebook {
        ensure_edit(hwnd)
    } else {
        with_view(hwnd, |view| view.edit).flatten()
    };
    let Some(edit) = edit else {
        return;
    };
    let panel = unsafe { GetParent(edit) };
    let (client, dpi) = geometry(panel);
    let text_font = crate::window::main_window::ui_fonts(hwnd).text;
    let inset_x = scale(FIELD_TEXT_INSET_AT_96_DPI, dpi);
    place_field(
        edit,
        SearchView::field_rect(client, dpi),
        text_font,
        inset_x,
        option_toggles::reserved_width(dpi),
    );
    let (replace, replace_open) = with_view(hwnd, |view| {
        view.list.row_height = scale(ROW_AT_96_DPI, dpi);
        (view.replace_edit, view.replace_open)
    })
    .unwrap_or((None, false));
    if let Some(replace) = replace {
        place_field(
            replace,
            SearchView::replace_field_rect(client, dpi),
            text_font,
            inset_x,
            inset_x,
        );
    }
    if show {
        unsafe {
            ShowWindow(edit, SW_SHOWNA);
        }
    } else {
        hide_box(edit);
    }
    if let Some(replace) = replace {
        if show && replace_open {
            unsafe {
                ShowWindow(replace, SW_SHOWNA);
            }
        } else {
            hide_box(replace);
        }
    }
}

/// Gives `edit` the sidebar's text font and centers it vertically in the painted `field`, `inset`
/// in from its left edge and `reserved` short of its right edge.
pub(super) fn place_field(edit: HWND, field: RECT, font: HFONT, inset: i32, reserved: i32) {
    unsafe {
        if !font.is_null() {
            SendMessageW(edit, WM_SETFONT, font as WPARAM, 0);
        }
    }
    let text = text_height(edit, font).clamp(1, (field.bottom - field.top - 2).max(1));
    let top = field.top + (field.bottom - field.top - text) / 2;
    let width = (field.right - field.left - inset - reserved).max(0);
    unsafe {
        MoveWindow(edit, field.left + inset, top, width, text, 1);
    }
}

/// Hides the box. A hidden window keeps the keyboard focus, so a focused box hands it to the
/// panel (which `side_panel` moves on to the editor when the panel closes).
pub(super) fn hide_box(edit: HWND) {
    unsafe {
        if GetFocus() == edit {
            SetFocus(GetParent(edit));
        }
        ShowWindow(edit, SW_HIDE);
    }
}

/// `side_panel::show_view` switched to Search. The user opened it, so the box is made now even
/// with no notebook: its toggles set the options (spec §4). `focus` puts the caret in the box. A
/// library change while the view was hidden runs the query again now, if the notes changed.
pub(crate) fn shown(hwnd: HWND, focus: bool) {
    ensure_edit(hwnd);
    layout(hwnd);
    if with_view(hwnd, |view| std::mem::take(&mut view.stale)).unwrap_or(false) {
        text_search_host::library_changed(hwnd);
    }
    let Some((panel, edit)) = with_view(hwnd, |view| (view.panel, view.edit)) else {
        return;
    };
    if !focus {
        return;
    }
    unsafe {
        match edit {
            Some(edit) => {
                SetFocus(edit);
                SendMessageW(edit, EM_SETSEL, 0, -1);
            }
            None => {
                SetFocus(panel);
            }
        }
    }
}

/// `side_panel::show_view` switched away from Search. The query and the replace text stay. The
/// tooltips go, or they would show over the other view.
pub(crate) fn hidden(hwnd: HWND) {
    let Some((edit, replace, tooltip, tools)) = with_view(hwnd, |view| {
        (
            view.edit,
            view.replace_edit,
            view.tooltip,
            std::mem::take(&mut view.tools_shown),
        )
    }) else {
        return;
    };
    if let Some(tooltip) = tooltip {
        for (id, _, _) in tools {
            tooltip.set_tool(id, RECT::default(), "");
        }
    }
    if let Some(replace) = replace {
        hide_box(replace);
    }
    if let Some(edit) = edit {
        hide_box(edit);
    }
}

/// A field's placeholder, painted where typed text starts: "Search text in <notebook>" in the
/// box, "Replace" in the replace field.
pub(super) fn paint_placeholder(hwnd: HWND, edit: HWND, replace: bool) -> bool {
    let Some((text, colors)) = with_view(hwnd, |view| {
        let text = if replace {
            REPLACE_PLACEHOLDER.to_owned()
        } else {
            view.placeholder.clone()
        };
        (text, view.colors)
    }) else {
        return false;
    };
    let mut paint = PAINTSTRUCT::default();
    let dc = unsafe { BeginPaint(edit, &mut paint) };
    if dc.is_null() {
        return true;
    }
    unsafe {
        let mut client = RECT::default();
        GetClientRect(edit, &mut client);
        fill(dc, client, colors.editor_background);
        let font = SendMessageW(edit, WM_GETFONT, 0, 0);
        client.left += (SendMessageW(edit, EM_GETMARGINS, 0, 0) & 0xffff) as i32;
        draw_text(
            dc,
            &text,
            client,
            font as _,
            colors.muted_foreground,
            DT_SINGLELINE | DT_NOPREFIX | DT_END_ELLIPSIS,
        );
        EndPaint(edit, &paint);
    }
    true
}

/// The subclass of both fields. `subclass_id` tells them apart: `SEARCH_HOOK_ID` for the search
/// box, `REPLACE_HOOK_ID` for the replace field.
unsafe extern "system" fn search_edit_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    subclass_id: usize,
    _ref_data: usize,
) -> LRESULT {
    if message == WM_NCDESTROY {
        // The last message: drop the hook and let the Edit finish; nothing else is looked up.
        unsafe {
            RemoveWindowSubclass(hwnd, Some(search_edit_proc), subclass_id);
            return DefSubclassProc(hwnd, message, wparam, lparam);
        }
    }
    let replace = subclass_id == REPLACE_HOOK_ID;
    let panel = unsafe { GetParent(hwnd) };
    let main = unsafe { GetParent(panel) };
    // A single-line Edit beeps at Enter, Escape and Tab characters; all three are handled on key
    // down.
    if message == WM_CHAR && matches!(wparam as u16, 0x0d | 0x1b | 0x09) {
        return 0;
    }
    // Alt+C, Alt+W and Alt+R flip the toggles before the menu band sees the letter (spec §4).
    if let Some(option) = option_toggles::alt_option(message, wparam, lparam) {
        toggle_option(main, option);
        return 0;
    }
    if option_toggles::is_toggle_char(message, wparam, lparam) {
        return 0;
    }
    if message == WM_PAINT
        && unsafe { GetWindowTextLengthW(hwnd) } == 0
        && paint_placeholder(main, hwnd, replace)
    {
        return 0;
    }
    // Ctrl+Alt+Enter in either field is Replace all (spec §11). With Ctrl held it can come as
    // WM_KEYDOWN or as WM_SYSKEYDOWN; either way it never opens a result.
    if matches!(message, WM_KEYDOWN | WM_SYSKEYDOWN)
        && wparam as u16 == VK_RETURN
        && key_down(VK_CONTROL)
        && key_down(VK_MENU)
    {
        // A held key's repeats (bit 30, the previous key state) run nothing again.
        if lparam & (1 << 30) == 0 {
            replace_all_requested(main);
        }
        return 0;
    }
    if message == WM_KEYDOWN {
        let ctrl = key_down(VK_CONTROL);
        match wparam as u16 {
            VK_DOWN | VK_NEXT => {
                enter_results(main, panel);
                return 0;
            }
            // Tab and Shift+Tab move on to the other field or the results.
            VK_TAB if !ctrl => {
                let from = if replace {
                    FocusStop::Replace
                } else {
                    FocusStop::Box
                };
                cycle_focus(main, panel, from, key_down(VK_SHIFT));
                return 0;
            }
            // Up from the replace field goes back to the search box above it.
            VK_UP if replace => {
                if let Some(edit) = with_view(main, |view| view.edit).flatten() {
                    unsafe {
                        SetFocus(edit);
                    }
                }
                return 0;
            }
            // Enter in the replace field replaces nothing: Replace all is Ctrl+Alt+Enter.
            VK_RETURN if replace => return 0,
            VK_RETURN => {
                open_selected(main, OpenMode::Permanent, true);
                return 0;
            }
            VK_ESCAPE => {
                // Esc clears the field; in an empty one it returns to the editor (spec §4).
                if unsafe { GetWindowTextLengthW(hwnd) } > 0 {
                    let empty = wide_null("");
                    // WM_SETTEXT comes back through this proc, which repaints the placeholder,
                    // and EN_CHANGE clears the results or the kept replace text.
                    unsafe {
                        SetWindowTextW(hwnd, empty.as_ptr());
                    }
                } else {
                    crate::window::main_window::focus_content(main);
                }
                return 0;
            }
            _ => {}
        }
    }
    // The Edit repaints only the text it changes; the placeholder must go, or come back, whole.
    let edits_text = matches!(
        message,
        WM_CHAR
            | WM_KEYDOWN
            | WM_PASTE
            | WM_CUT
            | WM_CLEAR
            | WM_UNDO
            | WM_SETTEXT
            | EM_UNDO
            | EM_REPLACESEL
    );
    let was_empty = edits_text && unsafe { GetWindowTextLengthW(hwnd) } == 0;
    let result = unsafe { DefSubclassProc(hwnd, message, wparam, lparam) };
    if edits_text && was_empty != (unsafe { GetWindowTextLengthW(hwnd) } == 0) {
        unsafe {
            InvalidateRect(hwnd, std::ptr::null(), 1);
        }
    }
    result
}
