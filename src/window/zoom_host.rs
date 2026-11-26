//! Which view the zoom commands, the status bar's zoom and its zoom picker act on, and the levels
//! the picker lists. An image tab zooms its image; an SVG or FastDraw tab zooms its picture, and a
//! Markdown tab its preview, while that preview fills the tab or has the focus beside the editor;
//! everything else zooms the editors (one zoom for every group, split editors plan amendment 11).

use crate::image_view::ImageView;
use crate::image_view::zoom::{STEPS, Zoom};
use crate::window::command_palette::{Picker, PickerKind};
use crate::window::commands::CommandId;
use crate::window::main_window as host_window;
use windows_sys::Win32::Foundation::HWND;

/// The Markdown preview's zoom levels in percent of the configured preview font size.
pub(crate) const PREVIEW_STEPS: [u32; 13] =
    [50, 67, 75, 80, 90, 100, 110, 125, 150, 175, 200, 250, 300];

/// The levels the picker offers for the editor, in percent of the configured font size. The
/// editor zooms in whole points, so a level lands on the nearest size it can show.
pub(crate) const EDITOR_STEPS: [u32; 12] = [50, 67, 75, 80, 90, 100, 110, 125, 150, 175, 200, 300];

/// The picker row of an image or SVG zoom that fits the picture to the view.
pub(crate) const FIT_LABEL: &str = "Fit";

/// What the picker's empty field shows.
pub(crate) const ZOOM_PLACEHOLDER: &str = "Select zoom level";

/// What a zoom acts on.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ZoomTarget {
    /// The active image tab.
    Image,
    /// The active SVG or FastDraw tab's picture.
    Svg,
    /// The Markdown previews: one zoom for every group, like the editors.
    Markdown,
    /// Every editor, and the Live views that follow them.
    Editor,
}

thread_local! {
    /// The target the open zoom picker was opened for: opening it moves the focus, which decides
    /// the target beside an editor.
    static PICKING: std::cell::Cell<Option<ZoomTarget>> = const { std::cell::Cell::new(None) };
}

/// What the active tab's zoom acts on now.
pub(crate) fn target(hwnd: HWND) -> ZoomTarget {
    if crate::window::image_host::active_is_image(hwnd) {
        return ZoomTarget::Image;
    }
    match crate::window::preview_host::zoomed_preview(hwnd) {
        Some(crate::window::preview_host::ZoomedPreview::Svg) => ZoomTarget::Svg,
        Some(crate::window::preview_host::ZoomedPreview::Markdown) => ZoomTarget::Markdown,
        None => ZoomTarget::Editor,
    }
}

fn picture(hwnd: HWND, target: ZoomTarget) -> Option<ImageView> {
    match target {
        ZoomTarget::Image => crate::window::image_host::active_view(hwnd),
        ZoomTarget::Svg => crate::window::preview_host::active_svg_view(hwnd),
        ZoomTarget::Markdown | ZoomTarget::Editor => None,
    }
}

/// Runs Zoom in, Zoom out or Reset zoom on whatever the active tab zooms.
pub(crate) fn run(hwnd: HWND, command: CommandId) {
    let target = target(hwnd);
    match target {
        ZoomTarget::Image | ZoomTarget::Svg => {
            // An image tab never zooms the hidden editor, even with no image view to zoom.
            if let Some(view) = picture(hwnd, target) {
                match command {
                    CommandId::ZoomIn => view.zoom_in(),
                    CommandId::ZoomOut => view.zoom_out(),
                    CommandId::ZoomReset => view.zoom_reset(),
                    _ => {}
                }
            }
        }
        ZoomTarget::Markdown => {
            let current = markdown_percent(hwnd);
            let next = match command {
                CommandId::ZoomIn => step_in(&PREVIEW_STEPS, current),
                CommandId::ZoomOut => step_out(&PREVIEW_STEPS, current),
                _ => 100,
            };
            set_markdown_percent(hwnd, next);
        }
        ZoomTarget::Editor => {
            for editor in host_window::all_editors(hwnd) {
                let _ = match command {
                    CommandId::ZoomIn => editor.zoom_in(),
                    CommandId::ZoomOut => editor.zoom_out(),
                    _ => editor.reset_zoom(),
                };
            }
        }
    }
    host_window::invalidate_status_bar(hwnd);
}

/// Ctrl+wheel over a Markdown preview: one step of the previews' zoom, whatever has the focus.
pub(crate) fn wheel_markdown(hwnd: HWND, zoom_in: bool) {
    let current = markdown_percent(hwnd);
    let next = if zoom_in {
        step_in(&PREVIEW_STEPS, current)
    } else {
        step_out(&PREVIEW_STEPS, current)
    };
    set_markdown_percent(hwnd, next);
    host_window::invalidate_status_bar(hwnd);
}

/// The zoom the status bar shows for a text tab, in percent.
pub(crate) fn percent(hwnd: HWND) -> u32 {
    match target(hwnd) {
        ZoomTarget::Markdown => markdown_percent(hwnd),
        target @ (ZoomTarget::Image | ZoomTarget::Svg) => picture(hwnd, target)
            .and_then(|view| view.status().zoom_percent)
            .unwrap_or(100),
        ZoomTarget::Editor => editor_percent(hwnd),
    }
}

fn editor_percent(hwnd: HWND) -> u32 {
    unsafe { host_window::app_ptr(hwnd) }.map_or(100, |app| {
        let app = unsafe { app.as_ref() };
        crate::window::status::editor_zoom_percent(
            app.settings.font_size,
            app.editor()
                .and_then(|editor| editor.zoom().ok())
                .unwrap_or(0),
        )
    })
}

pub(crate) fn markdown_percent(hwnd: HWND) -> u32 {
    unsafe { host_window::app_ptr(hwnd) }.map_or(100, |app| unsafe { app.as_ref() }.preview_zoom)
}

fn set_markdown_percent(hwnd: HWND, percent: u32) {
    let changed = unsafe { host_window::app_ptr(hwnd) }.is_some_and(|mut app| {
        let app = unsafe { app.as_mut() };
        let changed = app.preview_zoom != percent;
        app.preview_zoom = percent;
        changed
    });
    if changed {
        crate::window::preview_host::refresh_all_appearance(hwnd);
    }
}

/// A click on the status bar's zoom: the levels of what the active tab zooms, the current one
/// selected.
pub(crate) fn open_picker(hwnd: HWND) {
    let target = target(hwnd);
    let (items, initial) = match target {
        ZoomTarget::Image | ZoomTarget::Svg => {
            let Some(view) = picture(hwnd, target) else {
                return;
            };
            let initial = match view.zoom() {
                Zoom::Fit => 0,
                Zoom::FillWidth => {
                    1 + nearest(
                        &picture_percents(),
                        (view.stats().scale * 100.0).round() as u32,
                    )
                }
                Zoom::Scale(scale) => {
                    1 + nearest(&picture_percents(), (scale * 100.0).round() as u32)
                }
            };
            let items = std::iter::once(FIT_LABEL.to_owned())
                .chain(
                    picture_percents()
                        .iter()
                        .map(|percent| format!("{percent}%")),
                )
                .collect();
            (items, initial)
        }
        ZoomTarget::Markdown => (
            labels(&PREVIEW_STEPS),
            nearest(&PREVIEW_STEPS, markdown_percent(hwnd)),
        ),
        ZoomTarget::Editor => (
            labels(&EDITOR_STEPS),
            nearest(&EDITOR_STEPS, editor_percent(hwnd)),
        ),
    };
    PICKING.with(|picking| picking.set(Some(target)));
    host_window::open_picker(
        hwnd,
        Picker {
            kind: PickerKind::Zoom,
            items,
            create: None,
            initial: Some(initial),
        },
    );
}

/// The zoom picker's row `index` was chosen.
pub(crate) fn picked(hwnd: HWND, index: usize) {
    let Some(target) = PICKING.with(std::cell::Cell::take) else {
        return;
    };
    match target {
        ZoomTarget::Image | ZoomTarget::Svg => {
            let Some(view) = picture(hwnd, target) else {
                return;
            };
            if index == 0 {
                view.set_zoom(Zoom::Fit);
            } else if let Some(percent) = picture_percents().get(index - 1) {
                view.set_zoom(Zoom::Scale(*percent as f32 / 100.0));
            }
        }
        ZoomTarget::Markdown => {
            if let Some(&percent) = PREVIEW_STEPS.get(index) {
                set_markdown_percent(hwnd, percent);
            }
        }
        ZoomTarget::Editor => {
            let Some(&percent) = EDITOR_STEPS.get(index) else {
                return;
            };
            let font_size = unsafe { host_window::app_ptr(hwnd) }
                .map_or(11, |app| unsafe { app.as_ref() }.settings.font_size);
            let zoom = editor_zoom_for(font_size, percent);
            for editor in host_window::all_editors(hwnd) {
                let _ = editor.set_zoom(zoom);
            }
        }
    }
    host_window::invalidate_status_bar(hwnd);
}

/// The image view's zoom steps in percent.
fn picture_percents() -> Vec<u32> {
    STEPS
        .iter()
        .map(|step| (step * 100.0).round() as u32)
        .collect()
}

fn labels(steps: &[u32]) -> Vec<String> {
    steps.iter().map(|percent| format!("{percent}%")).collect()
}

/// The index of the step closest to `percent`.
pub(crate) fn nearest(steps: &[u32], percent: u32) -> usize {
    steps
        .iter()
        .enumerate()
        .min_by_key(|(_, step)| step.abs_diff(percent))
        .map_or(0, |(index, _)| index)
}

/// The first step above `current`, or the last.
pub(crate) fn step_in(steps: &[u32], current: u32) -> u32 {
    steps
        .iter()
        .copied()
        .find(|&step| step > current)
        .unwrap_or(steps[steps.len() - 1])
}

/// The last step below `current`, or the first.
pub(crate) fn step_out(steps: &[u32], current: u32) -> u32 {
    steps
        .iter()
        .rev()
        .copied()
        .find(|&step| step < current)
        .unwrap_or(steps[0])
}

/// The editor zoom (points added to `font_size`) closest to `percent` of it. Scintilla never
/// draws text below 2 points.
pub(crate) fn editor_zoom_for(font_size: u16, percent: u32) -> i32 {
    let base = i32::from(font_size.max(1));
    let wanted = (f64::from(base) * f64::from(percent) / 100.0).round() as i32;
    wanted.max(2) - base
}

#[cfg(test)]
mod tests {
    use super::{EDITOR_STEPS, PREVIEW_STEPS, editor_zoom_for, nearest, step_in, step_out};
    use crate::window::status::editor_zoom_percent;

    #[test]
    fn preview_steps_move_to_the_next_level_from_any_level() {
        // Break caught: Ctrl+wheel stuck at a level that is not on the list, or running past the
        // ends.
        assert_eq!(step_in(&PREVIEW_STEPS, 100), 110);
        assert_eq!(step_in(&PREVIEW_STEPS, 105), 110);
        assert_eq!(step_in(&PREVIEW_STEPS, 300), 300);
        assert_eq!(step_out(&PREVIEW_STEPS, 100), 90);
        assert_eq!(step_out(&PREVIEW_STEPS, 50), 50);
    }

    #[test]
    fn the_picker_selects_the_level_closest_to_the_current_zoom() {
        // Break caught: an editor at 109% (11 pt + 1) opening the picker on 50%.
        assert_eq!(EDITOR_STEPS[nearest(&EDITOR_STEPS, 109)], 110);
        assert_eq!(EDITOR_STEPS[nearest(&EDITOR_STEPS, 100)], 100);
        assert_eq!(PREVIEW_STEPS[nearest(&PREVIEW_STEPS, 999)], 300);
    }

    #[test]
    fn an_editor_level_lands_on_the_nearest_whole_point_size() {
        // Break caught: picking 200% of 11 pt adding 200 points, or 100% leaving a zoom behind.
        assert_eq!(editor_zoom_for(11, 100), 0);
        assert_eq!(editor_zoom_for(11, 200), 11);
        assert_eq!(editor_zoom_for(11, 50), -5);
        assert_eq!(editor_zoom_percent(11, editor_zoom_for(11, 150)), 155);
        assert_eq!(editor_zoom_for(2, 50), 0);
    }
}
