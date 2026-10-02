//! Image tabs (image preview spec §5–§6): owns the image view, shows it for the active image tab,
//! and hides it and frees its device resources for text tabs.

use crate::image_view::ImageView;
use crate::window::commands::CommandId;
use crate::window::main_window as host_window;
use crate::window::split_tree::GroupId;
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

/// Runs `action` on the active group's image host.
fn with_host<R>(hwnd: HWND, action: impl FnOnce(&mut ImageHost) -> R) -> Option<R> {
    unsafe { host_window::app_ptr(hwnd) }.and_then(|mut app| {
        Some(action(
            &mut unsafe { app.as_mut() }.active_group_mut()?.image,
        ))
    })
}

/// `with_host` for the group `id`, which need not be the active one.
pub(crate) fn with_group_host<R>(
    hwnd: HWND,
    id: GroupId,
    action: impl FnOnce(&mut ImageHost) -> R,
) -> Option<R> {
    unsafe { host_window::app_ptr(hwnd) }
        .and_then(|mut app| Some(action(&mut unsafe { app.as_mut() }.group_mut(id)?.image)))
}

fn view(hwnd: HWND) -> Option<ImageView> {
    with_host(hwnd, |host| host.view).flatten()
}

fn group_view(hwnd: HWND, id: GroupId) -> Option<ImageView> {
    with_group_host(hwnd, id, |host| host.view).flatten()
}

type ImageFile = (std::path::PathBuf, Option<crate::library::DiskStamp>);

/// The active tab's path and disk stamp, when it is an image tab.
fn active_image(hwnd: HWND) -> Option<ImageFile> {
    let app = unsafe { host_window::app_ptr(hwnd) }?;
    group_image(hwnd, unsafe { app.as_ref() }.tabs.active_group())
}

/// Group `id`'s active tab's path and disk stamp, when it is an image tab.
fn group_image(hwnd: HWND, id: GroupId) -> Option<ImageFile> {
    let app = unsafe { host_window::app_ptr(hwnd) }?;
    let tabs = &unsafe { app.as_ref() }.tabs;
    let document = tabs.document(tabs.group(id)?.active_document()?)?;
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

fn ensure_view(hwnd: HWND, id: GroupId) -> Option<ImageView> {
    if let Some(view) = group_view(hwnd, id) {
        return Some(view);
    }
    if with_group_host(hwnd, id, |host| host.unavailable).unwrap_or(true) {
        return None;
    }
    let parent = host_window::with_group_id(hwnd, id, |group| group.hwnd)?;
    let created = crate::window::preview_host::shared_graphics(hwnd).and_then(|graphics| {
        let (colors, high_contrast) = crate::window::preview_host::image_colors(hwnd);
        ImageView::create(parent, graphics, colors, high_contrast)
    });
    match created {
        Ok(view) => {
            with_group_host(hwnd, id, |host| host.view = Some(view));
            Some(view)
        }
        Err(error) => {
            with_group_host(hwnd, id, |host| host.unavailable = true);
            host_window::push_notice(hwnd, format!("FastPad could not display images: {error}"));
            None
        }
    }
}

/// Follows tab changes in the active group: shows the view for an image tab and moves the
/// keyboard focus onto it, or hides it and frees its render target for a text tab (or no tab).
pub(crate) fn sync(hwnd: HWND) {
    if let Some(app) = unsafe { host_window::app_ptr(hwnd) } {
        let id = unsafe { app.as_ref() }.tabs.active_group();
        sync_group(hwnd, id);
    }
}

/// `sync` for group `id`, which need not be the active one: a group left showing an image tab
/// shows it rather than whatever its hidden editor last drew. Only the active group takes the
/// focus.
pub(crate) fn sync_group(hwnd: HWND, id: GroupId) {
    let active = unsafe { host_window::app_ptr(hwnd) }
        .is_some_and(|app| unsafe { app.as_ref() }.tabs.active_group() == id);
    let Some((path, stamp)) = group_image(hwnd, id) else {
        if let Some(view) = group_view(hwnd, id) {
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
    let Some(view) = ensure_view(hwnd, id) else {
        return;
    };
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    view.show_file(&path, stamp, &name);
    host_window::layout_editor_and_find_bar(hwnd);
    unsafe { ShowWindow(view.hwnd(), SW_SHOWNA) };
    if !active {
        return;
    }
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

/// Lays out group `id`'s image view, shown when that group's active tab is an image.
pub(crate) fn layout(hwnd: HWND, id: GroupId, area: RECT) {
    let image = unsafe { host_window::app_ptr(hwnd) }.is_some_and(|app| {
        let tabs = &unsafe { app.as_ref() }.tabs;
        tabs.group(id)
            .and_then(|group| group.active_document())
            .and_then(|document| tabs.document(document))
            .is_some_and(|document| document.is_image())
    });
    if let Some(view) = with_group_host(hwnd, id, |host| host.view)
        .flatten()
        .filter(|_| image)
    {
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
