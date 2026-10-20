//! Painting the Live view: the visible lines, the selection behind them and the caret, with
//! Direct2D, and the scroll bar state a frame leaves behind. Each line's decorations (phase 3
//! plan, Task 6) are drawn with it: fills behind everything, then the selection, then bars,
//! lines, markers, checkboxes and images, then the text.

use super::*;
use crate::live::layout::Decoration;
use crate::live::styles::BodyStyle;
use crate::preview::render::{RectF, color_f, create_hwnd_target_with};
use windows::Win32::Foundation::D2DERR_RECREATE_TARGET;
use windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F;
use windows::Win32::Graphics::Direct2D::D2D1_DRAW_TEXT_OPTIONS_NONE;
use windows::Win32::Graphics::Direct2D::{
    D2D1_ANTIALIAS_MODE_ALIASED, D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, D2D1_ELLIPSE,
    D2D1_PRESENT_OPTIONS, D2D1_PRESENT_OPTIONS_IMMEDIATELY, D2D1_PRESENT_OPTIONS_RETAIN_CONTENTS,
};
use windows_numerics::{Matrix3x2, Vector2};
use windows_sys::Win32::Graphics::Gdi::GetUpdateRect;
use windows_sys::Win32::UI::WindowsAndMessaging::{SCROLLINFO, SIF_ALL};

/// How far the edited block's highlight reaches past the text on each side, in DIPs.
const BLOCK_BLEED: f32 = 4.0;

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
    // Pending edits and the caret's line first: both may invalidate more of the view.
    prepare(state);
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
            // Bitmaps belonged to the old target's device.
            state.pictures.cache.release_bitmaps();
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
            heading: brush(state.colors.heading)?,
            emphasis: brush(state.colors.emphasis)?,
            marker: brush(state.colors.marker)?,
            link: brush(state.colors.link)?,
            code: brush(state.colors.code)?,
            code_background: brush(state.colors.code_background)?,
            dim: brush(state.colors.dim)?,
            active_block: brush(state.colors.active_block)?,
        });
        state.brush_generation += 1;
    }

    let (_, view_height) = view_size(hwnd);
    let gutter = gutter_width(state);
    if (gutter - state.gutter).abs() > 0.01 {
        // Line numbers turned on or off, or the count gained a digit: the text moves.
        state.gutter = gutter;
        full = true;
    }
    let width = text_width(hwnd, gutter);
    if state.layout.line_count() != EditorText(&state.editor).line_count() {
        // An edit the view was not told about; never draw a stale document.
        load_document(state);
        full = true;
    }
    let ViewState {
        editor,
        layout,
        document,
        revealed,
        scroll_y,
        pictures,
        ..
    } = state;
    let source = EditorText(editor);
    let text = Styled {
        text: &source,
        document,
        revealed: revealed.clone(),
        images: &|dest: &str| pictures.resolve(dest),
    };
    let scroll_before = *scroll_y;
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
    // Image lines on screen whose pictures are not decoded yet: ask for them at the text width.
    let pending = visible
        .clone()
        .filter_map(|line| match &layout.laid_line(line)?.style.block.body {
            BodyStyle::Image { dest, .. } => match pictures.resolve(dest) {
                ImageInfo::Pending(path) => Some(path),
                _ => None,
            },
            _ => None,
        })
        .collect::<Vec<_>>();
    let pixel_text_width = (layout.width() * dpi as f32 / 96.0).ceil() as u32;
    for path in pending {
        pictures.cache.request(&path, pixel_text_width);
    }
    // Where a decoration reaching the right edge (`f32::MAX`) ends, in line coordinates.
    let text_right = layout.width();

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
        // Code backgrounds first, so a selection over inline code still shows.
        #[cfg(test)]
        let mut backgrounds = 0;
        #[cfg(test)]
        let mut painted = PaintedDecorations::default();
        let right = |x: f32| if x == f32::MAX { text_right } else { x };
        for (line, (top, bottom)) in visible.clone().zip(spans.iter().copied()) {
            if (bottom - scroll_y) < clip_top || (top - scroll_y) > clip_bottom {
                continue;
            }
            let Some(laid) = state.layout.laid_line(line) else {
                continue;
            };
            // The block being edited stands out, as the source view's current line does.
            if laid.style.revealed && state.colors.active_block != state.colors.background {
                target.FillRectangle(
                    &rect(-BLOCK_BLEED, top, text_right + BLOCK_BLEED, bottom),
                    &brushes.active_block,
                );
                #[cfg(test)]
                {
                    painted.active_block += 1;
                }
            }
            for decoration in &laid.decorations {
                if let Decoration::Fill { rect: area, paint } = decoration {
                    target.FillRectangle(
                        &rect(
                            area.left,
                            top + area.top,
                            right(area.right),
                            top + area.bottom,
                        ),
                        brushes.paint(*paint),
                    );
                    #[cfg(test)]
                    {
                        painted.fills += 1;
                    }
                }
            }
            for &(start, end, paint) in &laid.backgrounds {
                for area in laid.range_rects(start, end).unwrap_or_default() {
                    target.FillRectangle(
                        &rect(area.left, top + area.top, area.right, top + area.bottom),
                        brushes.paint(paint),
                    );
                    #[cfg(test)]
                    {
                        backgrounds += 1;
                    }
                }
            }
        }
        #[cfg(test)]
        {
            state.painted_backgrounds = backgrounds;
        }
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
        // Pictures are drawn last: an image line has no text to cover them.
        let mut images: Vec<(D2D_RECT_F, &std::path::Path)> = Vec::new();
        for (line, (top, bottom)) in visible.clone().zip(spans) {
            let on_screen = (top - scroll_y)..(bottom - scroll_y);
            if on_screen.end < clip_top || on_screen.start > clip_bottom {
                continue;
            }
            if let Some(laid) = state.layout.laid_line(line) {
                let at = |area: &RectF| {
                    rect(
                        area.left,
                        top + area.top,
                        right(area.right),
                        top + area.bottom,
                    )
                };
                let point = |(x, y): (f32, f32)| Vector2 {
                    X: text_x + right(x),
                    Y: top + y - scroll_y,
                };
                for decoration in &laid.decorations {
                    match decoration {
                        Decoration::Fill { .. } => {}
                        Decoration::Line { from, to, paint } => {
                            target.DrawLine(
                                point(*from),
                                point(*to),
                                brushes.paint(*paint),
                                1.0,
                                None,
                            );
                            #[cfg(test)]
                            {
                                painted.lines += 1;
                            }
                        }
                        Decoration::Bullet {
                            center,
                            radius,
                            depth,
                        } => {
                            let center = point(*center);
                            let ellipse = D2D1_ELLIPSE {
                                point: center,
                                radiusX: *radius,
                                radiusY: *radius,
                            };
                            match depth {
                                0 | 1 => target.FillEllipse(&ellipse, &brushes.text),
                                2 => target.DrawEllipse(&ellipse, &brushes.text, 1.0, None),
                                _ => target.FillRectangle(
                                    &D2D_RECT_F {
                                        left: center.X - radius * 0.8,
                                        top: center.Y - radius * 0.8,
                                        right: center.X + radius * 0.8,
                                        bottom: center.Y + radius * 0.8,
                                    },
                                    &brushes.text,
                                ),
                            }
                            #[cfg(test)]
                            {
                                painted.bullets += 1;
                            }
                        }
                        Decoration::Number { rect: area, text } => {
                            if let Ok(label) = state
                                .layout
                                .label_layout(text, (area.right - area.left).max(1.0))
                            {
                                target.DrawTextLayout(
                                    Vector2 {
                                        X: text_x + area.left,
                                        Y: top + area.top - scroll_y,
                                    },
                                    &label,
                                    &brushes.marker,
                                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                                );
                            }
                            #[cfg(test)]
                            {
                                painted.numbers += 1;
                            }
                        }
                        Decoration::Checkbox {
                            rect: area,
                            checked,
                            ..
                        } => {
                            let outline = at(area);
                            let inset = (area.right - area.left) * 0.22;
                            target.DrawRectangle(
                                &D2D_RECT_F {
                                    left: outline.left + 0.5,
                                    top: outline.top + 0.5,
                                    right: outline.right - 0.5,
                                    bottom: outline.bottom - 0.5,
                                },
                                &brushes.marker,
                                1.0,
                                None,
                            );
                            if *checked {
                                target.FillRectangle(
                                    &D2D_RECT_F {
                                        left: outline.left + inset,
                                        top: outline.top + inset,
                                        right: outline.right - inset,
                                        bottom: outline.bottom - inset,
                                    },
                                    &brushes.link,
                                );
                            }
                            #[cfg(test)]
                            {
                                painted.checkboxes += 1;
                            }
                        }
                        Decoration::Image { rect: area, path } => {
                            images.push((at(area), path.as_path()));
                        }
                    }
                }
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
                laid.bind(|paint| Some(brushes.paint(paint)), state.brush_generation);
                target.DrawTextLayout(
                    Vector2 {
                        X: text_x + laid.indent,
                        Y: top - scroll_y,
                    },
                    &laid.layout,
                    &brushes.text,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                );
            }
        }
        for (area, path) in images {
            if let Some(bitmap) = state.pictures.cache.bitmap(target, path) {
                target.DrawBitmap(
                    &bitmap,
                    Some(&area),
                    1.0,
                    D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                    None,
                );
                #[cfg(test)]
                {
                    painted.images += 1;
                }
            }
        }
        #[cfg(test)]
        {
            state.painted = painted;
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
