//! The `FastPadPreview` window procedure: routes paint, scroll, mouse, keyboard and focus
//! messages to the view's state.

use super::*;

pub(super) unsafe extern "system" fn preview_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_NCDESTROY => {
            let pointer = unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) } as *mut ViewState;
            if !pointer.is_null() {
                drop(unsafe { Box::from_raw(pointer) });
            }
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            let outcome = with_state(hwnd, |state| match paint(state) {
                Ok(outcome) if outcome.scroll.is_some() => {
                    state.paint_retried = false;
                    outcome
                }
                result => {
                    if result.is_err() {
                        drop_device_resources(state);
                    }
                    // Retry a failed frame once; a device that keeps failing must not spin.
                    PaintOutcome {
                        scroll: None,
                        repaint: !std::mem::replace(&mut state.paint_retried, true),
                        state_change: None,
                    }
                }
            });
            unsafe { ValidateRect(hwnd, std::ptr::null()) };
            // The state borrow has ended: these calls may send messages back to this window.
            if let Some(outcome) = outcome {
                if let Some(info) = outcome.scroll {
                    unsafe { SetScrollInfo(hwnd, SB_VERT, &info, 1) };
                }
                if outcome.repaint {
                    invalidate(hwnd);
                }
                if let Some(child) = outcome.state_change {
                    unsafe {
                        windows_sys::Win32::UI::Accessibility::NotifyWinEvent(
                            windows_sys::Win32::UI::WindowsAndMessaging::EVENT_OBJECT_STATECHANGE,
                            hwnd,
                            windows_sys::Win32::UI::WindowsAndMessaging::OBJID_CLIENT,
                            child,
                        )
                    };
                }
            }
            0
        }
        WM_SIZE => {
            let width = (lparam & 0xFFFF) as u32;
            let height = ((lparam >> 16) & 0xFFFF) as u32;
            with_state(hwnd, |state| {
                if let Some(target) = &state.target
                    && unsafe {
                        target.Resize(&D2D_SIZE_U {
                            width: width.max(1),
                            height: height.max(1),
                        })
                    }
                    .is_err()
                {
                    drop_device_resources(state);
                }
            });
            invalidate(hwnd);
            0
        }
        WM_GETDLGCODE => DLGC_WANTALLKEYS as LRESULT,
        WM_KILLFOCUS => {
            // The focus going to another window ends a drag, as losing the capture does.
            with_state(hwnd, |state| state.press = None);
            invalidate(hwnd);
            0
        }
        WM_SETFOCUS | WM_DPICHANGED_AFTERPARENT => {
            if message == WM_SETFOCUS {
                crate::window::post_content_focus(hwnd);
            }
            invalidate(hwnd);
            0
        }
        WM_MOUSEWHEEL | WM_MOUSEHWHEEL => {
            let delta = ((wparam >> 16) & 0xFFFF) as u16 as i16 as f32;
            let keys = (wparam & 0xFFFF) as u32;
            with_state(hwnd, |state| {
                let step = state.fonts.line_pitch() * 3.0 * delta / 120.0;
                let horizontal = message == WM_MOUSEHWHEEL || keys & MK_SHIFT != 0;
                if horizontal {
                    let mut point = windows_sys::Win32::Foundation::POINT {
                        x: (lparam & 0xFFFF) as u16 as i16 as i32,
                        y: ((lparam >> 16) & 0xFFFF) as u16 as i16 as i32,
                    };
                    unsafe { windows_sys::Win32::Graphics::Gdi::ScreenToClient(hwnd, &mut point) };
                    let document_y =
                        point.y as f32 / dpi_scale(hwnd) + state.scroll_y - bar_height(state);
                    let index = state.heights.index_at(document_y);
                    let (view_width, _) = view_size(hwnd);
                    let (_, content_width) = content_frame(view_width, state.centered);
                    if let Some(Some(laid)) = state.layouts.get(index)
                        && laid.scroll_width > content_width
                    {
                        let limit = laid.scroll_width - content_width;
                        let sign = if message == WM_MOUSEHWHEEL { 1.0 } else { -1.0 };
                        let offset = state.h_scroll.entry(index).or_insert(0.0);
                        *offset = (*offset + sign * step).clamp(0.0, limit);
                        invalidate(hwnd);
                    }
                } else {
                    let target = state.scroll_y - step;
                    set_scroll(state, target, true);
                }
            });
            0
        }
        WM_VSCROLL => {
            with_state(hwnd, |state| {
                let (_, view_height) = view_size(hwnd);
                let line = state.fonts.line_pitch();
                let page = (view_height - bar_height(state) - line).max(line);
                let target = match (wparam & 0xFFFF) as i32 {
                    SB_LINEUP => state.scroll_y - line,
                    SB_LINEDOWN => state.scroll_y + line,
                    SB_PAGEUP => state.scroll_y - page,
                    SB_PAGEDOWN => state.scroll_y + page,
                    SB_TOP => 0.0,
                    SB_BOTTOM => f32::MAX,
                    SB_THUMBTRACK => {
                        let mut info = SCROLLINFO {
                            cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
                            fMask: SIF_TRACKPOS,
                            ..Default::default()
                        };
                        unsafe { GetScrollInfo(hwnd, SB_VERT, &mut info) };
                        info.nTrackPos as f32 / dpi_scale(hwnd)
                    }
                    _ => return,
                };
                set_scroll(state, target, true);
            });
            0
        }
        WM_KEYDOWN => {
            let key = wparam as u16;
            let ctrl = unsafe { GetKeyState(VK_CONTROL as i32) } < 0;
            match key {
                // Ctrl+C is the main window's accelerator for Copy, which copies from here.
                VK_INSERT | VK_C if ctrl => {
                    PreviewView { hwnd }.copy();
                    return 0;
                }
                VK_A if ctrl => {
                    with_state(hwnd, select_all);
                    return 0;
                }
                VK_APPS => {
                    show_menu(hwnd, None);
                    return 0;
                }
                // Escape clears a selection before it leaves Preview.
                VK_ESCAPE if with_state(hwnd, clear).unwrap_or(false) => return 0,
                _ => {}
            }
            if key == VK_ESCAPE {
                unsafe {
                    PostMessageW(
                        crate::platform::win32::root_window(hwnd),
                        WM_FASTPAD_PREVIEW_ESCAPE,
                        hwnd as usize,
                        0,
                    )
                };
                return 0;
            }
            with_state(hwnd, |state| {
                let (_, view_height) = view_size(hwnd);
                let line = state.fonts.line_pitch();
                let page = (view_height - bar_height(state) - line).max(line);
                match key {
                    VK_UP => set_scroll(state, state.scroll_y - 2.0 * line, true),
                    VK_DOWN => set_scroll(state, state.scroll_y + 2.0 * line, true),
                    VK_PRIOR => set_scroll(state, state.scroll_y - page, true),
                    VK_NEXT => set_scroll(state, state.scroll_y + page, true),
                    VK_HOME => set_scroll(state, 0.0, true),
                    VK_END => set_scroll(state, f32::MAX, true),
                    VK_TAB => {
                        let backward = unsafe { GetKeyState(VK_SHIFT as i32) } < 0;
                        move_focus(state, !backward);
                    }
                    VK_RETURN => {
                        if let Some(focus) = state.focus {
                            activate(state, focus);
                        }
                    }
                    VK_SPACE => {
                        if let Some((block, index)) = state.focus
                            && state
                                .layouts
                                .get(block)
                                .and_then(Option::as_ref)
                                .and_then(|laid| laid.targets.get(index))
                                .is_some_and(|target| {
                                    matches!(target.kind, TargetKind::Disclosure { .. })
                                })
                        {
                            activate(state, (block, index));
                        }
                    }
                    _ => {}
                }
            });
            0
        }
        WM_SYSKEYDOWN if wparam as u16 == VK_F10 && unsafe { GetKeyState(VK_SHIFT as i32) } < 0 => {
            show_menu(hwnd, None);
            0
        }
        WM_CONTEXTMENU => {
            let keyboard = lparam == -1;
            let at = (!keyboard).then(|| {
                let mut point = windows_sys::Win32::Foundation::POINT {
                    x: (lparam & 0xFFFF) as u16 as i16 as i32,
                    y: ((lparam >> 16) & 0xFFFF) as u16 as i16 as i32,
                };
                unsafe { windows_sys::Win32::Graphics::Gdi::ScreenToClient(hwnd, &mut point) };
                (point.x, point.y)
            });
            show_menu(hwnd, at);
            0
        }
        WM_RBUTTONDOWN => {
            unsafe { SetFocus(hwnd) };
            0
        }
        WM_MOUSEMOVE => {
            let (x, y) = client_point(lparam);
            let button = wparam & MK_LBUTTON as usize != 0;
            let autoscroll = with_state(hwnd, |state| {
                state.over_text = over_text(state, x, y);
                if button { drag(state, x, y) } else { false }
            })
            .unwrap_or(false);
            if autoscroll {
                unsafe { SetTimer(hwnd, AUTOSCROLL_TIMER, AUTOSCROLL_MS, None) };
            } else {
                unsafe { KillTimer(hwnd, AUTOSCROLL_TIMER) };
            }
            with_state(hwnd, |state| {
                if !state.tracking_mouse {
                    let mut track = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    state.tracking_mouse = unsafe { TrackMouseEvent(&mut track) } != 0;
                }
                let hovered = target_at(state, x, y);
                if hovered != state.hover {
                    set_underline(state, state.hover, false);
                    set_underline(state, hovered, true);
                    state.hover = hovered;
                    post_hover(hwnd, hovered.and_then(|target| target_dest(state, target)));
                    invalidate(hwnd);
                }
            });
            0
        }
        WM_MOUSELEAVE => {
            with_state(hwnd, |state| {
                state.tracking_mouse = false;
                if state.hover.is_some() {
                    set_underline(state, state.hover, false);
                    state.hover = None;
                    post_hover(hwnd, None);
                    invalidate(hwnd);
                }
            });
            0
        }
        WM_SETCURSOR if (lparam & 0xFFFF) as u32 == HTCLIENT => {
            let cursor = with_state(hwnd, |state| {
                if state.hover.is_some() {
                    IDC_HAND
                } else if state.over_text || state.press.is_some_and(|press| press.dragging) {
                    IDC_IBEAM
                } else {
                    IDC_ARROW
                }
            })
            .unwrap_or(IDC_ARROW);
            unsafe { SetCursor(LoadCursorW(std::ptr::null_mut(), cursor)) };
            1
        }
        WM_LBUTTONDOWN => {
            unsafe { SetFocus(hwnd) };
            let (x, y) = client_point(lparam);
            let shift = wparam & MK_SHIFT as usize != 0;
            let time = unsafe { GetMessageTime() } as u32;
            let paused_bar = with_state(hwnd, |state| state.paused).unwrap_or(false)
                && (y as f32) < PAUSED_BAR_HEIGHT * dpi_scale(hwnd);
            let capture = !paused_bar
                && with_state(hwnd, |state| {
                    state.pressed = if shift { None } else { target_at(state, x, y) };
                    press(state, x, y, shift, time)
                })
                .unwrap_or(false);
            if capture {
                unsafe { SetCapture(hwnd) };
            }
            0
        }
        WM_TIMER if wparam == AUTOSCROLL_TIMER => {
            let mut point = windows_sys::Win32::Foundation::POINT::default();
            unsafe {
                GetCursorPos(&mut point);
                windows_sys::Win32::Graphics::Gdi::ScreenToClient(hwnd, &mut point);
            }
            let keep =
                with_state(hwnd, |state| autoscroll(state, point.x, point.y)).unwrap_or(false);
            if !keep {
                unsafe { KillTimer(hwnd, AUTOSCROLL_TIMER) };
            }
            0
        }
        WM_CAPTURECHANGED => {
            unsafe { KillTimer(hwnd, AUTOSCROLL_TIMER) };
            with_state(hwnd, release_press);
            invalidate(hwnd);
            0
        }
        WM_LBUTTONUP => {
            let (x, y) = client_point(lparam);
            unsafe { KillTimer(hwnd, AUTOSCROLL_TIMER) };
            let dragged = with_state(hwnd, |state| {
                release_press(state).is_some_and(|press| press.dragging)
            })
            .unwrap_or(false);
            if unsafe { GetCapture() } == hwnd {
                unsafe { ReleaseCapture() };
            }
            invalidate(hwnd);
            with_state(hwnd, |state| {
                if dragged {
                    state.pressed = None;
                    return;
                }
                if state.paused && (y as f32) < PAUSED_BAR_HEIGHT * dpi_scale(hwnd) {
                    unsafe {
                        PostMessageW(
                            crate::platform::win32::root_window(hwnd),
                            WM_FASTPAD_PREVIEW_REFRESH,
                            hwnd as usize,
                            0,
                        )
                    };
                    return;
                }
                let released = target_at(state, x, y);
                if released.is_some()
                    && released == state.pressed.take()
                    && let Some(released) = released
                {
                    activate(state, released);
                }
            });
            0
        }
        windows_sys::Win32::UI::WindowsAndMessaging::WM_GETOBJECT
            if lparam as i32 == windows_sys::Win32::UI::WindowsAndMessaging::OBJID_CLIENT =>
        {
            // Clone the snapshot inside the borrow; `LresultFromObject` runs after it ends.
            match with_state(hwnd, |state| Arc::clone(&state.accessible)) {
                Some(links) => crate::preview::accessible::object_result(hwnd, links, wparam),
                None => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
            }
        }
        WM_FASTPAD_PREVIEW_ACTIVATE => {
            // The accessibility provider resolved the target against the snapshot the client saw;
            // resolving an index here could land on another target after a repaint.
            if lparam != 0 {
                if wparam == ACTIVATE_DISCLOSURE {
                    let key = *unsafe { Box::from_raw(lparam as *mut DetailsKey) };
                    with_state(hwnd, |state| toggle_details(state, &key));
                } else {
                    let dest = *unsafe { Box::from_raw(lparam as *mut String) };
                    post_link(hwnd, dest);
                }
            }
            0
        }
        WM_FASTPAD_PREVIEW_IMAGE => {
            with_state(hwnd, |state| {
                if state.images.drain() {
                    for layout in &mut state.layouts {
                        if layout.as_ref().is_some_and(|laid| !laid.images.is_empty()) {
                            *layout = None;
                        }
                    }
                    invalidate(hwnd);
                }
            });
            0
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

/// Asks the main window for the context menu at client point `at`, or for the keyboard. Sent
/// after the state borrow ends: the menu calls back into the view.
fn show_menu(hwnd: HWND, at: Option<(i32, i32)>) {
    let Some(request) = with_state(hwnd, |state| menu_request(state, at)) else {
        return;
    };
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::SendMessageW(
            crate::platform::win32::root_window(hwnd),
            crate::window::WM_FASTPAD_PREVIEW_MENU,
            hwnd as usize,
            &request as *const MenuRequest as isize,
        )
    };
}
