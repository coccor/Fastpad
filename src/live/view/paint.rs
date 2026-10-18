//! Painting the Live view: the visible lines, the selection behind them and the caret, with
//! Direct2D, and the scroll bar state a frame leaves behind.

use super::*;
use crate::preview::render::{color_f, create_hwnd_target};
use windows::Win32::Foundation::D2DERR_RECREATE_TARGET;
use windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F;
use windows::Win32::Graphics::Direct2D::D2D1_DRAW_TEXT_OPTIONS_NONE;
use windows_numerics::{Matrix3x2, Vector2};
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
    let hwnd = state.hwnd;
    let mut client = RECT::default();
    unsafe { GetClientRect(hwnd, &mut client) };
    let (pixel_width, pixel_height) = (
        (client.right - client.left).max(1) as u32,
        (client.bottom - client.top).max(1) as u32,
    );
    let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
    match &state.target {
        None => {
            state.target = Some(create_hwnd_target(
                &state.graphics,
                hwnd,
                pixel_width,
                pixel_height,
                dpi,
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
        });
    }

    let (_, view_height) = view_size(hwnd);
    let width = text_width(hwnd);
    let ViewState {
        editor,
        layout,
        scroll_y,
        ..
    } = state;
    let text = EditorText(editor);
    if layout.line_count() != text.line_count() {
        // An edit the view was not told about; never draw a stale document.
        layout.reset(&text);
    }
    if (layout.width() - width).abs() > 0.5 {
        let anchor = layout.anchor(*scroll_y);
        layout.set_width(&text, width);
        *scroll_y = layout.scroll_for_anchor(anchor);
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

    let (anchor, caret) = text.selection();
    let selection = anchor.min(caret)..anchor.max(caret);
    let selection_rects = layout.selection_rects(&text, selection, visible.clone())?;
    let caret_rect = (state.focused && state.caret_on)
        .then(|| layout.caret_rect(&text, caret).ok())
        .flatten();
    let tops = visible
        .clone()
        .map(|line| layout.line_top(line))
        .collect::<Vec<_>>();

    let target = state.target.as_ref().expect("created above");
    let brushes = state.brushes.as_ref().expect("created above");
    let scroll_y = state.scroll_y;
    let rect = |left: f32, top: f32, right: f32, bottom: f32| D2D_RECT_F {
        left: PADDING + left,
        top: top - scroll_y,
        right: PADDING + right,
        bottom: bottom - scroll_y,
    };
    let result = unsafe {
        target.BeginDraw();
        target.SetTransform(&Matrix3x2::identity());
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
        for (line, top) in visible.clone().zip(tops) {
            if let Some(laid) = state.layout.laid_line(line) {
                target.DrawTextLayout(
                    Vector2 {
                        X: PADDING,
                        Y: top - scroll_y,
                    },
                    &laid.layout,
                    &brushes.text,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                );
            }
        }
        if let Some(caret) = caret_rect {
            let width = 1.0_f32.max(1.0 / dpi_scale(hwnd));
            target.FillRectangle(
                &rect(caret.left, caret.top, caret.left + width, caret.bottom),
                &brushes.caret,
            );
        }
        target.EndDraw(None, None)
    };
    match result {
        Err(error) if error.code() == D2DERR_RECREATE_TARGET => {
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
