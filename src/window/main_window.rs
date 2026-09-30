use crate::Result;
use crate::app::{App, WindowIdentity};
use crate::document::{CloseDecision, Document, DocumentId, RecoveryId};
use crate::editor::Editor;
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
use crate::window::settings_model::{MAX_FONT_SIZE, MIN_FONT_SIZE};
use crate::window::split_tree::GroupId;
use crate::window::tabs::CloseReviewKey;
use crate::window::titlebar::{
    HitTarget, LogoIcon, PointerState, TitleBarLayout, TitleFontHandles,
};
#[cfg(test)]
use std::cell::Cell;
use std::ffi::c_void;
use std::ptr::NonNull;
use windows_sys::Win32::Foundation::{HMODULE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{HDC, InvalidateRect};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::SystemInformation::GetTickCount;
use windows_sys::Win32::UI::Controls::{DRAWITEMSTRUCT, NMHDR, WM_MOUSELEAVE};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetFocus, GetKeyState, GetLastInputInfo, LASTINPUTINFO, ReleaseCapture, SetCapture, SetFocus,
    VK_CONTROL, VK_DOWN, VK_ESCAPE, VK_F10, VK_LEFT, VK_MENU, VK_RETURN, VK_RIGHT, VK_SHIFT, VK_UP,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    BN_CLICKED, CREATESTRUCTW, CreateWindowExW, DefWindowProcW, DestroyWindow, EN_CHANGE,
    GWL_STYLE, GWLP_USERDATA, GetClientRect, GetWindowLongPtrW, HICON, IMAGE_ICON, IsWindow,
    IsWindowVisible, IsZoomed, KillTimer, LR_DEFAULTCOLOR, LoadIconW, LoadImageW, MoveWindow,
    OBJID_CLIENT, PostMessageW, PostQuitMessage, QS_INPUT, RegisterClassW, SC_CLOSE, SC_KEYMENU,
    SC_MAXIMIZE, SC_MINIMIZE, SC_RESTORE, SW_HIDE, SW_SHOWNA, SWP_FRAMECHANGED, SWP_NOACTIVATE,
    SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SendMessageW, SetTimer, SetWindowLongPtrW, SetWindowPos,
    ShowWindow, UnregisterClassW, WHEEL_DELTA, WM_ACTIVATEAPP, WM_CAPTURECHANGED, WM_CLOSE,
    WM_COMMAND, WM_CTLCOLORBTN, WM_CTLCOLOREDIT, WM_CTLCOLORLISTBOX, WM_DESTROY, WM_DPICHANGED,
    WM_DRAWITEM, WM_DROPFILES, WM_DWMCOLORIZATIONCOLORCHANGED, WM_GETMINMAXINFO, WM_GETOBJECT,
    WM_KEYDOWN, WM_KILLFOCUS, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN,
    WM_MBUTTONUP, WM_MOUSEHWHEEL, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCCALCSIZE, WM_NCCREATE,
    WM_NCDESTROY, WM_NCHITTEST, WM_NCLBUTTONDBLCLK, WM_NCLBUTTONDOWN, WM_NCLBUTTONUP,
    WM_NCMOUSELEAVE, WM_NCMOUSEMOVE, WM_NOTIFY, WM_PAINT, WM_RBUTTONDOWN, WM_RBUTTONUP,
    WM_SETFOCUS, WM_SETTINGCHANGE, WM_SIZE, WM_SYSCOMMAND, WM_SYSKEYDOWN, WM_SYSKEYUP,
    WM_THEMECHANGED, WM_TIMER, WNDCLASSW, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
};
#[cfg(test)]
use windows_sys::Win32::UI::WindowsAndMessaging::{MSG, PM_NOREMOVE, PeekMessageW, WM_QUIT};

/// The input messages the startup chain yields to and the input drain takes: from the title
/// bar's and frame's mouse messages (WM_NCMOUSEMOVE, 0xA0) through the pointer messages. Every
/// kind `QS_INPUT` counts must be in range, or input the drain can't take keeps the chain
/// yielding forever. The drain's `PM_QS_INPUT` keeps posted non-input messages in range out.
pub(crate) const INPUT_MESSAGE_FIRST: u32 = WM_NCMOUSEMOVE;
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
        // Posted startup steps outrank WM_PAINT, so the tab strip paints now or only after them.
        if let Some(group) = group_hwnd(hwnd) {
            unsafe { windows_sys::Win32::Graphics::Gdi::UpdateWindow(group) };
        }
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
                unsafe { app_ptr(hwnd) }.and_then(|app| unsafe { app.as_ref() }.editor().cloned())
            {
                let _ = editor.set_text_padding(dpi);
            }
            // Replaces (and drops, which destroys) any icon loaded for the old DPI. The bar's own
            // resize below repaints it, so no separate invalidate is needed here.
            ensure_logo_icon(hwnd, dpi);
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

    // Every document comes from the host, so any editor can show any of them (split editors
    // spec §3.1).
    let host = Editor::create_document_host()?;
    // The editor, find bar, preview and image view live in the editor group (split editors spec
    // §4.2), which the main window lays out.
    let group = crate::window::editor_group::create(hwnd)?;
    let editor = create_editor(group)?.with_document_host(&host);
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
    let initial = editor.create_document()?;
    editor.use_document(&initial)?;
    let document = Document::untitled(DocumentId(1), recovery_id, initial);
    if let Some(mut app) = unsafe { app_ptr(hwnd) } {
        unsafe { app.as_mut() }.document_host = Some(host);
    }
    if !identity.is_live_for(hwnd) {
        return Err(crate::FastPadError::Invariant(
            "main window was destroyed while adopting the initial document",
        ));
    }

    unsafe {
        install_editor(hwnd, group, editor, document)?;
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
    unsafe { app.as_ref() }.editor().map(Editor::hwnd)
}

fn with_editor(hwnd: HWND, action: impl FnOnce(&Editor)) {
    let Some(app) = (unsafe { app_ptr(hwnd) }) else {
        return;
    };
    let Some(editor) = unsafe { app.as_ref() }.editor() else {
        return;
    };
    action(editor);
}

/// Lays out the sidebar, then the name box and the editor group right of it, below the title
/// strip. The sole layout choke point for all of them; the group lays out its own children
/// (`layout_group`).
pub(crate) fn layout_editor_and_find_bar(hwnd: HWND) {
    let mut rect = RECT::default();
    unsafe {
        GetClientRect(hwnd, &mut rect);
    }
    let dpi = unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForWindow(hwnd) }.max(96);
    crate::window::side_panel::layout(hwnd, rect, dpi);
    let left = crate::window::side_panel::left_edge(hwnd);
    layout_command_palette(hwnd);
    if group_hwnd(hwnd).is_none() {
        return;
    }
    let title_height = title_layout(hwnd).height + menu_band_height(hwnd);
    let width = (rect.right - rect.left - left).max(0);
    let font = title_chrome(hwnd).1.text();
    // The name box spans the editor area under the title row, over the top groups.
    if let Some(app) = unsafe { app_ptr(hwnd) }
        && let Some(name_box) = unsafe { app.as_ref() }.name_box.as_ref()
    {
        name_box.layout(left, width, title_height, dpi, font);
    }
    let (Some(area), Some(layout)) = (tree_area(hwnd), tree_layout(hwnd)) else {
        return;
    };
    for (id, rect) in &layout.groups {
        let Some(group) = with_group_id(hwnd, *id, |state| state.hwnd) else {
            continue;
        };
        // A top group reaches up into the title row and draws its strip there (spec §4.1); a
        // lower group draws its strip at its own top.
        let top = if rect.top == area.top { 0 } else { rect.top };
        unsafe {
            MoveWindow(
                group,
                rect.left,
                top,
                rect.right - rect.left,
                rect.bottom - top,
                1,
            );
        }
        // A move that keeps the group's size sends no WM_SIZE, but what is inside may have
        // changed.
        layout_group(hwnd, group);
    }
    unsafe { InvalidateRect(hwnd, std::ptr::null(), 0) };
}

/// The sash under the cursor sets the resize cursor; false elsewhere.
fn set_sash_cursor(hwnd: HWND) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetCursorPos, IDC_SIZENS, IDC_SIZEWE, LoadCursorW, SetCursor,
    };
    let mut point = windows_sys::Win32::Foundation::POINT { x: 0, y: 0 };
    unsafe {
        GetCursorPos(&mut point);
        windows_sys::Win32::Graphics::Gdi::ScreenToClient(hwnd, &mut point);
    }
    let Some(axis) =
        tree_layout(hwnd).and_then(|layout| layout.sash_at(point.x, point.y).map(|sash| sash.axis))
    else {
        return false;
    };
    let cursor = match axis {
        crate::window::split_tree::Axis::Row => IDC_SIZEWE,
        crate::window::split_tree::Axis::Column => IDC_SIZENS,
    };
    unsafe { SetCursor(LoadCursorW(std::ptr::null_mut(), cursor)) };
    true
}

/// A press on a sash starts dragging it; a second press there within the double-click time
/// shares its branch out evenly instead (spec §4.3). The main window has no `CS_DBLCLKS`, so the
/// double-click is timed here, as the tab strip does. False when no sash is under the point.
fn press_sash(hwnd: HWND, x: i32, y: i32) -> bool {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime;
    use windows_sys::Win32::UI::WindowsAndMessaging::GetMessageTime;
    let Some(sash) = tree_layout(hwnd).and_then(|layout| layout.sash_at(x, y).cloned()) else {
        return false;
    };
    let now = unsafe { GetMessageTime() } as u32;
    let double_click_time = unsafe { GetDoubleClickTime() };
    let equalized = unsafe { app_ptr(hwnd) }.is_some_and(|mut app| {
        let app = unsafe { app.as_mut() };
        let double = app.last_sash_click.as_ref().is_some_and(|(id, time)| {
            *id == sash.id && now.wrapping_sub(*time) <= double_click_time
        });
        if double {
            app.last_sash_click = None;
            app.layout.equalize(&sash.id);
        } else {
            app.last_sash_click = Some((sash.id.clone(), now));
            app.sash_drag = Some(sash);
        }
        double
    });
    if equalized {
        layout_editor_and_find_bar(hwnd);
    } else {
        unsafe { SetCapture(hwnd) };
    }
    true
}

/// Moves the dragged sash to the pointer; false when no sash is being dragged.
fn drag_sash(hwnd: HWND, lparam: LPARAM) -> bool {
    let (x, y) = (
        (lparam as u32 & 0xffff) as u16 as i16 as i32,
        ((lparam as u32 >> 16) & 0xffff) as u16 as i16 as i32,
    );
    let dpi = unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForWindow(hwnd) }.max(96);
    let moved = unsafe { app_ptr(hwnd) }.and_then(|mut app| {
        let app = unsafe { app.as_mut() };
        let sash = app.sash_drag.clone()?;
        let at = match sash.axis {
            crate::window::split_tree::Axis::Row => x,
            crate::window::split_tree::Axis::Column => y,
        };
        Some(app.layout.set_sash(&sash, at, dpi))
    });
    match moved {
        Some(changed) => {
            if changed {
                layout_editor_and_find_bar(hwnd);
                unsafe { windows_sys::Win32::Graphics::Gdi::UpdateWindow(hwnd) };
            }
            true
        }
        None => false,
    }
}

/// Ends a sash drag; returns whether one was under way.
fn end_sash_drag(hwnd: HWND) -> bool {
    unsafe { app_ptr(hwnd) }
        .is_some_and(|mut app| unsafe { app.as_mut() }.sash_drag.take().is_some())
}

/// The editor area the groups share, in the main window's client coordinates: below the title
/// row, above the status bar, right of the sidebar.
pub(crate) fn tree_area(hwnd: HWND) -> Option<crate::window::titlebar::Rect> {
    let mut client = RECT::default();
    unsafe {
        GetClientRect(hwnd, &mut client);
    }
    let left = crate::window::side_panel::left_edge(hwnd);
    let top = title_layout(hwnd).height;
    let bottom = (client.bottom - status_bar_height(hwnd)).max(top + 1);
    (client.right > left)
        .then(|| crate::window::titlebar::Rect::new(left, top, client.right, bottom))
}

/// Where each group and sash goes now.
pub(crate) fn tree_layout(hwnd: HWND) -> Option<crate::window::split_tree::TreeLayout> {
    let area = tree_area(hwnd)?;
    let dpi = unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForWindow(hwnd) }.max(96);
    let app = unsafe { app_ptr(hwnd) }?;
    Some(unsafe { app.as_ref() }.layout.layout(area, dpi))
}

/// The groups in layout order: left to right, top to bottom (spec §4.3).
pub(crate) fn group_order(hwnd: HWND) -> Vec<GroupId> {
    unsafe { app_ptr(hwnd) }
        .map(|app| unsafe { app.as_ref() }.layout.leaves())
        .unwrap_or_default()
}

/// Whether `group`'s window starts at the top of the main window's client area, so its strip is
/// in the title row.
fn is_top_group(hwnd: HWND, group: HWND) -> bool {
    let mut origin = windows_sys::Win32::Foundation::POINT { x: 0, y: 0 };
    unsafe { windows_sys::Win32::Graphics::Gdi::MapWindowPoints(group, hwnd, &mut origin, 1) };
    origin.y == 0
}

/// The active editor group's window, once the editor exists.
pub(crate) fn group_hwnd(hwnd: HWND) -> Option<HWND> {
    let app = unsafe { app_ptr(hwnd) }?;
    unsafe { app.as_ref() }
        .active_group()
        .map(|group| group.hwnd)
}

/// The group whose window is `group`.
/// The group whose window contains screen point `point`, with that window.
pub(crate) fn group_at(
    hwnd: HWND,
    point: windows_sys::Win32::Foundation::POINT,
) -> Option<(GroupId, HWND)> {
    let windows = unsafe { app_ptr(hwnd) }.map(|app| {
        unsafe { app.as_ref() }
            .groups
            .iter()
            .map(|group| (group.id, group.hwnd))
            .collect::<Vec<_>>()
    })?;
    windows.into_iter().find(|(_, window)| {
        let mut rect = RECT::default();
        let shown = unsafe {
            // Its own style: the main window of a test is never shown.
            GetWindowLongPtrW(*window, GWL_STYLE) as u32 & WS_VISIBLE != 0
                && windows_sys::Win32::UI::WindowsAndMessaging::GetWindowRect(*window, &mut rect)
                    != 0
        };
        shown
            && point.x >= rect.left
            && point.x < rect.right
            && point.y >= rect.top
            && point.y < rect.bottom
    })
}

pub(crate) fn group_id_of(hwnd: HWND, group: HWND) -> Option<GroupId> {
    let app = unsafe { app_ptr(hwnd) }?;
    unsafe { app.as_ref() }
        .groups
        .iter()
        .find(|state| state.hwnd == group)
        .map(|state| state.id)
}

/// Runs `f` on group `id`'s window state.
pub(crate) fn with_group_id<R>(
    hwnd: HWND,
    id: GroupId,
    f: impl FnOnce(&mut crate::window::editor_group::GroupWindow) -> R,
) -> Option<R> {
    let mut app = unsafe { app_ptr(hwnd) }?;
    unsafe { app.as_mut() }.group_mut(id).map(f)
}

pub(crate) fn group_editor(hwnd: HWND, id: GroupId) -> Option<Editor> {
    let app = unsafe { app_ptr(hwnd) }?;
    unsafe { app.as_ref() }
        .group(id)
        .map(|group| group.editor.clone())
}

/// Creates an empty group window with its own editor, set up like the others. The caller puts it
/// in the layout (split editors spec §4.2).
pub(crate) fn create_group(hwnd: HWND) -> Result<GroupId> {
    #[cfg(test)]
    if FAIL_GROUP_AFTER.with(|after| match after.get() {
        Some(0) => {
            after.set(None);
            true
        }
        Some(count) => {
            after.set(Some(count - 1));
            false
        }
        None => false,
    }) {
        return Err(crate::FastPadError::Invariant(
            "test: group creation failed",
        ));
    }
    let host = unsafe { app_ptr(hwnd) }
        .and_then(|app| unsafe { app.as_ref() }.document_host.clone())
        .ok_or(crate::FastPadError::Invariant(
            "main window app state was not available",
        ))?;
    let window = crate::window::editor_group::create(hwnd)?;
    let editor = match Editor::create_with_host(window, &host) {
        Ok(editor) => editor,
        Err(error) => {
            unsafe { DestroyWindow(window) };
            return Err(error);
        }
    };
    configure_editor(hwnd, &editor);
    // A new group shows nothing until a view is added.
    unsafe { ShowWindow(editor.hwnd(), SW_HIDE) };
    let Some(mut app) = (unsafe { app_ptr(hwnd) }) else {
        unsafe { DestroyWindow(window) };
        return Err(crate::FastPadError::Invariant(
            "main window app state was not available",
        ));
    };
    let editor_hwnd = editor.hwnd();
    let app = unsafe { app.as_mut() };
    let id = app.tabs.add_group();
    app.groups
        .push(crate::window::editor_group::GroupWindow::new(
            id, window, editor,
        ));
    // A group made after `BUILD_CHROME` takes Explorer drops too (split editors spec §6.2).
    let wrap = app.file_drops_accepted;
    if wrap {
        crate::window::library_host::wrap_group_drop_target(hwnd, window, editor_hwnd);
    }
    Ok(id)
}

#[cfg(test)]
thread_local! {
    /// How many more group windows are made before one fails, for the failure tests.
    static FAIL_GROUP_AFTER: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
pub(crate) fn fail_next_group_creation() {
    fail_group_creation_after(0);
}

/// The group window made after `count` more succeed fails.
#[cfg(test)]
pub(crate) fn fail_group_creation_after(count: usize) {
    FAIL_GROUP_AFTER.with(|after| after.set(Some(count)));
}

/// Destroys group `id`'s window, its editor and what it showed. The caller has already emptied it
/// and taken it out of the layout.
pub(crate) fn destroy_group(hwnd: HWND, id: GroupId) {
    if unsafe { app_ptr(hwnd) }.is_some_and(|app| {
        unsafe { app.as_ref() }
            .tab_drag
            .as_ref()
            .is_some_and(|drag| drag.source.group == id)
    }) {
        crate::window::tab_drag::cancel(hwnd);
    }
    let removed = unsafe { app_ptr(hwnd) }.and_then(|mut app| {
        let app = unsafe { app.as_mut() };
        let index = app.groups.iter().position(|group| group.id == id)?;
        app.tabs.remove_group(id).then(|| app.groups.remove(index))
    });
    if let Some(group) = removed {
        let window = group.hwnd;
        drop(group);
        unsafe { DestroyWindow(window) };
    }
}

/// Applies the editor settings, the theme's colours and the shared zoom to `editor`, as every
/// group's editor has them.
fn configure_editor(hwnd: HWND, editor: &Editor) {
    let Some((settings, palette, zoom)) = (unsafe { app_ptr(hwnd) }).map(|app| {
        let app = unsafe { app.as_ref() };
        let palette = Palette::for_cached_theme(app.theme, app.settings.theme);
        let zoom = app.editor().and_then(|editor| editor.zoom().ok());
        (app.settings.clone(), palette, zoom)
    }) else {
        return;
    };
    apply_settings_to(editor, &settings, palette);
    apply_colors_to(editor, palette, settings.highlight_current_line);
    if let Some(zoom) = zoom {
        let _ = editor.set_zoom(zoom);
    }
}

fn apply_settings_to(editor: &Editor, settings: &crate::config::Settings, palette: Palette) {
    let _ = editor.set_line_numbers(settings.line_numbers);
    let _ = editor.apply_view_settings(
        &settings.font_face,
        settings.font_size,
        settings.tab_width,
        settings.word_wrap,
    );
    let _ = editor.apply_whitespace_settings(settings.insert_spaces, settings.show_whitespace);
    let _ =
        editor.set_line_number_colors(palette.line_number_foreground, palette.editor_background);
    apply_chrome_colors_to(editor, palette, settings.highlight_current_line);
}

fn apply_colors_to(editor: &Editor, palette: Palette, highlight_current_line: bool) {
    let _ = editor.set_base_colors(palette.editor_foreground, palette.editor_background);
    let _ =
        editor.set_line_number_colors(palette.line_number_foreground, palette.editor_background);
    apply_chrome_colors_to(editor, palette, highlight_current_line);
    let _ = editor.set_selection_text_colors(palette.selection_foreground);
}

/// The selection backgrounds, and the caret line's when `highlight_current_line` is on.
fn apply_chrome_colors_to(editor: &Editor, palette: Palette, highlight_current_line: bool) {
    let _ = editor.set_chrome_colors(
        palette.selection_background,
        palette.inactive_selection_background,
        highlight_current_line.then_some(palette.caret_line_background),
    );
}

/// Every group's editor, the active group's first.
fn all_editors(hwnd: HWND) -> Vec<Editor> {
    unsafe { app_ptr(hwnd) }
        .map(|app| {
            let app = unsafe { app.as_ref() };
            let active = app.tabs.active_group();
            let mut editors = app
                .active_group()
                .map(|group| group.editor.clone())
                .into_iter()
                .collect::<Vec<_>>();
            editors.extend(
                app.groups
                    .iter()
                    .filter(|group| group.id != active)
                    .map(|group| group.editor.clone()),
            );
            editors
        })
        .unwrap_or_default()
}

/// The window the content area's children go into: the editor group, or the main window before
/// the group exists.
pub(crate) fn content_parent(hwnd: HWND) -> HWND {
    group_hwnd(hwnd).unwrap_or(hwnd)
}

/// Lays out the tab strip, the find bar, then the preview and the editor below them, in `group`'s
/// client area.
pub(crate) fn layout_group(hwnd: HWND, group: HWND) {
    let Some(id) = group_id_of(hwnd, group) else {
        return;
    };
    let Some(editor_hwnd) = group_editor(hwnd, id).map(|editor| editor.hwnd()) else {
        return;
    };
    let mut client = RECT::default();
    unsafe {
        GetClientRect(group, &mut client);
    }
    let dpi = unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForWindow(group) }.max(96);
    let font = title_chrome(hwnd).1.text();
    let width = client.right;
    let strip_height = crate::window::group_strip::strip_height(dpi);
    // The menu band and the name box sit under the title row, over the top groups' content.
    let band = if is_top_group(hwnd, group) {
        menu_band_height(hwnd) + name_box_band_height(hwnd, dpi)
    } else {
        0
    };
    let find_top = strip_height + band;
    let find_bar_height = unsafe { app_ptr(hwnd) }
        .and_then(|app| {
            let bar = unsafe { app.as_ref() }.group(id)?.find_bar.as_ref()?;
            bar.layout(0, width, find_top, dpi, font);
            bar.is_visible().then(|| find_bar::find_bar_height(dpi))
        })
        .unwrap_or(0);
    let top = find_top + find_bar_height;
    let area = RECT {
        left: 0,
        top,
        right: width,
        bottom: client.bottom.max(top),
    };
    with_group_id(hwnd, id, |state| {
        state.content =
            crate::window::titlebar::Rect::new(area.left, area.top, area.right, area.bottom)
    });
    set_group_region(hwnd, group, &client, strip_height, band);
    let rects = crate::window::preview_host::layout(hwnd, id, area, dpi);
    crate::window::image_host::layout(hwnd, id, area);
    crate::window::preview_buttons::layout(hwnd, group, area, dpi);
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
    unsafe {
        InvalidateRect(group, std::ptr::null(), 0);
    }
}

/// The visible name box's height, or 0.
fn name_box_band_height(hwnd: HWND, dpi: u32) -> i32 {
    unsafe { app_ptr(hwnd) }
        .filter(|app| {
            unsafe { app.as_ref() }
                .name_box
                .as_ref()
                .is_some_and(|name_box| name_box.is_visible())
        })
        .map_or(0, |_| crate::window::name_box::name_box_height(dpi))
}

/// Leaves out of the group window what the main window keeps in its area: the caption buttons at
/// the right end of the title row, and the band under it while the menu band or the name box
/// shows. The main window paints them and gets their input.
fn set_group_region(hwnd: HWND, group: HWND, client: &RECT, strip_height: i32, band: i32) {
    use windows_sys::Win32::Graphics::Gdi::{CombineRgn, CreateRectRgn, RGN_DIFF, SetWindowRgn};
    // Only the group under the caption buttons gives up the part of its strip they cover.
    let mut origin = windows_sys::Win32::Foundation::POINT { x: 0, y: 0 };
    unsafe { windows_sys::Win32::Graphics::Gdi::MapWindowPoints(group, hwnd, &mut origin, 1) };
    let caption_left = title_layout(hwnd).minimize.left - origin.x;
    let caption_left = if origin.y == 0 && caption_left < client.right {
        caption_left.max(0)
    } else {
        client.right
    };
    unsafe {
        let region = CreateRectRgn(0, 0, client.right, client.bottom);
        let cut = |left: i32, top: i32, right: i32, bottom: i32| {
            let part = CreateRectRgn(left, top, right, bottom);
            CombineRgn(region, region, part, RGN_DIFF);
            windows_sys::Win32::Graphics::Gdi::DeleteObject(part);
        };
        cut(caption_left, 0, client.right, strip_height);
        if band > 0 {
            cut(0, strip_height, client.right, strip_height + band);
        }
        // The system owns the region from here on.
        SetWindowRgn(group, region, 1);
    }
}

/// The group's answer to `WM_NCHITTEST`: its empty strip space, and a restored window's top
/// resize band, are the main window's caption (spec §4.1), so the window drags and resizes there.
pub(crate) fn group_hit_test(hwnd: HWND, group: HWND, lparam: LPARAM) -> LRESULT {
    use windows_sys::Win32::UI::WindowsAndMessaging::{HTCLIENT, HTTRANSPARENT, IsZoomed};
    let mut point = windows_sys::Win32::Foundation::POINT {
        x: (lparam as u32 & 0xffff) as u16 as i16 as i32,
        y: ((lparam as u32 >> 16) & 0xffff) as u16 as i16 as i32,
    };
    unsafe { windows_sys::Win32::Graphics::Gdi::ScreenToClient(group, &mut point) };
    let Some(layout) = group_id_of(hwnd, group).and_then(|id| strip_layout_of(hwnd, id)) else {
        return HTCLIENT as LRESULT;
    };
    let mut origin = windows_sys::Win32::Foundation::POINT { x: 0, y: 0 };
    unsafe { windows_sys::Win32::Graphics::Gdi::MapWindowPoints(group, hwnd, &mut origin, 1) };
    let in_title_row = origin.y == 0 && point.y >= 0 && point.y < layout.height;
    if !in_title_row {
        return HTCLIENT as LRESULT;
    }
    let resizes = unsafe { IsZoomed(hwnd) } == 0 && point.y < title_layout(hwnd).resize_border;
    let empty = layout.hit_test(crate::window::titlebar::Point::new(point.x, point.y))
        == crate::window::group_strip::StripTarget::Empty;
    if resizes || empty {
        HTTRANSPARENT as LRESULT
    } else {
        HTCLIENT as LRESULT
    }
}

/// A caption point (screen coordinates in `lparam`) over a title-row strip's empty space, as the
/// group window and its client coordinates.
fn caption_strip_point(hwnd: HWND, lparam: LPARAM) -> Option<(HWND, i32, i32)> {
    let groups = unsafe { app_ptr(hwnd) }.map(|app| {
        unsafe { app.as_ref() }
            .groups
            .iter()
            .map(|group| (group.id, group.hwnd))
            .collect::<Vec<_>>()
    })?;
    groups.into_iter().find_map(|(id, group)| {
        if !is_top_group(hwnd, group) {
            return None;
        }
        let mut point = windows_sys::Win32::Foundation::POINT {
            x: (lparam as u32 & 0xffff) as u16 as i16 as i32,
            y: ((lparam as u32 >> 16) & 0xffff) as u16 as i16 as i32,
        };
        unsafe { windows_sys::Win32::Graphics::Gdi::ScreenToClient(group, &mut point) };
        let layout = strip_layout_of(hwnd, id)?;
        (point.x >= 0
            && point.x < layout.bounds().right
            && point.y >= 0
            && point.y < layout.height
            && layout.hit_test(crate::window::titlebar::Point::new(point.x, point.y))
                == crate::window::group_strip::StripTarget::Empty)
            .then_some((group, point.x, point.y))
    })
}

/// Paints what the group's children leave uncovered: the tab strip, the preview divider, and the
/// empty hint while no tab is open.
pub(crate) fn paint_group(hwnd: HWND, group: HWND) {
    let mut paint = windows_sys::Win32::Graphics::Gdi::PAINTSTRUCT::default();
    let dc = unsafe { windows_sys::Win32::Graphics::Gdi::BeginPaint(group, &mut paint) };
    if dc.is_null() {
        return;
    }
    let Some(id) = group_id_of(hwnd, group) else {
        unsafe { windows_sys::Win32::Graphics::Gdi::EndPaint(group, &paint) };
        return;
    };
    let dpi = unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForWindow(group) }.max(96);
    let (palette, fonts, _) = title_chrome(hwnd);
    let mut client = RECT::default();
    unsafe {
        GetClientRect(group, &mut client);
    }
    if let Some(layout) = strip_layout_of(hwnd, id)
        && paint.rcPaint.top < layout.height
    {
        let (titles, active, _, _, preview_tab) = tab_snapshot(hwnd, id);
        let titles = titles.iter().map(String::as_str).collect::<Vec<_>>();
        let pointer = with_group_id(hwnd, id, |group| group.pointer).unwrap_or_default();
        let (is_active_group, group_count) = unsafe { app_ptr(hwnd) }
            .map(|app| {
                let app = unsafe { app.as_ref() };
                (app.tabs.active_group() == id, app.groups.len())
            })
            .unwrap_or((true, 1));
        unsafe {
            crate::window::group_strip::paint(
                dc,
                &layout,
                dpi,
                &crate::window::group_strip::StripPaint {
                    titles: &titles,
                    active,
                    preview_tab,
                    palette,
                    fonts,
                    pointer,
                    accent: crate::window::group_strip::tab_accent(
                        is_active_group,
                        group_count,
                        &palette,
                    ),
                },
            );
        }
        client.top = layout.height + menu_band_height(hwnd) + name_box_band_height(hwnd, dpi);
    }
    if group_tab_count(hwnd, id) == 0 {
        unsafe {
            crate::window::titlebar::paint_empty_hint(
                dc,
                client,
                EMPTY_TABS_HINT,
                palette,
                fonts.text(),
                dpi,
            );
        }
    }
    if let Some(divider) =
        crate::window::preview_host::with_group_host(hwnd, id, |host| host.divider_rect()).flatten()
    {
        unsafe { crate::window::titlebar::paint_divider(dc, divider, palette) };
    }
    unsafe {
        windows_sys::Win32::Graphics::Gdi::EndPaint(group, &paint);
    }
}

/// Focus given to the group window goes on to its content, as focus given to the frame does. With
/// no tab open the frame keeps it, to take the menu keys.
pub(crate) fn focus_group_content(hwnd: HWND) {
    let target = (tab_count(hwnd) > 0)
        .then(|| content_focus_target(hwnd))
        .flatten()
        .unwrap_or(hwnd);
    unsafe {
        SetFocus(target);
    }
}

fn open_find_bar(hwnd: HWND, mode: find_bar::FindBarMode) {
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
}

/// Makes the find bar the first time it's needed, with nothing of the `App` borrowed, because
/// creating its controls sends messages. Returns false when there's no bar and none could be
/// made.
fn ensure_find_bar(hwnd: HWND) -> bool {
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
fn show_search_view(hwnd: HWND) {
    // Read before the box takes the focus.
    let prefill = single_line_selection(hwnd);
    crate::window::side_panel::show_view(hwnd, crate::config::SidebarView::Search, true);
    if let Some(text) = prefill {
        crate::window::search_view::show_with_query(hwnd, &text);
    }
}

/// A `SearchToggle*` palette command shows the Search view first if it is hidden, so the option it
/// flips is visible.
fn toggle_search_option(hwnd: HWND, option: crate::search::SearchOption) {
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

/// Returns the keyboard focus to the content area.
pub(crate) fn focus_content(hwnd: HWND) {
    if let Some(target) = content_focus_target(hwnd) {
        unsafe {
            SetFocus(target);
        }
    }
}

/// The parts F6 moves between, in tab order (spec §10): the sidebar's two, then every editor
/// group by its place in the layout (split editors spec §6).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FocusPart {
    ActivityBar,
    Panel,
    Group(usize),
}

/// The part after `current`, skipping a closed panel and, with notes mode off, the sidebar.
pub(crate) fn next_focus_part(
    current: FocusPart,
    backwards: bool,
    sidebar: bool,
    panel_open: bool,
    groups: usize,
) -> FocusPart {
    let mut parts = Vec::new();
    if sidebar {
        parts.push(FocusPart::ActivityBar);
        if panel_open {
            parts.push(FocusPart::Panel);
        }
    }
    parts.extend((0..groups.max(1)).map(FocusPart::Group));
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
    let order = group_order(hwnd);
    let active = unsafe { app_ptr(hwnd) }.map(|app| unsafe { app.as_ref() }.tabs.active_group());
    let group_index = |id: Option<GroupId>| {
        id.and_then(|id| order.iter().position(|group| *group == id))
            .unwrap_or(0)
    };
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
        _ => {
            let inside = unsafe { app_ptr(hwnd) }
                .and_then(|app| unsafe { app.as_ref() }.group_containing(focus));
            FocusPart::Group(group_index(inside.or(active)))
        }
    };
    match next_focus_part(
        current,
        backwards,
        windows.is_some(),
        panel_open,
        order.len(),
    ) {
        FocusPart::ActivityBar => {
            if let Some((bar, _)) = windows {
                unsafe {
                    SetFocus(bar);
                }
            }
        }
        FocusPart::Panel => side_panel::show_view(hwnd, side_panel::current_view(hwnd), true),
        FocusPart::Group(index) => {
            if let Some(group) = order.get(index) {
                activate_group(hwnd, *group);
            }
            return_focus_to_editor(hwnd);
        }
    }
}

/// The colors the find bar and the name box are shown in.
pub(crate) fn current_palette(hwnd: HWND) -> Palette {
    title_chrome(hwnd).0
}

/// The Notebook view's file-type icon colours for the current theme. Call it with nothing of
/// the App borrowed.
pub(crate) fn current_file_icons(hwnd: HWND) -> crate::window::palette::FileIcons {
    unsafe { app_ptr(hwnd) }.map_or_else(crate::window::palette::FileIcons::neutral, |app| {
        let app = unsafe { app.as_ref() };
        crate::window::palette::FileIcons::for_cached_theme(app.theme, app.settings.theme)
    })
}

/// The Notebook tree's icon set and whether the theme is light (icon sets spec §3.1). Call it
/// with nothing of the App borrowed. The neutral first-paint theme is light.
pub(crate) fn current_icon_style(hwnd: HWND) -> (crate::config::FileIconSet, bool) {
    unsafe { app_ptr(hwnd) }.map_or((crate::config::FileIconSet::default(), true), |app| {
        let app = unsafe { app.as_ref() };
        let light = app
            .theme
            .is_none_or(|theme| !theme.effective_theme(app.settings.theme).is_dark());
        (app.settings.file_icons, light)
    })
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
            .find_bar()
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

/// Where keyboard focus belongs in the content area: the image view for an image tab, the preview
/// while it replaces the editor in Full mode, otherwise the editor.
fn content_focus_target(hwnd: HWND) -> Option<HWND> {
    crate::window::image_host::shown_view_hwnd(hwnd)
        .or_else(|| crate::window::preview_host::full_view_hwnd(hwnd))
        .or_else(|| unsafe { editor_hwnd(hwnd) })
}

/// Overlays the palette at the top of the editor area, even with no tab open (New and Open stay
/// available then), below the tab strip and a visible find bar or name box so all stay usable.
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

/// The theme's link colour, as the Markdown preview draws links.
pub(crate) fn link_color(hwnd: HWND) -> u32 {
    let theme = effective_theme(hwnd);
    let high_contrast = unsafe { app_ptr(hwnd) }.is_some_and(|app| {
        unsafe { app.as_ref() }
            .theme
            .is_some_and(|system| system.high_contrast)
    });
    crate::preview::colors::preview_colors(theme, high_contrast).link
}

/// Help → About FastPad, in the current theme's colors and the Markdown preview's link color.
fn show_about(hwnd: HWND) {
    crate::window::about::show(hwnd, current_palette(hwnd), link_color(hwnd));
}

/// The Settings dialog: File → Settings…, Ctrl+, and the activity bar's gear (settings dialog
/// spec §4.3).
pub(crate) fn show_settings(hwnd: HWND) {
    show_settings_page(hwnd, crate::window::settings_model::Page::General);
}

/// Preferences: Open Keyboard Shortcuts (keyboard shortcuts spec section 2).
pub(crate) fn show_keyboard_shortcuts(hwnd: HWND) {
    show_settings_page(hwnd, crate::window::settings_model::Page::Shortcuts);
}

fn show_settings_page(hwnd: HWND, page: crate::window::settings_model::Page) {
    let outcome =
        crate::window::settings_dialog::show(hwnd, current_palette(hwnd), link_color(hwnd), page);
    if outcome == crate::window::settings_dialog::Outcome::EditIni {
        edit_settings_file(hwnd);
    }
}

const EDIT_INI_NOTICE: &str = "Changes saved in fastpad.ini apply the next time FastPad starts.";

/// Preferences: Edit fastpad.ini. Creates the file (empty) when it doesn't exist yet, then opens
/// it in a tab through the normal open path (settings dialog spec §3.6).
pub(crate) fn edit_settings_file(hwnd: HWND) {
    let result = settings_file_for_editing().and_then(|path| {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        open_path(hwnd, &path)
    });
    match result {
        // Settings are read once, at startup: say so rather than leave a saved edit looking
        // ignored.
        Ok(()) => push_notice(hwnd, EDIT_INI_NOTICE.to_owned()),
        Err(error) => push_notice(hwnd, format!("FastPad could not open fastpad.ini: {error}")),
    }
}

#[cfg(not(test))]
fn settings_file_for_editing() -> Result<std::path::PathBuf> {
    crate::config::persisted::settings_file_path()
}

/// Tests open only the file they chose with `save_settings_to`.
#[cfg(test)]
fn settings_file_for_editing() -> Result<std::path::PathBuf> {
    TEST_SETTINGS_PATH
        .with(|path| path.borrow().clone())
        .ok_or(crate::FastPadError::Invariant(
            "a test opened fastpad.ini without save_settings_to",
        ))
}

pub(crate) fn open_command_palette(hwnd: HWND) {
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
        Some(newly_shown)
    });
    let Some(newly_shown) = newly_shown else {
        return;
    };
    if newly_shown {
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

/// Ctrl+P, the palette row and File → Go to note… (quick-open spec §3.1): the palette in the
/// `QuickOpen` picker with an empty query. While that picker already shows, nothing changes.
pub(crate) fn open_quick_open(hwnd: HWND) {
    let (visible, showing) = with_command_palette(hwnd, |palette| {
        let quick_open = palette
            .picker()
            .is_some_and(|picker| picker.kind == command_palette::PickerKind::QuickOpen);
        (palette.is_visible(), palette.is_visible() && quick_open)
    })
    .unwrap_or((false, false));
    if showing {
        return;
    }
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    // Still made on first use (spec §3.6), and with nothing borrowed: it creates windows.
    let missing = unsafe { app_ptr(hwnd) }
        .is_some_and(|app| unsafe { app.as_ref() }.command_palette.is_none());
    if missing {
        let Ok(created) = CommandPalette::create(hwnd) else {
            return;
        };
        if !identity.is_live_for(hwnd) {
            return;
        }
        if let Some(mut app) = unsafe { app_ptr(hwnd) } {
            let app = unsafe { app.as_mut() };
            if app.command_palette.is_none() {
                app.command_palette = Some(created);
            }
        }
    }
    // Switching from the open command list keeps the focus the palette first took from.
    if !visible {
        capture_palette_focus(hwnd);
    }
    let colors = title_chrome(hwnd).0;
    let shown = unsafe { app_ptr(hwnd) }.and_then(|mut app| {
        let palette = unsafe { app.as_mut() }.command_palette.as_mut()?;
        palette.mark_shown(colors);
        palette.set_picker(Some(command_palette::Picker {
            kind: command_palette::PickerKind::QuickOpen,
            items: Vec::new(),
            create: None,
        }));
        Some(())
    });
    if shown.is_none() {
        return;
    }
    // Always from an empty query. Clearing sends EN_CHANGE, which lists the rows.
    with_command_palette(hwnd, CommandPalette::clear_query);
    if !identity.is_live_for(hwnd) {
        return;
    }
    refilter_command_palette(hwnd);
    with_command_palette(hwnd, CommandPalette::focus_query);
}

/// The quick-open rows for `query` and the row to select (spec §3.1–3.4). Only reads what is
/// in memory: the tabs and `LibraryState.notes`.
fn quick_open_rows(hwnd: HWND, query: &str) -> (Vec<command_palette::PickerRow>, Option<usize>) {
    use crate::library::quick_open::{self, QuickMatch};
    use command_palette::PickerRow;
    let (text, line) = quick_open::split_line(query);
    let typed = !text.trim().is_empty();
    // `:<n>` alone needs no notebook: it moves the current tab's caret.
    if let Some(line) = line.filter(|_| !typed) {
        return (vec![PickerRow::GoToLine(line)], Some(0));
    }
    let Some(folder) = crate::window::library_host::folder(hwnd) else {
        return (vec![PickerRow::Notice(command_palette::NO_NOTEBOOK)], None);
    };
    if typed {
        let rows = crate::window::library_host::with_state(hwnd, |state| {
            quick_open::search(
                state.notes.iter().map(|note| &note.path),
                text,
                command_palette::QUICK_OPEN_ROWS,
            )
        })
        .unwrap_or_default()
        .into_iter()
        .map(|found| PickerRow::Note { found, line })
        .collect::<Vec<_>>();
        let selected = (!rows.is_empty()).then_some(0);
        return (rows, selected);
    }
    // Nothing typed: the notes open in tabs, the most recently used first (spec §3.2), across
    // groups (split editors spec §7). Tabs outside the notebook are left out here, untitled ones
    // by having no path.
    let order = group_order(hwnd);
    let number = |group: GroupId| {
        (order.len() > 1)
            .then(|| {
                order
                    .iter()
                    .position(|id| *id == group)
                    .map(|index| index + 1)
            })
            .flatten()
    };
    let open = unsafe { app_ptr(hwnd) }
        .map(|app| {
            let tabs = &unsafe { app.as_ref() }.tabs;
            let active = tabs
                .active()
                .map(|document| (tabs.active_group(), document.id));
            tabs.activation_order()
                .iter()
                .filter_map(|&(group, id)| {
                    let path = tabs.document(id)?.path.as_deref()?;
                    let relative = crate::library::record_path(&folder, path);
                    (!relative.is_absolute()).then(|| {
                        (
                            crate::library::path_key(&relative),
                            Some((group, id)) == active,
                            group,
                        )
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if open.is_empty() {
        return (Vec::new(), None);
    }
    // Then only the notes the library lists, spelled as it spells them.
    let (rows, first_active) = crate::window::library_host::with_state(hwnd, |state| {
        let wanted = open
            .iter()
            .map(|(key, ..)| key.as_str())
            .collect::<std::collections::HashSet<_>>();
        let mut listed = std::collections::HashMap::new();
        for note in &state.notes {
            let key = crate::library::path_key(&note.path);
            if wanted.contains(key.as_str()) {
                listed.insert(key, note.path.clone());
            }
        }
        let mut rows = Vec::new();
        let mut first_active = false;
        for (key, active, group) in &open {
            let Some(path) = listed.get(key) else {
                continue;
            };
            let Some(found) = QuickMatch::plain(path) else {
                continue;
            };
            if rows.is_empty() {
                first_active = *active;
            }
            rows.push(PickerRow::View {
                found,
                group: *group,
                number: number(*group),
            });
        }
        (rows, first_active)
    })
    .unwrap_or_default();
    // The current note leads, so the selection starts on the one before it: Ctrl+P then Enter
    // goes back to the previous note.
    let selected = match rows.len() {
        0 => None,
        1 => Some(0),
        _ if first_active => Some(1),
        _ => Some(0),
    };
    (rows, selected)
}

/// Shows `id`'s view in `group`, making that group active; never adds a view. False when `group`
/// has no view of `id`. The focus stays where it is.
pub(crate) fn focus_view(hwnd: HWND, group: GroupId, id: DocumentId) -> bool {
    let revision = unsafe { app_ptr(hwnd) }.and_then(|app| {
        let tabs = unsafe { app.as_ref() }.tabs.group(group)?;
        tabs.contains(id).then(|| tabs.view().snapshot().revision)
    });
    let Some(revision) = revision else {
        return false;
    };
    activate_group(hwnd, group);
    let shown = unsafe { app_ptr(hwnd) }.is_some_and(|app| {
        unsafe { app.as_ref() }
            .tabs
            .group(group)
            .is_some_and(|tabs| tabs.active_document() == Some(id))
    });
    shown || activate_document_in(hwnd, group, id, revision)
}

/// Opens a quick-open pick (spec §3.5). `relative` is resolved again against the notebook's
/// notes, since it may have left the library since the list was shown; then it opens as a
/// normal tab (an open one is switched to) with the focus in the editor, and `line` applies.
pub(crate) fn open_quick_open_choice(hwnd: HWND, relative: &std::path::Path, line: Option<u32>) {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    let Some(folder) = crate::window::library_host::folder(hwnd) else {
        return;
    };
    let path = folder.join(relative);
    let key = crate::library::path_key(relative);
    let listed = crate::window::library_host::with_state(hwnd, |state| {
        state
            .notes
            .iter()
            .any(|note| crate::library::path_key(&note.path) == key)
    })
    .unwrap_or(false);
    if !listed {
        report_open_failure(
            hwnd,
            &path,
            &crate::FastPadError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "the note is no longer in the notebook",
            )),
        );
        return;
    }
    if let Err(error) = open_note(hwnd, &path, OpenMode::Permanent, true) {
        report_open_failure(hwnd, &path, &error);
        return;
    }
    if let Some(line) = line
        && identity.is_live_for(hwnd)
    {
        go_to_line(hwnd, line);
    }
}

/// Moves the active tab's caret to the start of 1-based `line`, the last line when past the end,
/// and scrolls it into view (spec §3.4). Does nothing with no tab open.
pub(crate) fn go_to_line(hwnd: HWND, line: u32) {
    if tab_count(hwnd) == 0 {
        return;
    }
    let Some(editor) =
        unsafe { app_ptr(hwnd) }.and_then(|app| unsafe { app.as_ref() }.editor().cloned())
    else {
        return;
    };
    let _ = editor.go_to_line(line.saturating_sub(1) as usize);
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
        let quick_open = with_command_palette(hwnd, |palette| {
            palette
                .picker()
                .is_some_and(|picker| picker.kind == command_palette::PickerKind::QuickOpen)
        })
        .unwrap_or(false);
        // Built before the palette is borrowed: the rows read the tabs and the library.
        let quick_rows = quick_open.then(|| quick_open_rows(hwnd, &query));
        if let Some(mut app) = unsafe { app_ptr(hwnd) }
            && let Some(palette) = unsafe { app.as_mut() }.command_palette.as_mut()
        {
            match quick_rows {
                Some((rows, selected)) => palette.set_picker_rows(rows, selected),
                None => {
                    if let Some(picker) = palette.picker() {
                        let rows = command_palette::picker_rows(picker, &query);
                        let selected = (!rows.is_empty()).then_some(0);
                        palette.set_picker_rows(rows, selected);
                    }
                }
            }
        }
    } else {
        let has_tabs = tab_count(hwnd) > 0;
        let markdown = crate::window::preview_host::buttons_visible(hwnd);
        let image = crate::window::image_host::active_is_image(hwnd);
        let sidebar = notes_mode_enabled(hwnd);
        // New note and New folder need a notebook, open or loading, to put the item in (inline
        // naming spec §3.1).
        let notebook = crate::window::library_host::folder(hwnd).is_some();
        let groups = unsafe { app_ptr(hwnd) }.map_or(1, |app| unsafe { app.as_ref() }.groups.len());
        let entries = command_palette::filter_entries(&query, |command| {
            (has_tabs || !command.needs_document())
                && (!image || !command.needs_text())
                && (markdown || !command.is_markdown_preview())
                && (sidebar || !command.is_sidebar())
                && (notebook || !matches!(command, CommandId::NoteNew | CommandId::NoteNewFolder))
                // Close Group with one empty group would do nothing.
                && (has_tabs || groups > 1 || command != CommandId::CloseGroup)
        });
        let keymap = keymap(hwnd);
        if let Some(mut app) = unsafe { app_ptr(hwnd) }
            && let Some(palette) = unsafe { app.as_mut() }.command_palette.as_mut()
        {
            palette.set_entries(entries, &keymap);
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
        } else if let Some(bar) = app
            .groups
            .iter()
            .filter_map(|group| group.find_bar.as_ref())
            .find(|bar| bar.owns(panel))
        {
            bar.paint_panel(panel, glyph_font, text_font);
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
    // A quick-open row that can't be picked ("No notebook is open"), or no row at all, leaves
    // the picker open (spec §3.1).
    if matches!(pick, Some((command_palette::PickerKind::QuickOpen, None))) {
        return;
    }
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
        Some(bar.toggle_tools())
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
}

/// `EN_CHANGE` from a find field. Its text is read with nothing of the `App` borrowed and kept
/// as the field's accessible value.
fn find_field_changed(hwnd: HWND, control: HWND) {
    let text = find_bar::control_text(control);
    let query = unsafe { app_ptr(hwnd) }.is_some_and(|mut app| {
        unsafe { app.as_mut() }.find_bar_mut().is_some_and(|bar| {
            bar.field_changed(control, text);
            bar.is_query(control)
        })
    });
    if query {
        set_find_no_match(hwnd, false);
    }
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

fn name_box_owns(hwnd: HWND, control: HWND) -> bool {
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
fn find_again(hwnd: HWND, backward: bool) {
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
fn select_match(
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

const EMPTY_TABS_HINT: &str =
    "No tabs are open.\nPress Ctrl+N or double-click the tab bar to start a new one.";

pub(crate) fn tab_count(hwnd: HWND) -> usize {
    unsafe { app_ptr(hwnd) }
        .map(|app| unsafe { app.as_ref() }.tabs.len())
        .unwrap_or(1)
}

pub(crate) fn title_layout(hwnd: HWND) -> TitleBarLayout {
    crate::window::titlebar::layout_for_window(hwnd)
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

/// The window title for the active tab, as the title bar and the taskbar show it.
fn active_window_title(hwnd: HWND) -> String {
    window_title(
        unsafe { app_ptr(hwnd) }
            .and_then(|app| unsafe { app.as_ref() }.tabs.active().map(Document::title))
            .as_deref(),
    )
}

/// Brings the window text in line with the active tab. Runs on every frame paint, since every tab
/// change (switch, open, close, save, dirty state) repaints the title strip.
fn sync_window_title(hwnd: HWND) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetWindowTextW, SetWindowTextW};
    if unsafe { app_ptr(hwnd) }.is_none() {
        return;
    }
    let title = active_window_title(hwnd);
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

/// Group `id`'s tab titles, selected tab, strip scroll, whether it is empty, and its preview tab.
fn tab_snapshot(hwnd: HWND, id: GroupId) -> (Vec<String>, usize, i32, bool, Option<usize>) {
    unsafe { app_ptr(hwnd) }
        .and_then(|app| {
            let app = unsafe { app.as_ref() };
            let group = app.tabs.group(id)?;
            let documents = app.tabs.group_documents(id);
            Some((
                documents.iter().map(|document| document.title()).collect(),
                group.active_index(),
                group.scroll_offset(),
                app.group(id).is_some() && group.is_empty(),
                documents.iter().position(|document| document.preview),
            ))
        })
        .unwrap_or_else(|| (vec!["Untitled".to_owned()], 0, 0, false, None))
}

fn group_tab_count(hwnd: HWND, id: GroupId) -> usize {
    unsafe { app_ptr(hwnd) }
        .and_then(|app| Some(unsafe { app.as_ref() }.tabs.group(id)?.len()))
        .unwrap_or(0)
}

/// Runs `f` on the active editor group's window state.
pub(crate) fn with_group<R>(
    hwnd: HWND,
    f: impl FnOnce(&mut crate::window::editor_group::GroupWindow) -> R,
) -> Option<R> {
    let mut app = unsafe { app_ptr(hwnd) }?;
    unsafe { app.as_mut() }.active_group_mut().map(f)
}

/// The title-row strip in the main window's client coordinates, which the frame leaves to the
/// group when it paints the title row.
fn group_strip_bounds(hwnd: HWND) -> Vec<crate::window::titlebar::Rect> {
    let groups = unsafe { app_ptr(hwnd) }
        .map(|app| {
            unsafe { app.as_ref() }
                .groups
                .iter()
                .map(|group| (group.id, group.hwnd))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    groups
        .into_iter()
        .filter_map(|(id, group)| {
            let bounds = strip_layout_of(hwnd, id)?.bounds();
            let mut origin = windows_sys::Win32::Foundation::POINT { x: 0, y: 0 };
            unsafe {
                windows_sys::Win32::Graphics::Gdi::MapWindowPoints(group, hwnd, &mut origin, 1)
            };
            (origin.y == 0).then(|| {
                crate::window::titlebar::Rect::new(
                    origin.x,
                    0,
                    origin.x + bounds.right,
                    bounds.bottom,
                )
            })
        })
        .collect()
}

/// The active group's tab strip as laid out now, the same one it paints and hit-tests with.
pub(crate) fn strip_layout(hwnd: HWND) -> Option<crate::window::group_strip::StripLayout> {
    let id = unsafe { app_ptr(hwnd) }.map(|app| unsafe { app.as_ref() }.tabs.active_group())?;
    strip_layout_of(hwnd, id)
}

/// Group `id`'s tab strip as laid out now.
pub(crate) fn strip_layout_of(
    hwnd: HWND,
    id: GroupId,
) -> Option<crate::window::group_strip::StripLayout> {
    let (group, count, scroll) = unsafe { app_ptr(hwnd) }.and_then(|app| {
        let app = unsafe { app.as_ref() };
        let tabs = app.tabs.group(id)?;
        Some((app.group(id)?.hwnd, tabs.len(), tabs.scroll_offset()))
    })?;
    let dpi = unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForWindow(group) }.max(96);
    Some(crate::window::group_strip::StripLayout::calculate(
        crate::window::group_strip::strip_width(group),
        dpi,
        count,
        scroll,
    ))
}

pub(crate) fn invalidate_group_strip(hwnd: HWND, id: GroupId) {
    let (Some(group), Some(layout)) = (
        with_group_id(hwnd, id, |state| state.hwnd),
        strip_layout_of(hwnd, id),
    ) else {
        return;
    };
    let bounds = layout.bounds();
    let rect = RECT {
        left: bounds.left,
        top: bounds.top,
        right: bounds.right,
        bottom: bounds.bottom,
    };
    unsafe {
        InvalidateRect(group, &rect, 0);
    }
}

pub(crate) fn update_strip_pointer(
    hwnd: HWND,
    id: GroupId,
    update: impl FnOnce(
        crate::window::group_strip::StripPointer,
    ) -> crate::window::group_strip::StripPointer,
) {
    let changed = with_group_id(hwnd, id, |group| {
        let next = update(group.pointer);
        let changed = next != group.pointer;
        group.pointer = next;
        changed
    })
    .unwrap_or(false);
    if changed {
        invalidate_group_strip(hwnd, id);
    }
}

/// What the group-client point is over in group `id`'s strip; `None` below it.
pub(crate) fn strip_target(
    hwnd: HWND,
    id: GroupId,
    x: i32,
    y: i32,
) -> Option<crate::window::group_strip::StripTarget> {
    let layout = strip_layout_of(hwnd, id)?;
    (y >= 0 && y < layout.height)
        .then(|| layout.hit_test(crate::window::titlebar::Point::new(x, y)))
}

/// Scrolls the tabs when the wheel turns over the strip; reports whether it was over it. The
/// point is in screen coordinates, as wheel messages carry it.
fn scroll_tabs(hwnd: HWND, id: GroupId, group: HWND, lparam: LPARAM, delta: i32) -> bool {
    let mut point = windows_sys::Win32::Foundation::POINT {
        x: (lparam as u32 & 0xffff) as u16 as i16 as i32,
        y: ((lparam as u32 >> 16) & 0xffff) as u16 as i16 as i32,
    };
    unsafe {
        windows_sys::Win32::Graphics::Gdi::ScreenToClient(group, &mut point);
    }
    let Some(layout) = strip_layout_of(hwnd, id) else {
        return false;
    };
    if point.y < 0 || point.y >= layout.height || point.x < 0 || point.x >= layout.tabs.right {
        return false;
    }
    let scroll = layout.scroll_by_wheel(delta, WHEEL_DELTA as i32);
    if set_strip_scroll(hwnd, id, scroll) {
        // The hovered tab moved out from under the pointer; the next mouse move finds the new one.
        update_strip_pointer(hwnd, id, |pointer| pointer.hover(None));
        invalidate_group_strip(hwnd, id);
    }
    true
}

/// Scrolls group `id`'s tabs to `offset`; returns whether it moved.
fn set_strip_scroll(hwnd: HWND, id: GroupId, offset: i32) -> bool {
    unsafe { app_ptr(hwnd) }.is_some_and(|app| {
        unsafe { app.as_ref() }
            .tabs
            .group(id)
            .is_some_and(|tabs| tabs.selection().set_scroll_offset(offset))
    })
}

/// Starts dragging the tab scroll thumb. Pressing the track beside the thumb first jumps the
/// thumb there, centred under the pointer, so the same press can keep dragging it.
fn begin_tab_thumb_drag(hwnd: HWND, id: GroupId, group: HWND, x: i32) {
    let Some(layout) = strip_layout_of(hwnd, id) else {
        return;
    };
    let Some(thumb) = layout.scroll_thumb() else {
        return;
    };
    let grab = if (thumb.left..thumb.right).contains(&x) {
        x - thumb.left
    } else {
        let grab = (thumb.right - thumb.left) / 2;
        set_strip_scroll(hwnd, id, layout.scroll_for_thumb(x - grab));
        grab
    };
    with_group_id(hwnd, id, |state| state.thumb_grab = Some(grab));
    unsafe {
        SetCapture(group);
    }
    invalidate_group_strip(hwnd, id);
}

/// Follows the pointer while the thumb is dragged; reports whether a drag is in progress.
fn drag_tab_thumb(hwnd: HWND, id: GroupId, x: i32) -> bool {
    let Some(grab) = with_group_id(hwnd, id, |group| group.thumb_grab).flatten() else {
        return false;
    };
    let Some(layout) = strip_layout_of(hwnd, id) else {
        return true;
    };
    if set_strip_scroll(hwnd, id, layout.scroll_for_thumb(x - grab)) {
        invalidate_group_strip(hwnd, id);
    }
    true
}

/// Ends a thumb drag; reports whether one was in progress, so the release activates nothing.
fn end_tab_thumb_drag(hwnd: HWND, id: GroupId) -> bool {
    let dragging = with_group_id(hwnd, id, |group| group.thumb_grab.take())
        .flatten()
        .is_some();
    if dragging {
        unsafe {
            ReleaseCapture();
        }
    }
    dragging
}

/// Opens the tab-strip menu at group-client `x`, `y`. The menu belongs to the main window, whose
/// modal accounting holds deferred work while it is open.
fn show_group_strip_menu(hwnd: HWND, group: HWND, x: i32, y: i32) {
    let mut point = windows_sys::Win32::Foundation::POINT { x, y };
    unsafe {
        windows_sys::Win32::Graphics::Gdi::MapWindowPoints(group, hwnd, &mut point, 1);
    }
    let has_tabs = tab_count(hwnd) > 0;
    if let Some(command) = menus::show_tab_strip_menu(hwnd, point.x, point.y, has_tabs) {
        execute_command(hwnd, command);
    }
}

/// The tab strip's share of the group window's pointer messages (split editors spec §4.2): what
/// the title bar used to do for the tabs. `None` leaves the message to the window's default.
pub(crate) fn group_strip_message(
    hwnd: HWND,
    group: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> Option<LRESULT> {
    use crate::window::group_strip::StripTarget;
    // Everything below acts on this strip's own group; a press has already made it active.
    let id = group_id_of(hwnd, group)?;
    let (x, y) = (
        (lparam as u32 & 0xffff) as u16 as i16 as i32,
        ((lparam as u32 >> 16) & 0xffff) as u16 as i16 as i32,
    );
    // Clicking the strip leaves menu mode, as a click anywhere else off the menu band does.
    if matches!(
        message,
        WM_LBUTTONDOWN | WM_LBUTTONDBLCLK | WM_MBUTTONDOWN | WM_RBUTTONUP
    ) {
        exit_menu_mode(hwnd);
    }
    match message {
        WM_RBUTTONDOWN if crate::window::tab_drag::cancel_for_right_press(hwnd) => Some(0),
        WM_MOUSEMOVE => {
            if drag_tab_thumb(hwnd, id, x) {
                return Some(0);
            }
            if crate::window::tab_drag::mouse_move(hwnd, group, x, y, wparam) {
                return Some(0);
            }
            let mut track = windows_sys::Win32::UI::Input::KeyboardAndMouse::TRACKMOUSEEVENT {
                cbSize: std::mem::size_of::<
                    windows_sys::Win32::UI::Input::KeyboardAndMouse::TRACKMOUSEEVENT,
                >() as u32,
                dwFlags: windows_sys::Win32::UI::Input::KeyboardAndMouse::TME_LEAVE,
                hwndTrack: group,
                dwHoverTime: 0,
            };
            unsafe { windows_sys::Win32::UI::Input::KeyboardAndMouse::TrackMouseEvent(&mut track) };
            let target = strip_target(hwnd, id, x, y);
            update_strip_pointer(hwnd, id, |pointer| pointer.hover(target));
            Some(0)
        }
        WM_MOUSELEAVE => {
            update_strip_pointer(hwnd, id, |pointer| pointer.hover(None));
            // A middle press whose release never reaches the strip must not close a tab later.
            with_group_id(hwnd, id, |state| state.middle_press = None);
            Some(0)
        }
        // The class has CS_DBLCLKS, so a second click comes as a double-click: on empty strip it
        // opens a tab (VS Code style); anywhere else it is one more press.
        WM_LBUTTONDBLCLK if strip_target(hwnd, id, x, y) == Some(StripTarget::Empty) => {
            execute_command(hwnd, CommandId::New);
            Some(0)
        }
        WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
            let target = strip_target(hwnd, id, x, y)?;
            update_strip_pointer(hwnd, id, |pointer| {
                pointer.hover(Some(target)).press(Some(target))
            });
            if target == StripTarget::ScrollBar {
                begin_tab_thumb_drag(hwnd, id, group, x);
            }
            if let StripTarget::Tab(index) = target {
                crate::window::tab_drag::arm(hwnd, id, group, index, x, y);
            }
            Some(0)
        }
        // A release acts only over the target its press went down on, so the release that ends
        // a double-click on empty strip never hits the tab that double-click just opened.
        WM_LBUTTONUP => {
            if crate::window::tab_drag::release(hwnd, group, x, y, wparam) {
                update_strip_pointer(hwnd, id, |pointer| pointer.press(None));
                return Some(0);
            }
            let mut activated = None;
            update_strip_pointer(hwnd, id, |pointer| {
                let (next, released) = pointer.release(strip_target(hwnd, id, x, y));
                activated = released;
                next
            });
            if end_tab_thumb_drag(hwnd, id) {
                return Some(0);
            }
            match activated? {
                StripTarget::CloseTab(index) => {
                    activate_tab(hwnd, index);
                    execute_command(hwnd, CommandId::CloseTab);
                }
                StripTarget::Tab(index) => {
                    activate_tab(hwnd, index);
                    if tab_double_click(hwnd, index)
                        && let Some(id) = unsafe { app_ptr(hwnd) }
                            .and_then(|app| Some(unsafe { app.as_ref() }.tabs.active()?.id))
                    {
                        promote_tab(hwnd, id);
                    }
                }
                StripTarget::ScrollBar | StripTarget::Empty => {}
            }
            Some(0)
        }
        WM_RBUTTONUP if crate::window::tab_drag::right_release(hwnd) => Some(0),
        WM_RBUTTONUP if strip_target(hwnd, id, x, y) == Some(StripTarget::Empty) => {
            show_group_strip_menu(hwnd, group, x, y);
            Some(0)
        }
        // A tab's menu acts on that tab, so it is activated first.
        WM_RBUTTONUP => match strip_target(hwnd, id, x, y) {
            Some(StripTarget::Tab(index) | StripTarget::CloseTab(index)) => {
                activate_tab(hwnd, index);
                let mut point = windows_sys::Win32::Foundation::POINT { x, y };
                unsafe {
                    windows_sys::Win32::Graphics::Gdi::MapWindowPoints(group, hwnd, &mut point, 1)
                };
                if let Some(command) = menus::show_tab_menu(hwnd, point.x, point.y) {
                    execute_command(hwnd, command);
                }
                Some(0)
            }
            _ => None,
        },
        // A middle-click closes the tab under the pointer (quick-open spec §5).
        WM_MBUTTONDOWN => {
            let press = match strip_target(hwnd, id, x, y) {
                Some(StripTarget::Tab(index) | StripTarget::CloseTab(index)) => {
                    tab_id_at(hwnd, index).map(|id| (index, id))
                }
                _ => None,
            };
            with_group_id(hwnd, id, |state| state.middle_press = press);
            Some(0)
        }
        WM_MBUTTONUP => {
            let press = with_group_id(hwnd, id, |state| state.middle_press.take()).flatten();
            // Only over the pressed tab, and only while it still shows the same document.
            if let Some((index, document)) = press
                && let Some(StripTarget::Tab(released) | StripTarget::CloseTab(released)) =
                    strip_target(hwnd, id, x, y)
                && released == index
                && tab_id_at(hwnd, index) == Some(document)
            {
                close_tab_at(hwnd, index);
            }
            Some(0)
        }
        WM_MOUSEWHEEL | WM_MOUSEHWHEEL => {
            let delta = i32::from((wparam >> 16) as u16 as i16);
            // Wheel up scrolls toward the first tab; a tilt to the right toward the last.
            let delta = if message == WM_MOUSEWHEEL {
                -delta
            } else {
                delta
            };
            scroll_tabs(hwnd, id, group, lparam, delta).then_some(0)
        }
        WM_CAPTURECHANGED => {
            with_group_id(hwnd, id, |state| state.thumb_grab = None);
            if lparam as HWND != group {
                crate::window::tab_drag::cancel(hwnd);
            }
            Some(0)
        }
        _ => None,
    }
}

/// The tab strip's accessible object, for the group window's `WM_GETOBJECT`.
pub(crate) fn group_accessible_object(hwnd: HWND, group: HWND, wparam: WPARAM) -> LRESULT {
    let provider = unsafe { app_ptr(hwnd) }.and_then(|mut app| {
        let app = unsafe { app.as_mut() };
        let id = app.groups.iter().find(|state| state.hwnd == group)?.id;
        let tabs = app.tabs.group(id)?;
        let (view, selection) = (tabs.view(), tabs.selection());
        let state = app.group_mut(id)?;
        Some(state.accessibility.ensure(
            group,
            crate::window::accessibility::ProviderKind::GroupStrip,
            view,
            selection,
        ))
    });
    match provider {
        Some(provider) => unsafe { crate::window::accessibility::object_result(provider, wparam) },
        None => 0,
    }
}

/// Follows every change to the set of tabs or the active one: scrolls the active tab into view,
/// shows the editor only while a tab is open, and repaints.
pub(crate) fn refresh_tabs(hwnd: HWND) {
    let Some((count, active, editor_hwnd)) = (unsafe { app_ptr(hwnd) }).map(|app| {
        let app = unsafe { app.as_ref() };
        (
            app.tabs.len(),
            app.tabs.active_index(),
            app.editor().map(Editor::hwnd),
        )
    }) else {
        return;
    };
    let scroll = if count == 0 {
        0
    } else {
        strip_layout(hwnd).map_or(0, |layout| layout.scroll_to_reveal(active))
    };
    if let Some(app) = unsafe { app_ptr(hwnd) } {
        unsafe { app.as_ref() }.tabs.set_scroll_offset(scroll);
    }
    // An image tab shows the image view instead of the editor (image preview spec §5).
    let text_active = count > 0 && !crate::window::image_host::active_is_image(hwnd);
    if let Some(editor_hwnd) = editor_hwnd {
        let visible = unsafe { GetWindowLongPtrW(editor_hwnd, GWL_STYLE) } as u32 & WS_VISIBLE != 0;
        if !text_active && visible {
            close_find_bar(hwnd);
            unsafe {
                ShowWindow(editor_hwnd, SW_HIDE);
                if GetFocus() == editor_hwnd {
                    SetFocus(hwnd);
                }
            }
        } else if text_active && !visible {
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
    crate::window::image_host::sync(hwnd);
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

#[cfg(test)]
thread_local! {
    /// The last command `execute_command_with_note` received, for the tests that check what a
    /// key chord runs.
    static LAST_COMMAND: std::cell::Cell<Option<CommandId>> = const { std::cell::Cell::new(None) };
}

/// Counts `is_sidebar` commands that ran past the notes-mode guard below. The sidebar's own state
/// (`app.sidebar`, the Search view's options) already reads as empty/default with no sidebar to
/// hold it, so a test disabling notes mode has nothing else to observe; this hook makes the guard
/// itself a regression test rather than an untested `if`.
#[cfg(test)]
static SIDEBAR_COMMAND_RUNS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

#[cfg(test)]
fn sidebar_command_runs() -> u32 {
    SIDEBAR_COMMAND_RUNS.load(std::sync::atomic::Ordering::Relaxed)
}

/// `execute_command`, for a command chosen from the command palette: `recorded` is the sidebar
/// row that `capture_palette_focus` recorded when the palette opened, taken by
/// `run_command_palette_selection` before closing it moved focus off the panel (spec §6.3).
fn execute_command_with_note(hwnd: HWND, command: CommandId, recorded: Option<std::path::PathBuf>) {
    #[cfg(test)]
    LAST_COMMAND.with(|last| last.set(Some(command)));
    exit_menu_mode(hwnd);
    if file_population_active(hwnd) {
        return;
    }
    let tree_note = is_note_command(command)
        .then(|| recorded.or_else(|| crate::window::notebook_view::focused_note(hwnd)))
        .flatten();
    // Rename and Delete on a focused folder row act on the folder (notebook folders spec §4.2).
    let tree_folder = (matches!(command, CommandId::NoteRename | CommandId::NoteDelete)
        && tree_note.is_none())
    .then(|| crate::window::notebook_view::focused_folder(hwnd))
    .flatten();
    // A focused (or recorded) note or folder lets a note-scoped command through even with no
    // tab open.
    if command.needs_document()
        && tab_count(hwnd) == 0
        && tree_note.is_none()
        && tree_folder.is_none()
    {
        return;
    }
    // An image tab has no text to save, edit, search or relabel (image preview spec §5).
    if command.needs_text()
        && tree_note.is_none()
        && crate::window::image_host::active_is_image(hwnd)
    {
        return;
    }
    // Sidebar commands do nothing with notes mode off: there is no sidebar to act on (spec §5).
    if command.is_sidebar() && !notes_mode_enabled(hwnd) {
        return;
    }
    #[cfg(test)]
    if command.is_sidebar() {
        SIDEBAR_COMMAND_RUNS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    if let Some(index) = command.tab_index() {
        if index < tab_count(hwnd) {
            activate_tab(hwnd, index);
        }
        return;
    }
    if let Some(index) = command.group_index() {
        focus_group_number(hwnd, index);
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
        CommandId::New => crate::window::library_host::new_note_in(hwnd),
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
        CommandId::FindNext => find_again(hwnd, false),
        CommandId::FindPrevious => find_again(hwnd, true),
        CommandId::CommandPalette => open_command_palette(hwnd),
        CommandId::About => show_about(hwnd),
        CommandId::OpenSettings => show_settings(hwnd),
        CommandId::EditSettingsFile => edit_settings_file(hwnd),
        CommandId::OpenKeyboardShortcuts => show_keyboard_shortcuts(hwnd),
        CommandId::QuickOpen => open_quick_open(hwnd),
        CommandId::NoteNewFolder => crate::window::inline_name::new_folder(hwnd, None),
        CommandId::NoteNew => crate::window::inline_name::new_note(hwnd, None),
        CommandId::ThemeSystem => set_theme(hwnd, crate::config::ThemePreference::System),
        CommandId::ThemeLight => set_theme(hwnd, crate::config::ThemePreference::Light),
        CommandId::FileIconsMaterial => set_file_icons(hwnd, crate::config::FileIconSet::Material),
        CommandId::FileIconsMinimal => set_file_icons(hwnd, crate::config::FileIconSet::Minimal),
        CommandId::FileIconsSolid => set_file_icons(hwnd, crate::config::FileIconSet::Solid),
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
        CommandId::ToggleInsertSpaces => change_setting(hwnd, |settings| {
            settings.insert_spaces = !settings.insert_spaces;
            Some(("insert_spaces", settings.insert_spaces.to_string()))
        }),
        CommandId::ToggleShowWhitespace => change_setting(hwnd, |settings| {
            settings.show_whitespace = !settings.show_whitespace;
            Some(("show_whitespace", settings.show_whitespace.to_string()))
        }),
        CommandId::ToggleHighlightCurrentLine => change_setting(hwnd, |settings| {
            settings.highlight_current_line = !settings.highlight_current_line;
            Some((
                "highlight_current_line",
                settings.highlight_current_line.to_string(),
            ))
        }),
        CommandId::ToggleAlwaysOnTop => {
            change_setting(hwnd, |settings| {
                settings.always_on_top = !settings.always_on_top;
                Some(("always_on_top", settings.always_on_top.to_string()))
            });
            apply_always_on_top(hwnd);
        }
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
                match (&tree_note, &tree_folder) {
                    (Some(path), _) => crate::window::inline_name::rename_note_at(hwnd, path),
                    (None, Some(folder)) => crate::window::inline_name::rename(
                        hwnd,
                        &crate::library::tree::RowKind::Folder(folder.clone()),
                    ),
                    (None, None) => crate::window::inline_name::rename_active(hwnd),
                }
            }
        }
        CommandId::NoteDelete => {
            if crate::window::library_host::ready_library(hwnd) {
                match (&tree_note, &tree_folder) {
                    (Some(path), _) => crate::window::library_host::delete_file(hwnd, path),
                    (None, Some(folder)) => {
                        crate::window::library_host::delete_folder(hwnd, folder);
                    }
                    (None, None) => crate::window::library_host::delete_note(hwnd),
                }
            }
        }
        CommandId::CloseNotebook => crate::window::library_host::close_notebook(hwnd),
        CommandId::SplitRight => {
            split_active_group(hwnd, crate::window::split_tree::Direction::Right);
        }
        CommandId::MoveTabToNextGroup => move_active_view(hwnd, true),
        CommandId::MoveTabToPreviousGroup => move_active_view(hwnd, false),
        CommandId::SplitDown => {
            split_active_group(hwnd, crate::window::split_tree::Direction::Down);
        }
        CommandId::CloseGroup => {
            if let Some(group) =
                unsafe { app_ptr(hwnd) }.map(|app| unsafe { app.as_ref() }.tabs.active_group())
            {
                close_group(hwnd, group);
            }
        }
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
        CommandId::LanguagePlainText
        | CommandId::LanguageJson
        | CommandId::LanguageMarkdown
        | CommandId::LanguageBash
        | CommandId::LanguageBatch
        | CommandId::LanguageC
        | CommandId::LanguageCSharp
        | CommandId::LanguageCpp
        | CommandId::LanguageCss
        | CommandId::LanguageEnv
        | CommandId::LanguageHtml
        | CommandId::LanguageIni
        | CommandId::LanguageJavaScript
        | CommandId::LanguagePowerShell
        | CommandId::LanguageProperties
        | CommandId::LanguagePython
        | CommandId::LanguageRust
        | CommandId::LanguageSql
        | CommandId::LanguageSvg
        | CommandId::LanguageToml
        | CommandId::LanguageTypeScript
        | CommandId::LanguageXml
        | CommandId::LanguageYaml => {
            if let Some(language) = command.language() {
                apply_language(hwnd, language);
            }
        }
        CommandId::ValidateJson => validate_active_json(hwnd),
        CommandId::FormatJson => format_active_json(hwnd),
        CommandId::NextTab => cycle_tab(hwnd, true),
        CommandId::PreviousTab => cycle_tab(hwnd, false),
        CommandId::ZoomIn | CommandId::ZoomOut | CommandId::ZoomReset
            // An image tab never zooms the hidden editor, even with no image view to zoom.
            if crate::window::image_host::zoom(hwnd, command)
                || crate::window::image_host::active_is_image(hwnd)
                || crate::window::preview_host::zoom_svg(hwnd, command) => {}
        // One zoom for every group (split editors plan amendment 11).
        CommandId::ZoomIn => {
            for editor in all_editors(hwnd) {
                let _ = editor.zoom_in();
            }
        }
        CommandId::ZoomOut => {
            for editor in all_editors(hwnd) {
                let _ = editor.zoom_out();
            }
        }
        CommandId::ZoomReset => {
            for editor in all_editors(hwnd) {
                let _ = editor.reset_zoom();
            }
        }
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
        CommandId::ShowSearchView => show_search_view(hwnd),
        CommandId::ReplaceInNotes => crate::window::search_view::show_replace(hwnd),
        CommandId::SearchToggleCase
        | CommandId::SearchToggleWholeWord
        | CommandId::SearchToggleRegex => {
            if let Some(option) = command.search_option() {
                toggle_search_option(hwnd, option);
            }
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

/// Switches the Notebook tree's icon set and repaints the tree (icon sets spec §4). No rescan.
fn set_file_icons(hwnd: HWND, set: crate::config::FileIconSet) {
    change_setting(hwnd, |settings| {
        (settings.file_icons != set).then(|| {
            settings.file_icons = set;
            ("file_icons", set.token().to_owned())
        })
    });
    if let Some((_, panel)) = crate::window::side_panel::windows(hwnd) {
        unsafe { InvalidateRect(panel, std::ptr::null(), 0) };
    }
}

/// Makes one Settings dialog change through the same code the palette commands use, so it
/// applies at once and saves its one `fastpad.ini` line (settings dialog spec §4.2).
pub(crate) fn apply_settings_action(
    hwnd: HWND,
    action: crate::window::settings_model::SettingsAction,
) {
    use crate::window::settings_model::SettingsAction;
    match action {
        SettingsAction::SetTheme(theme) => set_theme(hwnd, theme),
        SettingsAction::SetFileIcons(set) => set_file_icons(hwnd, set),
        SettingsAction::SetFontFace(face) => change_setting(hwnd, |settings| {
            (settings.font_face != face).then(|| {
                settings.font_face.clone_from(&face);
                ("font_face", face)
            })
        }),
        SettingsAction::SetFontSize(size) => set_font_size(hwnd, |_| size),
        SettingsAction::SetTabWidth(width) => set_tab_width(hwnd, width),
        SettingsAction::Toggle(toggle) => execute_command(hwnd, toggle.command()),
    }
}

/// What the Settings dialog shows. Call it with nothing of the App borrowed.
pub(crate) fn settings_view(hwnd: HWND) -> crate::window::settings_model::SettingsView {
    let settings = unsafe { app_ptr(hwnd) }.map_or_else(crate::config::default_settings, |app| {
        unsafe { app.as_ref() }.settings.clone()
    });
    crate::window::settings_model::SettingsView {
        settings,
        notebook_autosave: crate::window::library_host::notebook_autosave(hwnd),
    }
}

/// Whether the Notebook view's Open Editors section is expanded (open editors spec §3.3).
pub(crate) fn open_editors_expanded(hwnd: HWND) -> bool {
    unsafe { app_ptr(hwnd) }
        .is_none_or(|app| unsafe { app.as_ref() }.settings.open_editors_expanded)
}

/// Collapses or expands the Open Editors section and saves it. Only the panel repaints.
pub(crate) fn set_open_editors_expanded(hwnd: HWND, expanded: bool) {
    change_setting(hwnd, |settings| {
        (settings.open_editors_expanded != expanded).then(|| {
            settings.open_editors_expanded = expanded;
            ("open_editors_expanded", expanded.to_string())
        })
    });
    if let Some((_, panel)) = crate::window::side_panel::windows(hwnd) {
        unsafe { InvalidateRect(panel, std::ptr::null(), 0) };
    }
}

/// The shortcuts in force for `hwnd`'s window (the defaults before it has an App).
pub(crate) fn keymap(hwnd: HWND) -> crate::window::keymap::Keymap {
    unsafe { app_ptr(hwnd) }.map_or_else(crate::window::keymap::Keymap::defaults, |app| {
        unsafe { app.as_ref() }.keymap.clone()
    })
}

/// The text menus show for `command`'s key in `hwnd`'s keymap, without copying the keymap.
pub(crate) fn first_key_text(hwnd: HWND, command: CommandId) -> Option<String> {
    match unsafe { app_ptr(hwnd) } {
        Some(app) => unsafe { app.as_ref() }.keymap.first_text(command),
        None => crate::window::keymap::Keymap::defaults().first_text(command),
    }
}

/// The commands with a `key.<id>=` line in `hwnd`'s settings, including lines the keymap ignored.
pub(crate) fn key_line_commands(hwnd: HWND) -> Vec<CommandId> {
    unsafe { app_ptr(hwnd) }.map_or_else(Vec::new, |app| {
        unsafe { app.as_ref() }
            .settings
            .key_overrides
            .keys()
            .filter_map(|id| crate::window::keymap::command_for_id(id))
            .collect()
    })
}

/// Puts `keymap` in force: the accelerator table is rebuilt now; the menu bar, which spells the
/// keys, is rebuilt the next time it opens.
fn install_keymap(app: &mut App, keymap: crate::window::keymap::Keymap) {
    app.accelerators = crate::window::menus::AcceleratorTable::create(&keymap).ok();
    if app.menu_mode.is_none() {
        app.menu_bar = None;
    }
    app.keymap = keymap;
}

/// Gives `command` exactly `keys` (keyboard shortcuts spec 6.6): applies at once and saves its
/// `key.<id>=` line, or removes the line when `keys` are the defaults.
pub(crate) fn set_command_keys(
    hwnd: HWND,
    command: CommandId,
    keys: Vec<crate::window::keymap::KeyStroke>,
) {
    use crate::window::keymap::{ini_key, ini_value};
    let Some(key) = ini_key(command) else {
        return;
    };
    let id = key["key.".len()..].to_owned();
    // The App borrow ends before saving: a failed save pushes a notice, which borrows it again.
    let Some(saved) = unsafe { app_ptr(hwnd) }.and_then(|mut app| {
        let app = unsafe { app.as_mut() };
        let keymap = app.keymap.with_keys(command, keys);
        // A line the keymap ignored (`key.file.save=Bogus`) is still in the settings and the
        // file; it is stale, and resetting the command removes it.
        let stale = !keymap.is_user(command) && app.settings.key_overrides.contains_key(&id);
        if keymap == app.keymap && !stale {
            return None;
        }
        let value = keymap
            .is_user(command)
            .then(|| ini_value(&keymap.keys_of(command)));
        match &value {
            Some(value) => app.settings.key_overrides.insert(id, value.clone()),
            None => app.settings.key_overrides.remove(&id),
        };
        install_keymap(app, keymap);
        Some(value)
    }) else {
        return;
    };
    let result = match saved {
        Some(value) => save_setting(&key, &value),
        None => remove_setting(&key),
    };
    if let Err(error) = result {
        push_notice(hwnd, format!("FastPad could not save fastpad.ini: {error}"));
    }
}

/// Gives `command` its default keys back and removes its `key.<id>=` line.
pub(crate) fn reset_command_keys(hwnd: HWND, command: CommandId) {
    set_command_keys(hwnd, command, crate::window::keymap::default_keys(command));
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
    // The sidebar's view, width and icon set change only the sidebar, which their callers redo,
    // and the Settings dialog's size only that dialog; the editor and the Markdown preview are
    // not restyled for them.
    let sidebar_only = matches!(
        key,
        "sidebar_view"
            | "sidebar_width"
            | "file_icons"
            | "open_editors_expanded"
            | "settings_size"
            | "always_on_top"
    );
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
fn remove_setting(key: &str) -> Result<()> {
    crate::config::remove_setting(key)
}

/// Like `save_setting` in tests: only a path a test chose with `save_settings_to` is touched.
#[cfg(test)]
fn remove_setting(key: &str) -> Result<()> {
    TEST_SETTINGS_PATH.with(|path| match path.borrow().as_deref() {
        Some(path) => crate::config::remove_setting_to(path, key),
        None => Ok(()),
    })
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
    if crate::window::image_host::active_is_image(hwnd) {
        return;
    }
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
        (unsafe { app_ptr(hwnd) }).and_then(|app| unsafe { app.as_ref() }.editor().cloned())
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
        Some(Err(_)) => {
            // An SVG's preview does not need Lexilla, so the tab stays an SVG in plain text.
            if language == crate::document::Language::Svg {
                if let Some(mut app) = unsafe { app_ptr(hwnd) } {
                    unsafe { app.as_mut() }.tabs.set_active_language(language);
                }
                invalidate_status_bar(hwnd);
                crate::window::preview_host::sync_visibility(hwnd);
            }
            push_notice(
                hwnd,
                "FastPad could not enable syntax highlighting for this file. It will remain in \
                 plain text."
                    .to_owned(),
            );
        }
        None => {}
    }
}

/// The active tab's language; plain text while no tab is open.
fn active_language(hwnd: HWND) -> crate::document::Language {
    unsafe { app_ptr(hwnd) }
        .and_then(|app| {
            unsafe { app.as_ref() }
                .tabs
                .active()
                .map(|document| document.language)
        })
        .unwrap_or(crate::document::Language::PlainText)
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
        let (keymap, problems) =
            crate::window::keymap::Keymap::from_ini(&app.settings.key_overrides);
        for problem in problems {
            app.notifications.push(format!("fastpad.ini: {problem}"));
        }
        if keymap != app.keymap {
            install_keymap(app, keymap);
        }
    }
    apply_editor_settings(hwnd);
    apply_always_on_top(hwnd);
    if sidebar_changed {
        let notes_mode = notes_mode_enabled(hwnd);
        crate::window::side_panel::notes_mode_changed(hwnd, notes_mode);
    }
}

/// Puts the window above every non-topmost window, or back among them, to match the
/// `always_on_top` setting. Neither move nor resize nor activate: only the z-order changes.
fn apply_always_on_top(hwnd: HWND) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{HWND_NOTOPMOST, HWND_TOPMOST};
    let Some(app) = (unsafe { app_ptr(hwnd) }) else {
        return;
    };
    let insert_after = if unsafe { app.as_ref() }.settings.always_on_top {
        HWND_TOPMOST
    } else {
        HWND_NOTOPMOST
    };
    unsafe {
        SetWindowPos(
            hwnd,
            insert_after,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
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
    let Some((settings, palette)) = (unsafe { app_ptr(hwnd) }).map(|app| {
        let app = unsafe { app.as_ref() };
        let palette = Palette::for_cached_theme(app.theme, app.settings.theme);
        (app.settings.clone(), palette)
    }) else {
        return;
    };
    for editor in all_editors(hwnd) {
        apply_settings_to(&editor, &settings, palette);
    }
    crate::window::preview_host::refresh_appearance(hwnd);
    crate::window::image_host::refresh_appearance(hwnd);
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
    load_and_show_logo(hwnd);
    unsafe {
        windows_sys::Win32::UI::Shell::DragAcceptFiles(hwnd, 1);
        InvalidateRect(hwnd, std::ptr::null(), 1);
    }
    crate::window::library_host::accept_editor_file_drops(hwnd);
}

/// Loads the activity bar's logo icon at the window's current DPI (the deferred chrome step, the
/// first time the icon is loaded at all: nothing before first paint touches it) and invalidates
/// only its rect on the bar, if the sidebar exists yet.
fn load_and_show_logo(hwnd: HWND) {
    let dpi = unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForWindow(hwnd) }.max(96);
    ensure_logo_icon(hwnd, dpi);
    let bar = unsafe { app_ptr(hwnd) }.and_then(|app| {
        unsafe { app.as_ref() }
            .sidebar
            .as_ref()
            .map(|sidebar| sidebar.bar)
    });
    if let Some(bar) = bar {
        let mut client = RECT::default();
        unsafe { GetClientRect(bar, &mut client) };
        let rect = crate::window::activity_bar::logo_rect(client, dpi);
        unsafe { InvalidateRect(bar, &rect, 1) };
    }
}

/// Loads the logo icon for `dpi` unless it is already loaded at that DPI, replacing (and, by
/// dropping it, destroying) any icon loaded at a different one. Call only from `build_chrome`
/// (after first paint) and the `WM_DPICHANGED` handler; a paint must never trigger a load.
fn ensure_logo_icon(hwnd: HWND, dpi: u32) {
    if let Some(mut app) = unsafe { app_ptr(hwnd) } {
        let app = unsafe { app.as_mut() };
        if app.logo_icon.as_ref().is_some_and(|logo| logo.dpi() == dpi) {
            return;
        }
        app.logo_icon = load_logo_icon(dpi).map(|icon| LogoIcon::new(dpi, icon));
    }
}

/// The logo icon loaded for `dpi`, or `None` before `build_chrome` has run, or momentarily while a
/// different DPI's icon hasn't been reloaded yet. Never loads; call it with nothing of the App
/// borrowed, from `activity_bar::paint`.
pub(crate) fn logo_icon(hwnd: HWND, dpi: u32) -> Option<HICON> {
    unsafe { app_ptr(hwnd) }.and_then(|app| {
        unsafe { app.as_ref() }
            .logo_icon
            .as_ref()
            .filter(|logo| logo.dpi() == dpi)
            .map(LogoIcon::icon)
    })
}

/// Loads the app's icon resource (`APP_ICON_RESOURCE_ID`, embedded by `build.rs`) at `dpi`'s pixel
/// size. No file I/O: it is already resident in the module. `None` if the resource is missing
/// (e.g. a test binary built without it) or the load otherwise fails.
fn load_logo_icon(dpi: u32) -> Option<HICON> {
    let px = crate::window::panel::scale(20, dpi);
    let instance = unsafe { GetModuleHandleW(std::ptr::null()) };
    let handle = unsafe {
        LoadImageW(
            instance,
            APP_ICON_RESOURCE_ID as *const u16,
            IMAGE_ICON,
            px,
            px,
            LR_DEFAULTCOLOR,
        )
    };
    (!handle.is_null()).then_some(handle)
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
            let editor = app.editor().cloned()?;
            let palette = Palette::for_cached_theme(app.theme, app.settings.theme);
            let frame_change = app.dark_frame_applied != palette.dark_frame;
            app.dark_frame_applied = palette.dark_frame;
            for group in &mut app.groups {
                if let Some(bar) = group.find_bar.as_mut() {
                    bar.set_colors(palette);
                }
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
    let highlight_current_line = unsafe { app_ptr(hwnd) }
        .is_none_or(|app| unsafe { app.as_ref() }.settings.highlight_current_line);
    for editor in all_editors(hwnd) {
        apply_colors_to(&editor, palette, highlight_current_line);
    }
    if let Some(app) = unsafe { app_ptr(hwnd) } {
        let app = unsafe { app.as_ref() };
        for bar in app
            .groups
            .iter()
            .filter_map(|group| group.find_bar.as_ref())
        {
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
    let others: Vec<GroupId> = unsafe { app_ptr(hwnd) }
        .map(|app| {
            let app = unsafe { app.as_ref() };
            let active = app.tabs.active_group();
            app.groups
                .iter()
                .map(|group| group.id)
                .filter(|id| *id != active)
                .collect()
        })
        .unwrap_or_default();
    for id in others {
        style_group_view(hwnd, id);
    }
    crate::window::preview_host::refresh_appearance(hwnd);
    crate::window::image_host::refresh_appearance(hwnd);
    crate::window::side_panel::refresh(hwnd);
}

/// Copies what a title-strip paint needs out of App, creating the per-DPI fonts on first use.
/// Before chrome is built the palette is the neutral compiled one (no theme queries).
pub(crate) fn title_chrome(hwnd: HWND) -> (Palette, TitleFontHandles, PointerState) {
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
    if let Some(image) = crate::window::image_host::status(hwnd) {
        let app = unsafe { app_ptr(hwnd) }?;
        let app = unsafe { app.as_ref() };
        app.status.as_ref()?;
        return Some(crate::window::status::image_status_bar_text(
            &app.notifications,
            &image,
        ));
    }
    let app = unsafe { app_ptr(hwnd) }?;
    let app = unsafe { app.as_ref() };
    app.status.as_ref()?;
    let active = app.tabs.active().and_then(|document| {
        Some(crate::window::status::ActiveDocumentStatus {
            caret: app.editor()?.caret_status().ok()?,
            language: document.language,
            encoding: document.encoding,
        })
    });
    let mut bar = crate::window::status::status_bar_text(&app.notifications, active);
    if app.notifications.pending().is_empty()
        && let Some(hint) = app
            .active_group()
            .and_then(|group| group.preview.status_hint())
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
        (unsafe { app_ptr(hwnd) }).and_then(|app| unsafe { app.as_ref() }.editor().cloned())
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
        (unsafe { app_ptr(hwnd) }).and_then(|app| unsafe { app.as_ref() }.editor().cloned())
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

/// A launch with no file leaves no tab: the empty untitled tab the window started with closes
/// once the session restore has had its turn, unless it is not alone or has text by now.
fn close_unused_startup_tab(hwnd: HWND) {
    let alone = unsafe { app_ptr(hwnd) }.is_some_and(|app| unsafe { app.as_ref() }.tabs.len() == 1);
    if alone && empty_startup_tab(hwnd).is_some() {
        close_active_document(hwnd);
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
        crate::launch::LaunchRequest::New => {
            close_unused_startup_tab(hwnd);
            unsafe {
                let _ = record_milestone(hwnd, Milestone::FileLoaded);
            }
        }
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
    let existing =
        unsafe { app_ptr(hwnd) }.and_then(|app| unsafe { app.as_ref() }.tabs.find_path(path));
    if let Some(id) = existing {
        return if open_in_active_group(hwnd, id) {
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
    if crate::library::title::is_raster_image_path(path) {
        return open_image_placed(hwnd, path, preview);
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
    // A file that is not text but starts with an image signature opens in an image tab (image
    // preview spec §4).
    let loaded = match crate::file::loader::load(path) {
        Err(crate::FastPadError::UnsupportedEncoding)
            if crate::file::sniff::file_looks_like_image(path) =>
        {
            return open_image_placed(hwnd, path, preview);
        }
        loaded => loaded?,
    };
    // A NUL byte cannot round-trip through Scintilla's UTF-8 buffer: the file is unsupported.
    if std::ffi::CString::new(loaded.text.as_str()).is_err() {
        return if crate::file::sniff::file_looks_like_image(path) {
            open_image_placed(hwnd, path, preview)
        } else {
            Err(crate::FastPadError::UnsupportedEncoding)
        };
    }
    let (editor, candidate_ids, replace_preview) = {
        let mut app = unsafe { app_ptr(hwnd) }.ok_or(crate::FastPadError::Invariant(
            "main window app state was not available",
        ))?;
        let app = unsafe { app.as_mut() };
        let editor = app
            .editor()
            .cloned()
            .ok_or(crate::FastPadError::Invariant("editor was not initialized"))?;
        // A preview goes where the preview tab is. Without one it is placed like any new tab,
        // reusing an empty start tab.
        let replace_preview = preview && app.tabs.preview_id().is_some();
        // An untitled tab another group also shows stays: that view keeps it.
        let candidate_ids = app
            .tabs
            .active()
            .filter(|active| !replace_preview && !active.dirty && active.path.is_none())
            .filter(|active| app.tabs.views_of(active.id).len() == 1)
            .map(|active| (active.id, active.recovery_id));
        (editor, candidate_ids, replace_preview)
    };
    remember_active_view(hwnd);
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
    let result = document
        .expect_text()
        .and_then(|handle| editor.use_document(handle))
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

/// Opens `path` in an image tab (image preview spec §5). Like a text open it reuses an empty start
/// tab or the preview tab, but reads no bytes: the image view decodes on a worker.
fn open_image_placed(hwnd: HWND, path: &std::path::Path, preview: bool) -> Result<()> {
    let identity = unsafe { window_identity(hwnd) }.ok_or(crate::FastPadError::Invariant(
        "main window app state was not available",
    ))?;
    if !path.is_file() {
        return Err(crate::FastPadError::Io(std::io::Error::from(
            std::io::ErrorKind::NotFound,
        )));
    }
    crate::window::library_host::autosave_active(hwnd);
    if !identity.is_live_for(hwnd) {
        return Err(crate::FastPadError::Invariant(
            "main window was destroyed during file open",
        ));
    }
    let stamp = crate::library::disk_stamp(path);
    let (editor, candidate_ids, replace_preview) = {
        let mut app = unsafe { app_ptr(hwnd) }.ok_or(crate::FastPadError::Invariant(
            "main window app state was not available",
        ))?;
        let app = unsafe { app.as_mut() };
        let editor = app
            .editor()
            .cloned()
            .ok_or(crate::FastPadError::Invariant("editor was not initialized"))?;
        let replace_preview = preview && app.tabs.preview_id().is_some();
        let candidate_ids = app
            .tabs
            .active()
            .filter(|active| {
                !replace_preview && !active.is_image() && !active.dirty && active.path.is_none()
            })
            .map(|active| (active.id, active.recovery_id));
        (editor, candidate_ids, replace_preview)
    };
    remember_active_view(hwnd);
    let reused_ids = match candidate_ids {
        Some(ids) if editor.text()?.is_empty() => Some(ids),
        _ => None,
    };
    let reuse = reused_ids.is_some();
    let (id, recovery_id) = match reused_ids {
        Some(ids) => ids,
        None => {
            let mut app = unsafe { app_ptr(hwnd) }.ok_or(crate::FastPadError::Invariant(
                "main window app state was not available",
            ))?;
            unsafe { app.as_mut() }.allocate_document_identity()
        }
    };
    let mut document = Document::image(id, recovery_id, path.to_path_buf());
    document.preview = preview;
    document.disk_stamp = stamp;
    let (commit, retired) = {
        let mut app = unsafe { app_ptr(hwnd) }.ok_or(crate::FastPadError::Invariant(
            "main window app state was not available",
        ))?;
        let app = unsafe { app.as_mut() };
        if replace_preview {
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
    commit?;
    // The retired tab's text document leaves the editor for an empty placeholder.
    let blank = editor.create_document()?;
    editor.use_document(&blank)?;
    drop(retired);
    unsafe {
        let _ = record_milestone(hwnd, Milestone::FileLoaded);
    }
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
            if !open_in_active_group(hwnd, id) {
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

/// Opens a Search result (spec §8). The note opens as `open_note` opens it. The find bar then
/// opens in Find mode with the query and options the shown results ran with, and selects the
/// first match from the start of the note. The find bar searches the live text: a phrase gone
/// since the search leaves the note open and the bar in its no-match state. `focus_editor` then
/// moves the focus to the editor, so F3 and Shift+F3 step on from the selected match. A note
/// that can't be opened (moved or deleted since the search) gets a notice, and the search runs
/// again.
pub(crate) fn open_search_result(
    hwnd: HWND,
    relative: &std::path::Path,
    mode: OpenMode,
    focus_editor: bool,
) {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    let Some(folder) = crate::window::library_host::folder(hwnd) else {
        return;
    };
    let path = folder.join(relative);
    // The results' query, not the box's text, which may be newer while the debounce runs.
    let search = crate::window::search_view::run_query(hwnd);
    if let Err(error) = open_note(hwnd, &path, mode, false) {
        push_notice(
            hwnd,
            format!("FastPad could not open {}: {error}", path.display()),
        );
        crate::window::text_search_host::run_now(hwnd);
        return;
    }
    if !identity.is_live_for(hwnd) {
        return;
    }
    if let Some((query, options)) = search.filter(|(query, _)| !query.is_empty()) {
        seed_find_bar(hwnd, &identity, &query, options);
    }
    if focus_editor && identity.is_live_for(hwnd) {
        focus_content(hwnd);
    }
}

/// Shows the find bar with `query` and `options` and selects the first match from position 0.
fn seed_find_bar(
    hwnd: HWND,
    identity: &WindowIdentity,
    query: &str,
    options: crate::search::MatchOptions,
) {
    crate::window::library_host::close_name_box(hwnd);
    if !identity.is_live_for(hwnd) || !ensure_find_bar(hwnd) {
        return;
    }
    let colors = title_chrome(hwnd).0;
    let pending = unsafe { app_ptr(hwnd) }.and_then(|mut app| {
        let bar = unsafe { app.as_mut() }.find_bar_mut()?;
        Some(bar.show_with(find_bar::FindBarMode::Find, query, options, colors))
    });
    let Some(pending) = pending else {
        return;
    };
    // Applied with nothing borrowed: the field's EN_CHANGE borrows the bar again.
    pending.apply();
    if !identity.is_live_for(hwnd) {
        return;
    }
    layout_editor_and_find_bar(hwnd);
    let Some(editor) =
        unsafe { app_ptr(hwnd) }.and_then(|app| unsafe { app.as_ref() }.editor().cloned())
    else {
        return;
    };
    let _ = editor.set_selection(0..0);
    select_match(
        hwnd,
        identity,
        &editor,
        query,
        options,
        0,
        find_bar::SearchDirection::Forward,
    );
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
    let Some(id) = tab_id_at(hwnd, index) else {
        return false;
    };
    with_group(hwnd, |group| {
        let double = group
            .last_tab_click
            .is_some_and(|(last, at)| last == id && now.wrapping_sub(at) <= limit);
        group.last_tab_click = if double { None } else { Some((id, now)) };
        double
    })
    .unwrap_or(false)
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
            .editor()
            .cloned()
            .ok_or(crate::FastPadError::Invariant("editor was not initialized"))?;
        let (id, recovery_id) = app.allocate_document_identity();
        (editor, id, recovery_id)
    };

    remember_active_view(hwnd);
    let document = Document::untitled(id, recovery_id, editor.create_document()?);
    document
        .expect_text()
        .and_then(|handle| editor.use_document(handle))?;
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
    let Some(group) =
        unsafe { app_ptr(hwnd) }.map(|app| unsafe { app.as_ref() }.tabs.active_group())
    else {
        return false;
    };
    activate_document_in(hwnd, group, id, revision)
}

/// Shows `id`'s view in `group`, provided `group`'s strip is still at `revision`.
pub(crate) fn activate_document_in(
    hwnd: HWND,
    group: GroupId,
    id: DocumentId,
    revision: u64,
) -> bool {
    if file_population_active(hwnd) {
        return false;
    }
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return false;
    };
    let leaving = unsafe { app_ptr(hwnd) }.and_then(|app| {
        let app = unsafe { app.as_ref() };
        let tabs = app.tabs.group(group)?;
        (tabs.view().snapshot().revision == revision).then(|| {
            (
                tabs.active_document() != Some(id),
                app.tabs.active_group() == group,
            )
        })
    });
    let Some((leaving, active)) = leaving else {
        return false;
    };
    // Saving the tab being left bumps the view revision itself, so the caller's revision is
    // checked before it; afterwards `activate_in` still refuses an `id` that has gone.
    if leaving && active {
        crate::window::library_host::autosave_active(hwnd);
        if !identity.is_live_for(hwnd) {
            return false;
        }
    }
    if leaving {
        remember_view(hwnd, group);
    }
    let activated = unsafe { app_ptr(hwnd) }
        .is_some_and(|mut app| unsafe { app.as_mut() }.tabs.activate_in(group, id));
    if !activated || !show_group_view(hwnd, group) || !identity.is_live_for(hwnd) {
        return false;
    }
    if active {
        refresh_tabs(hwnd);
        crate::window::image_host::check_disk(hwnd);
    } else if let Some(window) = with_group_id(hwnd, group, |state| state.hwnd) {
        unsafe { InvalidateRect(window, std::ptr::null(), 0) };
        crate::window::notebook_view::editors_changed(hwnd);
    }
    true
}

/// `remember_view` for the active group.
fn remember_active_view(hwnd: HWND) {
    if let Some(group) =
        unsafe { app_ptr(hwnd) }.map(|app| unsafe { app.as_ref() }.tabs.active_group())
    {
        remember_view(hwnd, group);
    }
}

/// Selects a tab for the strip provider of the group window `wparam`.
fn handle_accessible_select(hwnd: HWND, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if lparam == 0 {
        return 0;
    }
    let request = unsafe { *(lparam as *const AccessibleSelectRequest) };
    let Some(group) = group_id_of(hwnd, wparam as HWND) else {
        return 0;
    };
    isize::from(activate_document_in(
        hwnd,
        group,
        request.document_id,
        request.revision,
    ))
}

fn close_active_document(hwnd: HWND) {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    // Another group still shows the document: only this view closes, without asking (split
    // editors spec §5.4). Its text and dirty state stay with the other view.
    let shared = unsafe { app_ptr(hwnd) }.and_then(|app| {
        let app = unsafe { app.as_ref() };
        let review = app.tabs.active_close_review()?;
        (app.tabs.views_of(review.id).len() > 1).then(|| (review, app.editor().cloned()))
    });
    if let Some((review, Some(editor))) = shared {
        close_reviewed_document(hwnd, &identity, &editor, review, CloseDecision::Discard);
        return;
    }
    // A saved note is clean now and closes without a prompt; paused or failed ones still ask.
    crate::window::library_host::autosave_active(hwnd);
    if !identity.is_live_for(hwnd) {
        return;
    }
    let snapshot = unsafe { app_ptr(hwnd) }.and_then(|app| {
        let app = unsafe { app.as_ref() };
        let review = app.tabs.active_close_review()?;
        let document = app.tabs.document(review.id)?;
        Some((
            review,
            document.dirty,
            document.title(),
            app.editor().cloned(),
        ))
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

/// Closes tab `id` as a middle-click on it does (open editors spec §3.2), if it is still open.
pub(crate) fn close_document_tab(hwnd: HWND, id: DocumentId) {
    let index = unsafe { app_ptr(hwnd) }.and_then(|app| {
        let tabs = &unsafe { app.as_ref() }.tabs;
        tabs.group_documents(tabs.active_group())
            .into_iter()
            .position(|document| document.id == id)
    });
    if let Some(index) = index {
        close_tab_at(hwnd, index);
    }
}

/// Closes the tab at strip `index` (quick-open spec §5). A clean tab that isn't the active one
/// closes where it is, and the active tab stays. Any other tab is activated first, so a save
/// prompt asks about the tab on screen, and is then closed as Close tab closes it.
fn close_tab_at(hwnd: HWND, index: usize) {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    let target = unsafe { app_ptr(hwnd) }.and_then(|app| {
        let tabs = &unsafe { app.as_ref() }.tabs;
        let document = *tabs.group_documents(tabs.active_group()).get(index)?;
        let background = tabs.active().is_some_and(|active| active.id != document.id);
        Some((
            crate::window::tabs::CloseReview {
                id: document.id,
                generation: document.generation,
            },
            background && !document.dirty,
        ))
    });
    let Some((review, clean_background)) = target else {
        return;
    };
    if clean_background {
        close_background_document(hwnd, &identity, review);
    } else if activate_document_by_id(hwnd, review.id) && identity.is_live_for(hwnd) {
        execute_command(hwnd, CommandId::CloseTab);
    }
    // A middle-click moves no focus, so the palette can stay open in the QuickOpen picker while
    // a tab behind it closes; its empty-query rows (the open tabs) must drop the closed one.
    refresh_quick_open_after_close(hwnd);
}

/// Rebuilds an open quick-open picker's rows after `close_tab_at` closes a tab, so a tab closed
/// behind the palette does not linger in its "open tabs" rows.
fn refresh_quick_open_after_close(hwnd: HWND) {
    let showing_quick_open = with_command_palette(hwnd, |palette| {
        palette.is_visible()
            && palette
                .picker()
                .is_some_and(|picker| picker.kind == command_palette::PickerKind::QuickOpen)
    })
    .unwrap_or(false);
    if showing_quick_open {
        refilter_command_palette(hwnd);
    }
}

/// The document shown by tab `index` of the strip.
fn tab_id_at(hwnd: HWND, index: usize) -> Option<DocumentId> {
    unsafe { app_ptr(hwnd) }.and_then(|app| {
        let tabs = &unsafe { app.as_ref() }.tabs;
        tabs.group_documents(tabs.active_group())
            .get(index)
            .map(|document| document.id)
    })
}

/// Closes `id` without asking, discarding any unsaved edits, e.g. once its file is deleted.
/// Every group's view of `id` closes, each without asking (split editors spec §5.6).
pub(super) fn close_document_without_prompt(hwnd: HWND, id: DocumentId) {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    let previous = unsafe { app_ptr(hwnd) }.map(|app| unsafe { app.as_ref() }.tabs.active_group());
    close_every_view(hwnd, &identity, id);
    // Closing a view elsewhere activated its group; the one the user was in stays active.
    if let Some(previous) = previous
        && identity.is_live_for(hwnd)
        && with_group_id(hwnd, previous, |_| ()).is_some()
    {
        activate_group(hwnd, previous);
    }
}

fn close_every_view(hwnd: HWND, identity: &WindowIdentity, id: DocumentId) {
    let views = || {
        unsafe { app_ptr(hwnd) }.map_or(0, |app| unsafe { app.as_ref() }.tabs.views_of(id).len())
    };
    while let before @ 1.. = views() {
        if !activate_document_by_id(hwnd, id) || !identity.is_live_for(hwnd) {
            return;
        }
        let reviewed = unsafe { app_ptr(hwnd) }.and_then(|app| {
            let app = unsafe { app.as_ref() };
            Some((app.tabs.active_close_review()?, app.editor().cloned()?))
        });
        let Some((review, editor)) = reviewed else {
            return;
        };
        if review.id != id {
            return;
        }
        close_reviewed_document(hwnd, identity, &editor, review, CloseDecision::Discard);
        if !identity.is_live_for(hwnd) || views() >= before {
            return;
        }
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
    let group = unsafe { app_ptr(hwnd) }.map(|app| unsafe { app.as_ref() }.tabs.active_group());
    let switched = unsafe { app_ptr(hwnd) }.and_then(|mut app| {
        let app = unsafe { app.as_mut() };
        // `None` when the document is still shown in another group: it stays open.
        let closed = app.tabs.close_reviewed(review, decision).ok()?;
        let snapshots = app
            .recovery_root
            .as_deref()
            .zip(closed.as_ref())
            .map(|(root, closed)| {
                crate::recovery::snapshots_removed_on_close(
                    root,
                    closed,
                    decision == CloseDecision::Discard,
                )
            })
            .unwrap_or_default();
        let view_state = app
            .tabs
            .active()
            .map(|document| app.tabs.view_state(document.id))
            .unwrap_or_default();
        Some((
            closed,
            app.tabs.active_handle().cloned(),
            view_state,
            snapshots,
        ))
    });
    let Some((closed, active, view_state, snapshots)) = switched else {
        return;
    };
    // The view keeps its own reference to whatever it shows, so the last closed document is
    // swapped for an empty placeholder rather than lingering in the hidden editor.
    match active {
        Some(active) => {
            if editor.use_document(&active).is_ok() {
                let _ = editor.apply_view_state(view_state);
            }
        }
        None => {
            if let Ok(blank) = editor.create_document() {
                let _ = editor.use_document(&blank);
            }
        }
    }
    drop(closed);
    crate::recovery::remove_snapshot_files(&snapshots);
    if identity.is_live_for(hwnd) {
        refresh_tabs(hwnd);
        // A group closes with its last tab, unless it is the only one (spec §5.4).
        if let Some(group) = group {
            remove_empty_group(hwnd, group);
        }
    }
}

/// `close_reviewed_document` for a clean tab that isn't active: it closes where it is, its
/// recovery snapshots go, and the editor keeps showing the active tab (no document swap).
fn close_background_document(
    hwnd: HWND,
    identity: &WindowIdentity,
    review: crate::window::tabs::CloseReview,
) {
    let closed = unsafe { app_ptr(hwnd) }.and_then(|mut app| {
        let app = unsafe { app.as_mut() };
        let closed = app.tabs.close_clean_background(review).ok()?;
        let snapshots = app
            .recovery_root
            .as_deref()
            .zip(closed.as_ref())
            .map(|(root, closed)| crate::recovery::snapshots_removed_on_close(root, closed, true))
            .unwrap_or_default();
        Some((closed, snapshots))
    });
    let Some((closed, snapshots)) = closed else {
        return;
    };
    drop(closed);
    crate::recovery::remove_snapshot_files(&snapshots);
    if identity.is_live_for(hwnd) {
        refresh_tabs(hwnd);
    }
}

/// Closes tabs one at a time, reviewing each dirty one, until none remain or a close is refused.
/// Close all tabs closes the active group's tabs (split editors plan amendment 10). The group goes
/// with its last tab, which ends the loop, unless it is the only one.
fn close_all_documents(hwnd: HWND) {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return;
    };
    let active_group =
        || unsafe { app_ptr(hwnd) }.map(|app| unsafe { app.as_ref() }.tabs.active_group());
    let Some(group) = active_group() else {
        return;
    };
    loop {
        let before = tab_count(hwnd);
        if before == 0 || active_group() != Some(group) {
            return;
        }
        close_active_document(hwnd);
        if !identity.is_live_for(hwnd)
            || (active_group() == Some(group) && tab_count(hwnd) >= before)
        {
            return;
        }
    }
}

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
/// Shows open document `id` in the active group: its view there, or a new view when only another
/// group has one (split editors spec §5.3). A second view makes a preview tab normal.
fn open_in_active_group(hwnd: HWND, id: DocumentId) -> bool {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return false;
    };
    let target = unsafe { app_ptr(hwnd) }.and_then(|app| {
        let tabs = &unsafe { app.as_ref() }.tabs;
        tabs.document(id)?;
        let group = tabs.active_group();
        let state = tabs.group(group)?;
        Some((group, state.contains(id), state.view().snapshot().revision))
    });
    let Some((group, here, revision)) = target else {
        return false;
    };
    if here {
        return activate_document(hwnd, id, revision);
    }
    // The tab being left saves first, as switching tabs does.
    crate::window::library_host::autosave_active(hwnd);
    if !identity.is_live_for(hwnd) {
        return false;
    }
    remember_view(hwnd, group);
    let added = unsafe { app_ptr(hwnd) }.is_some_and(|mut app| {
        unsafe { app.as_mut() }
            .tabs
            .add_view(group, id, crate::editor::ViewState::default())
    });
    if !added || !show_group_view(hwnd, group) {
        return false;
    }
    refresh_tabs(hwnd);
    crate::window::image_host::check_disk(hwnd);
    true
}

/// Makes `id` the active document, or reports false when it no longer exists: its view in the
/// active group, else its view in the first group in layout order that has one, which becomes
/// the active group.
pub(super) fn activate_document_by_id(hwnd: HWND, id: DocumentId) -> bool {
    let group = unsafe { app_ptr(hwnd) }.and_then(|app| {
        let app = unsafe { app.as_ref() };
        let active = app.tabs.active_group();
        let views = app.tabs.views_of(id);
        if views.contains(&active) {
            return Some(active);
        }
        app.layout
            .leaves()
            .into_iter()
            .chain(views.iter().copied())
            .find(|group| views.contains(group))
    });
    let Some(group) = group else {
        return false;
    };
    activate_group(hwnd, group);
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
    if crate::window::image_host::active_is_image(hwnd) {
        return false;
    }
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
    if crate::window::image_host::active_is_image(hwnd) {
        return false;
    }
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
    // Every save path ends here: an image tab's editor holds an empty placeholder, never its bytes.
    if crate::window::image_host::active_is_image(hwnd) {
        return SaveOutcome::Failed;
    }
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
        let editor = app.editor().cloned()?;
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
        let editor = app.editor().cloned()?;
        let active = app.tabs.active()?;
        let document = crate::recovery::next_snapshot_document(
            app.tabs.documents(),
            app.last_snapshot_attempt,
        )?;
        let origin = document.recovery_origin.as_ref();
        let inactive = if document.id == active.id {
            None
        } else {
            Some(document.text_handle()?.clone())
        };
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
            inactive,
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
        Some(target) => with_background_document(hwnd, target, Editor::text),
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
    /// A background tab's document, read through the document host.
    inactive: Option<crate::editor::EditorDocument>,
}

/// Runs `f` on the document host showing `target`, a document no visible editor shows (split
/// editors spec §3.1). The host is never painted and its notifications reach no window, so the
/// visible editor's view and the notification handler see none of this; callers record edits
/// themselves (`Tabs::note_background_edit`).
/// The editor of a group whose active view shows `id`: the active group's if it does, else the
/// first in layout order.
fn editor_showing(app: &App, id: DocumentId) -> Option<Editor> {
    let active = app.tabs.active_group();
    let showing = groups_showing(app, id);
    let group = showing
        .iter()
        .find(|group| **group == active)
        .or_else(|| showing.first())?;
    app.group(*group).map(|state| state.editor.clone())
}

fn with_background_document<R>(
    hwnd: HWND,
    target: &crate::editor::EditorDocument,
    f: impl FnOnce(&Editor) -> Result<R>,
) -> Result<R> {
    let host = unsafe { app_ptr(hwnd) }
        .and_then(|app| unsafe { app.as_ref() }.document_host.clone())
        .ok_or(crate::FastPadError::Invariant(
            "the document host is missing",
        ))?;
    host.use_document(target)?;
    let result = f(&host);
    // Leave the host on a document of its own, so it never keeps a closed tab's text alive.
    if let Ok(blank) = host.create_document() {
        let _ = host.use_document(&blank);
    }
    result
}

/// The text of tab `id` as the editor has it, for the Search view's overlays. A background tab is
/// read through the document host (`with_background_document`). `None` without an editor or that
/// tab, while a file is being populated, or when Scintilla can't be read. Call it with nothing of
/// the App borrowed.
pub(crate) fn document_text(hwnd: HWND, id: DocumentId) -> Option<String> {
    let (editor, inactive) = unsafe { app_ptr(hwnd) }.and_then(|app| {
        let app = unsafe { app.as_ref() };
        // While a file is populated the editor may show a document that is not the active tab's.
        if app.populating_file {
            return None;
        }
        let editor = app.editor().cloned()?;
        let active = app.tabs.active()?;
        let target = app.tabs.document(id)?;
        if target.id == active.id {
            return Some((editor, None));
        }
        Some((editor, Some(target.text_handle()?.clone())))
    })?;
    match inactive {
        None => editor.text().ok(),
        Some(target) => with_background_document(hwnd, &target, Editor::text).ok(),
    }
}

/// Replaces every match of `matcher` in tab `id`'s live text with `template` (expanded in regex
/// mode), in the editor, as one undo action (note-search spec §12). The tab is not saved. The
/// active tab's edit raises Scintilla's notifications as typing does. A background tab is
/// edited through the document host (`with_background_document`), whose notifications reach no
/// window, and then marked edited by hand (`Tabs::note_background_edit`). Returns how many matches were replaced, or `None`
/// without an editor or that tab, while a file is being populated, or when Scintilla fails. Call
/// it with nothing of the App borrowed.
pub(crate) fn replace_in_document(
    hwnd: HWND,
    id: DocumentId,
    matcher: &crate::search::Matcher,
    template: &str,
) -> Option<usize> {
    let identity = unsafe { window_identity(hwnd) }?;
    let (editor, inactive) = unsafe { app_ptr(hwnd) }.and_then(|app| {
        let app = unsafe { app.as_ref() };
        // While a file is populated the editor may show a document that is not the active tab's.
        if app.populating_file {
            return None;
        }
        let target = app.tabs.document(id)?;
        // A group showing the document edits it, so its notifications record the change once.
        if let Some(editor) = editor_showing(app, id) {
            return Some((editor, None));
        }
        let editor = app.editor().cloned()?;
        Some((editor, Some(target.text_handle()?.clone())))
    })?;
    // Set once Scintilla is asked to change the text: from then on it may have changed, even if
    // a replacement then fails partway.
    let touched = std::cell::Cell::new(false);
    let replace = |editor: &Editor| -> Result<usize> {
        let edits = editor.with_document_text(|text| matcher.replacements(text, template))?;
        if edits.is_empty() {
            return Ok(0);
        }
        touched.set(true);
        editor.replace_ranges_with(&edits)
    };
    match inactive {
        None => replace(&editor).ok(),
        Some(target) => {
            // What the edit replaced, even if a later step fails.
            let done = std::cell::Cell::new(None);
            let result = with_background_document(hwnd, &target, |e| {
                let replaced = replace(e)?;
                done.set(Some(replaced));
                Ok(replaced)
            });
            // The tab's text may have changed (a replacement that failed partway, or a restore
            // that failed after it), so it is marked edited whatever the result.
            if touched.get() && identity.is_live_for(hwnd) {
                let changed = unsafe { app_ptr(hwnd) }
                    .is_some_and(|mut app| unsafe { app.as_mut() }.tabs.note_background_edit(id));
                if changed {
                    invalidate_title_strip(hwnd);
                }
            }
            done.get().or(result.ok())
        }
    }
}

/// A tab's text generation and disk stamp, taken when a reload of it begins.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TabMark {
    pub generation: u64,
    pub disk_stamp: Option<crate::library::DiskStamp>,
}

/// Shows `loaded` (read on a worker) in tab `id`, as a file open populates a tab: no undo
/// history and no notifications, the tab left clean, and its encoding and disk stamp taken from
/// the read. The caret and scroll position stay where they were, as far as the new text allows.
/// Only a tab still open on `path`, still clean, and with the same generation and disk stamp as
/// `mark` is changed: an edit since (saved or not) keeps its text, and its old disk stamp pauses
/// its autosave. Returns whether the tab was reloaded. Call it with nothing of the App borrowed.
pub(crate) fn reload_clean_document(
    hwnd: HWND,
    id: DocumentId,
    path: &std::path::Path,
    mark: TabMark,
    loaded: &crate::file::loader::LoadedFile,
    stamp: Option<crate::library::DiskStamp>,
) -> bool {
    let Some(identity) = (unsafe { window_identity(hwnd) }) else {
        return false;
    };
    let Some((editor, inactive)) = unsafe { app_ptr(hwnd) }.and_then(|app| {
        let app = unsafe { app.as_ref() };
        if app.populating_file {
            return None;
        }
        let target = app.tabs.document(id)?;
        let unchanged = TabMark {
            generation: target.generation,
            disk_stamp: target.disk_stamp,
        } == mark;
        if target.dirty || target.path.as_deref() != Some(path) || !unchanged {
            return None;
        }
        if let Some(editor) = editor_showing(app, id) {
            return Some((editor, None));
        }
        let editor = app.editor().cloned()?;
        Some((editor, Some(target.text_handle()?.clone())))
    }) else {
        return false;
    };
    let populate = |editor: &Editor| editor.populate_clean(&loaded.text);
    let populated = match &inactive {
        Some(target) => with_background_document(hwnd, target, populate),
        None => {
            use crate::editor::scintilla_constants::{
                SCI_GETFIRSTVISIBLELINE, SCI_SETFIRSTVISIBLELINE,
            };
            let selection = editor.selection();
            let first_line = unsafe { SendMessageW(editor.hwnd(), SCI_GETFIRSTVISIBLELINE, 0, 0) };
            set_file_population(hwnd, true);
            let populated = populate(&editor);
            if identity.is_live_for(hwnd) {
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
                set_file_population(hwnd, false);
            }
            populated
        }
    };
    if populated.is_err() || !identity.is_live_for(hwnd) {
        return false;
    }
    if let Some(mut app) = unsafe { app_ptr(hwnd) }
        && let Some(document) = unsafe { app.as_mut() }.tabs.document_mut(id)
    {
        document.encoding = loaded.encoding;
        document.disk_stamp = stamp;
        document.autosave_paused = false;
    }
    if inactive.is_none() {
        // Population suppressed SCN_MODIFIED: the preview reads the new text.
        crate::window::preview_host::document_reloaded(hwnd);
    }
    true
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
    let next = unsafe { app_ptr(hwnd) }.and_then(|app| {
        let restore = unsafe { app.as_ref() }.session_restore.as_ref()?;
        let (index, entry) = restore.next_entry()?;
        // A group whose window could not be made reopens its entries in the first group.
        let group = restore.groups[index].id.or(restore.groups.first()?.id)?;
        Some((group, entry.clone()))
    });
    let Some((group, entry)) = next else {
        finish_session_restore(hwnd);
        return RestoreStep::Done;
    };
    activate_group(hwnd, group);
    let restored = restore_session_entry(hwnd, group, &entry);
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
    if session.is_empty() {
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
        Some(crate::session::SessionRestore::new(&session, placeholder));
    set_up_restored_groups(hwnd);
    bind_ipc_for_restore(hwnd);
    true
}

/// Makes a group window for every saved group after the first, which is the group already open,
/// and arranges them as the manifest's layout (split editors spec §8). A group whose window can't
/// be made has its entries reopened in the first group (spec §9).
fn set_up_restored_groups(hwnd: HWND) {
    let Some((first, saved)) = unsafe { app_ptr(hwnd) }.and_then(|app| {
        let app = unsafe { app.as_ref() };
        let restore = app.session_restore.as_ref()?;
        let numbers = restore
            .groups
            .iter()
            .map(|group| group.number)
            .collect::<Vec<_>>();
        Some((app.tabs.active_group(), (numbers, restore.layout.clone())))
    }) else {
        return;
    };
    let (numbers, layout) = saved;
    let mut ids = vec![Some(first)];
    for _ in 1..numbers.len() {
        ids.push(create_group(hwnd).ok());
    }
    // A failed group leaves the layout; its share goes to its neighbours.
    let layout = numbers
        .iter()
        .zip(&ids)
        .filter(|(_, id)| id.is_none())
        .try_fold(layout, |layout, (number, _)| layout.without(*number));
    let group_of = |number: usize| {
        numbers
            .iter()
            .position(|saved| *saved == number)
            .and_then(|index| ids[index])
    };
    let made = ids.iter().flatten().copied().collect::<Vec<_>>();
    let tree = layout
        .and_then(|layout| crate::window::split_tree::SplitTree::from_session(&layout, &group_of))
        .filter(|tree| tree.leaves().len() == made.len())
        .unwrap_or_else(|| {
            // A layout that doesn't name exactly the groups made: a row of them in order.
            let mut tree = crate::window::split_tree::SplitTree::new(first);
            for pair in made.windows(2) {
                tree.split(
                    pair[0],
                    crate::window::split_tree::Direction::Right,
                    pair[1],
                );
            }
            tree
        });
    if let Some(mut app) = unsafe { app_ptr(hwnd) } {
        let app = unsafe { app.as_mut() };
        app.layout = tree;
        if let Some(restore) = app.session_restore.as_mut() {
            for (group, id) in restore.groups.iter_mut().zip(&ids) {
                group.id = *id;
            }
        }
    }
    layout_editor_and_find_bar(hwnd);
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
    let editor = app.editor()?;
    let empty = unsafe { SendMessageW(editor.hwnd(), SCI_GETLENGTH, 0, 0) } == 0;
    empty.then_some(active.id)
}

/// Reopens one manifest entry in `group`, the active group, and applies its language while it is
/// the active tab. Returns its document, or `None` when the entry could not be reopened. A file
/// or snapshot already reopened for another group becomes a second view of the same document.
fn restore_session_entry(
    hwnd: HWND,
    group: GroupId,
    entry: &crate::session::SessionEntry,
) -> Option<DocumentId> {
    match &entry.source {
        // An open file gets a view in the active group (split editors spec §5.3).
        crate::session::SessionSource::File(path) => open_path(hwnd, path).ok()?,
        crate::session::SessionSource::Snapshot(id) => {
            let reopened = unsafe { app_ptr(hwnd) }.and_then(|app| {
                let app = unsafe { app.as_ref() };
                let (_, document) = app
                    .session_restore
                    .as_ref()?
                    .snapshots
                    .iter()
                    .find(|(snapshot, _)| snapshot == id)?;
                app.tabs.document(*document).map(|_| *document)
            });
            match reopened {
                Some(document) => {
                    if !open_in_active_group(hwnd, document) {
                        return None;
                    }
                }
                None => {
                    let identity = unsafe { window_identity(hwnd) }?;
                    let root = recovery_root(hwnd)?;
                    let path = crate::recovery::snapshot::snapshot_path(&root, *id);
                    let snapshot =
                        crate::recovery::Snapshot::decode(&std::fs::read(&path).ok()?).ok()?;
                    let candidate = crate::recovery::SnapshotCandidate { path, snapshot };
                    open_snapshot_tab(hwnd, &identity, candidate, SnapshotTab::Session).ok()?;
                    // Recorded before the adoption below removes the file the id names.
                    if let Some(mut app) = unsafe { app_ptr(hwnd) } {
                        let app = unsafe { app.as_mut() };
                        let document = app.tabs.active().map(|document| document.id);
                        if let (Some(document), Some(restore)) =
                            (document, app.session_restore.as_mut())
                        {
                            restore.snapshots.push((*id, document));
                        }
                    }
                    adopt_restored_snapshot(hwnd, &identity, &root);
                }
            }
        }
    }
    apply_detected_language(hwnd);
    let (id, text) = unsafe { app_ptr(hwnd) }.and_then(|app| {
        let active = unsafe { app.as_ref() }.tabs.active()?;
        Some((active.id, !active.is_image()))
    })?;
    // Every tab lands where it was, not only the active one: the next entry's open records this
    // position for the tab as it takes the editor over (split editors spec §8).
    if text {
        apply_view_state(hwnd, group, entry);
    }
    Some(id)
}

/// The snapshot a session tab was just restored from belongs to the previous, exited process, so
/// every other FastPad process would take it for a crash leftover and offer it as "Recovered".
/// Rewriting the text under the tab's own ID, owned by this live process, and then removing the
/// source closes that gap. A failed write keeps the source, so the text is always in some file.
fn adopt_restored_snapshot(hwnd: HWND, identity: &WindowIdentity, root: &std::path::Path) {
    let job = unsafe { app_ptr(hwnd) }.and_then(|app| {
        let app = unsafe { app.as_ref() };
        let editor = app.editor().cloned()?;
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
    let restored_any = restore.restored_any();
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
    // Each group shows its saved active view where it was left.
    for (index, group) in restore.groups.iter().enumerate() {
        let Some(id) = group.id else {
            continue;
        };
        if let Some(active) = restore.active_view(index)
            && focus_view(hwnd, id, active)
            && identity.is_live_for(hwnd)
            && restore.saved_active_restored(index) == Some(active)
        {
            apply_view_state(hwnd, id, &group.entries[group.active]);
        }
        if !identity.is_live_for(hwnd) {
            return;
        }
    }
    // A group none of whose entries came back closes, unless it is the only one.
    for id in group_order(hwnd) {
        remove_empty_group(hwnd, id);
    }
    let active = restore
        .groups
        .get(restore.active_group)
        .and_then(|group| group.id)
        .filter(|id| group_order(hwnd).contains(id))
        .or_else(|| group_order(hwnd).first().copied());
    if let Some(active) = active {
        activate_group(hwnd, active);
    }
    layout_editor_and_find_bar(hwnd);
    refresh_tabs(hwnd);
    if !identity.is_live_for(hwnd) {
        return;
    }
    // Each restored tab entered the activation order at the front as it opened. Restart it from
    // the strip, the active tab first (quick-open spec §3.2).
    if let Some(mut app) = unsafe { app_ptr(hwnd) } {
        unsafe { app.as_mut() }.tabs.reset_activation_order();
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

/// Puts group `group`'s editor where `entry` was saved.
fn apply_view_state(hwnd: HWND, group: GroupId, entry: &crate::session::SessionEntry) {
    let Some(editor) = group_editor(hwnd, group) else {
        return;
    };
    // `apply_view_state` clamps: the file may have shrunk since the session was saved.
    let _ = editor.apply_view_state(crate::editor::ViewState {
        caret: entry.caret,
        anchor: entry.anchor,
        first_line: entry.first_line,
        x_offset: 0,
    });
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
        .groups
        .iter()
        .flat_map(|group| &group.entries)
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
            .editor()
            .cloned()
            .ok_or(crate::FastPadError::Invariant("editor was not initialized"))?;
        let (id, recovery_id) = app.allocate_document_identity();
        (editor, id, recovery_id)
    };
    remember_active_view(hwnd);
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
    let result = document
        .expect_text()
        .and_then(|handle| editor.use_document(handle))
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
    let written = if session.is_empty() {
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
/// Clean untitled tabs are empty and skipped. Every text tab keeps its caret and scroll position:
/// the shown one from the editor, the others from where they were left (split editors spec §8).
///
/// Every group is written in layout order, numbered from 1, with its views in strip order and
/// the layout (split editors spec §8). A document in two groups is written under both with the
/// same source. A group left with no entries is dropped, and the layout with it.
pub(crate) fn build_session(hwnd: HWND, root: &std::path::Path) -> Option<crate::session::Session> {
    use crate::session::{Session, SessionEntry, SessionGroup, SessionLayout, SessionSource};
    let app = unsafe { app_ptr(hwnd) }?;
    let app = unsafe { app.as_ref() };
    let order = app.layout.leaves();
    // `None` for a document with nothing to write (a clean untitled tab); an error (`?`) for a
    // dirty one whose text is in no snapshot.
    let source_of = |document: &Document| -> Option<Option<SessionSource>> {
        if document.dirty {
            let file = crate::recovery::current_snapshot_file(root, document)?;
            let id = crate::recovery::snapshot::snapshot_file_id(&file)?;
            return Some(Some(SessionSource::Snapshot(id)));
        }
        Some(
            document
                .path
                .as_ref()
                .filter(|path| path.to_str().is_some())
                .map(|path| SessionSource::File(path.clone())),
        )
    };
    let mut groups = Vec::new();
    let mut dropped = Vec::new();
    for (position, group) in order.iter().enumerate() {
        let number = position + 1;
        let active_id = app.tabs.group(*group)?.active_document();
        let editor = app.group(*group).map(|state| &state.editor);
        let mut active = 0;
        let mut entries = Vec::new();
        for document in app.tabs.group_documents(*group) {
            let is_active = Some(document.id) == active_id;
            let Some(source) = source_of(document)? else {
                if is_active {
                    active = entries.len().saturating_sub(1);
                }
                continue;
            };
            if is_active {
                active = entries.len();
            }
            let mut entry = SessionEntry::new(source);
            // An image tab has no caret: the editor holds a placeholder then.
            if !document.is_image() {
                let state = if is_active {
                    editor
                        .and_then(|editor| editor.view_state().ok())
                        .unwrap_or_default()
                } else {
                    app.tabs.view_state_in(*group, document.id)
                };
                entry.caret = state.caret;
                entry.anchor = state.anchor;
                entry.first_line = state.first_line;
            }
            entries.push(entry);
        }
        if entries.is_empty() {
            dropped.push(number);
            continue;
        }
        groups.push(SessionGroup {
            number,
            active,
            entries,
        });
    }
    let number_of = |id: GroupId| {
        order
            .iter()
            .position(|group| *group == id)
            .map_or(1, |index| index + 1)
    };
    let layout = dropped
        .iter()
        .try_fold(app.layout.to_session(&number_of), |layout, number| {
            layout.without(*number)
        })
        .unwrap_or(SessionLayout::Leaf(1));
    let active_number = number_of(app.tabs.active_group());
    let active_group = groups
        .iter()
        .position(|group| group.number == active_number)
        .unwrap_or(0);
    Some(Session {
        layout,
        active_group,
        groups,
    })
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

/// The group that records `document`'s changes: the first, in layout order, whose active view
/// shows it.
fn reporting_group(hwnd: HWND, document: DocumentId) -> Option<GroupId> {
    let app = unsafe { app_ptr(hwnd) }?;
    groups_showing(unsafe { app.as_ref() }, document)
        .into_iter()
        .next()
}

/// The groups whose active view shows `document`, in layout order.
fn groups_showing(app: &App, document: DocumentId) -> Vec<GroupId> {
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
fn group_of_child(hwnd: HWND, child: HWND) -> Option<GroupId> {
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
fn style_group_view(hwnd: HWND, id: GroupId) {
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
            app.menu_bar = MenuBar::create(&app.keymap).ok();
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
            menus::set_checked_language(menu, active_language(hwnd));
        }
        menus::set_text_commands_enabled(menu, !crate::window::image_host::active_is_image(hwnd));
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
    if crate::window::tab_drag::keeps_key(hwnd, message) {
        return true;
    }
    if menu_activation_message(hwnd, message)
        && unsafe { PostMessageW(hwnd, WM_SYSCOMMAND, SC_KEYMENU as usize, 0) } != 0
    {
        return true;
    }
    // Ctrl+W in the palette's field closes the palette, not a tab (quick-open spec §4). The
    // table would turn it into Close tab before the field's hook saw the key. Ctrl+Z and Ctrl+Y in
    // the Notebook tree's name field stay with the field (inline naming spec §5.1, §11).
    if palette_keeps_key(hwnd, message) || inline_name_keeps_key(hwnd, message) {
        return false;
    }
    if start_tab_for_typing(hwnd, message) {
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

/// A character typed with no tab to take it (the focus on the hidden editor of a group with no
/// tabs, or on the main window once the last tab closed) first opens an untitled tab there, so
/// the character lands in it: a window with no tabs is still instant-to-type. True when the
/// character was delivered here; the message loop then drops it.
fn start_tab_for_typing(
    hwnd: HWND,
    message: &windows_sys::Win32::UI::WindowsAndMessaging::MSG,
) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::WM_CHAR;
    let printable = message.wParam >= 0x20 && message.wParam != 0x7F;
    if message.message != WM_CHAR || !(printable || message.wParam == 0x0D) {
        return false;
    }
    let on_main = message.hwnd == hwnd;
    let id = if on_main {
        unsafe { app_ptr(hwnd) }.map(|app| unsafe { app.as_ref() }.tabs.active_group())
    } else {
        group_of_child(hwnd, message.hwnd).filter(|&id| {
            group_editor(hwnd, id).is_some_and(|editor| editor.hwnd() == message.hwnd)
        })
    };
    let Some(id) = id else {
        return false;
    };
    let empty = unsafe { app_ptr(hwnd) }.is_some_and(|app| {
        unsafe { app.as_ref() }
            .tabs
            .group(id)
            .is_some_and(|group| group.is_empty())
    });
    if !empty {
        return false;
    }
    activate_group(hwnd, id);
    execute_command(hwnd, CommandId::New);
    if !on_main {
        return false;
    }
    let Some(editor) = group_editor(hwnd, id) else {
        return false;
    };
    focus_content(hwnd);
    unsafe { SendMessageW(editor.hwnd(), WM_CHAR, message.wParam, message.lParam) };
    true
}

/// Ctrl+W (without Alt) aimed at one of the command palette's controls.
fn palette_keeps_key(
    hwnd: HWND,
    message: &windows_sys::Win32::UI::WindowsAndMessaging::MSG,
) -> bool {
    ctrl_letter_keydown(message, b'W') && command_palette_owns(hwnd, message.hwnd)
}

/// Ctrl+Z or Ctrl+Y (without Alt) aimed at the Notebook tree's inline name field: the field's own
/// undo, never the editor's Undo or Redo (inline naming spec §5.1, §11).
fn inline_name_keeps_key(
    hwnd: HWND,
    message: &windows_sys::Win32::UI::WindowsAndMessaging::MSG,
) -> bool {
    (ctrl_letter_keydown(message, b'Z') || ctrl_letter_keydown(message, b'Y'))
        && crate::window::inline_name::owns(hwnd, message.hwnd)
}

/// A WM_KEYDOWN of Ctrl+`letter` with Alt up.
fn ctrl_letter_keydown(
    message: &windows_sys::Win32::UI::WindowsAndMessaging::MSG,
    letter: u8,
) -> bool {
    message.message == WM_KEYDOWN
        && message.wParam == usize::from(letter)
        && unsafe { GetKeyState(VK_CONTROL as i32) } < 0
        && unsafe { GetKeyState(VK_MENU as i32) } >= 0
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

unsafe fn install_editor(
    hwnd: HWND,
    group: HWND,
    editor: Editor,
    document: Document,
) -> Result<()> {
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
    let id = app.tabs.active_group();
    app.groups
        .push(crate::window::editor_group::GroupWindow::new(
            id, group, editor,
        ));
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

/// Dispatches everything already posted to `hwnd`, leaving any WM_QUIT for the harness.
#[cfg(test)]
pub(crate) fn pump_posted_messages(hwnd: HWND) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, MSG, PM_REMOVE, PeekMessageW,
    };
    let mut message = MSG::default();
    while unsafe { PeekMessageW(&mut message, hwnd, 0, 0, PM_REMOVE) } != 0 {
        unsafe {
            DispatchMessageW(&message);
        }
    }
}

#[cfg(test)]
mod tests;
