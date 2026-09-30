//! Painting the preview: draws the visible blocks, the paused bar and focus with Direct2D,
//! and the scroll bar state a frame leaves behind.

use super::*;

/// What `WM_PAINT` must do once the state borrow has ended.
pub(super) struct PaintOutcome {
    /// Scroll bar state for a completed frame. `SetScrollInfo` can send `WM_SIZE` synchronously,
    /// so it must never run while `ViewState` is borrowed.
    pub(super) scroll: Option<SCROLLINFO>,
    /// Paint again: the device was lost, or a visible block is still without a layout.
    pub(super) repaint: bool,
    /// Accessible child id of a disclosure whose state changed, raised once the borrow has ended.
    pub(super) state_change: Option<i32>,
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
    if state.target.is_none() {
        let target = create_hwnd_target(&state.graphics, hwnd, pixel_width, pixel_height, dpi)?;
        state.brushes = None;
        state.target = Some(target);
    } else if let Some(target) = &state.target {
        // The target takes its DPI only at creation. After a move to a monitor with another DPI it
        // would draw at the old scale while hit-testing, scroll info, and image decodes use the
        // window's live DPI, so adopt the new one before drawing.
        let (mut dpi_x, mut dpi_y) = (0.0, 0.0);
        unsafe { target.GetDpi(&mut dpi_x, &mut dpi_y) };
        let dpi = dpi as f32;
        if (dpi_x - dpi).abs() > 0.5 || (dpi_y - dpi).abs() > 0.5 {
            unsafe { target.SetDpi(dpi, dpi) };
            // Layouts are in DIPs, but pixel snapping and image decode sizes follow the DPI.
            relayout(state);
        }
    }
    if state.brushes.is_none() {
        let target = state.target.as_ref().expect("created above");
        state.brushes = Some(Brushes::create(target, &state.colors)?);
        relayout(state);
    }
    let (view_width, view_height) = view_size(hwnd);
    let (content_left, content_width) = content_frame(view_width, state.centered);
    if (content_width - state.layout_width).abs() > 0.5
        && (!state.live_resize || state.layout_width == 0.0)
    {
        state.layout_width = content_width;
        relayout(state);
    }
    let bar = bar_height(state);
    for _ in 0..3 {
        let anchor = state.heights.anchor(state.scroll_y);
        let visible = visible_indices(state, view_height - bar);
        if !ensure_layouts(state, &visible)? {
            break;
        }
        state.scroll_y = state.heights.scroll_for_anchor(anchor);
    }
    state.scroll_y = state.scroll_y.clamp(0.0, max_scroll(state, view_height));

    let mut visible = visible_indices(state, view_height - bar);
    if visible.iter().any(|index| state.layouts[*index].is_none()) {
        // The clamp or the last anchor pass uncovered blocks the loop did not lay out.
        let anchor = state.heights.anchor(state.scroll_y);
        if ensure_layouts(state, &visible)? {
            let restored = state.heights.scroll_for_anchor(anchor);
            state.scroll_y = restored.clamp(0.0, max_scroll(state, view_height));
        }
        visible = visible_indices(state, view_height - bar);
    }
    let repaint = visible.iter().any(|index| state.layouts[*index].is_none());
    let tops = visible
        .iter()
        .map(|index| state.heights.top(*index))
        .collect::<Vec<_>>();
    let ViewState {
        target,
        brushes,
        layouts,
        images,
        h_scroll,
        focus,
        scroll_y,
        colors,
        graphics,
        fonts,
        paused,
        ..
    } = state;
    let target = target.as_ref().expect("created above");
    let brushes = brushes.as_ref().expect("created above");
    let result = unsafe {
        target.BeginDraw();
        target.SetTransform(&Matrix3x2::identity());
        target.Clear(Some(&color_f(colors.background)));
        for (index, top) in visible.iter().zip(tops) {
            let Some(laid) = layouts[*index].as_ref() else {
                continue;
            };
            let y = top - *scroll_y + bar;
            let offset = h_scroll.get(index).copied().unwrap_or(0.0);
            draw_ops(
                target,
                brushes,
                &laid.ops,
                &laid.images,
                images,
                content_left,
                y,
                offset,
            );
            if let Some((focus_block, focus_link)) = *focus
                && focus_block == *index
                && let Some(link) = laid.targets.get(focus_link)
            {
                for rect in link.visible_rects(offset) {
                    target.DrawRectangle(
                        &rect.offset(content_left, y).inflate(2.0).to_d2d(),
                        brushes.get(ColorRole::Focus),
                        2.0,
                        None,
                    );
                }
            }
        }
        if *paused {
            let bar_rect =
                crate::preview::render::RectF::new(0.0, 0.0, view_width, PAUSED_BAR_HEIGHT);
            target.FillRectangle(&bar_rect.to_d2d(), brushes.get(ColorRole::CodeBackground));
            if let Ok(format) = graphics.text_format(
                &fonts.body_family,
                fonts.body_size,
                DWRITE_FONT_WEIGHT_NORMAL,
                DWRITE_FONT_STYLE_NORMAL,
            ) {
                let text = PAUSED_TEXT.encode_utf16().collect::<Vec<_>>();
                if let Ok(layout) = graphics.dwrite.CreateTextLayout(
                    &text,
                    &format,
                    view_width - 2.0 * PADDING,
                    PAUSED_BAR_HEIGHT,
                ) {
                    target.DrawTextLayout(
                        Vector2 { X: PADDING, Y: 6.0 },
                        &layout,
                        brushes.get(ColorRole::Text),
                        windows::Win32::Graphics::Direct2D::D2D1_DRAW_TEXT_OPTIONS_NONE,
                    );
                }
            }
        }
        target.EndDraw(None, None)
    };
    match result {
        Err(error) if error.code() == D2DERR_RECREATE_TARGET => {
            drop_device_resources(state);
            Ok(PaintOutcome {
                scroll: None,
                repaint: true,
                state_change: None,
            })
        }
        Err(error) => Err(crate::preview::dwrite::hresult_error(error)),
        Ok(()) => {
            if let Some(opened) = state.opened_at.take() {
                state.stats.first_frame_micros = opened.elapsed().as_micros().max(1) as u64;
            }
            if let Some(started) = state.update_started.take() {
                state.stats.last_update_micros = started.elapsed().as_micros().max(1) as u64;
                state.stats.painted_updates += 1;
            }
            state.stats.content_height = state.heights.total();
            let links = visible_links(state);
            let state_change = state.state_change.take().and_then(|key| {
                links
                    .iter()
                    .position(|link| link.disclosure.as_ref().is_some_and(|d| d.key == key))
                    .map(|index| index as i32 + 1)
            });
            *state
                .accessible
                .write()
                .unwrap_or_else(|error| error.into_inner()) = links;
            Ok(PaintOutcome {
                scroll: Some(scroll_info(state, view_height)),
                repaint,
                state_change,
            })
        }
    }
}

pub(super) fn scroll_info(state: &mut ViewState, view_height: f32) -> SCROLLINFO {
    let scale = dpi_scale(state.hwnd);
    SCROLLINFO {
        cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
        fMask: SIF_ALL,
        nMin: 0,
        nMax: (state.heights.total() * scale) as i32,
        nPage: ((view_height - bar_height(state)) * scale).max(0.0) as u32,
        nPos: (state.scroll_y * scale) as i32,
        nTrackPos: 0,
    }
}
