//! Painting the Live view: the visible lines, the selection behind them and the caret, with
//! Direct2D, and the scroll bar state a frame leaves behind.

use super::*;
use crate::preview::render::{color_f, create_hwnd_target_with};
use windows::Win32::Foundation::D2DERR_RECREATE_TARGET;
use windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F;
use windows::Win32::Graphics::Direct2D::D2D1_DRAW_TEXT_OPTIONS_NONE;
use windows::Win32::Graphics::Direct2D::{
    D2D1_ANTIALIAS_MODE_ALIASED, D2D1_PRESENT_OPTIONS, D2D1_PRESENT_OPTIONS_IMMEDIATELY,
    D2D1_PRESENT_OPTIONS_RETAIN_CONTENTS,
};
use windows_numerics::{Matrix3x2, Vector2};
use windows_sys::Win32::Graphics::Gdi::GetUpdateRect;
use windows_sys::Win32::UI::WindowsAndMessaging::{SCROLLINFO, SIF_ALL};

/// What `WM_PAINT` must do once the state borrow has ended.
pub(super) struct PaintOutcome {
    /// Scroll bar state for a completed frame. `SetScrollInfo` can send `WM_SIZE` synchronously,
    /// so it must never run while `ViewState` is borrowed.
    pub(super) scroll: Option<SCROLLINFO>,
    /// Paint again: the device was lost.
    pub(super) repaint: bool,
}

pub(super) fn paint(state: &mut ViewState) -> Result<PaintOutcome> {
    #[cfg(test)]
    let started = std::time::Instant::now();
    let hwnd = state.hwnd;
    let mut client = RECT::default();
    unsafe { GetClientRect(hwnd, &mut client) };
    let (pixel_width, pixel_height) = (
        (client.right - client.left).max(1) as u32,
        (client.bottom - client.top).max(1) as u32,
    );
    let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
    // What this frame must repaint: the update region's bounds, or everything when the target is
    // new, the text rewraps or the view scrolls (the target keeps the rest from the last frame).
    let mut update = RECT::default();
    let mut full = unsafe { GetUpdateRect(hwnd, &mut update, 0) } == 0;
    match &state.target {
        None => {
            full = true;
            // Painting once per keystroke must not wait for the display to refresh.
            state.target = Some(create_hwnd_target_with(
                &state.graphics,
                hwnd,
                pixel_width,
                pixel_height,
                dpi,
                // Kept between frames, so a frame repaints only what changed.
                D2D1_PRESENT_OPTIONS(
                    D2D1_PRESENT_OPTIONS_IMMEDIATELY.0 | D2D1_PRESENT_OPTIONS_RETAIN_CONTENTS.0,
                ),
            )?);
            state.brushes = None;
        }
        Some(target) => {
            // The target takes its DPI only at creation; follow a move to another monitor.
            let (mut dpi_x, mut dpi_y) = (0.0, 0.0);
            unsafe { target.GetDpi(&mut dpi_x, &mut dpi_y) };
            if (dpi_x - dpi as f32).abs() > 0.5 || (dpi_y - dpi as f32).abs() > 0.5 {
                unsafe { target.SetDpi(dpi as f32, dpi as f32) };
            }
        }
    }
    if state.brushes.is_none() {
        let target = state.target.as_ref().expect("created above");
        let brush = |color: u32| {
            unsafe { target.CreateSolidColorBrush(&color_f(color), None) }
                .map_err(crate::preview::dwrite::hresult_error)
        };
        state.brushes = Some(Brushes {
            text: brush(state.colors.text)?,
            selection: brush(state.colors.selection)?,
            inactive_selection: brush(state.colors.inactive_selection)?,
            caret: brush(state.colors.caret)?,
            line_number: brush(state.colors.line_number)?,
        });
    }

    let (_, view_height) = view_size(hwnd);
    let gutter = gutter_width(state);
    if (gutter - state.gutter).abs() > 0.01 {
        // Line numbers turned on or off, or the count gained a digit: the text moves.
        state.gutter = gutter;
        full = true;
    }
    let width = text_width(hwnd, gutter);
    let ViewState {
        editor,
        layout,
        scroll_y,
        ..
    } = state;
    let text = EditorText(editor);
    let scroll_before = *scroll_y;
    if layout.line_count() != text.line_count() {
        // An edit the view was not told about; never draw a stale document.
        layout.reset(&text);
        full = true;
    }
    if (layout.width() - width).abs() > 0.5 {
        let anchor = layout.anchor(*scroll_y);
        layout.set_width(&text, width);
        *scroll_y = layout.scroll_for_anchor(anchor);
        full = true;
    }
    // Laying out a line replaces its estimated height; keep the top line steady while it does.
    let mut visible = 0..0;
    for _ in 0..3 {
        let anchor = layout.anchor(*scroll_y);
        visible = visible_lines(layout, *scroll_y, view_height);
        layout.ensure(&text, visible.clone())?;
        let steady = layout.scroll_for_anchor(anchor);
        if (steady - *scroll_y).abs() < 0.5 {
            break;
        }
        *scroll_y = steady;
    }
    *scroll_y = scroll::clamp(*scroll_y, view_height, layout.total_height());
    visible = visible_lines(layout, *scroll_y, view_height);
    layout.ensure(&text, visible.clone())?;
    full |= (*scroll_y - scroll_before).abs() > 0.01;

    let (anchor, caret) = text.selection();
    let selection = anchor.min(caret)..anchor.max(caret);
    let selection_rects = layout.selection_rects(&text, selection, visible.clone())?;
    let caret_rect = state
        .focused
        .then(|| layout.caret_rect(&text, caret).ok())
        .flatten();
    let spans = visible
        .clone()
        .map(|line| (layout.line_top(line), layout.line_top(line + 1)))
        .collect::<Vec<_>>();

    let target = state.target.as_ref().expect("created above");
    let brushes = state.brushes.as_ref().expect("created above");
    let scroll_y = state.scroll_y;
    let text_x = text_left(state);
    let rect = |left: f32, top: f32, right: f32, bottom: f32| D2D_RECT_F {
        left: text_x + left,
        top: top - scroll_y,
        right: text_x + right,
        bottom: bottom - scroll_y,
    };
    let scale = dpi_scale(hwnd);
    // The repainted band, in client DIPs.
    let (clip_top, clip_bottom) = if full {
        (f32::MIN, f32::MAX)
    } else {
        (update.top as f32 / scale, update.bottom as f32 / scale)
    };
    #[cfg(test)]
    record("paint: prepare", started);
    #[cfg(test)]
    let drawing = std::time::Instant::now();
    let result = unsafe {
        target.BeginDraw();
        target.SetTransform(&Matrix3x2::identity());
        target.PushAxisAlignedClip(
            &D2D_RECT_F {
                left: f32::MIN,
                top: clip_top,
                right: f32::MAX,
                bottom: clip_bottom,
            },
            D2D1_ANTIALIAS_MODE_ALIASED,
        );
        target.Clear(Some(&color_f(state.colors.background)));
        let selection_brush = if state.focused {
            &brushes.selection
        } else {
            &brushes.inactive_selection
        };
        for selected in &selection_rects {
            target.FillRectangle(
                &rect(selected.left, selected.top, selected.right, selected.bottom),
                selection_brush,
            );
        }
        for (line, (top, bottom)) in visible.clone().zip(spans) {
            let on_screen = (top - scroll_y)..(bottom - scroll_y);
            if on_screen.end < clip_top || on_screen.start > clip_bottom {
                continue;
            }
            if gutter > 0.0
                && let Ok(number) = state.layout.number_layout(line + 1, gutter - GUTTER_GAP)
            {
                target.DrawTextLayout(
                    Vector2 {
                        X: PADDING,
                        Y: top - scroll_y,
                    },
                    &number,
                    &brushes.line_number,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                );
            }
            if let Some(laid) = state.layout.laid_line(line) {
                target.DrawTextLayout(
                    Vector2 {
                        X: text_x,
                        Y: top - scroll_y,
                    },
                    &laid.layout,
                    &brushes.text,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                );
            }
        }
        if let Some(caret) = caret_rect.filter(|_| state.caret_on) {
            let width = 1.0_f32.max(1.0 / dpi_scale(hwnd));
            target.FillRectangle(
                &rect(caret.left, caret.top, caret.left + width, caret.bottom),
                &brushes.caret,
            );
        }
        target.PopAxisAlignedClip();
        target.EndDraw(None, None)
    };
    #[cfg(test)]
    record("paint: draw and present", drawing);
    state.caret_span = caret_rect.map(|caret| (caret.top, caret.bottom));
    state.marks = selection_rects
        .iter()
        .map(|selected| (selected.top, selected.bottom))
        .chain(state.caret_span)
        .reduce(|(top, bottom), (next_top, next_bottom)| {
            (top.min(next_top), bottom.max(next_bottom))
        });
    state.caret_px = caret_rect.map(|caret| {
        let x = ((text_x + caret.left) * scale).round() as i32;
        let y = ((caret.top - scroll_y) * scale).round() as i32;
        (x, y, ((caret.bottom - caret.top) * scale).round() as i32)
    });
    if let Some((x, y, _)) = state.caret_px {
        // Moving the hidden system caret sends no message.
        unsafe { windows_sys::Win32::UI::WindowsAndMessaging::SetCaretPos(x, y) };
    }
    match result {
        Err(error) if error.code() == D2DERR_RECREATE_TARGET => {
            // The next target starts blank: its first frame repaints everything.
            invalidate(hwnd);
            state.target = None;
            state.brushes = None;
            Ok(PaintOutcome {
                scroll: None,
                repaint: true,
            })
        }
        Err(error) => Err(crate::preview::dwrite::hresult_error(error)),
        Ok(()) => Ok(PaintOutcome {
            scroll: Some(scroll_info(state, view_height)),
            repaint: false,
        }),
    }
}

/// The lines at least partly inside the viewport at `scroll_y`.
fn visible_lines(
    layout: &mut LiveLayout,
    scroll_y: f32,
    view_height: f32,
) -> std::ops::Range<usize> {
    let first = layout.line_at_y(scroll_y);
    let last = layout.line_at_y(scroll_y + view_height);
    first..(last + 1).min(layout.line_count())
}

pub(super) fn scroll_info(state: &mut ViewState, view_height: f32) -> SCROLLINFO {
    let scale = dpi_scale(state.hwnd);
    SCROLLINFO {
        cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
        fMask: SIF_ALL,
        nMin: 0,
        nMax: (state.layout.total_height() * scale) as i32,
        nPage: (view_height * scale).max(0.0) as u32,
        nPos: (state.scroll_y * scale) as i32,
        nTrackPos: 0,
    }
}
