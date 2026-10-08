//! Input decoding for the Settings dialog: what clicks and keys mean to the model, and the
//! window procedure that routes every message.

use super::*;

/// What releasing the mouse on `hit` does.
pub(super) fn click_effect(dialog: &mut Dialog, hit: Hit) -> Effect {
    let view = &dialog.view;
    match hit {
        Hit::Close | Hit::TitleClose => Effect::Close,
        Hit::EditIni => Effect::EditIni,
        Hit::Nav(page) => Effect::ShowPage(page),
        Hit::Row(row, _) if !view.enabled(row) => Effect::None,
        Hit::Row(row, Part::Whole) => match (row.control(), row.toggle()) {
            (Control::Check, Some(toggle)) => Effect::Apply(
                crate::window::settings_model::SettingsAction::Toggle(toggle),
            ),
            (Control::Dropdown, _) => Effect::OpenDropdown(row),
            _ => Effect::None,
        },
        Hit::Row(row, Part::Segment(index)) => view
            .segment_action(row, index)
            .map_or(Effect::None, Effect::Apply),
        Hit::Row(row, part @ (Part::Minus | Part::Plus)) => {
            dialog.model.typed = None;
            let current = stepper_value(row, &view.settings);
            step_effect(row, current, step_value(row, current, part == Part::Plus))
        }
        Hit::Row(_, Part::Value) => Effect::Repaint,
        // Page clicks go through `page_click`.
        Hit::Page(_) => Effect::None,
    }
}

/// The focus a click on `hit` moves to.
pub(super) fn focus_of(hit: Hit) -> Focus {
    match hit {
        Hit::Row(row, _) => Focus::Row(row),
        Hit::Nav(_) => Focus::Nav,
        Hit::EditIni => Focus::EditIni,
        Hit::Close | Hit::TitleClose => Focus::Close,
        Hit::Page(crate::window::shortcuts_page::PageHit::RecordToggle) => Focus::Search,
        Hit::Page(_) => Focus::Table,
    }
}

pub(super) fn list_key(virtual_key: u16) -> Option<ListKey> {
    Some(match virtual_key {
        VK_UP => ListKey::Up,
        VK_DOWN => ListKey::Down,
        VK_PRIOR => ListKey::PageUp,
        VK_NEXT => ListKey::PageDown,
        VK_HOME => ListKey::Home,
        VK_END => ListKey::End,
        VK_RETURN => ListKey::Enter,
        VK_ESCAPE => ListKey::Escape,
        _ => return None,
    })
}

pub(super) fn model_key(virtual_key: u16) -> Option<Key> {
    Some(match virtual_key {
        VK_TAB => Key::Tab {
            back: unsafe { GetKeyState(i32::from(VK_SHIFT)) } < 0,
        },
        VK_SPACE => Key::Space,
        VK_RETURN => Key::Enter,
        VK_LEFT => Key::Left,
        VK_RIGHT => Key::Right,
        VK_UP => Key::Up,
        VK_DOWN => Key::Down,
        VK_ESCAPE => Key::Escape,
        _ => return None,
    })
}

pub(super) fn key_down(hwnd: HWND, virtual_key: u16) {
    // With a dropdown open, its keys go to the list; Tab closes it and moves on.
    let list_outcome = state(hwnd).and_then(|dialog| {
        let (_, list) = dialog.list.as_ref()?;
        let key = list_key(virtual_key)?;
        Some(list.key(key, unsafe { GetTickCount() }))
    });
    match list_outcome {
        Some(ListOutcome::Picked(index)) => return pick(hwnd, index),
        Some(ListOutcome::Dismissed) => {
            close_list(hwnd);
            return;
        }
        Some(_) => return,
        None => {
            if virtual_key == VK_TAB {
                close_list(hwnd);
            }
        }
    }
    if shortcuts_key(hwnd, virtual_key) {
        return;
    }
    let ctrl = unsafe { GetKeyState(i32::from(VK_CONTROL)) } < 0;
    let key = if ctrl && matches!(virtual_key, VK_PRIOR | VK_NEXT) {
        Some(Key::NextPage {
            back: virtual_key == VK_PRIOR,
        })
    } else {
        model_key(virtual_key)
    };
    let Some(key) = key else {
        return;
    };
    let effect = state(hwnd).map(|dialog| dialog.model.key(key, &dialog.view));
    if let Some(effect) = effect {
        run(hwnd, effect);
    }
}

pub(super) fn char_typed(hwnd: HWND, c: char) {
    let now = unsafe { GetTickCount() };
    let in_list = state(hwnd).and_then(|dialog| {
        let (_, list) = dialog.list.as_ref()?;
        Some(list.key(ListKey::Char(c), now))
    });
    if in_list.is_some() {
        return;
    }
    let key = match c {
        '\u{8}' => Key::Backspace,
        c if c.is_control() => return,
        c => Key::Char(c),
    };
    let effect = state(hwnd).map(|dialog| dialog.model.key(key, &dialog.view));
    if let Some(effect) = effect {
        run(hwnd, effect);
    }
}

pub(super) unsafe extern "system" fn dialog_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // Checked without forming a reference: `paint` holds a `&Dialog` across `BeginPaint`,
    // which sends WM_ERASEBKGND back here, so neither the check nor that arm may borrow.
    if unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } == 0 {
        return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
    }
    match message {
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            if let Some(dialog) = state(hwnd) {
                paint(hwnd, dialog);
            }
            0
        }
        WM_CLOSE => {
            close(hwnd);
            0
        }
        WM_KEYDOWN => {
            key_down(hwnd, wparam as u16);
            0
        }
        // On the shortcuts page Alt+K toggles record-keys search, and Alt combinations are
        // strokes for the recording box and the table; the rest (Alt+F4) keep their defaults.
        WM_SYSKEYDOWN if state(hwnd).is_some_and(|dialog| dialog.model.page == Page::Shortcuts) => {
            if shortcuts_key(hwnd, wparam as u16) {
                0
            } else {
                unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
            }
        }
        // No menu to open and no beep for Alt+letters on the shortcuts page.
        windows_sys::Win32::UI::WindowsAndMessaging::WM_SYSCHAR
            if state(hwnd).is_some_and(|dialog| dialog.model.page == Page::Shortcuts) =>
        {
            0
        }
        // Alt+Down opens a dropdown, as in a combo box.
        WM_SYSKEYDOWN if wparam as u16 == VK_DOWN => {
            let effect = state(hwnd).and_then(|dialog| {
                dialog
                    .list
                    .is_none()
                    .then(|| dialog.model.key(Key::AltDown, &dialog.view))
            });
            if let Some(effect) = effect {
                run(hwnd, effect);
            }
            0
        }
        WM_CHAR => {
            if let Some(c) = char::from_u32(wparam as u32) {
                char_typed(hwnd, c);
            }
            0
        }
        WM_LIST_PICKED => {
            let (x, y) = lparam_point(lparam);
            let time = unsafe { GetMessageTime() } as u32;
            if let Some(dialog) = state(hwnd) {
                dialog.picked_at = Some((time, POINT { x, y }));
            }
            pick(hwnd, wparam);
            0
        }
        WM_SETTINGS_REFRESH => {
            refresh(hwnd);
            0
        }
        WM_ACTIVATE => {
            if (wparam & 0xffff) as u32 == WA_INACTIVE {
                close_list(hwnd);
                return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
            }
            // DefWindowProcW focuses the dialog itself; the search field takes it back.
            let result = unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
            sync_focus(hwnd);
            result
        }
        windows_sys::Win32::UI::WindowsAndMessaging::WM_CTLCOLOREDIT => {
            let Some((foreground, background, brush)) = state(hwnd).map(|dialog| {
                (
                    dialog.colors.editor_foreground,
                    Tones::new(&dialog.colors).control,
                    dialog.search_brush,
                )
            }) else {
                return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
            };
            let dc = wparam as HDC;
            unsafe {
                SetTextColor(dc, foreground);
                SetBkColor(dc, background);
            }
            brush as LRESULT
        }
        windows_sys::Win32::UI::WindowsAndMessaging::WM_COMMAND
            if (wparam & 0xffff) == crate::window::shortcuts_page::SEARCH_CONTROL_ID
                && ((wparam >> 16) & 0xffff) as u32
                    == windows_sys::Win32::UI::WindowsAndMessaging::EN_CHANGE =>
        {
            let search = state(hwnd).and_then(|dialog| dialog.search);
            let text = search.map(window_text).unwrap_or_default();
            // In record-keys mode the dialog writes the field itself.
            let effect = state(hwnd).and_then(|dialog| {
                (!dialog.shortcuts.record_keys).then(|| dialog.shortcuts.set_text(&text))
            });
            if let Some(effect) = effect {
                run_shortcuts(hwnd, effect);
            }
            0
        }
        #[cfg(test)]
        WM_TEST_ANSWER => {
            if let Some(answer) = IN_LOOP.with(|answers| answers.borrow_mut().pop_front()) {
                answer(hwnd);
            }
            0
        }
        WM_MOUSEWHEEL => {
            let delta = ((wparam >> 16) & 0xffff) as i16;
            if let Some(dialog) = state(hwnd) {
                if let Some((_, list)) = &dialog.list {
                    list.wheel(delta);
                } else if dialog.model.page == Page::Shortcuts {
                    // Three rows a notch, small touchpad deltas adding up; General's scroll
                    // stays where it was.
                    let delta = i32::from(delta);
                    if dialog.wheel_rest.signum() == -delta.signum() {
                        dialog.wheel_rest = 0;
                    }
                    dialog.wheel_rest += delta * 3;
                    let rows = dialog.wheel_rest / 120;
                    dialog.wheel_rest -= rows * 120;
                    let top = dialog.shortcuts.top;
                    dialog.shortcuts.scroll(-rows as isize);
                    if dialog.shortcuts.top != top {
                        invalidate(hwnd);
                    }
                } else {
                    let pitch = dialog.layout.row_pitch();
                    let scroll = (dialog.scroll - i32::from(delta) * pitch / 40)
                        .clamp(0, dialog.layout.max_scroll());
                    if scroll != dialog.scroll {
                        dialog.scroll = scroll;
                        invalidate(hwnd);
                    }
                }
            }
            0
        }
        // The native frame stays hidden: the client is the whole window. Both forms leave the
        // proposed rect as it is.
        WM_NCCALCSIZE => 0,
        // A press on the title row is a non-client click that starts the move loop without a
        // WM_LBUTTONDOWN or a deactivation: close the list here, as any click elsewhere does.
        WM_NCLBUTTONDOWN => {
            close_list(hwnd);
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
        // The edges size the dialog (the native frame that would is hidden), and only the title
        // row drags it.
        WM_NCHITTEST => {
            let (x, y) = lparam_point(lparam);
            let mut point = POINT { x, y };
            unsafe { ScreenToClient(hwnd, &mut point) };
            let Some(layout) = state(hwnd).map(|dialog| dialog.layout) else {
                return HTCLIENT as LRESULT;
            };
            if unsafe { IsZoomed(hwnd) } == 0 {
                let border = unsafe {
                    GetSystemMetricsForDpi(SM_CXSIZEFRAME, layout.dpi)
                        + GetSystemMetricsForDpi(SM_CXPADDEDBORDER, layout.dpi)
                };
                if let Some(edge) =
                    sizing_edge(point.x, point.y, layout.width, layout.height, border)
                {
                    return edge as LRESULT;
                }
            }
            if point.y < layout.title.bottom && point.x < layout.title_close.left {
                HTCAPTION as LRESULT
            } else {
                HTCLIENT as LRESULT
            }
        }
        windows_sys::Win32::UI::WindowsAndMessaging::WM_EXITSIZEMOVE => {
            save_size(hwnd);
            0
        }
        WM_SIZE => {
            let (width, height) = lparam_point(lparam);
            if width > 0 && height > 0 {
                resized(hwnd, width, height);
            }
            0
        }
        // No smaller than a usable minimum, unless the work area is (the dialog opens smaller
        // there); maximized, the work area, not the whole monitor (a popup would cover the
        // taskbar).
        WM_GETMINMAXINFO => {
            let dpi = state(hwnd).map_or(96, |dialog| dialog.layout.dpi);
            unsafe { crate::window::titlebar::constrain_maximized_window(hwnd, lparam) };
            let (width, height) = Layout::min_size(dpi);
            let minmax = unsafe { &mut *(lparam as *mut MINMAXINFO) };
            minmax.ptMinTrackSize = POINT {
                x: width.min(minmax.ptMaxSize.x),
                y: height.min(minmax.ptMaxSize.y),
            };
            0
        }
        // Over the sizing edges, the sizing cursors DefWindowProcW picks from the hit test.
        WM_SETCURSOR if (lparam & 0xffff) as u32 != HTCLIENT => unsafe {
            DefWindowProcW(hwnd, message, wparam, lparam)
        },
        WM_SETCURSOR => {
            let mut point = POINT::default();
            unsafe {
                GetCursorPos(&mut point);
                ScreenToClient(hwnd, &mut point);
            }
            let on_link = state(hwnd).is_some_and(|dialog| {
                matches!(
                    hit_at(dialog, point.x, point.y),
                    Some(
                        Hit::EditIni
                            | Hit::Page(crate::window::shortcuts_page::PageHit::ConflictLink)
                    )
                )
            });
            let cursor = if on_link { IDC_HAND } else { IDC_ARROW };
            unsafe { SetCursor(LoadCursorW(std::ptr::null_mut(), cursor) as HCURSOR) };
            1
        }
        WM_MOUSEMOVE => {
            let (x, y) = lparam_point(lparam);
            if let Some(dialog) = state(hwnd) {
                let hot = hit_at(dialog, x, y);
                if !dialog.tracking_leave {
                    let mut track = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    dialog.tracking_leave = unsafe { TrackMouseEvent(&mut track) } != 0;
                }
                if hot != dialog.hot {
                    dialog.hot = hot;
                    invalidate(hwnd);
                }
            }
            0
        }
        windows_sys::Win32::UI::Controls::WM_MOUSELEAVE => {
            if let Some(dialog) = state(hwnd) {
                dialog.tracking_leave = false;
                if dialog.hot.take().is_some() {
                    invalidate(hwnd);
                }
            }
            0
        }
        WM_LBUTTONDOWN => {
            let (x, y) = lparam_point(lparam);
            let mut at = POINT { x, y };
            unsafe { ClientToScreen(hwnd, &mut at) };
            let now = unsafe { GetMessageTime() } as u32;
            let limits = unsafe {
                (
                    GetDoubleClickTime(),
                    GetSystemMetrics(SM_CXDOUBLECLK),
                    GetSystemMetrics(SM_CYDOUBLECLK),
                )
            };
            let swallow = state(hwnd).is_some_and(|dialog| {
                let swallow = dialog
                    .picked_at
                    .take()
                    .is_some_and(|picked| completes_double_click(picked, now, at, limits));
                dialog.swallowing = swallow;
                swallow
            });
            if swallow {
                return 0;
            }
            let open_row = state(hwnd).and_then(|dialog| dialog.list.as_ref().map(|(row, _)| *row));
            close_list(hwnd);
            let effect = state(hwnd).and_then(|dialog| {
                dialog.pressed = None;
                let hit = hit_at(dialog, x, y)?;
                // A click on the dropdown whose list was open only closes that list.
                dialog.pressed =
                    (open_row.map(|row| Hit::Row(row, Part::Whole)) != Some(hit)).then_some(hit);
                // A greyed row takes no focus: it stays where it was; nor does the recording
                // box, which keeps the table's.
                Some(match hit {
                    Hit::Row(row, _) if !dialog.view.enabled(row) => Effect::None,
                    Hit::Page(
                        crate::window::shortcuts_page::PageHit::RecordBox
                        | crate::window::shortcuts_page::PageHit::OutsideRecordBox
                        | crate::window::shortcuts_page::PageHit::ConflictLink,
                    ) => Effect::None,
                    _ => dialog.model.set_focus(focus_of(hit), &dialog.view),
                })
            });
            if let Some(effect) = effect {
                unsafe { SetCapture(hwnd) };
                run(hwnd, effect);
            }
            0
        }
        WM_LBUTTONUP => {
            if state(hwnd).is_some_and(|dialog| std::mem::take(&mut dialog.swallowing)) {
                return 0;
            }
            let (x, y) = lparam_point(lparam);
            unsafe { ReleaseCapture() };
            let released = state(hwnd).and_then(|dialog| {
                let pressed = dialog.pressed.take()?;
                (hit_at(dialog, x, y) == Some(pressed)).then_some(pressed)
            });
            invalidate(hwnd);
            match released {
                Some(Hit::Page(hit)) => page_click(hwnd, hit),
                Some(hit) => {
                    let effect = state(hwnd).map(|dialog| click_effect(dialog, hit));
                    if let Some(effect) = effect {
                        run(hwnd, effect);
                    }
                }
                None => {}
            }
            0
        }
        // Shift+F10 and the context-menu key: a context menu with no point (-1), for the selected
        // row, as in the notebook tree. The recording box keeps both keys (Shift+F10 is refused
        // there), so none comes while it is open.
        windows_sys::Win32::UI::WindowsAndMessaging::WM_CONTEXTMENU
            if lparam as u32 == u32::MAX =>
        {
            keyboard_context_menu(hwnd);
            0
        }
        // A right-click on a row selects it and opens its menu; while the recording box is open
        // no row is hit.
        windows_sys::Win32::UI::WindowsAndMessaging::WM_RBUTTONUP => {
            use crate::window::shortcuts_page::PageHit;
            let (x, y) = lparam_point(lparam);
            let row = state(hwnd).and_then(|dialog| match hit_at(dialog, x, y) {
                Some(Hit::Page(PageHit::Row(index) | PageHit::Pencil(index))) => {
                    dialog.shortcuts.select(index);
                    dialog.model.focus = Focus::Table;
                    Some(index)
                }
                _ => None,
            });
            if row.is_some() {
                sync_focus(hwnd);
                invalidate(hwnd);
                context_menu(hwnd, POINT { x, y });
            }
            0
        }
        WM_NCDESTROY => {
            let pointer = unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) } as *mut Dialog;
            // SAFETY: from `Box::into_raw` in `create`, released only here.
            drop(unsafe { Box::from_raw(pointer) });
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}
