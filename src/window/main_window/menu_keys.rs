//! The menu band's menu mode, opening menus, menu keys, and accelerator translation.

use super::*;

pub(super) fn ensure_accessibility(hwnd: HWND) -> *mut c_void {
    unsafe { app_ptr(hwnd) }
        .map(|mut app| unsafe { app.as_mut() }.ensure_accessibility())
        .unwrap_or(std::ptr::null_mut())
}

pub(crate) fn menu_mode(hwnd: HWND) -> Option<MenuMode> {
    unsafe { app_ptr(hwnd) }.and_then(|app| unsafe { app.as_ref() }.menu_mode)
}

pub(super) fn menu_band_height(hwnd: HWND) -> i32 {
    menu_mode(hwnd).map_or(0, |_| {
        menu_band::band_height(
            unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForWindow(hwnd) }.max(96),
        )
    })
}

/// The menu band's heading rectangles, or none outside menu mode.
pub(super) fn menu_headings(hwnd: HWND) -> Vec<RECT> {
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

pub(super) fn enter_menu_mode(hwnd: HWND, hot: usize) {
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

pub(super) fn exit_menu_mode(hwnd: HWND) {
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
pub(super) fn open_menu(hwnd: HWND, mut index: usize) {
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
pub(super) fn handle_menu_key(hwnd: HWND, message: u32, key: WPARAM) -> bool {
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

pub(super) fn hover_menu_heading(hwnd: HWND, lparam: LPARAM) {
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
    if alt_release_after_click(hwnd, message) {
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
    if editing_key_off_editor(hwnd, message) {
        return false;
    }
    if markdown_key(hwnd, message) {
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

/// A key bound to an editing-shortcut command while the focus is not in an editor: it stays
/// with the focused control, as VS Code's `editorTextFocus` (editing shortcuts spec §6).
fn editing_key_off_editor(
    hwnd: HWND,
    message: &windows_sys::Win32::UI::WindowsAndMessaging::MSG,
) -> bool {
    if !matches!(message.message, WM_KEYDOWN | WM_SYSKEYDOWN) {
        return false;
    }
    let down = |key: u16| unsafe { GetKeyState(i32::from(key)) } < 0;
    let Some(stroke) = crate::window::keymap::KeyStroke::from_key(
        message.wParam as u16,
        down(VK_CONTROL),
        down(VK_SHIFT),
        down(VK_MENU),
    ) else {
        return false;
    };
    let editing = unsafe { app_ptr(hwnd) }
        .and_then(|app| unsafe { app.as_ref() }.keymap.command_for(stroke))
        .is_some_and(CommandId::is_editing);
    editing && !is_group_editor(hwnd, message.hwnd)
}

/// A Markdown-scoped shortcut (Markdown design spec §9): taken only when a Markdown tab's editor has
/// focus, so the same key keeps its global command everywhere else.
fn markdown_key(hwnd: HWND, message: &windows_sys::Win32::UI::WindowsAndMessaging::MSG) -> bool {
    if !matches!(message.message, WM_KEYDOWN | WM_SYSKEYDOWN) {
        return false;
    }
    if active_language(hwnd) != crate::document::Language::Markdown {
        return false;
    }
    if unsafe { editor_hwnd(hwnd) } != Some(message.hwnd) {
        return false;
    }
    let down = |key: u16| unsafe { GetKeyState(i32::from(key)) } < 0;
    let Some(stroke) = crate::window::keymap::KeyStroke::from_key(
        message.wParam as u16,
        down(VK_CONTROL),
        down(VK_SHIFT),
        down(VK_MENU),
    ) else {
        return false;
    };
    let command = unsafe { app_ptr(hwnd) }.and_then(|app| {
        unsafe { app.as_ref() }
            .keymap
            .command_for_in(stroke, crate::window::commands::Scope::Markdown)
    });
    let Some(command) = command else {
        return false;
    };
    execute_command(hwnd, command);
    true
}

/// Whether `window` is one of the editor groups' editors.
fn is_group_editor(hwnd: HWND, window: HWND) -> bool {
    group_of_child(hwnd, window)
        .and_then(|id| group_editor(hwnd, id))
        .is_some_and(|editor| editor.hwnd() == window)
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
        // A held Alt auto-repeats (bit 30: the key was already down). Only the first press
        // starts a tap; a repeat after Alt+Click must not re-arm the menu.
        if message.lParam & (1 << 30) != 0 {
            return false;
        }
        app.set_menu_alt_pending(no_control_or_shift);
        app.set_menu_alt_clicked(false);
        return false;
    }
    if message.message == WM_SYSKEYUP && message.wParam == VK_MENU as usize {
        return app.take_menu_alt_pending();
    }
    // Alt+Click (a caret in the editor) is Alt with other input, not a bare tap.
    if matches!(
        message.message,
        WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN
    ) && unsafe { GetKeyState(VK_MENU as i32) } < 0
    {
        app.set_menu_alt_clicked(true);
    }
    if matches!(
        message.message,
        WM_KEYDOWN | WM_SYSKEYDOWN | WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN
    ) {
        app.set_menu_alt_pending(false);
    }
    false
}

/// The release of an Alt held for a click. Windows counts a click as no input, so passed on it
/// would open the menu band; consumed, it does nothing (editing shortcuts spec §5).
fn alt_release_after_click(
    hwnd: HWND,
    message: &windows_sys::Win32::UI::WindowsAndMessaging::MSG,
) -> bool {
    message.message == WM_SYSKEYUP
        && message.wParam == VK_MENU as usize
        && unsafe { app_ptr(hwnd) }
            .is_some_and(|mut app| unsafe { app.as_mut() }.take_menu_alt_clicked())
}
