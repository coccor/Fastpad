//! The editor toolbar (2026-10-22 editor toolbar spec): a row under each editor group's tab strip
//! showing the active tab's location on the left and, on Markdown and SVG tabs, the view mode
//! control on the right (view modes spec §2). A child window of the group; the find bar, the
//! editor and the previews are laid out below it.

use crate::platform::wide_null;
use crate::preview::ViewMode;
use crate::window::design::metrics::{CONTROL_RADIUS, scale};
use crate::window::design::round::{Corners, fill_rounded, radius_for};
use crate::window::design::text_scale::scale_text;
use crate::window::design::type_ramp::{self, TextStyle};
use crate::window::split_tree::GroupId;
use crate::window::titlebar::{
    Point, Rect, draw_text, fill, measure_text, restore_font, select_font,
};
use crate::window::tooltip::Tooltip;
use std::path::Path;
use windows_sys::Win32::Foundation::{
    ERROR_CLASS_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, DT_CENTER, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER,
    DeleteObject, EndPaint, InvalidateRect, MapWindowPoints, PAINTSTRUCT, SetBkMode, SetTextColor,
    TRANSPARENT,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::WM_MOUSELEAVE;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DefWindowProcW, FindWindowExW, GetClientRect, HWND_TOP, IDC_ARROW, IsWindowVisible,
    LoadCursorW, RegisterClassW, SWP_NOACTIVATE, SWP_SHOWWINDOW, SetWindowPos, WM_ERASEBKGND,
    WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT, WNDCLASSW, WS_CHILD,
    WS_CLIPSIBLINGS,
};

const CLASS_NAME: &str = "FastPadEditorToolbar";
/// The row's height, a segment's size, the track's padding and its gap from the right edge, at
/// 96 DPI and 100 % text size.
const ROW_HEIGHT: i32 = 32;
const SEGMENT_WIDTH: i32 = 72;
const SEGMENT_HEIGHT: i32 = 24;
const TRACK_PADDING: i32 = 2;
const RIGHT_MARGIN: i32 = 8;
const LEFT_MARGIN: i32 = 16;
/// Between the location's parts.
const SEPARATOR: &str = "  \u{203A}  ";

/// The toolbar's window, once created, and its pointer state; kept on its group.
#[derive(Debug)]
pub(crate) struct EditorToolbar {
    pub(crate) hwnd: HWND,
    /// The view modes shown, in order: every mode for Markdown, no Live for SVG, none otherwise.
    segments: &'static [ViewMode],
    hovered: Option<ViewMode>,
    pressed: Option<ViewMode>,
    /// Made on the first pointer move over the row; destroyed with the group.
    tooltip: Option<Tooltip>,
    /// The tooltip could not be made, and that is not tried again.
    tooltip_failed: bool,
}

impl Drop for EditorToolbar {
    fn drop(&mut self) {
        if let Some(tooltip) = self.tooltip.take() {
            tooltip.destroy();
        }
    }
}

impl Default for EditorToolbar {
    fn default() -> Self {
        Self {
            hwnd: std::ptr::null_mut(),
            segments: &[],
            hovered: None,
            pressed: None,
            tooltip: None,
            tooltip_failed: false,
        }
    }
}

/// The row's height at `dpi`.
pub(crate) fn row_height(dpi: u32) -> i32 {
    scale_text(ROW_HEIGHT, dpi)
}

/// The active group's toolbar, once created.
#[cfg(test)]
pub(crate) fn hwnd(main: HWND) -> Option<HWND> {
    crate::window::main_window::with_group(main, |group| group.toolbar.hwnd)
        .filter(|hwnd| !hwnd.is_null())
}

/// The modes group `id`'s active tab can be shown in, in the control's order; empty for a tab that
/// is neither Markdown nor SVG.
pub(crate) fn segments_for(main: HWND, id: GroupId) -> &'static [ViewMode] {
    match crate::window::preview_host::group_document(main, id).map(|(_, language, _)| language) {
        Some(crate::document::Language::Markdown) => &ViewMode::ALL,
        Some(crate::document::Language::Svg) => &ViewMode::SVG,
        _ => &[],
    }
}

/// The location's parts for a tab: inside `notebook`, the notebook folder, its subfolders and the
/// file; elsewhere the file's folder and the file; untitled, the tab's `title`.
pub(crate) fn location_segments(
    path: Option<&Path>,
    title: &str,
    notebook: Option<&Path>,
) -> Vec<String> {
    let Some(path) = path else {
        return vec![title.to_owned()];
    };
    let name = |part: &std::ffi::OsStr| part.to_string_lossy().into_owned();
    let file = path.file_name().map_or_else(|| title.to_owned(), name);
    if let Some(notebook) = notebook
        && let Ok(inside) = path.strip_prefix(notebook)
    {
        let mut parts = vec![
            notebook
                .file_name()
                .map_or_else(|| notebook.to_string_lossy().into_owned(), name),
        ];
        if let Some(folders) = inside.parent() {
            parts.extend(folders.iter().map(name));
        }
        parts.push(file);
        return parts;
    }
    match path.parent().and_then(Path::file_name) {
        Some(folder) => vec![name(folder), file],
        None => vec![file],
    }
}

/// Group `id`'s active tab's location.
fn location(main: HWND, id: GroupId) -> Vec<String> {
    let notebook = crate::window::library_host::folder(main);
    unsafe { crate::window::main_window::app_ptr(main) }
        .and_then(|app| {
            let tabs = &unsafe { app.as_ref() }.tabs;
            let document = tabs.document(tabs.group(id)?.active_document()?)?;
            Some(location_segments(
                document.path.as_deref(),
                &document.title(),
                notebook.as_deref(),
            ))
        })
        .unwrap_or_default()
}

/// The segment at `index` of `count`, in the toolbar window's client coordinates: right-aligned,
/// centred in the row. Computed from the window alone, so the accessibility provider can call it
/// from its own thread.
pub(crate) fn segment_rect(toolbar: HWND, count: usize, index: usize) -> Rect {
    let dpi = unsafe { GetDpiForWindow(toolbar) }.max(96);
    let mut client = RECT::default();
    unsafe { GetClientRect(toolbar, &mut client) };
    let track = track_rect(client.right, client.bottom, count, dpi);
    let padding = scale(TRACK_PADDING, dpi);
    let width = scale_text(SEGMENT_WIDTH, dpi);
    let left = track.left + padding + index as i32 * width;
    Rect::new(
        left,
        track.top + padding,
        left + width,
        track.bottom - padding,
    )
}

/// The track behind `count` segments in a row `width` × `height`.
fn track_rect(width: i32, height: i32, count: usize, dpi: u32) -> Rect {
    let padding = scale(TRACK_PADDING, dpi);
    let track_width = count as i32 * scale_text(SEGMENT_WIDTH, dpi) + 2 * padding;
    let track_height = scale_text(SEGMENT_HEIGHT, dpi) + 2 * padding;
    let right = width - scale(RIGHT_MARGIN, dpi);
    let top = (height - track_height) / 2;
    Rect::new(right - track_width, top, right, top + track_height)
}

/// The modes the toolbar `toolbar` shows, and its group.
fn segments(main: HWND, toolbar: HWND) -> (&'static [ViewMode], Option<GroupId>) {
    let id = crate::window::main_window::group_id_of(main, unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::GetParent(toolbar)
    });
    let segments = id
        .and_then(|id| {
            crate::window::main_window::with_group_id(main, id, |group| {
                (group.toolbar.hwnd == toolbar).then_some(group.toolbar.segments)
            })
            .flatten()
        })
        .unwrap_or(&[]);
    (segments, id)
}

fn segment_at(main: HWND, toolbar: HWND, x: i32, y: i32) -> Option<ViewMode> {
    let (segments, _) = segments(main, toolbar);
    segments
        .iter()
        .enumerate()
        .find(|(index, _)| segment_rect(toolbar, segments.len(), *index).contains(Point::new(x, y)))
        .map(|(_, mode)| *mode)
}

/// The shown toolbar inside `group`, for the group's accessibility provider, which has only the
/// group's window to go on.
pub(crate) fn find(group: HWND) -> Option<HWND> {
    let class = wide_null(CLASS_NAME);
    let toolbar = unsafe {
        FindWindowExW(
            group,
            std::ptr::null_mut(),
            class.as_ptr(),
            std::ptr::null(),
        )
    };
    (!toolbar.is_null() && unsafe { IsWindowVisible(toolbar) } != 0).then_some(toolbar)
}

/// The segment at `index` of `count` in `group`'s client coordinates while the toolbar shows.
pub(crate) fn rect_in_group(group: HWND, count: usize, index: usize) -> Option<Rect> {
    let toolbar = find(group)?;
    let mut origin = POINT { x: 0, y: 0 };
    unsafe { MapWindowPoints(toolbar, group, &mut origin, 1) };
    let rect = segment_rect(toolbar, count, index);
    Some(Rect::new(
        rect.left + origin.x,
        rect.top + origin.y,
        rect.right + origin.x,
        rect.bottom + origin.y,
    ))
}

/// Places `group`'s toolbar across the group at `top`, `width` wide, and returns the height it
/// takes: 0 only when its window could not be made.
pub(crate) fn layout(main: HWND, group: HWND, id: GroupId, top: i32, width: i32, dpi: u32) -> i32 {
    let existing = crate::window::main_window::with_group_id(main, id, |state| state.toolbar.hwnd)
        .filter(|hwnd| !hwnd.is_null());
    // Shown in an empty group too, empty, so opening the first tab never moves the editor area.
    let Some(toolbar) = existing.or_else(|| create(main, id, group)) else {
        return 0;
    };
    let segments = segments_for(main, id);
    // Other segments mean other tooltip rectangles: the tooltip is made again on the next hover.
    let stale = crate::window::main_window::with_group_id(main, id, |state| {
        let control = &mut state.toolbar;
        if control.segments == segments {
            return None;
        }
        control.segments = segments;
        control.hovered = None;
        control.pressed = None;
        control.tooltip_failed = false;
        control.tooltip.take()
    })
    .flatten();
    if let Some(tooltip) = stale {
        tooltip.destroy();
    }
    let height = row_height(dpi);
    unsafe {
        SetWindowPos(
            toolbar,
            HWND_TOP,
            0,
            top,
            width,
            height,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
        InvalidateRect(toolbar, std::ptr::null(), 0);
    }
    height
}

fn create(main: HWND, id: GroupId, group: HWND) -> Option<HWND> {
    let toolbar = crate::window::panel::create_child(
        group,
        register_class().ok()?,
        WS_CHILD | WS_CLIPSIBLINGS,
    )
    .ok()?;
    crate::window::main_window::with_group_id(main, id, |state| {
        state.toolbar.hwnd = toolbar;
    });
    Some(toolbar)
}

fn register_class() -> crate::Result<&'static [u16]> {
    static CLASS: std::sync::OnceLock<Vec<u16>> = std::sync::OnceLock::new();
    static REGISTERED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let name = CLASS.get_or_init(|| wide_null(CLASS_NAME));
    let registered = *REGISTERED.get_or_init(|| {
        let class = WNDCLASSW {
            lpfnWndProc: Some(toolbar_proc),
            hInstance: unsafe { GetModuleHandleW(std::ptr::null()) },
            hCursor: unsafe { LoadCursorW(std::ptr::null_mut(), IDC_ARROW) },
            lpszClassName: name.as_ptr(),
            ..Default::default()
        };
        unsafe { RegisterClassW(&class) != 0 || GetLastError() == ERROR_CLASS_ALREADY_EXISTS }
    });
    if registered {
        Ok(name)
    } else {
        Err(crate::FastPadError::Invariant(
            "the editor toolbar window class could not be registered",
        ))
    }
}

fn with_toolbar<R>(
    main: HWND,
    id: GroupId,
    action: impl FnOnce(&mut EditorToolbar) -> R,
) -> Option<R> {
    crate::window::main_window::with_group_id(main, id, |group| action(&mut group.toolbar))
}

/// The first pointer move over the row makes its tooltip and hands it that move, so the first
/// hover starts the tip's timer like any later one. Nothing before that needs it.
fn ensure_tooltip(main: HWND, toolbar: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) {
    let (segments, Some(id)) = segments(main, toolbar) else {
        return;
    };
    if segments.is_empty() {
        return;
    }
    let wanted = with_toolbar(main, id, |state| {
        state.hwnd == toolbar && state.tooltip.is_none() && !state.tooltip_failed
    })
    .unwrap_or(false);
    if !wanted {
        return;
    }
    // Made with nothing of the App borrowed: creating the control sends messages.
    let created = Tooltip::create(toolbar);
    let stored = with_toolbar(main, id, |state| {
        let stored = state.hwnd == toolbar && state.tooltip.is_none();
        if stored {
            state.tooltip = created;
            state.tooltip_failed = created.is_none();
        }
        stored
    })
    .unwrap_or(false);
    match created {
        Some(tooltip) if stored => {
            for (index, mode) in segments.iter().enumerate() {
                let rect = segment_rect(toolbar, segments.len(), index);
                let rect = RECT {
                    left: rect.left,
                    top: rect.top,
                    right: rect.right,
                    bottom: rect.bottom,
                };
                tooltip.set_tool(
                    index,
                    rect,
                    &crate::window::preview_host::button_text(main, *mode),
                );
            }
            tooltip.relay(message, wparam, lparam);
        }
        // The group went or the toolbar was replaced while the tooltip was being made.
        Some(tooltip) => tooltip.destroy(),
        None => {}
    }
}

fn update(main: HWND, toolbar: HWND, change: impl FnOnce(&mut EditorToolbar)) {
    if let (_, Some(id)) = segments(main, toolbar) {
        with_toolbar(main, id, change);
    }
    unsafe { InvalidateRect(toolbar, std::ptr::null(), 0) };
}

unsafe extern "system" fn toolbar_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let main = crate::platform::win32::root_window(hwnd);
    let (x, y) = (
        (lparam as u32 & 0xffff) as u16 as i16 as i32,
        ((lparam as u32 >> 16) & 0xffff) as u16 as i16 as i32,
    );
    match message {
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            paint(main, hwnd);
            0
        }
        WM_MOUSEMOVE => {
            ensure_tooltip(main, hwnd, message, wparam, lparam);
            let mut track = TRACKMOUSEEVENT {
                cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: hwnd,
                dwHoverTime: 0,
            };
            unsafe { TrackMouseEvent(&mut track) };
            let over = segment_at(main, hwnd, x, y);
            update(main, hwnd, |state| state.hovered = over);
            if let (_, Some(id)) = segments(main, hwnd) {
                crate::window::preview_host::in_group(id, || {
                    crate::window::preview_host::button_hover(main, over)
                });
            }
            0
        }
        WM_MOUSELEAVE => {
            update(main, hwnd, |state| {
                state.hovered = None;
                state.pressed = None;
            });
            if let (_, Some(id)) = segments(main, hwnd) {
                crate::window::preview_host::in_group(id, || {
                    crate::window::preview_host::button_hover(main, None)
                });
            }
            0
        }
        WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
            let over = segment_at(main, hwnd, x, y);
            update(main, hwnd, |state| {
                state.hovered = over;
                state.pressed = over;
            });
            0
        }
        // A click acts only when released over the segment it was pressed on, on this toolbar's
        // own group, which becomes the active one.
        WM_LBUTTONUP => {
            let over = segment_at(main, hwnd, x, y);
            let mut pressed = None;
            update(main, hwnd, |state| pressed = state.pressed.take());
            if let (Some(mode), (_, Some(id))) = (
                over.filter(|&mode| pressed == Some(mode)),
                segments(main, hwnd),
            ) {
                crate::window::main_window::activate_group(main, id);
                crate::window::preview_host::set_group_mode(main, id, mode);
            }
            0
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

fn paint(main: HWND, toolbar: HWND) {
    let mut paint = PAINTSTRUCT::default();
    let dc = unsafe { BeginPaint(toolbar, &mut paint) };
    if dc.is_null() {
        return;
    }
    let dpi = unsafe { GetDpiForWindow(toolbar) }.max(96);
    let (palette, _, _) = crate::window::main_window::title_chrome(main);
    let (segments, id) = segments(main, toolbar);
    let mode = id.map_or(ViewMode::Source, |id| {
        crate::window::preview_host::group_mode(main, id)
    });
    let (hovered, pressed) = id
        .and_then(|id| with_toolbar(main, id, |state| (state.hovered, state.pressed)))
        .unwrap_or_default();
    let parts = id.map(|id| location(main, id)).unwrap_or_default();
    let mut client = RECT::default();
    unsafe { GetClientRect(toolbar, &mut client) };
    let row = Rect::new(0, 0, client.right, client.bottom);
    let line = scale(1, dpi).max(1);
    unsafe {
        fill(dc, row, palette.editor_background);
        fill(
            dc,
            Rect::new(0, row.bottom - line, row.right, row.bottom),
            palette.stroke,
        );
        SetBkMode(dc, TRANSPARENT as i32);
    }
    let font = type_ramp::create(TextStyle::Body, dpi);
    let bold = type_ramp::create(TextStyle::PanelHeader, dpi);
    let previous = unsafe { select_font(dc, font) };
    let track = (!segments.is_empty())
        .then(|| track_rect(row.right, row.bottom - line, segments.len(), dpi));
    // The location, cut before it reaches the view control.
    let left = scale(LEFT_MARGIN, dpi);
    let right = track.map_or(row.right - scale(RIGHT_MARGIN, dpi), |track| {
        track.left - scale(RIGHT_MARGIN, dpi)
    });
    if let Some((file, folders)) = parts.split_last() {
        let format = DT_SINGLELINE | DT_VCENTER | DT_LEFT | DT_NOPREFIX | DT_END_ELLIPSIS;
        let prefix = folders
            .iter()
            .map(|part| format!("{part}{SEPARATOR}"))
            .collect::<String>();
        let available = (right - left).max(0);
        // Measured without the ellipsis flag: with it, DT_CALCRECT fits the text to the empty
        // starting rectangle and reports almost nothing.
        let measure = DT_SINGLELINE | DT_NOPREFIX;
        let name_width = unsafe { measure_text(dc, file, measure) }.min(available);
        let prefix_width =
            unsafe { measure_text(dc, &prefix, measure) }.min(available - name_width);
        let text_row = Rect::new(left, 0, right, row.bottom - line);
        unsafe {
            SetTextColor(dc, palette.muted_foreground);
            draw_text(
                dc,
                &prefix,
                Rect::new(left, text_row.top, left + prefix_width, text_row.bottom),
                format,
            );
            SetTextColor(dc, palette.editor_foreground);
            draw_text(
                dc,
                file,
                Rect::new(
                    left + prefix_width,
                    text_row.top,
                    left + prefix_width + name_width,
                    text_row.bottom,
                ),
                format,
            );
        }
    }
    if let Some(track) = track {
        let radius = radius_for(&palette, CONTROL_RADIUS, dpi);
        let native = |rect: Rect| RECT {
            left: rect.left,
            top: rect.top,
            right: rect.right,
            bottom: rect.bottom,
        };
        let track_color = if palette.high_contrast {
            palette.editor_background
        } else {
            palette.strip_background
        };
        unsafe {
            fill_rounded(
                dc,
                native(track),
                radius,
                Corners::ALL,
                track_color,
                palette.editor_background,
            );
        }
        let centered = DT_SINGLELINE | DT_VCENTER | DT_CENTER | DT_NOPREFIX;
        for (index, &segment) in segments.iter().enumerate() {
            let rect = segment_rect(toolbar, segments.len(), index);
            let current = segment == mode;
            let over = hovered == Some(segment);
            let held = over && pressed == Some(segment);
            unsafe {
                if current {
                    if palette.high_contrast {
                        fill(dc, rect, palette.selection_background);
                    } else {
                        crate::window::design::round::fill_bordered(
                            dc,
                            native(rect),
                            radius,
                            palette.editor_background,
                            palette.stroke,
                            track_color,
                        );
                    }
                } else if over {
                    if palette.high_contrast {
                        crate::window::design::round::fill_bordered(
                            dc,
                            native(rect),
                            0,
                            palette.editor_background,
                            palette.editor_foreground,
                            track_color,
                        );
                    } else {
                        fill_rounded(
                            dc,
                            native(rect),
                            radius,
                            Corners::ALL,
                            if held {
                                palette.pressed_background
                            } else {
                                palette.hover_background
                            },
                            track_color,
                        );
                    }
                }
                let color = match (current, over) {
                    (true, _) if palette.high_contrast => palette
                        .selection_foreground
                        .unwrap_or(palette.editor_foreground),
                    (true, _) => palette.editor_foreground,
                    (false, true) if !palette.high_contrast => palette.hover_foreground,
                    _ => palette.muted_foreground,
                };
                SetTextColor(dc, color);
                select_font(dc, if current { bold } else { font });
                draw_text(dc, segment.label(), rect, centered);
            }
        }
    }
    unsafe {
        restore_font(dc, previous);
        DeleteObject(font as _);
        DeleteObject(bold as _);
        EndPaint(toolbar, &paint);
    }
}

#[cfg(test)]
mod tests {
    use super::location_segments;
    use std::path::Path;

    #[test]
    fn a_note_shows_its_notebook_and_folders() {
        assert_eq!(
            location_segments(
                Some(Path::new(r"C:\Notes\Work\Plans\q4.md")),
                "q4.md",
                Some(Path::new(r"C:\Notes")),
            ),
            vec!["Notes", "Work", "Plans", "q4.md"]
        );
        assert_eq!(
            location_segments(
                Some(Path::new(r"C:\Notes\top.md")),
                "top.md",
                Some(Path::new(r"C:\Notes"))
            ),
            vec!["Notes", "top.md"]
        );
    }

    #[test]
    fn another_file_shows_its_folder_and_an_untitled_tab_its_title() {
        // Break caught: a file outside the notebook shown under the notebook's name, or a full
        // path crowding the row.
        assert_eq!(
            location_segments(
                Some(Path::new(r"D:\src\app\main.rs")),
                "main.rs",
                Some(Path::new(r"C:\Notes")),
            ),
            vec!["app", "main.rs"]
        );
        assert_eq!(
            location_segments(None, "Untitled 2", None),
            vec!["Untitled 2"]
        );
        assert_eq!(
            location_segments(Some(Path::new(r"C:\boot.ini")), "boot.ini", None),
            vec!["boot.ini"]
        );
    }
}
