//! The main window procedure and its message dispatch, `WM_NCCREATE`, painting, and the editor's
//! `WM_NOTIFY` notifications.

use super::*;

pub(super) unsafe extern "system" fn main_window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_NCCREATE => unsafe { on_nc_create(hwnd, wparam, lparam) },
        WM_SIZE => {
            layout_editor_and_find_bar(hwnd);
            invalidate_title_strip(hwnd);
            0
        }
        WM_SETFOCUS => {
            // The frame gets the focus when the window is activated again: an inline name edit
            // that was open when FastPad lost the focus takes it back (inline naming spec §5.3).
            if menu_mode(hwnd).is_none() && crate::window::inline_name::refocus(hwnd) {
                return 0;
            }
            // With no tab open the editor is hidden and the frame itself keeps the focus, as it
            // does in menu mode to take the menu keys. A Full preview takes the editor's place.
            if menu_mode(hwnd).is_none()
                && tab_count(hwnd) > 0
                && let Some(target) = content_focus_target(hwnd)
            {
                unsafe {
                    SetFocus(target);
                }
            }
            0
        }
        // Focus leaving the frame ends menu mode. Activating an inactive window from SetFocus
        // reports a loss to the frame itself, which is not one.
        WM_KILLFOCUS => {
            if wparam as HWND != hwnd && menu_mode(hwnd).is_some_and(|mode| !mode.open) {
                exit_menu_mode(hwnd);
            }
            0
        }
        WM_KEYDOWN | WM_SYSKEYDOWN
            if menu_mode(hwnd).is_some() && handle_menu_key(hwnd, message, wparam) =>
        {
            0
        }
        WM_CLOSE => {
            if file_population_active(hwnd) {
                return 0;
            }
            // A launch forwarded just before the review must be handled, not lost with the window.
            drain_ipc_requests(hwnd);
            crate::window::library_host::autosave_all(hwnd);
            if !save_session_for_close(hwnd) {
                let Some(discarded) = review_dirty_documents(hwnd) else {
                    return 0;
                };
                remove_session_snapshots(hwnd, &discarded);
            }
            crate::window::library_host::flush_before_close(hwnd);
            // Before `shutdown_ipc`, which gives up the instance mutex that marks the primary.
            save_placement(hwnd);
            shutdown_ipc(hwnd);
            clear_documents_for_shutdown(hwnd);
            unsafe {
                DestroyWindow(hwnd);
            }
            0
        }
        WM_DESTROY => {
            // A running text search stops reading: its posts would fail from here on anyway.
            crate::window::text_search_host::cancel(hwnd);
            // A replace stops too. Its write worker finishes the note in hand and is waited for
            // here: were the process to exit mid-save, that note could be left half written. On
            // an offline drive this can hold the close for one save's I/O timeout.
            crate::window::text_search_host::cancel_replace(hwnd);
            crate::window::text_search_host::join_writers(hwnd);
            // A copy into the notebook stops after the file in hand, and is waited for the same
            // way, so no copied file is left half written.
            crate::window::copy_host::stop(hwnd);
            // The panel goes with this window, not through `side_panel::destroy_windows`: its
            // drop target is revoked first, which releases it.
            if let Some((_, panel)) = crate::window::side_panel::windows(hwnd) {
                crate::window::panel_drop::revoke(panel);
            }
            // Dropping it here destroys the icon (`LogoIcon::drop`); `WM_NCDESTROY` still frees
            // the rest of App, but the logo shouldn't wait for that.
            if let Some(mut app) = unsafe { app_ptr(hwnd) } {
                unsafe { app.as_mut() }.logo_icon = None;
            }
            unsafe {
                KillTimer(hwnd, crate::recovery::RECOVERY_TIMER_ID);
                KillTimer(hwnd, crate::window::preview_host::PREVIEW_TIMER_ID);
                KillTimer(hwnd, crate::window::library_host::LIBRARY_WRITE_TIMER_ID);
                KillTimer(hwnd, crate::window::library_host::AUTOSAVE_TIMER_ID);
                PostQuitMessage(0);
            }
            0
        }
        WM_TIMER if wparam == crate::recovery::RECOVERY_TIMER_ID => {
            snapshot_when_idle(hwnd);
            0
        }
        WM_TIMER if wparam == crate::window::preview_host::PREVIEW_TIMER_ID => {
            crate::window::preview_host::flush(hwnd);
            0
        }
        WM_TIMER if wparam == crate::window::library_host::LIBRARY_WRITE_TIMER_ID => {
            crate::window::library_host::flush_now(hwnd);
            0
        }
        WM_TIMER if wparam == crate::window::library_host::AUTOSAVE_TIMER_ID => {
            crate::window::library_host::autosave_active(hwnd);
            0
        }
        WM_TIMER if wparam == FIND_COUNT_TIMER_ID => {
            find_count_timer(hwnd);
            0
        }
        WM_TIMER if wparam == crate::window::text_search_host::TEXT_SEARCH_TIMER_ID => {
            crate::window::text_search_host::timer(hwnd);
            0
        }
        WM_TIMER if wparam == crate::window::text_search_host::REPLACE_TIMER_ID => {
            crate::window::text_search_host::replace_timer(hwnd);
            0
        }
        WM_DROPFILES => {
            let drop = wparam as windows_sys::Win32::UI::Shell::HDROP;
            let paths = crate::platform::win32::dropped_paths(drop);
            let mut point = windows_sys::Win32::Foundation::POINT { x: 0, y: 0 };
            let at = unsafe { windows_sys::Win32::UI::Shell::DragQueryPoint(drop, &mut point) };
            unsafe { windows_sys::Win32::UI::Shell::DragFinish(drop) };
            // The group under the drop opens it: a strip, a preview or an image view (plan
            // amendment 6).
            if at != 0 {
                unsafe { windows_sys::Win32::Graphics::Gdi::ClientToScreen(hwnd, &mut point) };
                if let Some((group, _)) = group_at(hwnd, point) {
                    activate_group(hwnd, group);
                }
            }
            crate::window::library_host::files_dropped(hwnd, paths);
            0
        }
        WM_ACTIVATEAPP => {
            crate::window::library_host::activation_changed(hwnd, wparam != 0);
            if wparam != 0 {
                crate::window::image_host::check_disk(hwnd);
            }
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
        WM_PAINT => {
            sync_window_title(hwnd);
            let paint_title_strip = |hwnd, _, _, _| {
                let covered = group_strip_bounds(hwnd);
                let sashes = tree_layout(hwnd)
                    .map(|layout| layout.sashes.iter().map(|sash| sash.rect).collect())
                    .unwrap_or_default();
                let status = current_status_bar(hwnd);
                let (palette, fonts, pointer) = title_chrome(hwnd);
                let headings = menu_headings(hwnd);
                unsafe {
                    crate::window::titlebar::paint(
                        hwnd,
                        &crate::window::titlebar::TitlePaint {
                            covered,
                            sashes,
                            status: status.as_ref(),
                            palette,
                            fonts,
                            pointer,
                            menu: menu_mode(hwnd).map(|mode| (mode, headings.as_slice())),
                        },
                    )
                };
                0
            };
            let complete_first_paint = |hwnd| unsafe {
                mark_first_paint_complete(hwnd);
            };
            unsafe {
                handle_paint_with(
                    hwnd,
                    message,
                    wparam,
                    lparam,
                    paint_title_strip,
                    complete_first_paint,
                )
            }
        }
        WM_NCHITTEST => unsafe {
            crate::window::titlebar::nonclient_hit_test(hwnd, wparam, lparam)
        },
        WM_NCCALCSIZE => unsafe { crate::window::titlebar::reclaim_caption(hwnd, wparam, lparam) },
        WM_GETMINMAXINFO => unsafe {
            crate::window::titlebar::constrain_maximized_window(hwnd, lparam)
        },
        WM_MOUSEMOVE if drag_sash(hwnd, lparam) => 0,
        WM_MOUSEMOVE => {
            hover_menu_heading(hwnd, lparam);
            crate::window::titlebar::track_pointer_leave(hwnd, false);
            let target = client_title_target(hwnd, lparam);
            update_title_pointer(hwnd, |pointer| pointer.hover(target));
            0
        }
        WM_MOUSELEAVE => {
            update_title_pointer(hwnd, |pointer| pointer.leave(false));
            0
        }
        WM_NCMOUSEMOVE => {
            let target = HitTarget::from_nonclient_code(wparam);
            if target.is_some() {
                crate::window::titlebar::track_pointer_leave(hwnd, true);
            }
            update_title_pointer(hwnd, |pointer| pointer.hover(target));
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
        WM_NCMOUSELEAVE => {
            update_title_pointer(hwnd, |pointer| pointer.leave(true));
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
        windows_sys::Win32::UI::WindowsAndMessaging::WM_SETCURSOR if set_sash_cursor(hwnd) => 1,
        WM_LBUTTONDOWN => {
            let (x, y) = (
                (lparam as u32 & 0xffff) as u16 as i16 as i32,
                ((lparam as u32 >> 16) & 0xffff) as u16 as i16 as i32,
            );
            if press_sash(hwnd, x, y) {
                return 0;
            }
            if menu_mode(hwnd).is_some() {
                match menu_band::heading_at(&menu_headings(hwnd), x, y) {
                    Some(index) => {
                        open_menu(hwnd, index);
                        return 0;
                    }
                    None => exit_menu_mode(hwnd),
                }
            }
            let target = client_title_target(hwnd, lparam);
            update_title_pointer(hwnd, |pointer| pointer.hover(target).press(target));
            0
        }
        // Over a title-row strip's empty space the caption double-click opens a tab and the
        // right-click opens the strip menu, as the strip does (spec §4.1).
        WM_NCLBUTTONDBLCLK
            if wparam == windows_sys::Win32::UI::WindowsAndMessaging::HTCAPTION as usize
                && caption_strip_point(hwnd, lparam).is_some() =>
        {
            exit_menu_mode(hwnd);
            if let Some((group, ..)) = caption_strip_point(hwnd, lparam) {
                activate_group_window(hwnd, group);
            }
            execute_command(hwnd, CommandId::New);
            0
        }
        windows_sys::Win32::UI::WindowsAndMessaging::WM_NCRBUTTONUP
            if wparam == windows_sys::Win32::UI::WindowsAndMessaging::HTCAPTION as usize =>
        {
            match caption_strip_point(hwnd, lparam) {
                Some((group, x, y)) => {
                    exit_menu_mode(hwnd);
                    activate_group_window(hwnd, group);
                    show_group_strip_menu(hwnd, group, x, y);
                    0
                }
                None => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
            }
        }
        // DefWindowProc would run its own classic caption-button tracking loop over our strip.
        WM_NCLBUTTONDOWN | WM_NCLBUTTONDBLCLK
            if HitTarget::from_nonclient_code(wparam).is_some() =>
        {
            let target = HitTarget::from_nonclient_code(wparam);
            update_title_pointer(hwnd, |pointer| pointer.hover(target).press(target));
            0
        }
        WM_NCLBUTTONUP if HitTarget::from_nonclient_code(wparam).is_some() => {
            let target = HitTarget::from_nonclient_code(wparam);
            let mut activated = None;
            update_title_pointer(hwnd, |pointer| {
                let (next, released) = pointer.release(target);
                activated = released;
                next
            });
            if let Some(target) = activated {
                run_caption_button(hwnd, target);
            }
            0
        }
        WM_LBUTTONUP if end_sash_drag(hwnd) => {
            unsafe { ReleaseCapture() };
            0
        }
        WM_CAPTURECHANGED => {
            end_sash_drag(hwnd);
            0
        }
        WM_LBUTTONUP => {
            update_title_pointer(hwnd, |pointer| pointer.release(None).0);
            let point = crate::window::titlebar::Point::new(
                (lparam as u32 & 0xffff) as u16 as i16 as i32,
                ((lparam as u32 >> 16) & 0xffff) as u16 as i16 as i32,
            );
            if notice_contains(hwnd, point.y) {
                dismiss_notifications(hwnd);
            }
            0
        }
        WM_NOTIFY => {
            handle_editor_notification(hwnd, lparam);
            0
        }
        WM_FASTPAD_ACCESSIBLE_SELECT => handle_accessible_select(hwnd, wparam, lparam),
        WM_COMMAND
            if lparam != 0
                && ((wparam >> 16) & 0xffff) as u32 == EN_CHANGE
                && command_palette_owns(hwnd, lparam as HWND) =>
        {
            refilter_command_palette(hwnd);
            0
        }
        // A find field's text changed: it is kept for screen readers, and typing in the query
        // clears its no-match outline until the next search. The replacement field changes
        // nothing about the match.
        WM_COMMAND
            if lparam != 0
                && ((wparam >> 16) & 0xffff) as u32 == EN_CHANGE
                && find_bar_owns(hwnd, lparam as HWND) =>
        {
            find_field_changed(hwnd, lparam as HWND);
            0
        }
        WM_CTLCOLOREDIT if find_bar_owns(hwnd, lparam as HWND) => unsafe { app_ptr(hwnd) }
            .and_then(|app| {
                unsafe { app.as_ref() }
                    .find_bar()
                    .map(|bar| bar.control_color(wparam as HDC) as LRESULT)
            })
            .unwrap_or(0),
        WM_CTLCOLOREDIT | WM_CTLCOLORBTN if name_box_owns(hwnd, lparam as HWND) => {
            unsafe { app_ptr(hwnd) }
                .and_then(|app| {
                    let name_box = unsafe { app.as_ref() }.name_box.as_ref()?;
                    let dc = wparam as HDC;
                    Some(if message == WM_CTLCOLORBTN {
                        name_box.button_color(dc)
                    } else {
                        name_box.control_color(dc)
                    } as LRESULT)
                })
                .unwrap_or(0)
        }
        WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX if command_palette_owns(hwnd, lparam as HWND) => {
            with_command_palette(hwnd, |palette| {
                palette.control_color(wparam as HDC, lparam as HWND) as LRESULT
            })
            .unwrap_or(0)
        }
        WM_DRAWITEM if lparam != 0 => {
            let item = unsafe { &*(lparam as *const DRAWITEMSTRUCT) };
            if command_palette_owns(hwnd, item.hwndItem) {
                with_command_palette(hwnd, |palette| palette.draw_item(item));
            }
            1
        }
        // The name box's own buttons; their IDs are not command IDs.
        WM_COMMAND if lparam != 0 && name_box_owns(hwnd, lparam as HWND) => {
            if ((wparam >> 16) & 0xffff) as u32 == BN_CLICKED {
                match (wparam & 0xffff) as u16 {
                    crate::window::name_box::NAME_BOX_SAVE_ID => {
                        crate::window::library_host::name_box_submit(hwnd)
                    }
                    crate::window::name_box::NAME_BOX_BROWSE_ID => {
                        crate::window::library_host::name_box_browse(hwnd)
                    }
                    _ => {}
                }
            }
            0
        }
        WM_COMMAND => {
            if let Ok(command) = CommandId::try_from((wparam & 0xffff) as u16) {
                execute_command(hwnd, command);
            }
            0
        }
        // A tapped Alt or F10 toggles the menu band; Alt+letter opens that heading's dropdown.
        // Alt+Space (the system menu) and unknown letters keep the default handling.
        WM_SYSCOMMAND if wparam & 0xfff0 == SC_KEYMENU as usize => {
            if lparam == 0 {
                if menu_mode(hwnd).is_some() {
                    exit_menu_mode(hwnd);
                } else {
                    enter_menu_mode(hwnd, 0);
                }
                return 0;
            }
            match menu_band::mnemonic_heading(lparam as u32) {
                Some(index) => {
                    open_menu(hwnd, index);
                    0
                }
                None => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
            }
        }
        WM_GETOBJECT if lparam as i32 == OBJID_CLIENT => {
            let provider = ensure_accessibility(hwnd);
            if provider.is_null() {
                0
            } else {
                unsafe { accessibility::object_result(provider, wparam) }
            }
        }
        WM_DPICHANGED => {
            let suggested = unsafe { &*(lparam as *const RECT) };
            unsafe {
                SetWindowPos(
                    hwnd,
                    std::ptr::null_mut(),
                    suggested.left,
                    suggested.top,
                    suggested.right - suggested.left,
                    suggested.bottom - suggested.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
            let dpi = (wparam & 0xffff) as u32;
            // Replaces (and drops, which destroys) any icon loaded for the old DPI. The bar's own
            // resize below repaints it, so no separate invalidate is needed here.
            ensure_logo_icon(hwnd, dpi);
            // The suggested rectangle may keep the size, and then no WM_SIZE re-lays out the
            // sidebar and bands for the new DPI.
            refresh_metrics(hwnd, dpi);
            0
        }
        WM_SETTINGCHANGE | WM_THEMECHANGED | WM_DWMCOLORIZATIONCOLORCHANGED => {
            // The text-size value is re-read on every setting change (the parameter string is
            // not reliable across Windows versions); only a changed value costs a relayout.
            if message == WM_SETTINGCHANGE && crate::window::design::text_scale::refresh() {
                let dpi = unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForWindow(hwnd) }.max(96);
                refresh_metrics(hwnd, dpi);
            }
            refresh_theme(hwnd);
            unsafe {
                InvalidateRect(hwnd, std::ptr::null(), 1);
            }
            0
        }
        WM_NCDESTROY => {
            let app = unsafe { take_app(hwnd) };
            if let Some(app) = app.as_ref() {
                app.invalidate_window(hwnd);
            }
            let result = unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
            drop(app);
            result
        }
        _ => {
            // The worker's boxed result: handled at once, since a held message would lose it.
            if message == crate::window::WM_FASTPAD_LIBRARY_READY {
                crate::window::library_host::library_ready(hwnd, lparam);
                // The notebook's autosave switch may be known now.
                crate::window::settings_dialog::refresh_open(hwnd);
                return 0;
            }
            if message == crate::window::WM_FASTPAD_FILES_DROPPED {
                crate::window::library_host::editor_files_dropped(hwnd, wparam, lparam);
                return 0;
            }
            if message == crate::window::WM_FASTPAD_NOTEBOOK_CHECKED {
                crate::window::library_host::notebook_checked(hwnd, lparam);
                return 0;
            }
            // Focus left the Notebook tree's name field (inline naming spec §5.3): leaving
            // FastPad keeps the edit; a modal prompt that took it holds the commit until it ends.
            if message == crate::window::WM_FASTPAD_INLINE_NAME_LEFT {
                if wparam == crate::window::inline_name::LEFT_FASTPAD {
                    crate::window::inline_name::focus_left_fastpad(hwnd);
                } else if !crate::window::modal::hold_while_modal(hwnd, message) {
                    crate::window::inline_name::focus_left(hwnd);
                }
                return 0;
            }
            if message == crate::window::WM_FASTPAD_TEXT_SEARCH_BATCH {
                crate::window::text_search_host::batch_arrived(hwnd, lparam);
                return 0;
            }
            if message == crate::window::WM_FASTPAD_REPLACE_COUNTED {
                crate::window::text_search_host::replace_counted(hwnd, lparam);
                return 0;
            }
            if message == crate::window::WM_FASTPAD_REPLACE_WRITTEN {
                crate::window::text_search_host::replace_written(hwnd, lparam);
                return 0;
            }
            if message == crate::window::WM_FASTPAD_REPLACE_RELOADED {
                crate::window::text_search_host::replace_reloaded(hwnd, lparam);
                return 0;
            }
            if message == crate::window::WM_FASTPAD_COPY_DONE {
                crate::window::copy_host::copy_done(hwnd, lparam);
                return 0;
            }
            if message == crate::window::WM_FASTPAD_PANEL_DROPPED {
                crate::window::copy_host::panel_dropped(hwnd, lparam);
                return 0;
            }
            // A nested modal loop dispatches whatever is queued. Deferred startup units and the
            // IPC drain wait for it to end so they cannot change the document it acts on.
            if (message == crate::window::WM_FASTPAD_IPC_REQUEST
                || classify_deferred_message(message, false).is_some())
                && crate::window::modal::hold_while_modal(hwnd, message)
            {
                return 0;
            }
            if message == crate::window::WM_FASTPAD_IPC_REQUEST {
                return handle_ipc_requests(hwnd);
            }
            match message {
                crate::window::WM_FASTPAD_CONTENT_FOCUSED => {
                    if let Some(id) = group_of_child(hwnd, wparam as HWND) {
                        activate_group(hwnd, id);
                    }
                    return 0;
                }
                crate::window::WM_FASTPAD_PREVIEW_ESCAPE => {
                    if let Some(id) = group_of_child(hwnd, wparam as HWND) {
                        crate::window::preview_host::escape(hwnd, id);
                    }
                    return 0;
                }
                crate::window::WM_FASTPAD_PREVIEW_PARSED => {
                    crate::window::preview_host::parsed(hwnd, lparam);
                    return 0;
                }
                crate::window::WM_FASTPAD_IMAGE_STATUS => {
                    invalidate_status_bar(hwnd);
                    return 0;
                }
                crate::window::WM_FASTPAD_PREVIEW_LINK => {
                    match group_of_child(hwnd, wparam as HWND) {
                        Some(id) => crate::window::preview_host::follow_link(hwnd, id, lparam),
                        None if lparam != 0 => {
                            drop(unsafe { Box::from_raw(lparam as *mut String) });
                        }
                        None => {}
                    }
                    return 0;
                }
                crate::window::WM_FASTPAD_PREVIEW_HOVER => {
                    match group_of_child(hwnd, wparam as HWND) {
                        Some(id) => crate::window::preview_host::hover_link(hwnd, id, lparam),
                        None if lparam != 0 => {
                            drop(unsafe { Box::from_raw(lparam as *mut Option<String>) });
                        }
                        None => {}
                    }
                    return 0;
                }
                crate::window::WM_FASTPAD_PREVIEW_REFRESH => {
                    if let Some(id) = group_of_child(hwnd, wparam as HWND) {
                        crate::window::preview_host::refresh(hwnd, id);
                    }
                    return 0;
                }
                crate::window::WM_FASTPAD_PREVIEW_SCROLLED => {
                    if let Some(id) = group_of_child(hwnd, lparam as HWND) {
                        crate::window::preview_host::preview_scrolled(hwnd, id, wparam);
                    }
                    return 0;
                }
                _ => {}
            }
            if message == crate::window::WM_FASTPAD_DIAGNOSTIC_PREVIEW
                && unsafe { app_ptr(hwnd) }
                    .is_some_and(|app| unsafe { app.as_ref() }.launch.diagnostic)
            {
                return crate::window::preview_host::diagnostic(hwnd, wparam);
            }
            if message == crate::window::WM_FASTPAD_DIAGNOSTIC_JSON_COUNT
                && unsafe { app_ptr(hwnd) }
                    .is_some_and(|app| unsafe { app.as_ref() }.launch.diagnostic)
            {
                return crate::languages::json_invocation_count() as LRESULT;
            }
            if let Some(action) = classify_deferred_message(message, input_pending()) {
                return handle_deferred(hwnd, action);
            }
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
    }
}

pub(super) unsafe fn handle_paint_with<D, C>(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    default_window_proc: D,
    complete_first_paint: C,
) -> LRESULT
where
    D: FnOnce(HWND, u32, WPARAM, LPARAM) -> LRESULT,
    C: FnOnce(HWND),
{
    let identity = unsafe { window_identity(hwnd) };
    let result = default_window_proc(hwnd, message, wparam, lparam);
    if identity
        .as_ref()
        .is_some_and(|identity| identity.is_live_for(hwnd))
    {
        complete_first_paint(hwnd);
    }
    result
}

unsafe fn on_nc_create(hwnd: HWND, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let Some(mut app) = (unsafe { take_create_context_app(lparam) }) else {
        return 0;
    };
    if !app.bind_window(hwnd) {
        return 0;
    }
    store_app(hwnd, app);
    // The default handling stores the window text, which the taskbar button and Alt+Tab show.
    unsafe { DefWindowProcW(hwnd, WM_NCCREATE, wparam, lparam) }
}

fn handle_editor_notification(hwnd: HWND, lparam: LPARAM) {
    if file_population_active(hwnd) {
        return;
    }
    if lparam == 0 {
        return;
    }
    let notification = unsafe { &*(lparam as *const NMHDR) };
    // Every group's editor reports here; each one showing a document reports its changes.
    let Some((group, active, document, editor)) = unsafe { app_ptr(hwnd) }.and_then(|app| {
        let app = unsafe { app.as_ref() };
        let state = app
            .groups
            .iter()
            .find(|group| group.editor.hwnd() == notification.hwndFrom)?;
        Some((
            state.id,
            app.tabs.active_group() == state.id,
            app.tabs.group(state.id)?.active_document(),
            state.editor.clone(),
        ))
    }) else {
        return;
    };
    if notification.code == crate::editor::scintilla_constants::SCN_FOCUSIN {
        activate_group(hwnd, group);
        return;
    }
    if notification.code == crate::editor::scintilla_constants::SCN_UPDATEUI {
        if active {
            invalidate_status_bar(hwnd);
        }
        let update = unsafe { &*(lparam as *const crate::editor::ScintillaNotification) };
        if update.updated as u32 & crate::editor::scintilla_constants::SC_UPDATE_V_SCROLL != 0 {
            crate::window::preview_host::editor_scrolled(hwnd, group);
        }
        return;
    }
    if notification.code == crate::editor::scintilla_constants::SCN_ZOOM {
        let _ = editor.remeasure_line_numbers();
        return;
    }
    // A document shown in several groups notifies once per editor: its own changes are recorded
    // by one of them (split editors spec §3.3).
    let Some(document) =
        document.filter(|document| reporting_group(hwnd, *document) == Some(group))
    else {
        if notification.code == crate::editor::scintilla_constants::SCN_MODIFIED {
            let modification = unsafe { &*(lparam as *const crate::editor::ScintillaNotification) };
            if modification.lines_added != 0 {
                let _ = editor.refresh_line_numbers();
            }
        }
        return;
    };
    if notification.code == crate::editor::scintilla_constants::SCN_MODIFIED {
        let modification = unsafe { &*(lparam as *const crate::editor::ScintillaNotification) };
        let text_changes = crate::editor::scintilla_constants::SC_MOD_INSERTTEXT
            | crate::editor::scintilla_constants::SC_MOD_DELETETEXT;
        let text_change = modification.modification_type & text_changes as i32 != 0;
        if !text_change {
            return;
        }
        if modification.lines_added != 0 {
            let _ = editor.refresh_line_numbers();
        }
        let (promoted, showing) = unsafe { app_ptr(hwnd) }
            .map(|mut app| {
                let app = unsafe { app.as_mut() };
                let promoted = app.tabs.note_text_change(document);
                (promoted, groups_showing(app, document))
            })
            .unwrap_or_default();
        if promoted {
            invalidate_title_strip(hwnd);
        }
        crate::window::library_host::text_changed(
            hwnd,
            group,
            modification.position.max(0) as usize,
        );
        crate::window::library_host::schedule_autosave(hwnd);
        schedule_find_count(hwnd);
        for shown in showing {
            crate::window::preview_host::record_edit(hwnd, shown, modification);
        }
        return;
    }
    let dirty = match notification.code {
        crate::editor::scintilla_constants::SCN_SAVEPOINTLEFT => true,
        crate::editor::scintilla_constants::SCN_SAVEPOINTREACHED => false,
        _ => return,
    };
    let changed = unsafe { app_ptr(hwnd) }
        .map(|mut app| unsafe { app.as_mut() }.tabs.set_dirty(document, dirty))
        .unwrap_or(false);
    if changed {
        invalidate_title_strip(hwnd);
    }
    crate::window::notebook_view::editors_changed(hwnd);
}
