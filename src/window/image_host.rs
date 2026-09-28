//! Image tabs (image preview spec §5–§6): owns the image view, shows it for the active image tab,
//! and hides it and frees its device resources for text tabs.

use crate::image_view::ImageView;
use crate::window::commands::CommandId;
use crate::window::main_window as host_window;
use windows_sys::Win32::Foundation::{HWND, RECT};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetFocus, SetFocus};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    IsWindowVisible, MoveWindow, SW_HIDE, SW_SHOWNA, ShowWindow,
};

#[derive(Debug, Default)]
pub(crate) struct ImageHost {
    pub(crate) view: Option<ImageView>,
    /// Set once Direct2D failed to load, so the notice appears once.
    unavailable: bool,
}

/// The status-bar facts of the active image tab.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ImageStatus {
    pub size: Option<(u32, u32)>,
    pub format: Option<&'static str>,
    pub bytes: Option<u64>,
    pub zoom_percent: Option<u32>,
}

fn with_host<R>(hwnd: HWND, action: impl FnOnce(&mut ImageHost) -> R) -> Option<R> {
    unsafe { host_window::app_ptr(hwnd) }.map(|mut app| action(&mut unsafe { app.as_mut() }.image))
}

fn view(hwnd: HWND) -> Option<ImageView> {
    with_host(hwnd, |host| host.view).flatten()
}

/// The active tab's path, disk stamp and file name, when it is an image tab.
fn active_image(hwnd: HWND) -> Option<(std::path::PathBuf, Option<crate::library::DiskStamp>)> {
    let app = unsafe { host_window::app_ptr(hwnd) }?;
    let document = unsafe { app.as_ref() }.tabs.active()?;
    document
        .is_image()
        .then(|| (document.path.clone(), document.disk_stamp))
        .and_then(|(path, stamp)| Some((path?, stamp)))
}

pub(crate) fn active_is_image(hwnd: HWND) -> bool {
    active_image(hwnd).is_some()
}

pub(crate) fn shown_view_hwnd(hwnd: HWND) -> Option<HWND> {
    active_is_image(hwnd)
        .then(|| view(hwnd).map(|view| view.hwnd()))
        .flatten()
}

fn ensure_view(hwnd: HWND) -> Option<ImageView> {
    if let Some(view) = view(hwnd) {
        return Some(view);
    }
    if with_host(hwnd, |host| host.unavailable).unwrap_or(true) {
        return None;
    }
    let created = crate::window::preview_host::shared_graphics(hwnd).and_then(|graphics| {
        let (colors, high_contrast) = crate::window::preview_host::image_colors(hwnd);
        ImageView::create(hwnd, graphics, colors, high_contrast)
    });
    match created {
        Ok(view) => {
            with_host(hwnd, |host| host.view = Some(view));
            Some(view)
        }
        Err(error) => {
            with_host(hwnd, |host| host.unavailable = true);
            host_window::push_notice(hwnd, format!("FastPad could not display images: {error}"));
            None
        }
    }
}

/// Follows tab changes: shows the view for an image tab and moves the keyboard focus onto it,
/// or hides it and frees its render target for a text tab (or no tab).
pub(crate) fn sync(hwnd: HWND) {
    let Some((path, stamp)) = active_image(hwnd) else {
        if let Some(view) = view(hwnd) {
            let had_focus = unsafe { GetFocus() } == view.hwnd();
            unsafe { ShowWindow(view.hwnd(), SW_HIDE) };
            view.release();
            if had_focus {
                // With no tab left the editor is hidden: typing must not reach its placeholder.
                let target = unsafe { host_window::editor_hwnd(hwnd) }
                    .filter(|&editor| unsafe { IsWindowVisible(editor) } != 0)
                    .unwrap_or(hwnd);
                unsafe { SetFocus(target) };
            }
        }
        return;
    };
    let Some(view) = ensure_view(hwnd) else {
        return;
    };
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    view.show_file(&path, stamp, &name);
    host_window::layout_editor_and_find_bar(hwnd);
    unsafe { ShowWindow(view.hwnd(), SW_SHOWNA) };
    // The content area's focus follows it onto the image, whether it was on the editor or on a
    // Markdown or SVG preview that this tab hides.
    let focus = unsafe { GetFocus() };
    let in_content = focus == hwnd
        || unsafe { host_window::editor_hwnd(hwnd) } == Some(focus)
        || crate::window::preview_host::owns_view(hwnd, focus);
    if in_content {
        unsafe { SetFocus(view.hwnd()) };
    }
}

pub(crate) fn layout(hwnd: HWND, area: RECT) {
    if let Some(view) = view(hwnd).filter(|_| active_is_image(hwnd)) {
        unsafe {
            MoveWindow(
                view.hwnd(),
                area.left,
                area.top,
                area.right - area.left,
                area.bottom - area.top,
                1,
            )
        };
    }
}

/// Runs a zoom command on the active image tab; false when the active tab is not an image.
pub(crate) fn zoom(hwnd: HWND, command: CommandId) -> bool {
    let Some(view) = shown_view_hwnd(hwnd).and_then(|_| view(hwnd)) else {
        return false;
    };
    match command {
        CommandId::ZoomIn => view.zoom_in(),
        CommandId::ZoomOut => view.zoom_out(),
        CommandId::ZoomReset => view.zoom_reset(),
        _ => return false,
    }
    true
}

/// Re-reads the active image tab's disk stamp. A changed file is decoded again; a deleted one
/// shows "The file no longer exists."
pub(crate) fn check_disk(hwnd: HWND) {
    let Some((path, known)) = active_image(hwnd) else {
        return;
    };
    let now = crate::library::disk_stamp(&path);
    if now == known {
        return;
    }
    if let Some(mut app) = unsafe { host_window::app_ptr(hwnd) }
        && let Some(document) = unsafe { app.as_mut() }.tabs.active_mut()
    {
        document.disk_stamp = now;
    }
    match (now, view(hwnd)) {
        (None, Some(view)) => view.show_error(crate::image_view::decode::ImageError::Missing),
        (Some(_), Some(_)) => sync(hwnd),
        _ => {}
    }
    host_window::invalidate_status_bar(hwnd);
}

pub(crate) fn status(hwnd: HWND) -> Option<ImageStatus> {
    let (_, stamp) = active_image(hwnd)?;
    let view_status = view(hwnd).map(|view| view.status()).unwrap_or_default();
    Some(ImageStatus {
        size: view_status.size,
        format: view_status.format,
        bytes: stamp.map(|stamp| stamp.size),
        zoom_percent: view_status.zoom_percent,
    })
}

pub(crate) fn refresh_appearance(hwnd: HWND) {
    if let Some(view) = view(hwnd) {
        let (colors, high_contrast) = crate::window::preview_host::image_colors(hwnd);
        view.set_appearance(colors, high_contrast);
    }
}
