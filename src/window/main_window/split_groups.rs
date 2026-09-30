//! Split editors: numbering, placing and moving views between groups, splitting and closing
//! groups, activating a group, and showing and styling a group's view.

use super::*;

pub(crate) const NO_ROOM_TO_SPLIT: &str = "Not enough room to split";

/// Ctrl+1..8 focus group N in layout order, and Ctrl+9 (`usize::MAX`) the last group. A group
/// that doesn't exist yet is made to the right of the last one, showing the active document, as
/// VS Code does (split editors spec §6).
pub(crate) fn focus_group_number(hwnd: HWND, index: usize) {
    let order = group_order(hwnd);
    let Some(last) = order.last().copied() else {
        return;
    };
    let index = if index == usize::MAX {
        order.len() - 1
    } else {
        index
    };
    if let Some(group) = order.get(index) {
        activate_group(hwnd, *group);
        focus_content(hwnd);
        return;
    }
    let Some(source) =
        unsafe { app_ptr(hwnd) }.map(|app| unsafe { app.as_ref() }.tabs.active_group())
    else {
        return;
    };
    remember_view(hwnd, source);
    let Some(new) = split_group(hwnd, last, crate::window::split_tree::Direction::Right) else {
        return;
    };
    if let Some(mut app) = unsafe { app_ptr(hwnd) } {
        let app = unsafe { app.as_mut() };
        if let Some(id) = app.tabs.active().map(|document| document.id) {
            let state = app.tabs.view_state_in(source, id);
            app.tabs.add_view(new, id, state);
        }
    }
    activate_group(hwnd, new);
    show_group_view(hwnd, new);
    layout_editor_and_find_bar(hwnd);
    refresh_tabs(hwnd);
    focus_content(hwnd);
}

/// Ctrl+Alt+Right and Ctrl+Alt+Left: moves the active tab to the next or previous group in layout
/// order. Past the last group a new one opens to the right; before the first there is nowhere
/// to go. A group left without tabs closes (spec §5.2).
/// Puts document `id`'s view from group `from` into group `to` at strip `index` (`None`: the
/// end): moved, or with `copy` a second view at the same position. A group that already shows
/// `id` activates that view, and a move still removes the source view (spec §6.2). The source
/// group closes when that was its last tab; the focus goes to `to`.
pub(crate) fn place_view(
    hwnd: HWND,
    from: GroupId,
    id: DocumentId,
    to: GroupId,
    index: Option<usize>,
    copy: bool,
) -> bool {
    remember_view(hwnd, from);
    let placed = unsafe { app_ptr(hwnd) }.is_some_and(|mut app| {
        let tabs = &mut unsafe { app.as_mut() }.tabs;
        if copy {
            let state = tabs.view_state_in(from, id);
            tabs.add_view_at(to, id, state, index)
        } else {
            tabs.move_view_at(from, id, to, index)
        }
    });
    if !placed {
        return false;
    }
    activate_group(hwnd, to);
    show_group_view(hwnd, from);
    show_group_view(hwnd, to);
    remove_empty_group(hwnd, from);
    layout_editor_and_find_bar(hwnd);
    refresh_tabs(hwnd);
    focus_content(hwnd);
    true
}

pub(crate) fn move_active_view(hwnd: HWND, forward: bool) {
    let Some((id, source)) = unsafe { app_ptr(hwnd) }.and_then(|app| {
        let app = unsafe { app.as_ref() };
        Some((app.tabs.active()?.id, app.tabs.active_group()))
    }) else {
        return;
    };
    let order = group_order(hwnd);
    let Some(position) = order.iter().position(|group| *group == source) else {
        return;
    };
    let neighbour = if forward {
        order.get(position + 1).copied()
    } else {
        position
            .checked_sub(1)
            .and_then(|previous| order.get(previous).copied())
    };
    let target = match neighbour {
        Some(target) => target,
        None if forward => {
            match split_group(hwnd, source, crate::window::split_tree::Direction::Right) {
                Some(new) => new,
                None => return,
            }
        }
        None => return,
    };
    place_view(hwnd, source, id, target, None, false);
}

/// Makes an empty group beside `target`, or says why not (split editors spec §4.3, §9).
pub(crate) fn split_group(
    hwnd: HWND,
    target: GroupId,
    direction: crate::window::split_tree::Direction,
) -> Option<GroupId> {
    let area = tree_area(hwnd)?;
    let dpi = unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForWindow(hwnd) }.max(96);
    // A trial id: the real one is only handed out once the window exists.
    let trial = GroupId(u32::MAX);
    let fits = unsafe { app_ptr(hwnd) }.is_some_and(|app| {
        unsafe { app.as_ref() }
            .layout
            .fits_split(target, direction, trial, area, dpi)
    });
    if !fits {
        push_notice(hwnd, NO_ROOM_TO_SPLIT.to_owned());
        return None;
    }
    let new = match create_group(hwnd) {
        Ok(new) => new,
        Err(error) => {
            push_notice(
                hwnd,
                format!("FastPad could not open a new editor group: {error}"),
            );
            return None;
        }
    };
    if let Some(mut app) = unsafe { app_ptr(hwnd) } {
        unsafe { app.as_mut() }.layout.split(target, direction, new);
    }
    Some(new)
}

/// Ctrl+\ and Ctrl+Shift+\: a new group beside the active one, showing a new view of the active
/// document at the same position (spec §5.1).
pub(crate) fn split_active_group(hwnd: HWND, direction: crate::window::split_tree::Direction) {
    let Some(source) =
        unsafe { app_ptr(hwnd) }.map(|app| unsafe { app.as_ref() }.tabs.active_group())
    else {
        return;
    };
    remember_view(hwnd, source);
    let Some(new) = split_group(hwnd, source, direction) else {
        return;
    };
    if let Some(mut app) = unsafe { app_ptr(hwnd) } {
        let app = unsafe { app.as_mut() };
        if let Some(id) = app.tabs.active().map(|document| document.id) {
            let state = app.tabs.view_state_in(source, id);
            app.tabs.add_view(new, id, state);
        }
    }
    activate_group(hwnd, new);
    show_group_view(hwnd, new);
    layout_editor_and_find_bar(hwnd);
    refresh_tabs(hwnd);
    focus_content(hwnd);
}

/// Closes every tab of group `id`, asking about each unsaved one only where no other group shows
/// it, then the group itself unless it is the only one. A cancelled prompt stops there.
pub(crate) fn close_group(hwnd: HWND, id: GroupId) {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    activate_group(hwnd, id);
    let count = || {
        unsafe { app_ptr(hwnd) }.and_then(|app| Some(unsafe { app.as_ref() }.tabs.group(id)?.len()))
    };
    while let Some(before) = count().filter(|count| *count > 0) {
        close_active_document(hwnd);
        if !identity.is_live_for(hwnd) || count().is_some_and(|after| after >= before) {
            return;
        }
    }
    remove_empty_group(hwnd, id);
}

/// Removes group `id` once it has no tabs, unless it is the only group, and activates its
/// neighbour: the next group in layout order, else the previous one.
pub(crate) fn remove_empty_group(hwnd: HWND, id: GroupId) -> bool {
    let removable = unsafe { app_ptr(hwnd) }.is_some_and(|app| {
        let app = unsafe { app.as_ref() };
        app.groups.len() > 1 && app.tabs.group(id).is_some_and(|group| group.is_empty())
    });
    if !removable {
        return false;
    }
    let order = group_order(hwnd);
    let Some(index) = order.iter().position(|group| *group == id) else {
        return false;
    };
    let Some(neighbour) = order
        .get(index + 1)
        .or_else(|| {
            index
                .checked_sub(1)
                .and_then(|previous| order.get(previous))
        })
        .copied()
    else {
        return false;
    };
    activate_group(hwnd, neighbour);
    if let Some(mut app) = unsafe { app_ptr(hwnd) } {
        unsafe { app.as_mut() }.layout.remove(id);
    }
    destroy_group(hwnd, id);
    layout_editor_and_find_bar(hwnd);
    refresh_tabs(hwnd);
    focus_content(hwnd);
    true
}

/// The group that records `document`'s changes: the first, in layout order, whose active view
/// shows it.
pub(super) fn reporting_group(hwnd: HWND, document: DocumentId) -> Option<GroupId> {
    let app = unsafe { app_ptr(hwnd) }?;
    groups_showing(unsafe { app.as_ref() }, document)
        .into_iter()
        .next()
}

/// The groups whose active view shows `document`, in layout order.
pub(super) fn groups_showing(app: &App, document: DocumentId) -> Vec<GroupId> {
    let mut order = app.layout.leaves();
    // A group not in the layout yet (between its creation and its split) comes last.
    let unplaced = app
        .tabs
        .group_ids()
        .into_iter()
        .filter(|id| !order.contains(id))
        .collect::<Vec<_>>();
    order.extend(unplaced);
    order
        .into_iter()
        .filter(|id| {
            app.tabs
                .group(*id)
                .is_some_and(|group| group.active_document() == Some(document))
        })
        .collect()
}

/// Tells the main window that `child`, a content window inside an editor group (a preview, an
/// image view, a find field), got the focus, so its group becomes the active one.
pub(crate) fn post_content_focus(child: HWND) {
    unsafe {
        PostMessageW(
            crate::platform::win32::root_window(child),
            crate::window::WM_FASTPAD_CONTENT_FOCUSED,
            child as usize,
            0,
        )
    };
}

/// The group holding the content child `child`.
pub(super) fn group_of_child(hwnd: HWND, child: HWND) -> Option<GroupId> {
    let app = unsafe { app_ptr(hwnd) }?;
    unsafe { app.as_ref() }.group_containing(child)
}

/// Makes `id` the active group, which commands act on; returns whether it changed. The focus
/// stays where it is: this follows a click or focus arriving in the group.
pub(crate) fn activate_group(hwnd: HWND, id: GroupId) -> bool {
    let previous = unsafe { app_ptr(hwnd) }.and_then(|mut app| {
        let app = unsafe { app.as_mut() };
        let previous = app.tabs.active_group();
        app.tabs.set_active_group(id).then_some(previous)
    });
    let Some(previous) = previous else {
        return false;
    };
    for group in [previous, id] {
        if let Some(window) = with_group_id(hwnd, group, |state| state.hwnd) {
            // Only the active group floats the preview buttons.
            layout_group(hwnd, window);
            unsafe { InvalidateRect(window, std::ptr::null(), 0) };
        }
    }
    invalidate_status_bar(hwnd);
    invalidate_title_strip(hwnd);
    crate::window::side_panel::active_tab_changed(hwnd);
    true
}

/// A press on the group window `group` makes its group active, and takes the focus along when
/// another group had it: typing then goes where the commands go.
pub(crate) fn press_group_window(hwnd: HWND, group: HWND) {
    let Some(id) = group_id_of(hwnd, group) else {
        return;
    };
    let focused = group_of_child(hwnd, unsafe { GetFocus() });
    activate_group(hwnd, id);
    if focused.is_some_and(|focused| focused != id) {
        focus_content(hwnd);
    }
}

/// The focus arriving in the group window `group` makes its group active.
pub(crate) fn activate_group_window(hwnd: HWND, group: HWND) {
    if let Some(id) = group_id_of(hwnd, group) {
        activate_group(hwnd, id);
    }
}

/// Group `id`'s editor shows its active view's document where that view was left. An image tab
/// or an empty group shows no text: the editor holds an empty placeholder then, and a group that
/// isn't active hides it (`refresh_tabs` does that for the active group).
pub(crate) fn show_group_view(hwnd: HWND, id: GroupId) -> bool {
    let target = unsafe { app_ptr(hwnd) }.and_then(|app| {
        let app = unsafe { app.as_ref() };
        let editor = app.group(id)?.editor.clone();
        let document = app.tabs.group(id)?.active_document();
        let handle = document
            .and_then(|document| app.tabs.document(document))
            .and_then(Document::text_handle)
            .cloned();
        let state = document
            .map(|document| app.tabs.view_state_in(id, document))
            .unwrap_or_default();
        Some((editor, handle, state, app.tabs.active_group() == id))
    });
    let Some((editor, handle, state, active)) = target else {
        return false;
    };
    let text = handle.is_some();
    let handle = match handle {
        Some(handle) => handle,
        None => match editor.create_document() {
            Ok(blank) => blank,
            Err(_) => return false,
        },
    };
    if editor.use_document(&handle).is_err() {
        return false;
    }
    if text {
        let _ = editor.apply_view_state(state);
        style_group_view(hwnd, id);
    }
    if !active {
        unsafe { ShowWindow(editor.hwnd(), if text { SW_SHOWNA } else { SW_HIDE }) };
        crate::window::preview_host::in_group(id, || {
            crate::window::preview_host::sync_visibility(hwnd);
        });
    }
    true
}

/// Gives group `id`'s editor the style table of the language its active view's document has, and
/// the configured font back. The lexer belongs to the document but the styles to each editor, so
/// an editor that starts showing a document, or outlives a theme change, needs this.
pub(super) fn style_group_view(hwnd: HWND, id: GroupId) {
    let target = unsafe { app_ptr(hwnd) }.and_then(|app| {
        let app = unsafe { app.as_ref() };
        let editor = app.group(id)?.editor.clone();
        let document = app.tabs.document(app.tabs.group(id)?.active_document()?)?;
        let palette = Palette::for_cached_theme(app.theme, app.settings.theme);
        Some((editor, document.language, app.settings.clone(), palette))
    });
    let Some((editor, language, settings, palette)) = target else {
        return;
    };
    if crate::languages::apply_styles(&editor, language, effective_theme(hwnd)).is_ok() {
        apply_settings_to(&editor, &settings, palette);
    }
}

/// Records where group `id`'s editor is in its active text tab, so showing that view again lands
/// there (split editors spec §3.2). Called before anything else takes over that editor.
pub(crate) fn remember_view(hwnd: HWND, id: GroupId) {
    let shown = unsafe { app_ptr(hwnd) }.and_then(|app| {
        let app = unsafe { app.as_ref() };
        let document = app.tabs.group(id)?.active_document()?;
        let editor = app.group(id)?.editor.clone();
        (!app.tabs.document(document)?.is_image()).then_some((document, editor))
    });
    if let Some((document, editor)) = shown
        && let Ok(state) = editor.view_state()
        && let Some(mut app) = unsafe { app_ptr(hwnd) }
    {
        unsafe { app.as_mut() }
            .tabs
            .set_view_state_in(id, document, state);
    }
}

pub(crate) fn invalidate_title_strip(hwnd: HWND) {
    unsafe {
        InvalidateRect(hwnd, std::ptr::null(), 0);
    }
    // A document's title shows in every group with a view of it.
    let groups = unsafe { app_ptr(hwnd) }
        .map(|app| {
            unsafe { app.as_ref() }
                .groups
                .iter()
                .map(|group| group.hwnd)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    for group in groups {
        unsafe {
            InvalidateRect(group, std::ptr::null(), 0);
        }
    }
    // The Open Editors rows show what the strip does.
    crate::window::notebook_view::editors_changed(hwnd);
}

/// Refreshes the retained tab-view snapshot after a document's title-affecting field (e.g. its
/// untitled label) changed in place, and repaints the title strip.
pub(crate) fn refresh_tab_view(hwnd: HWND) {
    if let Some(app) = unsafe { app_ptr(hwnd) } {
        unsafe { app.as_ref() }.tabs.refresh_view();
    }
    invalidate_title_strip(hwnd);
}
