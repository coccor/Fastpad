use crate::Result;
use crate::app::{App, WindowIdentity};
use crate::document::{CloseDecision, Document, DocumentId, RecoveryId};
use crate::editor::{Editor, TextDirection};
use crate::perf::Milestone;
use crate::platform::{last_error, wide_null};
use crate::window::accessibility::{self, AccessibleSelectRequest, WM_FASTPAD_ACCESSIBLE_SELECT};
use crate::window::command_palette::{self, CommandPalette};
use crate::window::commands::CommandId;
use crate::window::find_bar;
use crate::window::menu_band::{self, MenuMode};
use crate::window::menus::{self, DropdownExit, MenuBar};
use crate::window::messages::{
    DeferredAction, classify_deferred_message, completed_milestone, deferred_start_message,
};
use crate::window::modal::prompt_close_decision;
use crate::window::palette::Palette;
use crate::window::tabs::CloseReviewKey;
use crate::window::titlebar::{HitTarget, PointerState, TitleBarLayout, TitleFontHandles};
#[cfg(test)]
use std::cell::Cell;
use std::ffi::c_void;
use std::ptr::NonNull;
use windows_sys::Win32::Foundation::{HMODULE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{HDC, InvalidateRect};
use windows_sys::Win32::System::SystemInformation::GetTickCount;
use windows_sys::Win32::UI::Controls::{DRAWITEMSTRUCT, NMHDR, WM_MOUSELEAVE};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetFocus, GetKeyState, GetLastInputInfo, LASTINPUTINFO, ReleaseCapture, SetCapture, SetFocus,
    VK_CONTROL, VK_DOWN, VK_ESCAPE, VK_F10, VK_LEFT, VK_MENU, VK_RETURN, VK_RIGHT, VK_SHIFT, VK_UP,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    BN_CLICKED, CREATESTRUCTW, CreateWindowExW, DefWindowProcW, DestroyWindow, EN_CHANGE,
    GWL_STYLE, GWLP_USERDATA, GetClientRect, GetWindowLongPtrW, HTCAPTION, IsWindow,
    IsWindowVisible, IsZoomed, KillTimer, LoadIconW, MoveWindow, OBJID_CLIENT, PostMessageW,
    PostQuitMessage, QS_INPUT, RegisterClassW, SC_CLOSE, SC_KEYMENU, SC_MAXIMIZE, SC_MINIMIZE,
    SC_RESTORE, SW_HIDE, SW_SHOWNA, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    SWP_NOZORDER, SendMessageW, SetTimer, SetWindowLongPtrW, SetWindowPos, ShowWindow,
    UnregisterClassW, WHEEL_DELTA, WM_ACTIVATEAPP, WM_CAPTURECHANGED, WM_CLOSE, WM_COMMAND,
    WM_CTLCOLORBTN, WM_CTLCOLOREDIT, WM_CTLCOLORLISTBOX, WM_DESTROY, WM_DPICHANGED, WM_DRAWITEM,
    WM_DROPFILES, WM_DWMCOLORIZATIONCOLORCHANGED, WM_GETMINMAXINFO, WM_GETOBJECT, WM_KEYDOWN,
    WM_KILLFOCUS, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEHWHEEL, WM_MOUSEMOVE, WM_MOUSEWHEEL,
    WM_NCCALCSIZE, WM_NCCREATE, WM_NCDESTROY, WM_NCHITTEST, WM_NCLBUTTONDBLCLK, WM_NCLBUTTONDOWN,
    WM_NCLBUTTONUP, WM_NCMOUSELEAVE, WM_NCMOUSEMOVE, WM_NCRBUTTONDOWN, WM_NCRBUTTONUP, WM_NOTIFY,
    WM_PAINT, WM_SETFOCUS, WM_SETTINGCHANGE, WM_SIZE, WM_SYSCOMMAND, WM_SYSKEYDOWN, WM_SYSKEYUP,
    WM_THEMECHANGED, WM_TIMER, WNDCLASSW, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
};
#[cfg(test)]
use windows_sys::Win32::UI::WindowsAndMessaging::{MSG, PM_NOREMOVE, PeekMessageW, WM_QUIT};

pub(crate) const INPUT_MESSAGE_FIRST: u32 =
    windows_sys::Win32::UI::WindowsAndMessaging::WM_INPUT_DEVICE_CHANGE;
pub(crate) const INPUT_MESSAGE_LAST: u32 =
    windows_sys::Win32::UI::WindowsAndMessaging::WM_POINTERROUTEDRELEASED;

/// Icon resource id embedded by `build.rs` from `assets/fastpad.ico`.
const APP_ICON_RESOURCE_ID: usize = 1;

pub struct MainWindowClass {
    class_name: Vec<u16>,
    instance: HMODULE,
}

pub struct WindowCreateContext<T> {
    value: Option<Box<T>>,
}

impl<T> WindowCreateContext<T> {
    pub fn new(value: Box<T>) -> Self {
        Self { value: Some(value) }
    }

    fn lp_param(&mut self) -> *mut c_void {
        self as *mut Self as *mut c_void
    }
}

impl MainWindowClass {
    pub fn register(instance: HMODULE) -> Result<Self> {
        let class_name = wide_null("FastPadMainWindow");
        let window_class = WNDCLASSW {
            lpfnWndProc: Some(main_window_proc),
            hInstance: instance,
            // MAKEINTRESOURCEW; a module without the resource (e.g. a test binary) gets null, which
            // falls back to the default window icon.
            hIcon: unsafe { LoadIconW(instance, APP_ICON_RESOURCE_ID as *const u16) },
            // The tab strip is client area; without a class cursor, hovering it keeps whatever
            // cursor was last shown (the editor's I-beam, a resize arrow).
            hCursor: unsafe {
                windows_sys::Win32::UI::WindowsAndMessaging::LoadCursorW(
                    std::ptr::null_mut(),
                    windows_sys::Win32::UI::WindowsAndMessaging::IDC_ARROW,
                )
            },
            lpszClassName: class_name.as_ptr(),
            ..Default::default()
        };
        let atom = unsafe { RegisterClassW(&window_class) };
        if atom == 0 {
            return Err(last_error());
        }
        Ok(Self {
            class_name,
            instance,
        })
    }

    pub fn create(&self, context: &mut WindowCreateContext<App>) -> Result<HWND> {
        let hwnd = unsafe {
            CreateWindowExW(
                0,
                self.class_name.as_ptr(),
                wide_null("FastPad").as_ptr(),
                WS_OVERLAPPEDWINDOW,
                100,
                100,
                1280,
                720,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                self.instance,
                context.lp_param().cast(),
            )
        };
        if hwnd.is_null() {
            return Err(last_error());
        }
        // Re-runs WM_NCCALCSIZE so the initial frame drops the native caption band.
        unsafe {
            SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                0,
                0,
                0,
                0,
                SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
        Ok(hwnd)
    }
}

impl Drop for MainWindowClass {
    fn drop(&mut self) {
        unsafe {
            UnregisterClassW(self.class_name.as_ptr(), self.instance);
        }
    }
}

pub(crate) unsafe fn maybe_post_deferred_start(hwnd: HWND, identity: &WindowIdentity) {
    // SAFETY: The caller guarantees `hwnd` is the live FastPad main window. The raw App pointer is
    // used only for the immediate pending-flag transition before posting the deferred message.
    if !identity.is_live_for(hwnd) {
        return;
    }
    let should_post = unsafe { take_deferred_start_pending(hwnd) };
    if should_post {
        unsafe {
            PostMessageW(hwnd, deferred_start_message(), 0, 0);
        }
    }
}

unsafe extern "system" fn main_window_proc(
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
            shutdown_ipc(hwnd);
            clear_documents_for_shutdown(hwnd);
            unsafe {
                DestroyWindow(hwnd);
            }
            0
        }
        WM_DESTROY => {
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
        WM_DROPFILES => {
            let drop = wparam as windows_sys::Win32::UI::Shell::HDROP;
            let paths = crate::platform::win32::dropped_paths(drop);
            unsafe { windows_sys::Win32::UI::Shell::DragFinish(drop) };
            crate::window::library_host::files_dropped(hwnd, paths);
            0
        }
        WM_ACTIVATEAPP => {
            crate::window::library_host::activation_changed(hwnd, wparam != 0);
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
        WM_PAINT => {
            sync_window_title(hwnd);
            let paint_title_strip = |hwnd, _, _, _| {
                let (titles, active, scroll, empty, preview_tab) = tab_snapshot(hwnd);
                let title_refs = titles.iter().map(String::as_str).collect::<Vec<_>>();
                let status = current_status_bar(hwnd);
                let (palette, fonts, pointer) = title_chrome(hwnd);
                let headings = menu_headings(hwnd);
                unsafe {
                    crate::window::titlebar::paint(
                        hwnd,
                        &crate::window::titlebar::TitlePaint {
                            titles: &title_refs,
                            active,
                            preview_tab,
                            scroll,
                            empty_hint: empty.then_some(EMPTY_TABS_HINT),
                            status: status.as_ref(),
                            palette,
                            fonts,
                            pointer,
                            menu: menu_mode(hwnd).map(|mode| (mode, headings.as_slice())),
                            preview: preview_buttons_visible(hwnd)
                                .then(|| crate::window::preview_host::mode(hwnd)),
                            divider: crate::window::preview_host::divider_rect(hwnd),
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
            crate::window::titlebar::nonclient_hit_test(
                hwnd,
                wparam,
                lparam,
                tab_count(hwnd),
                tab_scroll(hwnd),
                preview_buttons_visible(hwnd),
            )
        },
        WM_NCCALCSIZE => unsafe { crate::window::titlebar::reclaim_caption(hwnd, wparam, lparam) },
        WM_GETMINMAXINFO => unsafe {
            crate::window::titlebar::constrain_maximized_window(hwnd, lparam)
        },
        WM_MOUSEMOVE => {
            if crate::window::preview_host::drag_divider(
                hwnd,
                (lparam as u32 & 0xffff) as u16 as i16 as i32,
            ) {
                return 0;
            }
            hover_menu_heading(hwnd, lparam);
            drag_tab_thumb(hwnd, lparam);
            crate::window::titlebar::track_pointer_leave(hwnd, false);
            let target = client_title_target(hwnd, lparam);
            update_title_pointer(hwnd, |pointer| pointer.hover(target));
            crate::window::preview_host::button_hover(hwnd, target);
            0
        }
        WM_MOUSELEAVE => {
            update_title_pointer(hwnd, |pointer| pointer.leave(false));
            crate::window::preview_host::button_hover(hwnd, None);
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
            crate::window::preview_host::button_hover(hwnd, None);
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
        WM_LBUTTONDOWN => {
            let (x, y) = (
                (lparam as u32 & 0xffff) as u16 as i16 as i32,
                ((lparam as u32 >> 16) & 0xffff) as u16 as i16 as i32,
            );
            if crate::window::preview_host::begin_divider_drag(hwnd, x, y) {
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
            if target == Some(HitTarget::ScrollBar) {
                begin_tab_thumb_drag(hwnd, (lparam as u32 & 0xffff) as u16 as i16 as i32);
            }
            0
        }
        WM_CAPTURECHANGED => {
            if let Some(mut app) = unsafe { app_ptr(hwnd) } {
                unsafe { app.as_mut() }.tab_thumb_grab = None;
            }
            crate::window::preview_host::cancel_divider_drag(hwnd);
            0
        }
        // The empty tab-strip space is the only caption: double-clicking it opens a tab, VSCode
        // style, instead of maximizing. Over the sidebar's top strip it maximizes as usual.
        WM_NCLBUTTONDBLCLK if wparam == HTCAPTION as usize && !over_sidebar(hwnd, lparam) => {
            execute_command(hwnd, CommandId::New);
            0
        }
        // Its context menu replaces the system menu; Alt+Space still opens that.
        WM_NCRBUTTONDOWN if wparam == HTCAPTION as usize && !over_sidebar(hwnd, lparam) => 0,
        WM_NCRBUTTONUP if wparam == HTCAPTION as usize && !over_sidebar(hwnd, lparam) => {
            let mut point = windows_sys::Win32::Foundation::POINT {
                x: (lparam as u32 & 0xffff) as u16 as i16 as i32,
                y: ((lparam as u32 >> 16) & 0xffff) as u16 as i16 as i32,
            };
            unsafe {
                windows_sys::Win32::Graphics::Gdi::ScreenToClient(hwnd, &mut point);
            }
            let has_tabs = tab_count(hwnd) > 0;
            if let Some(command) = menus::show_tab_strip_menu(hwnd, point.x, point.y, has_tabs) {
                execute_command(hwnd, command);
            }
            0
        }
        WM_MOUSEWHEEL | WM_MOUSEHWHEEL => {
            let delta = i32::from((wparam >> 16) as u16 as i16);
            // Wheel up scrolls toward the first tab; a tilt to the right toward the last.
            let delta = if message == WM_MOUSEWHEEL {
                -delta
            } else {
                delta
            };
            if scroll_tabs(hwnd, lparam, delta) {
                0
            } else {
                unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
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
        WM_LBUTTONUP => {
            if crate::window::preview_host::end_divider_drag(hwnd) {
                return 0;
            }
            update_title_pointer(hwnd, |pointer| pointer.release(None).0);
            if end_tab_thumb_drag(hwnd) {
                return 0;
            }
            let point = crate::window::titlebar::Point::new(
                (lparam as u32 & 0xffff) as u16 as i16 as i32,
                ((lparam as u32 >> 16) & 0xffff) as u16 as i16 as i32,
            );
            if notice_contains(hwnd, point.y) {
                dismiss_notifications(hwnd);
                return 0;
            }
            let layout = title_layout(hwnd);
            match layout.hit_test(point) {
                crate::window::titlebar::HitTarget::Overflow => {
                    if let Some(command) = menus::show_overflow(hwnd, point.x, layout.height) {
                        execute_command(hwnd, command);
                    }
                }
                crate::window::titlebar::HitTarget::CloseTab(index) => {
                    activate_tab(hwnd, index);
                    execute_command(hwnd, CommandId::CloseTab);
                }
                crate::window::titlebar::HitTarget::Tab(index) => {
                    activate_tab(hwnd, index);
                    if tab_double_click(hwnd, index)
                        && let Some(id) = unsafe { app_ptr(hwnd) }
                            .and_then(|app| Some(unsafe { app.as_ref() }.tabs.active()?.id))
                    {
                        promote_tab(hwnd, id);
                    }
                }
                target @ (crate::window::titlebar::HitTarget::PreviewSide
                | crate::window::titlebar::HitTarget::PreviewFull) => {
                    crate::window::preview_host::click_button(hwnd, target)
                }
                _ => {}
            }
            0
        }
        WM_NOTIFY => {
            handle_editor_notification(hwnd, lparam);
            0
        }
        WM_FASTPAD_ACCESSIBLE_SELECT => handle_accessible_select(hwnd, lparam),
        WM_COMMAND
            if lparam != 0
                && ((wparam >> 16) & 0xffff) as u32 == EN_CHANGE
                && command_palette_owns(hwnd, lparam as HWND) =>
        {
            refilter_command_palette(hwnd);
            0
        }
        WM_CTLCOLOREDIT if find_bar_owns(hwnd, lparam as HWND) => unsafe { app_ptr(hwnd) }
            .and_then(|app| {
                unsafe { app.as_ref() }
                    .find_bar
                    .as_ref()
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
            if let Some(mut app) = unsafe { app_ptr(hwnd) } {
                unsafe { app.as_mut() }.title_fonts = None;
            }
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
                InvalidateRect(hwnd, std::ptr::null(), 1);
            }
            let dpi = (wparam & 0xffff) as u32;
            if let Some(editor) =
                unsafe { app_ptr(hwnd) }.and_then(|app| unsafe { app.as_ref() }.editor.clone())
            {
                let _ = editor.set_text_padding(dpi);
            }
            // The suggested rectangle may keep the size, and then no WM_SIZE re-lays out the
            // sidebar and bands for the new DPI.
            layout_editor_and_find_bar(hwnd);
            0
        }
        WM_SETTINGCHANGE | WM_THEMECHANGED | WM_DWMCOLORIZATIONCOLORCHANGED => {
            refresh_theme(hwnd);
            unsafe {
                InvalidateRect(hwnd, std::ptr::null(), 1);
            }
            0
        }
        windows_sys::Win32::UI::WindowsAndMessaging::WM_SETCURSOR
            if crate::window::preview_host::cursor_over_divider(hwnd) =>
        {
            unsafe {
                windows_sys::Win32::UI::WindowsAndMessaging::SetCursor(
                    windows_sys::Win32::UI::WindowsAndMessaging::LoadCursorW(
                        std::ptr::null_mut(),
                        windows_sys::Win32::UI::WindowsAndMessaging::IDC_SIZEWE,
                    ),
                )
            };
            1
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
                return 0;
            }
            if message == crate::window::WM_FASTPAD_FILES_DROPPED {
                crate::window::library_host::editor_files_dropped(hwnd, lparam);
                return 0;
            }
            if message == crate::window::WM_FASTPAD_NOTEBOOK_CHECKED {
                crate::window::library_host::notebook_checked(hwnd, lparam);
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
                crate::window::WM_FASTPAD_PREVIEW_ESCAPE => {
                    crate::window::preview_host::escape(hwnd);
                    return 0;
                }
                crate::window::WM_FASTPAD_PREVIEW_PARSED => {
                    crate::window::preview_host::parsed(hwnd, lparam);
                    return 0;
                }
                crate::window::WM_FASTPAD_PREVIEW_LINK => {
                    crate::window::preview_host::follow_link(hwnd, lparam);
                    return 0;
                }
                crate::window::WM_FASTPAD_PREVIEW_HOVER => {
                    crate::window::preview_host::hover_link(hwnd, lparam);
                    return 0;
                }
                crate::window::WM_FASTPAD_PREVIEW_REFRESH => {
                    crate::window::preview_host::refresh(hwnd);
                    return 0;
                }
                crate::window::WM_FASTPAD_PREVIEW_SCROLLED => {
                    crate::window::preview_host::preview_scrolled(hwnd, wparam);
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

unsafe fn handle_paint_with<D, C>(
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

fn handle_deferred(hwnd: HWND, action: DeferredAction) -> LRESULT {
    // Only `WM_FASTPAD_OPEN_REQUEST` processed with no input pending produces this action, so the
    // launch file opens on exactly the same input-readiness gate as the rest of the chain. The
    // open posts the language continuation (and records FileLoaded) itself.
    if action == DeferredAction::PostNext(crate::window::WM_FASTPAD_APPLY_LANGUAGE) {
        return handle_open_request(hwnd);
    }
    // Each of these actions is produced only by its own deferred message with no input pending:
    // `PostNext(WM_FASTPAD_RESTORE_SESSION)` by `WM_FASTPAD_LOAD_SETTINGS`, `RecordFullyReady` by
    // `WM_FASTPAD_BUILD_CHROME`. Running them before the milestone keeps the milestone honest.
    if action == DeferredAction::PostNext(crate::window::WM_FASTPAD_RESTORE_SESSION) {
        load_settings(hwnd);
    }
    // Only `WM_FASTPAD_RESTORE_SESSION` processed with no input pending produces this action.
    // Each pass reopens at most one session entry and reposts the unit until none remain.
    if action == DeferredAction::PostNext(crate::window::WM_FASTPAD_OPEN_LIBRARY)
        && restore_session_step(hwnd) == RestoreStep::Continue
    {
        unsafe {
            PostMessageW(hwnd, crate::window::WM_FASTPAD_RESTORE_SESSION, 0, 0);
        }
        return 0;
    }
    // Only `WM_FASTPAD_OPEN_LIBRARY` processed with no input pending produces this action.
    if action == DeferredAction::PostNext(crate::window::WM_FASTPAD_OPEN_REQUEST) {
        crate::window::library_host::open_library_step(hwnd);
    }
    if action == DeferredAction::RecordFullyReady {
        build_chrome(hwnd);
    }
    if let Some(milestone) = completed_milestone(action) {
        unsafe {
            let _ = record_milestone(hwnd, milestone);
        }
    }
    // `WM_FASTPAD_APPLY_LANGUAGE` is the only message that classifies into
    // `PostNext(WM_FASTPAD_RECOVERY)`, so this is exactly the point where that message has just
    // been processed (input was not pending). It serves double duty: advancing the deferred
    // startup chain (above/below) and, here, detecting and applying the active document's
    // language after every successful Open/Save As/launch load that posts it.
    if action == DeferredAction::PostNext(crate::window::WM_FASTPAD_RECOVERY) {
        apply_detected_language(hwnd);
        // A tab reopened mid-restore must not start recovery, IPC and chrome ahead of the rest of
        // the session. After the restore, `WM_FASTPAD_OPEN_REQUEST` always posts
        // `WM_FASTPAD_APPLY_LANGUAGE` again, so the chain resumes in order from there.
        if unsafe { app_ptr(hwnd) }
            .is_some_and(|app| unsafe { app.as_ref() }.session_restore.is_some())
        {
            return 0;
        }
    }
    // Only `WM_FASTPAD_RECOVERY` processed with no input pending produces this action.
    if action == DeferredAction::PostNext(crate::window::WM_FASTPAD_START_IPC) {
        recover_snapshots(hwnd);
    }
    // Only `WM_FASTPAD_START_IPC` processed with no input pending produces this action. A session
    // restore has already bound the pipe, and binding again is a no-op.
    if action == DeferredAction::PostNext(crate::window::WM_FASTPAD_BUILD_CHROME) {
        start_ipc_server_with(hwnd, crate::ipc::bind_session_server);
    }
    match action {
        DeferredAction::RepostSelf(message) => {
            unsafe {
                request_input_priority(hwnd);
            }
            unsafe {
                PostMessageW(hwnd, message, 0, 0);
            }
            0
        }
        DeferredAction::PostNext(message) => unsafe {
            PostMessageW(hwnd, message, 0, 0);
            0
        },
        DeferredAction::RecordFullyReady => 0,
    }
}

pub(crate) fn input_pending() -> bool {
    const STATUS_SHIFT: u32 = 16;
    let queue_status = input_queue_status_mask();
    let pending = unsafe {
        ((windows_sys::Win32::UI::WindowsAndMessaging::GetQueueStatus(queue_status)
            >> STATUS_SHIFT)
            & queue_status)
            != 0
    };
    #[cfg(test)]
    if pending && queue_status != QS_INPUT {
        let mut message = MSG::default();
        let found = unsafe {
            PeekMessageW(
                &mut message,
                std::ptr::null_mut(),
                INPUT_MESSAGE_FIRST,
                INPUT_MESSAGE_LAST,
                PM_NOREMOVE | (queue_status << STATUS_SHIFT),
            )
        } != 0;
        return found && message.message != WM_QUIT;
    }
    pending
}

pub(crate) fn input_queue_status_mask() -> u32 {
    #[cfg(test)]
    {
        TEST_INPUT_QUEUE_STATUS.with(Cell::get)
    }
    #[cfg(not(test))]
    {
        QS_INPUT
    }
}

#[cfg(test)]
thread_local! {
    static TEST_INPUT_QUEUE_STATUS: Cell<u32> = const { Cell::new(QS_INPUT) };
}

#[cfg(test)]
pub(crate) fn with_test_input_queue_status<R>(queue_status: u32, run: impl FnOnce() -> R) -> R {
    struct ResetQueueStatus(u32);

    impl Drop for ResetQueueStatus {
        fn drop(&mut self) {
            TEST_INPUT_QUEUE_STATUS.with(|status| status.set(self.0));
        }
    }

    TEST_INPUT_QUEUE_STATUS.with(|status| {
        let reset = ResetQueueStatus(status.replace(queue_status));
        let result = run();
        drop(reset);
        result
    })
}

unsafe fn take_app(hwnd: HWND) -> Option<Box<App>> {
    let raw = unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) as *mut App };
    (!raw.is_null()).then(|| unsafe { Box::from_raw(raw) })
}

pub(crate) unsafe fn initialize_editor_with<F>(
    hwnd: HWND,
    identity: &WindowIdentity,
    create_editor: F,
) -> Result<HWND>
where
    F: FnOnce(HWND) -> Result<Editor>,
{
    // SAFETY: The caller guarantees `hwnd` is the live FastPad main window whose `GWLP_USERDATA`
    // owns an App. No App reference is held across the reentrant editor creation callback.
    if !identity.is_live_for(hwnd) {
        return Err(crate::FastPadError::Invariant(
            "main window identity was not live during editor initialization",
        ));
    }
    unsafe {
        record_milestone(hwnd, Milestone::WindowCreated)?;
    }

    let editor = create_editor(hwnd)?;
    let editor_hwnd = editor.hwnd();
    if !identity.is_live_for(hwnd) {
        return Err(crate::FastPadError::Invariant(
            "main window was destroyed during editor initialization",
        ));
    }

    let recovery_id = unsafe { app_ptr(hwnd) }
        .map(|app| unsafe { app.as_ref() }.allocate_recovery_id())
        .ok_or(crate::FastPadError::Invariant(
            "main window app state was not available",
        ))?;
    let document = Document::untitled(DocumentId(1), recovery_id, editor.current_document()?);
    if !identity.is_live_for(hwnd) {
        return Err(crate::FastPadError::Invariant(
            "main window was destroyed while adopting the initial document",
        ));
    }

    unsafe {
        install_editor(hwnd, editor, document)?;
        record_milestone(hwnd, Milestone::EditorCreated)?;
    }
    // The sidebar comes with the window, before first paint, from the settings bootstrap read.
    crate::window::side_panel::create_for_first_frame(hwnd, notes_mode_enabled(hwnd));
    Ok(editor_hwnd)
}

pub(crate) unsafe fn input_priority_requested(hwnd: HWND, identity: &WindowIdentity) -> bool {
    // SAFETY: This helper reads a raw App pointer stored in `GWLP_USERDATA` and copies a boolean
    // flag without returning references across the FFI boundary.
    if !identity.is_live_for(hwnd) {
        return false;
    }
    unsafe { app_ptr(hwnd) }
        .map(|app| unsafe { app.as_ref() }.prioritizes_input())
        .unwrap_or(false)
}

pub(crate) unsafe fn clear_input_priority(hwnd: HWND, identity: &WindowIdentity) {
    // SAFETY: This helper mutates a boolean flag through the window-owned App pointer and does not
    // retain any reference across reentrant Win32 calls.
    if !identity.is_live_for(hwnd) {
        return;
    }
    if let Some(mut app) = unsafe { app_ptr(hwnd) } {
        unsafe { app.as_mut() }.clear_input_priority();
    }
}

unsafe fn record_milestone(hwnd: HWND, milestone: Milestone) -> Result<()> {
    // SAFETY: The App pointer is owned by the window and is only used for an immediate milestone
    // write before returning to the caller.
    let Some(mut app) = (unsafe { app_ptr(hwnd) }) else {
        return Err(crate::FastPadError::Invariant(
            "main window app state was not available",
        ));
    };
    unsafe { app.as_mut() }.startup.record_now(milestone)
}

unsafe fn mark_first_paint_complete(hwnd: HWND) {
    // SAFETY: The App pointer is used only for an immediate state transition after painting.
    if let Some(mut app) = unsafe { app_ptr(hwnd) } {
        unsafe { app.as_mut() }.mark_first_paint_complete();
    }
}

unsafe fn take_deferred_start_pending(hwnd: HWND) -> bool {
    // SAFETY: The App pointer is used only for an immediate flag read-reset before returning.
    unsafe { app_ptr(hwnd) }
        .map(|mut app| unsafe { app.as_mut() }.take_deferred_start_pending())
        .unwrap_or(false)
}

unsafe fn request_input_priority(hwnd: HWND) {
    // SAFETY: The App pointer is used only for an immediate flag write before returning.
    if let Some(mut app) = unsafe { app_ptr(hwnd) } {
        unsafe { app.as_mut() }.request_input_priority();
    }
}

pub(crate) unsafe fn editor_hwnd(hwnd: HWND) -> Option<HWND> {
    // SAFETY: The App pointer is used only to copy out the child HWND; no reference crosses into
    // any subsequent Win32 call.
    let app = unsafe { app_ptr(hwnd) }?;
    unsafe { app.as_ref() }.editor.as_ref().map(Editor::hwnd)
}

fn with_editor(hwnd: HWND, action: impl FnOnce(&Editor)) {
    let Some(app) = (unsafe { app_ptr(hwnd) }) else {
        return;
    };
    let Some(editor) = unsafe { app.as_ref() }.editor.as_ref() else {
        return;
    };
    action(editor);
}

/// Lays out the sidebar, then the find bar, the name box, the preview and the editor right of it,
/// below the title strip. The sole layout choke point for all of them.
pub(crate) fn layout_editor_and_find_bar(hwnd: HWND) {
    let mut rect = RECT::default();
    unsafe {
        GetClientRect(hwnd, &mut rect);
    }
    let dpi = unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForWindow(hwnd) }.max(96);
    crate::window::side_panel::layout(hwnd, rect, dpi);
    let left = crate::window::side_panel::left_edge(hwnd);
    // The accessibility provider locates the tabs from this, never from the App.
    if let Some(app) = unsafe { app_ptr(hwnd) } {
        unsafe { app.as_ref() }.tabs.set_strip_left(left);
    }
    layout_command_palette(hwnd);
    let Some(editor_hwnd) = (unsafe { editor_hwnd(hwnd) }) else {
        return;
    };
    let title_height = title_layout(hwnd).height + menu_band_height(hwnd);
    let width = (rect.right - rect.left - left).max(0);
    let font = title_chrome(hwnd).1.text();
    let find_bar_height = unsafe { app_ptr(hwnd) }
        .and_then(|app| {
            let bar = unsafe { app.as_ref() }.find_bar.as_ref()?;
            bar.layout(left, width, title_height, dpi, font);
            bar.is_visible().then(|| find_bar::find_bar_height(dpi))
        })
        .unwrap_or(0);
    // Opening either bar closes the other, so at most one of the two bands is ever reserved.
    let name_box_height = unsafe { app_ptr(hwnd) }
        .and_then(|app| {
            let name_box = unsafe { app.as_ref() }.name_box.as_ref()?;
            name_box.layout(left, width, title_height + find_bar_height, dpi, font);
            name_box
                .is_visible()
                .then(|| crate::window::name_box::name_box_height(dpi))
        })
        .unwrap_or(0);
    let content_top = title_height + find_bar_height + name_box_height;
    let status_height = status_bar_height(hwnd);
    let area = RECT {
        left,
        top: content_top,
        right: left + width,
        bottom: (rect.bottom - rect.top - status_height).max(content_top),
    };
    let rects = crate::window::preview_host::layout(hwnd, area, dpi);
    if let Some(editor_rect) = rects.editor {
        unsafe {
            MoveWindow(
                editor_hwnd,
                editor_rect.left,
                editor_rect.top,
                editor_rect.right - editor_rect.left,
                editor_rect.bottom - editor_rect.top,
                1,
            );
        }
    }
}

fn open_find_bar(hwnd: HWND, mode: find_bar::FindBarMode) {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    // The two bars share the band above the editor; only one shows at a time.
    crate::window::library_host::close_name_box(hwnd);
    if !identity.is_live_for(hwnd) {
        return;
    }
    // A single-line selection is a reasonable query prefill; a multi-line one is not (the bar has
    // no way to display it), so it's left alone rather than truncated or rejected.
    let prefill = unsafe { app_ptr(hwnd) }.and_then(|app| {
        let editor = unsafe { app.as_ref() }.editor.as_ref()?;
        let text = editor.selected_text().ok()?;
        (!text.is_empty() && !text.contains(['\n', '\r'])).then_some(text)
    });
    let colors = title_chrome(hwnd).0;
    let opened = unsafe { app_ptr(hwnd) }.is_some_and(|mut app| {
        let app = unsafe { app.as_mut() };
        if app.find_bar.is_none() {
            app.find_bar = find_bar::FindBar::create(hwnd).ok();
        }
        let Some(bar) = app.find_bar.as_mut() else {
            return false;
        };
        bar.show(mode, prefill.as_deref(), colors);
        true
    });
    if !opened || !identity.is_live_for(hwnd) {
        return;
    }
    layout_editor_and_find_bar(hwnd);
    if let Some(app) = unsafe { app_ptr(hwnd) }
        && let Some(bar) = unsafe { app.as_ref() }.find_bar.as_ref()
    {
        bar.focus_query();
    }
}

pub(crate) fn close_find_bar(hwnd: HWND) {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    let closed = unsafe { app_ptr(hwnd) }.is_some_and(|mut app| {
        let Some(bar) = unsafe { app.as_mut() }.find_bar.as_mut() else {
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

/// Returns the keyboard focus to the content area.
pub(crate) fn focus_content(hwnd: HWND) {
    if let Some(target) = content_focus_target(hwnd) {
        unsafe {
            SetFocus(target);
        }
    }
}

/// The three parts F6 moves between, in tab order (spec §10).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FocusPart {
    ActivityBar,
    Panel,
    Editor,
}

/// The part after `current`, skipping a closed panel and, with notes mode off, the sidebar.
pub(crate) fn next_focus_part(
    current: FocusPart,
    backwards: bool,
    sidebar: bool,
    panel_open: bool,
) -> FocusPart {
    let parts: &[FocusPart] = match (sidebar, panel_open) {
        (false, _) => &[FocusPart::Editor],
        (true, false) => &[FocusPart::ActivityBar, FocusPart::Editor],
        (true, true) => &[FocusPart::ActivityBar, FocusPart::Panel, FocusPart::Editor],
    };
    let index = parts
        .iter()
        .position(|part| *part == current)
        .unwrap_or(parts.len() - 1);
    let next = if backwards {
        (index + parts.len() - 1) % parts.len()
    } else {
        (index + 1) % parts.len()
    };
    parts[next]
}

/// The editor, or the frame while no tab is open.
pub(crate) fn return_focus_to_editor(hwnd: HWND) {
    if tab_count(hwnd) > 0 {
        focus_content(hwnd);
    } else {
        unsafe {
            SetFocus(hwnd);
        }
    }
}

/// F6 and Shift+F6: activity bar, panel, editor.
pub(crate) fn cycle_focus(hwnd: HWND, backwards: bool) {
    use crate::config::SidebarView;
    use crate::window::side_panel;
    let windows = side_panel::windows(hwnd);
    let panel_open = windows.is_some() && side_panel::current_view(hwnd) != SidebarView::Hidden;
    let focus = unsafe { GetFocus() };
    let current = match windows {
        Some((bar, _)) if focus == bar => FocusPart::ActivityBar,
        Some((_, panel))
            if focus == panel
                || unsafe {
                    windows_sys::Win32::UI::WindowsAndMessaging::IsChild(panel, focus)
                } != 0 =>
        {
            FocusPart::Panel
        }
        _ => FocusPart::Editor,
    };
    match next_focus_part(current, backwards, windows.is_some(), panel_open) {
        FocusPart::ActivityBar => {
            if let Some((bar, _)) = windows {
                unsafe {
                    SetFocus(bar);
                }
            }
        }
        FocusPart::Panel => side_panel::show_view(hwnd, side_panel::current_view(hwnd), true),
        FocusPart::Editor => return_focus_to_editor(hwnd),
    }
}

/// The colors the find bar and the name box are shown in.
pub(crate) fn current_palette(hwnd: HWND) -> Palette {
    title_chrome(hwnd).0
}

/// The sidebar's fonts at the window's DPI, created on first use and again after a DPI change.
/// Null handles without a sidebar. Call it with nothing of the App borrowed.
pub(crate) fn ui_fonts(hwnd: HWND) -> crate::window::side_panel::UiFonts {
    let dpi = unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForWindow(hwnd) }.max(96);
    unsafe { app_ptr(hwnd) }
        .and_then(|mut app| {
            unsafe { app.as_mut() }
                .sidebar
                .as_mut()
                .map(|sidebar| sidebar.fonts(dpi))
        })
        .unwrap_or_default()
}

/// The height of the visible find bar or name box band above the editor, or 0.
fn bar_band_height(hwnd: HWND) -> i32 {
    let dpi = unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForWindow(hwnd) }.max(96);
    unsafe { app_ptr(hwnd) }.map_or(0, |app| {
        let app = unsafe { app.as_ref() };
        let find = app
            .find_bar
            .as_ref()
            .filter(|bar| bar.is_visible())
            .map_or(0, |_| find_bar::find_bar_height(dpi));
        let name = app
            .name_box
            .as_ref()
            .filter(|name_box| name_box.is_visible())
            .map_or(0, |_| crate::window::name_box::name_box_height(dpi));
        find + name
    })
}

/// Where keyboard focus belongs in the content area: the preview while it replaces the editor in
/// Full mode, otherwise the editor.
fn content_focus_target(hwnd: HWND) -> Option<HWND> {
    crate::window::preview_host::full_view_hwnd(hwnd).or_else(|| unsafe { editor_hwnd(hwnd) })
}

/// Overlays the palette at the top of the editor area, even with no tab open (New and Open stay
/// available then), below a visible find bar or name box so both stay usable.
fn layout_command_palette(hwnd: HWND) {
    if !with_command_palette(hwnd, CommandPalette::is_visible).unwrap_or(false) {
        return;
    }
    let top = title_layout(hwnd).height + menu_band_height(hwnd) + bar_band_height(hwnd);
    let mut rect = RECT::default();
    unsafe {
        GetClientRect(hwnd, &mut rect);
    }
    let dpi = unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForWindow(hwnd) }.max(96);
    let font = title_chrome(hwnd).1.text();
    let left = crate::window::side_panel::left_edge(hwnd);
    let width = (rect.right - rect.left - left).max(0);
    if let Some(mut app) = unsafe { app_ptr(hwnd) }
        && let Some(palette) = unsafe { app.as_mut() }.command_palette.as_mut()
    {
        palette.measure(width, dpi, font);
    }
    with_command_palette(hwnd, |palette| {
        palette.apply_layout(left, width, top, dpi, font)
    });
}

/// Runs `action` on the palette through a shared borrow only, so re-entrant window-procedure
/// calls its Win32 messages trigger can borrow the App again.
fn with_command_palette<R>(hwnd: HWND, action: impl FnOnce(&CommandPalette) -> R) -> Option<R> {
    let app = unsafe { app_ptr(hwnd) }?;
    unsafe { app.as_ref() }.command_palette.as_ref().map(action)
}

/// Records where focus should return, and the sidebar's focused note if the panel had the
/// keyboard focus, before the command palette or a picker takes it for its query field (spec
/// §6.3). `close_command_palette` restores the focus; `run_command_palette_selection` takes the
/// note for the command it runs.
fn capture_palette_focus(hwnd: HWND) {
    let panel = crate::window::side_panel::windows(hwnd).map(|(_, panel)| panel);
    let focused_panel = panel.filter(|&panel| unsafe { GetFocus() } == panel);
    let note = focused_panel.and_then(|_| crate::window::notebook_view::focused_note(hwnd));
    if let Some(mut app) = unsafe { app_ptr(hwnd) } {
        let app = unsafe { app.as_mut() };
        app.palette_note_target = note;
        app.palette_focus_return = focused_panel.unwrap_or(std::ptr::null_mut());
    }
}

/// Takes the note `capture_palette_focus` recorded, if any. Consumed at most once per palette
/// visit: by the command it runs, or discarded when the palette closes without running one.
fn take_palette_note_target(hwnd: HWND) -> Option<std::path::PathBuf> {
    unsafe { app_ptr(hwnd) }.and_then(|mut app| unsafe { app.as_mut() }.palette_note_target.take())
}

pub(crate) fn open_command_palette(hwnd: HWND) {
    show_command_palette(hwnd, None);
}

/// The activity bar's Settings button: the palette listing only `SETTINGS_COMMANDS`.
pub(crate) fn open_settings_palette(hwnd: HWND) {
    show_command_palette(hwnd, Some(command_palette::SETTINGS_COMMANDS));
}

fn show_command_palette(hwnd: HWND, subset: Option<&'static [CommandId]>) {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    capture_palette_focus(hwnd);
    let colors = title_chrome(hwnd).0;
    let newly_shown = unsafe { app_ptr(hwnd) }.and_then(|mut app| {
        let app = unsafe { app.as_mut() };
        if app.command_palette.is_none() {
            app.command_palette = CommandPalette::create(hwnd).ok();
        }
        let palette = app.command_palette.as_mut()?;
        let newly_shown = palette.mark_shown(colors);
        // Reopening the palette normally always shows commands, even right after a picker.
        palette.set_picker(None);
        palette.set_subset(subset);
        Some(newly_shown)
    });
    let Some(newly_shown) = newly_shown else {
        return;
    };
    // A query typed for the full list would hide most settings, so Settings always starts empty.
    if newly_shown || subset.is_some() {
        // Clearing the field sends EN_CHANGE, which lists every available command.
        with_command_palette(hwnd, CommandPalette::clear_query);
    }
    if !identity.is_live_for(hwnd) {
        return;
    }
    refilter_command_palette(hwnd);
    with_command_palette(hwnd, CommandPalette::focus_query);
}

/// Opens the palette in picker mode: it lists `picker`'s items instead of commands, and the
/// choice made on Enter goes to `library_host::picked` instead of running a command.
pub(crate) fn open_picker(hwnd: HWND, picker: command_palette::Picker) {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    capture_palette_focus(hwnd);
    let colors = title_chrome(hwnd).0;
    let newly_shown = unsafe { app_ptr(hwnd) }.and_then(|mut app| {
        let app = unsafe { app.as_mut() };
        if app.command_palette.is_none() {
            app.command_palette = CommandPalette::create(hwnd).ok();
        }
        let palette = app.command_palette.as_mut()?;
        let newly_shown = palette.mark_shown(colors);
        palette.set_picker(Some(picker));
        Some(newly_shown)
    });
    let Some(newly_shown) = newly_shown else {
        return;
    };
    if newly_shown {
        with_command_palette(hwnd, CommandPalette::clear_query);
    }
    if !identity.is_live_for(hwnd) {
        return;
    }
    refilter_command_palette(hwnd);
    with_command_palette(hwnd, CommandPalette::focus_query);
}

/// Hides the palette. `restore_focus` returns the focus to the editor (or the frame with no tab);
/// it is false when the focus already moved somewhere else.
pub(crate) fn close_command_palette(hwnd: HWND, restore_focus: bool) {
    let was_visible = unsafe { app_ptr(hwnd) }.is_some_and(|mut app| {
        unsafe { app.as_mut() }
            .command_palette
            .as_mut()
            .is_some_and(CommandPalette::mark_hidden)
    });
    if !was_visible {
        return;
    }
    with_command_palette(hwnd, CommandPalette::hide_controls);
    // Repaint what the palette covered now: a command run right after this (Find) can move the
    // editor before its queued paint, leaving the palette's pixels in the unpainted gaps.
    if let Some(editor_hwnd) = unsafe { editor_hwnd(hwnd) } {
        unsafe {
            windows_sys::Win32::Graphics::Gdi::UpdateWindow(editor_hwnd);
        }
    }
    // A leftover note (the palette closed without running the command that would consume it)
    // must not leak into some later, unrelated command.
    let _ = take_palette_note_target(hwnd);
    let panel_return = unsafe { app_ptr(hwnd) }.and_then(|mut app| {
        let app = unsafe { app.as_mut() };
        let panel = std::mem::replace(&mut app.palette_focus_return, std::ptr::null_mut());
        (!panel.is_null()).then_some(panel)
    });
    if restore_focus {
        // The panel had focus when the palette opened: give it back, rather than the editor.
        let target = panel_return.unwrap_or_else(|| {
            if tab_count(hwnd) > 0 {
                content_focus_target(hwnd).unwrap_or(hwnd)
            } else {
                hwnd
            }
        });
        unsafe {
            SetFocus(target);
        }
    }
}

fn refilter_command_palette(hwnd: HWND) {
    let Some(query) = with_command_palette(hwnd, |palette| {
        palette.is_visible().then(|| palette.query_text())
    })
    .flatten() else {
        return;
    };
    let is_picker =
        with_command_palette(hwnd, |palette| palette.picker().is_some()).unwrap_or(false);
    if is_picker {
        if let Some(mut app) = unsafe { app_ptr(hwnd) }
            && let Some(palette) = unsafe { app.as_mut() }.command_palette.as_mut()
            && let Some(picker) = palette.picker()
        {
            let rows = command_palette::picker_rows(picker, &query);
            palette.set_picker_rows(rows);
        }
    } else {
        let has_tabs = tab_count(hwnd) > 0;
        let markdown = crate::window::preview_host::buttons_visible(hwnd);
        let sidebar = notes_mode_enabled(hwnd);
        let subset = with_command_palette(hwnd, CommandPalette::subset).flatten();
        let entries = command_palette::filter_entries(&query, |command| {
            subset.is_none_or(|subset| subset.contains(&command))
                && (has_tabs || !command.needs_document())
                && (markdown || !command.is_markdown_preview())
                && (sidebar || !command.is_sidebar())
        });
        if let Some(mut app) = unsafe { app_ptr(hwnd) }
            && let Some(palette) = unsafe { app.as_mut() }.command_palette.as_mut()
        {
            palette.set_entries(entries);
        }
    }
    with_command_palette(hwnd, CommandPalette::fill_list);
    layout_command_palette(hwnd);
}

/// `WM_PAINT` for a palette, find bar or name box panel.
pub(crate) fn paint_panel(hwnd: HWND, panel: HWND) {
    let fonts = title_chrome(hwnd).1;
    let (glyph_font, text_font) = (fonts.glyph(), fonts.text());
    let painted = unsafe { app_ptr(hwnd) }.is_some_and(|app| {
        let app = unsafe { app.as_ref() };
        if let Some(palette) = app.command_palette.as_ref().filter(|p| p.owns(panel)) {
            palette.paint_panel(panel);
            true
        } else if let Some(bar) = app.find_bar.as_ref().filter(|bar| bar.owns(panel)) {
            bar.paint_panel(panel, glyph_font);
            true
        } else if let Some(name_box) = app.name_box.as_ref().filter(|n| n.owns(panel)) {
            name_box.paint_panel(panel, text_font);
            true
        } else {
            false
        }
    });
    if !painted {
        // Validates the region so an orphaned panel does not repaint forever.
        unsafe {
            windows_sys::Win32::Graphics::Gdi::ValidateRect(panel, std::ptr::null());
        }
    }
}

pub(crate) fn run_command_palette_selection(hwnd: HWND) {
    let pick = with_command_palette(hwnd, |palette| {
        palette
            .picker()
            .map(|picker| (picker.kind, palette.selected_choice()))
    })
    .flatten();
    let command = if pick.is_none() {
        with_command_palette(hwnd, CommandPalette::selected_command).flatten()
    } else {
        None
    };
    // Taken before closing moves focus off the sidebar panel, so a note-scoped command still
    // knows which row was focused when the palette opened (spec §6.3).
    let note = take_palette_note_target(hwnd);
    close_command_palette(hwnd, true);
    match pick {
        Some((kind, Some(choice))) => crate::window::library_host::picked(hwnd, kind, choice),
        Some((_, None)) => {}
        None => {
            if let Some(command) = command {
                execute_command_with_note(hwnd, command, note);
            }
        }
    }
}

pub(crate) fn move_command_palette_selection(hwnd: HWND, step: isize) {
    with_command_palette(hwnd, |palette| palette.move_selection(step));
}

pub(crate) fn select_command_palette_row(hwnd: HWND, lparam: LPARAM) -> bool {
    with_command_palette(hwnd, |palette| palette.select_row_at(lparam)).unwrap_or(false)
}

pub(crate) fn focus_command_palette(hwnd: HWND) {
    with_command_palette(hwnd, CommandPalette::focus_query);
}

pub(crate) fn command_palette_owns(hwnd: HWND, control: HWND) -> bool {
    !control.is_null()
        && with_command_palette(hwnd, |palette| palette.owns(control)).unwrap_or(false)
}

/// Mouse input on a panel: hovering and clicking the find bar's close button.
pub(crate) fn panel_pointer(hwnd: HWND, panel: HWND, message: u32, lparam: LPARAM) {
    let close = unsafe { app_ptr(hwnd) }.is_some_and(|app| {
        unsafe { app.as_ref() }
            .find_bar
            .as_ref()
            .filter(|bar| bar.owns(panel))
            .is_some_and(|bar| bar.pointer(message, lparam))
    });
    if close {
        close_find_bar(hwnd);
    }
}

/// `WM_PAINT` for an empty find field; false when there is no find bar to paint it.
pub(crate) fn paint_find_placeholder(hwnd: HWND, edit: HWND) -> bool {
    unsafe { app_ptr(hwnd) }.is_some_and(|app| {
        unsafe { app.as_ref() }
            .find_bar
            .as_ref()
            .is_some_and(|bar| bar.paint_placeholder(edit))
    })
}

fn name_box_owns(hwnd: HWND, control: HWND) -> bool {
    unsafe { app_ptr(hwnd) }.is_some_and(|app| {
        unsafe { app.as_ref() }
            .name_box
            .as_ref()
            .is_some_and(|name_box| name_box.owns(control))
    })
}

fn find_bar_owns(hwnd: HWND, control: HWND) -> bool {
    unsafe { app_ptr(hwnd) }.is_some_and(|app| {
        unsafe { app.as_ref() }
            .find_bar
            .as_ref()
            .is_some_and(|bar| bar.owns(control))
    })
}

pub(crate) fn find_next(hwnd: HWND) {
    navigate_to_match(hwnd, false);
}

pub(crate) fn find_previous(hwnd: HWND) {
    navigate_to_match(hwnd, true);
}

fn navigate_to_match(hwnd: HWND, backward: bool) {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    let Some((editor, query)) = (unsafe { app_ptr(hwnd) }).and_then(|app| {
        let app = unsafe { app.as_ref() };
        let editor = app.editor.clone()?;
        let query = app.find_bar.as_ref()?.query_text();
        Some((editor, query))
    }) else {
        return;
    };
    if query.is_empty() {
        return;
    }
    let (Ok(selection), Ok(doc_len)) = (editor.selection(), editor.length()) else {
        return;
    };
    let origin = if backward {
        selection.start
    } else {
        selection.end
    };
    let direction = if backward {
        find_bar::SearchDirection::Backward
    } else {
        find_bar::SearchDirection::Forward
    };
    let mut state = find_bar::SearchState::new(&query, direction, origin);
    if let Ok(Some(found)) = state.next_editor_match(&editor, 0, doc_len)
        && identity.is_live_for(hwnd)
    {
        let _ = editor.set_selection(found);
        editor.scroll_caret_into_view();
    }
}

pub(crate) fn replace_current(hwnd: HWND) {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    let Some((editor, query, replacement)) = (unsafe { app_ptr(hwnd) }).and_then(|app| {
        let app = unsafe { app.as_ref() };
        let editor = app.editor.clone()?;
        let bar = app.find_bar.as_ref()?;
        Some((editor, bar.query_text(), bar.replace_text()))
    }) else {
        return;
    };
    if query.is_empty() {
        return;
    }
    // Only replace when the current selection is exactly the query match; otherwise this Enter
    // press just navigates to the next match, matching a bare Find field's behavior.
    if let (Ok(selection), Ok(selected)) = (editor.selection(), editor.selected_text())
        && selected == query
    {
        let _ = editor.replace_target(selection, &replacement);
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
    let Some((editor, query, replacement)) = (unsafe { app_ptr(hwnd) }).and_then(|app| {
        let app = unsafe { app.as_ref() };
        let editor = app.editor.clone()?;
        let bar = app.find_bar.as_ref()?;
        Some((editor, bar.query_text(), bar.replace_text()))
    }) else {
        return;
    };
    if query.is_empty() {
        return;
    }
    let _ = editor.replace_all(&query, &replacement, 0);
    if identity.is_live_for(hwnd) {
        editor.scroll_caret_into_view();
    }
}

const EMPTY_TABS_HINT: &str =
    "No tabs are open.\nPress Ctrl+N or double-click the tab bar to start a new one.";

pub(crate) fn tab_count(hwnd: HWND) -> usize {
    unsafe { app_ptr(hwnd) }
        .map(|app| unsafe { app.as_ref() }.tabs.len())
        .unwrap_or(1)
}

fn tab_scroll(hwnd: HWND) -> i32 {
    unsafe { app_ptr(hwnd) }
        .map(|app| unsafe { app.as_ref() }.tabs.scroll_offset())
        .unwrap_or(0)
}

pub(crate) fn title_layout(hwnd: HWND) -> TitleBarLayout {
    crate::window::titlebar::layout_for_window(
        hwnd,
        tab_count(hwnd),
        tab_scroll(hwnd),
        preview_buttons_visible(hwnd),
    )
}

/// Whether the title strip shows the Markdown preview buttons (the active tab is Markdown).
fn preview_buttons_visible(hwnd: HWND) -> bool {
    crate::window::preview_host::buttons_visible(hwnd)
}

/// Titles, active index, scroll offset, and whether the editor is hidden because no tab is open.
/// The frame's window text, which the taskbar button and Alt+Tab show: the active tab's title as
/// the tab strip paints it, followed by the app name.
fn window_title(active_tab: Option<&str>) -> String {
    active_tab.map_or_else(
        || "FastPad".to_owned(),
        |title| format!("{title} - FastPad"),
    )
}

/// Brings the window text in line with the active tab. Runs on every frame paint, since every tab
/// change (switch, open, close, save, dirty state) repaints the title strip.
fn sync_window_title(hwnd: HWND) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetWindowTextW, SetWindowTextW};
    let Some(title) = (unsafe { app_ptr(hwnd) }).map(|app| {
        window_title(
            unsafe { app.as_ref() }
                .tabs
                .active()
                .map(Document::title)
                .as_deref(),
        )
    }) else {
        return;
    };
    let wanted = title.encode_utf16().collect::<Vec<_>>();
    // One spare unit so a longer current title never reads as equal after truncation.
    let mut current = vec![0u16; wanted.len() + 2];
    let len = unsafe { GetWindowTextW(hwnd, current.as_mut_ptr(), current.len() as i32) };
    if current[..len.max(0) as usize] != wanted[..] {
        unsafe {
            SetWindowTextW(hwnd, wide_null(&title).as_ptr());
        }
    }
}

fn tab_snapshot(hwnd: HWND) -> (Vec<String>, usize, i32, bool, Option<usize>) {
    unsafe { app_ptr(hwnd) }
        .map(|app| {
            let app = unsafe { app.as_ref() };
            (
                app.tabs.titles().collect(),
                app.tabs.active_index(),
                app.tabs.scroll_offset(),
                app.editor.is_some() && app.tabs.is_empty(),
                app.tabs.documents().position(|document| document.preview),
            )
        })
        .unwrap_or_else(|| (vec!["Untitled".to_owned()], 0, 0, false, None))
}

/// Scrolls the tabs when the wheel turns over the tab strip; reports whether it was over it.
fn scroll_tabs(hwnd: HWND, lparam: LPARAM, delta: i32) -> bool {
    let mut point = windows_sys::Win32::Foundation::POINT {
        x: (lparam as u32 & 0xffff) as u16 as i16 as i32,
        y: ((lparam as u32 >> 16) & 0xffff) as u16 as i16 as i32,
    };
    unsafe {
        windows_sys::Win32::Graphics::Gdi::ScreenToClient(hwnd, &mut point);
    }
    let layout = title_layout(hwnd);
    if point.y < 0
        || point.y >= layout.height
        || point.x < layout.tabs.left
        || point.x >= layout.overflow.left
    {
        return false;
    }
    let scroll = layout.scroll_by_wheel(delta, WHEEL_DELTA as i32);
    let changed = unsafe { app_ptr(hwnd) }
        .is_some_and(|app| unsafe { app.as_ref() }.tabs.set_scroll_offset(scroll));
    if changed {
        // The hovered tab moved out from under the pointer; the next mouse move finds the new one.
        update_title_pointer(hwnd, |pointer| pointer.hover(None));
        crate::window::titlebar::invalidate_strip(hwnd);
    }
    true
}

/// Starts dragging the tab scroll thumb. Pressing the track beside the thumb first jumps the
/// thumb there, centred under the pointer, so the same press can keep dragging it.
fn begin_tab_thumb_drag(hwnd: HWND, x: i32) {
    let layout = title_layout(hwnd);
    let Some(thumb) = layout.scroll_thumb() else {
        return;
    };
    let grab = if (thumb.left..thumb.right).contains(&x) {
        x - thumb.left
    } else {
        let grab = (thumb.right - thumb.left) / 2;
        if let Some(app) = unsafe { app_ptr(hwnd) } {
            unsafe { app.as_ref() }
                .tabs
                .set_scroll_offset(layout.scroll_for_thumb(x - grab));
        }
        grab
    };
    if let Some(mut app) = unsafe { app_ptr(hwnd) } {
        unsafe { app.as_mut() }.tab_thumb_grab = Some(grab);
    }
    unsafe {
        SetCapture(hwnd);
    }
    crate::window::titlebar::invalidate_strip(hwnd);
}

fn drag_tab_thumb(hwnd: HWND, lparam: LPARAM) {
    let Some(grab) =
        unsafe { app_ptr(hwnd) }.and_then(|app| unsafe { app.as_ref() }.tab_thumb_grab)
    else {
        return;
    };
    let x = (lparam as u32 & 0xffff) as u16 as i16 as i32;
    let scroll = title_layout(hwnd).scroll_for_thumb(x - grab);
    if unsafe { app_ptr(hwnd) }
        .is_some_and(|app| unsafe { app.as_ref() }.tabs.set_scroll_offset(scroll))
    {
        crate::window::titlebar::invalidate_strip(hwnd);
    }
}

/// Ends a thumb drag; reports whether one was in progress, so the release activates nothing.
fn end_tab_thumb_drag(hwnd: HWND) -> bool {
    let dragging = unsafe { app_ptr(hwnd) }
        .and_then(|mut app| unsafe { app.as_mut() }.tab_thumb_grab.take())
        .is_some();
    if dragging {
        unsafe {
            ReleaseCapture();
        }
    }
    dragging
}

/// Follows every change to the set of tabs or the active one: scrolls the active tab into view,
/// shows the editor only while a tab is open, and repaints.
fn refresh_tabs(hwnd: HWND) {
    let Some((count, active, editor_hwnd)) = (unsafe { app_ptr(hwnd) }).map(|app| {
        let app = unsafe { app.as_ref() };
        (
            app.tabs.len(),
            app.tabs.active_index(),
            app.editor.as_ref().map(Editor::hwnd),
        )
    }) else {
        return;
    };
    let scroll = if count == 0 {
        0
    } else {
        title_layout(hwnd).scroll_to_reveal(active)
    };
    if let Some(app) = unsafe { app_ptr(hwnd) } {
        unsafe { app.as_ref() }.tabs.set_scroll_offset(scroll);
    }
    if let Some(editor_hwnd) = editor_hwnd {
        let visible = unsafe { GetWindowLongPtrW(editor_hwnd, GWL_STYLE) } as u32 & WS_VISIBLE != 0;
        if count == 0 && visible {
            close_find_bar(hwnd);
            unsafe {
                ShowWindow(editor_hwnd, SW_HIDE);
                if GetFocus() == editor_hwnd {
                    SetFocus(hwnd);
                }
            }
        } else if count > 0 && !visible {
            unsafe {
                ShowWindow(editor_hwnd, SW_SHOWNA);
                if GetFocus() == hwnd {
                    SetFocus(editor_hwnd);
                }
            }
        }
    }
    crate::window::library_host::close_stale_name_box(hwnd);
    unsafe {
        InvalidateRect(hwnd, std::ptr::null(), 0);
    }
    crate::window::preview_host::sync_visibility(hwnd);
    crate::window::library_host::refresh_label(hwnd);
    crate::window::side_panel::active_tab_changed(hwnd);
}

/// Commands that act on a note rather than a command in the general sense (spec §6.3): the row
/// recorded when the palette opened or currently focused in the sidebar's tree, else (except
/// Rename and Delete, whose no-target path looks up the active tab itself) the active tab's file.
fn is_note_command(command: CommandId) -> bool {
    matches!(
        command,
        CommandId::NoteTogglePin
            | CommandId::NoteMoveToNotebook
            | CommandId::NoteRevealInExplorer
            | CommandId::NoteRename
            | CommandId::NoteDelete
    )
}

/// Pin/Move to notebook/Reveal's target: `tree` (the row the palette recorded, or the tree's
/// currently focused row), else the active tab's file.
fn note_target(hwnd: HWND, tree: Option<&std::path::Path>) -> Option<std::path::PathBuf> {
    tree.map(std::path::Path::to_path_buf)
        .or_else(|| crate::window::library_host::active_file(hwnd))
}

fn execute_command(hwnd: HWND, command: CommandId) {
    execute_command_with_note(hwnd, command, None);
}

/// `execute_command`, for a command chosen from the command palette: `recorded` is the sidebar
/// row that `capture_palette_focus` recorded when the palette opened, taken by
/// `run_command_palette_selection` before closing it moved focus off the panel (spec §6.3).
fn execute_command_with_note(hwnd: HWND, command: CommandId, recorded: Option<std::path::PathBuf>) {
    exit_menu_mode(hwnd);
    if file_population_active(hwnd) {
        return;
    }
    let tree_note = is_note_command(command)
        .then(|| recorded.or_else(|| crate::window::notebook_view::focused_note(hwnd)))
        .flatten();
    // A focused (or recorded) note lets a note-scoped command through even with no tab open.
    if command.needs_document() && tab_count(hwnd) == 0 && tree_note.is_none() {
        return;
    }
    if let Some(index) = command.tab_index() {
        if index < tab_count(hwnd) {
            activate_tab(hwnd, index);
        }
        return;
    }
    match command {
        CommandId::Open => {
            let identity = unsafe { window_identity(hwnd) };
            // Modal Show reenters the window procedure. Only an owned identity crosses it.
            let selection = crate::window::modal::choose_open_path(hwnd);
            if identity
                .as_ref()
                .is_some_and(|identity| identity.is_live_for(hwnd))
            {
                match selection {
                    Ok(Some(path)) => {
                        if let Err(error) = App::open_path(hwnd, &path) {
                            report_open_failure(hwnd, &path, &error);
                        }
                    }
                    Ok(None) => {}
                    Err(error) => push_notice(
                        hwnd,
                        format!("FastPad could not show the Open dialog: {error}"),
                    ),
                }
            }
        }
        CommandId::New => crate::window::library_host::new_note_in(hwnd, None),
        CommandId::CloseTab => close_active_document(hwnd),
        CommandId::CloseAllTabs => close_all_documents(hwnd),
        CommandId::Save => crate::window::library_host::save_command(hwnd),
        CommandId::SaveAs => crate::window::library_host::save_as_command(hwnd),
        CommandId::Undo => with_editor(hwnd, |editor| {
            let _ = editor.undo();
        }),
        CommandId::Redo => with_editor(hwnd, |editor| {
            let _ = editor.redo();
        }),
        CommandId::Cut => with_editor(hwnd, |editor| {
            let _ = editor.cut();
        }),
        CommandId::Copy => with_editor(hwnd, |editor| {
            let _ = editor.copy();
        }),
        CommandId::Paste => with_editor(hwnd, |editor| {
            let _ = editor.paste();
        }),
        CommandId::Find => open_find_bar(hwnd, find_bar::FindBarMode::Find),
        CommandId::CommandPalette => open_command_palette(hwnd),
        CommandId::ThemeSystem => set_theme(hwnd, crate::config::ThemePreference::System),
        CommandId::ThemeLight => set_theme(hwnd, crate::config::ThemePreference::Light),
        CommandId::ThemeDark => set_theme(hwnd, crate::config::ThemePreference::Dark),
        CommandId::ThemeCatppuccin => set_theme(hwnd, crate::config::ThemePreference::Catppuccin),
        CommandId::ThemeCatppuccinLatte => {
            set_theme(hwnd, crate::config::ThemePreference::CatppuccinLatte)
        }
        CommandId::ThemeCatppuccinFrappe => {
            set_theme(hwnd, crate::config::ThemePreference::CatppuccinFrappe)
        }
        CommandId::ThemeCatppuccinMacchiato => {
            set_theme(hwnd, crate::config::ThemePreference::CatppuccinMacchiato)
        }
        CommandId::ThemeCatppuccinMocha => {
            set_theme(hwnd, crate::config::ThemePreference::CatppuccinMocha)
        }
        CommandId::ToggleWordWrap => change_setting(hwnd, |settings| {
            settings.word_wrap = !settings.word_wrap;
            Some(("word_wrap", settings.word_wrap.to_string()))
        }),
        CommandId::ToggleLineNumbers => change_setting(hwnd, |settings| {
            settings.line_numbers = !settings.line_numbers;
            Some(("line_numbers", settings.line_numbers.to_string()))
        }),
        CommandId::ToggleRestoreSession => {
            change_setting(hwnd, |settings| {
                settings.restore_session = !settings.restore_session;
                Some(("restore_session", settings.restore_session.to_string()))
            });
            let enabled = unsafe { app_ptr(hwnd) }
                .is_some_and(|app| unsafe { app.as_ref() }.settings.restore_session);
            push_notice(hwnd, crate::session::toggle_notice(enabled).to_owned());
        }
        CommandId::ToggleNotesMode => {
            change_setting(hwnd, |settings| {
                settings.notes_mode = !settings.notes_mode;
                Some(("notes_mode", settings.notes_mode.to_string()))
            });
            let enabled = unsafe { app_ptr(hwnd) }
                .is_some_and(|app| unsafe { app.as_ref() }.settings.notes_mode);
            crate::window::library_host::notes_mode_changed(hwnd, enabled);
            crate::window::side_panel::notes_mode_changed(hwnd, enabled);
            if enabled {
                // The library step ran before the sidebar existed; its view catches up here.
                crate::window::side_panel::refresh(hwnd);
                crate::window::library_host::show_labels(hwnd);
            } else {
                crate::window::library_host::clear_labels(hwnd);
            }
            push_notice(
                hwnd,
                crate::window::library_host::notes_mode_notice(enabled).to_owned(),
            );
        }
        CommandId::OpenFolder => crate::window::library_host::choose_and_open_folder(hwnd),
        CommandId::OpenRecentFolder => crate::window::library_host::open_recent_folder_picker(hwnd),
        CommandId::ToggleFolderAutosave => {
            crate::window::library_host::toggle_folder_autosave(hwnd);
        }
        CommandId::NoteReloadFromDisk => crate::window::library_host::reload_from_disk(hwnd),
        CommandId::NoteKeepMine => crate::window::library_host::keep_mine(hwnd),
        CommandId::NoteTogglePin => {
            if let Some(path) = note_target(hwnd, tree_note.as_deref()) {
                crate::window::library_host::toggle_pin(hwnd, &path);
            }
        }
        CommandId::NoteMoveToNotebook => {
            if let Some(path) = note_target(hwnd, tree_note.as_deref()) {
                crate::window::library_host::move_to_notebook(hwnd, &path);
            }
        }
        CommandId::NoteRevealInExplorer => {
            if let Some(path) = note_target(hwnd, tree_note.as_deref()) {
                crate::window::library_host::reveal(hwnd, &path);
            }
        }
        CommandId::NoteRename => {
            if crate::window::library_host::ready_library(hwnd) {
                match &tree_note {
                    Some(path) => crate::window::library_host::rename_file(hwnd, path),
                    None => crate::window::library_host::rename_note(hwnd),
                }
            }
        }
        CommandId::NoteDelete => {
            if crate::window::library_host::ready_library(hwnd) {
                match &tree_note {
                    Some(path) => crate::window::library_host::delete_file(hwnd, path),
                    None => crate::window::library_host::delete_note(hwnd),
                }
            }
        }
        CommandId::CloseNotebook => crate::window::library_host::close_notebook(hwnd),
        CommandId::ToggleNotebookFavorite => {
            crate::window::library_host::toggle_notebook_favorite(hwnd);
        }
        CommandId::FontSizeIncrease => {
            set_font_size(hwnd, |size| {
                size.saturating_add(1).min(MAX_FONT_SIZE.max(size))
            });
        }
        CommandId::FontSizeDecrease => {
            set_font_size(hwnd, |size| {
                size.saturating_sub(1).max(MIN_FONT_SIZE.min(size))
            });
        }
        CommandId::FontSizeReset => {
            set_font_size(hwnd, |_| crate::config::defaults::DEFAULT_FONT_SIZE);
        }
        CommandId::TabWidth2 => set_tab_width(hwnd, 2),
        CommandId::TabWidth4 => set_tab_width(hwnd, 4),
        CommandId::TabWidth8 => set_tab_width(hwnd, 8),
        CommandId::Replace => open_find_bar(hwnd, find_bar::FindBarMode::Replace),
        CommandId::LanguagePlainText => apply_language(hwnd, crate::document::Language::PlainText),
        CommandId::LanguageJson => apply_language(hwnd, crate::document::Language::Json),
        CommandId::LanguageMarkdown => apply_language(hwnd, crate::document::Language::Markdown),
        CommandId::ValidateJson => validate_active_json(hwnd),
        CommandId::FormatJson => format_active_json(hwnd),
        CommandId::NextTab => cycle_tab(hwnd, true),
        CommandId::PreviousTab => cycle_tab(hwnd, false),
        CommandId::ZoomIn => with_editor(hwnd, |editor| {
            let _ = editor.zoom_in();
        }),
        CommandId::ZoomOut => with_editor(hwnd, |editor| {
            let _ = editor.zoom_out();
        }),
        CommandId::ZoomReset => with_editor(hwnd, |editor| {
            let _ = editor.reset_zoom();
        }),
        CommandId::TextLeftToRight => with_editor(hwnd, |editor| {
            let _ = editor.set_text_direction(TextDirection::LeftToRight);
        }),
        CommandId::TextRightToLeft => with_editor(hwnd, |editor| {
            let _ = editor.set_text_direction(TextDirection::RightToLeft);
        }),
        CommandId::MarkdownPreviewCycle
        | CommandId::MarkdownPreviewSide
        | CommandId::MarkdownPreviewFull
        | CommandId::MarkdownPreviewClose => {
            crate::window::preview_host::run_command(hwnd, command)
        }
        CommandId::ToggleSidebar => crate::window::side_panel::toggle(hwnd),
        CommandId::FocusNextPane => cycle_focus(hwnd, false),
        CommandId::FocusPreviousPane => cycle_focus(hwnd, true),
        CommandId::ShowNotebookView => {
            crate::window::side_panel::show_view(hwnd, crate::config::SidebarView::Notebook, true)
        }
        CommandId::ShowSearchView => {
            crate::window::side_panel::show_view(hwnd, crate::config::SidebarView::Search, true)
        }
        CommandId::ShowFavoritesView => {
            crate::window::side_panel::show_view(hwnd, crate::config::SidebarView::Favorites, true)
        }
        _ => {
            if let Some(mut app) = unsafe { app_ptr(hwnd) } {
                unsafe { app.as_mut() }.execute(command);
            }
        }
    }
}

/// Font-size commands step within this range; a size set outside it in `fastpad.ini` is kept
/// until a step moves it back toward the range.
const MIN_FONT_SIZE: u16 = 6;
const MAX_FONT_SIZE: u16 = 72;

fn set_font_size(hwnd: HWND, next: impl FnOnce(u16) -> u16) {
    change_setting(hwnd, |settings| {
        let size = next(settings.font_size);
        (size != settings.font_size).then(|| {
            settings.font_size = size;
            ("font_size", size.to_string())
        })
    });
}

fn set_tab_width(hwnd: HWND, width: u8) {
    change_setting(hwnd, |settings| {
        (settings.tab_width != width).then(|| {
            settings.tab_width = width;
            ("tab_width", width.to_string())
        })
    });
}

fn set_theme(hwnd: HWND, theme: crate::config::ThemePreference) {
    change_setting(hwnd, |settings| {
        (settings.theme != theme).then(|| {
            settings.theme = theme;
            ("theme", theme.ini_value().to_owned())
        })
    });
}

/// Applies one settings change from a command and saves it to `fastpad.ini`. `change` edits the
/// in-memory settings and names the `key=value` it made, or returns `None` when nothing changed.
pub(crate) fn change_setting(
    hwnd: HWND,
    change: impl FnOnce(&mut crate::config::Settings) -> Option<(&'static str, String)>,
) {
    let Some((previous_theme, (key, value))) = unsafe { app_ptr(hwnd) }.and_then(|mut app| {
        let settings = &mut unsafe { app.as_mut() }.settings;
        let previous_theme = settings.theme;
        Some((previous_theme, change(settings)?))
    }) else {
        return;
    };
    let theme_changed = unsafe { app_ptr(hwnd) }
        .is_some_and(|app| unsafe { app.as_ref() }.settings.theme != previous_theme);
    // The sidebar's view and width change only the layout, which their callers redo; the editor
    // and the Markdown preview are not restyled for them.
    let sidebar_only = matches!(key, "sidebar_view" | "sidebar_width");
    if theme_changed {
        apply_theme(hwnd);
        unsafe {
            InvalidateRect(hwnd, std::ptr::null(), 1);
        }
    } else if !sidebar_only {
        apply_editor_settings(hwnd);
    }
    if let Err(error) = save_setting(key, &value) {
        push_notice(hwnd, format!("FastPad could not save fastpad.ini: {error}"));
    }
}

#[cfg(not(test))]
fn save_setting(key: &str, value: &str) -> Result<()> {
    crate::config::save_setting(key, value)
}

/// Tests never touch the real `%LocalAppData%\FastPadastpad.ini`: a setting is saved only to a
/// path a test chose with `save_settings_to`.
#[cfg(test)]
fn save_setting(key: &str, value: &str) -> Result<()> {
    TEST_SETTINGS_PATH.with(|path| match path.borrow().as_deref() {
        Some(path) => crate::config::save_setting_to(path, key, value),
        None => Ok(()),
    })
}

#[cfg(test)]
thread_local! {
    static TEST_SETTINGS_PATH: std::cell::RefCell<Option<std::path::PathBuf>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
#[allow(
    dead_code,
    reason = "not every source-linked test target saves settings"
)]
pub(crate) fn save_settings_to(path: Option<std::path::PathBuf>) {
    TEST_SETTINGS_PATH.with(|slot| *slot.borrow_mut() = path);
}

pub(super) fn file_population_active(hwnd: HWND) -> bool {
    unsafe { app_ptr(hwnd) }.is_some_and(|app| unsafe { app.as_ref() }.populating_file)
}

/// Detects the active document's language from its path (an untitled document has no path and
/// stays whatever it already is, i.e. plain text) and applies it. Reached only after `input_pending`
/// is false for `WM_FASTPAD_APPLY_LANGUAGE` (see `handle_deferred`), so this never runs ahead of
/// queued user input.
fn apply_detected_language(hwnd: HWND) {
    let path = unsafe { app_ptr(hwnd) }
        .and_then(|app| unsafe { app.as_ref() }.tabs.active()?.path.clone());
    let Some(path) = path else {
        return;
    };
    apply_language(hwnd, crate::languages::detect_language(&path));
}

/// Applies `language`'s lexer to the active editor via the (lazily created, per Task 13's
/// `find_bar`/`menu_bar`-style `Option<T>` precedent) `App::language_manager`, then records the
/// outcome on the active document's metadata: success updates `Document::language` to match what
/// is now actually shown; failure leaves the document's language metadata unchanged (the editor
/// itself is also left unchanged by `LanguageManager::apply` on failure) and surfaces the error.
fn apply_language(hwnd: HWND, language: crate::document::Language) {
    let Some(editor) =
        (unsafe { app_ptr(hwnd) }).and_then(|app| unsafe { app.as_ref() }.editor.clone())
    else {
        return;
    };
    let theme = effective_theme(hwnd);
    let result = unsafe { app_ptr(hwnd) }.map(|mut app| {
        let app = unsafe { app.as_mut() };
        if app.language_manager.is_none() {
            app.language_manager = Some(crate::languages::LanguageManager::new());
        }
        app.language_manager
            .as_mut()
            .expect("just populated above if it was absent")
            .apply(&editor, language, theme)
    });
    match result {
        Some(Ok(())) => {
            if let Some(mut app) = unsafe { app_ptr(hwnd) } {
                unsafe { app.as_mut() }.tabs.set_active_language(language);
            }
            invalidate_status_bar(hwnd);
            // Lexer style tables reset every style's font face; restore the configured one.
            apply_editor_settings(hwnd);
            crate::window::preview_host::sync_visibility(hwnd);
        }
        Some(Err(_)) => push_notice(
            hwnd,
            "FastPad could not enable syntax highlighting for this file. It will remain in plain \
             text."
                .to_owned(),
        ),
        None => {}
    }
}

/// Runs only inside `WM_FASTPAD_LOAD_SETTINGS`: applies the settings `bootstrap::run` read before
/// the window existed (or resolves and parses `fastpad.ini` now when nothing was preloaded),
/// applies the editor view settings in place, and queues every rejected line as a non-modal
/// notification.
fn load_settings(hwnd: HWND) {
    let preloaded = unsafe { app_ptr(hwnd) }.and_then(|mut app| {
        let app = unsafe { app.as_mut() };
        let warnings = app.preloaded_settings_warnings.take()?;
        Some((app.settings.clone(), warnings))
    });
    let (settings, warnings) = preloaded.unwrap_or_else(crate::config::load);
    apply_loaded_settings(hwnd, settings, warnings);
    start_recovery_timer(hwnd);
}

/// Applies an already-loaded settings/warnings pair, split out of `load_settings` so tests can drive
/// the reporting path directly instead of mutating the process-wide `LOCALAPPDATA` environment
/// variable to fake a corrupt `fastpad.ini` on disk.
fn apply_loaded_settings(
    hwnd: HWND,
    settings: crate::config::Settings,
    warnings: Vec<crate::config::SettingWarning>,
) {
    let mut sidebar_changed = true;
    if let Some(mut app) = unsafe { app_ptr(hwnd) } {
        let app = unsafe { app.as_mut() };
        let sidebar = |settings: &crate::config::Settings| {
            (
                settings.notes_mode,
                settings.sidebar_view,
                settings.sidebar_width,
            )
        };
        // Settings `bootstrap::run` preloaded already made the sidebar the first frame shows.
        sidebar_changed = sidebar(&app.settings) != sidebar(&settings)
            || app.sidebar.is_some() != settings.notes_mode;
        app.settings = settings;
        for warning in &warnings {
            app.notifications.push(settings_warning_message(warning));
        }
    }
    apply_editor_settings(hwnd);
    if sidebar_changed {
        let notes_mode = notes_mode_enabled(hwnd);
        crate::window::side_panel::notes_mode_changed(hwnd, notes_mode);
    }
}

fn settings_warning_message(warning: &crate::config::SettingWarning) -> String {
    if warning.line == 0 {
        format!("fastpad.ini: {}", warning.message)
    } else {
        format!("fastpad.ini line {}: {}", warning.line, warning.message)
    }
}

#[cfg(test)]
thread_local! {
    static EDITOR_SETTINGS_APPLIED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// How many times this thread has applied the editor settings, for the tests that check a
/// sidebar change does not.
#[cfg(test)]
fn editor_settings_applied() -> usize {
    EDITOR_SETTINGS_APPLIED.with(std::cell::Cell::get)
}

/// Also recolors the line numbers: this runs after every lexer change, whose style reset gives
/// the gutter full-contrast text. Before chrome exists the neutral palette matches Scintilla's own
/// black-on-white defaults.
fn apply_editor_settings(hwnd: HWND) {
    #[cfg(test)]
    EDITOR_SETTINGS_APPLIED.with(|count| count.set(count.get() + 1));
    let Some((editor, settings, palette)) = (unsafe { app_ptr(hwnd) }).and_then(|app| {
        let app = unsafe { app.as_ref() };
        let palette = Palette::for_cached_theme(app.theme, app.settings.theme);
        Some((app.editor.clone()?, app.settings.clone(), palette))
    }) else {
        return;
    };
    let _ = editor.set_line_numbers(settings.line_numbers);
    let _ = editor.apply_view_settings(
        &settings.font_face,
        settings.font_size,
        settings.tab_width,
        settings.word_wrap,
    );
    let _ =
        editor.set_line_number_colors(palette.line_number_foreground, palette.editor_background);
    crate::window::preview_host::refresh_appearance(hwnd);
}

/// Runs only inside `WM_FASTPAD_BUILD_CHROME`: the first system theme query, the status model,
/// and a repaint that makes any queued notifications visible.
fn build_chrome(hwnd: HWND) {
    let theme = crate::platform::theme::SystemTheme::detect();
    if let Some(mut app) = unsafe { app_ptr(hwnd) } {
        let app = unsafe { app.as_mut() };
        app.theme = Some(theme);
        app.status = Some(crate::window::status::StatusModel::new(theme));
    }
    apply_theme(hwnd);
    layout_editor_and_find_bar(hwnd);
    unsafe {
        windows_sys::Win32::UI::Shell::DragAcceptFiles(hwnd, 1);
        InvalidateRect(hwnd, std::ptr::null(), 1);
    }
    crate::window::library_host::accept_editor_file_drops(hwnd);
}

/// Re-queries the system theme after chrome exists and restyles the editor only on a real change.
fn refresh_theme(hwnd: HWND) {
    let changed = unsafe { app_ptr(hwnd) }.is_some_and(|mut app| {
        let app = unsafe { app.as_mut() };
        if app.theme.is_none() {
            return false;
        }
        let theme = crate::platform::theme::SystemTheme::detect();
        if app.theme == Some(theme) {
            return false;
        }
        app.theme = Some(theme);
        if let Some(status) = app.status.as_mut() {
            status.theme = theme;
        }
        true
    });
    if changed {
        apply_theme(hwnd);
    }
}

/// Before chrome exists there is no cached theme, so system-following preferences fall back to the
/// one-shot registry read `apply_language` has always used; fixed themes skip it.
pub(crate) fn effective_theme(hwnd: HWND) -> crate::platform::theme::Theme {
    let (theme, preference) = unsafe { app_ptr(hwnd) }
        .map(|app| {
            let app = unsafe { app.as_ref() };
            (app.theme, app.settings.theme)
        })
        .unwrap_or((None, crate::config::ThemePreference::System));
    match theme {
        Some(theme) => theme.effective_theme(preference),
        None => crate::platform::theme::Theme::resolve(
            preference,
            preference.follows_system() && crate::platform::theme::system_uses_dark_mode(),
        ),
    }
}

fn apply_theme(hwnd: HWND) {
    let Some((editor, language, palette, frame_change)) =
        (unsafe { app_ptr(hwnd) }).and_then(|mut app| {
            let app = unsafe { app.as_mut() };
            let editor = app.editor.clone()?;
            let palette = Palette::for_cached_theme(app.theme, app.settings.theme);
            let frame_change = app.dark_frame_applied != palette.dark_frame;
            app.dark_frame_applied = palette.dark_frame;
            if let Some(bar) = app.find_bar.as_mut() {
                bar.set_colors(palette);
            }
            if let Some(name_box) = app.name_box.as_mut() {
                name_box.set_colors(palette);
            }
            if let Some(command_palette) = app.command_palette.as_mut() {
                command_palette.set_colors(palette);
            }
            let language = app
                .tabs
                .active()
                .map_or(crate::document::Language::PlainText, |document| {
                    document.language
                });
            Some((editor, language, palette, frame_change))
        })
    else {
        return;
    };
    let _ = editor.set_base_colors(palette.editor_foreground, palette.editor_background);
    let _ =
        editor.set_line_number_colors(palette.line_number_foreground, palette.editor_background);
    let _ = editor.set_chrome_colors(
        palette.selection_background,
        palette.inactive_selection_background,
        palette.caret_line_background,
    );
    let _ = editor.set_selection_text_colors(palette.selection_foreground);
    if let Some(app) = unsafe { app_ptr(hwnd) } {
        let app = unsafe { app.as_ref() };
        if let Some(bar) = app.find_bar.as_ref() {
            bar.invalidate();
        }
        if let Some(name_box) = app.name_box.as_ref() {
            name_box.invalidate();
        }
        if let Some(command_palette) = app.command_palette.as_ref() {
            command_palette.invalidate();
        }
    }
    if frame_change {
        crate::window::titlebar::apply_frame_theme(hwnd, editor.hwnd(), palette.dark_frame);
    }
    if language != crate::document::Language::PlainText {
        apply_language(hwnd, language);
    }
    crate::window::preview_host::refresh_appearance(hwnd);
    crate::window::side_panel::refresh(hwnd);
}

/// Copies what a title-strip paint needs out of App, creating the per-DPI fonts on first use.
/// Before chrome is built the palette is the neutral compiled one (no theme queries).
fn title_chrome(hwnd: HWND) -> (Palette, TitleFontHandles, PointerState) {
    let dpi = unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForWindow(hwnd) }.max(96);
    unsafe { app_ptr(hwnd) }
        .map(|mut app| {
            let app = unsafe { app.as_mut() };
            if app
                .title_fonts
                .as_ref()
                .is_none_or(|fonts| fonts.dpi() != dpi)
            {
                app.title_fonts = Some(crate::window::titlebar::TitleFonts::create(dpi));
            }
            (
                Palette::for_cached_theme(app.theme, app.settings.theme),
                app.title_fonts
                    .as_ref()
                    .map(crate::window::titlebar::TitleFonts::handles)
                    .unwrap_or_default(),
                app.title_pointer,
            )
        })
        .unwrap_or_else(|| {
            (
                Palette::neutral(),
                TitleFontHandles::default(),
                PointerState::default(),
            )
        })
}

/// Whether the screen point in a non-client mouse message's `lparam` is over the sidebar.
fn over_sidebar(hwnd: HWND, lparam: LPARAM) -> bool {
    let mut point = windows_sys::Win32::Foundation::POINT {
        x: (lparam as u32 & 0xffff) as u16 as i16 as i32,
        y: ((lparam as u32 >> 16) & 0xffff) as u16 as i16 as i32,
    };
    unsafe {
        windows_sys::Win32::Graphics::Gdi::ScreenToClient(hwnd, &mut point);
    }
    point.x < crate::window::side_panel::left_edge(hwnd)
}

fn client_title_target(hwnd: HWND, lparam: LPARAM) -> Option<HitTarget> {
    let point = crate::window::titlebar::Point::new(
        (lparam as u32 & 0xffff) as u16 as i16 as i32,
        ((lparam as u32 >> 16) & 0xffff) as u16 as i16 as i32,
    );
    Some(title_layout(hwnd).hit_test(point))
}

fn update_title_pointer(hwnd: HWND, update: impl FnOnce(PointerState) -> PointerState) {
    let changed = unsafe { app_ptr(hwnd) }.is_some_and(|mut app| {
        let app = unsafe { app.as_mut() };
        let next = update(app.title_pointer);
        let changed = next != app.title_pointer;
        app.title_pointer = next;
        changed
    });
    if changed {
        crate::window::titlebar::invalidate_strip(hwnd);
    }
}

fn run_caption_button(hwnd: HWND, target: HitTarget) {
    let command = match target {
        HitTarget::Minimize => SC_MINIMIZE,
        HitTarget::Maximize if unsafe { IsZoomed(hwnd) } != 0 => SC_RESTORE,
        HitTarget::Maximize => SC_MAXIMIZE,
        HitTarget::Close => SC_CLOSE,
        _ => return,
    };
    unsafe {
        SendMessageW(hwnd, WM_SYSCOMMAND, command as usize, 0);
    }
}

/// The pending-notification text shown on the bottom bar, which exists only after
/// `WM_FASTPAD_BUILD_CHROME`.
fn current_status_text(hwnd: HWND) -> Option<String> {
    let app = unsafe { app_ptr(hwnd) }?;
    let app = unsafe { app.as_ref() };
    app.status.as_ref()?;
    crate::window::status::status_text(&app.notifications)
}

/// Everything the bottom bar paints, or `None` before `WM_FASTPAD_BUILD_CHROME` builds it.
fn current_status_bar(hwnd: HWND) -> Option<crate::window::status::StatusBarText> {
    let app = unsafe { app_ptr(hwnd) }?;
    let app = unsafe { app.as_ref() };
    app.status.as_ref()?;
    let active = app.tabs.active().and_then(|document| {
        Some(crate::window::status::ActiveDocumentStatus {
            caret: app.editor.as_ref()?.caret_status().ok()?,
            language: document.language,
            encoding: document.encoding,
        })
    });
    let mut bar = crate::window::status::status_bar_text(&app.notifications, active);
    if app.notifications.pending().is_empty()
        && let Some(hint) = app.preview.status_hint()
    {
        bar.left = hint;
    }
    Some(bar)
}

fn status_bar_height(hwnd: HWND) -> i32 {
    let built =
        unsafe { app_ptr(hwnd) }.is_some_and(|app| unsafe { app.as_ref() }.status.is_some());
    if !built {
        return 0;
    }
    crate::window::status::status_height(unsafe {
        windows_sys::Win32::UI::HiDpi::GetDpiForWindow(hwnd)
    })
}

fn status_bar_rect(hwnd: HWND) -> Option<RECT> {
    let height = status_bar_height(hwnd);
    if height == 0 {
        return None;
    }
    let mut rect = RECT::default();
    unsafe {
        GetClientRect(hwnd, &mut rect);
    }
    rect.top = (rect.bottom - height).max(rect.top);
    Some(rect)
}

/// Clicking the bar dismisses notifications only while one is showing.
fn notice_contains(hwnd: HWND, y: i32) -> bool {
    current_status_text(hwnd).is_some() && status_bar_rect(hwnd).is_some_and(|rect| y >= rect.top)
}

/// Repaints just the bottom bar, for caret, selection and language changes.
pub(crate) fn invalidate_status_bar(hwnd: HWND) {
    if let Some(rect) = status_bar_rect(hwnd) {
        unsafe {
            InvalidateRect(hwnd, &rect, 0);
        }
    }
}

fn dismiss_notifications(hwnd: HWND) {
    if let Some(mut app) = unsafe { app_ptr(hwnd) } {
        unsafe { app.as_mut() }.notifications.dismiss_all();
    }
    layout_editor_and_find_bar(hwnd);
    unsafe {
        InvalidateRect(hwnd, std::ptr::null(), 1);
    }
}

/// Validates the active document's current text as JSON and reports the outcome. Read-only: never
/// touches the editor's text, selection, or undo stack either way.
fn validate_active_json(hwnd: HWND) {
    let Some(editor) =
        (unsafe { app_ptr(hwnd) }).and_then(|app| unsafe { app.as_ref() }.editor.clone())
    else {
        return;
    };
    let Ok(text) = editor.text() else {
        return;
    };
    match crate::languages::validate_json(&text) {
        Ok(()) => push_notice(hwnd, "This document contains valid JSON.".to_owned()),
        Err(issue) => push_notice(hwnd, json_issue_message(&issue)),
    }
}

/// Formats the active document's full JSON text in place. Reads the current text and selection,
/// formats completely in memory first, and only on success mutates the editor: one full-buffer
/// `replace_target` bracketed by `begin_undo_action`/`end_undo_action` (Task 12's
/// `Editor::replace_all` precedent), so a single Undo restores the exact original bytes. The
/// selection is restored afterward, clamped to the (likely different) new length and then snapped
/// down to the nearest UTF-8 character boundary (`floor_char_boundary`): the pre-format byte
/// offsets have no guaranteed relationship to character boundaries in the reformatted text (JSON
/// string values keep their literal, possibly multi-byte, UTF-8 content), so clamping alone is not
/// enough to avoid handing Scintilla a mid-character position. Invalid JSON never starts an undo
/// action and leaves the document's bytes completely unchanged: the failure is detected before any
/// editor mutation is attempted.
fn format_active_json(hwnd: HWND) {
    let Some(editor) =
        (unsafe { app_ptr(hwnd) }).and_then(|app| unsafe { app.as_ref() }.editor.clone())
    else {
        return;
    };
    let Ok(text) = editor.text() else {
        return;
    };
    let selection = editor.selection().unwrap_or(0..0);
    match crate::languages::format_json(&text) {
        Ok(formatted) => {
            editor.begin_undo_action();
            let result = editor.replace_target(0..text.len(), &formatted);
            editor.end_undo_action();
            if result.is_err() {
                return;
            }
            let new_length = formatted.len();
            let start = floor_char_boundary(&formatted, selection.start.min(new_length));
            let end = floor_char_boundary(&formatted, selection.end.min(new_length));
            let _ = editor.set_selection(start..end);
        }
        Err(issue) => push_notice(hwnd, json_issue_message(&issue)),
    }
}

/// The largest UTF-8 character boundary in `text` at or before `position`. `position` may be
/// `text.len()` (a valid boundary, the end of the string) but must not exceed it. Used to snap a
/// byte offset carried over from a *different* string (the pre-format text) into a valid position
/// in `text` (the post-format text): after formatting, an old offset has no guaranteed
/// relationship to character boundaries in the reformatted bytes — `serde_json` passes multi-byte
/// UTF-8 through unescaped, so it can coincidentally land mid-character. `0` is always a valid
/// boundary, so this loop always terminates.
fn floor_char_boundary(text: &str, mut position: usize) -> usize {
    while !text.is_char_boundary(position) {
        position -= 1;
    }
    position
}

fn json_issue_message(issue: &crate::languages::JsonIssue) -> String {
    if issue.line == 0 && issue.column == 0 {
        format!("FastPad could not process this JSON: {}", issue.message)
    } else {
        format!(
            "This document is not valid JSON (line {}, column {}): {}",
            issue.line, issue.column, issue.message
        )
    }
}

fn handle_open_request(hwnd: HWND) -> LRESULT {
    let request = unsafe { app_ptr(hwnd) }.and_then(|mut app| {
        let app = unsafe { app.as_mut() };
        if app.launch_open_completed {
            return None;
        }
        app.launch_open_completed = true;
        Some(app.launch.request.clone())
    });
    let Some(request) = request else {
        return 0;
    };
    match request {
        crate::launch::LaunchRequest::Open(path) => {
            let path = std::path::Path::new(&path);
            if path.is_dir() {
                // OPEN_LIBRARY already opened it as the folder.
                if !notes_mode_enabled(hwnd) {
                    push_notice(
                        hwnd,
                        format!(
                            "{} is a folder. Turn on notes mode to open it as a notebook.",
                            path.display()
                        ),
                    );
                }
                unsafe {
                    let _ = record_milestone(hwnd, Milestone::FileLoaded);
                }
            } else {
                match App::open_path(hwnd, path) {
                    Ok(()) => return 0,
                    Err(error) => {
                        report_open_failure(hwnd, path, &error);
                        // The requested-file unit is finished either way; the milestone stays honest.
                        unsafe {
                            let _ = record_milestone(hwnd, Milestone::FileLoaded);
                        }
                    }
                }
            }
        }
        crate::launch::LaunchRequest::New => unsafe {
            let _ = record_milestone(hwnd, Milestone::FileLoaded);
        },
    }
    if unsafe { window_identity(hwnd) }.is_some_and(|identity| identity.is_live_for(hwnd)) {
        unsafe {
            PostMessageW(hwnd, crate::window::WM_FASTPAD_APPLY_LANGUAGE, 0, 0);
        }
    }
    0
}

fn notes_mode_enabled(hwnd: HWND) -> bool {
    unsafe { app_ptr(hwnd) }.is_some_and(|app| unsafe { app.as_ref() }.settings.notes_mode)
}

pub(crate) fn open_path(hwnd: HWND, path: &std::path::Path) -> Result<()> {
    open_path_placed(hwnd, path, false)
}

fn open_path_placed(hwnd: HWND, path: &std::path::Path, preview: bool) -> Result<()> {
    let identity = unsafe { window_identity(hwnd) }.ok_or(crate::FastPadError::Invariant(
        "main window app state was not available",
    ))?;
    if file_population_active(hwnd) {
        return Err(crate::FastPadError::Invariant(
            "file population is already active",
        ));
    }
    let existing = unsafe { app_ptr(hwnd) }.and_then(|app| {
        let app = unsafe { app.as_ref() };
        app.tabs
            .find_path(path)
            .map(|id| (id, app.tabs.view().snapshot().revision))
    });
    if let Some((id, revision)) = existing {
        return if activate_document(hwnd, id, revision) {
            unsafe {
                PostMessageW(hwnd, crate::window::WM_FASTPAD_APPLY_LANGUAGE, 0, 0);
            }
            Ok(())
        } else {
            Err(crate::FastPadError::Invariant(
                "existing file could not be activated",
            ))
        };
    }

    // The tab being left saves first; a failed or paused autosave leaves it dirty and open.
    crate::window::library_host::autosave_active(hwnd);
    if !identity.is_live_for(hwnd) {
        return Err(crate::FastPadError::Invariant(
            "main window was destroyed during file open",
        ));
    }
    // Read before the load: a change that lands during it then still pauses the next autosave.
    let stamp = crate::library::disk_stamp(path);
    // All fallible disk/decode/text validation occurs before touching active state.
    let loaded = crate::file::loader::load(path)?;
    // A NUL byte cannot round-trip through Scintilla's UTF-8 buffer: the file is unsupported.
    std::ffi::CString::new(loaded.text.as_str())
        .map_err(|_| crate::FastPadError::UnsupportedEncoding)?;
    let (editor, candidate_ids, replace_preview) = {
        let mut app = unsafe { app_ptr(hwnd) }.ok_or(crate::FastPadError::Invariant(
            "main window app state was not available",
        ))?;
        let app = unsafe { app.as_mut() };
        let editor = app
            .editor
            .clone()
            .ok_or(crate::FastPadError::Invariant("editor was not initialized"))?;
        // A preview goes where the preview tab is. Without one it is placed like any new tab,
        // reusing an empty start tab.
        let replace_preview = preview && app.tabs.preview_id().is_some();
        let candidate_ids = app
            .tabs
            .active()
            .filter(|active| !replace_preview && !active.dirty && active.path.is_none())
            .map(|active| (active.id, active.recovery_id));
        (editor, candidate_ids, replace_preview)
    };
    // With no tab open this is the hidden placeholder document.
    let previous = editor.current_document()?;
    let reused_ids = match candidate_ids {
        Some(ids) if editor.text()?.is_empty() => Some(ids),
        _ => None,
    };
    let reuse = reused_ids.is_some();
    if !identity.is_live_for(hwnd) {
        return Err(crate::FastPadError::Invariant(
            "main window was destroyed during file open",
        ));
    }
    let (id, recovery_id) = if let Some(ids) = reused_ids {
        ids
    } else {
        let mut app = unsafe { app_ptr(hwnd) }.ok_or(crate::FastPadError::Invariant(
            "main window app state was not available",
        ))?;
        unsafe { app.as_mut() }.allocate_document_identity()
    };
    let mut document = Document::untitled(id, recovery_id, editor.create_document()?);
    document.path = Some(loaded.path);
    document.encoding = loaded.encoding;
    document.preview = preview;
    if !identity.is_live_for(hwnd) {
        return Err(crate::FastPadError::Invariant(
            "main window was destroyed during file open",
        ));
    }
    unsafe { app_ptr(hwnd).unwrap().as_mut() }.populating_file = true;
    let result = editor
        .use_document(&document.handle)
        .and_then(|_| editor.populate_clean(&loaded.text));
    if result.is_err() && identity.is_live_for(hwnd) {
        let _ = editor.use_document(&previous);
    }
    if !identity.is_live_for(hwnd) {
        return Err(crate::FastPadError::Invariant(
            "main window was destroyed during file population",
        ));
    }
    let (commit, retired) = {
        let mut app = unsafe { app_ptr(hwnd) }.ok_or(crate::FastPadError::Invariant(
            "main window app state was not available",
        ))?;
        let app = unsafe { app.as_mut() };
        app.populating_file = false;
        result?;
        if replace_preview {
            // The old preview is never dirty, so dropping it loses nothing.
            (Ok(()), app.tabs.replace_preview(document))
        } else if reuse {
            let retired = app.tabs.replace_active_untitled(document);
            let commit = if retired.is_some() {
                Ok(())
            } else {
                Err(crate::FastPadError::Invariant(
                    "the reused tab closed during file open",
                ))
            };
            (commit, retired)
        } else {
            (
                app.tabs
                    .push(document)
                    .map_err(|_| crate::FastPadError::Invariant("duplicate document path")),
                None,
            )
        }
    };
    drop(retired);
    if commit.is_err() {
        let _ = editor.use_document(&previous);
    }
    commit?;
    unsafe {
        let _ = record_milestone(hwnd, Milestone::FileLoaded);
        PostMessageW(hwnd, crate::window::WM_FASTPAD_APPLY_LANGUAGE, 0, 0);
    }
    // Population suppressed SCN_MODIFIED, and a reused tab keeps its document id.
    crate::window::preview_host::document_reloaded(hwnd);
    refresh_tabs(hwnd);
    crate::window::library_host::document_loaded(hwnd, stamp);
    Ok(())
}

/// How `open_note` places a note that is not open yet.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OpenMode {
    /// In the preview tab, replaced in place by the next preview.
    Preview,
    /// In a normal tab. An open preview of the same note becomes normal.
    Permanent,
}

/// Opens `path` from the sidebar (spec §6.4). An already-open note is switched to, and a
/// `Permanent` open keeps it. Otherwise `Preview` replaces the preview tab in place and
/// `Permanent` opens a normal tab. `focus_editor` then moves the keyboard focus to the editor.
pub(crate) fn open_note(
    hwnd: HWND,
    path: &std::path::Path,
    mode: OpenMode,
    focus_editor: bool,
) -> Result<()> {
    let open = unsafe { app_ptr(hwnd) }
        .and_then(|app| unsafe { app.as_ref() }.tabs.find_stored_path(path));
    match open {
        Some(id) => {
            if !activate_document_by_id(hwnd, id) {
                return Err(crate::FastPadError::Invariant(
                    "the note's tab could not be activated",
                ));
            }
            unsafe {
                PostMessageW(hwnd, crate::window::WM_FASTPAD_APPLY_LANGUAGE, 0, 0);
            }
            if mode == OpenMode::Permanent {
                promote_tab(hwnd, id);
            }
        }
        None => open_path_placed(hwnd, path, mode == OpenMode::Preview)?,
    }
    if focus_editor {
        focus_content(hwnd);
    }
    Ok(())
}

/// Makes `id` a normal tab and repaints its label.
fn promote_tab(hwnd: HWND, id: DocumentId) {
    let promoted =
        unsafe { app_ptr(hwnd) }.is_some_and(|mut app| unsafe { app.as_mut() }.tabs.promote(id));
    if promoted {
        invalidate_title_strip(hwnd);
    }
}

/// Whether this click on tab `index` is the second of a double-click.
fn tab_double_click(hwnd: HWND, index: usize) -> bool {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime;
    use windows_sys::Win32::UI::WindowsAndMessaging::GetMessageTime;
    let now = unsafe { GetMessageTime() } as u32;
    let limit = unsafe { GetDoubleClickTime() };
    let Some(mut app) = (unsafe { app_ptr(hwnd) }) else {
        return false;
    };
    let app = unsafe { app.as_mut() };
    let Some(id) = app.tabs.view().snapshot().tabs.get(index).map(|tab| tab.id) else {
        return false;
    };
    let double = app
        .last_tab_click
        .is_some_and(|(last, at)| last == id && now.wrapping_sub(at) <= limit);
    app.last_tab_click = if double { None } else { Some((id, now)) };
    double
}

pub(crate) fn create_new_document(hwnd: HWND) -> Result<()> {
    let identity = unsafe { window_identity(hwnd) }.ok_or(crate::FastPadError::Invariant(
        "main window app state was not available",
    ))?;
    crate::window::library_host::autosave_active(hwnd);
    if !identity.is_live_for(hwnd) {
        return Err(crate::FastPadError::Invariant(
            "main window was destroyed while creating a document",
        ));
    }
    let (editor, id, recovery_id) = {
        let Some(mut app) = (unsafe { app_ptr(hwnd) }) else {
            return Err(crate::FastPadError::Invariant(
                "main window app state was not available",
            ));
        };
        let app = unsafe { app.as_mut() };
        let editor = app
            .editor
            .clone()
            .ok_or(crate::FastPadError::Invariant("editor was not initialized"))?;
        let (id, recovery_id) = app.allocate_document_identity();
        (editor, id, recovery_id)
    };

    let document = Document::untitled(id, recovery_id, editor.create_document()?);
    editor.use_document(&document.handle)?;
    if !identity.is_live_for(hwnd) {
        return Err(crate::FastPadError::Invariant(
            "main window was destroyed while creating a document",
        ));
    }
    let Some(mut app) = (unsafe { app_ptr(hwnd) }) else {
        return Err(crate::FastPadError::Invariant(
            "main window app state was not available",
        ));
    };
    let app = unsafe { app.as_mut() };
    app.tabs
        .push(document)
        .map_err(|_| crate::FastPadError::Invariant("duplicate document path"))?;
    refresh_tabs(hwnd);
    Ok(())
}

fn activate_tab(hwnd: HWND, index: usize) {
    let target = unsafe { app_ptr(hwnd) }.and_then(|app| {
        let view = unsafe { app.as_ref() }.tabs.view().snapshot();
        view.tabs.get(index).map(|tab| (tab.id, view.revision))
    });
    if let Some((id, revision)) = target {
        let _ = activate_document(hwnd, id, revision);
    }
}

/// Activates the tab after (or before) the active one, wrapping around the ends of the strip.
fn cycle_tab(hwnd: HWND, forward: bool) {
    let Some(active) =
        (unsafe { app_ptr(hwnd) }).map(|app| unsafe { app.as_ref() }.tabs.active_index())
    else {
        return;
    };
    let count = tab_count(hwnd);
    if count < 2 {
        return;
    }
    let target = if forward {
        (active + 1) % count
    } else {
        (active + count - 1) % count
    };
    activate_tab(hwnd, target);
}

fn activate_document(hwnd: HWND, id: DocumentId, revision: u64) -> bool {
    if file_population_active(hwnd) {
        return false;
    }
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return false;
    };
    let leaving = unsafe { app_ptr(hwnd) }.and_then(|app| {
        let app = unsafe { app.as_ref() };
        (app.tabs.view().snapshot().revision == revision)
            .then(|| app.tabs.active().is_some_and(|active| active.id != id))
    });
    let Some(leaving) = leaving else {
        return false;
    };
    // Saving the tab being left bumps the view revision itself, so the caller's revision is
    // checked before it; afterwards `activate` still refuses an `id` that has gone.
    if leaving {
        crate::window::library_host::autosave_active(hwnd);
        if !identity.is_live_for(hwnd) {
            return false;
        }
    }
    let target = unsafe { app_ptr(hwnd) }.and_then(|mut app| {
        let app = unsafe { app.as_mut() };
        let editor = app.editor.clone()?;
        app.tabs.activate(id).ok()?;
        Some((editor, app.tabs.active_handle()?.clone()))
    });
    let Some((editor, handle)) = target else {
        return false;
    };
    if editor.use_document(&handle).is_err() || !identity.is_live_for(hwnd) {
        return false;
    }
    refresh_tabs(hwnd);
    true
}

fn handle_accessible_select(hwnd: HWND, lparam: LPARAM) -> LRESULT {
    if lparam == 0 {
        return 0;
    }
    let request = unsafe { *(lparam as *const AccessibleSelectRequest) };
    isize::from(activate_document(
        hwnd,
        request.document_id,
        request.revision,
    ))
}

fn close_active_document(hwnd: HWND) {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    // A saved note is clean now and closes without a prompt; paused or failed ones still ask.
    crate::window::library_host::autosave_active(hwnd);
    if !identity.is_live_for(hwnd) {
        return;
    }
    let snapshot = unsafe { app_ptr(hwnd) }.and_then(|app| {
        let app = unsafe { app.as_ref() };
        let review = app.tabs.active_close_review()?;
        let document = app.tabs.document(review.id)?;
        Some((review, document.dirty, document.title(), app.editor.clone()))
    });
    let Some((review, dirty, title, Some(editor))) = snapshot else {
        return;
    };
    let decision = if dirty {
        prompt_close_decision(hwnd, &title)
    } else {
        CloseDecision::Discard
    };
    if decision == CloseDecision::Cancel || !identity.is_live_for(hwnd) {
        return;
    }
    // Saving clears the dirty flag, which advances the generation the prompt reviewed.
    let review = if decision == CloseDecision::Save {
        if !save_reviewed_document(hwnd, review.id) {
            return;
        }
        match unsafe { app_ptr(hwnd) }
            .and_then(|app| unsafe { app.as_ref() }.tabs.active_close_review())
        {
            Some(saved) if saved.id == review.id => saved,
            _ => return,
        }
    } else {
        review
    };
    close_reviewed_document(hwnd, &identity, &editor, review, decision);
}

/// Closes `id` without asking, discarding any unsaved edits, e.g. once its file is deleted.
pub(super) fn close_document_without_prompt(hwnd: HWND, id: DocumentId) {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    if !activate_document_by_id(hwnd, id) || !identity.is_live_for(hwnd) {
        return;
    }
    let reviewed = unsafe { app_ptr(hwnd) }.and_then(|app| {
        let app = unsafe { app.as_ref() };
        Some((app.tabs.active_close_review()?, app.editor.clone()?))
    });
    let Some((review, editor)) = reviewed else {
        return;
    };
    if review.id == id {
        close_reviewed_document(hwnd, &identity, &editor, review, CloseDecision::Discard);
    }
}

/// The close itself, once `decision` is settled: closes the reviewed tab, removes its recovery
/// snapshots and shows whichever tab takes its place.
fn close_reviewed_document(
    hwnd: HWND,
    identity: &WindowIdentity,
    editor: &Editor,
    review: crate::window::tabs::CloseReview,
    decision: CloseDecision,
) {
    let switched = unsafe { app_ptr(hwnd) }.and_then(|mut app| {
        let app = unsafe { app.as_mut() };
        let closed = app.tabs.close_reviewed(review, decision).ok()?;
        let snapshots = app
            .recovery_root
            .as_deref()
            .map(|root| {
                crate::recovery::snapshots_removed_on_close(
                    root,
                    &closed,
                    decision == CloseDecision::Discard,
                )
            })
            .unwrap_or_default();
        Some((closed, app.tabs.active_handle().cloned(), snapshots))
    });
    let Some((closed, active, snapshots)) = switched else {
        return;
    };
    // The view keeps its own reference to whatever it shows, so the last closed document is
    // swapped for an empty placeholder rather than lingering in the hidden editor.
    let active = active.or_else(|| editor.create_document().ok());
    if let Some(active) = active {
        let _ = editor.use_document(&active);
    }
    drop(closed);
    crate::recovery::remove_snapshot_files(&snapshots);
    if identity.is_live_for(hwnd) {
        refresh_tabs(hwnd);
    }
}

/// Closes tabs one at a time, reviewing each dirty one, until none remain or a close is refused.
fn close_all_documents(hwnd: HWND) {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    loop {
        let before = tab_count(hwnd);
        if before == 0 {
            return;
        }
        close_active_document(hwnd);
        if !identity.is_live_for(hwnd) || tab_count(hwnd) >= before {
            return;
        }
    }
}

/// Activates `id` (the prompt's modal loop can have activated another tab) and saves it. Reports
/// success only when that same document is the active, no-longer-dirty one afterwards.
fn save_reviewed_document(hwnd: HWND, id: DocumentId) -> bool {
    if !activate_document_by_id(hwnd, id) || !save_active_document(hwnd) {
        return false;
    }
    unsafe { app_ptr(hwnd) }.is_some_and(|app| {
        let app = unsafe { app.as_ref() };
        app.tabs.active().is_some_and(|active| active.id == id)
            && app
                .tabs
                .document(id)
                .is_some_and(|document| !document.dirty)
    })
}

/// Makes `id` the active document, or reports false when it no longer exists.
pub(super) fn activate_document_by_id(hwnd: HWND, id: DocumentId) -> bool {
    let target = unsafe { app_ptr(hwnd) }.and_then(|app| {
        let app = unsafe { app.as_ref() };
        if app.tabs.active().is_some_and(|active| active.id == id) {
            return Some(None);
        }
        app.tabs.document(id)?;
        Some(Some(app.tabs.view().snapshot().revision))
    });
    match target {
        Some(None) => true,
        Some(Some(revision)) => activate_document(hwnd, id, revision),
        None => false,
    }
}

pub(super) fn save_active_document(hwnd: HWND) -> bool {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return false;
    };
    let has_path = unsafe { app_ptr(hwnd) }
        .and_then(|app| Some(unsafe { app.as_ref() }.tabs.active()?.path.is_some()));
    match has_path {
        Some(true) => complete_save(hwnd, &identity, None),
        Some(false) => save_active_document_as(hwnd),
        None => false,
    }
}

pub(super) fn save_active_document_as(hwnd: HWND) -> bool {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return false;
    };
    let Some((target, named, notes_mode)) = (unsafe { app_ptr(hwnd) }).and_then(|app| {
        let app = unsafe { app.as_ref() };
        let document = app.tabs.active()?;
        Some((
            document.id,
            document
                .path
                .as_deref()
                .and_then(std::path::Path::file_name)
                .map(|name| name.to_string_lossy().into_owned()),
            app.settings.notes_mode,
        ))
    }) else {
        return false;
    };
    // Only an untitled tab in notes mode starts in the notes folder under its label's name; every
    // other Save As keeps the dialog's usual suggestion and starting folder.
    let (suggested, folder) = match named {
        Some(name) => (name, None),
        // With no notebook open this is notes mode off's Save As.
        None if notes_mode && crate::window::library_host::folder(hwnd).is_some() => (
            crate::window::library_host::suggested_file_name(hwnd),
            crate::window::library_host::first_save_folder(hwnd),
        ),
        None => ("Untitled.txt".to_owned(), None),
    };
    // Modal Show reenters the window procedure. Only an owned identity crosses it.
    let selection = crate::window::modal::choose_save_path(hwnd, &suggested, folder.as_deref());
    if !identity.is_live_for(hwnd) {
        return false;
    }
    let path = match selection {
        Ok(Some(path)) => path,
        // Cancelling the dialog is not an error and says nothing.
        Ok(None) => return false,
        Err(error) => {
            push_notice(
                hwnd,
                format!("FastPad could not open the Save As dialog: {error}"),
            );
            return false;
        }
    };
    // The dialog's modal loop can have activated another tab; save the document that was chosen.
    if !activate_document_by_id(hwnd, target) {
        return false;
    }
    complete_save(hwnd, &identity, Some(path))
}

/// Test-only entry point that drives Save As with an explicit path, bypassing the native dialog.
/// The real dialog interaction is covered by `select_save_file`'s tests; this exists because the
/// shell's own "Confirm Save As" collision handling for an existing target could not be driven
/// reliably through synthetic window messages on this host, so the collision-rejection tail below
/// (identical production code `save_active_document_as` reaches after a real dialog selection) is
/// exercised directly instead, mirroring `open_path`'s existing non-dialog test entry point.
#[cfg(test)]
#[allow(
    dead_code,
    reason = "consumed by the source-linked save_file integration target"
)]
pub(crate) fn save_path_as(hwnd: HWND, path: &std::path::Path) {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    complete_save(hwnd, &identity, Some(path.to_path_buf()));
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SaveOutcome {
    Saved,
    Failed,
    /// A first save found a file already at the chosen path and left it alone.
    NameTaken,
}

/// Shared tail of plain Save and Save As; see `save_active_to`.
pub(super) fn complete_save(
    hwnd: HWND,
    identity: &WindowIdentity,
    new_path: Option<std::path::PathBuf>,
) -> bool {
    save_active_to(hwnd, identity, new_path, false, true) == SaveOutcome::Saved
}

/// Plain Save for autosave: a failure pushes no generic notice, because the caller names the note.
pub(super) fn complete_autosave(hwnd: HWND, identity: &WindowIdentity) -> bool {
    save_active_to(hwnd, identity, None, false, false) == SaveOutcome::Saved
}

/// The first save of an untitled tab under a name picked in the name box. Never replaces a file:
/// one that appeared at `path` since the name was checked gives `NameTaken`, with no notice, the
/// tab still untitled and the file untouched.
pub(super) fn complete_first_save(
    hwnd: HWND,
    identity: &WindowIdentity,
    path: std::path::PathBuf,
) -> SaveOutcome {
    save_active_to(hwnd, identity, Some(path), true, true)
}

/// Shared tail of plain Save and Save As. `new_path` is `Some` only for Save As: the active
/// document's path is renamed (and checked against other open tabs' canonical paths) before the
/// write. Plain Save (`new_path: None`) writes to the document's existing path unchanged.
/// `create_new` refuses to replace an existing file (`SaveOutcome::NameTaken`). `report_failure`
/// pushes the generic "could not save" notice when the write fails.
///
/// For Save As, every failure after a successful rename (missing editor, a failed
/// `editor.text()` read, or a failed `save_atomic`) reverts the tab's path back to whatever it
/// held before this call: a failed write must never leave the tab claiming a path nothing was
/// actually written to, orphaning it from the path it was last genuinely saved at.
fn save_active_to(
    hwnd: HWND,
    identity: &WindowIdentity,
    new_path: Option<std::path::PathBuf>,
    create_new: bool,
    report_failure: bool,
) -> SaveOutcome {
    let is_save_as = new_path.is_some();
    let mut original_path: Option<std::path::PathBuf> = None;
    if let Some(path) = new_path {
        original_path = unsafe { app_ptr(hwnd) }
            .and_then(|app| unsafe { app.as_ref() }.tabs.active()?.path.clone());
        let outcome = unsafe { app_ptr(hwnd) }
            .map(|mut app| unsafe { app.as_mut() }.tabs.set_active_path(path));
        match outcome {
            Some(Ok(())) => {}
            Some(Err(_)) => {
                push_notice(
                    hwnd,
                    "This file is already open in another tab. Choose a different name.".to_owned(),
                );
                return SaveOutcome::Failed;
            }
            None => return SaveOutcome::Failed,
        }
    }
    if !identity.is_live_for(hwnd) {
        return SaveOutcome::Failed;
    }
    let Some((editor, path, encoding)) = (unsafe { app_ptr(hwnd) }).and_then(|app| {
        let app = unsafe { app.as_ref() };
        let editor = app.editor.clone()?;
        let document = app.tabs.active()?;
        Some((editor, document.path.clone()?, document.encoding))
    }) else {
        if is_save_as {
            revert_active_path(hwnd, original_path);
        }
        return SaveOutcome::Failed;
    };
    let Ok(text) = editor.text() else {
        if is_save_as {
            revert_active_path(hwnd, original_path);
        }
        return SaveOutcome::Failed;
    };
    let bytes = crate::file::encoding::encode(&text, encoding);
    let result = if create_new {
        crate::file::saver::save_atomic_new(&path, &bytes)
    } else {
        crate::file::saver::save_atomic(&path, &bytes)
    };
    if !identity.is_live_for(hwnd) {
        return SaveOutcome::Failed;
    }
    match result {
        Ok(()) => {
            remove_saved_document_snapshots(hwnd);
            editor.set_save_point();
            // A recovered tab undone to the empty save point gets no save-point notification.
            let cleaned = identity.is_live_for(hwnd)
                && unsafe { app_ptr(hwnd) }
                    .is_some_and(|mut app| unsafe { app.as_mut() }.tabs.set_active_dirty(false));
            if cleaned && !is_save_as {
                invalidate_title_strip(hwnd);
            }
            if is_save_as {
                unsafe {
                    PostMessageW(hwnd, crate::window::WM_FASTPAD_APPLY_LANGUAGE, 0, 0);
                }
                invalidate_title_strip(hwnd);
            }
            if identity.is_live_for(hwnd) {
                crate::window::library_host::document_saved(hwnd);
            }
            SaveOutcome::Saved
        }
        Err(error) => {
            if is_save_as {
                revert_active_path(hwnd, original_path);
            }
            if create_new && crate::file::saver::is_already_exists(&error) {
                return SaveOutcome::NameTaken;
            }
            if !report_failure {
                return SaveOutcome::Failed;
            }
            push_notice(
                hwnd,
                "FastPad could not save this file. The previous version on disk was not modified."
                    .to_owned(),
            );
            SaveOutcome::Failed
        }
    }
}

fn revert_active_path(hwnd: HWND, original_path: Option<std::path::PathBuf>) {
    if let Some(mut app) = unsafe { app_ptr(hwnd) } {
        unsafe { app.as_mut() }
            .tabs
            .revert_active_path(original_path);
    }
}

/// Returns the documents explicitly discarded, or `None` when the close was cancelled.
fn review_dirty_documents(hwnd: HWND) -> Option<Vec<DocumentId>> {
    let identity = unsafe { window_identity(hwnd) }?;
    let mut reviewed = Vec::<CloseReviewKey>::new();
    let mut discarded = Vec::<DocumentId>::new();
    loop {
        let pending = unsafe { app_ptr(hwnd) }.and_then(|app| {
            let app = unsafe { app.as_ref() };
            let review = app.tabs.next_dirty_review(&reviewed)?;
            let title = app.tabs.document(review.id)?.title();
            Some((review, title))
        });
        let Some((review, title)) = pending else {
            return Some(discarded);
        };
        let decision = prompt_close_decision(hwnd, &title);
        if decision == CloseDecision::Cancel || !identity.is_live_for(hwnd) {
            return None;
        }
        // A failed or cancelled save aborts the whole window close rather than losing the text.
        if decision == CloseDecision::Save {
            if !save_reviewed_document(hwnd, review.id) {
                return None;
            }
            continue;
        }
        let current = unsafe { app_ptr(hwnd) }
            .map(|app| unsafe { app.as_ref() }.tabs.dirty_review_is_current(review))
            .unwrap_or(false);
        if current {
            reviewed.push(review.key());
            discarded.retain(|id| *id != review.id);
            if decision == CloseDecision::Discard {
                discarded.push(review.id);
            }
        }
    }
}

fn start_recovery_timer(hwnd: HWND) {
    ensure_recovery_owner(hwnd);
    let Some(interval) = (unsafe { app_ptr(hwnd) })
        .map(|app| unsafe { app.as_ref() }.settings.recovery_interval_seconds)
    else {
        return;
    };
    unsafe {
        SetTimer(
            hwnd,
            crate::recovery::RECOVERY_TIMER_ID,
            crate::recovery::timer_period_ms(interval),
            None,
        );
    }
}

/// Holds the named mutex that tells other FastPad processes this one's snapshots are live, not
/// crash leftovers. Creating it again is a no-op once held.
fn ensure_recovery_owner(hwnd: HWND) {
    if let Some(mut app) = unsafe { app_ptr(hwnd) } {
        let app = unsafe { app.as_mut() };
        if app.recovery_owner.is_none() {
            app.recovery_owner = crate::recovery::create_owner_mutex(app.recovery_owner_id()).ok();
        }
    }
}

/// Resolves (once) and returns the Recovery directory; tests pre-seed `App::recovery_root`.
fn recovery_root(hwnd: HWND) -> Option<std::path::PathBuf> {
    let mut app = unsafe { app_ptr(hwnd) }?;
    let app = unsafe { app.as_mut() };
    if app.recovery_root.is_none() {
        app.recovery_root = crate::recovery::recovery_root().ok();
    }
    app.recovery_root.clone()
}

fn snapshot_when_idle(hwnd: HWND) {
    let mut info = LASTINPUTINFO {
        cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
        dwTime: 0,
    };
    if unsafe { GetLastInputInfo(&mut info) } == 0
        || !crate::recovery::input_idle(info.dwTime, unsafe { GetTickCount() })
    {
        return;
    }
    snapshot_next_document(hwnd);
}

/// Writes at most one dirty document whose generation has not been recorded yet.
fn snapshot_next_document(hwnd: HWND) {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    if file_population_active(hwnd) || crate::window::modal::modal_active(hwnd) {
        return;
    }
    let Some(root) = recovery_root(hwnd) else {
        return;
    };
    let job = unsafe { app_ptr(hwnd) }.and_then(|app| {
        let app = unsafe { app.as_ref() };
        let editor = app.editor.clone()?;
        let active = app.tabs.active()?;
        let document = crate::recovery::next_snapshot_document(
            app.tabs.documents(),
            app.last_snapshot_attempt,
        )?;
        let origin = document.recovery_origin.as_ref();
        Some(SnapshotJob {
            editor,
            id: document.id,
            generation: document.generation,
            recovery_id: document.recovery_id,
            original_path: document
                .path
                .clone()
                .or_else(|| origin.and_then(|origin| origin.original_path.clone())),
            encoding: document.encoding,
            source_snapshot: origin.map(|origin| origin.snapshot_path.clone()),
            inactive: (document.id != active.id)
                .then(|| (document.handle.clone(), active.handle.clone())),
        })
    });
    let Some(job) = job else {
        return;
    };
    // Recorded before the write so a document that keeps failing still yields the next tick.
    if let Some(mut app) = unsafe { app_ptr(hwnd) } {
        unsafe { app.as_mut() }.last_snapshot_attempt = Some(job.id);
    }
    let started = std::time::Instant::now();
    let text = match &job.inactive {
        None => job.editor.text(),
        Some((target, active)) => read_inactive_text(hwnd, &identity, &job.editor, target, active),
    };
    let Ok(text) = text else {
        return;
    };
    if !identity.is_live_for(hwnd) {
        return;
    }
    let snapshot =
        crate::recovery::Snapshot::new(job.recovery_id, job.original_path, job.encoding, text);
    let written = crate::recovery::write_snapshot(&root, &snapshot);
    drop(snapshot);
    let elapsed = started.elapsed();
    if let Some(mut app) = unsafe { app_ptr(hwnd) } {
        let app = unsafe { app.as_mut() };
        app.last_snapshot_duration = Some(elapsed);
        if written.is_ok() {
            app.tabs.record_recovery_generation(job.id, job.generation);
        }
    }
    // Once a recovered tab has its own snapshot, its source would only resurrect a stale duplicate.
    if let (Ok(written), Some(source)) = (written, job.source_snapshot)
        && written != source
    {
        crate::recovery::remove_snapshot_files(&[source]);
    }
}

struct SnapshotJob {
    editor: Editor,
    id: DocumentId,
    generation: u64,
    recovery_id: RecoveryId,
    original_path: Option<std::path::PathBuf>,
    encoding: crate::file::encoding::Encoding,
    source_snapshot: Option<std::path::PathBuf>,
    inactive: Option<(crate::editor::EditorDocument, crate::editor::EditorDocument)>,
}

/// Scintilla can only read the document shown in the view, so an inactive tab is swapped in and
/// out with notifications suppressed, restoring the visible selection and scroll position.
fn read_inactive_text(
    hwnd: HWND,
    identity: &WindowIdentity,
    editor: &Editor,
    target: &crate::editor::EditorDocument,
    active: &crate::editor::EditorDocument,
) -> Result<String> {
    use crate::editor::scintilla_constants::{SCI_GETFIRSTVISIBLELINE, SCI_SETFIRSTVISIBLELINE};
    let selection = editor.selection();
    let first_line = unsafe { SendMessageW(editor.hwnd(), SCI_GETFIRSTVISIBLELINE, 0, 0) };
    set_file_population(hwnd, true);
    let text = editor.use_document(target).and_then(|_| editor.text());
    let restored = if identity.is_live_for(hwnd) {
        editor.use_document(active)
    } else {
        Err(crate::FastPadError::Invariant(
            "main window was destroyed during a recovery snapshot",
        ))
    };
    if restored.is_ok() {
        if let Ok(selection) = selection {
            let _ = editor.set_selection(selection);
        }
        unsafe {
            SendMessageW(
                editor.hwnd(),
                SCI_SETFIRSTVISIBLELINE,
                first_line as usize,
                0,
            );
        }
    }
    if identity.is_live_for(hwnd) {
        set_file_population(hwnd, false);
    }
    restored?;
    text
}

fn set_file_population(hwnd: HWND, active: bool) {
    if let Some(mut app) = unsafe { app_ptr(hwnd) } {
        unsafe { app.as_mut() }.populating_file = active;
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RestoreStep {
    Continue,
    Done,
}

/// One `WM_FASTPAD_RESTORE_SESSION` pass. The first pass takes the manifest. Each pass reopens
/// at most one entry, and the pass that finds none left finishes the restore.
fn restore_session_step(hwnd: HWND) -> RestoreStep {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return RestoreStep::Done;
    };
    if file_population_active(hwnd) {
        return RestoreStep::Continue;
    }
    let started = unsafe { app_ptr(hwnd) }
        .is_some_and(|app| unsafe { app.as_ref() }.session_restore.is_some());
    if !started && !begin_session_restore(hwnd) {
        return RestoreStep::Done;
    }
    let entry = unsafe { app_ptr(hwnd) }.and_then(|app| {
        unsafe { app.as_ref() }
            .session_restore
            .as_ref()?
            .next_entry()
            .cloned()
    });
    let Some(entry) = entry else {
        finish_session_restore(hwnd);
        return RestoreStep::Done;
    };
    let restored = restore_session_entry(hwnd, &entry);
    if !identity.is_live_for(hwnd) {
        return RestoreStep::Done;
    }
    if let Some(mut app) = unsafe { app_ptr(hwnd) }
        && let Some(restore) = unsafe { app.as_mut() }.session_restore.as_mut()
    {
        restore.record(restored);
    }
    RestoreStep::Continue
}

/// Takes the manifest, deleting it so a crash from here on is recovery's alone, and remembers
/// the empty startup tab so it can be closed. False when there is nothing to restore.
fn begin_session_restore(hwnd: HWND) -> bool {
    let Some(path) = session_path(hwnd) else {
        // With the setting off this primary never restores the manifest, and its snapshots come
        // back through crash recovery instead. Left in place, it would reopen them a second time
        // once the setting is turned back on, and keep hiding them from other windows' recovery.
        if let Some(stale) = disabled_session_path(hwnd) {
            crate::session::remove(&stale);
        }
        return false;
    };
    let Some(session) = crate::session::read(&path) else {
        return false;
    };
    crate::session::remove(&path);
    if session.entries.is_empty() {
        return false;
    }
    let placeholder = empty_startup_tab(hwnd);
    // Restored snapshots are rewritten under this process's IDs, which only count as live while
    // it holds the owner mutex. `load_settings` has normally created it already.
    ensure_recovery_owner(hwnd);
    let Some(mut app) = (unsafe { app_ptr(hwnd) }) else {
        return false;
    };
    unsafe { app.as_mut() }.session_restore =
        Some(crate::session::SessionRestore::new(session, placeholder));
    bind_ipc_for_restore(hwnd);
    true
}

/// A launch made while a long session is reopening must reach this window, not time out and
/// open a separate one. `handle_ipc_requests` holds what it forwards until the restore is done.
/// Tests never bind the real single-instance pipe.
fn bind_ipc_for_restore(hwnd: HWND) {
    #[cfg(not(test))]
    start_ipc_server_with(hwnd, crate::ipc::bind_session_server);
    #[cfg(test)]
    let _ = hwnd;
}

/// The active tab when it is still the empty, untouched untitled tab every launch starts with.
fn empty_startup_tab(hwnd: HWND) -> Option<DocumentId> {
    use crate::editor::scintilla_constants::SCI_GETLENGTH;
    let app = unsafe { app_ptr(hwnd) }?;
    let app = unsafe { app.as_ref() };
    let active = app.tabs.active().filter(|document| {
        !document.dirty && document.path.is_none() && document.recovery_origin.is_none()
    })?;
    let editor = app.editor.as_ref()?;
    let empty = unsafe { SendMessageW(editor.hwnd(), SCI_GETLENGTH, 0, 0) } == 0;
    empty.then_some(active.id)
}

/// Reopens one manifest entry and applies its language while it is the active tab. Returns
/// its tab, or `None` when the entry could not be reopened.
fn restore_session_entry(hwnd: HWND, entry: &crate::session::SessionEntry) -> Option<DocumentId> {
    match &entry.source {
        crate::session::SessionSource::File(path) => open_path(hwnd, path).ok()?,
        crate::session::SessionSource::Snapshot(id) => {
            let identity = unsafe { window_identity(hwnd) }?;
            let root = recovery_root(hwnd)?;
            let path = crate::recovery::snapshot::snapshot_path(&root, *id);
            let snapshot = crate::recovery::Snapshot::decode(&std::fs::read(&path).ok()?).ok()?;
            let candidate = crate::recovery::SnapshotCandidate { path, snapshot };
            open_snapshot_tab(hwnd, &identity, candidate, SnapshotTab::Session).ok()?;
            adopt_restored_snapshot(hwnd, &identity, &root);
        }
    }
    apply_detected_language(hwnd);
    unsafe { app_ptr(hwnd) }.and_then(|app| Some(unsafe { app.as_ref() }.tabs.active()?.id))
}

/// The snapshot a session tab was just restored from belongs to the previous, exited process, so
/// every other FastPad process would take it for a crash leftover and offer it as "Recovered".
/// Rewriting the text under the tab's own ID, owned by this live process, and then removing the
/// source closes that gap. A failed write keeps the source, so the text is always in some file.
fn adopt_restored_snapshot(hwnd: HWND, identity: &WindowIdentity, root: &std::path::Path) {
    let job = unsafe { app_ptr(hwnd) }.and_then(|app| {
        let app = unsafe { app.as_ref() };
        let editor = app.editor.clone()?;
        let document = app.tabs.active()?;
        let origin = document.recovery_origin.as_ref()?;
        Some((
            editor,
            document.id,
            document.generation,
            document.recovery_id,
            document
                .path
                .clone()
                .or_else(|| origin.original_path.clone()),
            document.encoding,
            origin.snapshot_path.clone(),
        ))
    });
    let Some((editor, id, generation, recovery_id, original_path, encoding, source)) = job else {
        return;
    };
    let Ok(text) = editor.text() else {
        return;
    };
    if !identity.is_live_for(hwnd) {
        return;
    }
    let snapshot = crate::recovery::Snapshot::new(recovery_id, original_path, encoding, text);
    let Ok(written) = crate::recovery::write_snapshot(root, &snapshot) else {
        return;
    };
    if let Some(mut app) = unsafe { app_ptr(hwnd) } {
        unsafe { app.as_mut() }
            .tabs
            .record_recovery_generation(id, generation);
    }
    if written != source {
        crate::recovery::remove_snapshot_files(&[source]);
    }
}

/// Closes the empty startup tab once something replaced it, shows the saved active tab with its
/// caret and scroll position, and reports every entry that failed in one notice.
fn finish_session_restore(hwnd: HWND) {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    let Some(restore) =
        unsafe { app_ptr(hwnd) }.and_then(|mut app| unsafe { app.as_mut() }.session_restore.take())
    else {
        return;
    };
    let restored_any = restore.restored.iter().any(Option::is_some);
    if let Some(placeholder) = restore.placeholder
        && restored_any
        && still_empty_untitled(hwnd, placeholder)
        && activate_document_by_id(hwnd, placeholder)
        && identity.is_live_for(hwnd)
    {
        close_active_document(hwnd);
    }
    if !identity.is_live_for(hwnd) {
        return;
    }
    if let Some(active) = restore.active_tab()
        && activate_document_by_id(hwnd, active)
        && identity.is_live_for(hwnd)
        && restore.saved_active_restored() == Some(active)
    {
        apply_view_state(hwnd, &restore.session.entries[restore.session.active]);
    }
    if !identity.is_live_for(hwnd) {
        return;
    }
    if restore.failed > 0 {
        push_notice(hwnd, crate::session::restore_failure_notice(restore.failed));
    }
    // Launches forwarded during the restore were held in the queue. They open now, after every
    // restored tab, so the file the user just asked for ends up active.
    unsafe {
        PostMessageW(hwnd, crate::window::WM_FASTPAD_IPC_REQUEST, 0, 0);
    }
}

/// A reused startup tab now has a path, and a typed-in one is dirty. Neither may be closed.
fn still_empty_untitled(hwnd: HWND, id: DocumentId) -> bool {
    unsafe { app_ptr(hwnd) }.is_some_and(|app| {
        unsafe { app.as_ref() }
            .tabs
            .document(id)
            .is_some_and(|document| {
                !document.dirty && document.path.is_none() && document.recovery_origin.is_none()
            })
    })
}

fn apply_view_state(hwnd: HWND, entry: &crate::session::SessionEntry) {
    use crate::editor::scintilla_constants::SCI_GETLENGTH;
    let Some(editor) =
        (unsafe { app_ptr(hwnd) }).and_then(|app| unsafe { app.as_ref() }.editor.clone())
    else {
        return;
    };
    // The file may have shrunk since the session was saved.
    let length = unsafe { SendMessageW(editor.hwnd(), SCI_GETLENGTH, 0, 0) }.max(0) as usize;
    let _ = editor.set_selection(entry.anchor.min(length)..entry.caret.min(length));
    let _ = editor.set_first_visible_line(entry.first_line);
}

/// Snapshot files a saved `session.ini` still names. They are waiting for the primary window's
/// next restore, not left by a crash, so no window recovers them while session restore is on,
/// primary or not. With it off they come back through crash recovery like any other.
fn saved_session_snapshots(hwnd: HWND, root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let enabled = unsafe { app_ptr(hwnd) }
        .is_some_and(|app| unsafe { app.as_ref() }.settings.restore_session);
    let Some(session) = enabled
        .then(|| session_manifest_path(hwnd))
        .flatten()
        .and_then(|path| crate::session::read(&path))
    else {
        return Vec::new();
    };
    session
        .entries
        .iter()
        .filter_map(|entry| match entry.source {
            crate::session::SessionSource::Snapshot(id) => {
                Some(crate::recovery::snapshot::snapshot_path(root, id))
            }
            crate::session::SessionSource::File(_) => None,
        })
        .collect()
}

/// Opens every valid foreign snapshot as a recovered tab and reports them with one notice.
fn recover_snapshots(hwnd: HWND) {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    let Some(root) = recovery_root(hwnd) else {
        return;
    };
    // Every open posts the language unit, which continues into this one. While a session
    // restore is still reopening entries it owns their snapshots. Recovery runs again after
    // `WM_FASTPAD_OPEN_REQUEST`, which always posts the language unit.
    if unsafe { app_ptr(hwnd) }.is_some_and(|app| unsafe { app.as_ref() }.session_restore.is_some())
    {
        return;
    }
    let Ok(candidates) =
        crate::recovery::discover_snapshots_with(&root, crate::recovery::owner_is_alive)
    else {
        return;
    };
    let saved = saved_session_snapshots(hwnd, &root);
    let mut recovered = 0;
    for candidate in candidates {
        if saved.contains(&candidate.path) {
            continue;
        }
        // This process's own snapshots, and ones an open tab was already recovered or restored
        // from, are held by a live tab rather than left behind by a crash.
        let claimed = unsafe { app_ptr(hwnd) }.is_none_or(|app| {
            let app = unsafe { app.as_ref() };
            app.owns_recovery_id(candidate.snapshot.recovery_id)
                || app.tabs.documents().any(|document| {
                    document
                        .recovery_origin
                        .as_ref()
                        .is_some_and(|origin| origin.snapshot_path == candidate.path)
                })
        });
        if claimed {
            continue;
        }
        if open_snapshot_tab(hwnd, &identity, candidate, SnapshotTab::Recovered).is_ok() {
            recovered += 1;
        }
        if !identity.is_live_for(hwnd) {
            return;
        }
    }
    if recovered == 0 {
        return;
    }
    let chrome_built = unsafe { app_ptr(hwnd) }.is_some_and(|mut app| {
        let app = unsafe { app.as_mut() };
        app.notifications
            .push(crate::recovery::recovered_notice(recovered));
        app.status.is_some()
    });
    if chrome_built {
        layout_editor_and_find_bar(hwnd);
    }
    unsafe {
        InvalidateRect(hwnd, std::ptr::null(), 1);
    }
}

const IPC_UNAVAILABLE_NOTICE: &str =
    "FastPad could not start its single-instance listener; later launches open separate windows.";

/// Binds the pipe server only for the process that owns the session instance mutex.
fn start_ipc_server_with(hwnd: HWND, bind: impl FnOnce() -> Result<crate::ipc::IpcServer>) {
    let Some(mut app) = (unsafe { app_ptr(hwnd) }) else {
        return;
    };
    // Binding makes no window calls, so this App borrow cannot be re-entered.
    let app = unsafe { app.as_mut() };
    if app.instance_mutex.is_none() || app.ipc.is_some() {
        return;
    }
    match bind() {
        Ok(server) => app.ipc = Some(server),
        Err(_) => stop_ipc(app),
    }
}

fn stop_ipc(app: &mut App) {
    app.ipc = None;
    // Releasing the mutex sends later launches to independent processes instead of a dead pipe.
    app.instance_mutex = None;
    app.notifications.push(IPC_UNAVAILABLE_NOTICE);
}

/// Copies the pipe event out of App for one wait; callers must not dispatch while using it.
pub(crate) fn ipc_wait_handle(
    hwnd: HWND,
    identity: &WindowIdentity,
) -> Option<windows_sys::Win32::Foundation::HANDLE> {
    if !identity.is_live_for(hwnd) {
        return None;
    }
    let app = unsafe { app_ptr(hwnd) }?;
    unsafe { app.as_ref() }
        .ipc
        .as_ref()
        .map(crate::ipc::IpcServer::event)
}

/// Services a signaled pipe event: queues decoded requests and posts one drain message.
pub(crate) fn service_ipc(hwnd: HWND, identity: &WindowIdentity) {
    if !identity.is_live_for(hwnd) {
        return;
    }
    let Some(mut app) = (unsafe { app_ptr(hwnd) }) else {
        return;
    };
    let queued = {
        let app = unsafe { app.as_mut() };
        let Some(server) = app.ipc.as_mut() else {
            return;
        };
        match server.poll() {
            Ok(requests) => {
                let queued = !requests.is_empty();
                app.ipc_requests.extend(requests);
                Some(queued)
            }
            Err(_) => {
                stop_ipc(app);
                None
            }
        }
    };
    match queued {
        Some(true) => unsafe {
            PostMessageW(hwnd, crate::window::WM_FASTPAD_IPC_REQUEST, 0, 0);
        },
        Some(false) => {}
        None => refresh_notifications(hwnd),
    }
}

fn handle_ipc_requests(hwnd: HWND) -> LRESULT {
    // The pipe is bound as soon as a session restore starts. A forwarded file opened now would
    // be buried under the tabs still to be restored, so requests wait in the queue until
    // `finish_session_restore` posts this message again.
    if unsafe { app_ptr(hwnd) }.is_some_and(|app| unsafe { app.as_ref() }.session_restore.is_some())
    {
        return 0;
    }
    open_ipc_requests(hwnd)
}

/// Handles every queued forwarded launch now, restore or not.
fn open_ipc_requests(hwnd: HWND) -> LRESULT {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return 0;
    };
    let requests = unsafe { app_ptr(hwnd) }
        .map(|mut app| std::mem::take(&mut unsafe { app.as_mut() }.ipc_requests))
        .unwrap_or_default();
    for request in requests {
        if !identity.is_live_for(hwnd) {
            return 0;
        }
        match request {
            crate::ipc::IpcRequest::Open(path) => {
                if let Err(error) = App::open_path(hwnd, &path) {
                    report_open_failure(hwnd, &path, &error);
                }
            }
            crate::ipc::IpcRequest::New => execute_command(hwnd, CommandId::New),
            crate::ipc::IpcRequest::Activate => {}
            crate::ipc::IpcRequest::OpenFolder(path) => {
                crate::window::library_host::open_folder(hwnd, &path)
            }
        }
        if identity.is_live_for(hwnd) {
            bring_to_foreground(hwnd);
        }
    }
    0
}

fn bring_to_foreground(hwnd: HWND) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        IsIconic, SW_RESTORE, SetForegroundWindow, ShowWindow,
    };
    unsafe {
        if IsIconic(hwnd) != 0 {
            ShowWindow(hwnd, SW_RESTORE);
        }
        SetForegroundWindow(hwnd);
    }
}

/// Reports a rejected Open (missing file, unsupported encoding, NUL bytes) by naming the file.
pub(crate) fn report_open_failure(hwnd: HWND, path: &std::path::Path, error: &crate::FastPadError) {
    push_notice(
        hwnd,
        format!("FastPad could not open {}: {error}", path.display()),
    );
}

pub(crate) fn push_notice(hwnd: HWND, message: String) {
    if let Some(mut app) = unsafe { app_ptr(hwnd) } {
        unsafe { app.as_mut() }.notifications.push(message);
    }
    refresh_notifications(hwnd);
}

fn refresh_notifications(hwnd: HWND) {
    let chrome_built =
        unsafe { app_ptr(hwnd) }.is_some_and(|app| unsafe { app.as_ref() }.status.is_some());
    if chrome_built {
        layout_editor_and_find_bar(hwnd);
    }
    unsafe {
        InvalidateRect(hwnd, std::ptr::null(), 1);
    }
}

/// How a snapshot comes back as a tab: after a crash (untitled, titled "Recovered: ...") or from
/// the last session (bound to its file again, so Ctrl+S saves where it came from).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SnapshotTab {
    Recovered,
    Session,
}

fn open_snapshot_tab(
    hwnd: HWND,
    identity: &WindowIdentity,
    candidate: crate::recovery::SnapshotCandidate,
    kind: SnapshotTab,
) -> Result<()> {
    if file_population_active(hwnd) {
        return Err(crate::FastPadError::Invariant(
            "file population is already active",
        ));
    }
    let crate::recovery::SnapshotCandidate { path, snapshot } = candidate;
    let from_session = kind == SnapshotTab::Session;
    // A session tab edits its file again, unless another tab already has that file open.
    let bound_path = snapshot.original_path.clone().filter(|original| {
        from_session
            && unsafe { app_ptr(hwnd) }
                .is_some_and(|app| unsafe { app.as_ref() }.tabs.find_path(original).is_none())
    });
    let (editor, id, recovery_id) = {
        let mut app = unsafe { app_ptr(hwnd) }.ok_or(crate::FastPadError::Invariant(
            "main window app state was not available",
        ))?;
        let app = unsafe { app.as_mut() };
        let editor = app
            .editor
            .clone()
            .ok_or(crate::FastPadError::Invariant("editor was not initialized"))?;
        let (id, recovery_id) = app.allocate_document_identity();
        (editor, id, recovery_id)
    };
    let previous = editor.current_document()?;
    let mut document = Document::untitled(id, recovery_id, editor.create_document()?);
    document.path = bound_path;
    document.encoding = snapshot.encoding;
    document.dirty = true;
    document.recovery_generation = Some(document.generation);
    document.recovery_origin = Some(crate::document::RecoveryOrigin {
        snapshot_path: path,
        original_path: snapshot.original_path,
        from_session,
    });
    // Only the active tab's label follows its edits, so a background tab gets its label here.
    crate::window::library_host::label_restored_document(hwnd, &mut document, &snapshot.text);
    if !identity.is_live_for(hwnd) {
        return Err(crate::FastPadError::Invariant(
            "main window was destroyed during recovery",
        ));
    }
    set_file_population(hwnd, true);
    // Undo collection stays on so the loaded text leaves the save point: the tab starts dirty.
    let result = editor
        .use_document(&document.handle)
        .and_then(|_| editor.set_text(&snapshot.text));
    drop(snapshot.text);
    if result.is_err() && identity.is_live_for(hwnd) {
        let _ = editor.use_document(&previous);
    }
    if !identity.is_live_for(hwnd) {
        return Err(crate::FastPadError::Invariant(
            "main window was destroyed during recovery",
        ));
    }
    set_file_population(hwnd, false);
    result?;
    let pushed = unsafe { app_ptr(hwnd) }
        .ok_or(crate::FastPadError::Invariant(
            "main window app state was not available",
        ))
        .and_then(|mut app| {
            unsafe { app.as_mut() }
                .tabs
                .push(document)
                .map_err(|_| crate::FastPadError::Invariant("duplicate document path"))
        });
    if pushed.is_err() {
        let _ = editor.use_document(&previous);
    }
    pushed?;
    refresh_tabs(hwnd);
    Ok(())
}

/// A successful save supersedes both the document's own snapshot and any recovery source.
pub(super) fn remove_saved_document_snapshots(hwnd: HWND) {
    let files = unsafe { app_ptr(hwnd) }.map(|mut app| {
        let app = unsafe { app.as_mut() };
        let own = app.recovery_root.as_deref().map(|root| {
            let active = app.tabs.active()?;
            Some(crate::recovery::snapshot::snapshot_path(
                root,
                active.recovery_id,
            ))
        });
        let source = app
            .tabs
            .take_active_recovery_origin()
            .map(|origin| origin.snapshot_path);
        own.flatten().into_iter().chain(source).collect::<Vec<_>>()
    });
    crate::recovery::remove_snapshot_files(&files.unwrap_or_default());
}

fn remove_session_snapshots(hwnd: HWND, discarded: &[DocumentId]) {
    let files = unsafe { app_ptr(hwnd) }.and_then(|app| {
        let app = unsafe { app.as_ref() };
        let root = app.recovery_root.as_deref()?;
        Some(
            app.tabs
                .documents()
                .flat_map(|document| {
                    crate::recovery::snapshots_removed_on_close(
                        root,
                        document,
                        discarded.contains(&document.id),
                    )
                })
                .collect::<Vec<_>>(),
        )
    });
    crate::recovery::remove_snapshot_files(&files.unwrap_or_default());
}

/// Where this window keeps its session, or `None` when it does not take part: session restore
/// is off, or this is not the primary instance. Tests never resolve the real path.
fn session_path(hwnd: HWND) -> Option<std::path::PathBuf> {
    let app = unsafe { app_ptr(hwnd) }?;
    let app = unsafe { app.as_ref() };
    if !app.settings.restore_session || app.instance_mutex.is_none() {
        return None;
    }
    session_manifest_path(hwnd)
}

/// The primary window's manifest when session restore is off, so a stale one can be deleted.
fn disabled_session_path(hwnd: HWND) -> Option<std::path::PathBuf> {
    let app = unsafe { app_ptr(hwnd) }?;
    let app = unsafe { app.as_ref() };
    if app.settings.restore_session || app.instance_mutex.is_none() {
        return None;
    }
    session_manifest_path(hwnd)
}

/// Where the manifest lives, whichever window asks. Only `session_path` and
/// `disabled_session_path` decide whether this window may change it. Tests never resolve the
/// real path, only a pre-seeded `App::session_path`.
fn session_manifest_path(hwnd: HWND) -> Option<std::path::PathBuf> {
    let mut app = unsafe { app_ptr(hwnd) }?;
    let app = unsafe { app.as_mut() };
    #[cfg(not(test))]
    if app.session_path.is_none() {
        app.session_path = crate::session::session_file_path().ok();
    }
    app.session_path.clone()
}

/// With session restore on, records every tab in `session.ini` instead of asking about unsaved
/// changes. False sends the caller to the review prompts: the feature is off, or some unsaved
/// text could not be secured in a snapshot and must not close silently.
fn save_session_for_close(hwnd: HWND) -> bool {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return false;
    };
    let Some(path) = session_path(hwnd) else {
        // Turned off during this session: a manifest from an earlier close must not outlive the
        // prompts this close shows instead.
        if let Some(stale) = disabled_session_path(hwnd) {
            crate::session::remove(&stale);
        }
        return false;
    };
    // Mid-restore, the manifest is gone and unrestored entries are in no tab. The review flow
    // keeps their snapshots on disk for recovery instead.
    if unsafe { app_ptr(hwnd) }.is_some_and(|app| unsafe { app.as_ref() }.session_restore.is_some())
    {
        return false;
    }
    let Some(root) = recovery_root(hwnd) else {
        return false;
    };
    let pending = unsafe { app_ptr(hwnd) }.map_or(0, |app| {
        unsafe { app.as_ref() }
            .tabs
            .documents()
            .filter(|document| crate::recovery::needs_snapshot(document))
            .count()
    });
    // Each call writes the next document still needing one, so `pending` calls cover them all.
    for _ in 0..pending {
        snapshot_next_document(hwnd);
        if !identity.is_live_for(hwnd) {
            return false;
        }
    }
    let Some(session) = build_session(hwnd, &root) else {
        return false;
    };
    let written = if session.entries.is_empty() {
        crate::session::remove(&path);
        Ok(())
    } else {
        crate::session::write(&path, &session)
    };
    if written.is_err() {
        return false;
    }
    let files = unsafe { app_ptr(hwnd) }.map(|app| {
        unsafe { app.as_ref() }
            .tabs
            .documents()
            .flat_map(|document| {
                crate::recovery::snapshots_removed_on_session_close(&root, document)
            })
            .collect::<Vec<_>>()
    });
    crate::recovery::remove_snapshot_files(&files.unwrap_or_default());
    true
}

/// The manifest for the open tabs, or `None` when a dirty tab's text is in no snapshot file.
/// Clean untitled tabs are empty and skipped. Only the shown tab has a caret and scroll
/// position worth keeping, because switching tabs resets the view.
fn build_session(hwnd: HWND, root: &std::path::Path) -> Option<crate::session::Session> {
    use crate::editor::scintilla_constants::{SCI_GETANCHOR, SCI_GETCURRENTPOS};
    use crate::session::{Session, SessionEntry, SessionSource};
    let app = unsafe { app_ptr(hwnd) }?;
    let app = unsafe { app.as_ref() };
    let active_id = app.tabs.active().map(|document| document.id);
    let mut session = Session::default();
    let mut active_entry = None;
    for document in app.tabs.documents() {
        let is_active = Some(document.id) == active_id;
        let source = if document.dirty {
            let file = crate::recovery::current_snapshot_file(root, document)?;
            SessionSource::Snapshot(crate::recovery::snapshot::snapshot_file_id(&file)?)
        } else if let Some(path) = document
            .path
            .as_ref()
            .filter(|path| path.to_str().is_some())
        {
            SessionSource::File(path.clone())
        } else {
            if is_active {
                session.active = session.entries.len().saturating_sub(1);
            }
            continue;
        };
        if is_active {
            session.active = session.entries.len();
            active_entry = Some(session.entries.len());
        }
        session.entries.push(SessionEntry::new(source));
    }
    if let (Some(index), Some(editor)) = (active_entry, app.editor.as_ref()) {
        let read = |message| unsafe { SendMessageW(editor.hwnd(), message, 0, 0) }.max(0) as usize;
        let entry = &mut session.entries[index];
        entry.caret = read(SCI_GETCURRENTPOS);
        entry.anchor = read(SCI_GETANCHOR);
        entry.first_line = editor.first_visible_line().unwrap_or(0);
    }
    Some(session)
}

/// Services the pipe and handles everything already queued, before a close review begins. This
/// ignores the mid-restore hold: a close then uses the review, which must see a forwarded file
/// as a tab rather than drop it with the window. If the review is cancelled, the restore goes on
/// and the extra tab is simply one more open tab.
fn drain_ipc_requests(hwnd: HWND) {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    service_ipc(hwnd, &identity);
    if identity.is_live_for(hwnd) {
        open_ipc_requests(hwnd);
    }
}

/// Stops accepting forwarded launches before releasing the mutex that invites the next primary to
/// bind the pipe; the reverse order would let it bind while this server still exists.
fn shutdown_ipc(hwnd: HWND) {
    let Some(mut app) = (unsafe { app_ptr(hwnd) }) else {
        return;
    };
    let app = unsafe { app.as_mut() };
    drop(app.ipc.take());
    drop(app.instance_mutex.take());
}

fn clear_documents_for_shutdown(hwnd: HWND) {
    if let Some(mut app) = unsafe { app_ptr(hwnd) } {
        unsafe { app.as_mut() }.tabs.clear_for_shutdown();
    }
}

fn handle_editor_notification(hwnd: HWND, lparam: LPARAM) {
    if file_population_active(hwnd) {
        return;
    }
    if lparam == 0 {
        return;
    }
    let notification = unsafe { &*(lparam as *const NMHDR) };
    if unsafe { editor_hwnd(hwnd) } != Some(notification.hwndFrom) {
        return;
    }
    if notification.code == crate::editor::scintilla_constants::SCN_UPDATEUI {
        invalidate_status_bar(hwnd);
        let update = unsafe { &*(lparam as *const crate::editor::ScintillaNotification) };
        if update.updated as u32 & crate::editor::scintilla_constants::SC_UPDATE_V_SCROLL != 0 {
            crate::window::preview_host::editor_scrolled(hwnd);
        }
        return;
    }
    if notification.code == crate::editor::scintilla_constants::SCN_ZOOM {
        with_editor(hwnd, |editor| {
            let _ = editor.remeasure_line_numbers();
        });
        return;
    }
    if notification.code == crate::editor::scintilla_constants::SCN_MODIFIED {
        let modification = unsafe { &*(lparam as *const crate::editor::ScintillaNotification) };
        let text_changes = crate::editor::scintilla_constants::SC_MOD_INSERTTEXT
            | crate::editor::scintilla_constants::SC_MOD_DELETETEXT;
        let text_change = modification.modification_type & text_changes as i32 != 0;
        let mut promoted = false;
        if text_change && let Some(mut app) = unsafe { app_ptr(hwnd) } {
            let app = unsafe { app.as_mut() };
            promoted = app.tabs.note_active_text_change();
            if modification.lines_added != 0
                && let Some(editor) = app.editor.as_ref()
            {
                let _ = editor.refresh_line_numbers();
            }
        }
        if promoted {
            invalidate_title_strip(hwnd);
        }
        if text_change {
            crate::window::library_host::text_changed(hwnd, modification.position.max(0) as usize);
            crate::window::library_host::schedule_autosave(hwnd);
            crate::window::preview_host::record_edit(hwnd, modification);
        }
        return;
    }
    let dirty = match notification.code {
        crate::editor::scintilla_constants::SCN_SAVEPOINTLEFT => true,
        crate::editor::scintilla_constants::SCN_SAVEPOINTREACHED => false,
        _ => return,
    };
    let changed = unsafe { app_ptr(hwnd) }
        .map(|mut app| {
            let app = unsafe { app.as_mut() };
            app.tabs.set_active_dirty(dirty)
        })
        .unwrap_or(false);
    if changed {
        invalidate_title_strip(hwnd);
    }
}

pub(crate) fn invalidate_title_strip(hwnd: HWND) {
    unsafe {
        InvalidateRect(hwnd, std::ptr::null(), 0);
    }
}

/// Refreshes the retained tab-view snapshot after a document's title-affecting field (e.g. its
/// untitled label) changed in place, and repaints the title strip.
pub(crate) fn refresh_tab_view(hwnd: HWND) {
    if let Some(app) = unsafe { app_ptr(hwnd) } {
        unsafe { app.as_ref() }.tabs.refresh_view();
    }
    invalidate_title_strip(hwnd);
}

fn ensure_accessibility(hwnd: HWND) -> *mut c_void {
    unsafe { app_ptr(hwnd) }
        .map(|mut app| unsafe { app.as_mut() }.ensure_accessibility())
        .unwrap_or(std::ptr::null_mut())
}

pub(crate) fn menu_mode(hwnd: HWND) -> Option<MenuMode> {
    unsafe { app_ptr(hwnd) }.and_then(|app| unsafe { app.as_ref() }.menu_mode)
}

fn menu_band_height(hwnd: HWND) -> i32 {
    menu_mode(hwnd).map_or(0, |_| {
        menu_band::band_height(
            unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForWindow(hwnd) }.max(96),
        )
    })
}

/// The menu band's heading rectangles, or none outside menu mode.
fn menu_headings(hwnd: HWND) -> Vec<RECT> {
    if menu_mode(hwnd).is_none() {
        return Vec::new();
    }
    let dpi = unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForWindow(hwnd) }.max(96);
    let widths = menu_band::measure_titles(hwnd, title_chrome(hwnd).1.text());
    menu_band::heading_rects(
        &widths,
        crate::window::side_panel::left_edge(hwnd),
        title_layout(hwnd).height,
        dpi,
    )
}

/// Stores `mode`, re-laying out the window when the band appears or disappears.
fn set_menu_mode(hwnd: HWND, mode: Option<MenuMode>) {
    let Some(mut app) = (unsafe { app_ptr(hwnd) }) else {
        return;
    };
    let app = unsafe { app.as_mut() };
    let previous = std::mem::replace(&mut app.menu_mode, mode);
    if previous == mode {
        return;
    }
    if previous.is_some() != mode.is_some() {
        layout_editor_and_find_bar(hwnd);
    }
    unsafe {
        InvalidateRect(hwnd, std::ptr::null(), 0);
    }
}

fn enter_menu_mode(hwnd: HWND, hot: usize) {
    if menu_mode(hwnd).is_some() {
        return;
    }
    let ready = unsafe { app_ptr(hwnd) }.is_some_and(|mut app| {
        let app = unsafe { app.as_mut() };
        if app.menu_bar.is_none() {
            app.menu_bar = MenuBar::create().ok();
        }
        app.menu_return_focus = unsafe { GetFocus() };
        app.menu_bar.is_some()
    });
    if !ready {
        return;
    }
    set_menu_mode(hwnd, Some(MenuMode { hot, open: false }));
    unsafe {
        SetFocus(hwnd);
    }
}

fn exit_menu_mode(hwnd: HWND) {
    if menu_mode(hwnd).is_none() {
        return;
    }
    set_menu_mode(hwnd, None);
    // Focus moved elsewhere (a click on the editor, another app) is left where it went.
    if unsafe { GetFocus() } != hwnd {
        return;
    }
    let previous = unsafe { app_ptr(hwnd) }
        .map(|app| unsafe { app.as_ref() }.menu_return_focus)
        .unwrap_or(std::ptr::null_mut());
    let target = if !previous.is_null()
        && previous != hwnd
        && unsafe { IsWindow(previous) } != 0
        && unsafe { IsWindowVisible(previous) } != 0
    {
        Some(previous)
    } else if tab_count(hwnd) > 0 {
        content_focus_target(hwnd)
    } else {
        None
    };
    if let Some(target) = target {
        unsafe {
            SetFocus(target);
        }
    }
}

/// Opens heading `index`'s dropdown, then follows the user between headings until a command is
/// picked or the menu is dismissed.
fn open_menu(hwnd: HWND, mut index: usize) {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    enter_menu_mode(hwnd, index);
    loop {
        let Some(menu) = unsafe { app_ptr(hwnd) }.and_then(|app| {
            unsafe { app.as_ref() }
                .menu_bar
                .as_ref()
                .map(|bar| bar.dropdown(index))
        }) else {
            return;
        };
        if index == crate::window::menu_band::VIEW_MENU_INDEX {
            menus::set_markdown_preview_enabled(
                menu,
                crate::window::preview_host::buttons_visible(hwnd),
            );
            menus::set_sidebar_enabled(menu, notes_mode_enabled(hwnd));
        }
        set_menu_mode(
            hwnd,
            Some(MenuMode {
                hot: index,
                open: true,
            }),
        );
        unsafe {
            windows_sys::Win32::Graphics::Gdi::UpdateWindow(hwnd);
        }
        let headings = menu_headings(hwnd);
        let exit = menus::track_dropdown(hwnd, menu, index, &headings);
        if !identity.is_live_for(hwnd) {
            return;
        }
        match exit {
            DropdownExit::Switch(next) => index = next,
            DropdownExit::Escape => {
                set_menu_mode(
                    hwnd,
                    Some(MenuMode {
                        hot: index,
                        open: false,
                    }),
                );
                return;
            }
            DropdownExit::Dismissed => {
                exit_menu_mode(hwnd);
                return;
            }
            DropdownExit::Command(command) => {
                exit_menu_mode(hwnd);
                execute_command(hwnd, command);
                return;
            }
        }
    }
}

/// Keyboard navigation of the menu band; false leaves the key to the default handling.
fn handle_menu_key(hwnd: HWND, message: u32, key: WPARAM) -> bool {
    let Some(mode) = menu_mode(hwnd) else {
        return false;
    };
    let Ok(key) = u16::try_from(key) else {
        return false;
    };
    match key {
        VK_LEFT | VK_RIGHT => {
            let hot = menu_band::neighbor(mode.hot, key == VK_RIGHT);
            set_menu_mode(hwnd, Some(MenuMode { hot, ..mode }));
        }
        VK_DOWN | VK_UP | VK_RETURN => open_menu(hwnd, mode.hot),
        VK_ESCAPE => exit_menu_mode(hwnd),
        // Alt and F10 toggle the band through `translate_accelerator`.
        VK_MENU | VK_F10 | VK_SHIFT | VK_CONTROL => {}
        // Alt+letter arrives again as SC_KEYMENU with the letter, which opens the heading.
        _ if message == WM_SYSKEYDOWN => return false,
        _ => match menu_band::mnemonic_heading(u32::from(key)) {
            Some(index) => open_menu(hwnd, index),
            None => exit_menu_mode(hwnd),
        },
    }
    true
}

fn hover_menu_heading(hwnd: HWND, lparam: LPARAM) {
    let Some(mode) = menu_mode(hwnd).filter(|mode| !mode.open) else {
        return;
    };
    let (x, y) = (
        (lparam as u32 & 0xffff) as u16 as i16 as i32,
        ((lparam as u32 >> 16) & 0xffff) as u16 as i16 as i32,
    );
    if let Some(hot) = menu_band::heading_at(&menu_headings(hwnd), x, y) {
        set_menu_mode(hwnd, Some(MenuMode { hot, ..mode }));
    }
}

pub(crate) unsafe fn translate_accelerator(
    hwnd: HWND,
    identity: &WindowIdentity,
    message: &windows_sys::Win32::UI::WindowsAndMessaging::MSG,
) -> bool {
    if !identity.is_live_for(hwnd) {
        return false;
    }
    if menu_activation_message(hwnd, message)
        && unsafe { PostMessageW(hwnd, WM_SYSCOMMAND, SC_KEYMENU as usize, 0) } != 0
    {
        return true;
    }
    let accelerator = unsafe { app_ptr(hwnd) }.and_then(|app| {
        unsafe { app.as_ref() }
            .accelerators
            .as_ref()
            .map(|table| table.raw())
    });
    accelerator.is_some_and(|accelerator| menus::translate_accelerator(accelerator, hwnd, message))
}

fn menu_activation_message(
    hwnd: HWND,
    message: &windows_sys::Win32::UI::WindowsAndMessaging::MSG,
) -> bool {
    let no_control_or_shift = unsafe { GetKeyState(VK_CONTROL as i32) } >= 0
        && unsafe { GetKeyState(VK_SHIFT as i32) } >= 0;
    let f10 = matches!(message.message, WM_KEYDOWN | WM_SYSKEYDOWN)
        && message.wParam == VK_F10 as usize
        && no_control_or_shift
        && unsafe { GetKeyState(VK_MENU as i32) } >= 0;
    let Some(mut app) = (unsafe { app_ptr(hwnd) }) else {
        return f10;
    };
    let app = unsafe { app.as_mut() };
    if f10 {
        app.set_menu_alt_pending(false);
        return true;
    }
    if message.message == WM_SYSKEYDOWN && message.wParam == VK_MENU as usize {
        app.set_menu_alt_pending(no_control_or_shift);
        return false;
    }
    if message.message == WM_SYSKEYUP && message.wParam == VK_MENU as usize {
        return app.take_menu_alt_pending();
    }
    if matches!(message.message, WM_KEYDOWN | WM_SYSKEYDOWN) {
        app.set_menu_alt_pending(false);
    }
    false
}

unsafe fn install_editor(hwnd: HWND, editor: Editor, document: Document) -> Result<()> {
    // SAFETY: The App pointer is re-fetched after editor creation so initialization never mutates
    // an App reference borrowed across a reentrant Win32 call.
    let Some(mut app) = (unsafe { app_ptr(hwnd) }) else {
        return Err(crate::FastPadError::Invariant(
            "main window app state was not available",
        ));
    };
    let app = unsafe { app.as_mut() };
    app.tabs
        .push(document)
        .map_err(|_| crate::FastPadError::Invariant("duplicate document path"))?;
    app.editor = Some(editor);
    Ok(())
}

pub(crate) unsafe fn app_ptr(hwnd: HWND) -> Option<NonNull<App>> {
    // SAFETY: `GWLP_USERDATA` is written exactly once from `WM_NCCREATE` with a `Box<App>` owned
    // by the window and cleared in `WM_NCDESTROY`. Callers must not keep references alive across
    // reentrant Win32 calls; they may only copy values or perform immediate mutation.
    NonNull::new(unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut App })
}

pub(super) unsafe fn window_identity(hwnd: HWND) -> Option<WindowIdentity> {
    // SAFETY: Clone only the App's stable identity token. The temporary App reference ends before
    // callers cross any reentrant Win32 boundary.
    let app = unsafe { app_ptr(hwnd) }?;
    Some(unsafe { app.as_ref() }.window_identity())
}

unsafe fn take_create_context_app(lparam: LPARAM) -> Option<Box<App>> {
    let create = unsafe { &mut *(lparam as *mut CREATESTRUCTW) };
    let context = create.lpCreateParams as *mut WindowCreateContext<App>;
    if context.is_null() {
        return None;
    }
    unsafe { (*context).value.take() }
}

fn store_app(hwnd: HWND, value: Box<App>) {
    unsafe {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(value) as isize);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        MainWindowClass, WindowCreateContext, execute_command, handle_paint_with,
        mark_first_paint_complete, take_deferred_start_pending, with_command_palette,
    };
    use crate::app::App;
    use crate::document::{CloseDecision, Language, RecoveryId};
    use crate::editor::scintilla_constants::SCI_GETMODIFY;
    use crate::file::encoding::Encoding;
    use crate::languages::LanguageManager;
    use crate::launch::LaunchOptions;
    use crate::perf::StartupMetrics;
    use crate::recovery::snapshot::snapshot_path;
    use crate::recovery::{Snapshot, write_snapshot};
    use crate::session::{Session, SessionEntry, SessionSource};
    use crate::window::commands::CommandId;
    use crate::window::menus::answer_next_popup_menu;
    use crate::window::modal::{answer_next_close_prompt, answer_next_save_dialog};
    use std::cell::RefCell;
    use std::path::PathBuf;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use windows_sys::Win32::Foundation::{HWND, LRESULT, RECT};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DestroyWindow, DispatchMessageW, GWLP_USERDATA, GetClientRect, GetWindowLongPtrW, IsWindow,
        MSG, PM_REMOVE, PeekMessageW, SendMessageW, WM_CLOSE, WM_PAINT,
    };

    /// Dispatches everything already posted to `hwnd`, leaving any WM_QUIT for the harness.
    fn pump_posted_messages(hwnd: HWND) {
        let mut message = MSG::default();
        while unsafe { PeekMessageW(&mut message, hwnd, 0, 0, PM_REMOVE) } != 0 {
            unsafe {
                DispatchMessageW(&message);
            }
        }
    }

    #[test]
    fn the_main_window_has_a_title_for_the_taskbar() {
        // Break caught: WM_NCCREATE handled without the default processing leaves the window text
        // empty, so the taskbar button and Alt+Tab show only the icon.
        let window = ProductionWindow::new(make_app());
        let mut text = [0u16; 32];
        let len = unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::GetWindowTextW(
                window.hwnd,
                text.as_mut_ptr(),
                text.len() as i32,
            )
        };
        assert_eq!(String::from_utf16_lossy(&text[..len as usize]), "FastPad");
    }

    #[test]
    fn the_main_window_class_resets_the_cursor_over_its_client_area() {
        // Break caught: without a class cursor, WM_SETCURSOR over the tab strip (HTCLIENT) leaves
        // whatever cursor was last shown, such as the editor's I-beam or a resize arrow.
        let window = ProductionWindow::new(make_app());
        let cursor = unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::GetClassLongPtrW(
                window.hwnd,
                windows_sys::Win32::UI::WindowsAndMessaging::GCLP_HCURSOR,
            )
        };
        assert_ne!(cursor, 0);
    }

    #[test]
    fn the_window_title_follows_the_active_tab_and_its_dirty_state() {
        // Break caught: the taskbar button keeps showing a stale or bare title while the tab strip
        // shows which file is open and whether it has unsaved changes.
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        let window_text = || {
            unsafe {
                SendMessageW(window.hwnd, WM_PAINT, 0, 0);
            }
            let mut text = [0u16; 64];
            let len = unsafe {
                windows_sys::Win32::UI::WindowsAndMessaging::GetWindowTextW(
                    window.hwnd,
                    text.as_mut_ptr(),
                    text.len() as i32,
                )
            };
            String::from_utf16_lossy(&text[..len as usize])
        };

        assert_eq!(window_text(), "Untitled - FastPad");
        editor.set_text("dirty").unwrap();
        // Notes mode is on by default, so the untitled tab picks up "dirty" as its label.
        assert_eq!(window_text(), "dirty * - FastPad");
        assert_eq!(super::window_title(None), "FastPad");
    }

    #[test]
    fn a_forwarded_request_is_handled_before_the_close_review_starts() {
        // Break caught: a launch forwarded just before WM_CLOSE is dropped when the window closes,
        // so the file the user double-clicked silently never opens.
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        editor.set_text("dirty").unwrap();
        app_mut(window.hwnd)
            .ipc_requests
            .push(crate::ipc::IpcRequest::New);
        let tabs_at_prompt = Arc::new(AtomicUsize::new(0));
        let observed = Arc::clone(&tabs_at_prompt);
        answer_next_close_prompt(move |hwnd| {
            observed.store(app_mut(hwnd).tabs.len(), Ordering::SeqCst);
            CloseDecision::Cancel
        });

        unsafe {
            SendMessageW(window.hwnd, WM_CLOSE, 0, 0);
        }

        assert_ne!(
            unsafe { IsWindow(window.hwnd) },
            0,
            "Cancel must abort the close"
        );
        assert_eq!(
            tabs_at_prompt.load(Ordering::SeqCst),
            2,
            "the queued request must be handled before the review starts"
        );
        assert!(app_mut(window.hwnd).ipc_requests.is_empty());
    }

    #[test]
    fn a_confirmed_close_releases_the_pipe_server_and_the_instance_mutex() {
        // Break caught: dropping the instance mutex before the pipe server lets the next launch
        // claim the session and fail to bind a name this process still owns.
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        let names = crate::ipc::server::tests::unique_names();
        app_mut(window.hwnd).instance_mutex = Some(unnamed_mutex());
        super::start_ipc_server_with(window.hwnd, || {
            crate::ipc::IpcServer::bind(&names, &crate::ipc::CurrentUserAcl::current()?)
        });
        assert!(app_mut(window.hwnd).ipc.is_some());

        unsafe {
            SendMessageW(window.hwnd, WM_CLOSE, 0, 0);
        }

        assert_eq!(unsafe { IsWindow(window.hwnd) }, 0);
        crate::ipc::IpcServer::bind(&names, &crate::ipc::CurrentUserAcl::current().unwrap())
            .expect("the pipe name must be free once the window has closed");
    }

    #[test]
    fn deferred_startup_work_is_held_until_the_modal_prompt_closes() {
        // Break caught: a nested modal loop dispatches deferred chain units, so recovery can push
        // and activate tabs while a close prompt or file dialog is deciding about another one.
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        let root = RecoveryScratch::new("modal-deferred");
        write_snapshot(
            root.path(),
            &Snapshot::new(
                RecoveryId::from_u128(0x5151),
                None,
                Encoding::Utf8,
                "recovered elsewhere",
            ),
        )
        .unwrap();
        app_mut(window.hwnd).recovery_root = Some(root.path().to_path_buf());
        editor.set_text("dirty").unwrap();
        let before = app_mut(window.hwnd).tabs.len();

        answer_next_close_prompt(|hwnd| {
            unsafe {
                SendMessageW(hwnd, crate::window::WM_FASTPAD_RECOVERY, 0, 0);
            }
            CloseDecision::Cancel
        });
        execute_command(window.hwnd, CommandId::CloseTab);

        assert_eq!(
            app_mut(window.hwnd).tabs.len(),
            before,
            "the recovery unit ran inside the modal loop"
        );

        pump_posted_messages(window.hwnd);

        assert_eq!(
            app_mut(window.hwnd).tabs.len(),
            before + 1,
            "the held recovery unit must run once the modal loop ends"
        );
    }

    #[test]
    fn recovery_never_reopens_a_snapshot_an_open_tab_already_holds() {
        // Break caught: a later Open (every open re-runs the recovery unit) or a session restore
        // opening a second copy of text that is already in a tab.
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        let root = RecoveryScratch::new("claimed");
        write_snapshot(
            root.path(),
            &Snapshot::new(
                RecoveryId::from_u128(0x7171),
                None,
                Encoding::Utf8,
                "held once",
            ),
        )
        .unwrap();
        app_mut(window.hwnd).recovery_root = Some(root.path().to_path_buf());

        unsafe { SendMessageW(window.hwnd, crate::window::WM_FASTPAD_RECOVERY, 0, 0) };
        assert_eq!(app_mut(window.hwnd).tabs.len(), 2);
        unsafe { SendMessageW(window.hwnd, crate::window::WM_FASTPAD_RECOVERY, 0, 0) };
        assert_eq!(app_mut(window.hwnd).tabs.len(), 2);
    }

    #[test]
    fn queued_ipc_requests_wait_for_the_overflow_menu_to_close() {
        // Break caught: the overflow menu's own modal loop dispatches a forwarded request, so a
        // new tab becomes active underneath it and the command the user picks acts on that tab
        // instead of the one they opened the menu on.
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        editor.set_text("dirty").unwrap();
        app_mut(window.hwnd)
            .ipc_requests
            .push(crate::ipc::IpcRequest::New);
        let before = app_mut(window.hwnd).tabs.len();

        answer_next_popup_menu(|hwnd| {
            unsafe {
                SendMessageW(hwnd, crate::window::WM_FASTPAD_IPC_REQUEST, 0, 0);
            }
            None
        });
        assert!(crate::window::menus::show_overflow(window.hwnd, 0, 0).is_none());

        assert_eq!(
            app_mut(window.hwnd).tabs.len(),
            before,
            "a forwarded request was handled inside the overflow menu's modal loop"
        );

        pump_posted_messages(window.hwnd);

        assert_eq!(
            app_mut(window.hwnd).tabs.len(),
            before + 1,
            "the held request must run once the overflow menu closes"
        );
    }

    #[test]
    fn closing_every_tab_hides_the_editor_until_the_empty_strip_opens_a_new_one() {
        // Break caught: the last tab being silently replaced (its close button looks inert), the
        // hidden editor still taking edits, or the empty strip's double-click and context menu
        // not reaching New and Close all tabs.
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GWL_STYLE, GetWindowLongPtrW, HTCAPTION, WM_NCLBUTTONDBLCLK, WM_NCRBUTTONUP, WS_VISIBLE,
        };
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        let editor_visible =
            || unsafe { GetWindowLongPtrW(editor.hwnd(), GWL_STYLE) } as u32 & WS_VISIBLE != 0;
        assert!(editor_visible());

        execute_command(window.hwnd, CommandId::CloseTab);

        assert!(app_mut(window.hwnd).tabs.is_empty());
        assert!(!editor_visible());
        execute_command(window.hwnd, CommandId::Paste);
        execute_command(window.hwnd, CommandId::CloseTab);
        assert_eq!(editor.text().unwrap(), "");

        // The empty strip right of the sidebar; over the sidebar the caption maximizes instead.
        let drag = super::title_layout(window.hwnd).drag_region.center();
        let strip = screen_lparam(window.hwnd, drag.x, drag.y);
        unsafe {
            SendMessageW(window.hwnd, WM_NCLBUTTONDBLCLK, HTCAPTION as usize, strip);
            SendMessageW(window.hwnd, WM_NCLBUTTONDBLCLK, HTCAPTION as usize, strip);
        }
        assert_eq!(app_mut(window.hwnd).tabs.len(), 2);
        assert!(editor_visible());

        answer_next_popup_menu(|_| Some(CommandId::CloseAllTabs));
        let drag = super::title_layout(window.hwnd).drag_region.center();
        let strip = screen_lparam(window.hwnd, drag.x, drag.y);
        unsafe {
            SendMessageW(window.hwnd, WM_NCRBUTTONUP, HTCAPTION as usize, strip);
        }
        assert!(app_mut(window.hwnd).tabs.is_empty());
        assert!(!editor_visible());
    }

    #[test]
    fn keyboard_shortcuts_reach_their_commands_through_the_accelerator_table() {
        // Break caught: every shortcut dead in the running app while command-level tests pass,
        // because nothing exercised the accelerator translation the message loop depends on.
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
            GetKeyboardState, SetKeyboardState, VK_CONTROL,
        };
        use windows_sys::Win32::UI::WindowsAndMessaging::{MSG, WM_KEYDOWN};
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        let identity = unsafe { super::window_identity(window.hwnd).unwrap() };
        let mut keys = [0u8; 256];
        unsafe { GetKeyboardState(keys.as_mut_ptr()) };
        let original = keys;
        keys[VK_CONTROL as usize] = 0x80;
        unsafe { SetKeyboardState(keys.as_ptr()) };
        let message = MSG {
            hwnd: editor.hwnd(),
            message: WM_KEYDOWN,
            wParam: usize::from(b'T'),
            ..Default::default()
        };
        let translated = unsafe { super::translate_accelerator(window.hwnd, &identity, &message) };
        unsafe { SetKeyboardState(original.as_ptr()) };

        assert!(translated, "Ctrl+T was not translated");
        assert_eq!(app_mut(window.hwnd).tabs.len(), 2);
    }

    #[test]
    fn tab_shortcuts_cycle_with_wrap_around_and_select_by_position() {
        // Break caught: Ctrl+Tab stopping at the last tab, or Ctrl+9 with fewer than nine tabs
        // activating some other tab instead of doing nothing.
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        execute_command(window.hwnd, CommandId::New);
        execute_command(window.hwnd, CommandId::New);
        let active = || app_mut(window.hwnd).tabs.active_index();
        assert_eq!(active(), 2);

        execute_command(window.hwnd, CommandId::NextTab);
        assert_eq!(active(), 0);
        execute_command(window.hwnd, CommandId::PreviousTab);
        assert_eq!(active(), 2);
        execute_command(window.hwnd, CommandId::PreviousTab);
        assert_eq!(active(), 1);
        execute_command(window.hwnd, CommandId::SelectTab1);
        assert_eq!(active(), 0);
        execute_command(window.hwnd, CommandId::SelectTab3);
        assert_eq!(active(), 2);
        execute_command(window.hwnd, CommandId::SelectTab9);
        assert_eq!(active(), 2);
    }

    #[test]
    fn zoom_resizes_the_line_number_gutter_and_direction_mirrors_the_editor() {
        // Break caught: zoomed digits clipped by a gutter measured at the unzoomed size, or the
        // direction shortcuts leaving the editor window unmirrored.
        use crate::editor::scintilla_constants::SCI_GETZOOM;
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GWL_EXSTYLE, GetWindowLongPtrW, WS_EX_LAYOUTRTL,
        };
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        let zoom = || unsafe { SendMessageW(editor.hwnd(), SCI_GETZOOM, 0, 0) };
        let unzoomed_width = line_number_margin_width(&editor);

        execute_command(window.hwnd, CommandId::ZoomIn);
        execute_command(window.hwnd, CommandId::ZoomIn);
        assert_eq!(zoom(), 2);
        assert!(line_number_margin_width(&editor) > unzoomed_width);
        execute_command(window.hwnd, CommandId::ZoomOut);
        assert_eq!(zoom(), 1);
        execute_command(window.hwnd, CommandId::ZoomReset);
        assert_eq!(zoom(), 0);
        assert_eq!(line_number_margin_width(&editor), unzoomed_width);

        let mirrored = || {
            (unsafe { GetWindowLongPtrW(editor.hwnd(), GWL_EXSTYLE) }) as u32 & WS_EX_LAYOUTRTL != 0
        };
        assert!(!mirrored());
        execute_command(window.hwnd, CommandId::TextRightToLeft);
        assert!(mirrored());
        execute_command(window.hwnd, CommandId::TextLeftToRight);
        assert!(!mirrored());
    }

    #[test]
    fn the_command_palette_filters_as_typed_and_runs_the_selection_on_enter() {
        // Break caught: a palette whose field never refilters the list, or whose Enter leaves the
        // palette open or runs nothing.
        use crate::editor::scintilla_constants::SCI_GETZOOM;
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_DOWN, VK_ESCAPE, VK_RETURN};
        use windows_sys::Win32::UI::WindowsAndMessaging::{SetWindowTextW, WM_KEYDOWN};
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        let palette = |hwnd| app_mut(hwnd).command_palette.as_ref().unwrap();

        // The test window itself is never shown, so check the panel's own style bit.
        let panel_visible = |hwnd| {
            (unsafe { GetWindowLongPtrW(palette(hwnd).panel_hwnd(), super::GWL_STYLE) }) as u32
                & super::WS_VISIBLE
                != 0
        };
        execute_command(window.hwnd, CommandId::CommandPalette);
        assert!(palette(window.hwnd).is_visible());
        assert!(panel_visible(window.hwnd));
        // Markdown preview commands are listed only while the active tab is Markdown.
        assert_eq!(
            palette(window.hwnd).shown().len(),
            crate::window::command_palette::ENTRIES
                .iter()
                .filter(|entry| !entry.command.is_markdown_preview())
                .count()
        );
        let query = palette(window.hwnd).query_hwnd();
        let typed = crate::platform::wide_null("zoom");
        unsafe { SetWindowTextW(query, typed.as_ptr()) };
        let shown = palette(window.hwnd)
            .shown()
            .iter()
            .map(|entry| entry.command)
            .collect::<Vec<_>>();
        assert_eq!(
            shown,
            [CommandId::ZoomIn, CommandId::ZoomOut, CommandId::ZoomReset]
        );

        unsafe { SendMessageW(query, WM_KEYDOWN, VK_DOWN as usize, 0) };
        assert_eq!(
            palette(window.hwnd).selected_command(),
            Some(CommandId::ZoomOut)
        );
        unsafe { SendMessageW(query, WM_KEYDOWN, VK_RETURN as usize, 0) };
        assert!(!palette(window.hwnd).is_visible());
        assert!(!panel_visible(window.hwnd));
        assert_eq!(
            unsafe { SendMessageW(editor.hwnd(), SCI_GETZOOM, 0, 0) },
            -1
        );

        // Reopening starts from an empty query; Escape closes without running anything.
        execute_command(window.hwnd, CommandId::CommandPalette);
        assert_eq!(palette(window.hwnd).query_text(), "");
        unsafe { SendMessageW(query, WM_KEYDOWN, VK_ESCAPE as usize, 0) };
        assert!(!palette(window.hwnd).is_visible());
        assert_eq!(
            unsafe { SendMessageW(editor.hwnd(), SCI_GETZOOM, 0, 0) },
            -1
        );
    }

    #[test]
    fn a_picker_lists_its_items_and_enter_reports_the_choice() {
        // Break caught: a picker whose choice never reaches library_host, or that leaves the
        // palette stuck showing runtime items the next time it opens for commands.
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        super::open_picker(
            window.hwnd,
            crate::window::command_palette::Picker {
                kind: crate::window::command_palette::PickerKind::RecentFolder,
                items: vec![r"D:\A".into(), r"D:\B".into()],
                create: None,
            },
        );
        super::move_command_palette_selection(window.hwnd, 1);
        super::run_command_palette_selection(window.hwnd);
        assert_eq!(
            crate::window::library_host::take_last_pick(),
            Some((
                crate::window::command_palette::PickerKind::RecentFolder,
                crate::window::command_palette::PickerChoice::Item(1)
            ))
        );
        // The palette went back to command mode.
        execute_command(window.hwnd, CommandId::CommandPalette);
        assert!(with_command_palette(window.hwnd, |p| p.picker().is_none()).unwrap());
    }

    #[test]
    fn alt_shows_a_painted_menu_band_that_pushes_the_editor_down_and_runs_dropdown_commands() {
        // Break caught: Alt attached a native menu bar, which Windows drew unthemed over the editor
        // (the reclaimed caption leaves it no room) and left a "File" remnant behind after Escape.
        use crate::window::menus::{DropdownExit, answer_next_dropdown};
        use windows_sys::Win32::UI::WindowsAndMessaging::{GetMenu, GetWindowRect, SC_KEYMENU};
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        super::build_chrome(window.hwnd);
        let editor_top = || {
            let mut rect = RECT::default();
            let mut origin = windows_sys::Win32::Foundation::POINT::default();
            unsafe {
                GetWindowRect(editor.hwnd(), &mut rect);
                windows_sys::Win32::Graphics::Gdi::ClientToScreen(window.hwnd, &mut origin);
            }
            rect.top - origin.y
        };
        let key_menu = |letter: u8| unsafe {
            SendMessageW(
                window.hwnd,
                super::WM_SYSCOMMAND,
                SC_KEYMENU as usize,
                letter as isize,
            )
        };
        let dpi = unsafe { GetDpiForWindow(window.hwnd) }.max(96);
        let title_height = super::title_layout(window.hwnd).height;
        let band = super::menu_band::band_height(dpi);

        key_menu(0);
        assert_eq!(
            app_mut(window.hwnd).menu_mode,
            Some(super::MenuMode {
                hot: 0,
                open: false
            })
        );
        assert!(
            unsafe { GetMenu(window.hwnd) }.is_null(),
            "no native menu bar"
        );
        assert_eq!(editor_top(), title_height + band);
        assert_eq!(super::menu_headings(window.hwnd).len(), 4);

        key_menu(0);
        assert_eq!(app_mut(window.hwnd).menu_mode, None);
        assert_eq!(editor_top(), title_height);

        // Alt+E opens Edit; Right moves to Search, whose Escape leaves Search highlighted.
        answer_next_dropdown(|hwnd, heading| {
            assert_eq!(heading, 1);
            assert_eq!(
                app_mut(hwnd).menu_mode,
                Some(super::MenuMode { hot: 1, open: true })
            );
            DropdownExit::Switch(2)
        });
        answer_next_dropdown(|_, heading| {
            assert_eq!(heading, 2);
            DropdownExit::Escape
        });
        key_menu(b'e');
        assert_eq!(
            app_mut(window.hwnd).menu_mode,
            Some(super::MenuMode {
                hot: 2,
                open: false
            })
        );

        // Down opens the highlighted heading; a picked command leaves menu mode before it runs.
        answer_next_dropdown(|_, heading| {
            assert_eq!(heading, 2);
            DropdownExit::Command(CommandId::Find)
        });
        assert!(super::handle_menu_key(
            window.hwnd,
            windows_sys::Win32::UI::WindowsAndMessaging::WM_KEYDOWN,
            super::VK_DOWN as usize,
        ));
        assert_eq!(app_mut(window.hwnd).menu_mode, None);
        assert!(app_mut(window.hwnd).find_bar.as_ref().unwrap().is_visible());
        assert_eq!(
            editor_top(),
            title_height + super::find_bar::find_bar_height(dpi)
        );
    }

    #[test]
    fn the_find_bar_panel_reserves_its_band_above_the_editor_and_follows_theme_changes() {
        // Break caught: a find bar whose painted band is not reserved (the editor draws over it),
        // or that keeps the old colors after the theme changes while it is open.
        use windows_sys::Win32::UI::WindowsAndMessaging::GetWindowRect;
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        super::build_chrome(window.hwnd);
        let top_of = |child| {
            let mut rect = RECT::default();
            let mut origin = windows_sys::Win32::Foundation::POINT::default();
            unsafe {
                GetWindowRect(child, &mut rect);
                windows_sys::Win32::Graphics::Gdi::ClientToScreen(window.hwnd, &mut origin);
            }
            (rect.top - origin.y, rect.bottom - rect.top)
        };
        let visible = |child| {
            (unsafe { GetWindowLongPtrW(child, super::GWL_STYLE) }) as u32 & super::WS_VISIBLE != 0
        };
        let dpi = unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForWindow(window.hwnd) }.max(96);
        let title_height = super::title_layout(window.hwnd).height;

        execute_command(window.hwnd, CommandId::Find);
        let panel = app_mut(window.hwnd).find_bar.as_ref().unwrap().panel_hwnd();
        assert!(visible(panel));
        let band = super::find_bar::find_bar_height(dpi);
        assert_eq!(top_of(panel), (title_height, band));
        assert_eq!(top_of(editor.hwnd()).0, title_height + band);

        let brush_before = {
            let bar = app_mut(window.hwnd).find_bar.as_ref().unwrap();
            bar.control_color(std::ptr::null_mut())
        };
        execute_command(window.hwnd, CommandId::ThemeCatppuccinMocha);
        let bar = app_mut(window.hwnd).find_bar.as_ref().unwrap();
        assert!(bar.is_visible());
        assert_ne!(bar.control_color(std::ptr::null_mut()), brush_before);

        assert_eq!(bar.placeholder(bar.query_hwnd()), Some("Find"));
        assert_eq!(bar.placeholder(bar.replace_hwnd()), Some("Replace"));

        // A click released on the close button at the bar's right end closes it.
        let mut client = RECT::default();
        unsafe { GetClientRect(panel, &mut client) };
        let point = |x: i32, y: i32| ((y as u32) << 16 | (x as u32 & 0xffff)) as super::LPARAM;
        let close_point = point(client.right - band / 2, band / 2);
        super::panel_pointer(
            window.hwnd,
            panel,
            windows_sys::Win32::UI::WindowsAndMessaging::WM_LBUTTONUP,
            point(client.right / 2, band / 2),
        );
        assert!(
            visible(panel),
            "a click on the field area must not close the bar"
        );
        super::panel_pointer(
            window.hwnd,
            panel,
            windows_sys::Win32::UI::WindowsAndMessaging::WM_LBUTTONUP,
            close_point,
        );
        assert!(!visible(panel));
        assert_eq!(top_of(editor.hwnd()).0, title_height);
    }

    #[test]
    fn with_no_tab_open_the_command_palette_lists_only_commands_that_need_no_document() {
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        install_test_editor(&window);
        execute_command(window.hwnd, CommandId::CloseAllTabs);
        assert!(app_mut(window.hwnd).tabs.is_empty());

        execute_command(window.hwnd, CommandId::CommandPalette);
        let shown = app_mut(window.hwnd)
            .command_palette
            .as_ref()
            .unwrap()
            .shown()
            .iter()
            .map(|entry| entry.command)
            .collect::<Vec<_>>();
        assert!(shown.iter().all(|command| !command.needs_document()));
        assert!(shown.contains(&CommandId::Open));
        assert!(shown.contains(&CommandId::ThemeDark));
        assert!(!shown.contains(&CommandId::Save));
    }

    #[test]
    fn setting_commands_apply_to_the_editor_and_save_only_their_own_ini_lines() {
        // Break caught: a palette setting that changes the editor but is lost on restart, or that
        // rewrites fastpad.ini and drops what the user wrote there by hand.
        use crate::editor::scintilla_constants::{
            SCI_GETTABWIDTH, SCI_STYLEGETBACK, STYLE_DEFAULT,
        };
        const SCI_GETWRAPMODE: u32 = 2269;
        const SCI_STYLEGETSIZEFRACTIONAL: u32 = 2062;
        let _scintilla = load_native_scintilla();
        let scratch = RecoveryScratch::new("settings");
        let ini = scratch.path().join("fastpad.ini");
        std::fs::write(&ini, "# kept\r\nfont_face=Cascadia Mono\r\ntab_width=4\r\n").unwrap();
        super::save_settings_to(Some(ini.clone()));
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        let send = |message, wparam| unsafe { SendMessageW(editor.hwnd(), message, wparam, 0) };

        execute_command(window.hwnd, CommandId::ToggleWordWrap);
        assert_ne!(send(SCI_GETWRAPMODE, 0), 0);
        execute_command(window.hwnd, CommandId::TabWidth8);
        assert_eq!(send(SCI_GETTABWIDTH, 0), 8);
        execute_command(window.hwnd, CommandId::FontSizeIncrease);
        execute_command(window.hwnd, CommandId::FontSizeIncrease);
        assert_eq!(
            send(SCI_STYLEGETSIZEFRACTIONAL, STYLE_DEFAULT as usize),
            1300
        );
        execute_command(window.hwnd, CommandId::FontSizeReset);
        assert_eq!(
            send(SCI_STYLEGETSIZEFRACTIONAL, STYLE_DEFAULT as usize),
            1100
        );
        execute_command(window.hwnd, CommandId::ToggleLineNumbers);
        assert!(!app_mut(window.hwnd).settings.line_numbers);

        super::build_chrome(window.hwnd);
        execute_command(window.hwnd, CommandId::ThemeDark);
        assert_eq!(
            send(SCI_STYLEGETBACK, STYLE_DEFAULT as usize) as u32,
            crate::window::palette::Palette::for_theme(crate::platform::theme::Theme::Dark, false,)
                .editor_background
        );
        execute_command(window.hwnd, CommandId::ThemeCatppuccinMocha);
        assert_eq!(
            send(SCI_STYLEGETBACK, STYLE_DEFAULT as usize) as u32,
            crate::window::palette::Palette::for_theme(
                crate::platform::theme::Theme::CatppuccinMocha,
                false,
            )
            .editor_background
        );
        execute_command(window.hwnd, CommandId::ThemeLight);
        assert_eq!(
            send(SCI_STYLEGETBACK, STYLE_DEFAULT as usize) as u32,
            crate::window::palette::Palette::for_theme(
                crate::platform::theme::Theme::Light,
                false,
            )
            .editor_background
        );
        super::save_settings_to(None);

        assert_eq!(
            std::fs::read_to_string(&ini).unwrap(),
            "# kept\r\nfont_face=Cascadia Mono\r\ntab_width=8\r\nword_wrap=true\r\n\
             font_size=11\r\nline_numbers=false\r\ntheme=light\r\n"
        );
        let (reloaded, warnings) = {
            let mut settings = crate::config::default_settings();
            let delta = crate::config::parse(&std::fs::read_to_string(&ini).unwrap());
            settings.apply_delta(&delta);
            (settings, delta.warnings)
        };
        assert!(warnings.is_empty());
        // The window never loaded this file, so only the hand-written font differs.
        assert_eq!(reloaded.font_face, "Cascadia Mono");
        assert_eq!(
            crate::config::Settings {
                font_face: app_mut(window.hwnd).settings.font_face.clone(),
                ..reloaded
            },
            app_mut(window.hwnd).settings
        );
    }

    #[test]
    fn session_toggle_saves_only_its_line_and_says_so() {
        // Break caught: a toggle that flips the flag but is lost on restart, rewrites the user's
        // fastpad.ini, or leaves no sign of which state it chose (menus show no checkmarks).
        let _scintilla = load_native_scintilla();
        let scratch = RecoveryScratch::new("session-toggle");
        let ini = scratch.path().join("fastpad.ini");
        std::fs::write(&ini, "# kept\r\n").unwrap();
        super::save_settings_to(Some(ini.clone()));
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        assert!(app_mut(window.hwnd).settings.restore_session);

        execute_command(window.hwnd, CommandId::ToggleRestoreSession);

        assert!(!app_mut(window.hwnd).settings.restore_session);
        assert_eq!(
            std::fs::read_to_string(&ini).unwrap(),
            "# kept\r\nrestore_session=false\r\n"
        );
        assert!(
            app_mut(window.hwnd)
                .notifications
                .pending()
                .iter()
                .any(|notice| notice.message == crate::session::toggle_notice(false))
        );
        super::save_settings_to(None);
    }

    #[test]
    fn notes_mode_toggle_saves_only_its_line_and_says_so() {
        // Break caught: a toggle lost on restart, or one that rewrites the rest of fastpad.ini.
        let _scintilla = load_native_scintilla();
        let scratch = RecoveryScratch::new("notes-toggle");
        let ini = scratch.path().join("fastpad.ini");
        std::fs::write(&ini, "# kept\r\n").unwrap();
        super::save_settings_to(Some(ini.clone()));
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        execute_command(window.hwnd, CommandId::ToggleNotesMode);
        assert!(!app_mut(window.hwnd).settings.notes_mode);
        assert_eq!(
            std::fs::read_to_string(&ini).unwrap(),
            "# kept\r\nnotes_mode=false\r\n"
        );
        assert!(
            notices(window.hwnd)
                .contains(&crate::window::library_host::notes_mode_notice(false).to_owned())
        );
        super::save_settings_to(None);
    }

    #[test]
    fn an_untitled_tab_is_labelled_by_its_first_line_as_you_type() {
        // Break caught: every untitled tab reading "Untitled", or the label recomputed on every
        // keystroke far below the first line.
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        super::create_new_document(window.hwnd).unwrap();
        editor.set_text("\n## Meeting notes\nbody").unwrap();
        pump_posted_messages(window.hwnd);
        let title = || app_mut(window.hwnd).tabs.active().unwrap().title();
        assert_eq!(title(), "Meeting notes *");
        assert_eq!(app_mut(window.hwnd).tabs.active().unwrap().label_watch, 1);

        app_mut(window.hwnd).settings.notes_mode = false;
        crate::window::library_host::clear_labels(window.hwnd);
        assert_eq!(title(), "Untitled *");
    }

    #[test]
    fn dragging_the_tab_scroll_thumb_scrolls_the_tabs_without_activating_one() {
        // Break caught: a scroll bar that is only painted, so overflowing tabs cannot be reached
        // without a mouse wheel, or a drag release that also clicks the tab under the pointer.
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE,
        };
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        for _ in 0..40 {
            execute_command(window.hwnd, CommandId::New);
        }
        super::activate_tab(window.hwnd, 0);
        let layout = super::title_layout(window.hwnd);
        assert_eq!(layout.scroll, 0);
        let thumb = layout.scroll_thumb().expect("40 tabs overflow the strip");
        let pack = |x: i32, y: i32| (x as u16 as u32 | ((y as u16 as u32) << 16)) as isize;
        let y = thumb.center().y;
        let far_right = layout.tabs.right + 500;

        unsafe {
            SendMessageW(window.hwnd, WM_LBUTTONDOWN, 1, pack(thumb.center().x, y));
            SendMessageW(window.hwnd, WM_MOUSEMOVE, 1, pack(far_right, y));
        }
        assert_eq!(super::tab_scroll(window.hwnd), layout.max_scroll);
        unsafe {
            SendMessageW(window.hwnd, WM_LBUTTONUP, 0, pack(far_right, y));
            SendMessageW(window.hwnd, WM_MOUSEMOVE, 0, pack(layout.tabs.left, y));
        }
        assert_eq!(
            super::tab_scroll(window.hwnd),
            layout.max_scroll,
            "moving after the release must not keep dragging"
        );
        assert_eq!(app_mut(window.hwnd).tabs.active_index(), 0);

        // Pressing the track away from the thumb jumps there.
        let track = super::title_layout(window.hwnd).scroll_bar.unwrap();
        unsafe {
            SendMessageW(window.hwnd, WM_LBUTTONDOWN, 1, pack(track.left, y));
            SendMessageW(window.hwnd, WM_LBUTTONUP, 0, pack(track.left, y));
        }
        assert_eq!(super::tab_scroll(window.hwnd), 0);
    }

    #[test]
    fn queued_ipc_requests_wait_for_the_modal_prompt_to_close() {
        // Break caught: a forwarded launch is dispatched inside a close prompt (opening tabs the
        // review never saw) or dropped entirely instead of staying queued.
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        editor.set_text("dirty").unwrap();
        app_mut(window.hwnd)
            .ipc_requests
            .push(crate::ipc::IpcRequest::New);
        let before = app_mut(window.hwnd).tabs.len();

        answer_next_close_prompt(|hwnd| {
            unsafe {
                SendMessageW(hwnd, crate::window::WM_FASTPAD_IPC_REQUEST, 0, 0);
            }
            CloseDecision::Cancel
        });
        execute_command(window.hwnd, CommandId::CloseTab);

        assert_eq!(
            app_mut(window.hwnd).tabs.len(),
            before,
            "a forwarded request was handled inside the modal loop"
        );
        assert_eq!(
            app_mut(window.hwnd).ipc_requests.len(),
            1,
            "the request must stay queued while a modal loop runs"
        );

        pump_posted_messages(window.hwnd);

        assert_eq!(app_mut(window.hwnd).tabs.len(), before + 1);
        assert!(app_mut(window.hwnd).ipc_requests.is_empty());
    }

    #[test]
    fn recovery_snapshot_ticks_are_skipped_inside_a_modal_prompt() {
        // Break caught: the recovery WM_TIMER fires inside a modal loop and swaps documents in and
        // out of the view under the operation the modal dialog is about to complete.
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        let root = RecoveryScratch::new("modal-snapshot");
        app_mut(window.hwnd).recovery_root = Some(root.path().to_path_buf());
        editor.set_text("typed").unwrap();
        let snapshot = snapshot_path(
            root.path(),
            app_mut(window.hwnd).tabs.active().unwrap().recovery_id,
        );

        answer_next_close_prompt(|hwnd| {
            super::snapshot_next_document(hwnd);
            CloseDecision::Cancel
        });
        execute_command(window.hwnd, CommandId::CloseTab);

        assert!(
            !snapshot.exists(),
            "a snapshot tick ran inside the modal loop"
        );

        super::snapshot_next_document(window.hwnd);

        assert!(snapshot.exists(), "snapshots must resume after the modal");
    }

    #[test]
    fn save_as_writes_the_document_chosen_before_the_dialog_opened() {
        // Break caught: the Save As dialog's modal loop activates another tab (a recovered one, a
        // forwarded open), and complete_save then renames and overwrites whatever is active now.
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        let root = RecoveryScratch::new("modal-save-as");
        let target = root.path().join("chosen.txt");
        editor.set_text("alpha").unwrap();
        let chosen = app_mut(window.hwnd).tabs.active().unwrap().id;
        let destination = target.clone();
        answer_next_save_dialog(move |hwnd| {
            super::create_new_document(hwnd).unwrap();
            Some(destination)
        });

        assert!(super::save_active_document_as(window.hwnd));

        assert_eq!(std::fs::read(&target).unwrap(), b"alpha");
        let app = app_mut(window.hwnd);
        assert_eq!(app.tabs.len(), 2);
        assert_eq!(app.tabs.active().unwrap().id, chosen);
        assert_eq!(
            app.tabs.document(chosen).unwrap().path.as_deref(),
            Some(target.as_path())
        );
    }

    #[test]
    fn folder_and_confirm_seams_answer_inside_their_modal_scope() {
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        crate::window::answer_next_folder_dialog(|hwnd| {
            assert!(crate::window::modal::modal_active(hwnd));
            Some(std::path::PathBuf::from(r"D:\Notes"))
        });
        assert_eq!(
            crate::window::modal::choose_folder(window.hwnd).unwrap(),
            Some(std::path::PathBuf::from(r"D:\Notes"))
        );
        crate::window::answer_next_confirm(|hwnd| {
            assert!(crate::window::modal::modal_active(hwnd));
            false
        });
        assert!(!crate::window::modal::confirm(window.hwnd, "Delete?"));
    }

    #[test]
    fn failed_language_activation_leaves_document_language_unchanged_and_records_a_warning() {
        // Break caught: a failed Lexilla load/lexer-creation must not record the requested
        // language on Document metadata when the editor itself was left exactly as it was
        // (LanguageManager::apply never installs a lexer before a real pointer is in hand), and
        // must surface something the caller can show in a notification instead of failing silently.
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let identity = unsafe { super::window_identity(window.hwnd).unwrap() };
        unsafe {
            super::initialize_editor_with(window.hwnd, &identity, crate::editor::Editor::create)
        }
        .unwrap();

        // Seed a LanguageManager pointed at a Lexilla.dll path that cannot possibly load, so the
        // activation below fails deterministically without depending on the real native DLL.
        let missing = std::env::temp_dir().join(format!(
            "fastpad-main-window-missing-lexilla-test-{}.dll",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&missing);
        unsafe {
            super::app_ptr(window.hwnd)
                .unwrap()
                .as_mut()
                .language_manager = Some(LanguageManager::with_dll_path_for_test(missing));
        }
        let language_before = unsafe { super::app_ptr(window.hwnd).unwrap().as_ref() }
            .tabs
            .active()
            .unwrap()
            .language;
        assert_eq!(language_before, Language::PlainText);

        execute_command(window.hwnd, CommandId::LanguageJson);

        assert_eq!(
            notices(window.hwnd),
            vec![
                "FastPad could not enable syntax highlighting for this file. It will remain in \
                 plain text."
                    .to_owned()
            ]
        );
        let language_after = unsafe { super::app_ptr(window.hwnd).unwrap().as_ref() }
            .tabs
            .active()
            .unwrap()
            .language;
        assert_eq!(language_after, Language::PlainText);
    }

    #[test]
    fn corrupt_settings_are_reported_on_the_bottom_bar_that_chrome_reserves() {
        // Break caught: nothing else asserts that invalid fastpad.ini lines actually reach the
        // user. If load_settings stopped queuing warnings, the painted bottom bar stopped showing
        // them, layout stopped reserving room for it, or dismissing a notice collapsed the bar,
        // every other test would still pass.
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let identity = unsafe { super::window_identity(window.hwnd).unwrap() };
        unsafe {
            super::initialize_editor_with(window.hwnd, &identity, crate::editor::Editor::create)
        }
        .unwrap();
        let editor_hwnd = unsafe { super::app_ptr(window.hwnd).unwrap().as_ref() }
            .editor
            .as_ref()
            .unwrap()
            .hwnd();

        // No bottom bar exists before WM_FASTPAD_BUILD_CHROME, regardless of pending warnings.
        assert_eq!(super::current_status_text(window.hwnd), None);
        assert_eq!(super::current_status_bar(window.hwnd), None);

        let warnings = vec![
            crate::config::SettingWarning {
                line: 3,
                message: "invalid value for tab_width: \"nope\"".to_owned(),
            },
            crate::config::SettingWarning {
                line: 5,
                message: "unknown setting key: bogus".to_owned(),
            },
        ];
        super::apply_loaded_settings(window.hwnd, crate::config::default_settings(), warnings);

        assert_eq!(
            unsafe { super::app_ptr(window.hwnd).unwrap().as_ref() }
                .notifications
                .len(),
            2
        );
        assert_eq!(
            super::current_status_text(window.hwnd),
            None,
            "chrome has not been built yet, so there is still nowhere to paint the warning"
        );

        super::build_chrome(window.hwnd);

        let status = super::current_status_text(window.hwnd);
        assert!(
            status
                .as_deref()
                .is_some_and(|text| text.contains("fastpad.ini line 3")),
            "expected the first warning's line reference in {status:?}"
        );
        let mut client = RECT::default();
        let mut shown = RECT::default();
        unsafe {
            GetClientRect(window.hwnd, &mut client);
            GetClientRect(editor_hwnd, &mut shown);
        }
        let dpi = unsafe { GetDpiForWindow(window.hwnd) };
        let title_height = super::title_layout(window.hwnd).height;
        assert_eq!(
            (client.bottom - client.top) - (shown.bottom - shown.top),
            title_height + crate::window::status::status_height(dpi),
            "the editor should leave exactly the bottom bar's height below it"
        );

        super::dismiss_notifications(window.hwnd);

        assert_eq!(super::current_status_text(window.hwnd), None);
        let bar = super::current_status_bar(window.hwnd).unwrap();
        assert_eq!(bar.left, "Ln 1, Col 1");
        assert_eq!(bar.right, "Plain Text    UTF-8");
        let mut dismissed = RECT::default();
        unsafe {
            GetClientRect(editor_hwnd, &mut dismissed);
        }
        assert_eq!(
            dismissed.bottom - dismissed.top,
            shown.bottom - shown.top,
            "the bottom bar stays after its notices are dismissed"
        );
    }

    fn line_number_margin_width(editor: &crate::editor::Editor) -> isize {
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::SendMessageW(
                editor.hwnd(),
                crate::editor::scintilla_constants::SCI_GETMARGINWIDTHN,
                0,
                0,
            )
        }
    }

    #[test]
    fn line_numbers_follow_edits_and_the_line_numbers_setting() {
        // Break caught: typing or pasting past line 99 without re-sizing the gutter clips the
        // numbers, and line_numbers=false in fastpad.ini leaving the gutter visible.
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        let two_digits = line_number_margin_width(&editor);
        assert!(two_digits > 0, "line numbers are shown by default");

        editor.replace_target(0..0, &"\n".repeat(150)).unwrap();
        let three_digits = line_number_margin_width(&editor);
        assert!(
            three_digits > two_digits,
            "151 lines need a wider gutter than {two_digits}px, got {three_digits}px"
        );

        editor.undo().unwrap();
        assert_eq!(line_number_margin_width(&editor), two_digits);

        let mut settings = crate::config::default_settings();
        settings.line_numbers = false;
        super::apply_loaded_settings(window.hwnd, settings, Vec::new());
        assert_eq!(line_number_margin_width(&editor), 0);
    }

    #[test]
    fn format_json_command_reformats_with_two_spaces_in_one_undo_step() {
        // Break caught: Format JSON not actually rewriting the buffer, or splitting the rewrite
        // into more than one undo action (which would force repeated Ctrl+Z to fully undo it).
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        editor.populate_clean("{\"a\":[1,2]}").unwrap();

        execute_command(window.hwnd, CommandId::FormatJson);

        assert_eq!(
            editor.text().unwrap(),
            "{\n  \"a\": [\n    1,\n    2\n  ]\n}"
        );
        assert!(notices(window.hwnd).is_empty());
        assert!(editor.can_undo().unwrap());
        editor.undo().unwrap();
        assert_eq!(editor.text().unwrap(), "{\"a\":[1,2]}");
        assert!(!editor.can_undo().unwrap());
    }

    #[test]
    fn format_json_command_on_invalid_json_leaves_bytes_unchanged_and_reports_an_issue() {
        // Break caught: Format JSON starting an undo action or mutating the buffer before
        // discovering the source does not parse, and/or swallowing the failure instead of
        // surfacing it.
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        editor.populate_clean("{ bad").unwrap();

        execute_command(window.hwnd, CommandId::FormatJson);

        assert_eq!(editor.text().unwrap(), "{ bad");
        assert!(!editor.can_undo().unwrap());
        let issues = notices(window.hwnd);
        assert_eq!(issues.len(), 1);
        assert!(issues[0].contains("line 1"), "{}", issues[0]);
    }

    #[test]
    fn format_json_command_clamps_the_restored_selection_to_the_new_shorter_length() {
        // Break caught: restoring the pre-format selection verbatim after formatting shrank the
        // document, leaving an out-of-range SCI_SETSEL instead of a clamped one.
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        let padding = " ".repeat(50);
        let source = format!("{{{padding}\"a\":1}}");
        editor.populate_clean(&source).unwrap();
        editor.set_selection(55..58).unwrap();

        execute_command(window.hwnd, CommandId::FormatJson);

        let formatted_len = editor.text().unwrap().len();
        assert!(formatted_len < 55, "expected formatting to shrink the text");
        assert_eq!(editor.selection().unwrap(), formatted_len..formatted_len);
    }

    #[test]
    fn format_json_command_snaps_the_restored_selection_to_a_utf8_char_boundary() {
        // Break caught: reusing a pre-format byte offset verbatim (once only clamped to the new
        // length) against the post-format text can land mid-character, since it has no guaranteed
        // relationship to character boundaries in the reformatted bytes. Here the caret sits at an
        // ordinary, valid boundary in the *compact* source (right after "héllo"'s closing quote);
        // at that exact raw byte offset, the *pretty-printed* text — which keeps "é"'s literal
        // two-byte UTF-8 encoding but reflows the surrounding whitespace — instead lands squarely
        // between "é"'s two bytes.
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        let source = "{\"a\":\"h\u{e9}llo\",\"b\":1}";
        let formatted = crate::languages::format_json(source).unwrap();

        let boundary_in_source = source.find("\",\"b\"").unwrap(); // right before the closing '"'
        assert!(source.is_char_boundary(boundary_in_source));

        let e_char_start = formatted.find('\u{e9}').unwrap();
        assert_eq!(
            boundary_in_source,
            e_char_start + 1,
            "test setup: expected the reused raw byte offset to land inside é's encoding"
        );
        assert!(!formatted.is_char_boundary(boundary_in_source));

        editor.populate_clean(source).unwrap();
        editor
            .set_selection(boundary_in_source..boundary_in_source)
            .unwrap();

        execute_command(window.hwnd, CommandId::FormatJson);

        assert_eq!(editor.text().unwrap(), formatted);
        let restored = editor.selection().unwrap();
        assert!(
            formatted.is_char_boundary(restored.start) && formatted.is_char_boundary(restored.end),
            "restored selection {restored:?} is not on a UTF-8 character boundary"
        );
        // Snapped backward to the boundary immediately before "é", not forward past it.
        assert_eq!(restored, e_char_start..e_char_start);
    }

    #[test]
    fn validate_json_command_reports_success_and_never_mutates_the_document() {
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        editor.populate_clean("{\"a\":1}").unwrap();

        execute_command(window.hwnd, CommandId::ValidateJson);

        let reported = notices(window.hwnd);
        assert_eq!(reported.len(), 1);
        assert!(reported[0].contains("valid JSON"), "{reported:?}");
        assert_eq!(editor.text().unwrap(), "{\"a\":1}");
        assert!(!editor.can_undo().unwrap());
    }

    #[test]
    fn validate_json_command_reports_the_line_and_column_for_invalid_json() {
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        editor.populate_clean("{\n  bad\n}").unwrap();

        execute_command(window.hwnd, CommandId::ValidateJson);

        let issues = notices(window.hwnd);
        assert_eq!(issues.len(), 1);
        assert!(
            issues[0].contains("line 2") && issues[0].contains("column 3"),
            "{}",
            issues[0]
        );
        assert_eq!(editor.text().unwrap(), "{\n  bad\n}");
    }

    #[test]
    fn recovery_discovery_opens_foreign_snapshots_as_dirty_recovered_tabs_with_one_notice() {
        // Break caught: recovered text opened clean, untitled-looking, without a notice, or this
        // process's own live snapshots reopened as duplicates.
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        let root = RecoveryScratch::new("discover");
        let source = write_snapshot(
            root.path(),
            &Snapshot::new(
                RecoveryId::from_u128(0x77),
                Some(PathBuf::from(r"C:\docs\notes.md")),
                Encoding::Utf16Le,
                "recovered body",
            ),
        )
        .unwrap();
        let own_id = app_mut(window.hwnd).allocate_recovery_id();
        let own = write_snapshot(
            root.path(),
            &Snapshot::new(own_id, None, Encoding::Utf8, "live"),
        )
        .unwrap();
        std::fs::write(root.path().join("torn.fps"), b"FPS1").unwrap();
        app_mut(window.hwnd).recovery_root = Some(root.path().to_path_buf());

        super::recover_snapshots(window.hwnd);

        let (tabs, title, dirty, path, encoding, origin, notices) = {
            let app = app_mut(window.hwnd);
            let active = app.tabs.active().unwrap();
            (
                app.tabs.len(),
                active.title(),
                active.dirty,
                active.path.clone(),
                active.encoding,
                active.recovery_origin.clone(),
                app.notifications.len(),
            )
        };
        assert_eq!(tabs, 2);
        assert_eq!(title, "Recovered: notes.md *");
        assert!(dirty);
        assert_eq!(path, None);
        assert_eq!(encoding, Encoding::Utf16Le);
        assert_eq!(origin.unwrap().snapshot_path, source);
        assert_eq!(notices, 1);
        assert_eq!(editor.text().unwrap(), "recovered body");
        assert_ne!(
            unsafe { SendMessageW(editor.hwnd(), SCI_GETMODIFY, 0, 0) },
            0,
            "Scintilla itself must treat the recovered text as unsaved"
        );
        assert!(source.exists() && own.exists());
        assert!(root.path().join("torn.fps.invalid").exists());
    }

    #[test]
    fn snapshots_write_one_changed_dirty_document_per_tick_without_disturbing_the_view() {
        // Break caught: several documents written per tick, unchanged generations rewritten, or an
        // inactive tab's snapshot swapping the visible document or selection.
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        let root = RecoveryScratch::new("tick");
        app_mut(window.hwnd).recovery_root = Some(root.path().to_path_buf());
        editor.set_text("alpha").unwrap();
        super::create_new_document(window.hwnd).unwrap();
        editor.set_text("beta\nline").unwrap();
        editor.set_selection(2..3).unwrap();
        let ids = app_mut(window.hwnd)
            .tabs
            .documents()
            .map(|document| document.recovery_id)
            .collect::<Vec<_>>();
        let first = snapshot_path(root.path(), ids[0]);
        let second = snapshot_path(root.path(), ids[1]);

        super::snapshot_next_document(window.hwnd);

        assert_eq!(read_snapshot_text(&first), "alpha");
        assert!(!second.exists());
        assert_eq!(editor.text().unwrap(), "beta\nline");
        assert_eq!(editor.selection().unwrap(), 2..3);
        assert!(app_mut(window.hwnd).last_snapshot_duration.is_some());

        super::snapshot_next_document(window.hwnd);
        assert_eq!(read_snapshot_text(&second), "beta\nline");

        std::fs::remove_file(&first).unwrap();
        std::fs::remove_file(&second).unwrap();
        super::snapshot_next_document(window.hwnd);
        assert!(!first.exists() && !second.exists());
    }

    #[test]
    fn saving_a_recovered_tab_never_touches_the_original_and_removes_its_snapshots() {
        // Break caught: Save silently writing the original path, or a saved recovered document
        // leaving snapshots that resurrect it after the next crash.
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        let root = RecoveryScratch::new("save");
        let original = root.path().join("original.txt");
        std::fs::write(&original, b"original").unwrap();
        let source = write_snapshot(
            root.path(),
            &Snapshot::new(
                RecoveryId::from_u128(0x99),
                Some(original.clone()),
                Encoding::Utf8,
                "recovered",
            ),
        )
        .unwrap();
        app_mut(window.hwnd).recovery_root = Some(root.path().to_path_buf());
        super::recover_snapshots(window.hwnd);
        editor.set_text("recovered and edited").unwrap();

        super::snapshot_next_document(window.hwnd);
        let own = snapshot_path(
            root.path(),
            app_mut(window.hwnd).tabs.active().unwrap().recovery_id,
        );
        assert_eq!(read_snapshot_text(&own), "recovered and edited");
        assert!(
            !source.exists(),
            "the stale source would reopen as a duplicate"
        );

        let target = root.path().join("saved.txt");
        super::save_path_as(window.hwnd, &target);

        assert_eq!(std::fs::read(&target).unwrap(), b"recovered and edited");
        assert_eq!(std::fs::read(&original).unwrap(), b"original");
        assert!(!own.exists());
        let app = app_mut(window.hwnd);
        assert_eq!(app.tabs.active().unwrap().title(), "saved.txt");
        assert_eq!(app.tabs.active().unwrap().recovery_origin, None);
    }

    #[test]
    fn clean_window_close_removes_this_sessions_snapshots() {
        // Break caught: snapshots outliving a clean exit and restoring tabs on the next launch.
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        let root = RecoveryScratch::new("close");
        app_mut(window.hwnd).recovery_root = Some(root.path().to_path_buf());
        editor.set_text("typed").unwrap();
        super::snapshot_next_document(window.hwnd);
        let own = snapshot_path(
            root.path(),
            app_mut(window.hwnd).tabs.active().unwrap().recovery_id,
        );
        assert!(own.exists());
        editor.set_save_point();

        unsafe {
            SendMessageW(window.hwnd, WM_CLOSE, 0, 0);
        }

        assert_eq!(unsafe { IsWindow(window.hwnd) }, 0);
        assert!(!own.exists());
    }

    #[test]
    fn undoing_a_recovered_tab_keeps_it_dirty_and_its_source_through_clean_close_cleanup() {
        // Break caught: undo reaching Scintilla's empty save point marks the recovered tab clean,
        // so closing skips the prompt and deletes the only copy of its text.
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        let root = RecoveryScratch::new("undo");
        let source = write_snapshot(
            root.path(),
            &Snapshot::new(
                RecoveryId::from_u128(0x55),
                None,
                Encoding::Utf8,
                "only copy",
            ),
        )
        .unwrap();
        app_mut(window.hwnd).recovery_root = Some(root.path().to_path_buf());
        super::recover_snapshots(window.hwnd);

        while editor.can_undo().unwrap() {
            editor.undo().unwrap();
        }

        assert_eq!(editor.text().unwrap(), "");
        assert_eq!(
            unsafe { SendMessageW(editor.hwnd(), SCI_GETMODIFY, 0, 0) },
            0,
            "test setup: Scintilla reached its save point"
        );
        let app = app_mut(window.hwnd);
        let active = app.tabs.active().unwrap().id;
        assert!(app.tabs.active().unwrap().dirty);
        assert_eq!(
            app.tabs.next_dirty_review(&[]).map(|review| review.id),
            Some(active),
            "window close must still prompt for the recovered tab"
        );
        super::remove_session_snapshots(window.hwnd, &[]);
        assert!(source.exists());
    }

    #[test]
    fn discovery_skips_snapshots_whose_owner_process_is_still_running() {
        // Break caught: a second instance opening, quarantining, or later deleting a live
        // instance's snapshots.
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        let root = RecoveryScratch::new("live-owner");
        let process_start = 0x5EED_0000_0000_0000 | u64::from(std::process::id());
        let live = RecoveryId::compose(process_start, 4_000_000_001, 1);
        let torn = snapshot_path(
            root.path(),
            RecoveryId::compose(process_start, 4_000_000_001, 2),
        );
        let owner = crate::recovery::create_owner_mutex(live).unwrap();
        let valid = write_snapshot(
            root.path(),
            &Snapshot::new(live, None, Encoding::Utf8, "live elsewhere"),
        )
        .unwrap();
        std::fs::write(&torn, b"FPS1").unwrap();
        app_mut(window.hwnd).recovery_root = Some(root.path().to_path_buf());

        super::recover_snapshots(window.hwnd);

        assert_eq!(app_mut(window.hwnd).tabs.len(), 1);
        assert!(valid.exists() && torn.exists());

        drop(owner);
        super::recover_snapshots(window.hwnd);

        assert_eq!(app_mut(window.hwnd).tabs.len(), 2);
        assert!(valid.exists());
        assert!(
            !torn.exists(),
            "a dead owner's torn snapshot is quarantined"
        );
    }

    fn app_mut<'a>(hwnd: HWND) -> &'a mut App {
        unsafe { super::app_ptr(hwnd).unwrap().as_mut() }
    }

    /// The non-modal notification messages currently queued on the window (spec 239).
    fn notices(hwnd: HWND) -> Vec<String> {
        app_mut(hwnd)
            .notifications
            .pending()
            .iter()
            .map(|notice| notice.message.clone())
            .collect()
    }

    fn read_snapshot_text(path: &std::path::Path) -> String {
        Snapshot::decode(&std::fs::read(path).unwrap())
            .unwrap()
            .text
    }

    struct RecoveryScratch(PathBuf);

    impl RecoveryScratch {
        fn new(label: &str) -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "fastpad-window-recovery-{label}-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &std::path::Path {
            &self.0
        }
    }

    impl Drop for RecoveryScratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Installs a real Scintilla editor onto `window` (mirroring
    /// `failed_language_activation_leaves_document_language_unchanged_and_records_a_warning`'s own
    /// setup) and returns it for direct `text`/`set_text`/`selection` calls in JSON command tests.
    fn install_test_editor(window: &ProductionWindow) -> crate::editor::Editor {
        let identity = unsafe { super::window_identity(window.hwnd).unwrap() };
        unsafe {
            super::initialize_editor_with(window.hwnd, &identity, crate::editor::Editor::create)
        }
        .unwrap();
        // What the first WM_SIZE does once `bootstrap::run` shows the window.
        super::layout_editor_and_find_bar(window.hwnd);
        unsafe { super::app_ptr(window.hwnd).unwrap().as_ref() }
            .editor
            .clone()
            .unwrap()
    }

    fn sidebar_windows(hwnd: HWND) -> (HWND, HWND) {
        let sidebar = app_mut(hwnd)
            .sidebar
            .as_ref()
            .expect("notes mode shows the sidebar");
        (sidebar.bar, sidebar.panel)
    }

    /// Moves the pointer onto the activity bar, which makes its tooltip.
    fn hover_bar(bar: HWND) {
        unsafe {
            SendMessageW(
                bar,
                windows_sys::Win32::UI::WindowsAndMessaging::WM_MOUSEMOVE,
                0,
                client_lparam(10, 60),
            );
        }
    }

    fn client_lparam(x: i32, y: i32) -> super::LPARAM {
        ((y as u32) << 16 | (x as u32 & 0xffff)) as super::LPARAM
    }

    /// `window`'s client point `x`, `y` as a screen-coordinate `lParam`, as WM_NCHITTEST gets it.
    fn screen_lparam(window: HWND, x: i32, y: i32) -> super::LPARAM {
        let mut point = windows_sys::Win32::Foundation::POINT { x, y };
        unsafe { windows_sys::Win32::Graphics::Gdi::ClientToScreen(window, &mut point) };
        client_lparam(point.x, point.y)
    }

    fn client_size(window: HWND) -> (i32, i32) {
        let mut rect = RECT::default();
        unsafe { GetClientRect(window, &mut rect) };
        (rect.right, rect.bottom)
    }

    /// `child`'s left edge in `parent`'s client coordinates.
    fn left_of(child: HWND, parent: HWND) -> i32 {
        use windows_sys::Win32::UI::WindowsAndMessaging::GetWindowRect;
        let mut rect = RECT::default();
        let mut origin = windows_sys::Win32::Foundation::POINT::default();
        unsafe {
            GetWindowRect(child, &mut rect);
            windows_sys::Win32::Graphics::Gdi::ClientToScreen(parent, &mut origin);
        }
        rect.left - origin.x
    }

    /// The test window is never shown, so check the child's own style bit.
    fn is_shown(window: HWND) -> bool {
        (unsafe { GetWindowLongPtrW(window, super::GWL_STYLE) }) as u32 & super::WS_VISIBLE != 0
    }

    fn click(window: HWND, x: i32, y: i32) {
        use windows_sys::Win32::UI::WindowsAndMessaging::{WM_LBUTTONDOWN, WM_LBUTTONUP};
        unsafe {
            SendMessageW(window, WM_LBUTTONDOWN, 0, client_lparam(x, y));
            SendMessageW(window, WM_LBUTTONUP, 0, client_lparam(x, y));
        }
    }

    fn button_center(
        hwnd: HWND,
        button: crate::window::activity_bar::ActivityButton,
    ) -> (i32, i32) {
        let (bar, _) = sidebar_windows(hwnd);
        let (width, height) = client_size(bar);
        let client = RECT {
            left: 0,
            top: 0,
            right: width,
            bottom: height,
        };
        let dpi = unsafe { GetDpiForWindow(bar) }.max(96);
        let rect = crate::window::activity_bar::button_rects(client, dpi)[button.index()];
        ((rect.left + rect.right) / 2, (rect.top + rect.bottom) / 2)
    }

    /// Resizes the window so its client area is `client_width` wide.
    fn set_client_width(hwnd: HWND, client_width: i32) {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GetWindowRect, SWP_NOMOVE, SWP_NOZORDER, SetWindowPos,
        };
        let mut frame = RECT::default();
        unsafe { GetWindowRect(hwnd, &mut frame) };
        let border = (frame.right - frame.left) - client_size(hwnd).0;
        unsafe {
            SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                0,
                0,
                client_width + border,
                frame.bottom - frame.top,
                SWP_NOMOVE | SWP_NOZORDER,
            );
        }
    }

    /// A scratch `fastpad.ini` holding only a comment, which settings saves go to.
    fn settings_scratch(label: &str) -> (RecoveryScratch, PathBuf) {
        let scratch = RecoveryScratch::new(label);
        let ini = scratch.path().join("fastpad.ini");
        std::fs::write(&ini, "# kept\r\n").unwrap();
        super::save_settings_to(Some(ini.clone()));
        (scratch, ini)
    }

    #[test]
    fn with_notes_mode_off_there_is_no_sidebar_and_nothing_moves() {
        // Break caught: an activity bar, or a gap where it would be, with notes mode off, where
        // the layout must stay exactly what it was before the sidebar existed.
        let _scintilla = load_native_scintilla();
        let mut app = make_app();
        app.settings.notes_mode = false;
        let window = ProductionWindow::new(app);
        let editor = install_test_editor(&window);
        assert!(app_mut(window.hwnd).sidebar.is_none());
        assert_eq!(crate::window::side_panel::left_edge(window.hwnd), 0);
        assert_eq!(super::title_layout(window.hwnd).tab(0).left, 0);
        assert_eq!(left_of(editor.hwnd(), window.hwnd), 0);
        assert_eq!(
            crate::window::side_panel::current_view(window.hwnd),
            crate::config::SidebarView::Hidden
        );
        execute_command(window.hwnd, CommandId::ToggleSidebar);
        assert!(app_mut(window.hwnd).sidebar.is_none());
        execute_command(window.hwnd, CommandId::CommandPalette);
        assert!(
            app_mut(window.hwnd)
                .command_palette
                .as_ref()
                .unwrap()
                .shown()
                .iter()
                .all(|entry| !entry.command.is_sidebar())
        );
        super::close_command_palette(window.hwnd, false);

        app_mut(window.hwnd).settings.notes_mode = true;
        crate::window::side_panel::notes_mode_changed(window.hwnd, true);
        let (bar, _) = sidebar_windows(window.hwnd);
        hover_bar(bar);
        let tip = app_mut(window.hwnd)
            .sidebar
            .as_ref()
            .and_then(|sidebar| sidebar.tooltip)
            .expect("the activity bar has a tooltip")
            .hwnd();
        let left = crate::window::side_panel::left_edge(window.hwnd);
        assert!(left > 0);
        assert_eq!(left_of(editor.hwnd(), window.hwnd), left);

        app_mut(window.hwnd).settings.notes_mode = false;
        crate::window::side_panel::notes_mode_changed(window.hwnd, false);
        assert_eq!(unsafe { IsWindow(bar) }, 0);
        // Break caught: a tooltip left alive (owned by the main window, not the bar) each time
        // notes mode goes off.
        assert_eq!(unsafe { IsWindow(tip) }, 0);
        assert_eq!(crate::window::side_panel::left_edge(window.hwnd), 0);
        assert_eq!(left_of(editor.hwnd(), window.hwnd), 0);
    }

    #[test]
    fn the_sidebar_takes_the_left_edge_and_everything_else_starts_right_of_it() {
        // Break caught: tabs, the find bar, the menu band or the editor still starting at x = 0,
        // under the activity bar and panel.
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        let (bar, panel) = sidebar_windows(window.hwnd);
        let dpi = unsafe { GetDpiForWindow(window.hwnd) }.max(96);
        let (width, height) = client_size(window.hwnd);
        let saved = app_mut(window.hwnd).settings.sidebar_width;
        let (activity, panel_width) =
            crate::window::side_panel::sidebar_widths(width, dpi, true, saved);
        assert_eq!(client_size(bar), (activity, height));
        assert_eq!(client_size(panel), (panel_width, height));
        assert_eq!(left_of(panel, window.hwnd), activity);
        let left = activity + panel_width;
        assert_eq!(crate::window::side_panel::left_edge(window.hwnd), left);
        assert_eq!(super::title_layout(window.hwnd).tab(0).left, left);
        assert_eq!(left_of(editor.hwnd(), window.hwnd), left);
        assert_eq!(client_size(editor.hwnd()).0, width - left);
        // Nothing before the pointer needs the tooltip, so the first frame goes without it.
        assert!(
            app_mut(window.hwnd)
                .sidebar
                .as_ref()
                .unwrap()
                .tooltip
                .is_none()
        );
        hover_bar(bar);
        assert_eq!(
            app_mut(window.hwnd)
                .sidebar
                .as_ref()
                .unwrap()
                .tooltip
                .unwrap()
                .tool_count(),
            4
        );
        // An empty text removes a tool instead of showing an empty tip.
        let tooltip = app_mut(window.hwnd)
            .sidebar
            .as_ref()
            .unwrap()
            .tooltip
            .unwrap();
        tooltip.set_tool(9, RECT::default(), "extra");
        assert_eq!(tooltip.tool_count(), 5);
        tooltip.set_tool(9, RECT::default(), "");
        assert_eq!(tooltip.tool_count(), 4);

        execute_command(window.hwnd, CommandId::Find);
        let find = app_mut(window.hwnd).find_bar.as_ref().unwrap().panel_hwnd();
        assert_eq!(left_of(find, window.hwnd), left);
        assert_eq!(client_size(find).0, width - left);
        super::close_find_bar(window.hwnd);

        execute_command(window.hwnd, CommandId::CommandPalette);
        let palette = app_mut(window.hwnd)
            .command_palette
            .as_ref()
            .unwrap()
            .panel_hwnd();
        assert!(left_of(palette, window.hwnd) >= left);
        super::close_command_palette(window.hwnd, false);

        unsafe {
            SendMessageW(
                window.hwnd,
                super::WM_SYSCOMMAND,
                super::SC_KEYMENU as usize,
                0,
            )
        };
        assert_eq!(
            super::menu_headings(window.hwnd)[0].left,
            left + crate::window::panel::scale(4, dpi)
        );
        unsafe {
            SendMessageW(
                window.hwnd,
                super::WM_SYSCOMMAND,
                super::SC_KEYMENU as usize,
                0,
            )
        };
    }

    #[test]
    fn the_sidebar_top_strip_and_panel_header_are_caption() {
        // Break caught: child windows under the title row that swallow the caption, so the
        // window can no longer be dragged or top-resized there, or a lost left-border resize.
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GetWindowRect, HTCAPTION, HTCLIENT, HTLEFT, HTTRANSPARENT, WM_NCHITTEST,
        };
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        let (bar, panel) = sidebar_windows(window.hwnd);
        let dpi = unsafe { GetDpiForWindow(window.hwnd) }.max(96);
        let layout = super::title_layout(window.hwnd);
        let hit = |target: HWND, x: i32, y: i32| unsafe {
            SendMessageW(target, WM_NCHITTEST, 0, screen_lparam(target, x, y))
        };
        // Below the top resize band and above the first button.
        let strip_y = layout.height - 2;
        assert!(strip_y >= layout.resize_border);
        let bar_x = client_size(bar).0 / 2;
        assert_eq!(hit(bar, bar_x, strip_y), HTTRANSPARENT as LRESULT);
        assert_eq!(hit(window.hwnd, bar_x, strip_y), HTCAPTION as LRESULT);
        let (button_x, button_y) = button_center(
            window.hwnd,
            crate::window::activity_bar::ActivityButton::Notebook,
        );
        assert_eq!(hit(bar, button_x, button_y), HTCLIENT as LRESULT);

        let header_y = layout.resize_border + 2;
        let header = crate::window::panel::scale(crate::window::side_panel::HEADER_HEIGHT_96, dpi);
        assert!(header_y < header);
        // Left of the Notebook header's title (which starts 12 px in and stays client area, so
        // its tooltip works): empty header, a drag area.
        let panel_x = crate::window::panel::scale(4, dpi);
        assert_eq!(
            hit(panel, client_size(panel).0 / 2, header_y),
            HTCLIENT as LRESULT,
            "the notebook's name is not a drag area"
        );
        assert_eq!(hit(panel, panel_x, header_y), HTTRANSPARENT as LRESULT);
        assert_eq!(
            hit(window.hwnd, left_of(panel, window.hwnd) + panel_x, header_y),
            HTCAPTION as LRESULT
        );
        // Below the header, and on the resize edge, the panel keeps its own input.
        assert_eq!(hit(panel, panel_x, header + 10), HTCLIENT as LRESULT);
        assert_eq!(
            hit(panel, client_size(panel).0 - 1, header_y),
            HTCLIENT as LRESULT
        );

        // The left border is outside the client area, so no child covers it.
        let mut frame = RECT::default();
        let mut origin = windows_sys::Win32::Foundation::POINT::default();
        unsafe {
            GetWindowRect(window.hwnd, &mut frame);
            windows_sys::Win32::Graphics::Gdi::ClientToScreen(window.hwnd, &mut origin);
        }
        if origin.x > frame.left {
            let border = client_lparam(
                frame.left + (origin.x - frame.left) / 2,
                origin.y + client_size(window.hwnd).1 / 2,
            );
            assert_eq!(
                unsafe { SendMessageW(window.hwnd, WM_NCHITTEST, 0, border) },
                HTLEFT as LRESULT
            );
        }
    }

    #[test]
    fn clicking_the_active_view_icon_closes_the_sidebar_panel_and_saves_none() {
        // Break caught: an icon that only ever opens its view, so the mouse cannot close the
        // panel, or a closed panel that reopens after a restart.
        use crate::config::SidebarView;
        use crate::window::activity_bar::ActivityButton;
        let _scintilla = load_native_scintilla();
        let (_scratch, ini) = settings_scratch("sidebar-click");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        let (bar, panel) = sidebar_windows(window.hwnd);
        let activity = client_size(bar).0;
        let view = || crate::window::side_panel::current_view(window.hwnd);
        let saved = || std::fs::read_to_string(&ini).unwrap();

        let (x, y) = button_center(window.hwnd, ActivityButton::Notebook);
        click(bar, x, y);
        assert_eq!(view(), SidebarView::Hidden);
        assert!(!is_shown(panel));
        assert_eq!(crate::window::side_panel::left_edge(window.hwnd), activity);
        assert_eq!(saved(), "# kept\r\nsidebar_view=none\r\n");

        click(bar, x, y);
        assert_eq!(view(), SidebarView::Notebook);
        assert!(is_shown(panel));
        assert_eq!(saved(), "# kept\r\nsidebar_view=notebook\r\n");

        let (x, y) = button_center(window.hwnd, ActivityButton::Search);
        click(bar, x, y);
        assert_eq!(view(), SidebarView::Search);
        assert_eq!(saved(), "# kept\r\nsidebar_view=search\r\n");

        // Settings opens the command palette listing only the settings commands.
        let (x, y) = button_center(window.hwnd, ActivityButton::Settings);
        click(bar, x, y);
        let palette = app_mut(window.hwnd).command_palette.as_ref().unwrap();
        assert!(palette.is_visible());
        assert_eq!(
            palette.subset(),
            Some(crate::window::command_palette::SETTINGS_COMMANDS)
        );
        assert_eq!(view(), SidebarView::Search);
        super::save_settings_to(None);
    }

    #[test]
    fn ctrl_b_toggles_the_sidebar_back_to_the_last_view_and_saves_each_change() {
        // Break caught: a toggle that forgets which view was open, a shortcut that never
        // reaches its command, or a change lost on restart.
        use crate::config::SidebarView;
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
            GetKeyboardState, SetKeyboardState, VK_CONTROL, VK_SHIFT,
        };
        use windows_sys::Win32::UI::WindowsAndMessaging::{MSG, WM_KEYDOWN};
        let _scintilla = load_native_scintilla();
        let (_scratch, ini) = settings_scratch("sidebar-ctrl-b");
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        let identity = unsafe { super::window_identity(window.hwnd).unwrap() };
        let press = |key: u8, shift: bool| {
            let mut keys = [0u8; 256];
            unsafe { GetKeyboardState(keys.as_mut_ptr()) };
            let original = keys;
            keys[VK_CONTROL as usize] = 0x80;
            keys[VK_SHIFT as usize] = if shift { 0x80 } else { 0 };
            unsafe { SetKeyboardState(keys.as_ptr()) };
            let message = MSG {
                hwnd: editor.hwnd(),
                message: WM_KEYDOWN,
                wParam: usize::from(key),
                ..Default::default()
            };
            let translated =
                unsafe { super::translate_accelerator(window.hwnd, &identity, &message) };
            unsafe { SetKeyboardState(original.as_ptr()) };
            translated
        };
        let view = || crate::window::side_panel::current_view(window.hwnd);
        let saved = || std::fs::read_to_string(&ini).unwrap();

        assert_eq!(view(), SidebarView::Notebook);
        assert!(press(b'B', false));
        assert_eq!(view(), SidebarView::Hidden);
        assert_eq!(saved(), "# kept\r\nsidebar_view=none\r\n");
        assert!(press(b'K', false));
        assert_eq!(view(), SidebarView::Search);
        assert!(press(b'B', false));
        assert!(press(b'B', false));
        assert_eq!(view(), SidebarView::Search, "Ctrl+B reopens the last view");
        assert!(press(b'E', true));
        assert_eq!(view(), SidebarView::Notebook);
        execute_command(window.hwnd, CommandId::ShowFavoritesView);
        assert_eq!(view(), SidebarView::Favorites);
        assert_eq!(saved(), "# kept\r\nsidebar_view=favorites\r\n");
        super::save_settings_to(None);
    }

    #[test]
    fn a_narrow_window_squeezes_the_panel_without_saving_it() {
        // Break caught: an editor pushed below its 320 px minimum, a negative panel width, or a
        // squeeze written to fastpad.ini so the panel stays narrow once the window widens again.
        let _scintilla = load_native_scintilla();
        let (_scratch, ini) = settings_scratch("sidebar-squeeze");
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        let (_, panel) = sidebar_windows(window.hwnd);
        let dpi = unsafe { GetDpiForWindow(window.hwnd) }.max(96);
        let scale = |value| crate::window::panel::scale(value, dpi);

        set_client_width(window.hwnd, scale(44 + 320 + 200));
        let (width, _) = client_size(window.hwnd);
        let (activity, squeezed) = crate::window::side_panel::sidebar_widths(width, dpi, true, 260);
        assert_eq!(squeezed, width - activity - scale(320));
        assert!(
            squeezed > 0 && squeezed < scale(260),
            "the window squeezes the panel"
        );
        assert_eq!(client_size(panel).0, squeezed);
        assert_eq!(
            crate::window::side_panel::left_edge(window.hwnd),
            activity + squeezed
        );
        assert_eq!(
            client_size(editor.hwnd()).0,
            scale(320).max(width - activity - squeezed)
        );
        assert_eq!(app_mut(window.hwnd).settings.sidebar_width, 260);

        set_client_width(window.hwnd, scale(1200));
        assert_eq!(client_size(panel).0, scale(260));

        // Narrower than the activity bar and the editor minimum: the panel hides, never goes
        // below zero.
        set_client_width(window.hwnd, scale(300));
        assert!(!is_shown(panel));
        assert_eq!(
            crate::window::side_panel::left_edge(window.hwnd),
            scale(44).min(client_size(window.hwnd).0)
        );
        assert_eq!(app_mut(window.hwnd).settings.sidebar_width, 260);
        assert_eq!(std::fs::read_to_string(&ini).unwrap(), "# kept\r\n");
        super::save_settings_to(None);
    }

    #[test]
    fn dragging_the_sidebar_edge_resizes_it_and_saves_the_width_once_on_release() {
        // Break caught: a drag that writes fastpad.ini on every mouse move, never saves, ignores
        // the 180–480 range, or an edge double-click that leaves a custom width in place.
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE,
        };
        let _scintilla = load_native_scintilla();
        let (_scratch, ini) = settings_scratch("sidebar-drag");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        let (_, panel) = sidebar_windows(window.hwnd);
        let dpi = unsafe { GetDpiForWindow(window.hwnd) }.max(96);
        let scale = |value| crate::window::panel::scale(value, dpi);
        set_client_width(window.hwnd, scale(1400));
        let saved = || std::fs::read_to_string(&ini).unwrap();
        let send = |message, x: i32| unsafe {
            SendMessageW(
                panel,
                message,
                0,
                client_lparam(x, client_size(panel).1 / 2),
            );
        };

        send(WM_LBUTTONDOWN, client_size(panel).0 - 1);
        send(WM_MOUSEMOVE, scale(300));
        assert_eq!(client_size(panel).0, scale(300));
        assert_eq!(saved(), "# kept\r\n", "nothing is saved mid-drag");
        send(WM_LBUTTONUP, scale(300));
        assert_eq!(saved(), "# kept\r\nsidebar_width=300\r\n");
        assert_eq!(app_mut(window.hwnd).settings.sidebar_width, 300);

        send(WM_LBUTTONDOWN, client_size(panel).0 - 1);
        send(WM_MOUSEMOVE, scale(900));
        send(WM_LBUTTONUP, scale(900));
        assert_eq!(saved(), "# kept\r\nsidebar_width=480\r\n");
        assert_eq!(client_size(panel).0, scale(480));

        send(WM_LBUTTONDBLCLK, client_size(panel).0 - 1);
        assert_eq!(saved(), "# kept\r\nsidebar_width=260\r\n");
        assert_eq!(client_size(panel).0, scale(260));
        super::save_settings_to(None);
    }

    #[test]
    fn the_editor_reads_single_lines_without_their_line_endings() {
        // Break caught: a line reader that keeps the CR/LF, misreads Scintilla's line count, or
        // panics instead of returning empty text past the last line.
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        editor.set_text("first\r\nsecond\nthird").unwrap();
        assert_eq!(editor.line_count().unwrap(), 3);
        assert_eq!(editor.line_text(0).unwrap(), "first");
        assert_eq!(editor.line_text(1).unwrap(), "second");
        assert_eq!(editor.line_text(2).unwrap(), "third");
        assert_eq!(editor.line_text(9).unwrap(), "");
    }

    #[test]
    fn the_editor_reads_multi_byte_utf8_lines_without_their_line_endings() {
        // Break caught: a byte-length-based line reader splitting or corrupting a multi-byte
        // UTF-8 character at the line boundary instead of returning the line whole.
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        editor.set_text("h\u{e9}llo \u{1f600}\r\nsecond").unwrap();
        assert_eq!(editor.line_text(0).unwrap(), "h\u{e9}llo \u{1f600}");
    }

    #[test]
    fn create_context_drops_untransferred_value_on_pre_window_failure() {
        // Break caught: bootstrap manually reclaiming a create-time App allocation is unsafe once
        // ownership can also transfer through WM_NCCREATE.
        let drops = Arc::new(AtomicUsize::new(0));
        {
            let _context = WindowCreateContext::new(Box::new(DropProbe::new(Arc::clone(&drops))));
        }

        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn production_nc_create_transfers_app_and_nc_destroy_clears_the_window() {
        // Break caught: bypassing the real WM_NCCREATE/WM_NCDESTROY ownership path can leave the
        // production window without App state or leave the HWND alive after teardown.
        let window = ProductionWindow::new(make_app());
        assert_ne!(unsafe { GetWindowLongPtrW(window.hwnd, GWLP_USERDATA) }, 0);
        unsafe {
            DestroyWindow(window.hwnd);
        }
        assert_eq!(unsafe { IsWindow(window.hwnd) }, 0);
    }

    #[test]
    fn initial_editor_installation_updates_a_retained_empty_tab_view() {
        // Break caught: accessibility requested before editor creation retains an obsolete view.
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let view = unsafe { super::app_ptr(window.hwnd).unwrap().as_ref() }
            .tabs
            .view();
        assert!(view.snapshot().tabs.is_empty());
        let identity = unsafe { super::window_identity(window.hwnd).unwrap() };
        unsafe {
            super::initialize_editor_with(window.hwnd, &identity, crate::editor::Editor::create)
        }
        .unwrap();
        assert_eq!(view.snapshot().tabs.len(), 1);
    }

    #[test]
    fn native_wm_close_releases_all_owned_documents_before_editor_destruction() {
        // Break caught: clearing tabs after DestroyWindow skips real releases at the dead endpoint.
        if std::env::var_os("FASTPAD_REQUIRE_APPVERIF").is_some() {
            let verifier = crate::platform::wide_null("verifier.dll");
            assert!(
                !unsafe { GetModuleHandleW(verifier.as_ptr()) }.is_null(),
                "Application Verifier must actually be loaded for a claimed verifier run"
            );
        }
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let identity = unsafe { super::window_identity(window.hwnd).unwrap() };
        let editor = unsafe {
            super::initialize_editor_with(window.hwnd, &identity, crate::editor::Editor::create)
        }
        .unwrap();
        super::create_new_document(window.hwnd).unwrap();
        let (_, releases) = crate::editor::scintilla::release_observation::during(|| unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::SendMessageW(
                window.hwnd,
                windows_sys::Win32::UI::WindowsAndMessaging::WM_CLOSE,
                0,
                0,
            )
        });
        assert_eq!(releases.len(), 2);
        assert_ne!(releases[0].document, releases[1].document);
        assert!(
            releases
                .iter()
                .all(|release| release.hwnd == editor && release.window_was_live)
        );
        assert_eq!(unsafe { IsWindow(editor) }, 0);
        assert_eq!(unsafe { IsWindow(window.hwnd) }, 0);
    }

    #[test]
    fn original_window_identity_stays_invalid_after_replacement_creation() {
        // Break caught: an IsWindow-only liveness check can accept a recycled HWND and read the
        // replacement window's GWLP_USERDATA as the original App.
        let original_app = make_app();
        let original_identity = original_app.window_identity();
        let original = ProductionWindow::new(original_app);
        assert!(original_identity.is_live_for(original.hwnd));

        unsafe {
            DestroyWindow(original.hwnd);
        }
        assert!(original_identity.is_invalidated());
        drop(original);

        let replacement_app = make_app();
        let replacement_identity = replacement_app.window_identity();
        let replacement = ProductionWindow::new(replacement_app);
        assert!(replacement_identity.is_live_for(replacement.hwnd));
        assert!(original_identity.is_invalidated());
        assert!(!original_identity.is_live_for(replacement.hwnd));
    }

    #[test]
    fn reentrant_paint_completion_does_not_mutate_replacement_app() {
        // Break caught: removing the post-DefWindowProc identity gate lets an old WM_PAINT
        // completion mutate the App found in a recycled HWND's replacement GWLP_USERDATA slot.
        const PAINT_RESULT: LRESULT = 73;
        let mut original = Some(ProductionWindow::new(make_app()));
        let original_hwnd = original.as_ref().unwrap().hwnd;
        let replacement = RefCell::new(None::<ProductionWindow>);

        let default_window_proc = |hwnd, _, _, _| {
            assert_ne!(unsafe { DestroyWindow(hwnd) }, 0);
            drop(original.take());
            replacement.replace(Some(ProductionWindow::new(make_app())));
            PAINT_RESULT
        };
        let complete_first_paint = |_| {
            let replacement = replacement.borrow();
            let replacement = replacement.as_ref().unwrap();
            unsafe {
                mark_first_paint_complete(replacement.hwnd);
            }
        };
        let result = unsafe {
            handle_paint_with(
                original_hwnd,
                WM_PAINT,
                0,
                0,
                default_window_proc,
                complete_first_paint,
            )
        };

        assert_eq!(result, PAINT_RESULT);
        let replacement = replacement.borrow();
        let replacement = replacement.as_ref().unwrap();
        assert!(!unsafe { take_deferred_start_pending(replacement.hwnd) });
    }

    fn unnamed_mutex() -> crate::platform::OwnedHandle {
        let raw = unsafe {
            windows_sys::Win32::System::Threading::CreateMutexW(
                std::ptr::null(),
                0,
                std::ptr::null(),
            )
        };
        unsafe { crate::platform::OwnedHandle::from_raw_owned(raw) }.unwrap()
    }

    #[test]
    fn ipc_bind_failure_releases_the_instance_mutex_and_notifies_exactly_once() {
        // Break caught: keeping the mutex after a failed bind makes every later launch wait on a
        // pipe that will never exist; retrying or re-notifying spams the status line.
        let window = ProductionWindow::new(make_app());
        unsafe { super::app_ptr(window.hwnd).unwrap().as_mut() }.instance_mutex =
            Some(unnamed_mutex());

        super::start_ipc_server_with(window.hwnd, || {
            Err(crate::FastPadError::Ipc("simulated bind failure"))
        });
        super::start_ipc_server_with(window.hwnd, || unreachable!("no mutex means no server"));

        let app = unsafe { super::app_ptr(window.hwnd).unwrap().as_ref() };
        assert!(app.ipc.is_none());
        assert!(app.instance_mutex.is_none());
        assert_eq!(app.notifications.len(), 1);
    }

    #[test]
    fn process_without_instance_mutex_never_binds_a_server() {
        // Break caught: a --new-window or fallback process squats the primary's pipe name.
        let window = ProductionWindow::new(make_app());
        super::start_ipc_server_with(window.hwnd, || unreachable!("no mutex means no server"));
        let app = unsafe { super::app_ptr(window.hwnd).unwrap().as_ref() };
        assert!(app.ipc.is_none());
        assert_eq!(app.notifications.len(), 0);
    }

    fn deliver_frame(window: &ProductionWindow, names: &crate::ipc::InstanceNames, frame: Vec<u8>) {
        use std::time::{Duration, Instant};
        use windows_sys::Win32::Foundation::WAIT_OBJECT_0;
        use windows_sys::Win32::System::Threading::WaitForSingleObject;
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            DispatchMessageW, MSG, PM_REMOVE, PeekMessageW, TranslateMessage,
        };
        let pipe = names.clone();
        let client = std::thread::spawn(move || {
            crate::ipc::client::send_frame(&pipe, &frame, Duration::from_secs(2))
        });
        let identity = unsafe { super::window_identity(window.hwnd).unwrap() };
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut quiet_since = None;
        while Instant::now() < deadline {
            let event = super::ipc_wait_handle(window.hwnd, &identity).unwrap();
            if unsafe { WaitForSingleObject(event, 20) } == WAIT_OBJECT_0 {
                super::service_ipc(window.hwnd, &identity);
                quiet_since = None;
            } else if client.is_finished() {
                let since = *quiet_since.get_or_insert_with(Instant::now);
                if since.elapsed() >= Duration::from_millis(150) {
                    break;
                }
            }
            let mut message = MSG::default();
            while unsafe { PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_REMOVE) } != 0
            {
                unsafe {
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
        }
        client.join().unwrap().unwrap();
    }

    #[test]
    fn ipc_requests_reach_the_window_as_tabs_and_malformed_frames_change_nothing() {
        // Break caught: decoded requests never leave the pipe, duplicate opens add tabs, Activate
        // mutates tabs, or a malformed frame reaches application state.
        use crate::ipc::{IpcRequest, encode_frame};
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        unsafe {
            SendMessageW(
                editor.hwnd(),
                windows_sys::Win32::UI::WindowsAndMessaging::WM_CHAR,
                b'x' as usize,
                0,
            );
        }
        unsafe { super::app_ptr(window.hwnd).unwrap().as_mut() }.instance_mutex =
            Some(unnamed_mutex());
        let names = crate::ipc::server::tests::unique_names();
        super::start_ipc_server_with(window.hwnd, || {
            crate::ipc::IpcServer::bind(&names, &crate::ipc::CurrentUserAcl::current()?)
        });
        let scratch = RecoveryScratch::new("ipc-open");
        let file = scratch.path().join("forwarded.txt");
        std::fs::write(&file, b"forwarded text").unwrap();
        let tabs = || {
            unsafe { super::app_ptr(window.hwnd).unwrap().as_ref() }
                .tabs
                .len()
        };

        let open = encode_frame(&IpcRequest::Open(file.clone())).unwrap();
        deliver_frame(&window, &names, open.clone());
        assert_eq!(tabs(), 2);
        assert_eq!(editor.text().unwrap(), "forwarded text");
        deliver_frame(&window, &names, open);
        assert_eq!(tabs(), 2);
        deliver_frame(
            &window,
            &names,
            encode_frame(&IpcRequest::Activate).unwrap(),
        );
        assert_eq!(tabs(), 2);
        deliver_frame(&window, &names, b"FPI1\x09\0\0\0\0".to_vec());
        assert_eq!(tabs(), 2);
        deliver_frame(&window, &names, encode_frame(&IpcRequest::New).unwrap());
        assert_eq!(tabs(), 3);
        assert!(
            unsafe { super::app_ptr(window.hwnd).unwrap().as_ref() }
                .ipc_requests
                .is_empty()
        );
    }

    fn make_app() -> Box<App> {
        Box::new(App::new(
            LaunchOptions::default(),
            StartupMetrics::with_frequency(1, 0),
        ))
    }

    fn load_native_scintilla() -> crate::platform::OwnedModule {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("native/out/x64/Scintilla.dll");
        let path = crate::platform::wide_null(path.to_str().unwrap());
        let module = unsafe {
            windows_sys::Win32::System::LibraryLoader::LoadLibraryExW(
                path.as_ptr(),
                std::ptr::null_mut(),
                windows_sys::Win32::System::LibraryLoader::LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR
                    | windows_sys::Win32::System::LibraryLoader::LOAD_LIBRARY_SEARCH_SYSTEM32,
            )
        };
        unsafe { crate::platform::OwnedModule::from_raw_owned(module) }.unwrap()
    }

    struct DropProbe {
        drops: Arc<AtomicUsize>,
    }

    impl DropProbe {
        fn new(drops: Arc<AtomicUsize>) -> Self {
            Self { drops }
        }
    }

    impl Drop for DropProbe {
        fn drop(&mut self) {
            self.drops.fetch_add(1, Ordering::SeqCst);
        }
    }

    struct ProductionWindow {
        hwnd: HWND,
        _class: MainWindowClass,
    }

    impl ProductionWindow {
        fn new(app: Box<App>) -> Self {
            let instance = unsafe { GetModuleHandleW(std::ptr::null()) };
            let class = MainWindowClass::register(instance).unwrap();
            let mut context = WindowCreateContext::new(app);
            let hwnd = class.create(&mut context).unwrap();

            Self {
                hwnd,
                _class: class,
            }
        }
    }

    impl Drop for ProductionWindow {
        fn drop(&mut self) {
            unsafe {
                let _ = DestroyWindow(self.hwnd);
            }
        }
    }

    /// Makes the window a primary instance saving its session under `scratch`.
    fn enable_session(hwnd: HWND, scratch: &RecoveryScratch) {
        let recovery = scratch.path().join("Recovery");
        std::fs::create_dir_all(&recovery).unwrap();
        let app = app_mut(hwnd);
        app.instance_mutex = Some(unnamed_mutex());
        app.recovery_root = Some(recovery);
        app.session_path = Some(scratch.path().join("session.ini"));
    }

    fn write_session(scratch: &RecoveryScratch, entries: Vec<SessionEntry>, active: usize) {
        crate::session::write(
            &scratch.path().join("session.ini"),
            &Session { active, entries },
        )
        .unwrap();
    }

    #[test]
    fn session_close_records_every_tab_without_prompting() {
        // Break caught: a session close that still asks about unsaved text, drops an unsaved or
        // clean tab from the manifest, loses the active tab, or deletes the snapshot the next
        // launch needs.
        let _scintilla = load_native_scintilla();
        let scratch = RecoveryScratch::new("session-close");
        let file = scratch.path().join("notes.txt");
        std::fs::write(&file, "saved text").unwrap();
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        enable_session(window.hwnd, &scratch);
        App::open_path(window.hwnd, &file).unwrap();
        execute_command(window.hwnd, CommandId::New);
        editor.set_text("unsaved words").unwrap();
        execute_command(window.hwnd, CommandId::New);
        let prompted = std::rc::Rc::new(std::cell::Cell::new(false));
        let seen = std::rc::Rc::clone(&prompted);
        answer_next_close_prompt(move |_| {
            seen.set(true);
            CloseDecision::Cancel
        });

        unsafe { SendMessageW(window.hwnd, WM_CLOSE, 0, 0) };

        assert!(!prompted.get(), "session restore must not prompt");
        assert_eq!(unsafe { IsWindow(window.hwnd) }, 0);
        let session = crate::session::read(&scratch.path().join("session.ini")).unwrap();
        assert_eq!(
            session.entries.len(),
            2,
            "the empty untitled tab is skipped"
        );
        assert_eq!(session.entries[0].source, SessionSource::File(file));
        let SessionSource::Snapshot(id) = session.entries[1].source else {
            panic!("the unsaved tab must be recorded as a snapshot");
        };
        assert_eq!(
            session.active, 1,
            "the skipped active tab falls back to the one before"
        );
        let snapshot =
            crate::recovery::snapshot::snapshot_path(&scratch.path().join("Recovery"), id);
        let snapshot = Snapshot::decode(&std::fs::read(snapshot).unwrap()).unwrap();
        assert_eq!(snapshot.text, "unsaved words");
    }

    #[test]
    fn session_close_still_prompts_when_restore_is_off() {
        // Break caught: the setting being ignored, so unsaved text is kept silently even though
        // the user asked to be prompted, or a manifest from an earlier close outliving it.
        let _scintilla = load_native_scintilla();
        let scratch = RecoveryScratch::new("session-off");
        write_session(
            &scratch,
            vec![SessionEntry::new(SessionSource::Snapshot(
                RecoveryId::from_u128(0x5e58),
            ))],
            0,
        );
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        enable_session(window.hwnd, &scratch);
        app_mut(window.hwnd).settings.restore_session = false;
        editor.set_text("dirty").unwrap();
        answer_next_close_prompt(|_| CloseDecision::Cancel);

        unsafe { SendMessageW(window.hwnd, WM_CLOSE, 0, 0) };

        assert_ne!(
            unsafe { IsWindow(window.hwnd) },
            0,
            "Cancel keeps the window"
        );
        assert!(!scratch.path().join("session.ini").exists());
    }

    /// Runs only the session unit until it hands over to `WM_FASTPAD_OPEN_LIBRARY`, without
    /// pumping the rest of the chain (which would bind the real single-instance pipe).
    fn run_session_restore(hwnd: HWND) {
        use windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW;
        let restore = crate::window::WM_FASTPAD_RESTORE_SESSION;
        unsafe { PostMessageW(hwnd, restore, 0, 0) };
        let mut message = MSG::default();
        while unsafe { PeekMessageW(&mut message, hwnd, restore, restore, PM_REMOVE) } != 0 {
            unsafe { DispatchMessageW(&message) };
        }
    }

    #[test]
    fn session_restore_reopens_files_and_unsaved_text_in_order() {
        // Break caught: restored tabs out of order, an unsaved file reopening untitled (so Ctrl+S
        // asks for a path), a stray empty startup tab, a lost caret, a manifest that restores
        // twice, crash recovery opening a restored snapshot again, or a restored tab still
        // pointing at the exited process's snapshot, which other windows would recover.
        let _scintilla = load_native_scintilla();
        let scratch = RecoveryScratch::new("session-restore");
        let recovery = scratch.path().join("Recovery");
        let notes = scratch.path().join("notes.txt");
        std::fs::write(&notes, "saved text").unwrap();
        let draft = scratch.path().join("draft.txt");
        std::fs::write(&draft, "on disk").unwrap();
        let draft_id = RecoveryId::from_u128(0x5e55);
        write_snapshot(
            &recovery,
            &Snapshot::new(
                draft_id,
                Some(draft.clone()),
                Encoding::Utf8,
                "unsaved draft",
            ),
        )
        .unwrap();
        let scratch_id = RecoveryId::from_u128(0x5e56);
        write_snapshot(
            &recovery,
            &Snapshot::new(scratch_id, None, Encoding::Utf8, "scratch words"),
        )
        .unwrap();
        write_session(
            &scratch,
            vec![
                SessionEntry {
                    source: SessionSource::Snapshot(draft_id),
                    caret: 3,
                    anchor: 1,
                    first_line: 0,
                },
                SessionEntry::new(SessionSource::File(notes.clone())),
                SessionEntry::new(SessionSource::Snapshot(scratch_id)),
            ],
            0,
        );
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        enable_session(window.hwnd, &scratch);

        run_session_restore(window.hwnd);

        {
            let app = app_mut(window.hwnd);
            let documents = app.tabs.documents().collect::<Vec<_>>();
            assert_eq!(documents.len(), 3, "the empty startup tab is closed");
            assert_eq!(documents[0].path.as_deref(), Some(draft.as_path()));
            assert!(documents[0].dirty);
            assert_eq!(documents[0].title(), "draft.txt *");
            assert_eq!(documents[1].path.as_deref(), Some(notes.as_path()));
            assert!(!documents[1].dirty);
            assert_eq!(documents[2].path, None);
            // Notes mode is on by default: this restored tab was briefly active while its
            // document loaded, and its untitled label was picked up from its first line.
            assert_eq!(documents[2].title(), "scratch words *");
            assert_eq!(app.tabs.active_index(), 0);
        }
        assert_eq!(editor.text().unwrap(), "unsaved draft");
        assert_eq!(editor.selection().unwrap(), 1..3);
        assert!(
            !scratch.path().join("session.ini").exists(),
            "the manifest is consumed"
        );
        let own = |index: usize| {
            let id = app_mut(window.hwnd)
                .tabs
                .documents()
                .nth(index)
                .unwrap()
                .recovery_id;
            crate::recovery::snapshot::snapshot_path(&recovery, id)
        };
        for (index, source, text) in [
            (0, draft_id, "unsaved draft"),
            (2, scratch_id, "scratch words"),
        ] {
            assert!(
                !crate::recovery::snapshot::snapshot_path(&recovery, source).exists(),
                "the dead process's snapshot would look like a crash leftover to other windows"
            );
            let adopted = Snapshot::decode(&std::fs::read(own(index)).unwrap()).unwrap();
            assert_eq!(adopted.text, text);
        }
        let draft_snapshot = own(0);

        unsafe { SendMessageW(window.hwnd, crate::window::WM_FASTPAD_RECOVERY, 0, 0) };
        assert_eq!(
            app_mut(window.hwnd).tabs.len(),
            3,
            "recovery must not duplicate a tab"
        );

        execute_command(window.hwnd, CommandId::Save);
        assert_eq!(std::fs::read_to_string(&draft).unwrap(), "unsaved draft");
        assert!(
            !draft_snapshot.exists(),
            "saving a restored tab removes its snapshot"
        );
    }

    #[test]
    fn session_restore_skips_unreopenable_entries_with_one_notice() {
        // Break caught: one missing file aborting the rest of the restore, a notice per file, or
        // no tab activated when the saved active entry is the one that failed.
        let _scintilla = load_native_scintilla();
        let scratch = RecoveryScratch::new("session-missing");
        let kept = scratch.path().join("kept.txt");
        std::fs::write(&kept, "still here").unwrap();
        write_session(
            &scratch,
            vec![
                SessionEntry::new(SessionSource::File(scratch.path().join("gone.txt"))),
                SessionEntry::new(SessionSource::File(kept.clone())),
                SessionEntry::new(SessionSource::Snapshot(RecoveryId::from_u128(0xdead))),
            ],
            0,
        );
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        enable_session(window.hwnd, &scratch);

        run_session_restore(window.hwnd);

        let app = app_mut(window.hwnd);
        assert_eq!(app.tabs.len(), 1);
        assert_eq!(
            app.tabs.active().unwrap().path.as_deref(),
            Some(kept.as_path())
        );
        assert_eq!(editor.text().unwrap(), "still here");
        let notices = app
            .notifications
            .pending()
            .iter()
            .filter(|notice| notice.message.contains("last session"))
            .map(|notice| notice.message.clone())
            .collect::<Vec<_>>();
        assert_eq!(notices, vec![crate::session::restore_failure_notice(2)]);
    }

    #[test]
    fn session_restore_ignores_a_window_outside_the_session() {
        // Break caught: a --new-window instance consuming the primary window's session.
        let _scintilla = load_native_scintilla();
        let scratch = RecoveryScratch::new("session-outside");
        let notes = scratch.path().join("notes.txt");
        std::fs::write(&notes, "saved text").unwrap();
        write_session(
            &scratch,
            vec![SessionEntry::new(SessionSource::File(notes))],
            0,
        );
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        enable_session(window.hwnd, &scratch);

        app_mut(window.hwnd).instance_mutex = None;
        run_session_restore(window.hwnd);

        assert_eq!(app_mut(window.hwnd).tabs.len(), 1);
        assert_eq!(app_mut(window.hwnd).tabs.active().unwrap().path, None);
        assert!(scratch.path().join("session.ini").exists());
    }

    #[test]
    fn session_restore_with_the_setting_off_deletes_a_stale_manifest() {
        // Break caught: a primary with the setting off leaving an old manifest behind, so turning
        // the setting back on later reopens tabs crash recovery already brought back.
        let _scintilla = load_native_scintilla();
        let scratch = RecoveryScratch::new("session-stale");
        let notes = scratch.path().join("notes.txt");
        std::fs::write(&notes, "saved text").unwrap();
        write_session(
            &scratch,
            vec![SessionEntry::new(SessionSource::File(notes))],
            0,
        );
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        enable_session(window.hwnd, &scratch);
        app_mut(window.hwnd).settings.restore_session = false;

        run_session_restore(window.hwnd);

        assert_eq!(app_mut(window.hwnd).tabs.len(), 1);
        assert_eq!(app_mut(window.hwnd).tabs.active().unwrap().path, None);
        assert!(app_mut(window.hwnd).session_restore.is_none());
        assert!(!scratch.path().join("session.ini").exists());
    }

    #[test]
    fn recovery_leaves_snapshots_a_saved_session_names_while_restore_is_on() {
        // Break caught: a --new-window instance recovering the primary's saved unsaved tabs as
        // "Recovered: ..." (and deleting them on Discard) before the next launch restores them.
        let _scintilla = load_native_scintilla();
        let scratch = RecoveryScratch::new("session-recover-skip");
        let recovery = scratch.path().join("Recovery");
        let saved_id = RecoveryId::from_u128(0x5e57);
        write_snapshot(
            &recovery,
            &Snapshot::new(saved_id, None, Encoding::Utf8, "kept for the session"),
        )
        .unwrap();
        write_session(
            &scratch,
            vec![SessionEntry::new(SessionSource::Snapshot(saved_id))],
            0,
        );
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        enable_session(window.hwnd, &scratch);
        app_mut(window.hwnd).instance_mutex = None;

        super::recover_snapshots(window.hwnd);

        assert_eq!(
            app_mut(window.hwnd).tabs.len(),
            1,
            "the saved snapshot waits"
        );
        assert!(crate::recovery::snapshot::snapshot_path(&recovery, saved_id).exists());

        app_mut(window.hwnd).settings.restore_session = false;
        super::recover_snapshots(window.hwnd);

        assert_eq!(
            app_mut(window.hwnd).tabs.len(),
            2,
            "with the setting off, crash recovery brings it back"
        );
    }

    #[test]
    fn session_restore_holds_forwarded_launches_until_it_finishes() {
        // Break caught: a forwarded file opening mid-restore and being buried under the rest of
        // the session, or never opening because the held request is not replayed.
        use windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW;
        let _scintilla = load_native_scintilla();
        let scratch = RecoveryScratch::new("session-forwarded");
        let first = scratch.path().join("first.txt");
        std::fs::write(&first, "one").unwrap();
        let second = scratch.path().join("second.txt");
        std::fs::write(&second, "two").unwrap();
        let forwarded = scratch.path().join("forwarded.txt");
        std::fs::write(&forwarded, "asked for mid-restore").unwrap();
        write_session(
            &scratch,
            vec![
                SessionEntry::new(SessionSource::File(first)),
                SessionEntry::new(SessionSource::File(second)),
            ],
            0,
        );
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        enable_session(window.hwnd, &scratch);
        let restore = crate::window::WM_FASTPAD_RESTORE_SESSION;
        let request = crate::window::WM_FASTPAD_IPC_REQUEST;

        unsafe { PostMessageW(window.hwnd, restore, 0, 0) };
        let mut message = MSG::default();
        assert_ne!(
            unsafe { PeekMessageW(&mut message, window.hwnd, restore, restore, PM_REMOVE) },
            0
        );
        unsafe { DispatchMessageW(&message) };
        assert!(app_mut(window.hwnd).session_restore.is_some());
        app_mut(window.hwnd)
            .ipc_requests
            .push(crate::ipc::IpcRequest::Open(forwarded.clone()));
        unsafe { SendMessageW(window.hwnd, request, 0, 0) };

        assert!(
            app_mut(window.hwnd).tabs.find_path(&forwarded).is_none(),
            "a forwarded file must wait for the restore"
        );
        assert_eq!(app_mut(window.hwnd).ipc_requests.len(), 1);

        run_session_restore(window.hwnd);
        assert!(app_mut(window.hwnd).session_restore.is_none());
        assert_ne!(
            unsafe { PeekMessageW(&mut message, window.hwnd, request, request, PM_REMOVE) },
            0,
            "finishing the restore replays the held requests"
        );
        unsafe { DispatchMessageW(&message) };
        discard_posted(window.hwnd, crate::window::WM_FASTPAD_APPLY_LANGUAGE);
        discard_posted(window.hwnd, crate::window::WM_FASTPAD_RECOVERY);
        discard_posted(window.hwnd, crate::window::WM_FASTPAD_OPEN_LIBRARY);

        let app = app_mut(window.hwnd);
        assert_eq!(app.tabs.len(), 3);
        assert_eq!(
            app.tabs.active().unwrap().path.as_deref(),
            Some(forwarded.as_path())
        );
        assert_eq!(editor.text().unwrap(), "asked for mid-restore");
    }

    #[test]
    fn session_restore_opens_the_launch_file_last() {
        // Break caught: the command-line file opening before the restored tabs, so a restored tab
        // ends up active instead of the file the user just asked for.
        let _scintilla = load_native_scintilla();
        let scratch = RecoveryScratch::new("session-launch");
        let restored = scratch.path().join("restored.txt");
        std::fs::write(&restored, "from last time").unwrap();
        let launched = scratch.path().join("launched.txt");
        std::fs::write(&launched, "asked for now").unwrap();
        write_session(
            &scratch,
            vec![SessionEntry::new(SessionSource::File(restored))],
            0,
        );
        let mut app = make_app();
        app.launch.request = crate::launch::LaunchRequest::Open(launched.into_os_string());
        let window = ProductionWindow::new(app);
        let editor = install_test_editor(&window);
        enable_session(window.hwnd, &scratch);

        run_session_restore(window.hwnd);
        unsafe { SendMessageW(window.hwnd, crate::window::WM_FASTPAD_OPEN_REQUEST, 0, 0) };

        assert_eq!(app_mut(window.hwnd).tabs.len(), 2);
        assert_eq!(app_mut(window.hwnd).tabs.active_index(), 1);
        assert_eq!(editor.text().unwrap(), "asked for now");
    }

    /// Removes every queued `message` for `hwnd` without dispatching it.
    fn discard_posted(hwnd: HWND, message: u32) {
        let mut queued = MSG::default();
        while unsafe { PeekMessageW(&mut queued, hwnd, message, message, PM_REMOVE) } != 0 {}
    }

    struct LibraryScratch {
        root: std::path::PathBuf,
    }

    impl LibraryScratch {
        fn new(label: &str) -> Self {
            let root = std::env::temp_dir()
                .join(format!("fastpad-libhost-{label}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(root.join("notes")).unwrap();
            std::fs::create_dir_all(root.join("data")).unwrap();
            Self { root }
        }
        fn folder(&self) -> std::path::PathBuf {
            self.root.join("notes")
        }
        fn data(&self) -> std::path::PathBuf {
            self.root.join("data")
        }
        fn note(&self, name: &str, text: &str) -> std::path::PathBuf {
            let path = self.folder().join(name);
            std::fs::write(&path, text).unwrap();
            path
        }
        /// Loads the folder synchronously and installs it, as LIBRARY_READY would.
        fn install(&self, hwnd: HWND) {
            let local = crate::library::local::local_file(&self.data(), &self.folder());
            let state =
                crate::library::load(&self.folder(), &local, crate::library::now_unix()).unwrap();
            crate::window::library_host::install_for_test(hwnd, state);
        }
    }

    impl Drop for LibraryScratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    /// Pumps posted messages until `done` or 5 s.
    fn pump_until(hwnd: HWND, done: impl Fn() -> bool) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !done() {
            assert!(std::time::Instant::now() < deadline, "timed out");
            pump_posted_messages(hwnd);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    fn tab_paths(hwnd: HWND) -> Vec<Option<std::path::PathBuf>> {
        app_mut(hwnd)
            .tabs
            .documents()
            .map(|document| document.path.clone())
            .collect()
    }

    #[test]
    fn the_first_edit_promotes_the_preview_so_a_later_click_opens_a_new_preview() {
        // Break caught: a click replacing a preview the user had started typing into, which drops
        // their text, or the edit not promoting so the tab keeps being replaced.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("preview-edit");
        let a = scratch.note("a.md", "a");
        let b = scratch.note("b.md", "b");
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        scratch.install(window.hwnd);
        // Autosave (unrelated to preview promotion) would otherwise clean `a` the moment `b` is
        // opened, since opening a file autosaves the tab being left; see
        // `switching_tabs_autosaves_the_tab_being_left`.
        execute_command(window.hwnd, CommandId::ToggleFolderAutosave);

        super::open_note(window.hwnd, &a, super::OpenMode::Preview, false).unwrap();
        assert_eq!(
            tab_paths(window.hwnd),
            [Some(a.clone())],
            "the empty start tab is reused"
        );
        assert!(app_mut(window.hwnd).tabs.active().unwrap().preview);

        editor.set_text("a, edited").unwrap();
        assert!(!app_mut(window.hwnd).tabs.active().unwrap().preview);
        assert_eq!(app_mut(window.hwnd).tabs.preview_id(), None);

        super::open_note(window.hwnd, &b, super::OpenMode::Preview, false).unwrap();
        assert_eq!(tab_paths(window.hwnd), [Some(a.clone()), Some(b.clone())]);
        assert!(app_mut(window.hwnd).tabs.active().unwrap().preview);
        let a_tab = app_mut(window.hwnd).tabs.find_stored_path(&a).unwrap();
        assert!(app_mut(window.hwnd).tabs.document(a_tab).unwrap().dirty);
    }

    #[test]
    fn a_second_preview_replaces_the_first_in_place_keeping_its_tab_index() {
        // Break caught: the replacement landing at the end of the strip, or a normal tab being
        // replaced instead of the preview.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("preview-replace");
        let x = scratch.note("x.md", "x");
        let a = scratch.note("a.md", "a");
        let y = scratch.note("y.md", "y");
        let b = scratch.note("b.md", "b");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        scratch.install(window.hwnd);
        super::open_path(window.hwnd, &x).unwrap();
        super::open_note(window.hwnd, &a, super::OpenMode::Preview, false).unwrap();
        super::open_path(window.hwnd, &y).unwrap();

        super::open_note(window.hwnd, &b, super::OpenMode::Preview, false).unwrap();

        assert_eq!(tab_paths(window.hwnd), [Some(x), Some(b), Some(y)]);
        assert_eq!(app_mut(window.hwnd).tabs.active_index(), 1);
        assert!(app_mut(window.hwnd).tabs.active().unwrap().preview);
    }

    #[test]
    fn opening_an_already_open_note_switches_to_its_tab() {
        // Break caught: a click on an open note replacing the preview with a second tab for the
        // same file, or doing nothing.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("preview-open");
        let a = scratch.note("a.md", "a");
        let b = scratch.note("b.md", "b");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        scratch.install(window.hwnd);
        super::open_path(window.hwnd, &a).unwrap();
        super::open_path(window.hwnd, &b).unwrap();

        super::open_note(window.hwnd, &a, super::OpenMode::Preview, false).unwrap();

        assert_eq!(super::tab_count(window.hwnd), 2);
        assert_eq!(
            app_mut(window.hwnd).tabs.active().unwrap().path.as_deref(),
            Some(a.as_path())
        );
        assert_eq!(app_mut(window.hwnd).tabs.preview_id(), None);
    }

    #[test]
    fn a_permanent_open_a_save_or_a_tab_double_click_keeps_the_preview() {
        // Break caught: Ctrl+Enter or a double-click opening a second tab for a note already in the
        // preview, or a saved preview still being replaced by the next click.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("preview-keep");
        let a = scratch.note("a.md", "a");
        let b = scratch.note("b.md", "b");
        let c = scratch.note("c.md", "c");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        scratch.install(window.hwnd);

        super::open_note(window.hwnd, &a, super::OpenMode::Preview, false).unwrap();
        super::open_note(window.hwnd, &a, super::OpenMode::Permanent, false).unwrap();
        assert_eq!(super::tab_count(window.hwnd), 1);
        assert_eq!(app_mut(window.hwnd).tabs.preview_id(), None);

        super::open_note(window.hwnd, &b, super::OpenMode::Preview, false).unwrap();
        crate::window::library_host::document_saved(window.hwnd);
        assert_eq!(
            app_mut(window.hwnd).tabs.preview_id(),
            None,
            "a save promotes"
        );

        super::open_note(window.hwnd, &c, super::OpenMode::Preview, false).unwrap();
        let index = app_mut(window.hwnd).tabs.active_index();
        let center = super::title_layout(window.hwnd).tab(index).center();
        let pack = |x: i32, y: i32| (x as u16 as u32 | ((y as u16 as u32) << 16)) as isize;
        // Both clicks carry the same message time, well inside the double-click time.
        for _ in 0..2 {
            unsafe {
                SendMessageW(
                    window.hwnd,
                    windows_sys::Win32::UI::WindowsAndMessaging::WM_LBUTTONUP,
                    0,
                    pack(center.x, center.y),
                );
            }
        }
        assert_eq!(
            app_mut(window.hwnd).tabs.preview_id(),
            None,
            "a double-click promotes"
        );
        assert_eq!(super::tab_count(window.hwnd), 3);
    }

    #[test]
    fn a_preview_tab_is_kept_by_the_session_and_comes_back_as_a_normal_tab() {
        // Break caught: the session skipping the preview tab, so it vanishes at restart, or the
        // restored tab still being a preview that the next click silently replaces.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("preview-session");
        let a = scratch.note("a.md", "a");
        let recovery = RecoveryScratch::new("preview-session");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        scratch.install(window.hwnd);
        super::open_note(window.hwnd, &a, super::OpenMode::Preview, false).unwrap();

        let session = super::build_session(window.hwnd, recovery.path()).unwrap();
        assert_eq!(session.entries.len(), 1);
        assert!(matches!(&session.entries[0].source, SessionSource::File(path) if *path == a));

        let id = app_mut(window.hwnd).tabs.active().unwrap().id;
        super::close_document_without_prompt(window.hwnd, id);
        super::restore_session_entry(window.hwnd, &session.entries[0]).unwrap();
        assert!(!app_mut(window.hwnd).tabs.active().unwrap().preview);
    }

    #[test]
    fn opening_another_folder_flushes_the_old_one_remembers_the_new_one_and_keeps_tabs() {
        let _scintilla = load_native_scintilla();
        let first = LibraryScratch::new("switch-a");
        let second = LibraryScratch::new("switch-b");
        let a = first.note("a.md", "a");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        app_mut(window.hwnd).library.data_dir = Some(first.data());
        first.install(window.hwnd);
        super::open_path(window.hwnd, &a).unwrap();
        crate::window::library_host::with_state(window.hwnd, |state| {
            let mut ids = crate::library::ids::IdSource::new(1, 1);
            let target = state.note_ref(&mut ids, &a);
            state
                .apply(crate::library::ops::PendingOp::SetPinned {
                    note: target,
                    value: true,
                })
                .unwrap();
        });

        crate::window::answer_next_folder_dialog({
            let folder = second.folder();
            move |_| Some(folder)
        });
        execute_command(window.hwnd, CommandId::OpenFolder);

        assert!(
            crate::library::store::library_file(&first.folder()).exists(),
            "old folder flushed"
        );
        assert_eq!(
            crate::window::library_host::folder(window.hwnd),
            Some(second.folder())
        );
        let recent = crate::library::local::read_folders(&crate::library::local::folders_file(
            &first.data(),
        ));
        assert_eq!(recent.folders.first(), Some(&second.folder()));
        assert_eq!(super::tab_count(window.hwnd), 1, "open tabs stay open");
        pump_until(window.hwnd, || app_mut(window.hwnd).library.state.is_some());
    }

    #[test]
    fn opening_a_path_that_is_not_a_folder_explains_why() {
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        crate::window::library_host::open_folder(
            window.hwnd,
            std::path::Path::new(r"Z:\no\such\folder"),
        );
        assert!(
            notices(window.hwnd)
                .iter()
                .any(|n| n.contains("is not a folder"))
        );
        assert_eq!(crate::window::library_host::folder(window.hwnd), None);
    }

    #[test]
    fn a_failed_flush_keeps_the_current_folder_open() {
        // Break caught: switching folders after library.ini could not be written, which drops
        // the unsaved pins with the old state.
        let _scintilla = load_native_scintilla();
        let first = LibraryScratch::new("flushfail-a");
        let second = LibraryScratch::new("flushfail-b");
        let a = first.note("a.md", "a");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        app_mut(window.hwnd).library.data_dir = Some(first.data());
        first.install(window.hwnd);
        crate::window::library_host::with_state(window.hwnd, |state| {
            let mut ids = crate::library::ids::IdSource::new(1, 1);
            let target = state.note_ref(&mut ids, &a);
            state
                .apply(crate::library::ops::PendingOp::SetPinned {
                    note: target,
                    value: true,
                })
                .unwrap();
        });
        // A file where the .fastpad directory belongs makes the write fail.
        std::fs::write(first.folder().join(".fastpad"), "not a directory").unwrap();

        crate::window::library_host::open_folder(window.hwnd, &second.folder());

        assert_eq!(
            crate::window::library_host::folder(window.hwnd),
            Some(first.folder())
        );
        assert!(app_mut(window.hwnd).library.state.is_some());
        assert!(
            notices(window.hwnd)
                .iter()
                .any(|n| n.contains("kept this notebook open")),
            "{:?}",
            notices(window.hwnd)
        );
        let recent = crate::library::local::read_folders(&crate::library::local::folders_file(
            &first.data(),
        ));
        assert!(!recent.folders.contains(&second.folder()));
    }

    #[test]
    fn a_recent_folder_pick_opens_the_row_that_was_shown() {
        // Break caught: resolving the chosen row against folders.ini re-read after the picker
        // opened, which opens a different folder when another window changed the list.
        let _scintilla = load_native_scintilla();
        let first = LibraryScratch::new("recent-a");
        let second = LibraryScratch::new("recent-b");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        app_mut(window.hwnd).library.data_dir = Some(first.data());
        let folders_file = crate::library::local::folders_file(&first.data());
        let write = |order: Vec<std::path::PathBuf>| {
            crate::library::local::write_folders(
                &folders_file,
                &crate::library::local::RecentFolders {
                    folders: order,
                    ..Default::default()
                },
            )
            .unwrap();
        };
        write(vec![first.folder(), second.folder()]);
        execute_command(window.hwnd, CommandId::OpenRecentFolder);
        write(vec![second.folder(), first.folder()]);

        crate::window::library_host::picked(
            window.hwnd,
            crate::window::command_palette::PickerKind::RecentFolder,
            crate::window::command_palette::PickerChoice::Item(0),
        );

        pump_until(window.hwnd, || {
            crate::window::library_host::folder(window.hwnd) == Some(first.folder())
        });
        pump_until(window.hwnd, || app_mut(window.hwnd).library.state.is_some());
    }

    #[test]
    fn a_launch_argument_naming_a_folder_is_not_reported_as_a_failed_open() {
        // Break caught: `fastpad D:\Notes` opening the folder as the library and then also
        // trying to open it as a file, which reports "could not open".
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("launch-dir");
        let mut app = make_app();
        app.launch.request = crate::launch::LaunchRequest::Open(scratch.folder().into_os_string());
        let window = ProductionWindow::new(app);
        let _editor = install_test_editor(&window);
        let tabs = super::tab_count(window.hwnd);

        unsafe { SendMessageW(window.hwnd, crate::window::WM_FASTPAD_OPEN_REQUEST, 0, 0) };

        assert!(
            !notices(window.hwnd)
                .iter()
                .any(|n| n.contains("could not open")),
            "{:?}",
            notices(window.hwnd)
        );
        assert_eq!(super::tab_count(window.hwnd), tabs);
    }

    #[test]
    fn files_dropped_on_the_editor_reach_the_drop_handler() {
        // Break caught: Scintilla's own OLE drop target refusing Explorer's files, so a drop on
        // the editor (most of the window) did nothing.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("editor-drop");
        let note = scratch.note("dropped.md", "dropped");
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        crate::window::library_host::accept_editor_file_drops(window.hwnd);

        let effects =
            crate::editor::file_drop::test_support::drag_and_drop(editor.hwnd(), &[&note]);

        assert_eq!(
            effects,
            [windows_sys::Win32::System::Ole::DROPEFFECT_COPY; 3]
        );
        pump_until(window.hwnd, || editor.text().unwrap() == "dropped");
    }

    #[test]
    fn the_library_step_loads_the_remembered_folder_on_a_worker_thread() {
        // Break caught: the scan running on the UI thread, or the remembered folder ignored.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("startup");
        scratch.note("a.md", "a");
        let mut folders = crate::library::local::RecentFolders::default();
        folders.push(scratch.folder());
        crate::library::local::write_folders(
            &crate::library::local::folders_file(&scratch.data()),
            &folders,
        )
        .unwrap();
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        app_mut(window.hwnd).library.data_dir = Some(scratch.data());
        crate::window::library_host::open_library_step(window.hwnd);
        assert_eq!(
            crate::window::library_host::folder(window.hwnd),
            Some(scratch.folder())
        );
        pump_until(window.hwnd, || app_mut(window.hwnd).library.state.is_some());
        assert_eq!(
            app_mut(window.hwnd)
                .library
                .state
                .as_ref()
                .unwrap()
                .notes
                .len(),
            1
        );
    }

    #[test]
    fn with_notes_mode_off_the_library_step_does_nothing() {
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("off");
        let window = ProductionWindow::new(make_app());
        app_mut(window.hwnd).settings.notes_mode = false;
        app_mut(window.hwnd).library.data_dir = Some(scratch.data());
        crate::window::library_host::open_library_step(window.hwnd);
        assert_eq!(crate::window::library_host::folder(window.hwnd), None);
        assert!(!app_mut(window.hwnd).library.scanning);
    }

    #[test]
    fn a_stale_ready_message_is_dropped_and_writes_are_flushed_on_demand() {
        // Break caught: a slow scan of the previous folder replacing the folder just opened.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("stale");
        let a = scratch.note("a.md", "a");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        scratch.install(window.hwnd);
        let stale = crate::window::library_host::test_ready_payload(
            app_mut(window.hwnd).library.generation.wrapping_sub(1),
            Err("old".into()),
        );
        crate::window::library_host::library_ready(window.hwnd, stale);
        assert!(app_mut(window.hwnd).library.state.is_some());
        assert!(notices(window.hwnd).iter().all(|n| !n.contains("old")));

        crate::window::library_host::with_state(window.hwnd, |state| {
            let mut ids = crate::library::ids::IdSource::new(1, 1);
            let target = state.note_ref(&mut ids, &a);
            state
                .apply(crate::library::ops::PendingOp::SetPinned {
                    note: target,
                    value: true,
                })
                .unwrap();
        });
        crate::window::library_host::flush_now(window.hwnd);
        assert!(crate::library::store::library_file(&scratch.folder()).exists());
    }

    #[test]
    fn session_restore_holds_the_startup_chain_until_it_finishes() {
        // Break caught: a tab reopened mid-restore posting the language unit, which then runs
        // recovery, binds the IPC pipe and stamps FullyReady before the rest of the session is
        // back, so a forwarded launch can open mid-restore and lose the active tab.
        use windows_sys::Win32::UI::WindowsAndMessaging::{PM_NOREMOVE, PostMessageW};
        let _scintilla = load_native_scintilla();
        let scratch = RecoveryScratch::new("session-chain");
        let first = scratch.path().join("first.txt");
        std::fs::write(&first, "one").unwrap();
        let second = scratch.path().join("second.txt");
        std::fs::write(&second, "two").unwrap();
        write_session(
            &scratch,
            vec![
                SessionEntry::new(SessionSource::File(first)),
                SessionEntry::new(SessionSource::File(second)),
            ],
            0,
        );
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        enable_session(window.hwnd, &scratch);
        let restore = crate::window::WM_FASTPAD_RESTORE_SESSION;
        let language = crate::window::WM_FASTPAD_APPLY_LANGUAGE;
        let recovery = crate::window::WM_FASTPAD_RECOVERY;

        unsafe { PostMessageW(window.hwnd, restore, 0, 0) };
        let mut message = MSG::default();
        assert_ne!(
            unsafe { PeekMessageW(&mut message, window.hwnd, restore, restore, PM_REMOVE) },
            0
        );
        unsafe { DispatchMessageW(&message) };
        assert!(
            app_mut(window.hwnd).session_restore.is_some(),
            "one entry is still to come"
        );
        let mut languages = 0;
        while unsafe { PeekMessageW(&mut message, window.hwnd, language, language, PM_REMOVE) } != 0
        {
            languages += 1;
            unsafe { DispatchMessageW(&message) };
        }

        assert!(languages > 0, "the reopened tab posts the language unit");
        let recovery_posted =
            unsafe { PeekMessageW(&mut message, window.hwnd, recovery, recovery, PM_NOREMOVE) }
                != 0;
        run_session_restore(window.hwnd);
        discard_posted(window.hwnd, language);
        discard_posted(window.hwnd, recovery);
        assert!(
            !recovery_posted,
            "the chain must wait for the restore to finish"
        );
        assert!(app_mut(window.hwnd).session_restore.is_none());
        assert_eq!(app_mut(window.hwnd).tabs.len(), 2);
    }

    #[test]
    fn session_close_falls_back_to_the_prompt_when_the_manifest_cannot_be_written() {
        // Break caught: a failed manifest write still closing silently, so the unsaved text is
        // named by no session and the user was never asked about it.
        let _scintilla = load_native_scintilla();
        let scratch = RecoveryScratch::new("session-unwritable");
        let blocker = scratch.path().join("blocker");
        std::fs::write(&blocker, "a file, not a folder").unwrap();
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        enable_session(window.hwnd, &scratch);
        app_mut(window.hwnd).session_path = Some(blocker.join("session.ini"));
        editor.set_text("unsaved words").unwrap();
        let prompted = std::rc::Rc::new(std::cell::Cell::new(false));
        let seen = std::rc::Rc::clone(&prompted);
        answer_next_close_prompt(move |_| {
            seen.set(true);
            CloseDecision::Cancel
        });

        unsafe { SendMessageW(window.hwnd, WM_CLOSE, 0, 0) };

        assert!(
            prompted.get(),
            "an unwritable manifest must fall back to the prompt"
        );
        assert_ne!(
            unsafe { IsWindow(window.hwnd) },
            0,
            "Cancel keeps the window"
        );
    }

    #[test]
    fn session_close_falls_back_to_the_prompt_when_a_snapshot_cannot_be_written() {
        // Break caught: a dirty tab whose text reached no snapshot file closing silently, so the
        // manifest names nothing for it and the user was never asked.
        let _scintilla = load_native_scintilla();
        let scratch = RecoveryScratch::new("session-no-snapshot");
        let blocker = scratch.path().join("blocker");
        std::fs::write(&blocker, "a file, not a folder").unwrap();
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        enable_session(window.hwnd, &scratch);
        app_mut(window.hwnd).recovery_root = Some(blocker);
        editor.set_text("unsaved words").unwrap();
        let prompted = std::rc::Rc::new(std::cell::Cell::new(false));
        let seen = std::rc::Rc::clone(&prompted);
        answer_next_close_prompt(move |_| {
            seen.set(true);
            CloseDecision::Cancel
        });

        unsafe { SendMessageW(window.hwnd, WM_CLOSE, 0, 0) };

        assert!(
            prompted.get(),
            "a failed snapshot write must fall back to the prompt"
        );
        assert_ne!(
            unsafe { IsWindow(window.hwnd) },
            0,
            "Cancel keeps the window"
        );
        assert!(!scratch.path().join("session.ini").exists());
    }

    #[test]
    fn session_close_during_a_restore_uses_the_prompt() {
        // Break caught: a close mid-restore writing a manifest of only the tabs reopened so far,
        // silently dropping the entries still to come.
        let _scintilla = load_native_scintilla();
        let scratch = RecoveryScratch::new("session-mid-restore");
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        enable_session(window.hwnd, &scratch);
        app_mut(window.hwnd).session_restore = Some(crate::session::SessionRestore::new(
            Session::default(),
            None,
        ));
        editor.set_text("unsaved words").unwrap();
        let prompted = std::rc::Rc::new(std::cell::Cell::new(false));
        let seen = std::rc::Rc::clone(&prompted);
        answer_next_close_prompt(move |_| {
            seen.set(true);
            CloseDecision::Cancel
        });

        unsafe { SendMessageW(window.hwnd, WM_CLOSE, 0, 0) };

        assert!(prompted.get(), "a close mid-restore must use the review");
        assert_ne!(
            unsafe { IsWindow(window.hwnd) },
            0,
            "Cancel keeps the window"
        );
        assert!(!scratch.path().join("session.ini").exists());
        app_mut(window.hwnd).session_restore = None;
    }

    fn type_into_name_box(hwnd: HWND, text: &str) {
        let edit = app_mut(hwnd).name_box.as_ref().unwrap().edit_hwnd();
        let wide = crate::platform::wide_null(text);
        unsafe { windows_sys::Win32::UI::WindowsAndMessaging::SetWindowTextW(edit, wide.as_ptr()) };
    }

    #[test]
    fn the_first_save_of_an_untitled_note_asks_for_a_name_in_the_folder_prefilled_from_its_label() {
        // Break caught: Ctrl+S on a new note opening the system dialog in some random folder, or
        // saving without letting the user confirm the name.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("first-save");
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        scratch.install(window.hwnd);
        super::create_new_document(window.hwnd).unwrap();
        editor.set_text("Meeting: notes?\nbody").unwrap();
        pump_posted_messages(window.hwnd);

        execute_command(window.hwnd, CommandId::Save);
        let name_box = app_mut(window.hwnd).name_box.as_ref().unwrap();
        assert!(name_box.is_visible());
        assert_eq!(name_box.text(), "Meeting notes.md");

        crate::window::library_host::name_box_submit(window.hwnd);
        let saved = scratch.folder().join("Meeting notes.md");
        assert_eq!(
            std::fs::read_to_string(&saved).unwrap(),
            "Meeting: notes?\nbody"
        );
        assert!(!app_mut(window.hwnd).name_box.as_ref().unwrap().is_visible());
        assert_eq!(
            app_mut(window.hwnd).tabs.active().unwrap().path.as_deref(),
            Some(saved.as_path())
        );
        assert!(
            app_mut(window.hwnd)
                .library
                .state
                .as_ref()
                .unwrap()
                .notes
                .iter()
                .any(|n| n.path == std::path::Path::new("Meeting notes.md"))
        );
        assert!(
            app_mut(window.hwnd)
                .tabs
                .active()
                .unwrap()
                .disk_stamp
                .is_some()
        );
    }

    #[test]
    fn a_name_that_already_exists_is_refused_with_a_suggestion() {
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("clash");
        scratch.note("Plan.md", "old");
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        scratch.install(window.hwnd);
        super::create_new_document(window.hwnd).unwrap();
        editor.set_text("Plan").unwrap();
        execute_command(window.hwnd, CommandId::Save);
        type_into_name_box(window.hwnd, "plan.MD");
        crate::window::library_host::name_box_submit(window.hwnd);
        assert_eq!(
            std::fs::read_to_string(scratch.folder().join("Plan.md")).unwrap(),
            "old"
        );
        let name_box = app_mut(window.hwnd).name_box.as_ref().unwrap();
        assert!(name_box.is_visible());
        assert_eq!(
            name_box.error(),
            Some("plan.MD already exists. Try plan 2.MD.")
        );
    }

    #[test]
    fn browse_and_the_close_prompt_use_the_save_dialog_starting_with_the_suggested_name() {
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("browse");
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        scratch.install(window.hwnd);
        super::create_new_document(window.hwnd).unwrap();
        editor.set_text("Ideas").unwrap();
        assert_eq!(
            crate::window::library_host::suggested_file_name(window.hwnd),
            "Ideas.md"
        );
        let elsewhere = scratch.root.join("elsewhere.md");
        crate::window::answer_next_save_dialog({
            let elsewhere = elsewhere.clone();
            move |_| Some(elsewhere)
        });
        execute_command(window.hwnd, CommandId::Save);
        crate::window::library_host::name_box_browse(window.hwnd);
        assert_eq!(std::fs::read_to_string(&elsewhere).unwrap(), "Ideas");
    }

    #[test]
    fn with_notes_mode_off_save_uses_the_dialog_as_before() {
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("mode-off-save");
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        app_mut(window.hwnd).settings.notes_mode = false;
        super::create_new_document(window.hwnd).unwrap();
        editor.set_text("x").unwrap();
        let target = scratch.root.join("x.txt");
        crate::window::answer_next_save_dialog({
            let target = target.clone();
            move |_| Some(target)
        });
        execute_command(window.hwnd, CommandId::Save);
        assert!(target.exists());
        assert!(app_mut(window.hwnd).name_box.is_none());
        // Break caught: notes-mode naming leaking into plain-editor Save As.
        assert_eq!(
            crate::window::modal::take_last_save_request(),
            Some(("Untitled.txt".to_owned(), None))
        );
    }

    #[test]
    fn save_as_on_a_tab_with_a_path_starts_where_the_dialog_always_did() {
        // Break caught: Save As of an ordinary file jumping to the notes folder.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("save-as-titled");
        let outside = scratch.root.join("outside.txt");
        std::fs::write(&outside, "o").unwrap();
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        scratch.install(window.hwnd);
        super::open_path(window.hwnd, &outside).unwrap();
        crate::window::answer_next_save_dialog(|_| None);
        execute_command(window.hwnd, CommandId::SaveAs);
        assert_eq!(
            crate::window::modal::take_last_save_request(),
            Some(("outside.txt".to_owned(), None))
        );
        assert!(app_mut(window.hwnd).name_box.is_none());
    }

    #[test]
    fn a_first_save_never_replaces_a_file_that_took_the_name_after_the_check() {
        // Break caught: a note created in Explorer between the name check and the write being
        // overwritten by the first save.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("first-save-race");
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        scratch.install(window.hwnd);
        super::create_new_document(window.hwnd).unwrap();
        editor.set_text("mine").unwrap();
        let late = scratch.note("Late.md", "theirs");
        let identity = unsafe { super::window_identity(window.hwnd) }.unwrap();
        assert_eq!(
            super::complete_first_save(window.hwnd, &identity, late.clone()),
            super::SaveOutcome::NameTaken
        );
        assert_eq!(std::fs::read_to_string(&late).unwrap(), "theirs");
        let active = app_mut(window.hwnd).tabs.active().unwrap();
        assert!(active.path.is_none());
        assert!(active.dirty);
    }

    fn open_first_save_box(
        label: &str,
    ) -> (LibraryScratch, ProductionWindow, crate::editor::Editor) {
        let scratch = LibraryScratch::new(label);
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        scratch.install(window.hwnd);
        super::create_new_document(window.hwnd).unwrap();
        editor.set_text("Draft").unwrap();
        execute_command(window.hwnd, CommandId::Save);
        assert!(app_mut(window.hwnd).name_box.as_ref().unwrap().is_visible());
        (scratch, window, editor)
    }

    fn name_box_visible(hwnd: HWND) -> bool {
        app_mut(hwnd)
            .name_box
            .as_ref()
            .is_some_and(|name_box| name_box.is_visible())
    }

    #[test]
    fn escape_in_the_name_box_cancels_the_save() {
        let _scintilla = load_native_scintilla();
        let (_scratch, window, _editor) = open_first_save_box("escape");
        let edit = app_mut(window.hwnd).name_box.as_ref().unwrap().edit_hwnd();
        unsafe {
            SendMessageW(
                edit,
                windows_sys::Win32::UI::WindowsAndMessaging::WM_KEYDOWN,
                windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE as usize,
                0,
            );
        }
        assert!(!name_box_visible(window.hwnd));
        let active = app_mut(window.hwnd).tabs.active().unwrap();
        assert!(active.path.is_none());
        assert!(active.dirty);
    }

    #[test]
    fn the_name_box_closes_with_its_tab_or_when_another_tab_is_activated() {
        // Break caught: a box left naming a tab that is gone or not the one on screen.
        let _scintilla = load_native_scintilla();
        let (_scratch, window, _editor) = open_first_save_box("tab-change");
        super::create_new_document(window.hwnd).unwrap();
        assert!(!name_box_visible(window.hwnd));

        execute_command(window.hwnd, CommandId::Save);
        assert!(name_box_visible(window.hwnd));
        super::close_active_document(window.hwnd);
        assert!(!name_box_visible(window.hwnd));
    }

    #[test]
    fn the_name_box_closes_once_its_tab_is_saved_another_way() {
        let _scintilla = load_native_scintilla();
        let (scratch, window, _editor) = open_first_save_box("saved-elsewhere");
        super::save_path_as(window.hwnd, &scratch.root.join("elsewhere.md"));
        assert!(!name_box_visible(window.hwnd));
    }

    #[test]
    fn saving_again_while_the_box_is_open_keeps_what_was_typed() {
        let _scintilla = load_native_scintilla();
        let (_scratch, window, _editor) = open_first_save_box("save-twice");
        type_into_name_box(window.hwnd, "Typed name.md");
        execute_command(window.hwnd, CommandId::Save);
        let name_box = app_mut(window.hwnd).name_box.as_ref().unwrap();
        assert!(name_box.is_visible());
        assert_eq!(name_box.text(), "Typed name.md");
    }

    #[test]
    fn opening_find_closes_the_name_box() {
        let _scintilla = load_native_scintilla();
        let (_scratch, window, _editor) = open_first_save_box("find-closes");
        execute_command(window.hwnd, CommandId::Find);
        assert!(!name_box_visible(window.hwnd));
        assert!(app_mut(window.hwnd).find_bar.as_ref().unwrap().is_visible());
    }

    #[test]
    fn typed_names_with_invalid_characters_or_device_names_save_under_the_sanitized_name() {
        // Break caught: a typed "con" or "a/b" failing the save with a Windows path error.
        let _scintilla = load_native_scintilla();
        let (scratch, window, _editor) = open_first_save_box("sanitize");
        type_into_name_box(window.hwnd, "a/b: c?.md");
        crate::window::library_host::name_box_submit(window.hwnd);
        assert_eq!(
            std::fs::read_to_string(scratch.folder().join("ab c.md")).unwrap(),
            "Draft"
        );

        super::create_new_document(window.hwnd).unwrap();
        app_mut(window.hwnd)
            .editor
            .as_ref()
            .unwrap()
            .set_text("Device")
            .unwrap();
        execute_command(window.hwnd, CommandId::Save);
        type_into_name_box(window.hwnd, "con");
        crate::window::library_host::name_box_submit(window.hwnd);
        assert_eq!(
            std::fs::read_to_string(scratch.folder().join("con_.md")).unwrap(),
            "Device"
        );
        assert!(!name_box_visible(window.hwnd));
    }

    fn open_note(
        window: &ProductionWindow,
        scratch: &LibraryScratch,
        name: &str,
        text: &str,
    ) -> std::path::PathBuf {
        let path = scratch.note(name, text);
        scratch.install(window.hwnd);
        super::open_path(window.hwnd, &path).unwrap();
        pump_posted_messages(window.hwnd);
        path
    }

    #[test]
    fn closing_the_notebook_saves_its_notes_keeps_the_tabs_and_is_remembered_as_closed() {
        // Break caught: Close notebook dropping a dirty note's edits, closing its tab, or leaving
        // folders.ini without open=none so the next start reopens the notebook anyway.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("close-notebook");
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        app_mut(window.hwnd).library.data_dir = Some(scratch.data());
        crate::library::local::write_folders(
            &crate::library::local::folders_file(&scratch.data()),
            &crate::library::local::RecentFolders {
                folders: vec![scratch.folder()],
                ..Default::default()
            },
        )
        .unwrap();
        let path = open_note(&window, &scratch, "a.md", "one");
        editor.set_text("two").unwrap();

        execute_command(window.hwnd, CommandId::CloseNotebook);

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "two");
        assert_eq!(super::tab_count(window.hwnd), 1);
        assert_eq!(
            app_mut(window.hwnd).tabs.active().unwrap().path.as_deref(),
            Some(path.as_path())
        );
        assert_eq!(crate::window::library_host::folder(window.hwnd), None);
        assert!(app_mut(window.hwnd).library.state.is_none());
        let folders = crate::library::local::read_folders(&crate::library::local::folders_file(
            &scratch.data(),
        ));
        assert!(folders.closed);
        assert!(
            folders.folders.contains(&scratch.folder()),
            "it stays in the recent list"
        );
        // The tab is a plain file now: an edit is not autosaved.
        editor.set_text("three").unwrap();
        assert_eq!(
            crate::window::library_host::autosave_active(window.hwnd),
            crate::window::library_host::Autosave::NotEligible
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "two");
    }

    #[test]
    fn with_no_notebook_open_ctrl_s_uses_the_save_dialog_like_notes_mode_off() {
        // Break caught: after Close notebook, Ctrl+S on a new tab opening the name box for a notebook
        // that is gone, or the Save As dialog starting in the closed notebook under the tab's label.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("no-notebook-save");
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        app_mut(window.hwnd).library.data_dir = Some(scratch.data());
        scratch.install(window.hwnd);
        execute_command(window.hwnd, CommandId::CloseNotebook);
        execute_command(window.hwnd, CommandId::New);
        editor.set_text("Plan\nbody").unwrap();
        let target = scratch.root.join("plan.txt");
        crate::window::answer_next_save_dialog({
            let target = target.clone();
            move |_| Some(target)
        });

        execute_command(window.hwnd, CommandId::Save);

        assert_eq!(std::fs::read_to_string(&target).unwrap(), "Plan\nbody");
        assert!(
            app_mut(window.hwnd)
                .name_box
                .as_ref()
                .is_none_or(|name_box| !name_box.is_visible())
        );
        assert_eq!(
            crate::window::modal::take_last_save_request(),
            Some(("Untitled.txt".to_owned(), None))
        );
    }

    #[test]
    fn the_notebook_favorite_toggles_in_folders_ini_and_the_cached_lists() {
        // Break caught: a star that changes the sidebar but not folders.ini (lost at restart), or
        // favorite lists that read folders.ini on every sidebar paint.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("favorite-notebook");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        app_mut(window.hwnd).library.data_dir = Some(scratch.data());
        scratch.install(window.hwnd);
        let file = crate::library::local::folders_file(&scratch.data());

        execute_command(window.hwnd, CommandId::ToggleNotebookFavorite);
        assert!(crate::window::library_host::is_favorite(window.hwnd));
        assert_eq!(
            crate::window::library_host::favorites(window.hwnd),
            vec![scratch.folder()]
        );
        assert!(crate::library::local::read_folders(&file).is_favorite(&scratch.folder()));
        assert!(
            notices(window.hwnd)
                .iter()
                .any(|n| n.contains("to favorite notebooks"))
        );

        execute_command(window.hwnd, CommandId::ToggleNotebookFavorite);
        assert!(!crate::window::library_host::is_favorite(window.hwnd));
        assert!(!crate::library::local::read_folders(&file).is_favorite(&scratch.folder()));

        execute_command(window.hwnd, CommandId::ToggleNotebookFavorite);
        crate::window::library_host::remove_favorite(window.hwnd, &scratch.folder());
        assert!(crate::window::library_host::favorites(window.hwnd).is_empty());
        assert!(!crate::library::local::read_folders(&file).is_favorite(&scratch.folder()));

        // Listing answers from the cache: a file changed behind FastPad's back is not re-read.
        execute_command(window.hwnd, CommandId::ToggleNotebookFavorite);
        crate::library::local::write_folders(
            &file,
            &crate::library::local::RecentFolders::default(),
        )
        .unwrap();
        assert_eq!(
            crate::window::library_host::favorites(window.hwnd),
            vec![scratch.folder()]
        );
    }

    #[test]
    fn opening_a_listed_notebook_that_is_missing_says_so_and_changes_nothing() {
        // Break caught: a click on an offline favorite unloading the open notebook first, checking
        // the drive on the UI thread, or falling back to Documents\FastPad the way startup does.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("listed-missing");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        app_mut(window.hwnd).library.data_dir = Some(scratch.data());
        scratch.install(window.hwnd);
        let missing = scratch.root.join("gone");
        let checks = crate::library::folder_checks();

        crate::window::library_host::open_listed_notebook(window.hwnd, &missing);
        assert_eq!(
            crate::library::folder_checks(),
            checks,
            "checked on the worker"
        );
        pump_until(window.hwnd, || {
            notices(window.hwnd)
                .iter()
                .any(|n| n.contains("is not available"))
        });

        assert_eq!(
            crate::window::library_host::folder(window.hwnd),
            Some(scratch.folder())
        );
        assert!(app_mut(window.hwnd).library.state.is_some());
        let folders = crate::library::local::read_folders(&crate::library::local::folders_file(
            &scratch.data(),
        ));
        assert!(!folders.folders.contains(&missing));
    }

    #[test]
    fn opening_a_listed_notebook_switches_once_the_worker_finds_it() {
        // Break caught: an explicit open that switches before the check lands (so a missing folder
        // would already have unloaded the notebook), or never switches at all.
        let _scintilla = load_native_scintilla();
        let first = LibraryScratch::new("listed-a");
        let second = LibraryScratch::new("listed-b");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        app_mut(window.hwnd).library.data_dir = Some(first.data());
        first.install(window.hwnd);

        crate::window::library_host::open_listed_notebook(window.hwnd, &second.folder());
        assert_eq!(
            crate::window::library_host::folder(window.hwnd),
            Some(first.folder()),
            "nothing changes before the check lands"
        );
        pump_until(window.hwnd, || {
            crate::window::library_host::folder(window.hwnd) == Some(second.folder())
        });
        pump_until(window.hwnd, || app_mut(window.hwnd).library.state.is_some());
        assert_eq!(
            crate::window::library_host::recent_notebooks(window.hwnd).first(),
            Some(&second.folder())
        );
    }

    #[test]
    fn a_stale_listed_notebook_check_is_dropped_once_the_user_has_moved_on() {
        // Break caught: a slow existence check for a notebook landing after the user already
        // closed the open notebook (or switched to another one), reopening or replacing it anyway.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("stale-check");
        let stale = LibraryScratch::new("stale-check-target");
        let fresh = LibraryScratch::new("stale-check-fresh");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        app_mut(window.hwnd).library.data_dir = Some(scratch.data());
        scratch.install(window.hwnd);

        crate::window::library_host::open_listed_notebook(window.hwnd, &stale.folder());
        // Close notebook runs on this thread before the check's worker answer is pumped: the
        // close must invalidate the still-in-flight check.
        execute_command(window.hwnd, CommandId::CloseNotebook);
        // Give the worker's existence check time to land and be pumped, then confirm it changed
        // nothing: there is no positive signal for "was dropped", so this waits out a generous
        // margin instead of polling for an effect that must not occur.
        for _ in 0..60 {
            pump_posted_messages(window.hwnd);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(
            crate::window::library_host::folder(window.hwnd),
            None,
            "a check that started before Close notebook must not reopen the notebook"
        );

        // A later, non-stale listed open still works normally.
        crate::window::library_host::open_listed_notebook(window.hwnd, &fresh.folder());
        pump_until(window.hwnd, || {
            crate::window::library_host::folder(window.hwnd) == Some(fresh.folder())
        });
    }

    #[test]
    fn a_note_inside_the_folder_autosaves_and_closing_it_never_prompts() {
        // Break caught: a notes-folder file still asking "Save changes?" or losing edits on close.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("autosave");
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        let path = open_note(&window, &scratch, "a.md", "one");
        editor.set_text("two").unwrap();
        assert_eq!(
            crate::window::library_host::autosave_active(window.hwnd),
            crate::window::library_host::Autosave::Saved
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "two");
        assert!(!app_mut(window.hwnd).tabs.active().unwrap().dirty);

        editor.set_text("three").unwrap();
        super::close_active_document(window.hwnd); // no answer_next_close_prompt: a prompt would fail the test
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "three");
    }

    #[test]
    fn a_file_changed_on_disk_pauses_autosave_until_the_user_chooses() {
        // Break caught: autosave silently overwriting an edit OneDrive just synced from another PC.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("guard");
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        let path = open_note(&window, &scratch, "a.md", "one");
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&path, "from the other PC").unwrap();
        editor.set_text("mine").unwrap();
        assert_eq!(
            crate::window::library_host::autosave_active(window.hwnd),
            crate::window::library_host::Autosave::Paused
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "from the other PC");
        assert!(
            notices(window.hwnd)
                .iter()
                .any(|n| n.contains("changed on disk"))
        );

        execute_command(window.hwnd, CommandId::NoteReloadFromDisk);
        assert_eq!(editor.text().unwrap(), "from the other PC");
        assert!(!app_mut(window.hwnd).tabs.active().unwrap().autosave_paused);

        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&path, "again").unwrap();
        editor.set_text("mine for real").unwrap();
        assert_eq!(
            crate::window::library_host::autosave_active(window.hwnd),
            crate::window::library_host::Autosave::Paused
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "again");
        assert!(app_mut(window.hwnd).tabs.active().unwrap().dirty);
        execute_command(window.hwnd, CommandId::NoteKeepMine);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "mine for real");
    }

    #[test]
    fn files_outside_the_folder_and_folders_with_autosave_off_are_not_autosaved() {
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("not-eligible");
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        scratch.install(window.hwnd);
        let outside = scratch.root.join("outside.md");
        std::fs::write(&outside, "x").unwrap();
        super::open_path(window.hwnd, &outside).unwrap();
        editor.set_text("y").unwrap();
        assert_eq!(
            crate::window::library_host::autosave_active(window.hwnd),
            crate::window::library_host::Autosave::NotEligible
        );
        assert_eq!(std::fs::read_to_string(&outside).unwrap(), "x");

        let inside = open_note(&window, &scratch, "b.md", "b");
        execute_command(window.hwnd, CommandId::ToggleFolderAutosave);
        editor.set_text("c").unwrap();
        assert_eq!(
            crate::window::library_host::autosave_active(window.hwnd),
            crate::window::library_host::Autosave::NotEligible
        );
        assert_eq!(std::fs::read_to_string(&inside).unwrap(), "b");
        assert!(
            !app_mut(window.hwnd)
                .library
                .state
                .as_ref()
                .unwrap()
                .local
                .autosave
        );
    }

    #[test]
    fn switching_tabs_autosaves_the_tab_being_left() {
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("switch-save");
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        let a = open_note(&window, &scratch, "a.md", "a");
        let b = scratch.note("b.md", "b");
        super::open_path(window.hwnd, &b).unwrap();
        editor.set_text("b2").unwrap();
        execute_command(window.hwnd, CommandId::SelectTab1);
        assert_eq!(std::fs::read_to_string(&b).unwrap(), "b2");
        editor.set_text("a2").unwrap();
        super::create_new_document(window.hwnd).unwrap();
        assert_eq!(
            std::fs::read_to_string(&a).unwrap(),
            "a2",
            "a new tab also leaves the old one"
        );
    }

    #[test]
    fn a_note_with_no_known_disk_stamp_pauses_instead_of_overwriting() {
        // Break caught: a tab restored from a snapshot autosaving over a file that changed while
        // FastPad was closed, because it has no stamp to compare against.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("no-stamp");
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        let path = open_note(&window, &scratch, "a.md", "on disk");
        let id = app_mut(window.hwnd).tabs.active().unwrap().id;
        app_mut(window.hwnd)
            .tabs
            .document_mut(id)
            .unwrap()
            .disk_stamp = None;
        editor.set_text("restored").unwrap();
        assert_eq!(
            crate::window::library_host::autosave_active(window.hwnd),
            crate::window::library_host::Autosave::Paused
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "on disk");
        assert!(app_mut(window.hwnd).tabs.active().unwrap().dirty);
    }

    #[test]
    fn a_single_edit_autosaves_once_the_idle_timer_fires() {
        // Break caught: the first keystroke after a save not arming the timer, because the edit
        // notification arrives before the tab is marked dirty.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("idle-timer");
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        let path = open_note(&window, &scratch, "a.md", "one");
        editor.replace_target(0..0, "x").unwrap();
        pump_until(window.hwnd, || {
            std::fs::read_to_string(&path).is_ok_and(|text| text == "xone")
        });
        assert!(!app_mut(window.hwnd).tabs.active().unwrap().dirty);
    }

    #[test]
    fn a_note_whose_file_was_deleted_and_has_no_stamp_is_not_recreated() {
        // Break caught: a restored tab re-creating a note the user deleted on another PC.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("no-stamp-deleted");
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        let path = open_note(&window, &scratch, "a.md", "on disk");
        let id = app_mut(window.hwnd).tabs.active().unwrap().id;
        app_mut(window.hwnd)
            .tabs
            .document_mut(id)
            .unwrap()
            .disk_stamp = None;
        std::fs::remove_file(&path).unwrap();
        editor.set_text("restored").unwrap();
        assert_eq!(
            crate::window::library_host::autosave_active(window.hwnd),
            crate::window::library_host::Autosave::Paused
        );
        assert!(!path.exists());
        assert!(app_mut(window.hwnd).tabs.active().unwrap().dirty);
    }

    #[test]
    fn a_folder_whose_state_has_not_loaded_is_not_autosaved() {
        // Break caught: a folder with autosave turned off being autosaved while it (re)loads.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("state-loading");
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        let path = open_note(&window, &scratch, "a.md", "one");
        execute_command(window.hwnd, CommandId::ToggleFolderAutosave);
        assert!(
            !app_mut(window.hwnd)
                .library
                .state
                .as_ref()
                .unwrap()
                .local
                .autosave
        );
        app_mut(window.hwnd).library.state = None;
        editor.set_text("two").unwrap();
        assert_eq!(
            crate::window::library_host::autosave_active(window.hwnd),
            crate::window::library_host::Autosave::NotEligible
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "one");
        assert!(app_mut(window.hwnd).tabs.active().unwrap().dirty);
    }

    #[test]
    fn keep_my_version_on_an_untitled_tab_does_nothing() {
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("keep-untitled");
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        scratch.install(window.hwnd);
        super::create_new_document(window.hwnd).unwrap();
        editor.set_text("draft").unwrap();
        let before = notices(window.hwnd).len();
        execute_command(window.hwnd, CommandId::NoteKeepMine);
        assert_eq!(notices(window.hwnd).len(), before);
        let active = app_mut(window.hwnd).tabs.active().unwrap();
        assert!(active.path.is_none() && active.dirty);
    }

    #[test]
    fn a_failed_autosave_names_the_note_once_and_keeps_the_tab_dirty() {
        // Break caught: a failed write marking the note clean, or two notices for one failure.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("autosave-fails");
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        let path = open_note(&window, &scratch, "a.md", "one");
        editor.set_text("two").unwrap();
        // A file held open without sharing makes the atomic replace fail; its stamp is unchanged.
        use std::os::windows::fs::OpenOptionsExt;
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&path)
            .unwrap();
        let before = notices(window.hwnd).len();
        let outcome = crate::window::library_host::autosave_active(window.hwnd);
        drop(lock);
        assert_eq!(outcome, crate::window::library_host::Autosave::Failed);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "one");
        assert!(app_mut(window.hwnd).tabs.active().unwrap().dirty);
        let added = &notices(window.hwnd)[before..];
        assert_eq!(added.len(), 1, "{added:?}");
        assert!(added[0].starts_with("Autosave failed for "), "{added:?}");
    }

    #[test]
    fn closing_the_window_autosaves_every_eligible_note_and_keeps_the_active_tab() {
        // Break caught: a close leaving a second dirty note unsaved, saving a file outside the
        // folder or a paused note, or leaving the session on whichever tab saved last.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("autosave-all");
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        let a = open_note(&window, &scratch, "a.md", "a");
        // Off while the tabs are made dirty, so switching between them does not save them.
        execute_command(window.hwnd, CommandId::ToggleFolderAutosave);
        editor.set_text("a2").unwrap();
        let b = scratch.note("b.md", "b");
        super::open_path(window.hwnd, &b).unwrap();
        editor.set_text("b2").unwrap();
        let paused = scratch.note("p.md", "p");
        super::open_path(window.hwnd, &paused).unwrap();
        editor.set_text("p2").unwrap();
        let paused_id = app_mut(window.hwnd).tabs.active().unwrap().id;
        app_mut(window.hwnd)
            .tabs
            .document_mut(paused_id)
            .unwrap()
            .autosave_paused = true;
        let outside = scratch.root.join("outside.md");
        std::fs::write(&outside, "x").unwrap();
        super::open_path(window.hwnd, &outside).unwrap();
        editor.set_text("x2").unwrap();
        let outside_id = app_mut(window.hwnd).tabs.active().unwrap().id;
        execute_command(window.hwnd, CommandId::ToggleFolderAutosave);

        crate::window::library_host::autosave_all(window.hwnd);
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "a2");
        assert_eq!(std::fs::read_to_string(&b).unwrap(), "b2");
        assert_eq!(std::fs::read_to_string(&paused).unwrap(), "p");
        assert_eq!(std::fs::read_to_string(&outside).unwrap(), "x");
        let app = app_mut(window.hwnd);
        let dirty = |path: &std::path::Path| {
            app.tabs
                .documents()
                .find(|document| document.path.as_deref() == Some(path))
                .unwrap()
                .dirty
        };
        assert!(!dirty(&a) && !dirty(&b));
        assert!(dirty(&paused) && dirty(&outside));
        assert_eq!(app.tabs.active().unwrap().id, outside_id);
    }

    #[test]
    fn switching_to_another_app_autosaves_the_active_note() {
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("deactivate");
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        let path = open_note(&window, &scratch, "a.md", "one");
        editor.set_text("two").unwrap();
        crate::window::library_host::activation_changed(window.hwnd, false);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "two");
        assert!(!app_mut(window.hwnd).tabs.active().unwrap().dirty);
    }

    fn library(hwnd: HWND) -> &'static crate::library::model::Library {
        &app_mut(hwnd).library.state.as_ref().unwrap().library
    }

    #[test]
    fn toggling_a_pin_applies_to_the_active_note_and_persists() {
        // Break caught: the pin command changing only memory, recording the wrong file, or an
        // unpin leaving a record behind.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("pin");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        let path = open_note(&window, &scratch, "a.md", "a");
        let hwnd = window.hwnd;

        execute_command(hwnd, CommandId::NoteTogglePin);
        assert!(
            app_mut(hwnd)
                .library
                .state
                .as_ref()
                .unwrap()
                .is_pinned(&path)
        );
        assert!(notices(hwnd).iter().any(|n| n == "Pinned."));
        crate::window::library_host::flush_now(hwnd);
        let ini = std::fs::read_to_string(crate::library::store::library_file(&scratch.folder()))
            .unwrap();
        assert!(ini.starts_with("version=2\r\n"), "{ini:?}");
        assert!(ini.contains("|p|") && ini.ends_with("|a.md\r\n"), "{ini:?}");
        let reloaded =
            crate::library::load(&scratch.folder(), &scratch.root.join("x.ini"), 0).unwrap();
        assert!(reloaded.is_pinned(&path));

        execute_command(hwnd, CommandId::NoteTogglePin);
        crate::window::library_host::flush_now(hwnd);
        assert!(notices(hwnd).iter().any(|n| n == "Unpinned."));
        assert!(
            library(hwnd).notes.is_empty(),
            "an unpinned note keeps no record"
        );
    }

    #[test]
    fn pinning_a_file_outside_the_open_notebook_is_refused() {
        // Break caught: a pin on a file outside the notebook writing an absolute-path record
        // that version 2 of library.ini cannot hold.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("pin-outside");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        scratch.install(window.hwnd);
        let outside = scratch.root.join("outside.md");
        std::fs::write(&outside, "x").unwrap();
        super::open_path(window.hwnd, &outside).unwrap();
        execute_command(window.hwnd, CommandId::NoteTogglePin);
        assert!(
            notices(window.hwnd)
                .iter()
                .any(|n| n == "Only notes in the open notebook can be pinned.")
        );
        let state = app_mut(window.hwnd).library.state.as_ref().unwrap();
        assert!(state.pending.is_empty());
        assert!(state.library.notes.is_empty());
    }

    #[test]
    fn an_untitled_tab_must_be_saved_before_it_can_be_organized() {
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("untitled-organize");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        scratch.install(window.hwnd);
        super::create_new_document(window.hwnd).unwrap();
        execute_command(window.hwnd, CommandId::NoteTogglePin);
        assert!(
            notices(window.hwnd)
                .iter()
                .any(|n| n == "Save this note first to organize it.")
        );
    }

    #[test]
    fn an_unreadable_library_disables_pinning_with_an_explanation() {
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("readonly");
        let ini = crate::library::store::library_file(&scratch.folder());
        std::fs::create_dir_all(ini.parent().unwrap()).unwrap();
        std::fs::write(&ini, "version=99\r\n").unwrap();
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        open_note(&window, &scratch, "a.md", "a");
        execute_command(window.hwnd, CommandId::NoteTogglePin);
        crate::window::library_host::flush_now(window.hwnd);
        assert_eq!(std::fs::read_to_string(&ini).unwrap(), "version=99\r\n");
        assert!(notices(window.hwnd).iter().any(|n| n.contains("read-only")));
    }

    #[test]
    fn renaming_a_note_renames_its_file_and_keeps_its_metadata() {
        // Break caught: a rename losing the note's pin, or the tab still pointing at the old path.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("rename");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        let old = open_note(&window, &scratch, "a.md", "text");
        execute_command(window.hwnd, CommandId::NoteTogglePin);
        execute_command(window.hwnd, CommandId::NoteRename);
        assert_eq!(
            app_mut(window.hwnd).name_box.as_ref().unwrap().text(),
            "a.md"
        );
        type_into_name_box(window.hwnd, "Plan");
        crate::window::library_host::name_box_submit(window.hwnd);
        let new = scratch.folder().join("Plan.md");
        assert!(!old.exists());
        assert_eq!(std::fs::read_to_string(&new).unwrap(), "text");
        assert_eq!(
            app_mut(window.hwnd).tabs.active().unwrap().path.as_deref(),
            Some(new.as_path())
        );
        let state = app_mut(window.hwnd).library.state.as_ref().unwrap();
        assert!(state.is_pinned(&new));
    }

    #[test]
    fn renaming_onto_an_existing_file_is_refused() {
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("rename-clash");
        scratch.note("b.md", "b");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        let a = open_note(&window, &scratch, "a.md", "a");
        execute_command(window.hwnd, CommandId::NoteRename);
        type_into_name_box(window.hwnd, "B.md");
        crate::window::library_host::name_box_submit(window.hwnd);
        assert!(a.exists());
        assert_eq!(
            std::fs::read_to_string(scratch.folder().join("b.md")).unwrap(),
            "b"
        );
        assert!(
            app_mut(window.hwnd)
                .name_box
                .as_ref()
                .unwrap()
                .error()
                .is_some()
        );
    }

    #[test]
    fn renaming_a_note_changing_only_letter_case_works() {
        // Break caught: the clash check or the tab collision check treating the note's own file
        // as a different one that already has the new name.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("rename-case");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        open_note(&window, &scratch, "plan.md", "text");
        execute_command(window.hwnd, CommandId::NoteRename);
        type_into_name_box(window.hwnd, "Plan.md");
        crate::window::library_host::name_box_submit(window.hwnd);
        let names: Vec<String> = std::fs::read_dir(scratch.folder())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".md"))
            .collect();
        assert_eq!(names, ["Plan.md"]);
        let new = scratch.folder().join("Plan.md");
        assert_eq!(
            app_mut(window.hwnd)
                .tabs
                .active()
                .unwrap()
                .path
                .as_deref()
                .and_then(|path| path.file_name())
                .map(|name| name.to_string_lossy().into_owned()),
            Some("Plan.md".to_owned())
        );
        assert!(!name_box_visible(window.hwnd));
        assert_eq!(std::fs::read_to_string(&new).unwrap(), "text");
    }

    #[test]
    fn deleting_a_note_asks_then_recycles_it_and_keeps_its_record_hidden() {
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("delete-note");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        let path = open_note(&window, &scratch, "a.md", "a");
        execute_command(window.hwnd, CommandId::NoteTogglePin);
        crate::window::answer_next_confirm(|_| false);
        execute_command(window.hwnd, CommandId::NoteDelete);
        assert!(path.exists());
        crate::window::answer_next_confirm(|_| true);
        execute_command(window.hwnd, CommandId::NoteDelete);
        assert!(!path.exists());
        assert_eq!(super::tab_count(window.hwnd), 0);
        let state = app_mut(window.hwnd).library.state.as_ref().unwrap();
        let record = state.record_for(&path).unwrap();
        assert!(record.deleted);
        assert!(state.local.missing_since(record.id).is_some());
        assert!(state.notes.is_empty());
    }

    #[test]
    fn a_note_renamed_outside_fastpad_moves_its_open_tab_and_autosave_resumes() {
        // Break caught: the rescan looking the tab up through the old, now missing path, so the
        // tab kept it: autosave stayed paused, and Keep mine or Ctrl+S re-created the old file
        // while the metadata followed the new one.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("outside-rename");
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        let old = open_note(&window, &scratch, "a.md", "text");
        execute_command(window.hwnd, CommandId::NoteTogglePin);
        crate::window::library_host::flush_now(window.hwnd);
        let new = scratch.folder().join("b.md");
        std::fs::rename(&old, &new).unwrap();
        editor.set_text("edited").unwrap();
        assert_eq!(
            crate::window::library_host::autosave_active(window.hwnd),
            crate::window::library_host::Autosave::Paused,
            "the old file is gone, so autosave pauses until the rescan"
        );

        scratch.install(window.hwnd);

        let active = app_mut(window.hwnd).tabs.active().unwrap();
        assert_eq!(active.path.as_deref(), Some(new.as_path()));
        assert!(!active.autosave_paused, "the moved file is unchanged");
        assert_eq!(
            crate::window::library_host::autosave_active(window.hwnd),
            crate::window::library_host::Autosave::Saved
        );
        assert_eq!(std::fs::read_to_string(&new).unwrap(), "edited");
        assert!(!old.exists(), "the old name is not re-created");
        let state = app_mut(window.hwnd).library.state.as_ref().unwrap();
        assert!(state.record_for(&new).unwrap().pinned);
    }

    #[test]
    fn a_note_moved_and_changed_outside_fastpad_follows_but_does_not_autosave_over_the_change() {
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("outside-rename-changed");
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        let old = open_note(&window, &scratch, "a.md", "text");
        execute_command(window.hwnd, CommandId::NoteTogglePin);
        crate::window::library_host::flush_now(window.hwnd);
        let new = scratch.folder().join("b.md");
        std::fs::rename(&old, &new).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::OpenOptions::new()
            .append(true)
            .open(&new)
            .and_then(|mut file| std::io::Write::write_all(&mut file, b" and more"))
            .unwrap();

        scratch.install(window.hwnd);
        editor.set_text("mine").unwrap();

        assert_eq!(
            app_mut(window.hwnd).tabs.active().unwrap().path.as_deref(),
            Some(new.as_path())
        );
        assert_eq!(
            crate::window::library_host::autosave_active(window.hwnd),
            crate::window::library_host::Autosave::Paused
        );
        assert_eq!(std::fs::read_to_string(&new).unwrap(), "text and more");
    }

    #[test]
    fn renaming_to_the_prefilled_name_keeps_a_non_note_or_extensionless_file_as_it_is() {
        // Break caught: "script.py" prefilled and submitted as-is becoming script.py.py, and an
        // extensionless README gaining ".md".
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("rename-kinds");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        for name in ["script.py", "README"] {
            let path = open_note(&window, &scratch, name, "x");
            execute_command(window.hwnd, CommandId::NoteRename);
            assert_eq!(app_mut(window.hwnd).name_box.as_ref().unwrap().text(), name);
            crate::window::library_host::name_box_submit(window.hwnd);
            assert!(!name_box_visible(window.hwnd));
            assert!(path.exists(), "{name} is left alone");
            assert_eq!(
                app_mut(window.hwnd).tabs.active().unwrap().path.as_deref(),
                Some(path.as_path())
            );
        }
        let mut names: Vec<String> = std::fs::read_dir(scratch.folder())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| !name.starts_with('.'))
            .collect();
        names.sort();
        assert_eq!(names, ["README", "script.py"]);

        execute_command(window.hwnd, CommandId::NoteRename);
        type_into_name_box(window.hwnd, "tool");
        crate::window::library_host::name_box_submit(window.hwnd);
        assert!(
            scratch.folder().join("tool").exists(),
            "no extension is added"
        );
    }

    #[test]
    fn turning_notes_mode_on_labels_every_untitled_tab_without_switching_to_it() {
        // Break caught: only the active tab getting its label, every other untitled tab reading
        // "Untitled" until the user visited it.
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        app_mut(window.hwnd).settings.notes_mode = false;
        super::create_new_document(window.hwnd).unwrap();
        editor.set_text("First idea\nbody").unwrap();
        let first = app_mut(window.hwnd).tabs.active().unwrap().id;
        super::create_new_document(window.hwnd).unwrap();
        editor.set_text("Second idea").unwrap();
        pump_posted_messages(window.hwnd);
        let title = |id| app_mut(window.hwnd).tabs.document(id).unwrap().title();
        assert_eq!(title(first), "Untitled *");

        let settings = RecoveryScratch::new("labels-toggle");
        super::save_settings_to(Some(settings.path().join("fastpad.ini")));
        execute_command(window.hwnd, CommandId::ToggleNotesMode);
        super::save_settings_to(None);
        assert!(app_mut(window.hwnd).settings.notes_mode);

        assert_eq!(title(first), "First idea *");
        let active = app_mut(window.hwnd).tabs.active().unwrap().id;
        assert_ne!(active, first, "no tab switch");
        assert_eq!(title(active), "Second idea *");
    }

    #[test]
    fn restored_and_recovered_untitled_tabs_are_labelled_from_their_text() {
        // Break caught: a background untitled tab restored from the session reading "Untitled"
        // because only the active tab's label is computed.
        let _scintilla = load_native_scintilla();
        let scratch = RecoveryScratch::new("session-labels");
        let recovery = scratch.path().join("Recovery");
        let first_id = RecoveryId::from_u128(0x1ab1);
        let second_id = RecoveryId::from_u128(0x1ab2);
        for (id, text) in [(first_id, "# Shopping\nmilk"), (second_id, "Plans")] {
            write_snapshot(&recovery, &Snapshot::new(id, None, Encoding::Utf8, text)).unwrap();
        }
        write_session(
            &scratch,
            vec![
                SessionEntry::new(SessionSource::Snapshot(first_id)),
                SessionEntry::new(SessionSource::Snapshot(second_id)),
            ],
            1,
        );
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        enable_session(window.hwnd, &scratch);

        run_session_restore(window.hwnd);

        {
            let app = app_mut(window.hwnd);
            let documents = app.tabs.documents().collect::<Vec<_>>();
            assert_eq!(documents[0].title(), "Shopping *");
            assert_eq!(documents[0].label_watch, 0);
            assert_eq!(documents[1].title(), "Plans *");
        }

        // A crash-recovered tab keeps its "Recovered:" title but knows its label for a save.
        let root = RecoveryScratch::new("recovered-label");
        write_snapshot(
            root.path(),
            &Snapshot::new(
                RecoveryId::from_u128(0x1ab3),
                None,
                Encoding::Utf8,
                "Lost thought",
            ),
        )
        .unwrap();
        app_mut(window.hwnd).recovery_root = Some(root.path().to_path_buf());
        super::recover_snapshots(window.hwnd);
        let recovered = app_mut(window.hwnd).tabs.active().unwrap();
        assert_eq!(recovered.untitled_label.as_deref(), Some("Lost thought"));
    }

    #[test]
    fn a_metadata_flush_leaves_the_local_file_alone_and_an_expansion_change_writes_it() {
        // Break caught: every 500 ms metadata flush, every rescan or every opened note
        // re-encoding and rewriting the whole per-PC local file (with its scan cache) on the UI
        // thread.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("local-untouched");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        open_note(&window, &scratch, "a.md", "a");
        crate::window::library_host::flush_now(window.hwnd);
        let local = crate::library::local::local_file(&scratch.data(), &scratch.folder());
        assert!(local.exists(), "the load wrote it");
        std::fs::remove_file(&local).unwrap();

        let b = scratch.note("b.md", "b");
        super::open_path(window.hwnd, &b).unwrap();
        crate::window::library_host::flush_now(window.hwnd);
        assert!(!local.exists(), "opening a note changes nothing local");

        execute_command(window.hwnd, CommandId::NoteTogglePin);
        crate::window::library_host::flush_now(window.hwnd);
        assert!(crate::library::store::library_file(&scratch.folder()).exists());
        assert!(!local.exists(), "only library.ini changed");

        crate::window::library_host::with_state(window.hwnd, |state| {
            state.local.set_expanded(std::path::Path::new("sub"), true);
        });
        crate::window::library_host::flush_now(window.hwnd);
        assert!(
            std::fs::read_to_string(&local)
                .unwrap()
                .contains("expanded=sub\r\n"),
            "an expansion change is written"
        );
    }

    #[test]
    fn the_library_step_checks_no_folder_on_the_ui_thread_and_the_worker_falls_back() {
        // Break caught: the startup existence check on a remembered folder on an offline mapped
        // drive stalling the UI thread for an SMB timeout.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("startup-gone");
        let gone = scratch.root.join("gone");
        let mut folders = crate::library::local::RecentFolders::default();
        folders.push(gone.clone());
        crate::library::local::write_folders(
            &crate::library::local::folders_file(&scratch.data()),
            &folders,
        )
        .unwrap();
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        app_mut(window.hwnd).library.data_dir = Some(scratch.data());

        let before = crate::library::folder_checks();
        crate::window::library_host::open_library_step(window.hwnd);
        assert_eq!(crate::library::folder_checks(), before, "no stat here");
        assert_eq!(crate::window::library_host::folder(window.hwnd), Some(gone));

        pump_until(window.hwnd, || app_mut(window.hwnd).library.state.is_some());
        let fallback = crate::library::normalize_folder(
            &crate::platform::paths::default_notes_folder().unwrap(),
        );
        assert_eq!(
            crate::window::library_host::folder(window.hwnd),
            Some(fallback)
        );
        assert!(
            notices(window.hwnd)
                .iter()
                .any(|n| n.contains("could not find the notebook")),
            "{:?}",
            notices(window.hwnd)
        );
    }

    #[test]
    fn a_session_that_ended_with_no_notebook_open_opens_none_at_startup() {
        // Break caught: open=none ignored, so a closed notebook came back at the next start, or
        // a worker started (and stat-ed a folder) to find out there was nothing to open.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("startup-closed");
        let mut folders = crate::library::local::RecentFolders::default();
        folders.push(scratch.folder());
        folders.set_closed(true);
        crate::library::local::write_folders(
            &crate::library::local::folders_file(&scratch.data()),
            &folders,
        )
        .unwrap();
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        app_mut(window.hwnd).library.data_dir = Some(scratch.data());

        let before = crate::library::folder_checks();
        crate::window::library_host::open_library_step(window.hwnd);
        assert_eq!(crate::library::folder_checks(), before);
        assert_eq!(crate::window::library_host::folder(window.hwnd), None);
        assert!(!app_mut(window.hwnd).library.scanning, "no worker starts");
    }

    #[test]
    fn turning_notes_mode_off_says_so_when_metadata_cannot_be_written_and_closes_the_name_box() {
        // Break caught: the toggle dropping unsaved pins silently when the flush
        // failed, or leaving a name box open that did nothing on Enter.
        let _scintilla = load_native_scintilla();
        let (scratch, window, _editor) = open_first_save_box("mode-off-flush");
        let a = scratch.note("a.md", "a");
        crate::window::library_host::with_state(window.hwnd, |state| {
            let mut ids = crate::library::ids::IdSource::new(1, 1);
            let target = state.note_ref(&mut ids, &a);
            state
                .apply(crate::library::ops::PendingOp::SetPinned {
                    note: target,
                    value: true,
                })
                .unwrap();
        });
        std::fs::write(scratch.folder().join(".fastpad"), "not a directory").unwrap();

        app_mut(window.hwnd).settings.notes_mode = false;
        crate::window::library_host::notes_mode_changed(window.hwnd, false);

        assert!(!name_box_visible(window.hwnd));
        assert!(
            app_mut(window.hwnd).library.state.is_none(),
            "the setting applies"
        );
        let expected = format!(
            "Metadata changes could not be written to {}",
            scratch
                .folder()
                .join(".fastpad")
                .join("library.ini")
                .display()
        );
        assert!(
            notices(window.hwnd).contains(&expected),
            "{:?}",
            notices(window.hwnd)
        );
    }

    #[test]
    fn deleting_a_note_with_unsaved_edits_says_they_are_discarded() {
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("delete-dirty");
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        let path = open_note(&window, &scratch, "a.md", "a");
        crate::window::answer_next_confirm(|_| false);
        execute_command(window.hwnd, CommandId::NoteDelete);
        assert_eq!(
            crate::window::modal::take_last_confirm().as_deref(),
            Some("Move \u{201c}a.md\u{201d} to the Recycle Bin?")
        );
        editor.set_text("unsaved").unwrap();
        crate::window::answer_next_confirm(|_| false);
        execute_command(window.hwnd, CommandId::NoteDelete);
        assert_eq!(
            crate::window::modal::take_last_confirm().as_deref(),
            Some("Move \u{201c}a.md\u{201d} to the Recycle Bin and discard unsaved changes?")
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "a",
            "nothing is autosaved first"
        );
    }

    #[test]
    fn opening_another_folder_autosaves_the_old_folders_notes_and_normalizes_the_new_path() {
        // Break caught: a dirty note in the old folder left unsaved (and no longer autosaved)
        // after a switch, or a folder spelled with a trailing separator becoming a second
        // recent folder.
        let _scintilla = load_native_scintilla();
        let first = LibraryScratch::new("switch-autosave-a");
        let second = LibraryScratch::new("switch-autosave-b");
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        app_mut(window.hwnd).library.data_dir = Some(first.data());
        let a = open_note(&window, &first, "a.md", "one");
        // Off while editing, so nothing but the switch saves it.
        execute_command(window.hwnd, CommandId::ToggleFolderAutosave);
        editor.set_text("two").unwrap();
        execute_command(window.hwnd, CommandId::ToggleFolderAutosave);

        let spelled = std::path::PathBuf::from(format!("{}\\", second.folder().display()));
        crate::window::library_host::open_folder(window.hwnd, &spelled);

        assert_eq!(std::fs::read_to_string(&a).unwrap(), "two");
        assert_eq!(
            crate::window::library_host::folder(window.hwnd),
            Some(second.folder())
        );
        let recent = crate::library::local::read_folders(&crate::library::local::folders_file(
            &first.data(),
        ));
        assert_eq!(recent.folders.first(), Some(&second.folder()));
        pump_until(window.hwnd, || app_mut(window.hwnd).library.state.is_some());
    }

    #[test]
    fn a_library_file_held_open_by_a_sync_keeps_the_operations_and_retries_without_a_notice() {
        // Break caught: a sharing violation on library.ini turning organizing off or dropping
        // the pending pins with an error notice.
        use std::os::windows::fs::OpenOptionsExt;
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("busy-flush-window");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        let a = open_note(&window, &scratch, "a.md", "a");
        execute_command(window.hwnd, CommandId::NoteTogglePin);
        // Another PC's sync writes the file, so the flush must re-read it, and holds it open.
        let ini = crate::library::store::library_file(&scratch.folder());
        crate::library::store::write(&ini, &crate::library::model::Library::default()).unwrap();
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&ini)
            .unwrap();
        let before = notices(window.hwnd).len();
        crate::window::library_host::flush_now(window.hwnd);
        drop(lock);
        assert_eq!(
            notices(window.hwnd).len(),
            before,
            "no notice for a brief sync"
        );
        let state = app_mut(window.hwnd).library.state.as_ref().unwrap();
        assert_eq!(state.metadata, crate::library::Metadata::Ready);
        assert_eq!(state.pending.len(), 1);

        crate::window::library_host::flush_now(window.hwnd);
        let reloaded =
            crate::library::load(&scratch.folder(), &scratch.root.join("x.ini"), 0).unwrap();
        assert!(reloaded.is_pinned(&a));
    }

    use crate::library::tree::RowKind;
    use crate::window::notebook_view::{Activation, Mode, NotebookView};

    /// Task 6 creates the sidebar with the window when notes mode is on; this makes sure of it.
    fn ensure_sidebar(hwnd: HWND) {
        if app_mut(hwnd).sidebar.is_none() {
            crate::window::side_panel::notes_mode_changed(hwnd, true);
        }
    }

    fn notebook_view<'a>(hwnd: HWND) -> &'a mut NotebookView {
        &mut app_mut(hwnd).sidebar.as_mut().unwrap().notebook
    }

    fn row_of(hwnd: HWND, kind: &RowKind) -> usize {
        crate::library::tree::row_index(&notebook_view(hwnd).rows, kind)
            .unwrap_or_else(|| panic!("{kind:?} is not in {:?}", notebook_view(hwnd).rows))
    }

    fn selected_kind(hwnd: HWND) -> Option<RowKind> {
        let view = notebook_view(hwnd);
        view.list
            .selected
            .and_then(|index| view.rows.get(index))
            .map(|row| row.kind.clone())
    }

    fn select_row(hwnd: HWND, kind: &RowKind) {
        let index = row_of(hwnd, kind);
        notebook_view(hwnd).list.selected = Some(index);
    }

    fn rescan_and_wait(hwnd: HWND) {
        crate::window::library_host::request_rescan(hwnd);
        pump_until(hwnd, || !app_mut(hwnd).library.scanning);
    }

    #[test]
    fn a_rescan_keeps_selection_and_expansion_by_path() {
        // Break caught: a rescan that rebuilds the rows and keeps the selected index, so the
        // highlight jumps to another note; one that collapses the folder the user had open; or a
        // vanished selection left pointing past the end of the list.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("rescan-selection");
        std::fs::create_dir_all(scratch.folder().join("sub")).unwrap();
        scratch.note(r"sub\b.md", "b");
        scratch.note("c.md", "c");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        ensure_sidebar(window.hwnd);
        app_mut(window.hwnd).library.data_dir = Some(scratch.data());
        scratch.install(window.hwnd);
        crate::window::library_host::set_expanded(window.hwnd, std::path::Path::new("sub"), true);
        crate::window::notebook_view::rebuild(window.hwnd);
        let b = RowKind::Note(r"sub\b.md".into());
        select_row(window.hwnd, &b);

        scratch.note(r"sub\a.md", "a");
        rescan_and_wait(window.hwnd);
        assert_eq!(
            selected_kind(window.hwnd),
            Some(b.clone()),
            "followed by path"
        );
        let sub = row_of(window.hwnd, &RowKind::Folder("sub".into()));
        assert!(notebook_view(window.hwnd).rows[sub].expanded);
        assert!(row_of(window.hwnd, &RowKind::Note(r"sub\a.md".into())) < row_of(window.hwnd, &b));

        let before = notebook_view(window.hwnd).list.selected.unwrap();
        std::fs::remove_file(scratch.folder().join(r"sub\b.md")).unwrap();
        rescan_and_wait(window.hwnd);
        let view = notebook_view(window.hwnd);
        let after = view
            .list
            .selected
            .expect("the selection moves, it does not vanish");
        assert!(after < view.rows.len());
        assert_eq!(after, before.min(view.rows.len() - 1));
        assert!(view.rows[sub].expanded);
    }

    #[test]
    fn clicking_a_note_row_opens_the_preview_and_a_double_click_keeps_it() {
        // Break caught: a click opening a normal tab every time (tabs pile up), or a double-click
        // opening a second tab instead of keeping the preview.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("view-click");
        let a = scratch.note("a.md", "a");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        ensure_sidebar(window.hwnd);
        scratch.install(window.hwnd);
        let row = row_of(window.hwnd, &RowKind::Note("a.md".into()));

        crate::window::notebook_view::activate(window.hwnd, row, Activation::Click);
        let active = app_mut(window.hwnd).tabs.active().unwrap();
        assert_eq!(active.path.as_deref(), Some(a.as_path()));
        assert!(active.preview);

        let row = row_of(window.hwnd, &RowKind::Note("a.md".into()));
        crate::window::notebook_view::activate(window.hwnd, row, Activation::Permanent);
        assert_eq!(super::tab_count(window.hwnd), 1);
        assert!(!app_mut(window.hwnd).tabs.active().unwrap().preview);
    }

    #[test]
    fn switching_to_a_note_in_a_subfolder_selects_its_row_and_expands_its_folders() {
        // Break caught: the tree not following the active tab, or following it into a collapsed
        // folder so the selected row is hidden, or forgetting that expansion at the next start.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("view-reveal");
        std::fs::create_dir_all(scratch.folder().join(r"sub\deep")).unwrap();
        let b = scratch.note(r"sub\deep\b.md", "b");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        ensure_sidebar(window.hwnd);
        app_mut(window.hwnd).library.data_dir = Some(scratch.data());
        scratch.install(window.hwnd);

        super::open_path(window.hwnd, &b).unwrap();
        crate::window::side_panel::active_tab_changed(window.hwnd);

        assert_eq!(
            selected_kind(window.hwnd),
            Some(RowKind::Note(r"sub\deep\b.md".into()))
        );
        let expanded = crate::window::library_host::expanded(window.hwnd);
        assert!(expanded.contains(&std::path::PathBuf::from("sub")));
        assert!(expanded.contains(&std::path::PathBuf::from(r"sub\deep")));
        let local = crate::library::local::local_file(&scratch.data(), &scratch.folder());
        let written = crate::library::local::read(&local, &scratch.folder());
        assert!(
            written
                .expanded
                .contains(&std::path::PathBuf::from(r"sub\deep"))
        );
    }

    #[test]
    fn right_expands_a_folder_then_enters_it_and_left_climbs_back_out() {
        // Break caught: arrow keys that only move up and down, so a folder cannot be opened from
        // the keyboard, or Left on a child that does nothing.
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_LEFT, VK_RIGHT};
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("view-keys");
        std::fs::create_dir_all(scratch.folder().join("sub")).unwrap();
        scratch.note(r"sub\a.md", "a");
        scratch.note("z.md", "z");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        ensure_sidebar(window.hwnd);
        app_mut(window.hwnd).library.data_dir = Some(scratch.data());
        scratch.install(window.hwnd);
        let sub = RowKind::Folder("sub".into());
        select_row(window.hwnd, &sub);
        let key = |key| crate::window::notebook_view::key_down(window.hwnd, key);

        assert!(key(VK_RIGHT));
        assert!(notebook_view(window.hwnd).rows[row_of(window.hwnd, &sub)].expanded);
        assert!(key(VK_RIGHT));
        assert_eq!(
            selected_kind(window.hwnd),
            Some(RowKind::Note(r"sub\a.md".into()))
        );
        assert!(key(VK_LEFT));
        assert_eq!(selected_kind(window.hwnd), Some(sub.clone()));
        assert!(key(VK_LEFT));
        assert!(!notebook_view(window.hwnd).rows[row_of(window.hwnd, &sub)].expanded);
    }

    #[test]
    fn the_view_says_loading_then_shows_the_tree_and_recent_notebooks_once_closed() {
        // Break caught: an empty panel while the worker loads, a tree left on screen after Close
        // notebook, or a no-notebook state without the RECENT list.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("view-states");
        scratch.note("a.md", "a");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        ensure_sidebar(window.hwnd);
        app_mut(window.hwnd).library.data_dir = Some(scratch.data());
        crate::library::local::write_folders(
            &crate::library::local::folders_file(&scratch.data()),
            &crate::library::local::RecentFolders {
                folders: vec![scratch.folder()],
                ..Default::default()
            },
        )
        .unwrap();

        app_mut(window.hwnd).library.folder = Some(scratch.folder());
        crate::window::notebook_view::rebuild(window.hwnd);
        assert_eq!(notebook_view(window.hwnd).mode, Mode::Loading);

        scratch.install(window.hwnd);
        assert_eq!(notebook_view(window.hwnd).mode, Mode::Tree);

        execute_command(window.hwnd, CommandId::CloseNotebook);
        assert_eq!(notebook_view(window.hwnd).mode, Mode::NoNotebook);
        assert_eq!(notebook_view(window.hwnd).recent, vec![scratch.folder()]);
    }

    #[test]
    fn a_failed_load_offers_retry_and_open_instead_of_loading_forever() {
        // Break caught: a notebook whose load failed showing "Loading…" in both views for good,
        // with no way to try again but reopening it.
        use windows_sys::Win32::Foundation::LPARAM;
        use windows_sys::Win32::UI::WindowsAndMessaging::{WM_LBUTTONDOWN, WM_LBUTTONUP};
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("view-failed");
        scratch.note("a.md", "a");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        ensure_sidebar(window.hwnd);
        app_mut(window.hwnd).library.data_dir = Some(scratch.data());
        app_mut(window.hwnd).library.folder = Some(scratch.folder());
        let generation = app_mut(window.hwnd).library.generation;
        crate::window::library_host::library_ready(
            window.hwnd,
            crate::window::library_host::test_ready_payload(generation, Err("boom".to_owned())),
        );
        assert_eq!(notebook_view(window.hwnd).mode, Mode::Failed);
        let panel = notebook_view(window.hwnd).panel;
        let mut client = RECT::default();
        unsafe { GetClientRect(panel, &mut client) };
        let dpi = unsafe { GetDpiForWindow(panel) }.max(96);
        let buttons = notebook_view(window.hwnd).buttons(client, dpi);
        let names: Vec<&str> = buttons.iter().map(|(name, _)| name.as_str()).collect();
        assert!(
            names.ends_with(&["Retry", "Open notebook…"]),
            "exposed to screen readers like the empty-state buttons: {names:?}"
        );
        crate::window::side_panel::show_view(
            window.hwnd,
            crate::config::SidebarView::Search,
            false,
        );
        assert_eq!(
            crate::window::search_view::status(window.hwnd),
            Some(crate::window::notebook_view::LOAD_FAILED)
        );

        crate::window::side_panel::show_view(
            window.hwnd,
            crate::config::SidebarView::Notebook,
            false,
        );
        let retry = buttons[buttons.len() - 2].1;
        let lparam = ((((retry.top + retry.bottom) / 2) as u32) << 16
            | ((retry.left + retry.right) / 2) as u32) as LPARAM;
        unsafe {
            SendMessageW(panel, WM_LBUTTONDOWN, 0, lparam);
            SendMessageW(panel, WM_LBUTTONUP, 0, lparam);
        }
        assert_eq!(
            notebook_view(window.hwnd).mode,
            Mode::Loading,
            "Retry clears the failure and loads the same notebook again"
        );
        pump_until(window.hwnd, || app_mut(window.hwnd).library.state.is_some());
        assert_eq!(notebook_view(window.hwnd).mode, Mode::Tree);
    }

    #[test]
    fn at_startup_both_views_say_loading_for_the_notebook_being_opened() {
        // Break caught: the Search view saying "Open a notebook to search it." while the
        // remembered notebook loads.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("startup-loading");
        scratch.note("a.md", "a");
        write_notebooks(&scratch.data(), vec![scratch.folder()], vec![]);
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        ensure_sidebar(window.hwnd);
        app_mut(window.hwnd).library.data_dir = Some(scratch.data());
        crate::window::side_panel::show_view(
            window.hwnd,
            crate::config::SidebarView::Search,
            false,
        );
        assert_eq!(
            crate::window::search_view::status(window.hwnd),
            Some(crate::window::search_view::NO_NOTEBOOK)
        );

        crate::window::library_host::open_library_step(window.hwnd);
        assert_eq!(
            crate::window::search_view::status(window.hwnd),
            Some(crate::window::search_view::LOADING)
        );
        assert_eq!(notebook_view(window.hwnd).mode, Mode::Loading);
        pump_until(window.hwnd, || app_mut(window.hwnd).library.state.is_some());
        assert_eq!(crate::window::search_view::status(window.hwnd), None);
    }

    #[test]
    fn an_empty_notebook_says_so_until_an_untitled_tab_appears_as_an_unsaved_row() {
        // Break caught: a blank panel for a notebook with no notes, or a new untitled tab that the
        // tree does not show until it is saved.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("view-empty");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        ensure_sidebar(window.hwnd);
        scratch.install(window.hwnd);
        let start = app_mut(window.hwnd).tabs.active().unwrap().id;
        super::close_document_without_prompt(window.hwnd, start);
        crate::window::notebook_view::rebuild(window.hwnd);
        assert_eq!(notebook_view(window.hwnd).mode, Mode::Empty);

        execute_command(window.hwnd, CommandId::New);
        crate::window::notebook_view::rebuild(window.hwnd);
        let id = app_mut(window.hwnd).tabs.active().unwrap().id;
        let view = notebook_view(window.hwnd);
        assert_eq!(view.mode, Mode::Tree);
        assert_eq!(view.rows[0].kind, RowKind::Unsaved(id.0));
        assert_eq!(view.rows[0].name, "Untitled");
    }

    #[test]
    fn the_header_star_favorites_the_notebook_and_every_state_paints() {
        // Break caught: a star that does nothing, or a paint path that panics on an empty tree,
        // the loading state or the no-notebook state.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("view-star");
        scratch.note("a.md", "a");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        ensure_sidebar(window.hwnd);
        app_mut(window.hwnd).library.data_dir = Some(scratch.data());
        scratch.install(window.hwnd);

        crate::window::notebook_view::header_clicked(
            window.hwnd,
            crate::window::notebook_view::HeaderButton::Favorite,
        );
        assert!(crate::window::library_host::is_favorite(window.hwnd));

        let panel = notebook_view(window.hwnd).panel;
        let area = RECT {
            left: 0,
            top: 0,
            right: 260,
            bottom: 400,
        };
        let dc = unsafe { windows_sys::Win32::Graphics::Gdi::GetDC(panel) };
        let paint = |hwnd: HWND| {
            let view_paint = crate::window::side_panel::view_paint(hwnd, panel, dc, area);
            crate::window::notebook_view::paint(hwnd, &view_paint);
        };
        paint(window.hwnd);
        execute_command(window.hwnd, CommandId::CloseNotebook);
        paint(window.hwnd);
        app_mut(window.hwnd).library.folder = Some(scratch.folder());
        crate::window::notebook_view::rebuild(window.hwnd);
        paint(window.hwnd);
        unsafe { windows_sys::Win32::Graphics::Gdi::ReleaseDC(panel, dc) };
    }

    #[test]
    fn switching_tabs_in_an_unchanged_notebook_and_typing_a_first_line_do_not_reflatten() {
        // Break caught: every tab switch, and every keystroke in an untitled tab's first line,
        // flattening the whole tree again (tens of milliseconds in a big, expanded notebook).
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("view-no-reflatten");
        let a = scratch.note("a.md", "a");
        let b = scratch.note("b.md", "b");
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        ensure_sidebar(window.hwnd);
        scratch.install(window.hwnd);
        super::open_path(window.hwnd, &a).unwrap();
        super::open_path(window.hwnd, &b).unwrap();
        let id_of =
            |path: &std::path::Path| app_mut(window.hwnd).tabs.find_stored_path(path).unwrap();
        let (a_id, b_id) = (id_of(&a), id_of(&b));
        // The deferred startup steps (theme, chrome) refresh the sidebar once; let them run.
        pump_posted_messages(window.hwnd);
        let before = notebook_view(window.hwnd).rebuilds;

        assert!(super::activate_document_by_id(window.hwnd, a_id));
        assert_eq!(
            selected_kind(window.hwnd),
            Some(RowKind::Note("a.md".into()))
        );
        assert!(super::activate_document_by_id(window.hwnd, b_id));
        assert_eq!(
            selected_kind(window.hwnd),
            Some(RowKind::Note("b.md".into()))
        );
        assert_eq!(notebook_view(window.hwnd).rebuilds, before, "no re-flatten");

        // A new untitled tab adds a row, so that switch does rebuild; typing its first line
        // then only renames the row.
        execute_command(window.hwnd, CommandId::New);
        let untitled = app_mut(window.hwnd).tabs.active().unwrap().id;
        let after_new = notebook_view(window.hwnd).rebuilds;
        assert_eq!(after_new, before + 1, "Ctrl+N flattens the tree once");
        editor.set_text("Groceries").unwrap();
        pump_posted_messages(window.hwnd);
        let row = row_of(window.hwnd, &RowKind::Unsaved(untitled.0));
        assert_eq!(notebook_view(window.hwnd).rows[row].name, "Groceries");
        assert_eq!(notebook_view(window.hwnd).rebuilds, after_new);
        assert!(super::activate_document_by_id(window.hwnd, a_id));
        assert_eq!(
            notebook_view(window.hwnd).rebuilds,
            after_new,
            "the renamed row's label is part of the key, so switching away does not rebuild"
        );
    }

    #[test]
    fn switching_sidebar_views_saves_the_view_without_restyling_the_editor_or_reflattening() {
        // Break caught: every view switch (and every panel resize) re-applying the editor
        // settings, which lays the Markdown preview out again, or flattening an unchanged tree
        // each time the Notebook view comes back.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("view-switch-cheap");
        scratch.note("a.md", "a");
        let settings = scratch.root.join("fastpad.ini");
        super::save_settings_to(Some(settings.clone()));
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        ensure_sidebar(window.hwnd);
        scratch.install(window.hwnd);
        pump_posted_messages(window.hwnd);
        let applied = super::editor_settings_applied();
        let rebuilds = notebook_view(window.hwnd).rebuilds;
        use crate::config::SidebarView;

        crate::window::side_panel::show_view(window.hwnd, SidebarView::Search, false);
        crate::window::side_panel::show_view(window.hwnd, SidebarView::Notebook, false);
        super::save_settings_to(None);

        assert_eq!(super::editor_settings_applied(), applied);
        assert_eq!(notebook_view(window.hwnd).rebuilds, rebuilds);
        assert_eq!(
            crate::window::side_panel::current_view(window.hwnd),
            SidebarView::Notebook
        );
        let saved = std::fs::read_to_string(&settings).unwrap();
        assert!(saved.contains("sidebar_view=notebook"), "{saved}");
    }

    #[test]
    fn a_notebooks_first_load_selects_the_restored_active_note_and_expands_its_folders() {
        // Break caught: a restored session whose active note sits in a collapsed folder, with
        // nothing selected, until the user switches tabs.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("view-restore-reveal");
        std::fs::create_dir_all(scratch.folder().join(r"sub\deep")).unwrap();
        let b = scratch.note(r"sub\deep\b.md", "b");
        scratch.note("c.md", "c");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        ensure_sidebar(window.hwnd);
        app_mut(window.hwnd).library.data_dir = Some(scratch.data());
        super::open_path(window.hwnd, &b).unwrap();

        scratch.install(window.hwnd);

        assert_eq!(
            selected_kind(window.hwnd),
            Some(RowKind::Note(r"sub\deep\b.md".into()))
        );
        let expanded = crate::window::library_host::expanded(window.hwnd);
        assert!(expanded.contains(&std::path::PathBuf::from("sub")));
        assert!(expanded.contains(&std::path::PathBuf::from(r"sub\deep")));
    }

    fn write_notebooks(
        data: &std::path::Path,
        folders: Vec<std::path::PathBuf>,
        favorites: Vec<std::path::PathBuf>,
    ) {
        crate::library::local::write_folders(
            &crate::library::local::folders_file(data),
            &crate::library::local::RecentFolders {
                folders,
                favorites,
                closed: false,
            },
        )
        .unwrap();
    }

    #[test]
    fn moving_a_note_to_another_notebook_moves_the_file_drops_its_pin_and_its_tab_follows() {
        // Break caught: a move that copies without deleting, a pin record left pointing at a file
        // that left the notebook, or a tab still on the old path, where autosave would recreate it.
        let _scintilla = load_native_scintilla();
        let first = LibraryScratch::new("move-a");
        let second = LibraryScratch::new("move-b");
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        app_mut(window.hwnd).library.data_dir = Some(first.data());
        let a = open_note(&window, &first, "a.md", "a");
        crate::window::library_host::toggle_pin(window.hwnd, &a);
        write_notebooks(&first.data(), vec![first.folder(), second.folder()], vec![]);

        execute_command(window.hwnd, CommandId::NoteMoveToNotebook);
        crate::window::library_host::picked(
            window.hwnd,
            crate::window::command_palette::PickerKind::MoveToNotebook,
            crate::window::command_palette::PickerChoice::Item(0),
        );

        let moved = second.folder().join("a.md");
        assert!(!a.exists());
        assert_eq!(std::fs::read_to_string(&moved).unwrap(), "a");
        assert_eq!(
            app_mut(window.hwnd).tabs.active().unwrap().path.as_deref(),
            Some(moved.as_path())
        );
        crate::window::library_host::with_state(window.hwnd, |state| {
            assert!(!state.is_pinned(&a));
            assert!(state.record_for(&a).is_none());
            assert!(
                !state
                    .notes
                    .iter()
                    .any(|note| note.path == std::path::Path::new("a.md"))
            );
        });
        editor.set_text("b").unwrap();
        assert_eq!(
            crate::window::library_host::autosave_active(window.hwnd),
            crate::window::library_host::Autosave::NotEligible,
            "a plain file outside the notebook now"
        );
    }

    #[test]
    fn a_move_onto_an_existing_name_changes_nothing_and_says_why() {
        // Break caught: MoveFileExW's replace flag, or a fallback copy, overwriting the other
        // notebook's note of the same name.
        let _scintilla = load_native_scintilla();
        let first = LibraryScratch::new("move-clash-a");
        let second = LibraryScratch::new("move-clash-b");
        let theirs = second.note("a.md", "theirs");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        app_mut(window.hwnd).library.data_dir = Some(first.data());
        let a = open_note(&window, &first, "a.md", "mine");
        write_notebooks(&first.data(), vec![second.folder()], vec![]);

        crate::window::library_host::move_to_notebook(window.hwnd, &a);
        crate::window::library_host::picked(
            window.hwnd,
            crate::window::command_palette::PickerKind::MoveToNotebook,
            crate::window::command_palette::PickerChoice::Item(0),
        );

        assert_eq!(std::fs::read_to_string(&a).unwrap(), "mine");
        assert_eq!(std::fs::read_to_string(&theirs).unwrap(), "theirs");
        assert_eq!(
            app_mut(window.hwnd).tabs.active().unwrap().path.as_deref(),
            Some(a.as_path())
        );
        assert!(
            notices(window.hwnd)
                .iter()
                .any(|n| n.contains("already exists"))
        );
    }

    #[test]
    fn move_offers_favorites_by_name_then_recent_never_the_open_one_then_browse() {
        // Break caught: the open notebook offered as a destination, a notebook listed twice, or
        // Browse… not reachable.
        let _scintilla = load_native_scintilla();
        let first = LibraryScratch::new("move-list");
        let third = LibraryScratch::new("move-browse");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        app_mut(window.hwnd).library.data_dir = Some(first.data());
        let a = open_note(&window, &first, "a.md", "a");
        let (zeta, alpha, beta) = (
            first.root.join("Zeta"),
            first.root.join("alpha"),
            first.root.join("beta"),
        );
        write_notebooks(
            &first.data(),
            vec![first.folder(), beta.clone(), alpha.clone()],
            vec![zeta.clone(), alpha.clone(), first.folder()],
        );

        crate::window::library_host::move_to_notebook(window.hwnd, &a);
        let (note, destinations) = app_mut(window.hwnd).library.shown_move.clone().unwrap();
        assert_eq!(note, a);
        assert_eq!(destinations, vec![alpha, zeta, beta]);

        crate::window::answer_next_folder_dialog({
            let folder = third.folder();
            move |_| Some(folder)
        });
        crate::window::library_host::picked(
            window.hwnd,
            crate::window::command_palette::PickerKind::MoveToNotebook,
            crate::window::command_palette::PickerChoice::Item(3),
        );
        assert!(third.folder().join("a.md").exists());
    }

    #[test]
    fn a_new_note_saves_into_the_folder_selected_when_it_was_created_or_the_root_if_that_is_gone() {
        // Break caught: Ctrl+N with a subfolder selected saving into the notebook root anyway, or
        // a first save failing because the remembered folder was deleted meanwhile.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("new-note-folder");
        std::fs::create_dir_all(scratch.folder().join("sub")).unwrap();
        scratch.note(r"sub\b.md", "b");
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        ensure_sidebar(window.hwnd);
        app_mut(window.hwnd).library.data_dir = Some(scratch.data());
        scratch.install(window.hwnd);
        crate::window::library_host::set_expanded(window.hwnd, std::path::Path::new("sub"), true);
        crate::window::notebook_view::rebuild(window.hwnd);
        select_row(window.hwnd, &RowKind::Note(r"sub\b.md".into()));

        execute_command(window.hwnd, CommandId::New);
        assert_eq!(
            app_mut(window.hwnd).tabs.active().unwrap().save_folder,
            Some(scratch.folder().join("sub"))
        );
        editor.set_text("Idea").unwrap();
        execute_command(window.hwnd, CommandId::Save);
        crate::window::library_host::name_box_submit(window.hwnd);
        assert!(scratch.folder().join(r"sub\Idea.md").exists());

        let gone = scratch.folder().join("gone");
        crate::window::library_host::new_note_in(window.hwnd, Some(gone));
        editor.set_text("Other").unwrap();
        execute_command(window.hwnd, CommandId::Save);
        crate::window::library_host::name_box_submit(window.hwnd);
        assert!(scratch.folder().join("Other.md").exists());
    }

    #[test]
    fn the_context_menu_acts_on_its_row_not_the_active_tab() {
        // Break caught: Pin from a row's menu pinning the active tab's note instead, "New note
        // here" ignoring the folder, or Close tab on an unsaved row closing another tab.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("context-menu");
        std::fs::create_dir_all(scratch.folder().join("sub")).unwrap();
        scratch.note("b.md", "b");
        scratch.note(r"sub\c.md", "c");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        ensure_sidebar(window.hwnd);
        app_mut(window.hwnd).library.data_dir = Some(scratch.data());
        let a = open_note(&window, &scratch, "a.md", "a");
        let menu = |kind: &RowKind, answer: CommandId| {
            crate::window::menus::answer_next_popup_menu(move |_| Some(answer));
            let index = row_of(window.hwnd, kind);
            crate::window::notebook_view::open_context_menu(window.hwnd, index, None);
        };

        menu(&RowKind::Note("b.md".into()), CommandId::NoteTogglePin);
        crate::window::library_host::with_state(window.hwnd, |state| {
            assert!(state.is_pinned(&scratch.folder().join("b.md")));
            assert!(!state.is_pinned(&a));
        });

        menu(&RowKind::Folder("sub".into()), CommandId::New);
        let untitled = app_mut(window.hwnd).tabs.active().unwrap();
        assert_eq!(untitled.save_folder, Some(scratch.folder().join("sub")));
        let untitled = untitled.id;

        let before = super::tab_count(window.hwnd);
        menu(&RowKind::Unsaved(untitled.0), CommandId::CloseTab);
        assert_eq!(super::tab_count(window.hwnd), before - 1);
        assert!(app_mut(window.hwnd).tabs.document(untitled).is_none());

        menu(
            &RowKind::Folder("sub".into()),
            CommandId::NoteRevealInExplorer,
        );
        execute_command(window.hwnd, CommandId::NoteRevealInExplorer);
        assert_eq!(
            crate::platform::shell::take_revealed(),
            vec![scratch.folder().join("sub"), a.clone()]
        );
    }

    #[test]
    fn f2_on_a_note_row_that_is_not_open_opens_it_and_the_rename_box() {
        // Break caught: F2 in the tree renaming the active tab's note instead of the selected one,
        // or doing nothing because the name box needs a tab.
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_F2;
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("tree-f2");
        let a = scratch.note("a.md", "a");
        let b = scratch.note("b.md", "b");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        ensure_sidebar(window.hwnd);
        scratch.install(window.hwnd);
        super::open_path(window.hwnd, &a).unwrap();
        select_row(window.hwnd, &RowKind::Note("b.md".into()));

        assert!(crate::window::notebook_view::key_down(window.hwnd, VK_F2));

        let active = app_mut(window.hwnd).tabs.active().unwrap();
        assert_eq!(active.path.as_deref(), Some(b.as_path()));
        assert!(!active.preview);
        let id = active.id;
        let name_box = app_mut(window.hwnd).name_box.as_ref().unwrap();
        assert!(name_box.is_visible());
        assert_eq!(
            name_box.purpose(),
            Some(&crate::window::name_box::NamePurpose::RenameNote(id))
        );
    }

    #[test]
    fn palette_commands_act_on_the_row_focused_when_the_palette_opened() {
        // Break caught: opening the palette moves focus to its query field, so by the time the
        // chosen command runs, a live focus check sees nothing on the panel and falls back to
        // the active tab instead of the row the user actually picked -- wrong for spec §6.3, and
        // dangerous for Delete.
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetFocus, SetFocus, VK_RETURN};
        use windows_sys::Win32::UI::WindowsAndMessaging::{SetWindowTextW, WM_KEYDOWN};
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("palette-note-target");
        let active_path = scratch.note("active.md", "active");
        scratch.note("row.md", "row");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        ensure_sidebar(window.hwnd);
        scratch.install(window.hwnd);
        super::open_path(window.hwnd, &active_path).unwrap();
        crate::window::notebook_view::rebuild(window.hwnd);
        select_row(window.hwnd, &RowKind::Note("row.md".into()));
        let (_, panel) = sidebar_windows(window.hwnd);
        unsafe { SetFocus(panel) };
        assert_eq!(
            unsafe { GetFocus() },
            panel,
            "the panel must hold focus to record the row"
        );

        execute_command(window.hwnd, CommandId::CommandPalette);
        // Opening the palette moved focus to its own query field.
        assert_ne!(unsafe { GetFocus() }, panel);
        let query = app_mut(window.hwnd)
            .command_palette
            .as_ref()
            .unwrap()
            .query_hwnd();
        let typed = crate::platform::wide_null("Toggle pin");
        unsafe { SetWindowTextW(query, typed.as_ptr()) };
        unsafe { SendMessageW(query, WM_KEYDOWN, VK_RETURN as usize, 0) };

        crate::window::library_host::with_state(window.hwnd, |state| {
            assert!(state.is_pinned(&scratch.folder().join("row.md")));
            assert!(!state.is_pinned(&active_path));
        });
        // The panel had focus when the palette opened, so it gets it back.
        assert_eq!(unsafe { GetFocus() }, panel);
    }

    #[test]
    fn moving_a_note_onto_a_path_already_open_in_another_tab_is_refused() {
        // Break caught: the target file having been deleted on disk lets the clash check through,
        // then MoveFileExW succeeds and rebind_open_tab silently fails because another tab already
        // has that path, leaving that tab pointing at a file that no longer exists anywhere.
        let _scintilla = load_native_scintilla();
        let first = LibraryScratch::new("move-target-open-a");
        let second = LibraryScratch::new("move-target-open-b");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        app_mut(window.hwnd).library.data_dir = Some(first.data());
        let a = open_note(&window, &first, "a.md", "a");
        let target = second.note("a.md", "theirs");
        super::open_path(window.hwnd, &target).unwrap();
        std::fs::remove_file(&target).unwrap();
        write_notebooks(&first.data(), vec![second.folder()], vec![]);

        crate::window::library_host::move_to_notebook(window.hwnd, &a);
        crate::window::library_host::picked(
            window.hwnd,
            crate::window::command_palette::PickerKind::MoveToNotebook,
            crate::window::command_palette::PickerChoice::Item(0),
        );

        assert!(a.exists());
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "a");
        assert!(
            !target.exists(),
            "nothing was moved, so the deleted file stays deleted"
        );
        assert!(
            notices(window.hwnd)
                .iter()
                .any(|n| n.contains("already has"))
        );
    }

    #[test]
    fn note_commands_reach_a_focused_tree_row_even_with_no_tab_open() {
        // Break caught: the needs_document gate returning early for Ctrl+Shift+M and Reveal
        // whenever no tab happens to be open, even though the tree still has a focused row.
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus;
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("gate-no-tabs");
        let path = scratch.note("a.md", "a");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        ensure_sidebar(window.hwnd);
        scratch.install(window.hwnd);
        crate::window::notebook_view::rebuild(window.hwnd);
        select_row(window.hwnd, &RowKind::Note("a.md".into()));
        let (_, panel) = sidebar_windows(window.hwnd);
        unsafe { SetFocus(panel) };
        while super::tab_count(window.hwnd) > 0 {
            super::close_active_document(window.hwnd);
        }
        assert_eq!(super::tab_count(window.hwnd), 0);

        execute_command(window.hwnd, CommandId::NoteRevealInExplorer);

        assert_eq!(crate::platform::shell::take_revealed(), vec![path]);
    }

    #[test]
    fn moving_a_note_with_unsaved_edits_keeps_them_and_writes_only_at_the_new_path() {
        // Break caught: a move losing the tab's unsaved edits, or an autosave after the move
        // recreating the file at the old location instead of writing it to the new one.
        let _scintilla = load_native_scintilla();
        let first = LibraryScratch::new("move-dirty-a");
        let second = LibraryScratch::new("move-dirty-b");
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        app_mut(window.hwnd).library.data_dir = Some(first.data());
        let a = open_note(&window, &first, "a.md", "a");
        editor.set_text("unsaved edit").unwrap();
        write_notebooks(&first.data(), vec![first.folder(), second.folder()], vec![]);

        execute_command(window.hwnd, CommandId::NoteMoveToNotebook);
        crate::window::library_host::picked(
            window.hwnd,
            crate::window::command_palette::PickerKind::MoveToNotebook,
            crate::window::command_palette::PickerChoice::Item(0),
        );

        let moved = second.folder().join("a.md");
        assert!(!a.exists());
        assert_eq!(
            std::fs::read_to_string(&moved).unwrap(),
            "a",
            "the move only relocates the file on disk; the unsaved edit stays in the buffer"
        );
        let active = app_mut(window.hwnd).tabs.active().unwrap();
        assert_eq!(active.path.as_deref(), Some(moved.as_path()));
        assert!(active.dirty, "the unsaved edit followed the tab");
        assert_eq!(editor.text().unwrap(), "unsaved edit");

        // The move took the note out of the open notebook, so autosave no longer applies to it;
        // an explicit save must still land only at the new path, never re-create the old one.
        crate::window::library_host::save_command(window.hwnd);
        assert!(
            !a.exists(),
            "a save must not recreate the file at the old location"
        );
        assert_eq!(std::fs::read_to_string(&moved).unwrap(), "unsaved edit");
    }

    fn sidebar_panel(hwnd: HWND) -> HWND {
        crate::window::side_panel::windows(hwnd).unwrap().1
    }

    fn type_into_search(hwnd: HWND, text: &str) {
        let edit = crate::window::search_view::edit_hwnd(hwnd).unwrap();
        let wide = crate::platform::wide_null(text);
        // The Edit sends EN_CHANGE to the panel, which re-runs the search synchronously.
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::SetWindowTextW(edit, wide.as_ptr());
        }
    }

    #[test]
    fn the_search_view_matches_names_shows_folders_and_opens_the_preview_tab() {
        // Break caught: search over full paths instead of names, results without their folder,
        // or Enter opening a normal tab instead of the preview tab.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("search-view");
        scratch.note("Alpha.md", "a");
        scratch.note("beta.md", "b");
        std::fs::create_dir_all(scratch.folder().join("sub")).unwrap();
        scratch.note(r"sub\alphabet.md", "c");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        scratch.install(window.hwnd);
        crate::window::side_panel::show_view(window.hwnd, crate::config::SidebarView::Search, true);
        assert_eq!(crate::window::search_view::status(window.hwnd), None);

        type_into_search(window.hwnd, "alp");
        assert_eq!(
            crate::window::search_view::shown_results(window.hwnd),
            vec![
                ("Alpha".to_owned(), String::new()),
                ("alphabet".to_owned(), "sub".to_owned())
            ]
        );
        crate::window::search_view::open_selected(window.hwnd, super::OpenMode::Preview, false);
        let active = app_mut(window.hwnd).tabs.active().unwrap();
        assert_eq!(
            active.path.as_deref(),
            Some(scratch.folder().join("Alpha.md").as_path())
        );
        let active_id = active.id;
        assert_eq!(app_mut(window.hwnd).tabs.preview_id(), Some(active_id));

        type_into_search(window.hwnd, "zzz");
        assert_eq!(
            crate::window::search_view::status(window.hwnd),
            Some(crate::window::search_view::NO_MATCH)
        );
    }

    #[test]
    fn the_search_query_survives_a_view_switch_but_not_a_notebook_switch() {
        // Break caught: the query lost whenever another view is shown, or kept (with results
        // from the old notebook) after a different notebook opens.
        let _scintilla = load_native_scintilla();
        let first = LibraryScratch::new("search-keep-a");
        first.note("plan.md", "p");
        let second = LibraryScratch::new("search-keep-b");
        second.note("other.md", "o");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        first.install(window.hwnd);
        use crate::config::SidebarView;
        crate::window::side_panel::show_view(window.hwnd, SidebarView::Search, true);
        type_into_search(window.hwnd, "pl");
        crate::window::side_panel::show_view(window.hwnd, SidebarView::Notebook, false);
        crate::window::side_panel::show_view(window.hwnd, SidebarView::Search, false);
        assert_eq!(
            crate::window::search_view::shown_results(window.hwnd).len(),
            1
        );

        second.install(window.hwnd);
        assert!(crate::window::search_view::shown_results(window.hwnd).is_empty());
        let edit = crate::window::search_view::edit_hwnd(window.hwnd).unwrap();
        assert_eq!(
            unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetWindowTextLengthW(edit) },
            0
        );
    }

    #[test]
    fn a_hidden_search_view_searches_again_only_once_it_shows() {
        // Break caught: every library refresh re-running a query nobody sees, or the results
        // staying stale when the Search view comes back.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("search-stale");
        scratch.note("plan.md", "p");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        scratch.install(window.hwnd);
        use crate::config::SidebarView;
        // The box is made when the Search view first shows, not with the sidebar.
        assert!(crate::window::search_view::edit_hwnd(window.hwnd).is_none());
        crate::window::side_panel::show_view(window.hwnd, SidebarView::Search, true);
        assert!(crate::window::search_view::edit_hwnd(window.hwnd).is_some());
        type_into_search(window.hwnd, "pl");
        assert_eq!(
            crate::window::search_view::shown_results(window.hwnd).len(),
            1
        );

        crate::window::side_panel::show_view(window.hwnd, SidebarView::Notebook, false);
        let added = scratch.note("planning.md", "q");
        crate::window::library_host::with_state(window.hwnd, |state| state.add_note(&added));
        crate::window::side_panel::refresh(window.hwnd);
        assert_eq!(
            crate::window::search_view::shown_results(window.hwnd).len(),
            1,
            "a hidden Search view is not searched again"
        );
        crate::window::side_panel::show_view(window.hwnd, SidebarView::Search, false);
        assert_eq!(
            crate::window::search_view::shown_results(window.hwnd),
            vec![
                ("plan".to_owned(), String::new()),
                ("planning".to_owned(), String::new())
            ]
        );
    }

    fn search_selected(hwnd: HWND) -> Option<usize> {
        app_mut(hwnd).sidebar.as_ref().unwrap().search.list.selected
    }

    #[test]
    fn saving_a_listed_note_keeps_the_search_selection_and_does_not_rebuild_the_tree() {
        // Break caught: every save (autosave included) rebuilding the sidebar and snapping the
        // Search selection back to the first result while the user browses the list.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("search-save-keeps");
        scratch.note("plan.md", "p");
        let planning = scratch.note("planning.md", "q");
        scratch.note("plans.md", "r");
        let window = ProductionWindow::new(make_app());
        let editor = install_test_editor(&window);
        scratch.install(window.hwnd);
        use crate::config::SidebarView;
        crate::window::side_panel::show_view(window.hwnd, SidebarView::Search, true);
        type_into_search(window.hwnd, "plan");
        let results = crate::window::search_view::shown_results(window.hwnd);
        assert_eq!(results.len(), 3);
        let index = results
            .iter()
            .position(|(name, _)| name == "planning")
            .unwrap();
        assert_ne!(index, 0);
        app_mut(window.hwnd)
            .sidebar
            .as_mut()
            .unwrap()
            .search
            .list
            .selected = Some(index);
        super::open_path(window.hwnd, &planning).unwrap();
        pump_posted_messages(window.hwnd);
        let rebuilds = notebook_view(window.hwnd).rebuilds;

        editor.set_text("edited").unwrap();
        assert!(super::save_active_document(window.hwnd));
        assert_eq!(std::fs::read_to_string(&planning).unwrap(), "edited");
        assert_eq!(
            search_selected(window.hwnd),
            Some(index),
            "kept by the save"
        );
        assert_eq!(
            notebook_view(window.hwnd).rebuilds,
            rebuilds,
            "a save of a listed note changes no row"
        );

        // A library refresh runs the same query again, and the selection stays by path.
        crate::window::side_panel::refresh(window.hwnd);
        assert_eq!(
            search_selected(window.hwnd),
            Some(index),
            "kept by a re-run"
        );
        // A new query starts at the top.
        type_into_search(window.hwnd, "pla");
        assert_eq!(search_selected(window.hwnd), Some(0));
    }

    #[test]
    fn the_search_field_is_client_area_and_a_press_on_its_padding_focuses_the_box() {
        // Break caught: a press on the field's border or padding starting a window drag (the
        // whole Search header answered HTTRANSPARENT), so the box could only be focused by
        // hitting its text line exactly.
        use windows_sys::Win32::Foundation::{LPARAM, POINT};
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetFocus, SetFocus};
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            HTTRANSPARENT, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_NCHITTEST,
        };
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("search-field-client");
        scratch.note("plan.md", "p");
        let window = shown_window();
        let _editor = install_test_editor(&window);
        scratch.install(window.hwnd);
        crate::window::side_panel::show_view(
            window.hwnd,
            crate::config::SidebarView::Search,
            false,
        );
        let panel = sidebar_panel(window.hwnd);
        let mut client = RECT::default();
        unsafe { GetClientRect(panel, &mut client) };
        let dpi = unsafe { GetDpiForWindow(panel) }.max(96);
        let field = crate::window::search_view::SearchView::field_rect(client, dpi);
        let hit_test = |x: i32, y: i32| {
            let mut point = POINT { x, y };
            unsafe { windows_sys::Win32::Graphics::Gdi::ClientToScreen(panel, &mut point) };
            let lparam = ((point.y as u16 as u32) << 16 | point.x as u16 as u32) as LPARAM;
            unsafe { SendMessageW(panel, WM_NCHITTEST, 0, lparam) }
        };
        // The field's top-left padding, outside the Edit, and the header left of the field.
        assert_ne!(
            hit_test(field.left + 1, field.top + 1),
            HTTRANSPARENT as LRESULT,
            "the field is client area"
        );
        assert_eq!(
            hit_test(field.left - 2, field.top + 1),
            HTTRANSPARENT as LRESULT,
            "the header around the field still drags the window"
        );

        let edit = crate::window::search_view::edit_hwnd(window.hwnd).unwrap();
        unsafe { SetFocus(window.hwnd) };
        let lparam = (((field.top + 1) as u32) << 16 | (field.left + 1) as u32) as LPARAM;
        unsafe {
            SendMessageW(panel, WM_LBUTTONDOWN, 0, lparam);
            SendMessageW(panel, WM_LBUTTONUP, 0, lparam);
        }
        assert_eq!(
            unsafe { GetFocus() },
            edit,
            "the press put the caret in the box"
        );
    }

    #[test]
    fn with_no_notebook_the_search_view_says_to_open_one() {
        // Break caught: an empty Search view with a live box that searches nothing.
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        crate::window::side_panel::show_view(
            window.hwnd,
            crate::config::SidebarView::Search,
            false,
        );
        assert_eq!(
            crate::window::search_view::status(window.hwnd),
            Some(crate::window::search_view::NO_NOTEBOOK)
        );
    }

    #[test]
    fn a_favorite_opens_from_the_favorites_view_and_its_menu_removes_it() {
        // Break caught: a click on a favorite not switching the notebook or leaving the Favorites
        // view up, or "Remove from favorites" in the row menu doing nothing.
        let _scintilla = load_native_scintilla();
        let first = LibraryScratch::new("favorites-a");
        let second = LibraryScratch::new("favorites-b");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        app_mut(window.hwnd).library.data_dir = Some(first.data());
        second.install(window.hwnd);
        crate::window::library_host::toggle_notebook_favorite(window.hwnd);
        first.install(window.hwnd);
        use crate::config::SidebarView;
        crate::window::side_panel::show_view(window.hwnd, SidebarView::Favorites, true);
        let rows = crate::window::favorites_view::shown_rows(window.hwnd);
        assert_eq!(rows.len(), 1);
        assert!(!rows[0].open);

        crate::window::favorites_view::run(
            window.hwnd,
            crate::window::favorites_view::FavoriteAction::Open(second.folder()),
            true,
        );
        // The folder is checked on a worker; the view switches once the notebook is open.
        pump_until(window.hwnd, || {
            crate::window::side_panel::current_view(window.hwnd) == SidebarView::Notebook
        });
        assert!(crate::library::model::same_path(
            &crate::window::library_host::folder(window.hwnd).unwrap(),
            &second.folder()
        ));

        crate::window::side_panel::show_view(window.hwnd, SidebarView::Favorites, true);
        let panel = sidebar_panel(window.hwnd);
        unsafe {
            SendMessageW(
                panel,
                windows_sys::Win32::UI::WindowsAndMessaging::WM_KEYDOWN,
                0x24,
                0,
            );
        }
        crate::window::menus::answer_next_popup_menu(|_| Some(CommandId::ToggleNotebookFavorite));
        // Shift+F10 arrives as WM_CONTEXTMENU with (-1, -1).
        unsafe {
            SendMessageW(
                panel,
                windows_sys::Win32::UI::WindowsAndMessaging::WM_CONTEXTMENU,
                panel as usize,
                0xffff_ffff,
            );
        }
        assert!(crate::window::favorites_view::shown_rows(window.hwnd).is_empty());
    }

    #[test]
    fn the_settings_button_lists_only_settings_and_the_next_palette_lists_everything() {
        // Break caught: Settings showing the full command list, or its filter sticking to the
        // next Ctrl+Shift+P.
        let window = ProductionWindow::new(make_app());
        super::open_settings_palette(window.hwnd);
        let shown = with_command_palette(window.hwnd, |palette| {
            palette
                .shown()
                .iter()
                .map(|entry| entry.command)
                .collect::<Vec<_>>()
        })
        .unwrap();
        assert!(shown.contains(&CommandId::ThemeDark));
        assert!(
            shown
                .iter()
                .all(|command| crate::window::command_palette::SETTINGS_COMMANDS.contains(command))
        );
        super::close_command_palette(window.hwnd, false);

        execute_command(window.hwnd, CommandId::CommandPalette);
        let shown = with_command_palette(window.hwnd, |palette| palette.shown().len()).unwrap();
        assert!(shown > crate::window::command_palette::SETTINGS_COMMANDS.len());
    }

    #[test]
    fn f6_order_skips_a_closed_panel_and_a_missing_sidebar() {
        // Break caught: F6 landing in a hidden panel, or getting stuck when notes mode is off.
        use super::{FocusPart, next_focus_part};
        assert_eq!(
            next_focus_part(FocusPart::Editor, false, true, true),
            FocusPart::ActivityBar
        );
        assert_eq!(
            next_focus_part(FocusPart::ActivityBar, false, true, true),
            FocusPart::Panel
        );
        assert_eq!(
            next_focus_part(FocusPart::Panel, false, true, true),
            FocusPart::Editor
        );
        assert_eq!(
            next_focus_part(FocusPart::ActivityBar, true, true, true),
            FocusPart::Editor
        );
        assert_eq!(
            next_focus_part(FocusPart::ActivityBar, false, true, false),
            FocusPart::Editor
        );
        assert_eq!(
            next_focus_part(FocusPart::Editor, false, false, false),
            FocusPart::Editor
        );
    }

    fn focused() -> HWND {
        unsafe { windows_sys::Win32::UI::Input::KeyboardAndMouse::GetFocus() }
    }

    fn shown_window() -> ProductionWindow {
        let window = ProductionWindow::new(make_app());
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::ShowWindow(
                window.hwnd,
                windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOW,
            );
        }
        window
    }

    #[test]
    fn f6_cycles_activity_bar_panel_and_editor_and_shift_f6_goes_back() {
        // Break caught: F6 doing nothing, skipping the panel, or leaving the focus in a closed
        // panel.
        let _scintilla = load_native_scintilla();
        let window = shown_window();
        let _editor = install_test_editor(&window);
        let (bar, panel) = crate::window::side_panel::windows(window.hwnd).unwrap();
        use crate::config::SidebarView;
        crate::window::side_panel::show_view(window.hwnd, SidebarView::Notebook, false);
        super::return_focus_to_editor(window.hwnd);
        let editor = focused();

        execute_command(window.hwnd, CommandId::FocusNextPane);
        assert_eq!(focused(), bar);
        execute_command(window.hwnd, CommandId::FocusNextPane);
        assert_eq!(focused(), panel);
        execute_command(window.hwnd, CommandId::FocusNextPane);
        assert_eq!(focused(), editor);
        execute_command(window.hwnd, CommandId::FocusPreviousPane);
        assert_eq!(focused(), panel);

        crate::window::side_panel::toggle(window.hwnd);
        assert_eq!(
            crate::window::side_panel::current_view(window.hwnd),
            SidebarView::Hidden
        );
        super::return_focus_to_editor(window.hwnd);
        execute_command(window.hwnd, CommandId::FocusNextPane);
        assert_eq!(focused(), bar);
        execute_command(window.hwnd, CommandId::FocusNextPane);
        assert_eq!(focused(), editor, "a closed panel is skipped");
    }

    #[test]
    fn escape_in_the_panel_returns_the_focus_to_the_editor() {
        // Break caught: Esc in the tree leaving the keyboard stuck in the sidebar.
        let _scintilla = load_native_scintilla();
        let window = shown_window();
        let _editor = install_test_editor(&window);
        super::return_focus_to_editor(window.hwnd);
        let editor = focused();
        crate::window::side_panel::show_view(
            window.hwnd,
            crate::config::SidebarView::Notebook,
            true,
        );
        let panel = crate::window::side_panel::windows(window.hwnd).unwrap().1;
        assert_eq!(focused(), panel);
        unsafe {
            SendMessageW(
                panel,
                windows_sys::Win32::UI::WindowsAndMessaging::WM_KEYDOWN,
                windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE as usize,
                0,
            );
        }
        assert_eq!(focused(), editor);
    }

    #[test]
    fn the_activity_bar_moves_with_arrows_and_presses_with_enter() {
        // Break caught: activity-bar buttons reachable only with the mouse, or their pressed
        // state not following the shown view.
        let _scintilla = load_native_scintilla();
        let window = shown_window();
        let _editor = install_test_editor(&window);
        use crate::config::SidebarView;
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{SetFocus, VK_DOWN, VK_RETURN};
        use windows_sys::Win32::UI::WindowsAndMessaging::WM_KEYDOWN;
        crate::window::side_panel::show_view(window.hwnd, SidebarView::Notebook, false);
        let bar = crate::window::side_panel::windows(window.hwnd).unwrap().0;
        unsafe {
            SetFocus(bar);
        }
        assert_eq!(crate::window::side_panel::bar_focus(window.hwnd), 0);
        let items = crate::window::activity_bar::accessible_items(bar);
        assert_eq!(items.len(), 4);
        assert_ne!(
            items[0].state & crate::window::sidebar_accessibility::STATE_PRESSED,
            0
        );
        assert_ne!(
            items[0].state & crate::window::sidebar_accessibility::STATE_FOCUSED,
            0
        );
        assert_eq!(items[3].name, "Settings");

        unsafe {
            SendMessageW(bar, WM_KEYDOWN, VK_DOWN as usize, 0);
        }
        assert_eq!(crate::window::side_panel::bar_focus(window.hwnd), 1);
        unsafe {
            SendMessageW(bar, WM_KEYDOWN, VK_RETURN as usize, 0);
        }
        assert_eq!(
            crate::window::side_panel::current_view(window.hwnd),
            SidebarView::Search
        );
        let items = crate::window::activity_bar::accessible_items(bar);
        assert_ne!(
            items[1].state & crate::window::sidebar_accessibility::STATE_PRESSED,
            0
        );
        assert_eq!(
            items[0].state & crate::window::sidebar_accessibility::STATE_PRESSED,
            0
        );
    }

    #[test]
    fn the_panel_exposes_the_tree_as_an_outline_with_pinned_and_folder_states() {
        // Break caught: the tree invisible to screen readers, the child count not matching the
        // visible rows, or a pin and a collapsed folder not reported.
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("tree-msaa");
        let a = scratch.note("a.md", "a");
        std::fs::create_dir_all(scratch.folder().join("sub")).unwrap();
        scratch.note(r"sub\b.md", "b");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        scratch.install(window.hwnd);
        crate::window::library_host::toggle_pin(window.hwnd, &a);
        crate::window::side_panel::show_view(
            window.hwnd,
            crate::config::SidebarView::Notebook,
            false,
        );
        let panel = crate::window::side_panel::windows(window.hwnd).unwrap().1;
        let count = crate::window::side_panel::accessible_item_count(panel);
        let items = (0..count)
            .filter_map(|index| crate::window::side_panel::accessible_item(panel, index))
            .collect::<Vec<_>>();
        assert_eq!(items.len(), count);
        let rows = items
            .iter()
            .filter(|item| {
                item.role == windows_sys::Win32::UI::Accessibility::ROLE_SYSTEM_OUTLINEITEM
                    && !item.name.ends_with(", unsaved")
            })
            .collect::<Vec<_>>();
        // The window's untitled tab is an unsaved row, left out above. "sub" is collapsed, so b
        // is not a row: pinned a first, then the folder.
        assert_eq!(rows.len(), 2, "{items:?}");
        assert_eq!(rows[0].name, "a, pinned");
        assert_eq!(rows[1].name, "sub");
        assert_ne!(
            rows[1].state & crate::window::sidebar_accessibility::STATE_COLLAPSED,
            0
        );

        let provider = crate::window::sidebar_accessibility::create_for_test(
            panel,
            &crate::window::side_panel::PANEL_ACCESSIBLE,
        );
        let table = &crate::window::sidebar_accessibility::SIDEBAR_VTABLE;
        use crate::window::accessibility::{RawVariant, VariantValue};
        unsafe {
            let mut children = 0;
            (table.get_acc_child_count)(provider, &mut children);
            assert_eq!(children as usize, count);
            let mut role = RawVariant::empty();
            (table.get_acc_role)(provider, RawVariant::integer(0), &mut role);
            assert_eq!(
                role.child_id(),
                Some(windows_sys::Win32::UI::Accessibility::ROLE_SYSTEM_OUTLINE as i32)
            );
            (table.release)(provider);
        }
    }

    #[test]
    fn pinning_a_note_that_is_not_first_raises_reorder_and_state_change() {
        // Break caught: a pin re-sorting the selected row to the top at the same count, so screen
        // readers hear only a new selection: no state change and no reorder of its siblings.
        use crate::window::sidebar_accessibility::take_raised;
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            EVENT_OBJECT_REORDER, EVENT_OBJECT_SELECTION, EVENT_OBJECT_STATECHANGE,
        };
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("pin-msaa");
        scratch.note("a.md", "a");
        let b = scratch.note("b.md", "b");
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        scratch.install(window.hwnd);
        crate::window::side_panel::show_view(
            window.hwnd,
            crate::config::SidebarView::Notebook,
            false,
        );
        let panel = crate::window::side_panel::windows(window.hwnd).unwrap().1;
        let kind = RowKind::Note(crate::library::record_path(&scratch.folder(), &b));
        let before = row_of(window.hwnd, &kind);
        assert!(before > 0, "{:?}", notebook_view(window.hwnd).rows);
        notebook_view(window.hwnd).list.selected = Some(before);
        let source = &crate::window::side_panel::PANEL_ACCESSIBLE;
        let count = (source.count)(panel);
        take_raised();

        crate::window::library_host::toggle_pin(window.hwnd, &b);

        let after = row_of(window.hwnd, &kind);
        assert!(after < before, "the pinned note moves up");
        assert_eq!((source.count)(panel), count, "the same number of children");
        let id = (source.current)(panel).unwrap() as i32 + 1;
        let raised = take_raised()
            .into_iter()
            .filter(|&(hwnd, _, _)| hwnd == panel as usize)
            .map(|(_, event, child)| (event, child))
            .collect::<Vec<_>>();
        assert!(raised.contains(&(EVENT_OBJECT_REORDER, 0)), "{raised:?}");
        assert!(raised.contains(&(EVENT_OBJECT_SELECTION, id)), "{raised:?}");
        assert!(
            raised.contains(&(EVENT_OBJECT_STATECHANGE, id)),
            "{raised:?}"
        );
        assert!(
            (source.item)(panel, id as usize - 1)
                .unwrap()
                .name
                .ends_with(", pinned")
        );
    }

    #[test]
    fn arrowing_through_recent_notebooks_selects_and_announces_them() {
        // Break caught: the no-notebook state's RECENT list reporting no selection, so arrowing
        // through it raises no events and no item is ever STATE_SYSTEM_SELECTED.
        use crate::window::sidebar_accessibility::{STATE_FOCUSED, STATE_SELECTED, take_raised};
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_DOWN;
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            EVENT_OBJECT_FOCUS, EVENT_OBJECT_SELECTION, WM_KEYDOWN,
        };
        let _scintilla = load_native_scintilla();
        let scratch = LibraryScratch::new("recent-msaa-a");
        let other = LibraryScratch::new("recent-msaa-b");
        scratch.note("a.md", "a");
        let window = shown_window();
        let _editor = install_test_editor(&window);
        ensure_sidebar(window.hwnd);
        app_mut(window.hwnd).library.data_dir = Some(scratch.data());
        crate::library::local::write_folders(
            &crate::library::local::folders_file(&scratch.data()),
            &crate::library::local::RecentFolders {
                folders: vec![scratch.folder(), other.folder()],
                ..Default::default()
            },
        )
        .unwrap();
        scratch.install(window.hwnd);
        execute_command(window.hwnd, CommandId::CloseNotebook);
        assert_eq!(notebook_view(window.hwnd).mode, Mode::NoNotebook);
        let recent = notebook_view(window.hwnd).recent.clone();
        assert!(recent.len() >= 2, "{recent:?}");
        crate::window::side_panel::show_view(
            window.hwnd,
            crate::config::SidebarView::Notebook,
            true,
        );
        let panel = crate::window::side_panel::windows(window.hwnd).unwrap().1;
        assert_eq!(focused(), panel);
        let source = &crate::window::side_panel::PANEL_ACCESSIBLE;
        let buttons = (source.count)(panel) - recent.len();

        for _ in 0..2 {
            take_raised();
            unsafe {
                SendMessageW(panel, WM_KEYDOWN, VK_DOWN as usize, 0);
            }
            let selected = notebook_view(window.hwnd).list.selected.unwrap();
            let id = (buttons + selected) as i32 + 1;
            assert_eq!((source.current)(panel), Some(buttons + selected));
            let raised = take_raised();
            let panel_id = panel as usize;
            assert!(
                raised.contains(&(panel_id, EVENT_OBJECT_SELECTION, id)),
                "{raised:?}"
            );
            assert!(
                raised.contains(&(panel_id, EVENT_OBJECT_FOCUS, id)),
                "{raised:?}"
            );
            let item = (source.item)(panel, id as usize - 1).unwrap();
            assert_ne!(item.state & STATE_SELECTED, 0, "{item:?}");
            assert_ne!(item.state & STATE_FOCUSED, 0, "{item:?}");
            assert_eq!(
                item.name,
                notebook_view(window.hwnd).recent_name(selected),
                "indexed by list position"
            );
        }
        assert_eq!(notebook_view(window.hwnd).list.selected, Some(1));
    }

    #[test]
    fn a_query_from_another_thread_is_answered_on_the_window_thread() {
        // Break caught: an MSAA client's RPC thread reading App directly, or its marshalled query
        // rejected by the pointer check meant for foreign senders.
        let _scintilla = load_native_scintilla();
        let window = ProductionWindow::new(make_app());
        let _editor = install_test_editor(&window);
        crate::window::side_panel::show_view(
            window.hwnd,
            crate::config::SidebarView::Notebook,
            false,
        );
        let panel = crate::window::side_panel::windows(window.hwnd).unwrap().1;
        let expected = crate::window::side_panel::accessible_item_count(panel);
        assert!(expected > 0);
        let provider = crate::window::sidebar_accessibility::create_for_test(
            panel,
            &crate::window::side_panel::PANEL_ACCESSIBLE,
        ) as usize;
        let worker = std::thread::spawn(move || {
            let provider = provider as *mut std::ffi::c_void;
            let table = &crate::window::sidebar_accessibility::SIDEBAR_VTABLE;
            let mut count = 0;
            unsafe {
                assert_eq!(
                    (table.get_acc_child_count)(provider, &mut count),
                    windows_sys::Win32::Foundation::S_OK
                );
                (table.release)(provider);
            }
            count
        });
        // Messages sent from the worker are delivered while this thread peeks.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !worker.is_finished() {
            assert!(
                std::time::Instant::now() < deadline,
                "the query was never answered"
            );
            let mut message = windows_sys::Win32::UI::WindowsAndMessaging::MSG::default();
            unsafe {
                windows_sys::Win32::UI::WindowsAndMessaging::PeekMessageW(
                    &mut message,
                    std::ptr::null_mut(),
                    0,
                    0,
                    windows_sys::Win32::UI::WindowsAndMessaging::PM_NOREMOVE,
                );
            }
        }
        assert_eq!(worker.join().unwrap() as usize, expected);
    }
}
