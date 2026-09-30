//! Keyboard focus in the content area and F6 cycling, plus the current theme's palette, file
//! icons and UI fonts.

use super::*;

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
pub(super) fn bar_band_height(hwnd: HWND) -> i32 {
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
pub(super) fn content_focus_target(hwnd: HWND) -> Option<HWND> {
    crate::window::image_host::shown_view_hwnd(hwnd)
        .or_else(|| crate::window::preview_host::full_view_hwnd(hwnd))
        .or_else(|| unsafe { editor_hwnd(hwnd) })
}
