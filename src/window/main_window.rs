//! The main window. This root holds the imports its child modules share (each starts with
//! `use super::*;`) and re-exports their items, so callers keep using `main_window::name`.

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

mod background_documents;
mod chrome;
mod command_dispatch;
mod command_palette_ui;
mod document_tabs;
mod find;
mod focus;
mod group_layout;
mod ipc_host;
mod language_tools;
mod menu_keys;
mod opening;
mod placement;
mod saving;
mod session_restore;
mod session_save;
mod settings_apply;
mod snapshots;
mod split_groups;
mod startup;
mod tab_strip;
mod window_class;
mod wndproc;

pub(crate) use background_documents::*;
pub(crate) use chrome::*;
use command_dispatch::*;
pub(crate) use command_palette_ui::*;
pub(crate) use document_tabs::*;
pub(crate) use find::*;
pub(crate) use focus::*;
pub(crate) use group_layout::*;
pub(crate) use ipc_host::*;
use language_tools::*;
pub(crate) use menu_keys::*;
pub(crate) use opening::*;
pub(crate) use placement::restore_placement;
use placement::save_placement;
#[cfg(test)]
pub(crate) use saving::save_path_as;
pub(in crate::window) use saving::*;
use session_restore::*;
pub(crate) use session_save::*;
pub(crate) use settings_apply::*;
pub(in crate::window) use snapshots::*;
pub(crate) use split_groups::*;
pub(crate) use startup::*;
pub(crate) use tab_strip::*;
pub(crate) use window_class::*;
use wndproc::*;

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
